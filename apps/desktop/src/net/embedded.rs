//! The embedded game server (ARCHITECTURE §2.5, lane N2; contract §N2).
//!
//! [`EmbeddedBackend`] starts `euchre_server::start(ServerConfig::try_local(<data_dir>/server))`
//! on a tokio runtime of its own (one per boot, see [`ServerRuntime`]), then serves every
//! [`Backend`] call through a
//! [`RemoteBackend`] built with [`RemoteConfig::embedded`]. Its [`Supervisor`] is the
//! [`ServerHost`] the bridge boots at start, watches (`RunningServer::closed`), restarts on the
//! player's word and stops at quit, so the engine never outlives the app.
//!
//! One task owns the running server (the server task). The supervisor talks to it over a
//! channel and never holds a lock across an `.await`. Whatever might block while it stops (a
//! server or a half-started boot, whose engine `Drop` waits up to 15 s) is stopped or dropped
//! on the server's own runtime, bounded, so the bridge's workers never stall. Once a server
//! has stopped, its runtime shuts down and takes every task the server left behind with it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use euchre_profile_model::{ClientMatchReplay, ProfileSettings, ProfileStats, SettingsPatch};
use euchre_protocol::MatchListPage;
use euchre_server::{BootError, MIN_FREE_DISK_BYTES, RunningServer, ServerConfig};
use rivetkit_engine_process::{EngineResolverConfig, resolve_engine_binary_path};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::fetch::{FetchError, FetchPlan, FetchTiming, fetch_engine};
use super::local::{InstanceLock, LockError, Watchdog};
use super::remote::{RemoteBackend, RemoteConfig};
use super::{
    Backend, BootProblem, ConnEvent, Identity, NetError, RecentTables, ServerHost, ServerPhase,
    TableConfig, TableConn,
};

/// The server's own dir under the data dir (`secrets.json`, `engine/`).
pub const SERVER_DIR: &str = "server";
/// How long a backend call waits for a booting server before it answers
/// [`NetError::ServerStopped`], and how long `euchre_server::start` may take. A first launch
/// may download the engine first (that has its own per-request timeout).
pub const BOOT_WAIT: Duration = Duration::from_secs(120);
/// How long quit and restart wait for `RunningServer::shutdown` before dropping it (the drop
/// still stops the engine: SIGTERM, then SIGKILL after 15 s).
pub const STOP_BUDGET: Duration = Duration::from_secs(25);
/// How long an aborted server (or a cancelled boot) may take to stop its engine on its
/// runtime: the engine's own SIGTERM-then-SIGKILL window plus a margin.
pub const DROP_WAIT: Duration = Duration::from_secs(17);
/// How long a server whose engine already died may take to be cleared (its HTTP side and
/// its tasks): nothing is left to drain, so it is dropped rather than shut down (review fix
/// PX-5).
pub const DEAD_STOP: Duration = Duration::from_secs(2);
/// A slow-download signal: the supervisor moves to [`ServerPhase::DownloadingSlow`].
type SlowSignal = Arc<dyn Fn() + Send + Sync>;
/// The engine endpoint the binary resolver is handed. It only reads the version and the
/// releases endpoint from it; nothing is dialled.
const RESOLVER_ENDPOINT: &str = "http://127.0.0.1:6420";

/// What the embedded server needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedConfig {
    /// The app's data dir (`net::data_dir()`); the server lives in `<data_dir>/server`, the
    /// device record and recent tables in `<data_dir>` itself.
    pub data_dir: PathBuf,
    /// Tests only, on a fresh data dir: pin the engine port (`ServerConfig::with_engine_port`)
    /// inside the lane's port block. `None` (the app) lets `ServerConfig::try_local` pick a
    /// free port on first run and reuse the recorded one after.
    pub engine_port: Option<u16>,
    /// Tests only: `ServerConfig::fast_tempo` (no table pacing waits).
    pub fast_tempo: bool,
    /// Tests only: `cfg.ai = AiConfig::default()` (heuristic seats, no model calls), so a
    /// provider key in the developer's environment never reaches a test.
    pub offline_ai: bool,
    /// How long a boot waits for another copy of Euchre to let go of this data dir
    /// ([`super::local::INSTANCE_WAIT`]).
    pub instance_wait: Duration,
}

impl EmbeddedConfig {
    /// The app's configuration for `data_dir`.
    pub fn new(data_dir: PathBuf) -> EmbeddedConfig {
        EmbeddedConfig {
            data_dir,
            engine_port: None,
            fast_tempo: false,
            offline_ai: false,
            instance_wait: super::local::INSTANCE_WAIT,
        }
    }

    /// A test's configuration: a fresh `data_dir`, the engine on `engine_port` (the lane's
    /// port block; the engine binds it, +1 and +5), fast tempo, offline AI.
    pub fn for_tests(data_dir: PathBuf, engine_port: u16) -> EmbeddedConfig {
        EmbeddedConfig {
            data_dir,
            engine_port: Some(engine_port),
            fast_tempo: true,
            offline_ai: true,
            instance_wait: super::local::INSTANCE_WAIT,
        }
    }

    /// `<data_dir>/server`.
    pub fn server_dir(&self) -> PathBuf {
        self.data_dir.join(SERVER_DIR)
    }

    /// The engine's storage, `<data_dir>/server/engine` (`ServerConfig::try_local`).
    pub fn engine_storage(&self) -> PathBuf {
        self.server_dir().join("engine")
    }

    /// The remote configuration for the server once it serves at `url`.
    pub fn remote_config(&self, url: String) -> RemoteConfig {
        RemoteConfig::embedded(url, &self.data_dir)
    }
}

/// A started server as the supervisor sees it: `RunningServer`, or a test double.
#[async_trait]
trait Served: Send + Sync {
    /// `http://127.0.0.1:<port>`.
    fn url(&self) -> String;
    /// The spawned engine's pid.
    fn engine_pid(&self) -> Option<u32>;
    /// Resolves when the server stopped by itself; the reason, for the log.
    async fn closed(&self) -> String;
    /// Drains and stops it (the caller bounds it).
    async fn shutdown(self: Box<Self>) -> anyhow::Result<()>;
}

#[async_trait]
impl Served for RunningServer {
    fn url(&self) -> String {
        RunningServer::url(self)
    }

    fn engine_pid(&self) -> Option<u32> {
        RunningServer::engine_pid(self)
    }

    async fn closed(&self) -> String {
        RunningServer::closed(self).await.to_string()
    }

    async fn shutdown(self: Box<Self>) -> anyhow::Result<()> {
        RunningServer::shutdown(*self).await
    }
}

/// How a boot finds its engine and starts the server: the real one, or a test double (the
/// seam the supervisor's unit tests drive).
#[async_trait]
trait Starter: Send + Sync {
    /// [`engine_source`].
    async fn engine_source(&self) -> EngineSource;
    /// [`download_engine`]; `slow` is called once if it turns slow.
    async fn download(&self, slow: SlowSignal) -> Result<PathBuf, BootProblem>;
    /// `euchre_server::start`. Owns what it needs, so it runs as a task on the server's own
    /// runtime, and a boot that is cancelled or runs out of time aborts it there.
    async fn start(self: Arc<Self>, cfg: ServerConfig) -> anyhow::Result<Box<dyn Served>>;
}

/// The real [`Starter`].
struct ServerStarter;

