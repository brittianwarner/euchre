//! The embedded server's footing on this machine: one copy of Euchre per data dir, the
//! engine's first port, whose engine is on the storage, and a watchdog that stops the engine
//! when the app dies without quitting (review fixes LDS-1, LDS-2, LDS-3, LDS-6).

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use euchre_server_core::rivet::{
    ENGINE_PID_FILE, engine_ports, loopback_port_in_use, recorded_engine_port,
};
use tokio_util::sync::CancellationToken;

/// The lock file in the data dir: held by the one copy of Euchre that runs a server there.
pub const LOCK_FILE: &str = "euchre.lock";
/// How long a launch waits for the copy before it (quitting, say) to let go of the data dir
/// and its engine.
pub const INSTANCE_WAIT: Duration = Duration::from_secs(20);
/// How often a waiting launch looks again.
const POLL: Duration = Duration::from_millis(250);
/// The guard ports a first run picks from. The engine also binds `+1` and `+5`, so the
/// highest base is 29990 (`+5` = 29995).
pub const FIRST_PORTS: (u16, u16) = (20_000, 29_990);
/// The step between candidate guard ports (keeps `+1` and `+5` inside each block).
const PORT_STEP: u16 = 10;

/// The data dir's lock, held while this process may run a server there. Released when it
/// drops (or the process exits, however it exits).
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

/// Why the lock was not taken.
#[derive(Debug)]
pub enum LockError {
    /// Another process holds it.
    Held,
    /// The lock file could not be opened or locked.
    Io(io::Error),
}

/// Takes `<data_dir>/euchre.lock` (mode 0600) now, or says why not.
pub fn try_lock(data_dir: &Path) -> Result<InstanceLock, LockError> {
    crate::startup::create_private_dir(data_dir).map_err(LockError::Io)?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options
        .open(data_dir.join(LOCK_FILE))
        .map_err(LockError::Io)?;
    match file.try_lock() {
        Ok(()) => Ok(InstanceLock { _file: file }),
        Err(TryLockError::WouldBlock) => Err(LockError::Held),
        Err(TryLockError::Error(err)) => Err(LockError::Io(err)),
    }
}

/// [`try_lock`] again every 250 ms for up to `wait` (a copy that is quitting lets go within
/// it). `None` when cancelled.
pub async fn lock_within(
    data_dir: &Path,
    wait: Duration,
    cancel: &CancellationToken,
) -> Option<Result<InstanceLock, LockError>> {
    let deadline = Instant::now() + wait;
    loop {
        match try_lock(data_dir) {
            Err(LockError::Held) if Instant::now() < deadline => {}
            other => return Some(other),
        }
        tokio::select! {
            () = tokio::time::sleep(POLL) => {}
            () = cancel.cancelled() => return None,
        }
    }
}

/// `<storage>/.rivetkit/var/engine`.
fn engine_var_dir(storage: &Path) -> PathBuf {
    storage.join(".rivetkit").join("var").join("engine")
}

/// The engine's config path on `storage`: every engine started there runs with
/// `--config <this>`.
pub fn engine_config_path(storage: &Path) -> PathBuf {
    engine_var_dir(storage).join("config.json")
}

/// The engine and owner pids recorded on `storage` (`<engine pid> <owner pid>` in the pid
/// file), whether or not they still run.
pub fn recorded_engine(storage: &Path) -> Option<(u32, Option<u32>)> {
    let text = std::fs::read_to_string(engine_var_dir(storage).join(ENGINE_PID_FILE)).ok()?;
    let mut fields = text.split_whitespace();
    let engine: u32 = fields.next()?.parse().ok()?;
    let owner = fields.next().and_then(|f| f.parse().ok());
    Some((engine, owner))
}

/// The process that owns the engine recorded on `storage`, when it is alive and is not this
/// process.
pub fn other_live_owner(storage: &Path) -> Option<u32> {
    let owner = recorded_engine(storage)?.1?;
    (owner != std::process::id() && process_command(owner).is_some()).then_some(owner)
}

/// Waits (every 250 ms, until `deadline`) for the other process that owns the engine on
/// `storage` to go. `Some(pid)` when it is still alive at the deadline; `None` once there is
/// none, or when cancelled.
pub async fn wait_owner_gone(
    storage: &Path,
    deadline: Instant,
    cancel: &CancellationToken,
) -> Option<u32> {
    loop {
        let owner = other_live_owner(storage)?;
        if Instant::now() >= deadline {
            return Some(owner);
        }
        tokio::select! {
            () = tokio::time::sleep(POLL) => {}
            () = cancel.cancelled() => return None,
        }
    }
}

