//! The embedded game server end to end (contract §N2.10): a real engine and server on the
//! lane's port block, driven through the real bridge thread. Skipped unless `EUCHRE_E2E=1`.
//!
//! - `EUCHRE_TEST_ENGINE_PORT`: the engine port `P` (default 6860; the engine also binds P+1
//!   and P+5).
//! - `EUCHRE_TEST_TMP`: the root for the run's data, `<root>/n2-<port>` (printed; kept when the
//!   test fails, removed when it passes). A run that was killed leaves it behind: the next run
//!   on the same port stops the engine it left (only one whose command line names that
//!   root's storage) and starts clean.
//! - `EUCHRE_LIVE_FIXTURE_OUT`: when set, the real views and saved tables are written there as
//!   a `scenes::LiveFixture` for the `live-*` scenes.
//!
//! `first_launch_offline_reads_engine_missing` is a second, separate opt-in
//! (`EUCHRE_E2E_OFFLINE=1`): the runner points `HOME` at an empty dir (so rivetkit's engine
//! cache is empty) and the proxies at a closed loopback port (so the download fails at once).
//!
//! `an_engine_stops_when_its_app_is_killed` (review fix LDS-1) runs this test binary again as
//! the app (`engine_owner_child`, on `P+2`), SIGKILLs that child once its engine serves, and
//! checks the watchdog stops the engine and frees its ports.
//!
//! It never touches the player's data dir and only ever signals the engine it started: every
//! signal is checked against the engine's command line first (review fix LDS-7).

use std::io::{BufRead as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use euchre_desktop::net::local;

use euchre_desktop::net::bridge::{Bridge, BridgeConfig, BridgeThread, Intent, Reply, UiSink};
use euchre_desktop::net::embedded::{EmbeddedBackend, EmbeddedConfig};
use euchre_desktop::net::{BootProblem, ConnEvent, NetError, ServerPhase, TableConfig};
use euchre_desktop::scenes::LiveFixture;
use euchre_protocol::{PublicGameView, TableEvent};

const BOOT: Duration = Duration::from_secs(120);
const STOP_NOTICED: Duration = Duration::from_secs(15);
const PORTS_CLOSE: Duration = Duration::from_secs(5);
const QUIT: Duration = Duration::from_secs(45);
const PLAY: Duration = Duration::from_secs(90);

/// One app's worth of bridge: the real thread over an embedded backend.
struct Live {
    bridge: Option<Bridge>,
    thread: Option<BridgeThread>,
    replies: Receiver<Reply>,
    backend: EmbeddedBackend,
    /// Every engine pid this run started, for the clean-up on a failure.
    pids: Vec<u32>,
    /// The engine's storage: a pid is signalled only while its command line names it.
    storage: PathBuf,
}

impl Live {
    fn start(data_dir: &Path, port: u16) -> Live {
        let config = EmbeddedConfig::for_tests(data_dir.to_path_buf(), port);
        let storage = config.engine_storage();
        let backend = EmbeddedBackend::new(config);
        let (bridge, intents) = Bridge::channel();
        let (sink, replies) = UiSink::channel();
        let thread = BridgeThread::spawn(
            BridgeConfig {
                backend: std::sync::Arc::new(backend.clone()),
                sink,
                prefs: None,
                quit_on_signals: false,
            },
            intents,
        )
        .expect("bridge thread");
        Live {
            bridge: Some(bridge),
            thread: Some(thread),
            replies,
            backend,
            pids: Vec::new(),
            storage,
        }
    }

    fn send(&self, intent: Intent) {
        if let Some(bridge) = &self.bridge {
            bridge.send(intent);
        }
    }

    /// The next reply `pick` takes, within `limit`; the others are dropped.
    fn wait<T>(
        &mut self,
        what: &str,
        limit: Duration,
        mut pick: impl FnMut(Reply) -> Option<T>,
    ) -> T {
        let end = Instant::now() + limit;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "timed out waiting for {what}");
            match self
                .replies
                .recv_timeout(left.min(Duration::from_millis(250)))
            {
                Ok(reply) => {
                    if let Some(t) = pick(reply) {
                        return t;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(err) => panic!("bridge gone while waiting for {what}: {err}"),
            }
        }
    }

    /// Waits for `phase`, collecting the phases on the way.
    fn wait_phase(
        &mut self,
        what: &str,
        limit: Duration,
        done: impl Fn(&ServerPhase) -> bool,
    ) -> Vec<ServerPhase> {
        let mut seen = Vec::new();
        self.wait(what, limit, |reply| match reply {
            Reply::Server(phase) => {
                let stop = done(&phase);
                seen.push(phase);
                stop.then_some(())
            }
            _ => None,
        });
        if let Some(pid) = self.backend.supervisor().engine_pid()
            && !self.pids.contains(&pid)
        {
            self.pids.push(pid);
        }
        seen
    }

    fn url(&self) -> String {
        self.backend
            .remote()
            .and_then(|r| r.config().map(|c| c.base_url.clone()))
            .expect("a server url")
    }

    /// The first sync on `conn_id` where it is the player's turn (the table waits there).
    fn my_turn(&mut self, conn_id: u64) -> PublicGameView {
        self.wait("the player's turn", PLAY, |reply| match reply {
            Reply::Conn {
                conn_id: c,
                event: ConnEvent::Event(TableEvent::Sync(sync)),
            } if c == conn_id && !sync.view.legal.is_empty() => Some(sync.view),
            Reply::Conn {
                conn_id: c,
                event: ConnEvent::Ended(err),
            } if c == conn_id => panic!("table ended: {err:?}"),
            _ => None,
        })
    }

    /// Sends `Shutdown` and waits for the bridge to end; how long it took.
    fn stop(&mut self) -> Duration {
        let started = Instant::now();
        self.send(Intent::Shutdown);
        self.bridge = None;
        let ended = self.thread.take().is_none_or(|t| t.join_within(QUIT));
        assert!(ended, "the bridge ended within {QUIT:?}");
        started.elapsed()
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.send(Intent::Shutdown);
            self.bridge = None;
            if let Some(t) = self.thread.take() {
                let _ = t.join_within(QUIT);
            }
        }
        // Only ever the engines this run started, only if they outlived the stop, and only
        // while the pid is still that engine (a pid can be reused).
        for pid in &self.pids {
            if local::is_engine_of(*pid, &self.storage) {
                eprintln!("engine {pid} outlived the run; stopping it");
                let _ = Command::new("kill")
                    .args(["-TERM", &pid.to_string()])
                    .status();
            }
        }
    }
}