#[async_trait]
impl Starter for ServerStarter {
    async fn engine_source(&self) -> EngineSource {
        engine_source().await
    }

    async fn download(&self, slow: SlowSignal) -> Result<PathBuf, BootProblem> {
        download_engine(slow).await
    }

    async fn start(self: Arc<Self>, cfg: ServerConfig) -> anyhow::Result<Box<dyn Served>> {
        let server = euchre_server::start(cfg).await?;
        Ok(Box::new(server))
    }
}

/// What the supervisor and its server task share.
struct Shared {
    phase: watch::Sender<ServerPhase>,
    live: parking_lot::Mutex<Live>,
    /// Every phase the supervisor moved to, in order (tests check the edges).
    #[cfg(test)]
    trail: parking_lot::Mutex<Vec<ServerPhase>>,
}

/// The running server's handles. Never held across an `.await`.
#[derive(Default)]
struct Live {
    /// A boot began; the first `boot()` call wins, later ones do nothing.
    booted: bool,
    /// The backend of the running server (cleared when it stops).
    remote: Option<Arc<RemoteBackend>>,
    /// The engine's pid while ready.
    engine_pid: Option<u32>,
    /// Asks the server task to stop the server.
    stop: Option<oneshot::Sender<()>>,
    /// The server task; it ends once the server is stopped.
    task: Option<JoinHandle<()>>,
}

impl Shared {
    fn phase(&self) -> ServerPhase {
        self.phase.borrow().clone()
    }

    /// Moves to `next` when the edge is allowed (contract §N2.2); returns whether it moved.
    fn set_phase(&self, next: ServerPhase) -> bool {
        let moved = self.phase.send_if_modified(|current| {
            if *current == next || !allowed_edge(current, &next) {
                return false;
            }
            *current = next.clone();
            true
        });
        if moved {
            #[cfg(test)]
            self.trail.lock().push(next);
        } else if self.phase() != next {
            tracing::debug!(from = ?self.phase(), to = ?next, "server phase change refused");
        }
        moved
    }
}

/// The only phase changes there are: `Starting` ⇄ `Downloading` (first launch), which may
/// turn `DownloadingSlow` (and from there back to `Starting`), `Starting` → `Ready` or
/// `Failed`, `Ready` → `Stopped`, and `Stopped` or `Failed` → `Starting`.
fn allowed_edge(from: &ServerPhase, to: &ServerPhase) -> bool {
    use ServerPhase::{Downloading, DownloadingSlow, Failed, Ready, Starting, Stopped};
    matches!(
        (from, to),
        (Starting, Downloading | Ready | Failed(_))
            | (Downloading, Starting | DownloadingSlow)
            | (DownloadingSlow, Starting)
            | (Ready, Stopped)
            | (Stopped | Failed(_), Starting)
    )
}

/// The server's lifecycle, shared by the backend and the bridge.
pub struct Supervisor {
    config: EmbeddedConfig,
    starter: Arc<dyn Starter>,
    shared: Arc<Shared>,
    /// Serializes boot, restart and shutdown (an async lock: it is held while they await).
    lifecycle: tokio::sync::Mutex<()>,
    /// Cancelled by [`ServerHost::shutdown`]: a boot in flight stops and none starts again.
    cancel: CancellationToken,
    /// The data dir's lock, taken by the first boot and held until the server is stopped
    /// at quit (review fixes LDS-2, LDS-6).
    instance: parking_lot::Mutex<Option<InstanceLock>>,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor")
            .field("config", &self.config)
            .field("phase", &self.shared.phase())
            .finish_non_exhaustive()
    }
}

impl Supervisor {
    /// A supervisor in [`ServerPhase::Starting`]; nothing runs until [`ServerHost::boot`].
    pub fn new(config: EmbeddedConfig) -> Supervisor {
        Supervisor::with_starter(config, Arc::new(ServerStarter))
    }

    fn with_starter(config: EmbeddedConfig, starter: Arc<dyn Starter>) -> Supervisor {
        let (phase, _) = watch::channel(ServerPhase::Starting);
        Supervisor {
            config,
            starter,
            shared: Arc::new(Shared {
                phase,
                live: parking_lot::Mutex::default(),
                #[cfg(test)]
                trail: parking_lot::Mutex::new(vec![ServerPhase::Starting]),
            }),
            lifecycle: tokio::sync::Mutex::new(()),
            cancel: CancellationToken::new(),
            instance: parking_lot::Mutex::new(None),
        }
    }

    /// The configuration.
    pub fn config(&self) -> &EmbeddedConfig {
        &self.config
    }

    /// The phase now.
    pub fn phase(&self) -> ServerPhase {
        self.shared.phase()
    }

    /// The backend of the running server, once [`ServerPhase::Ready`].
    pub fn remote(&self) -> Option<Arc<RemoteBackend>> {
        self.shared.live.lock().remote.clone()
    }

    /// Tests: the running server's engine pid (`RunningServer::engine_pid`), so a live test
    /// can stop the engine it started and watch [`ServerPhase::Stopped`].
    pub fn engine_pid(&self) -> Option<u32> {
        self.shared.live.lock().engine_pid
    }