/// A live process's full command line (`ps -ww -o command=`), or `None`.
pub fn process_command(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-ww", "-o", "command=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// True when `pid` is the rivet engine started on `storage` (its command carries
/// `--config <storage's config.json>`).
pub fn is_engine_of(pid: u32, storage: &Path) -> bool {
    process_command(pid).is_some_and(|command| {
        command.contains("rivet-engine")
            && command.contains(&format!(
                "--config {}",
                engine_config_path(storage).display()
            ))
    })
}

/// A first run's engine guard port: a block in [`FIRST_PORTS`] whose guard, peer and
/// metrics ports are all free, starting at a spot that depends on `seed` (the data dir) so
/// two data dirs rarely race for one block. `None` when `storage` already has a port (it
/// must keep it) or nothing is free.
pub fn first_run_port(storage: &Path, seed: &Path) -> Option<u16> {
    if recorded_engine_port(storage).is_some() {
        return None;
    }
    first_free_block(seed, loopback_port_in_use)
}

/// The first block from `seed`'s spot whose three ports `in_use` calls free.
fn first_free_block(seed: &Path, in_use: impl Fn(u16) -> bool) -> Option<u16> {
    let (lo, hi) = FIRST_PORTS;
    let blocks = (hi - lo) / PORT_STEP + 1;
    // FNV-1a over the path: stable across runs and Rust versions.
    let hash = seed
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        });
    let start = u16::try_from(hash % u64::from(blocks)).unwrap_or(0);
    (0..blocks)
        .map(|i| lo + ((start + i) % blocks) * PORT_STEP)
        .find(|port| engine_ports(*port).iter().all(|p| !in_use(*p)))
}

/// The engine's keeper outside the app: a tiny shell that waits on a pipe from the app.
/// When the app dies (even by SIGKILL) the pipe closes, and the shell stops the engine if
/// it is still the one this app started (`--config <storage's config.json>`): SIGTERM, then
/// SIGKILL after 15 s. A clean stop closes the pipe too, after the engine is gone, so the
/// shell finds nothing to do.
#[derive(Debug)]
pub struct Watchdog {
    child: Option<Child>,
}

/// The watchdog's script: `$1` the engine pid, `$2` its config path.
#[cfg(unix)]
const WATCHDOG_SCRIPT: &str = r#"
trap '' HUP INT
pid="$1"; cfg="$2"
while read -r _; do :; done
ours() {
  case "$(ps -ww -o command= -p "$pid" 2>/dev/null)" in
    *rivet-engine*"--config $cfg"*) return 0 ;;
  esac
  return 1
}
ours || exit 0
kill -TERM "$pid" 2>/dev/null
i=0
while [ "$i" -lt 15 ]; do
  ours || exit 0
  sleep 1
  i=$((i + 1))
done
ours && kill -KILL "$pid" 2>/dev/null
exit 0
"#;

impl Watchdog {
    /// Starts the watchdog for the engine `pid` on `storage` (unix; elsewhere it does
    /// nothing).
    pub fn arm(pid: u32, storage: &Path) -> Watchdog {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            let spawned = Command::new("/bin/sh")
                .arg("-c")
                .arg(WATCHDOG_SCRIPT)
                .arg("euchre-engine-watchdog")
                .arg(pid.to_string())
                .arg(engine_config_path(storage))
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                // Its own group: a terminal's Ctrl-C to the app's group never reaches it.
                .process_group(0)
                .spawn();
            match spawned {
                Ok(child) => {
                    tracing::debug!(pid, watchdog = child.id(), "engine watchdog armed");
                    Watchdog { child: Some(child) }
                }
                Err(err) => {
                    tracing::warn!(%err, "the engine watchdog could not start");
                    Watchdog { child: None }
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (pid, storage);
            Watchdog { child: None }
        }
    }

    /// The watchdog's pid (tests).
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }
}

impl Drop for Watchdog {
    /// Closes the pipe (the watchdog stops the engine only if it still runs) and reaps the
    /// shell off this thread.
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            drop(child.stdin.take());
            let _ = std::thread::Builder::new()
                .name("euchre-watchdog-reap".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_is_one_holder_at_a_time() {
        let dir = tempfile::tempdir().expect("temp dir");
        let first = try_lock(dir.path()).expect("the first copy takes it");
        assert!(matches!(try_lock(dir.path()), Err(LockError::Held)));
        drop(first);
        // A child another test forks holds a copy of the descriptor until it execs, so the
        // lock can outlive the drop by a moment.
        let again = (0..200)
            .find_map(|_| {
                try_lock(dir.path()).ok().or_else(|| {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                })
            })
            .expect("free once the first lets go");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.path().join(LOCK_FILE))
                .expect("lock file")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        drop(again);
    }