/// The engine storage of the embedded server on `data`.
fn storage_of(data: &Path) -> PathBuf {
    EmbeddedConfig::for_tests(data.to_path_buf(), 0).engine_storage()
}

/// Stops the engine an earlier, killed run left on `root`'s storage (only one whose command
/// line names that storage, and whose owner is gone), then clears `root` (review fix LDS-5).
fn reclaim(root: &Path) {
    let storage = storage_of(&root.join("data"));
    if let Some((engine, _)) = local::recorded_engine(&storage)
        && local::other_live_owner(&storage).is_none()
        && local::is_engine_of(engine, &storage)
    {
        eprintln!("stopping engine {engine}, left by an earlier run");
        let _ = Command::new("kill")
            .args(["-TERM", &engine.to_string()])
            .status();
        let deadline = Instant::now() + Duration::from_secs(15);
        while local::is_engine_of(engine, &storage) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(200),
    )
    .is_ok()
}

fn http_port(url: &str) -> u16 {
    url.rsplit(':')
        .next()
        .and_then(|p| p.trim_end_matches('/').parse().ok())
        .expect("a loopback url with a port")
}

/// Waits until none of `ports` answers and `pid` is gone.
fn all_closed(pid: u32, ports: &[u16]) -> Duration {
    let started = Instant::now();
    while alive(pid) || ports.iter().any(|&p| listening(p)) {
        assert!(
            started.elapsed() < PORTS_CLOSE,
            "engine {pid} alive: {}; listening: {:?}",
            alive(pid),
            ports.iter().filter(|&&p| listening(p)).collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    started.elapsed()
}

fn ms(d: Duration) -> u128 {
    d.as_millis()
}

struct Root(PathBuf);

impl Root {
    /// `<base>/<name>`, reclaimed from an earlier run that was killed.
    fn claim(base: &Path, name: &str) -> Root {
        let root = base.join(name);
        reclaim(&root);
        Root(root)
    }
}

fn base_dir() -> PathBuf {
    std::env::var_os("EUCHRE_TEST_TMP")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn base_port() -> u16 {
    std::env::var("EUCHRE_TEST_ENGINE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(6860)
}

impl Drop for Root {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("kept for a look: {}", self.0.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[test]
fn embedded_server_boots_restarts_and_stops() {
    if std::env::var("EUCHRE_E2E").as_deref() != Ok("1") {
        eprintln!("skipped: set EUCHRE_E2E=1 to boot a real engine");
        return;
    }
    // The app's milestones (boot_ms, stop_ms) and warnings, on stderr.
    euchre_desktop::startup::init_logging(None);
    let port = base_port();
    let root = Root::claim(&base_dir(), &format!("n2-{port}"));
    let data = root.0.join("data");
    std::fs::create_dir_all(&data).expect("data dir");
    println!("n2 live root: {}", root.0.display());
    let engine_ports = [port, port + 1, port + 5];
    for p in engine_ports {
        assert!(!listening(p), "port {p} is free before the run");
    }

    // 1. Starting, then Ready.
    let boot_started = Instant::now();
    let mut app = Live::start(&data, port);
    let seen = app.wait_phase("Ready", BOOT, |p| *p == ServerPhase::Ready);
    let boot_cold = boot_started.elapsed();
    assert!(
        seen.iter()
            .any(|p| matches!(p, ServerPhase::Starting | ServerPhase::Downloading)),
        "{seen:?}"
    );
    let url = app.url();
    let pid = app
        .backend
        .supervisor()
        .engine_pid()
        .expect("an engine pid");
    println!("ready at {url} (engine {pid}) in {} ms", ms(boot_cold));

    // 2. Who is playing; the device record is private.
    app.send(Intent::Identify);
    let identity = app.wait("identity", BOOT, |r| match r {
        Reply::Identity(result) => Some(result),
        _ => None,
    });
    assert!(identity.is_ok(), "{identity:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(data.join("device.json"))
            .expect("device.json")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // 3. A table, open, at the player's turn.
    app.send(Intent::CreateTable {
        conn_id: 1,
        cfg: TableConfig::default(),
    });
    let game_id = app.wait("TableCreated", BOOT, |r| match r {
        Reply::TableCreated { conn_id: 1, result } => Some(result.expect("created")),
        _ => None,
    });
    let before = app.my_turn(1);
    println!(
        "table {game_id}: hand {} turn {} score {:?}",
        before.hand_no, before.turn_id, before.score
    );

    // 4. The engine dies: the server says Stopped, the table ends with ServerStopped.
    assert!(
        local::is_engine_of(pid, &storage_of(&data)),
        "{pid} is our engine"
    );
    let killed = Instant::now();
    let status = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("kill");
    assert!(status.success());
    let (mut stopped, mut ended) = (None, None);
    app.wait("Stopped and Ended", STOP_NOTICED, |r| {
        match r {
            Reply::Server(ServerPhase::Stopped) => stopped = Some(killed.elapsed()),
            Reply::Conn {
                conn_id: 1,
                event: ConnEvent::Ended(err),
            } => ended = Some(err),
            _ => {}
        }
        (stopped.is_some() && ended.is_some()).then_some(())
    });
    assert_eq!(ended, Some(NetError::ServerStopped));
    let noticed = stopped.expect("stopped");
    println!("engine death noticed in {} ms", ms(noticed));

    // 5. Restart: Starting, Ready at the same URL, and the table where it was.
    let restarted = Instant::now();
    app.send(Intent::RestartServer);
    let seen = app.wait_phase("Ready again", BOOT, |p| *p == ServerPhase::Ready);
    let boot_warm = restarted.elapsed();
    assert_eq!(seen.first(), Some(&ServerPhase::Starting), "{seen:?}");
    assert_eq!(app.url(), url, "the same url");
    let pid = app
        .backend
        .supervisor()
        .engine_pid()
        .expect("a new engine pid");
    app.send(Intent::OpenTable {
        conn_id: 2,
        game_id: game_id.clone(),
    });
    let after = app.wait("the reopened table", BOOT, |r| match r {
        Reply::Conn {
            conn_id: 2,
            event: ConnEvent::Event(TableEvent::Sync(sync)),
        } => Some(sync.view),
        Reply::TableOpened {
            conn_id: 2,
            result: Err(err),
        } => panic!("reopen failed: {err:?}"),
        _ => None,
    });
    assert_eq!(
        (after.hand_no, &after.turn_id, after.score),
        (before.hand_no, &before.turn_id, before.score),
        "the same turn"
    );
    println!("restarted in {} ms; table resumed", ms(boot_warm));

    if let Some(out) = std::env::var_os("EUCHRE_LIVE_FIXTURE_OUT") {
        app.send(Intent::RecentTables);
        let recent = app.wait("recent tables", BOOT, |r| match r {
            Reply::RecentTables(result) => Some(result.ok()),
            _ => None,
        });
        app.send(Intent::ListGames { cursor: None });
        let games = app.wait("games", BOOT, |r| match r {
            Reply::Games { result, .. } => Some(result.ok()),
            _ => None,
        });
        let fixture = LiveFixture {
            views: vec![before.clone(), after.clone()],
            games,
            recent,
            ..LiveFixture::default()
        };
        std::fs::write(&out, serde_json::to_vec_pretty(&fixture).expect("json")).expect("fixture");
        println!("live fixture: {}", PathBuf::from(out).display());
    }

    // 6. Quit: the bridge ends, the engine is gone and every port closes.
    let http = http_port(&url);
    let quit = app.stop();
    let mut ports = engine_ports.to_vec();
    ports.push(http);
    let closed = all_closed(pid, &ports);
    println!(
        "shutdown in {} ms (+{} ms until every port closed)",
        ms(quit),
        ms(closed)
    );
    drop(app);

    // 7. Another program on the engine port: PortInUse; once it is free, Try again serves.
    let squatter = TcpListener::bind(("127.0.0.1", port)).expect("hold the engine port");
    let failed_started = Instant::now();
    let mut app = Live::start(&data, port);
    let seen = app.wait_phase("PortInUse", BOOT, |p| matches!(p, ServerPhase::Failed(_)));
    assert_eq!(
        seen.last(),
        Some(&ServerPhase::Failed(BootProblem::PortInUse { port })),
        "{seen:?}"
    );
    let port_failed = failed_started.elapsed();
    drop(squatter);
    let retried = Instant::now();
    app.send(Intent::RestartServer);
    app.wait_phase("Ready after the port freed", BOOT, |p| {
        *p == ServerPhase::Ready
    });
    let boot_retry = retried.elapsed();
    let pid = app
        .backend
        .supervisor()
        .engine_pid()
        .expect("an engine pid");
    let http = http_port(&app.url());
    let quit2 = app.stop();
    let mut ports = engine_ports.to_vec();
    ports.push(http);
    let closed2 = all_closed(pid, &ports);
    drop(app);
    println!(
        "N2 timings: boot_cold_ms={} death_noticed_ms={} restart_ready_ms={} shutdown_ms={} \
         ports_closed_after_ms={} port_in_use_ms={} retry_ready_ms={} shutdown2_ms={} \
         ports_closed2_after_ms={}",
        ms(boot_cold),
        ms(noticed),
        ms(boot_warm),
        ms(quit),
        ms(closed),
        ms(port_failed),
        ms(boot_retry),
        ms(quit2),
        ms(closed2)
    );
}

/// The child's data dir; set only when the parent test runs it.
const OWNER_ENV: &str = "EUCHRE_E2E_OWNER_DATA";

/// Review fix LDS-1: the app dying without quitting (SIGKILL) takes its engine with it. The
/// parent runs this binary again as the app ([`engine_owner_child`]), kills it once the
/// engine serves, and waits for the engine to go and its ports to free.
#[test]
fn an_engine_stops_when_its_app_is_killed() {
    if std::env::var("EUCHRE_E2E").as_deref() != Ok("1") {
        eprintln!("skipped: set EUCHRE_E2E=1 to boot a real engine");
        return;
    }
    // Beside the other test's block: P+2, P+3 and P+7.
    let port = base_port() + 2;
    let root = Root::claim(&base_dir(), &format!("n2-owner-{port}"));
    let data = root.0.join("data");
    std::fs::create_dir_all(&data).expect("data dir");
    let storage = storage_of(&data);
    let engine_ports = [port, port + 1, port + 5];
    for p in engine_ports {
        assert!(!listening(p), "port {p} is free before the run");
    }
    let child = Command::new(std::env::current_exe().expect("this test binary"))
        .args([
            "engine_owner_child",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env(OWNER_ENV, &data)
        .env("EUCHRE_TEST_ENGINE_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the owner app");
    // Killed and reaped on every way out, so a failed assert never leaves it running.
    let mut child = Owner(child);
    let owner = child.0.id();
    let stdout = child.0.stdout.take().expect("its stdout");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            // libtest prints "test engine_owner_child ... " before the test runs, so the
            // pid can share that line.
            if let Some(pid) = line
                .rsplit_once("ENGINE ")
                .and_then(|(_, p)| p.trim().parse().ok())
            {
                let _ = tx.send(pid);
            }
        }
    });
    let engine: u32 = match rx.recv_timeout(BOOT + Duration::from_secs(10)) {
        Ok(pid) => pid,
        Err(err) => panic!("the owner app never served: {err}"),
    };
    assert!(
        local::is_engine_of(engine, &storage),
        "{engine} is the child's engine"
    );
    assert_eq!(
        local::recorded_engine(&storage),
        Some((engine, Some(owner))),
        "the child owns it"
    );
    let killed = Instant::now();
    child.0.kill().expect("SIGKILL the owner app");
    child.0.wait().expect("reap it");
    let deadline = killed + Duration::from_secs(20);
    while (local::is_engine_of(engine, &storage) || engine_ports.iter().any(|&p| listening(p)))
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    let gone = killed.elapsed();
    let left = local::is_engine_of(engine, &storage);
    let open: Vec<u16> = engine_ports.into_iter().filter(|&p| listening(p)).collect();
    if left {
        let _ = Command::new("kill")
            .args(["-TERM", &engine.to_string()])
            .status();
    }
    assert!(!left, "engine {engine} outlived its app by {gone:?}");
    assert!(open.is_empty(), "still listening: {open:?}");
    println!(
        "N2 owner killed: engine gone and ports free in {} ms",
        ms(gone)
    );
}

/// The owner app child process; killed and reaped when dropped.
struct Owner(std::process::Child);

impl Drop for Owner {
    fn drop(&mut self) {
        // Our own child, by its handle: never a pid lookup.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The app for [`an_engine_stops_when_its_app_is_killed`]: boots the embedded server on
/// `$EUCHRE_E2E_OWNER_DATA`, prints its engine pid, then waits to be killed. Does nothing
/// unless the parent runs it.
#[test]
#[ignore = "run by an_engine_stops_when_its_app_is_killed"]
fn engine_owner_child() {
    let Some(data) = std::env::var_os(OWNER_ENV).map(PathBuf::from) else {
        return;
    };
    euchre_desktop::startup::init_logging(None);
    let mut app = Live::start(&data, base_port());
    app.wait_phase("Ready", BOOT, |p| *p == ServerPhase::Ready);
    let pid = app
        .backend
        .supervisor()
        .engine_pid()
        .expect("an engine pid");
    println!("\nENGINE {pid}");
    let _ = std::io::stdout().flush();
    // The parent kills this process long before; if it never does, quit cleanly.
    std::thread::sleep(Duration::from_secs(180));
}

/// The first launch on a machine with no engine and no network: Downloading, then the
/// "connect to the internet" problem, and no engine ever starts.
#[test]
fn first_launch_offline_reads_engine_missing() {
    if std::env::var("EUCHRE_E2E_OFFLINE").as_deref() != Ok("1") {
        eprintln!("skipped: set EUCHRE_E2E_OFFLINE=1 (with HOME on an empty dir) to try it");
        return;
    }
    euchre_desktop::startup::init_logging(None);
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    assert!(
        !home.join(".rivetkit").join("engine").exists(),
        "HOME ({}) must have no rivetkit engine cache",
        home.display()
    );
    let port = base_port();
    let root = Root::claim(&base_dir(), &format!("n2-offline-{port}"));
    let data = root.0.join("data");
    std::fs::create_dir_all(&data).expect("data dir");

    let began = Instant::now();
    let mut app = Live::start(&data, port);
    let seen = app.wait_phase("a failed first start", BOOT, |p| {
        matches!(p, ServerPhase::Failed(_) | ServerPhase::Ready)
    });
    let failed = began.elapsed();
    assert!(seen.contains(&ServerPhase::Downloading), "{seen:?}");
    assert_eq!(
        seen.last(),
        Some(&ServerPhase::Failed(BootProblem::EngineMissing)),
        "{seen:?}"
    );
    assert_eq!(app.backend.supervisor().engine_pid(), None);
    let quit = app.stop();
    drop(app);
    for p in [port, port + 1, port + 5] {
        assert!(!listening(p), "port {p} stays closed");
    }
    println!(
        "N2 offline first launch: phases={seen:?} failed_ms={} shutdown_ms={}",
        ms(failed),
        ms(quit)
    );
}