    /// One boot, with the lifecycle lock held: find the engine, read the server's saved
    /// config, start it under [`BOOT_WAIT`], then hand it to a new server task.
    async fn boot_locked(&self) {
        if self.cancel.is_cancelled() {
            return;
        }
        let shared = &self.shared;
        shared.set_phase(ServerPhase::Starting);
        let began = Instant::now();
        // One copy of Euchre per data dir: wait (bounded) for one that is quitting.
        if self.instance.lock().is_none() {
            let locked = super::local::lock_within(
                &self.config.data_dir,
                self.config.instance_wait,
                &self.cancel,
            )
            .await;
            match locked {
                None => return,
                Some(Ok(lock)) => *self.instance.lock() = Some(lock),
                Some(Err(LockError::Held)) => {
                    tracing::warn!("another copy of Euchre holds this data dir");
                    shared.set_phase(ServerPhase::Failed(BootProblem::AlreadyOpen));
                    return;
                }
                Some(Err(LockError::Io(err))) => {
                    // Play on: the engine's own owner record still guards the storage.
                    tracing::warn!(kind = ?err.kind(), "the data dir lock could not be taken");
                }
            }
        }
        if let Err(err) = crate::startup::secure_data_dir(&self.config.data_dir) {
            tracing::warn!(kind = ?err.kind(), "the data dir could not be made private");
        }
        let source = tokio::select! {
            source = self.starter.engine_source() => source,
            () = self.cancel.cancelled() => return,
        };
        let binary = match source {
            EngineSource::Env(_) => None,
            EngineSource::Bundled(path) | EngineSource::Cache(path) => Some(path),
            EngineSource::Download => {
                shared.set_phase(ServerPhase::Downloading);
                let slow: SlowSignal = {
                    let shared = Arc::clone(shared);
                    Arc::new(move || {
                        shared.set_phase(ServerPhase::DownloadingSlow);
                    })
                };
                let fetched = tokio::select! {
                    fetched = self.starter.download(slow) => fetched,
                    () = self.cancel.cancelled() => return,
                };
                shared.set_phase(ServerPhase::Starting);
                match fetched {
                    Ok(path) => Some(path),
                    Err(problem) => {
                        shared.set_phase(ServerPhase::Failed(problem));
                        return;
                    }
                }
            }
        };
        let mut cfg = match ServerConfig::try_local(self.config.server_dir()) {
            Ok(cfg) => cfg,
            Err(err) => {
                tracing::warn!("the game server's saved data could not be read: {err:#}");
                shared.set_phase(ServerPhase::Failed(BootProblem::DataConflict));
                return;
            }
        };
        let storage = self.config.engine_storage();
        if let Some(port) = self.config.engine_port {
            cfg = cfg.with_engine_port(port);
        } else if let Some(port) = super::local::first_run_port(&storage, &self.config.data_dir) {
            // A first run: a block in 20000-29999 whose three ports are free, rather than an
            // ephemeral port another program may take later (review fix LDS-3).
            tracing::info!(port, "first run: the engine takes this port from now on");
            cfg = cfg.with_engine_port(port);
        }
        cfg.fast_tempo = self.config.fast_tempo;
        if self.config.offline_ai {
            cfg.ai = euchre_server_core::config::AiConfig::default();
        }
        if binary.is_some() {
            cfg.engine.binary_path = binary;
        }

        // An older copy of Euchre that is quitting still owns the engine: give it the rest of
        // the wait to stop it (review fix LDS-2).
        let deadline = began + self.config.instance_wait;
        if let Some(owner) = super::local::wait_owner_gone(&storage, deadline, &self.cancel).await {
            tracing::warn!(
                owner,
                "another copy of Euchre still runs this data dir's engine"
            );
            shared.set_phase(ServerPhase::Failed(BootProblem::AlreadyOpen));
            return;
        }
        if self.cancel.is_cancelled() {
            return;
        }

        let runtime = match ServerRuntime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                tracing::warn!(%err, "the game server's runtime could not be built");
                shared.set_phase(ServerPhase::Failed(BootProblem::Other));
                return;
            }
        };
        let mut starting = runtime.handle().spawn(Arc::clone(&self.starter).start(cfg));
        let started = tokio::select! {
            started = tokio::time::timeout(BOOT_WAIT, &mut starting) => started,
            () = self.cancel.cancelled() => {
                tracing::info!("boot cancelled: the app is quitting");
                abort_bounded(starting, "a cancelled boot").await;
                return;
            }
        };
        let served = match started {
            Err(_elapsed) => {
                tracing::warn!(after = ?BOOT_WAIT, "the game server did not start in time");
                abort_bounded(starting, "a boot that ran out of time").await;
                shared.set_phase(ServerPhase::Failed(BootProblem::TooSlow));
                return;
            }
            Ok(Err(err)) => {
                tracing::warn!(%err, "the game server's start task failed");
                shared.set_phase(ServerPhase::Failed(BootProblem::Other));
                return;
            }
            Ok(Ok(Err(err))) => {
                tracing::warn!("the game server did not start: {err:#}");
                let problem = match boot_problem(&err) {
                    // Another copy of Euchre behind the busy port says so; anything else is
                    // another program (review fix LDS-3).
                    BootProblem::PortInUse { .. }
                        if super::local::other_live_owner(&storage).is_some() =>
                    {
                        BootProblem::AlreadyOpen
                    }
                    problem => problem,
                };
                shared.set_phase(ServerPhase::Failed(problem));
                return;
            }
            Ok(Ok(Ok(served))) => served,
        };
        let url = served.url();
        let remote = match RemoteBackend::new(self.config.remote_config(url.clone())) {
            Ok(remote) => Arc::new(remote),
            Err(err) => {
                tracing::warn!(?err, "the game server's client could not be built");
                stop_served(served, runtime).await;
                shared.set_phase(ServerPhase::Failed(BootProblem::Other));
                return;
            }
        };
        let engine_pid = served.engine_pid();
        // The engine outlives nothing: if this process dies without quitting, the watchdog
        // stops it (review fix LDS-1).
        let watchdog = engine_pid.map(|pid| Watchdog::arm(pid, &storage));
        let (stop_tx, stop_rx) = oneshot::channel();
        {
            let mut live = shared.live.lock();
            live.remote = Some(Arc::clone(&remote));
            live.engine_pid = engine_pid;
            live.stop = Some(stop_tx);
        }
        shared.set_phase(ServerPhase::Ready);
        crate::startup::set_engine_link_expected(true);
        tracing::info!(
            %url,
            ?engine_pid,
            boot_ms = began.elapsed().as_millis(),
            "the game server is ready"
        );
        let task = tokio::spawn(serve(
            Arc::clone(shared),
            Running {
                served,
                runtime,
                watchdog,
            },
            remote,
            stop_rx,
        ));
        shared.live.lock().task = Some(task);
    }
}

/// A started server, the runtime it runs on, and its engine's watchdog.
struct Running {
    served: Box<dyn Served>,
    runtime: ServerRuntime,
    /// Dropped once the server is stopped.
    watchdog: Option<Watchdog>,
}

/// The server task: waits for the server to stop by itself or for a stop request, then ends
/// its tables and stops it within [`STOP_BUDGET`] (and the bounded drop).
async fn serve(
    shared: Arc<Shared>,
    Running {
        served,
        runtime,
        watchdog,
    }: Running,
    remote: Arc<RemoteBackend>,
    stop: oneshot::Receiver<()>,
) {
    let failure = tokio::select! {
        why = served.closed() => Some(why),
        // A request, or the supervisor went away.
        _ = stop => None,
    };
    // From here the engine client's connection errors are expected, not news.
    crate::startup::set_engine_link_expected(false);
    tracing::info!("the game server is stopping; its engine link closes now");
    remote.end_tables(NetError::ServerStopped);
    {
        let mut live = shared.live.lock();
        live.remote = None;
        live.engine_pid = None;
    }
    if let Some(why) = failure {
        tracing::warn!(failure = %why, "the game server stopped");
        shared.set_phase(ServerPhase::Stopped);
        // Its engine is gone: there is nothing to drain or deregister with, so the server
        // is dropped and its runtime cleared at once instead of waiting out the registry's
        // grace (review fix PX-5).
        clear_dead(served, runtime).await;
    } else {
        stop_served(served, runtime).await;
    }
    drop(watchdog);
}

/// Clears a server whose engine died: drops it on its runtime (aborting its HTTP side and
/// registry, unbinding its tables), then shuts the runtime down within [`DEAD_STOP`], so its
/// listener and every task it left are gone before a restart boots again.
async fn clear_dead(served: Box<dyn Served>, runtime: ServerRuntime) {
    let began = Instant::now();
    let dropping = runtime.handle().spawn(async move { drop(served) });
    if tokio::time::timeout(DEAD_STOP, dropping).await.is_err() {
        tracing::warn!("the stopped server is slow to clear");
    }
    runtime.shutdown(DEAD_STOP).await;
    tracing::info!(
        stop_ms = began.elapsed().as_millis(),
        "the stopped game server is cleared"
    );
}

/// `shutdown()` on the server's runtime under [`STOP_BUDGET`]; past it, the rest is aborted
/// (bounded). Then the runtime shuts down with whatever the server left running.
async fn stop_served(served: Box<dyn Served>, runtime: ServerRuntime) {
    let began = std::time::Instant::now();
    let mut stopping = runtime.handle().spawn(served.shutdown());
    match tokio::time::timeout(STOP_BUDGET, &mut stopping).await {
        Ok(Ok(Ok(()))) => tracing::info!(
            stop_ms = began.elapsed().as_millis(),
            "the game server stopped cleanly"
        ),
        Ok(Ok(Err(err))) => tracing::warn!("the game server did not stop cleanly: {err:#}"),
        Ok(Err(err)) => tracing::warn!(%err, "the game server's stop task failed"),
        Err(_elapsed) => {
            tracing::warn!(after = ?STOP_BUDGET, "the game server is slow to stop; dropping it");
            abort_bounded(stopping, "a server past its stop budget").await;
        }
    }
    drop(runtime);
}