    #[tokio::test]
    async fn a_launch_waits_for_the_copy_that_is_quitting() {
        let dir = tempfile::tempdir().expect("temp dir");
        let held = try_lock(dir.path()).expect("held");
        let cancel = CancellationToken::new();
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            drop(held);
        });
        let began = Instant::now();
        let got = lock_within(dir.path(), Duration::from_secs(5), &cancel)
            .await
            .expect("not cancelled");
        assert!(got.is_ok(), "taken once released: {got:?}");
        assert!(began.elapsed() >= Duration::from_millis(300));
        release.await.expect("release");

        // Held past the wait: it says so.
        let _held = got.expect("lock");
        let got = lock_within(dir.path(), Duration::from_millis(300), &cancel)
            .await
            .expect("not cancelled");
        assert!(matches!(got, Err(LockError::Held)));
        cancel.cancel();
        assert!(
            lock_within(dir.path(), Duration::from_secs(5), &cancel)
                .await
                .is_none()
        );
    }

    #[test]
    fn first_ports_stay_in_range_and_skip_busy_blocks() {
        let seed = Path::new("/data/euchre");
        let free = first_free_block(seed, |_| false).expect("a block");
        assert!((FIRST_PORTS.0..=FIRST_PORTS.1).contains(&free), "{free}");
        assert_eq!(free % PORT_STEP, 0);
        assert!(free + 5 <= 29_999);
        // Stable for one data dir.
        assert_eq!(first_free_block(seed, |_| false), Some(free));
        // A busy guard, peer or metrics port moves it to the next block.
        for busy in engine_ports(free) {
            let next = first_free_block(seed, |p| p == busy).expect("a block");
            assert_ne!(next, free, "busy {busy}");
        }
        // Wraps from the top block to the bottom one.
        let top = first_free_block(seed, |p| p < FIRST_PORTS.1).expect("the top block");
        assert_eq!(top, FIRST_PORTS.1);
        assert_eq!(first_free_block(seed, |_| true), None);
    }

    #[test]
    fn a_recorded_port_is_kept() {
        let dir = tempfile::tempdir().expect("temp dir");
        let var = engine_var_dir(dir.path());
        std::fs::create_dir_all(&var).expect("var dir");
        std::fs::write(
            var.join(euchre_server_core::rivet::ENGINE_PORT_FILE),
            "6420\n",
        )
        .expect("port file");
        assert_eq!(first_run_port(dir.path(), dir.path()), None);
    }

    #[test]
    fn the_engine_owner_is_read_from_the_pid_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(other_live_owner(dir.path()), None, "no pid file");
        let var = engine_var_dir(dir.path());
        std::fs::create_dir_all(&var).expect("var dir");
        let pid_file = var.join(ENGINE_PID_FILE);
        // This process owns it: not another copy.
        std::fs::write(&pid_file, format!("1 {}\n", std::process::id())).expect("pid");
        assert_eq!(other_live_owner(dir.path()), None);
        assert_eq!(
            recorded_engine(dir.path()),
            Some((1, Some(std::process::id())))
        );
        // A live other process (a child we start and stop).
        let mut child = Command::new("/bin/sleep").arg("30").spawn().expect("sleep");
        std::fs::write(&pid_file, format!("1 {}\n", child.id())).expect("pid");
        assert_eq!(other_live_owner(dir.path()), Some(child.id()));
        child.kill().expect("kill our sleep");
        child.wait().expect("reap");
        assert_eq!(other_live_owner(dir.path()), None, "a dead owner");
        // An old file without the owner field.
        std::fs::write(&pid_file, "1\n").expect("pid");
        assert_eq!(other_live_owner(dir.path()), None);
        assert_eq!(recorded_engine(dir.path()), Some((1, None)));
    }

    /// The watchdog stops only a process whose command carries `rivet-engine` and this
    /// storage's `--config`, and only once its pipe closes.
    #[cfg(unix)]
    #[test]
    fn the_watchdog_stops_its_engine_when_the_pipe_closes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = engine_config_path(dir.path());
        // A stand-in engine: a shell loop whose argv carries `rivet-engine --config <cfg>`
        // (a loop, so the shell never execs away from that argv). It ends within 0.1 s of a
        // SIGTERM.
        let fake = |config: &Path| {
            Command::new("/bin/sh")
                .arg("-c")
                .arg("trap 'exit 0' TERM; while :; do sleep 0.1; done")
                .arg("rivet-engine")
                .arg("--config")
                .arg(config)
                .stdin(Stdio::null())
                .spawn()
                .expect("stand-in")
        };
        let gone = |child: &mut Child| {
            let end = Instant::now() + Duration::from_secs(10);
            while Instant::now() < end {
                if child.try_wait().expect("try_wait").is_some() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            false
        };

        let mut ours = fake(&config);
        assert!(
            process_command(ours.id())
                .is_some_and(|c| c.contains("rivet-engine") && c.contains("--config")),
            "the stand-in carries the engine's command"
        );
        let watchdog = Watchdog::arm(ours.id(), dir.path());
        assert!(watchdog.pid().is_some());
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            ours.try_wait().expect("try_wait").is_none(),
            "armed: no signal"
        );
        drop(watchdog);
        assert!(gone(&mut ours), "the pipe closed: the engine is stopped");

        // Another storage's engine is never touched.
        let other = tempfile::tempdir().expect("temp dir");
        let mut theirs = fake(&engine_config_path(other.path()));
        drop(Watchdog::arm(theirs.id(), dir.path()));
        std::thread::sleep(Duration::from_millis(800));
        assert!(
            theirs.try_wait().expect("try_wait").is_none(),
            "not ours: left alone"
        );
        theirs.kill().expect("kill our stand-in");
        theirs.wait().expect("reap");
    }
}
