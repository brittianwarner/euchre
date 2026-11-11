//! What the process does before anything else (contract §N2.7, §N2.8): keep loopback off any
//! proxy, then start the log.
//!
//! - [`reexec_with_no_proxy`] runs first in `main`, before any thread, runtime, logger or GPU
//!   warm-up. The reqwest clients inside rivetkit-client and the server's engine client read
//!   the system proxy settings, so a configured proxy would capture the loopback calls to the
//!   engine and the `/rivet` gateway. `std::env::set_var` is `unsafe` in edition 2024 and the
//!   workspace forbids unsafe code, so when `NO_PROXY` lacks a loopback name the process
//!   replaces itself (`exec`, same pid, same args) with the merged list set.
//! - [`init_logging`]: `RUST_LOG`, else `warn,euchre_desktop=info`. A window run appends to
//!   `<data_dir>/logs/euchre.log` (rotated past 5 MiB, mode 0600) and also writes to stderr when
//!   it is a terminal; a scene or headless run writes to stderr only and creates nothing.

use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

/// Set on the re-executed process, so it never re-executes again.
pub const REEXEC_ENV: &str = "EUCHRE_REEXEC";
/// The loopback names every run keeps off a proxy.
pub const LOOPBACK: [&str; 3] = ["127.0.0.1", "localhost", "::1"];
/// The log directory under the data dir.
pub const LOG_DIR: &str = "logs";
/// The log file.
pub const LOG_FILE: &str = "euchre.log";
/// The previous log, after a rotation.
pub const LOG_FILE_OLD: &str = "euchre.log.1";
/// A log over this size is rotated at start.
pub const LOG_ROTATE_BYTES: u64 = 5 << 20;
/// The filter when `RUST_LOG` is unset: quiet, with the app's own milestones.
pub const DEFAULT_FILTER: &str = "warn,euchre_desktop=info";

/// A note from before the logger existed (the re-exec failed), logged once it starts.
static EARLY_NOTE: OnceLock<String> = OnceLock::new();

/// Whether the embedded server's engine link is meant to be up (the server is ready). While
/// it is not (booting, stopping, stopped), the engine client's own connection errors are
/// expected noise and stay out of the log (review fix LDS-10); the supervisor logs one calm
/// line of its own instead.
static ENGINE_LINK_EXPECTED: AtomicBool = AtomicBool::new(false);

/// The log target prefix of the engine client inside the embedded server.
const ENVOY_TARGET: &str = "rivet_envoy_client";

/// Marks the embedded server's engine link as expected (`true`, while ready) or not.
pub fn set_engine_link_expected(expected: bool) {
    ENGINE_LINK_EXPECTED.store(expected, Ordering::Relaxed);
}

/// Whether a log event from `target` is kept: everything except the engine client's, which
/// is kept only while its link is expected.
pub fn log_target_kept(target: &str) -> bool {
    target_kept(target, ENGINE_LINK_EXPECTED.load(Ordering::Relaxed))
}

/// [`log_target_kept`] with the link's state given.
fn target_kept(target: &str, link_expected: bool) -> bool {
    !target.starts_with(ENVOY_TARGET) || link_expected
}

/// The `NO_PROXY` list with `127.0.0.1`, `localhost` and `::1` added to `existing` (the
/// entries of `NO_PROXY` and `no_proxy`, comma separated); `None` when nothing is missing
/// (a `*` entry already covers everything).
pub fn merged_no_proxy(existing: Option<&str>) -> Option<String> {
    let mut entries: Vec<&str> = Vec::new();
    for entry in existing.unwrap_or_default().split(',').map(str::trim) {
        if !entry.is_empty() && !entries.iter().any(|e| e.eq_ignore_ascii_case(entry)) {
            entries.push(entry);
        }
    }
    if entries.contains(&"*") {
        return None;
    }
    let missing: Vec<&str> = LOOPBACK
        .into_iter()
        .filter(|host| !entries.iter().any(|e| e.eq_ignore_ascii_case(host)))
        .collect();
    if missing.is_empty() {
        return None;
    }
    entries.extend(missing);
    Some(entries.join(","))
}

/// Re-executes this process with loopback added to `NO_PROXY` and `no_proxy`, unless they
/// already have it, this already is the re-executed process, or this is a scene or headless
/// run (those never touch the network). On success it never returns. If `exec` fails (or off
/// unix) it notes a warning for the log and the run goes on: the desktop's own loopback
/// clients bypass proxies anyway.
pub fn reexec_with_no_proxy() {
    if std::env::var_os(REEXEC_ENV).is_some() || scene_run() {
        return;
    }
    let existing: Vec<String> = ["NO_PROXY", "no_proxy"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|v| v.to_string_lossy().into_owned())
        .collect();
    let Some(merged) = merged_no_proxy(Some(&existing.join(","))) else {
        return;
    };
    let note = reexec(&merged);
    let _ = EARLY_NOTE.set(note);
}