/// One server's own tokio runtime, shut down once that server has stopped.
///
/// A server whose engine died cannot drain its registry, so `RunningServer::shutdown` aborts
/// the registry after its grace. The engine client's reconnect loop is a detached task that
/// the abort does not reach: left on a shared runtime it keeps dialling and registers again
/// with the next engine on the same port, next to the new server's runner
/// (`docs/port/requests/desktop-net-stale-registry.md`). On a runtime of its own, every task
/// the server spawned ends with it.
struct ServerRuntime {
    handle: tokio::runtime::Handle,
    /// Taken only by the drop.
    runtime: Option<tokio::runtime::Runtime>,
}

impl ServerRuntime {
    fn new() -> std::io::Result<ServerRuntime> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("euchre-server")
            .enable_all()
            .build()?;
        Ok(ServerRuntime {
            handle: runtime.handle().clone(),
            runtime: Some(runtime),
        })
    }

    fn handle(&self) -> &tokio::runtime::Handle {
        &self.handle
    }

    /// Shuts the runtime down, waiting up to `wait` for its tasks to end (on a blocking
    /// thread, never on the bridge's workers).
    async fn shutdown(mut self, wait: Duration) {
        if let Some(runtime) = self.runtime.take() {
            let joined = tokio::task::spawn_blocking(move || runtime.shutdown_timeout(wait)).await;
            if let Err(err) = joined {
                tracing::warn!(%err, "the server runtime did not shut down cleanly");
            }
        }
    }
}

impl Drop for ServerRuntime {
    fn drop(&mut self) {
        // Never blocks, so it is safe on the bridge's workers.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// Aborts a task on a server's runtime and waits up to [`DROP_WAIT`] for its future to be
/// dropped (an engine's `Drop` may wait for the process to exit).
async fn abort_bounded<T>(task: JoinHandle<T>, what: &'static str) {
    task.abort();
    if tokio::time::timeout(DROP_WAIT, task).await.is_err() {
        tracing::warn!(what, "still stopping; the process exit takes it");
    }
}

#[async_trait]
impl ServerHost for Supervisor {
    fn phases(&self) -> watch::Receiver<ServerPhase> {
        self.shared.phase.subscribe()
    }

    async fn boot(&self) {
        if self.cancel.is_cancelled()
            || std::mem::replace(&mut self.shared.live.lock().booted, true)
        {
            return;
        }
        let _lifecycle = self.lifecycle.lock().await;
        self.boot_locked().await;
    }

    async fn restart(&self) {
        if self.cancel.is_cancelled() {
            return;
        }
        // Flip to `Starting` at once (the player sees the press land); only from `Stopped` or
        // `Failed`, so a second press while it boots does nothing.
        let from = self.shared.phase();
        if !matches!(from, ServerPhase::Stopped | ServerPhase::Failed(_))
            || !self.shared.set_phase(ServerPhase::Starting)
        {
            tracing::info!(phase = ?from, "restart ignored: the game server is not stopped");
            return;
        }
        let _lifecycle = self.lifecycle.lock().await;
        self.shared.live.lock().booted = true;
        // The old server task ends once its server is stopped (it frees the ports first).
        let old = self.shared.live.lock().task.take();
        if let Some(old) = old
            && let Err(err) = old.await
        {
            tracing::warn!(%err, "the old server task failed");
        }
        self.boot_locked().await;
    }

    async fn shutdown(&self) {
        self.cancel.cancel();
        let _lifecycle = self.lifecycle.lock().await;
        let (stop, task) = {
            let mut live = self.shared.live.lock();
            (live.stop.take(), live.task.take())
        };
        if let Some(stop) = stop {
            let _ = stop.send(());
        }
        if let Some(task) = task
            && let Err(err) = task.await
        {
            tracing::warn!(%err, "the server task failed while stopping");
        }
        // The engine is stopped: a relaunch may take the data dir now.
        drop(self.instance.lock().take());
    }
}

/// Maps a failed `euchre_server::start` to what the player reads: the first
/// `euchre_server::BootError` in the error chain, else [`BootProblem::Other`].
pub fn boot_problem(err: &anyhow::Error) -> BootProblem {
    let boot = err
        .downcast_ref::<BootError>()
        .or_else(|| err.chain().find_map(|e| e.downcast_ref::<BootError>()));
    let Some(boot) = boot else {
        return BootProblem::Other;
    };
    match boot {
        BootError::EnginePortInUse { port, .. } => BootProblem::PortInUse { port: *port },
        BootError::LowDisk { free_bytes, .. } => BootProblem::LowDisk {
            free_bytes: *free_bytes,
            need_bytes: MIN_FREE_DISK_BYTES,
        },
        BootError::EngineBinaryMissing => BootProblem::EngineMissing,
        BootError::NotReady(_) => BootProblem::TooSlow,
        BootError::AlreadyRunning
        | BootError::HttpPortInUse { .. }
        | BootError::KeysConflict
        | BootError::TableConfigConflict => BootProblem::Relaunch,
        BootError::EnginePortChanged { .. } => BootProblem::Other,
    }
}

/// Where the engine binary comes from, in order (contract §N2.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineSource {
    /// `RIVET_ENGINE_BINARY_PATH` (`ServerConfig::try_local` reads it; nothing to pass).
    Env(PathBuf),
    /// Bundled with the app ([`bundled_engine`]); passed as `cfg.engine.binary_path`.
    Bundled(PathBuf),
    /// Already on this machine: rivetkit's cache (`~/.rivetkit/engine/v3/3.1.0/...`) or a
    /// dev-tree build, as `rivetkit_engine_process::resolve_engine_binary_path` finds it with
    /// downloads off; passed as `cfg.engine.binary_path`.
    Cache(PathBuf),
    /// Nothing local: [`download_engine`] fetches it into rivetkit's cache first.
    Download,
}

/// The binary resolver's settings, downloads off (the download is [`download_engine`]'s).
fn resolver() -> EngineResolverConfig {
    EngineResolverConfig::from_parts(RESOLVER_ENDPOINT, None, None, None, false)
}

/// Resolves [`EngineSource`] without network access, in the order of the variants.
pub async fn engine_source() -> EngineSource {
    if let Some(path) = std::env::var_os("RIVET_ENGINE_BINARY_PATH").filter(|p| !p.is_empty()) {
        return EngineSource::Env(PathBuf::from(path));
    }
    if let Some(path) = std::env::current_exe()
        .ok()
        .and_then(|exe| bundled_engine(&exe))
    {
        return EngineSource::Bundled(path);
    }
    match resolve_engine_binary_path(&resolver()).await {
        Ok(path) => EngineSource::Cache(path),
        Err(err) => {
            tracing::info!("no engine on this machine yet; downloading it: {err:#}");
            EngineSource::Download
        }
    }
}

/// The engine binary's file name on this platform.
const ENGINE_FILE: &str = if cfg!(windows) {
    "rivet-engine.exe"
} else {
    "rivet-engine"
};

/// The engine shipped with the app, if any: `<exe dir>/rivet-engine`, else the macOS
/// bundle's `<exe dir>/../Resources/rivet-engine` (`.exe` on Windows).
pub fn bundled_engine(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    [
        dir.join(ENGINE_FILE),
        dir.join("..").join("Resources").join(ENGINE_FILE),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

/// The first launch with no engine on the machine: download it into rivetkit's cache
/// ([`super::fetch`]: streamed, resumable, checksum-verified, no total cap) and return its
/// path. `slow` is called once when it turns slow. A release server it never reaches is
/// [`BootProblem::EngineMissing`]; a download that stops arriving is
/// [`BootProblem::DownloadStalled`].
pub async fn download_engine(slow: SlowSignal) -> Result<PathBuf, BootProblem> {
    let began = Instant::now();
    let Some(plan) = FetchPlan::for_this_machine() else {
        tracing::warn!("no place to keep the engine (no home dir)");
        return Err(BootProblem::EngineMissing);
    };
    match fetch_engine(&plan, FetchTiming::default(), &*slow).await {
        Ok(path) => {
            tracing::info!(
                path = %path.display(),
                download_ms = began.elapsed().as_millis(),
                "engine downloaded"
            );
            Ok(path)
        }
        Err(err) => {
            tracing::warn!(%err, after_ms = began.elapsed().as_millis(), "the engine could not be downloaded");
            Err(match err {
                FetchError::Stalled => BootProblem::DownloadStalled,
                FetchError::Unreachable | FetchError::Bad(_) => BootProblem::EngineMissing,
                FetchError::Disk(_) => BootProblem::Other,
            })
        }
    }
}

/// The embedded backend: the [`Supervisor`] plus delegation to its [`RemoteBackend`].
#[derive(Clone, Debug)]
pub struct EmbeddedBackend {
    supervisor: Arc<Supervisor>,
}

impl EmbeddedBackend {
    /// A backend for `config`. Nothing starts until the bridge boots its host.
    pub fn new(config: EmbeddedConfig) -> EmbeddedBackend {
        EmbeddedBackend {
            supervisor: Arc::new(Supervisor::new(config)),
        }
    }

    /// The supervisor (the bridge reaches it through [`Backend::host`]).
    pub fn supervisor(&self) -> &Arc<Supervisor> {
        &self.supervisor
    }

    /// The running server's backend, once ready (the live test interrupts its tables).
    pub fn remote(&self) -> Option<Arc<RemoteBackend>> {
        self.supervisor.remote()
    }

    /// Waits for [`ServerPhase::Ready`] (up to [`BOOT_WAIT`]) and returns the running
    /// server's backend; a stopped or failed server, or one that never came up, is
    /// [`NetError::ServerStopped`].
    async fn ready(&self) -> Result<Arc<RemoteBackend>, NetError> {
        let mut phases = self.supervisor.phases();
        let wait = async {
            loop {
                let phase = phases.borrow_and_update().clone();
                match phase {
                    ServerPhase::Ready => return true,
                    ServerPhase::Stopped | ServerPhase::Failed(_) => return false,
                    ServerPhase::Starting
                    | ServerPhase::Downloading
                    | ServerPhase::DownloadingSlow => {}
                }
                if phases.changed().await.is_err() {
                    return false;
                }
            }
        };
        match tokio::time::timeout(BOOT_WAIT, wait).await {
            Ok(true) => self.supervisor.remote().ok_or(NetError::ServerStopped),
            Ok(false) | Err(_) => Err(NetError::ServerStopped),
        }
    }
}

/// A delegate's error as the embedded backend reports it: a server that cannot be reached at
/// all went away mid-call ([`NetError::ServerStopped`]; `closed()` is about to say so).
fn embedded_error(err: NetError) -> NetError {
    match err {
        NetError::Offline => NetError::ServerStopped,
        other => other,
    }
}

/// Forwards a table's stream, reading an `Ended(Offline)` as `Ended(ServerStopped)`.
///
/// When the app drops its end (the table closed), the forwarder drops the
/// remote stream at once, so the remote connection closes now rather than at
/// the table's next event.
fn embedded_events(
    mut events: mpsc::UnboundedReceiver<ConnEvent>,
) -> mpsc::UnboundedReceiver<ConnEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                event = events.recv() => event,
                () = tx.closed() => break,
            };
            let Some(event) = event else { break };
            let event = match event {
                ConnEvent::Ended(err) => ConnEvent::Ended(embedded_error(err)),
                other => other,
            };
            if tx.send(event).is_err() {
                break;
            }
        }
    });
    rx
}