/// `exec`s the current executable with the same arguments and `merged` set; returns why it
/// could not.
#[cfg(unix)]
fn reexec(merged: &str) -> String {
    use std::os::unix::process::CommandExt;
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => return format!("NO_PROXY not set: no current executable ({err})"),
    };
    let err = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env("NO_PROXY", merged)
        .env("no_proxy", merged)
        .env(REEXEC_ENV, "1")
        .exec();
    format!("NO_PROXY not set: re-exec failed ({err})")
}

#[cfg(not(unix))]
fn reexec(_merged: &str) -> String {
    "NO_PROXY not set: this platform cannot re-exec".to_owned()
}

/// A headless render or a `--scene` run (`rune::is_headless`, or `--scene` in the args).
pub fn scene_run() -> bool {
    rune::is_headless() || std::env::args().any(|a| a == "--scene")
}

/// Starts the log. `log_dir_of`: the data dir of a window run (the file goes in its `logs/`),
/// `None` for a scene or headless run (stderr only). A file that cannot be opened falls back to
/// stderr, with a warning.
pub fn init_logging(log_dir_of: Option<&Path>) {
    let filter =
        || EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let quiet_envoy = || {
        tracing_subscriber::filter::filter_fn(|meta: &tracing::Metadata<'_>| {
            log_target_kept(meta.target())
        })
    };
    let opened = log_dir_of.map(open_log);
    match opened {
        Some(Ok(file)) => {
            let to_file = fmt::layer()
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(file));
            let to_terminal = io::stderr()
                .is_terminal()
                .then(|| fmt::layer().with_writer(io::stderr));
            let _ = tracing_subscriber::registry()
                .with(filter())
                .with(quiet_envoy())
                .with(to_file)
                .with(to_terminal)
                .try_init();
        }
        other => {
            let _ = tracing_subscriber::registry()
                .with(filter())
                .with(quiet_envoy())
                .with(fmt::layer().with_writer(io::stderr))
                .try_init();
            if let Some(Err(err)) = other {
                tracing::warn!(%err, "the log file could not be opened; logging to stderr");
            }
        }
    }
    if let Some(note) = EARLY_NOTE.get() {
        tracing::warn!("{note}");
    }
}

/// `<data_dir>/logs/euchre.log`, opened for append (0600; the dirs 0700). A file over
/// [`LOG_ROTATE_BYTES`] is renamed to `euchre.log.1` first, replacing the old one.
pub fn open_log(data_dir: &Path) -> io::Result<File> {
    let dir = data_dir.join(LOG_DIR);
    create_private_dir(&dir)?;
    let path = dir.join(LOG_FILE);
    if fs::metadata(&path).is_ok_and(|m| m.len() > LOG_ROTATE_BYTES) {
        fs::rename(&path, dir.join(LOG_FILE_OLD))?;
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// The log file's path for `data_dir`.
pub fn log_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LOG_DIR).join(LOG_FILE)
}