#[async_trait]
impl Backend for EmbeddedBackend {
    fn name(&self) -> &'static str {
        "embedded"
    }

    async fn identity(&self) -> Result<Identity, NetError> {
        let remote = self.ready().await?;
        remote.identity().await.map_err(embedded_error)
    }

    async fn create_table(&self, cfg: TableConfig) -> Result<String, NetError> {
        let remote = self.ready().await?;
        remote.create_table(cfg).await.map_err(embedded_error)
    }

    async fn open_table(&self, game_id: &str) -> Result<TableConn, NetError> {
        let remote = self.ready().await?;
        let conn = remote.open_table(game_id).await.map_err(embedded_error)?;
        Ok(TableConn {
            events: embedded_events(conn.events),
            ..conn
        })
    }

    async fn list_games(&self, cursor: Option<String>) -> Result<MatchListPage, NetError> {
        let remote = self.ready().await?;
        remote.list_games(cursor).await.map_err(embedded_error)
    }

    async fn match_detail(&self, match_id: &str) -> Result<ClientMatchReplay, NetError> {
        let remote = self.ready().await?;
        remote.match_detail(match_id).await.map_err(embedded_error)
    }

    async fn stats(&self) -> Result<ProfileStats, NetError> {
        let remote = self.ready().await?;
        remote.stats().await.map_err(embedded_error)
    }

    async fn get_settings(&self) -> Result<ProfileSettings, NetError> {
        let remote = self.ready().await?;
        remote.get_settings().await.map_err(embedded_error)
    }

    async fn patch_settings(&self, patch: SettingsPatch) -> Result<ProfileSettings, NetError> {
        let remote = self.ready().await?;
        remote.patch_settings(patch).await.map_err(embedded_error)
    }

    async fn recent_tables(&self) -> Result<RecentTables, NetError> {
        let remote = self.ready().await?;
        remote.recent_tables().await.map_err(embedded_error)
    }

    fn host(&self) -> Option<Arc<dyn ServerHost>> {
        Some(Arc::clone(&self.supervisor) as Arc<dyn ServerHost>)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use anyhow::Context as _;
    use tokio::sync::Notify;

    use super::*;

    /// What one fake start does.
    enum Start {
        /// Serves at once.
        Ok,
        /// Fails with this error.
        Fail(anyhow::Error),
        /// Never finishes (until dropped).
        Hang,
        /// Waits for the gate, then serves.
        Gate(Arc<Notify>),
    }

    /// Everything the fakes record, in order.
    #[derive(Default)]
    struct Log {
        events: parking_lot::Mutex<Vec<String>>,
    }

    impl Log {
        fn push(&self, event: impl Into<String>) {
            self.events.lock().push(event.into());
        }
        fn has(&self, event: &str) -> bool {
            self.events.lock().iter().any(|e| e == event)
        }
        fn count(&self, event: &str) -> usize {
            self.events.lock().iter().filter(|e| *e == event).count()
        }
    }

    struct FakeServed {
        n: usize,
        url: String,
        closed: CancellationToken,
        log: Arc<Log>,
    }

    #[async_trait]
    impl Served for FakeServed {
        fn url(&self) -> String {
            self.url.clone()
        }
        fn engine_pid(&self) -> Option<u32> {
            Some(4200 + u32::try_from(self.n).unwrap_or(0))
        }
        async fn closed(&self) -> String {
            self.closed.cancelled().await;
            "the local engine exited (signal: 15)".to_owned()
        }
        async fn shutdown(self: Box<Self>) -> anyhow::Result<()> {
            self.log.push(format!("shutdown {}", self.n));
            Ok(())
        }
    }

    impl Drop for FakeServed {
        fn drop(&mut self) {
            self.log.push(format!("gone {}", self.n));
        }
    }

    /// Records that a start future was dropped before it finished.
    struct DropFlag(Arc<Log>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.push("start dropped");
        }
    }

    struct FakeStarter {
        source: EngineSource,
        download: Result<PathBuf, BootProblem>,
        /// The download says it is slow first.
        download_slow: bool,
        starts: parking_lot::Mutex<VecDeque<Start>>,
        started: AtomicUsize,
        url: String,
        /// One token per started server: cancelling it is that server's engine exiting.
        closers: parking_lot::Mutex<Vec<CancellationToken>>,
        log: Arc<Log>,
    }

    impl FakeStarter {
        fn new(starts: Vec<Start>) -> FakeStarter {
            FakeStarter {
                source: EngineSource::Cache(PathBuf::from("/opt/rivet-engine")),
                download: Ok(PathBuf::from("/opt/rivet-engine")),
                download_slow: false,
                starts: parking_lot::Mutex::new(starts.into()),
                started: AtomicUsize::new(0),
                url: "http://127.0.0.1:9".to_owned(),
                closers: parking_lot::Mutex::default(),
                log: Arc::new(Log::default()),
            }
        }

        /// The newest server's engine exits.
        fn close_latest(&self) {
            if let Some(token) = self.closers.lock().last() {
                token.cancel();
            }
        }
    }

    #[async_trait]
    impl Starter for FakeStarter {
        async fn engine_source(&self) -> EngineSource {
            self.source.clone()
        }
        async fn download(&self, slow: SlowSignal) -> Result<PathBuf, BootProblem> {
            if self.download_slow {
                slow();
            }
            self.download.clone()
        }
        async fn start(self: Arc<Self>, cfg: ServerConfig) -> anyhow::Result<Box<dyn Served>> {
            let n = self.started.fetch_add(1, Ordering::SeqCst) + 1;
            self.log.push(format!("start {n}"));
            assert!(cfg.fast_tempo, "test config reaches the server");
            let step = self.starts.lock().pop_front().unwrap_or(Start::Ok);
            let flag = DropFlag(Arc::clone(&self.log));
            match step {
                Start::Ok => {}
                Start::Fail(err) => {
                    std::mem::forget(flag);
                    return Err(err);
                }
                Start::Hang => std::future::pending::<()>().await,
                Start::Gate(gate) => gate.notified().await,
            }
            std::mem::forget(flag);
            let closed = CancellationToken::new();
            self.closers.lock().push(closed.clone());
            Ok(Box::new(FakeServed {
                n,
                url: self.url.clone(),
                closed,
                log: Arc::clone(&self.log),
            }))
        }
    }

    fn supervisor(starter: &Arc<FakeStarter>) -> (Arc<Supervisor>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = EmbeddedConfig::for_tests(dir.path().to_path_buf(), 6860);
        let starter: Arc<dyn Starter> = Arc::clone(starter) as Arc<dyn Starter>;
        (Arc::new(Supervisor::with_starter(config, starter)), dir)
    }

    /// Waits (up to 5 s) for `want`.
    async fn reach(sup: &Supervisor, want: &ServerPhase) {
        let mut phases = sup.phases();
        let got = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if *phases.borrow_and_update() == *want {
                    return;
                }
                if phases.changed().await.is_err() {
                    return;
                }
            }
        })
        .await;
        assert!(got.is_ok(), "never reached {want:?}; at {:?}", sup.phase());
    }

    #[test]
    fn every_boot_error_maps_to_its_problem() {
        let port = 6860;
        let cases: Vec<(BootError, BootProblem)> = vec![
            (
                BootError::EnginePortInUse {
                    port,
                    ports: vec![port, port + 1, port + 5],
                },
                BootProblem::PortInUse { port },
            ),
            (
                BootError::LowDisk {
                    path: PathBuf::from("/data/engine"),
                    free_bytes: 2 << 30,
                },
                BootProblem::LowDisk {
                    free_bytes: 2 << 30,
                    need_bytes: 4 << 30,
                },
            ),
            (BootError::EngineBinaryMissing, BootProblem::EngineMissing),
            (
                BootError::NotReady(Duration::from_secs(60)),
                BootProblem::TooSlow,
            ),
            (BootError::AlreadyRunning, BootProblem::Relaunch),
            (
                BootError::HttpPortInUse {
                    addr: SocketAddr::from(([127, 0, 0, 1], 4100)),
                },
                BootProblem::Relaunch,
            ),
            (BootError::KeysConflict, BootProblem::Relaunch),
            (BootError::TableConfigConflict, BootProblem::Relaunch),
            (
                BootError::EnginePortChanged {
                    recorded: 6420,
                    requested: port,
                },
                BootProblem::Other,
            ),
        ];
        for (boot, problem) in cases {
            let label = format!("{boot:?}");
            // Returned directly (`?` on a `BootError`).
            let direct = anyhow::Error::new(clone_boot(&boot));
            assert_eq!(boot_problem(&direct), problem, "direct {label}");
            // Added as context over the engine's own error, the way `diagnose` does.
            let context = anyhow::anyhow!("engine exited").context(clone_boot(&boot));
            assert_eq!(boot_problem(&context), problem, "context {label}");
            // Wrapped once more by a caller.
            let wrapped = Err::<(), _>(context)
                .context("starting the game server")
                .expect_err("an error");
            assert_eq!(boot_problem(&wrapped), problem, "wrapped {label}");
        }
        assert_eq!(
            boot_problem(&anyhow::anyhow!("something else")),
            BootProblem::Other
        );
    }

    fn clone_boot(boot: &BootError) -> BootError {
        match boot {
            BootError::AlreadyRunning => BootError::AlreadyRunning,
            BootError::EnginePortInUse { port, ports } => BootError::EnginePortInUse {
                port: *port,
                ports: ports.clone(),
            },
            BootError::EnginePortChanged {
                recorded,
                requested,
            } => BootError::EnginePortChanged {
                recorded: *recorded,
                requested: *requested,
            },
            BootError::HttpPortInUse { addr } => BootError::HttpPortInUse { addr: *addr },
            BootError::LowDisk { path, free_bytes } => BootError::LowDisk {
                path: path.clone(),
                free_bytes: *free_bytes,
            },
            BootError::EngineBinaryMissing => BootError::EngineBinaryMissing,
            BootError::KeysConflict => BootError::KeysConflict,
            BootError::TableConfigConflict => BootError::TableConfigConflict,
            BootError::NotReady(d) => BootError::NotReady(*d),
        }
    }

    #[test]
    fn bundled_engine_checks_exe_dir_then_resources() {
        let root = tempfile::tempdir().expect("temp dir");
        let macos = root.path().join("Euchre.app/Contents/MacOS");
        let resources = root.path().join("Euchre.app/Contents/Resources");
        std::fs::create_dir_all(&macos).expect("MacOS dir");
        std::fs::create_dir_all(&resources).expect("Resources dir");
        let exe = macos.join("euchre");
        assert_eq!(bundled_engine(&exe), None, "nothing bundled");

        std::fs::write(resources.join(ENGINE_FILE), b"engine").expect("resources engine");
        let found = bundled_engine(&exe).expect("the bundle's Resources engine");
        assert_eq!(
            found.canonicalize().expect("canonical"),
            resources
                .join(ENGINE_FILE)
                .canonicalize()
                .expect("canonical")
        );

        std::fs::write(macos.join(ENGINE_FILE), b"engine").expect("exe-dir engine");
        assert_eq!(
            bundled_engine(&exe),
            Some(macos.join(ENGINE_FILE)),
            "next to the exe wins"
        );
        // A directory with the engine's name is not an engine.
        let other = root.path().join("other");
        std::fs::create_dir_all(other.join(ENGINE_FILE)).expect("dir");
        assert_eq!(bundled_engine(&other.join("euchre")), None);
    }

    #[tokio::test]
    async fn boot_reaches_ready() {
        let starter = Arc::new(FakeStarter::new(vec![Start::Ok]));
        let (sup, _dir) = supervisor(&starter);
        sup.boot().await;
        assert_eq!(sup.phase(), ServerPhase::Ready);
        let remote = sup.remote().expect("a remote backend once ready");
        let config = remote.config().expect("configured");
        assert_eq!(config.base_url, starter.url);
        assert_eq!(config.state_dir, sup.config().data_dir);
        assert_eq!(sup.engine_pid(), Some(4201));
        // A second boot does nothing.
        sup.boot().await;
        assert_eq!(starter.log.count("start 1"), 1);
        assert!(!starter.log.has("start 2"));
        sup.shutdown().await;
        assert!(starter.log.has("shutdown 1"));
    }

    #[tokio::test]
    async fn boot_error_fails_with_its_problem() {
        let err = anyhow::Error::new(BootError::LowDisk {
            path: PathBuf::from("/data"),
            free_bytes: 1 << 30,
        });
        let starter = Arc::new(FakeStarter::new(vec![Start::Fail(err)]));
        let (sup, _dir) = supervisor(&starter);
        sup.boot().await;
        assert_eq!(
            sup.phase(),
            ServerPhase::Failed(BootProblem::LowDisk {
                free_bytes: 1 << 30,
                need_bytes: MIN_FREE_DISK_BYTES,
            })
        );
        assert!(sup.remote().is_none());
        assert_eq!(sup.engine_pid(), None);

        // The first-start download failing is the "connect to the internet" problem.
        let mut offline = FakeStarter::new(vec![]);
        offline.source = EngineSource::Download;
        offline.download = Err(BootProblem::EngineMissing);
        let offline = Arc::new(offline);
        let (sup, _dir) = supervisor(&offline);
        sup.boot().await;
        assert_eq!(sup.phase(), ServerPhase::Failed(BootProblem::EngineMissing));
        assert!(!offline.log.has("start 1"), "no start without an engine");
    }

    #[tokio::test]
    async fn closed_stops_and_ends_tables() {
        let starter = Arc::new(FakeStarter::new(vec![Start::Ok]));
        let (sup, _dir) = supervisor(&starter);
        sup.boot().await;
        assert_eq!(sup.phase(), ServerPhase::Ready);
        let backend = EmbeddedBackend {
            supervisor: Arc::clone(&sup),
        };
        starter.close_latest();
        reach(&sup, &ServerPhase::Stopped).await;
        assert!(sup.remote().is_none(), "the remote is gone with the server");
        assert_eq!(sup.engine_pid(), None);
        // The server task clears the stopped server at once: its engine is gone, so there is
        // nothing to drain and no registry grace to wait out (review fix PX-5).
        tokio::time::timeout(Duration::from_secs(5), async {
            while !starter.log.has("gone 1") {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the stopped server is cleared");
        assert!(
            !starter.log.has("shutdown 1"),
            "a dead server is not drained"
        );
        // Every call (and so every table) now answers that the server stopped.
        assert_eq!(backend.identity().await, Err(NetError::ServerStopped));
        assert!(matches!(
            backend.open_table("g1").await,
            Err(NetError::ServerStopped)
        ));
        sup.shutdown().await;
    }

    #[tokio::test]
    async fn restart_from_stopped_boots_again() {
        let starter = Arc::new(FakeStarter::new(vec![Start::Ok, Start::Ok]));
        let (sup, _dir) = supervisor(&starter);
        sup.boot().await;
        let first_url = sup.remote().and_then(|r| r.config().cloned());
        // Restart does nothing while ready.
        sup.restart().await;
        assert!(!starter.log.has("start 2"));
        starter.close_latest();
        reach(&sup, &ServerPhase::Stopped).await;
        sup.restart().await;
        assert_eq!(sup.phase(), ServerPhase::Ready);
        assert_eq!(sup.engine_pid(), Some(4202));
        assert_eq!(sup.remote().and_then(|r| r.config().cloned()), first_url);
        // The old server was stopped before the new one started.
        let events = starter.log.events.lock().clone();
        let old_down = events.iter().position(|e| e == "gone 1");
        let new_up = events.iter().position(|e| e == "start 2");
        assert!(old_down.is_some() && old_down < new_up, "{events:?}");
        sup.shutdown().await;
        assert!(starter.log.has("shutdown 2"));
    }

    #[tokio::test]
    async fn shutdown_while_booting_cancels() {
        let starter = Arc::new(FakeStarter::new(vec![Start::Hang]));
        let (sup, _dir) = supervisor(&starter);
        let booting = tokio::spawn({
            let sup = Arc::clone(&sup);
            async move { sup.boot().await }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !starter.log.has("start 1") {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("the boot reached start");
        tokio::time::timeout(Duration::from_secs(5), sup.shutdown())
            .await
            .expect("shutdown cancels the boot promptly");
        booting.await.expect("boot task");
        assert!(
            starter.log.has("start dropped"),
            "the half-started server is dropped"
        );
        assert_eq!(
            sup.phase(),
            ServerPhase::Starting,
            "the phase is left as it is"
        );
        // Nothing boots after a shutdown.
        sup.restart().await;
        sup.boot().await;
        assert_eq!(starter.started.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn calls_wait_for_ready_then_delegate() {
        let gate = Arc::new(Notify::new());
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let mut starter = FakeStarter::new(vec![Start::Gate(Arc::clone(&gate))]);
        starter.url = server.uri();
        let starter = Arc::new(starter);
        let (sup, _dir) = supervisor(&starter);
        let backend = EmbeddedBackend {
            supervisor: Arc::clone(&sup),
        };
        let booting = tokio::spawn({
            let sup = Arc::clone(&sup);
            async move { sup.boot().await }
        });
        let call = tokio::spawn({
            let backend = backend.clone();
            async move { backend.identity().await }
        });
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(!call.is_finished(), "a call during the boot waits");
        assert_eq!(sup.phase(), ServerPhase::Starting);
        gate.notify_one();
        booting.await.expect("boot");
        let answer = tokio::time::timeout(Duration::from_secs(30), call)
            .await
            .expect("the call answers once ready")
            .expect("call task");
        // It went to the running server: a 5xx passes through as it is (never "stopped").
        assert_ne!(answer, Err(NetError::ServerStopped));
        let reached = server.received_requests().await.unwrap_or_default();
        assert!(
            answer == Err(NetError::NotWired) || !reached.is_empty(),
            "the call reached the server: {answer:?}"
        );
        // A delegate that cannot reach the server at all reads as a stopped server.
        assert_eq!(embedded_error(NetError::Offline), NetError::ServerStopped);
        assert_eq!(embedded_error(NetError::Forbidden), NetError::Forbidden);
        sup.shutdown().await;
    }

    #[tokio::test]
    async fn calls_after_a_failure_answer_server_stopped() {
        let err = anyhow::anyhow!("boom").context(BootError::NotReady(Duration::from_secs(60)));
        let starter = Arc::new(FakeStarter::new(vec![Start::Fail(err)]));
        let (sup, _dir) = supervisor(&starter);
        let backend = EmbeddedBackend {
            supervisor: Arc::clone(&sup),
        };
        sup.boot().await;
        assert_eq!(sup.phase(), ServerPhase::Failed(BootProblem::TooSlow));
        let answer = tokio::time::timeout(Duration::from_secs(1), backend.list_games(None))
            .await
            .expect("a failed server answers at once");
        assert_eq!(answer, Err(NetError::ServerStopped));
        assert_eq!(backend.recent_tables().await, Err(NetError::ServerStopped));
        sup.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_is_idempotent() {
        let starter = Arc::new(FakeStarter::new(vec![Start::Ok]));
        let (sup, _dir) = supervisor(&starter);
        sup.boot().await;
        tokio::join!(sup.shutdown(), sup.shutdown());
        sup.shutdown().await;
        assert_eq!(starter.log.count("shutdown 1"), 1);
        assert!(sup.remote().is_none());
        assert_eq!(
            sup.phase(),
            ServerPhase::Ready,
            "the phase is left as it is"
        );
    }

    #[tokio::test]
    async fn phases_follow_only_the_allowed_edges() {
        let err = anyhow::Error::new(BootError::EnginePortInUse {
            port: 6860,
            ports: vec![6860, 6861, 6865],
        });
        let mut starter = FakeStarter::new(vec![Start::Ok, Start::Fail(err), Start::Ok]);
        starter.source = EngineSource::Download;
        let starter = Arc::new(starter);
        let (sup, _dir) = supervisor(&starter);
        sup.boot().await;
        starter.close_latest();
        reach(&sup, &ServerPhase::Stopped).await;
        sup.restart().await;
        assert_eq!(
            sup.phase(),
            ServerPhase::Failed(BootProblem::PortInUse { port: 6860 })
        );
        sup.restart().await;
        assert_eq!(sup.phase(), ServerPhase::Ready);
        sup.shutdown().await;

        use ServerPhase::{Downloading, Failed, Ready, Starting, Stopped};
        let trail = sup.shared.trail.lock().clone();
        assert_eq!(
            trail,
            vec![
                Starting,
                Downloading,
                Starting,
                Ready,
                Stopped,
                Starting,
                Downloading,
                Starting,
                Failed(BootProblem::PortInUse { port: 6860 }),
                Starting,
                Downloading,
                Starting,
                Ready,
            ]
        );
        for pair in trail.windows(2) {
            assert!(allowed_edge(&pair[0], &pair[1]), "{pair:?}");
        }
        // And nothing else moves it.
        assert!(!allowed_edge(&Ready, &Starting));
        assert!(!allowed_edge(&Starting, &Stopped));
        assert!(!allowed_edge(&Stopped, &Ready));
        assert!(!allowed_edge(&Failed(BootProblem::Other), &Ready));
        assert!(!allowed_edge(&Downloading, &Ready));
    }

    /// Review fix PX-1: a download that turns slow says so, then starts as usual.
    #[tokio::test]
    async fn a_slow_download_says_so_then_starts() {
        let mut starter = FakeStarter::new(vec![Start::Ok]);
        starter.source = EngineSource::Download;
        starter.download_slow = true;
        let starter = Arc::new(starter);
        let (sup, _dir) = supervisor(&starter);
        sup.boot().await;
        assert_eq!(sup.phase(), ServerPhase::Ready);
        sup.shutdown().await;
        use ServerPhase::{Downloading, DownloadingSlow, Ready, Starting};
        let trail = sup.shared.trail.lock().clone();
        assert_eq!(
            trail,
            vec![Starting, Downloading, DownloadingSlow, Starting, Ready]
        );
        assert!(!allowed_edge(&DownloadingSlow, &Ready));
        assert!(!allowed_edge(&DownloadingSlow, &Downloading));
    }

    /// Review fixes LDS-2 and LDS-6: one copy per data dir. A launch waits for a copy that
    /// is quitting and starts once it lets go; past the wait it says another copy is open,
    /// without starting anything.
    #[tokio::test]
    async fn a_second_copy_waits_for_the_first_then_says_it_is_open() {
        let dir = tempfile::tempdir().expect("temp dir");
        let held = super::super::local::try_lock(dir.path()).expect("the first copy's lock");
        let build = |wait: Duration, starter: &Arc<FakeStarter>| {
            let mut config = EmbeddedConfig::for_tests(dir.path().to_path_buf(), 6860);
            config.instance_wait = wait;
            let starter: Arc<dyn Starter> = Arc::clone(starter) as Arc<dyn Starter>;
            Arc::new(Supervisor::with_starter(config, starter))
        };

        // The first copy never lets go.
        let starter = Arc::new(FakeStarter::new(vec![Start::Ok]));
        let sup = build(Duration::from_millis(300), &starter);
        let began = Instant::now();
        sup.boot().await;
        assert_eq!(sup.phase(), ServerPhase::Failed(BootProblem::AlreadyOpen));
        assert!(began.elapsed() >= Duration::from_millis(300));
        assert!(!starter.log.has("start 1"), "nothing starts beside it");
        sup.shutdown().await;

        // The first copy quits within the wait.
        let starter = Arc::new(FakeStarter::new(vec![Start::Ok]));
        let sup = build(Duration::from_secs(5), &starter);
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            drop(held);
        });
        sup.boot().await;
        assert_eq!(sup.phase(), ServerPhase::Ready);
        release.await.expect("release");
        // And it holds the lock now: a third copy is refused at once.
        assert!(matches!(
            super::super::local::try_lock(dir.path()),
            Err(super::super::local::LockError::Held)
        ));
        sup.shutdown().await;
        // Shutdown lets go of it.
        assert!(super::super::local::try_lock(dir.path()).is_ok());
    }

    #[tokio::test]
    async fn ended_offline_reads_as_server_stopped() {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut events = embedded_events(rx);
        tx.send(ConnEvent::Status(true)).expect("send");
        tx.send(ConnEvent::Ended(NetError::Offline)).expect("send");
        drop(tx);
        assert_eq!(events.recv().await, Some(ConnEvent::Status(true)));
        assert_eq!(
            events.recv().await,
            Some(ConnEvent::Ended(NetError::ServerStopped))
        );
        assert_eq!(events.recv().await, None);
    }
}