/// `create_dir_all` with owner-only permissions on unix: every dir it creates is 0700. An
/// existing dir keeps its mode (see [`make_private`]).
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Creates `dir` 0700, or narrows an existing one to 0700 (unix): for dirs only Euchre uses,
/// such as the server's.
pub fn make_private(dir: &Path) -> io::Result<()> {
    create_private_dir(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir)?.permissions().mode();
        if mode & 0o077 != 0 {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

/// What Euchre keeps in its data dir; a dir holding anything else (a data dir set to a
/// shared folder) keeps its mode.
const DATA_DIR_NAMES: [&str; 8] = [
    "logs",
    "server",
    "remote",
    "device",
    "recent-tables",
    "euchre",
    "prefs",
    ".DS_Store",
];

/// The data dir and the server's dir in it, 0700 (review fix LDS-4): the device session and
/// the server's keys live there. The data dir itself is narrowed only when it holds nothing
/// but Euchre's own files.
pub fn secure_data_dir(data_dir: &Path) -> io::Result<()> {
    create_private_dir(data_dir)?;
    let only_ours = fs::read_dir(data_dir)?.all(|entry| {
        entry.is_ok_and(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            DATA_DIR_NAMES.iter().any(|ours| name.starts_with(ours))
        })
    });
    if only_ours {
        make_private(data_dir)?;
    }
    make_private(&data_dir.join(crate::net::embedded::SERVER_DIR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merged_no_proxy_adds_only_what_is_missing() {
        assert_eq!(
            merged_no_proxy(None).as_deref(),
            Some("127.0.0.1,localhost,::1")
        );
        assert_eq!(
            merged_no_proxy(Some("")).as_deref(),
            Some("127.0.0.1,localhost,::1")
        );
        assert_eq!(
            merged_no_proxy(Some(" corp.example , LOCALHOST")).as_deref(),
            Some("corp.example,LOCALHOST,127.0.0.1,::1")
        );
        // NO_PROXY and no_proxy joined: duplicates collapse.
        assert_eq!(
            merged_no_proxy(Some("127.0.0.1,localhost,127.0.0.1")).as_deref(),
            Some("127.0.0.1,localhost,::1")
        );
        assert_eq!(merged_no_proxy(Some("127.0.0.1,localhost,::1")), None);
        assert_eq!(merged_no_proxy(Some("::1,LocalHost,127.0.0.1,.corp")), None);
        assert_eq!(merged_no_proxy(Some("*")), None);
    }

    #[test]
    fn engine_client_logs_only_while_its_link_is_expected() {
        assert!(!target_kept(
            "rivet_envoy_client::connection::native",
            false
        ));
        assert!(target_kept("euchre_desktop::net::embedded", false));
        assert!(target_kept("rivetkit_client", false));
        assert!(target_kept("rivet_envoy_client::connection::native", true));
    }

    #[cfg(unix)]
    #[test]
    fn data_and_server_dirs_end_up_private() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).expect("meta").permissions().mode() & 0o777;
        let root = tempfile::tempdir().expect("temp dir");
        // An older data dir, made 0755 with a 0755 server dir and the usual files.
        let data = root.path().join("data");
        fs::create_dir_all(data.join("server/engine")).expect("dirs");
        fs::write(data.join("device.json"), b"{}").expect("device");
        fs::write(data.join("recent-tables.json"), b"[]").expect("recent");
        for dir in [&data, &data.join("server")] {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        secure_data_dir(&data).expect("secured");
        assert_eq!(mode(&data), 0o700);
        assert_eq!(mode(&data.join("server")), 0o700);
        // A fresh one is created private.
        let fresh = root.path().join("fresh");
        secure_data_dir(&fresh).expect("created");
        assert_eq!(mode(&fresh), 0o700);
        assert_eq!(mode(&fresh.join("server")), 0o700);
        // A data dir set to a shared folder keeps its mode; the server dir is still private.
        let shared = root.path().join("shared");
        fs::create_dir_all(&shared).expect("shared");
        fs::write(shared.join("notes.txt"), b"mine").expect("other file");
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o755)).expect("chmod");
        secure_data_dir(&shared).expect("secured");
        assert_eq!(mode(&shared), 0o755);
        assert_eq!(mode(&shared.join("server")), 0o700);
    }

    #[test]
    fn log_file_rotates_past_five_mib() {
        let root = tempfile::tempdir().expect("temp dir");
        let data = root.path().join("data");
        // First open: creates the dirs and an empty file.
        drop(open_log(&data).expect("open"));
        let path = log_path(&data);
        assert_eq!(fs::metadata(&path).expect("log").len(), 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(&data.join(LOG_DIR)), 0o700);
        }
        // At the limit: kept and appended to.
        let limit = usize::try_from(LOG_ROTATE_BYTES).expect("fits");
        fs::write(&path, vec![b'a'; limit]).expect("fill");
        {
            use std::io::Write;
            let mut f = open_log(&data).expect("open");
            f.write_all(b"b").expect("append");
        }
        assert_eq!(
            fs::metadata(&path).expect("log").len(),
            LOG_ROTATE_BYTES + 1
        );
        assert!(!data.join(LOG_DIR).join(LOG_FILE_OLD).exists());
        // Past it: moved to .1 (replacing an older one) and started fresh.
        fs::write(data.join(LOG_DIR).join(LOG_FILE_OLD), b"old").expect("old");
        drop(open_log(&data).expect("open"));
        assert_eq!(fs::metadata(&path).expect("log").len(), 0);
        assert_eq!(
            fs::metadata(data.join(LOG_DIR).join(LOG_FILE_OLD))
                .expect("rotated")
                .len(),
            LOG_ROTATE_BYTES + 1
        );
    }
}
