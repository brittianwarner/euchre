//! Boot and shutdown (05 §12, ARCHITECTURE §2.4).
//!
//! # Boot order
//!
//! 1. Take the process lease (one live server per process) and check the
//!    engine settings (`assert_rivet_env_safety`).
//! 2. Bind the HTTP listener, so the port is known. A local server then allows
//!    its own loopback origins (`http://127.0.0.1:<port>`,
//!    `http://localhost:<port>`); without them every request is `421 Unknown
//!    host`.
//! 3. Build the registry: `playerProfile` (keys conflict fails here),
//!    `euchreTable` (config conflict fails here), `aiSeat` from the server config,
//!    and `health`. No engine is running yet, so a conflict leaves nothing behind.
//! 4. Spawn the engine (`EngineMode::Spawn`): free-disk check, then
//!    `LocalEngine::start` with no download, the configured admin token and data
//!    under `<data_dir>/engine`.
//! 5. `ensure_rivet_namespace` (engine 3: `POST /namespaces`, `PUT /pools/{pool}`).
//! 6. Serve the registry (`ServeConfig::from_env` plus the overrides below) and
//!    wait until the envoy answers `health.ping` (G36).
//! 7. Bind the table's client slot, build the router (gateway token issuer on,
//!    static web build if present), mark `/health` ready, serve HTTP.
//!
//! # Supervision
//!
//! After boot the server watches the spawned engine (polled every second) and
//! the actor runner (`Registry::serve`). When either stops by itself, `/health`
//! turns `503` for good and [`RunningServer::closed`] resolves with the cause;
//! [`run`] then stops the rest and exits non-zero.
//!
//! # Shutdown
//!
//! [`RunningServer::shutdown`]: `/health` turns `503 draining` and new tables are
//! refused; the HTTP server stops accepting and lets in-flight requests finish
//! (bounded); requests still open after that get `503` and their connections
//! close; then the registry stops (bounded); then the engine stops. Dropping a
//! [`RunningServer`] instead cancels everything at once and still stops the
//! engine. [`run`] stops at once on a second SIGTERM or ctrl-c.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use euchre_ai_seat::{AiSeatRuntime, SeatClient};
use euchre_http::downloads::{BucketSettings, DownloadSource};
use euchre_http::{
    ApiConfig, ApiError, ApiState, EnsureRivetNamespaceOptions, Health, RivetEnv,
    assert_rivet_env_safety, ensure_rivet_namespace,
};
use euchre_server_core::ServerConfig;
use euchre_server_core::auth::Authenticator;
use euchre_server_core::config::{EngineMode, OriginPolicy, Secret, random_secret};
use euchre_server_core::rivet::{
    LOCAL_ENGINE_NAMESPACE, LocalEngine, build_client_with_pool, wait_ready,
};
use euchre_table::TableConfig;
use parking_lot::Mutex;
use rivetkit::Action as _;
use rivetkit::client::{Client, GetOrCreateOptions};
use rivetkit::{EngineSpawnMode, Registry, ServeConfig};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::engine::{self, SpawnSpec};
use crate::health_actor::{self, HEALTH_ACTOR, Ping, health_key};
use crate::process;
use crate::{BootError, ServerFailure};

/// The static web build the deployed binary serves, relative to its working
/// directory (TS `resolve('build/client')`).
pub const DEFAULT_STATIC_DIR: &str = "build/client";

/// `503` body for `POST /api/tables` while the server drains.
pub const DRAINING_NEW_TABLE: &str = "The server is restarting. Start a new game in a moment.";

/// `503` body for a request still open when the HTTP drain runs out.
pub const RESTARTING: &str = "The server is restarting. Try again in a moment.";

/// How long the envoy may take to serve its first `health.ping`.
const READY_TIMEOUT: Duration = Duration::from_secs(60);
/// How long in-flight HTTP requests get once shutdown starts.
const HTTP_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
/// How long connections get to close after the requests still open are ended.
const HTTP_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// How often the supervisor checks that the spawned engine still runs.
const ENGINE_POLL: Duration = Duration::from_secs(1);
/// Registry grace once the engine is gone: no actor can be drained through it.
const REGISTRY_GRACE_ENGINE_GONE: Duration = Duration::from_secs(5);
/// How long the registry gets to stop its actors (TS `gracePeriodMs: 90000` in
/// deployments).
const REGISTRY_GRACE_DEPLOYED: Duration = Duration::from_secs(90);
const REGISTRY_GRACE_LOCAL: Duration = Duration::from_secs(20);
/// Each `health.ping` attempt.
const PING_TIMEOUT: Duration = Duration::from_secs(10);

/// Options beside the [`ServerConfig`].
#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    /// Static web build to serve, with `index.html` as the fallback for client
    /// routes (D4). Used only when it is an existing directory; `None` serves
    /// the API and the gateway only.
    pub static_dir: Option<PathBuf>,
    /// Desktop downloads folder served under `/download/`
    /// (`euchre_http::downloads`). `None`, or a folder that does not exist,
    /// serves the empty manifest.
    pub downloads_dir: Option<PathBuf>,
    /// Desktop downloads bucket; when set it wins over `downloads_dir`.
    pub downloads_bucket: Option<BucketSettings>,
}

impl StartOptions {
    /// The deployed binary's options: the web build at [`DEFAULT_STATIC_DIR`] and
    /// the desktop downloads from the environment: the bucket when all five
    /// `EUCHRE_DOWNLOADS_S3_*` variables are set, else the folder from
    /// `EUCHRE_DOWNLOADS_DIR` (default `downloads`); partly set logs one warning
    /// and serves none.
    pub fn deployed() -> Self {
        let (downloads_dir, downloads_bucket) = match DownloadSource::from_env() {
            DownloadSource::Disk(dir) => (Some(dir), None),
            DownloadSource::Bucket(bucket) => (None, Some(bucket)),
            DownloadSource::Off => (None, None),
        };
        Self {
            static_dir: Some(PathBuf::from(DEFAULT_STATIC_DIR)),
            downloads_dir,
            downloads_bucket,
        }
    }
}

struct Background<T> {
    cancel: CancellationToken,
    handle: JoinHandle<T>,
}

/// What the supervisor shares with the server: the first failure and the
/// switches it flips.
#[derive(Clone, Default)]
struct Supervision {
    health: Health,
    /// Cancelled when shutdown starts: from then on an exit is ours.
    stopping: CancellationToken,
    /// Cancelled on the first failure.
    closed: CancellationToken,
    failure: Arc<OnceLock<ServerFailure>>,
}

impl Supervision {
    fn new(health: Health) -> Self {
        Self {
            health,
            ..Self::default()
        }
    }

    /// Record `failure` unless shutdown started or one is recorded already:
    /// `/health` turns `503` and [`RunningServer::closed`] resolves.
    fn report(&self, failure: ServerFailure) {
        if self.stopping.is_cancelled() {
            return;
        }
        if self.failure.set(failure.clone()).is_ok() {
            tracing::error!(error = %failure, "the server stopped serving; /health answers 503");
            self.health.start_draining();
            self.closed.cancel();
        }
    }

    fn engine_gone(&self) -> bool {
        matches!(self.failure.get(), Some(ServerFailure::EngineExited { .. }))
    }
}

/// Reports [`ServerFailure::RunnerStopped`] when dropped, which the registry
/// task does as it ends: by returning, by an error, or by a panic.
struct RunnerWatch {
    supervision: Supervision,
    reason: Option<String>,
}

impl Drop for RunnerWatch {
    fn drop(&mut self) {
        let reason = self
            .reason
            .take()
            .unwrap_or_else(|| "its task ended without a result".to_owned());
        self.supervision
            .report(ServerFailure::RunnerStopped { reason });
    }
}

/// Poll the spawned engine until it exits (a failure), shutdown starts, or
/// shutdown takes it.
fn watch_engine(engine: Arc<Mutex<Option<LocalEngine>>>, supervision: Supervision) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(ENGINE_POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                () = supervision.stopping.cancelled() => return,
                _ = tick.tick() => {}
            }
            let exited = match engine.lock().as_mut() {
                None => return,
                Some(engine) => engine.try_wait(),
            };
            match exited {
                Ok(None) => {}
                Ok(Some(status)) => {
                    supervision.report(ServerFailure::EngineExited {
                        status: status.to_string(),
                    });
                    return;
                }
                Err(e) => tracing::warn!(error = %e, "could not check the local engine"),
            }
        }
    });
}

/// A started server. [`RunningServer::shutdown`] drains and stops it; dropping it
/// stops it too, without the drain.
pub struct RunningServer {
    /// Address the HTTP API and the `/rivet` gateway are bound to.
    pub http_addr: SocketAddr,
    /// Engine endpoint the actors and the server's client use (loopback in
    /// local mode).
    pub engine_endpoint: String,
    /// Rivet namespace.
    pub namespace: String,
    health: Health,
    pool: String,
    registry_grace: Duration,
    client: Arc<Client>,
    supervision: Supervision,
    /// Cancelled when the HTTP drain runs out: open requests get `503`.
    hard_stop: CancellationToken,
    http: Option<Background<std::io::Result<()>>>,
    registry: Option<Background<Result<()>>>,
    engine: Arc<Mutex<Option<LocalEngine>>>,
    _lease: process::Lease,
}

impl std::fmt::Debug for RunningServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningServer")
            .field("http_addr", &self.http_addr)
            .field("engine_endpoint", &self.engine_endpoint)
            .field("namespace", &self.namespace)
            .field("engine_pid", &self.engine_pid())
            .finish_non_exhaustive()
    }
}

impl RunningServer {
    /// `http://<host>:<port>` of the HTTP API (the loopback address of the same
    /// family when bound to an unspecified one: `127.0.0.1` or `[::1]`).
    pub fn url(&self) -> String {
        loopback_url(self.http_addr)
    }

    /// The readiness and draining switch behind `/health`.
    pub fn health(&self) -> &Health {
        &self.health
    }

    /// Process id of the spawned engine (`None` for an external engine, or once
    /// shutdown took it).
    pub fn engine_pid(&self) -> Option<u32> {
        self.engine.lock().as_ref().and_then(LocalEngine::pid)
    }

    /// Resolves when the server stopped serving by itself: the spawned engine
    /// exited or the actor runner stopped. `/health` answers `503` from then on;
    /// call [`RunningServer::shutdown`] and start a new server. Never resolves
    /// while the server is healthy.
    pub async fn closed(&self) -> ServerFailure {
        self.supervision.closed.cancelled().await;
        self.supervision
            .failure
            .get()
            .cloned()
            .unwrap_or_else(|| ServerFailure::RunnerStopped {
                reason: "unknown".to_owned(),
            })
    }

    /// The failure [`RunningServer::closed`] reports, if one happened yet.
    pub fn failure(&self) -> Option<ServerFailure> {
        self.supervision.failure.get().cloned()
    }

    /// Drain and stop: HTTP (in-flight requests finish), then the registry, then
    /// the spawned engine, awaiting each.
    pub async fn shutdown(mut self) -> Result<()> {
        self.stop().await
    }

    async fn stop(&mut self) -> Result<()> {
        self.supervision.stopping.cancel();
        self.health.start_draining();
        if let Some(mut http) = self.http.take() {
            http.cancel.cancel();
            let drained = tokio::time::timeout(HTTP_DRAIN_TIMEOUT, &mut http.handle).await;
            let joined = match drained {
                Ok(joined) => Some(joined),
                Err(_) => {
                    tracing::warn!(
                        after = ?HTTP_DRAIN_TIMEOUT,
                        "HTTP requests still open after the drain; ending them with 503"
                    );
                    self.hard_stop.cancel();
                    tokio::time::timeout(HTTP_CLOSE_TIMEOUT, &mut http.handle)
                        .await
                        .ok()
                }
            };
            match joined {
                Some(Ok(Ok(()))) => {}
                Some(Ok(Err(e))) => tracing::warn!(error = %e, "HTTP server stopped with an error"),
                Some(Err(e)) => tracing::warn!(error = %e, "HTTP server task failed"),
                None => {
                    tracing::warn!(
                        after = ?HTTP_CLOSE_TIMEOUT,
                        "HTTP connections still open; leaving them to the process exit"
                    );
                    http.handle.abort();
                }
            }
        }
        if let Some(mut registry) = self.registry.take() {
            registry.cancel.cancel();
            let grace = if self.supervision.engine_gone() {
                REGISTRY_GRACE_ENGINE_GONE.min(self.registry_grace)
            } else {
                self.registry_grace
            };
            match tokio::time::timeout(grace, &mut registry.handle).await {
                Ok(joined) => log_registry_exit(joined),
                Err(_) => {
                    tracing::warn!(after = ?grace, "registry did not stop in time");
                    registry.handle.abort();
                }
            }
        }
        process::table_client().unbind();
        let engine = self.engine.lock().take();
        if let Some(engine) = engine {
            engine.shutdown().await.context("stop the local engine")?;
        }
        Ok(())
    }

    async fn wait_for_runner(&mut self) -> Result<()> {
        enum Outcome {
            Ready(Result<()>),
            Exited(Result<Result<()>, JoinError>),
        }
        let client = self.client.clone();
        let pool = self.pool.clone();
        let probe = move || {
            let client = client.clone();
            let pool = pool.clone();
            async move { ping(&client, &pool).await }
        };
        let Some(registry) = self.registry.as_mut() else {
            bail!("the actor runner is not running");
        };
        let outcome = tokio::select! {
            ready = wait_ready(READY_TIMEOUT, probe) => Outcome::Ready(ready),
            exited = &mut registry.handle => Outcome::Exited(exited),
        };
        match outcome {
            Outcome::Ready(ready) => ready.context(BootError::NotReady(READY_TIMEOUT)),
            Outcome::Exited(exited) => {
                self.registry = None;
                match exited {
                    Ok(Ok(())) => bail!("the actor runner stopped during boot"),
                    Ok(Err(e)) => Err(e.context("the actor runner failed to start")),
                    Err(e) => bail!("the actor runner task failed: {e}"),
                }
            }
        }
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.supervision.stopping.cancel();
        let engine = self.engine.lock().take();
        let live = self.http.is_some() || self.registry.is_some() || engine.is_some();
        if !live {
            return;
        }
        self.health.start_draining();
        self.hard_stop.cancel();
        if let Some(http) = self.http.take() {
            http.cancel.cancel();
            http.handle.abort();
        }
        if let Some(registry) = self.registry.take() {
            registry.cancel.cancel();
            registry.handle.abort();
        }
        process::table_client().unbind();
        // LocalEngine's Drop stops the engine (SIGTERM, then SIGKILL after 15 s).
        drop(engine);
    }
}

fn log_registry_exit(joined: Result<Result<()>, JoinError>) {
    match joined {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!(error = %format!("{e:#}"), "registry stopped with an error"),
        Err(e) => tracing::warn!(error = %e, "registry task failed"),
    }
}

/// Start the server with default [`StartOptions`] (no static web build) and
/// return once it serves requests.
///
/// The desktop calls `start(ServerConfig::local(app_data_dir))`. Boot errors a
/// caller can act on carry a [`BootError`] (`err.downcast_ref::<BootError>()`).
pub async fn start(cfg: ServerConfig) -> Result<RunningServer> {
    start_with(cfg, StartOptions::default()).await
}

/// [`start`] with explicit [`StartOptions`].
pub async fn start_with(mut cfg: ServerConfig, options: StartOptions) -> Result<RunningServer> {
    let lease = process::acquire()?;
    let spawn = cfg.engine.mode == EngineMode::Spawn;
    check_engine_env(&cfg, spawn)?;
    let token = match cfg.engine.token.clone() {
        Some(token) => token,
        None if spawn => Secret::new(random_secret()),
        None => bail!("RIVET_TOKEN must be set to reach an external engine"),
    };
    if spawn {
        cfg.engine.endpoint = format!("http://127.0.0.1:{}", cfg.engine.engine_port);
        cfg.engine.namespace = LOCAL_ENGINE_NAMESPACE.to_owned();
    }
    cfg.engine.token = Some(token.clone());
    let local = spawn && !cfg.production;

    let listener = process::bind_http(cfg.http.bind, local).await?;
    let http_addr = listener.local_addr().context("HTTP listener address")?;
    if local {
        allow_loopback(&mut cfg.origins, http_addr)?;
    }

    let auth = Authenticator::from_config(&cfg);
    let mut registry = Registry::new();
    euchre_profile::register(&mut registry, auth.clone()).map_err(|_| BootError::KeysConflict)?;
    let mut table = TableConfig::from_server_config(&cfg);
    table.client = process::table_client();
    euchre_table::register(&mut registry, table).map_err(|_| BootError::TableConfigConflict)?;
    let client = build_client_with_pool(
        &cfg.engine.endpoint,
        &cfg.engine.namespace,
        Some(token.expose()),
        Some(&cfg.engine.pool_name),
    );
    euchre_ai_seat::register(
        &mut registry,
        AiSeatRuntime::from_config(SeatClient::new(client.clone()), &cfg),
    );
    health_actor::register(&mut registry);

    let engine = if spawn {
        let storage_path = match (&cfg.engine.storage_path, &cfg.data_dir) {
            (Some(path), _) => path.clone(),
            (None, Some(dir)) => dir.join("engine"),
            (None, None) => std::env::current_dir().context("working directory")?,
        };
        Some(
            engine::spawn(SpawnSpec {
                port: cfg.engine.engine_port,
                storage_path,
                token: token.clone(),
                binary_path: cfg.engine.binary_path.clone(),
            })
            .await?,
        )
    } else {
        None
    };

    let health = Health::new();
    let supervision = Supervision::new(health.clone());
    let engine = Arc::new(Mutex::new(engine));
    if engine.lock().is_some() {
        watch_engine(engine.clone(), supervision.clone());
    }
    let mut server = RunningServer {
        http_addr,
        engine_endpoint: cfg.engine.endpoint.clone(),
        namespace: cfg.engine.namespace.clone(),
        health,
        pool: cfg.engine.pool_name.clone(),
        registry_grace: if cfg.production || !spawn {
            REGISTRY_GRACE_DEPLOYED
        } else {
            REGISTRY_GRACE_LOCAL
        },
        client,
        supervision,
        hard_stop: CancellationToken::new(),
        http: None,
        registry: None,
        engine,
        _lease: lease,
    };
    match finish_boot(&mut server, &cfg, registry, auth, listener, options, spawn).await {
        Ok(()) => {
            tracing::info!(
                http = %server.http_addr,
                engine = %server.engine_endpoint,
                namespace = %server.namespace,
                "euchre server ready"
            );
            Ok(server)
        }
        Err(e) => {
            if let Err(stop) = server.stop().await {
                tracing::warn!(error = %format!("{stop:#}"), "cleanup after a failed boot");
            }
            Err(e)
        }
    }
}

async fn finish_boot(
    server: &mut RunningServer,
    cfg: &ServerConfig,
    registry: Registry,
    auth: Authenticator,
    listener: TcpListener,
    options: StartOptions,
    spawn: bool,
) -> Result<()> {
    let Some(token) = cfg.engine.token.clone() else {
        bail!("no engine token");
    };
    bootstrap(cfg, &token, spawn).await?;

    let serve = serve_config(cfg, &token, spawn)?;
    let cancel = CancellationToken::new();
    let shutdown = cancel.clone();
    let supervision = server.supervision.clone();
    server.registry = Some(Background {
        cancel,
        handle: tokio::spawn(async move {
            let mut watch = RunnerWatch {
                supervision,
                reason: None,
            };
            let served = registry.serve_with_config(serve, shutdown).await;
            watch.reason = Some(match &served {
                Ok(()) => "it returned".to_owned(),
                Err(e) => format!("{e:#}"),
            });
            served
        }),
    });
    server.wait_for_runner().await?;
    process::table_client().bind(server.client.clone());

    let mut api = ApiConfig::from_server_config(cfg);
    api.engine_token = Some(token);
    api.static_dir = static_dir(&options);
    api.downloads_dir = options.downloads_dir.clone();
    api.downloads_bucket = options.downloads_bucket.clone();
    let state = ApiState::new(api, auth, server.client.clone(), server.health.clone())
        .context("build the HTTP state")?;
    let gate = Gate {
        health: server.health.clone(),
        hard_stop: server.hard_stop.clone(),
    };
    let app = euchre_http::router(state).layer(middleware::from_fn_with_state(gate, drain_gate));
    server.health.set_ready();
    let cancel = CancellationToken::new();
    let signal = cancel.clone();
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move { signal.cancelled().await })
        .await
    });
    server.http = Some(Background { cancel, handle });
    Ok(())
}

/// TS `assertRivetEnvSafety` (self-hosted for an external engine).
fn check_engine_env(cfg: &ServerConfig, spawn: bool) -> Result<()> {
    assert_rivet_env_safety(RivetEnv {
        endpoint: Some(cfg.engine.endpoint.as_str()),
        public_endpoint: cfg.engine.public_endpoint.as_deref(),
        token: cfg.engine.token.as_ref().map(Secret::expose),
        namespace: Some(cfg.engine.namespace.as_str()),
        selfhosted: !spawn,
    })
    .map_err(anyhow::Error::new)
}

/// Allow the server's own loopback origins so its `Host` resolves (else `421`).
/// A host that already maps to a configured origin keeps it.
fn allow_loopback(origins: &mut OriginPolicy, addr: SocketAddr) -> Result<()> {
    let port = addr.port();
    let mut hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if addr.is_ipv6() {
        hosts.push(format!("[::1]:{port}"));
    }
    for host in hosts {
        if origins.public_origin_for_host(&host).is_none() {
            origins
                .allow(&format!("http://{host}"))
                .with_context(|| format!("allow the loopback origin http://{host}"))?;
        }
    }
    Ok(())
}

/// Namespace and runner pool, engine 3 dialect (05 §5.4).
async fn bootstrap(cfg: &ServerConfig, token: &Secret, spawn: bool) -> Result<()> {
    let mut builder = reqwest::Client::builder();
    if spawn {
        builder = builder.no_proxy();
    }
    let http = builder.build().context("build the engine admin client")?;
    let opts = EnsureRivetNamespaceOptions::new(
        cfg.engine.endpoint.clone(),
        token.expose(),
        cfg.engine.namespace.clone(),
        cfg.engine.pool_name.clone(),
    );
    let done = ensure_rivet_namespace(&http, &opts).await.map_err(|e| {
        anyhow::Error::new(e).context("ensure the engine namespace and runner pool")
    })?;
    tracing::info!(
        namespace = %cfg.engine.namespace,
        pool = %cfg.engine.pool_name,
        created = done.namespace_created,
        attempts = done.attempts,
        "engine namespace and runner pool ready"
    );
    Ok(())
}

/// `ServeConfig::from_env()` with this server's engine on top (G31). The engine
/// is never spawned or downloaded by rivetkit: a spawned one is ours already.
fn serve_config(cfg: &ServerConfig, token: &Secret, spawn: bool) -> Result<ServeConfig> {
    let mut config = ServeConfig::from_env().context("ServeConfig::from_env")?;
    config.endpoint = cfg.engine.endpoint.clone();
    config.token = Some(token.expose().to_owned());
    config.namespace = cfg.engine.namespace.clone();
    config.pool_name = cfg.engine.pool_name.clone();
    config.engine_spawn = EngineSpawnMode::Never;
    config.engine_auto_download = false;
    config.start_services = false;
    if spawn {
        config.engine_host = Some("127.0.0.1".to_owned());
        config.engine_port = Some(cfg.engine.engine_port);
        config.version = process::next_version(0);
    } else {
        config.version = cfg.engine.version;
    }
    Ok(config)
}

fn static_dir(options: &StartOptions) -> Option<PathBuf> {
    let dir = options.static_dir.as_ref()?;
    if dir.is_dir() {
        Some(dir.clone())
    } else {
        tracing::info!(dir = %dir.display(), "no web build there; serving the API only");
        None
    }
}

/// One `health.ping` through the shared client.
async fn ping(client: &Client, pool: &str) -> Result<()> {
    let handle = client.get_or_create(
        HEALTH_ACTOR,
        health_key(),
        GetOrCreateOptions {
            pool_name: Some(pool.to_owned()),
            ..GetOrCreateOptions::default()
        },
    )?;
    let reply = tokio::time::timeout(PING_TIMEOUT, handle.action(Ping::NAME, vec![]))
        .await
        .context("health.ping timed out")??;
    if reply.get("ok") == Some(&Value::Bool(true)) {
        Ok(())
    } else {
        bail!("unexpected health.ping reply: {reply}")
    }
}

/// `http://` plus `addr`, with an unspecified IP swapped for the loopback
/// address of its family.
fn loopback_url(addr: SocketAddr) -> String {
    let ip = match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    format!("http://{}", SocketAddr::new(ip, addr.port()))
}

/// The drain switches [`drain_gate`] reads.
#[derive(Clone)]
struct Gate {
    health: Health,
    hard_stop: CancellationToken,
}

/// While draining, refuse new tables; everything else (sessions for existing
/// tables, profile reads, in-flight requests) still runs, until the drain runs
/// out: then a request still open gets `503` ([`RESTARTING`]) and its
/// connection closes.
async fn drain_gate(State(gate): State<Gate>, req: Request, next: Next) -> Response {
    if gate.health.is_draining()
        && req.method() == Method::POST
        && req.uri().path() == "/api/tables"
    {
        return ApiError::unavailable(DRAINING_NEW_TABLE).into_response();
    }
    tokio::select! {
        res = next.run(req) => res,
        () = gate.hard_stop.cancelled() => ApiError::unavailable(RESTARTING).into_response(),
    }
}

/// Start, then run until SIGTERM or ctrl-c, then drain and stop. The deployed
/// binary's entry point ([`StartOptions::deployed`]).
///
/// Returns an error (a non-zero exit) when the server stopped serving by itself
/// ([`RunningServer::closed`]), or when a second signal cut the drain short.
pub async fn run(cfg: ServerConfig) -> Result<()> {
    let server = start_with(cfg, StartOptions::deployed()).await?;
    let failure = tokio::select! {
        () = shutdown_signal() => None,
        failure = server.closed() => Some(failure),
    };
    match &failure {
        None => tracing::info!("shutting down; a second SIGTERM or ctrl-c stops at once"),
        Some(_) => tracing::info!("shutting down after a failure"),
    }
    let stopped = {
        let stop = server.shutdown();
        tokio::pin!(stop);
        tokio::select! {
            stopped = &mut stop => stopped,
            () = shutdown_signal() => {
                tracing::warn!("second signal; stopping without the drain");
                // Dropping the drain drops the server, which stops the engine.
                Err(anyhow::anyhow!("stopped by a second signal before the drain finished"))
            }
        }
    };
    match failure {
        Some(failure) => match stopped {
            Ok(()) => Err(anyhow::Error::new(failure)),
            Err(e) => Err(e.context(failure)),
        },
        None => stopped,
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "no SIGTERM handler; waiting for ctrl-c only");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use euchre_server_core::auth::AuthKeys;
    use tower::ServiceExt as _;

    fn auth() -> Authenticator {
        Authenticator::new(AuthKeys {
            guest: Secret::new("g".repeat(40)),
            profile: Secret::new("p".repeat(40)),
            internal: Secret::new("i".repeat(40)),
            identity: Secret::new("d".repeat(40)),
            jwt_issuer: None,
            allow_unsigned_email: false,
        })
    }

    /// The composed app on a client that never connects (no engine needed).
    fn app(health: &Health) -> Result<axum::Router> {
        let mut cfg = ServerConfig::local(std::env::temp_dir());
        allow_loopback(&mut cfg.origins, SocketAddr::from(([127, 0, 0, 1], 4100)))?;
        let client = build_client_with_pool("http://127.0.0.1:9", "default", None, None);
        let state = ApiState::new(
            ApiConfig::from_server_config(&cfg),
            auth(),
            client,
            health.clone(),
        )?;
        Ok(app_with(state, health, &CancellationToken::new()))
    }

    fn app_with(state: ApiState, health: &Health, hard_stop: &CancellationToken) -> axum::Router {
        euchre_http::router(state).layer(middleware::from_fn_with_state(
            Gate {
                health: health.clone(),
                hard_stop: hard_stop.clone(),
            },
            drain_gate,
        ))
    }

    fn request(method: Method, path: &str) -> Result<Request> {
        Ok(Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:4100")
            .body(Body::empty())?)
    }

    async fn status_and_body(app: axum::Router, req: Request) -> Result<(StatusCode, String)> {
        let res = app.oneshot(req).await?;
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await?;
        Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
    }

    #[tokio::test]
    async fn draining_refuses_new_tables_and_reports_draining() -> Result<()> {
        let health = Health::new();
        health.set_ready();
        let (status, body) =
            status_and_body(app(&health)?, request(Method::GET, "/health")?).await?;
        assert_eq!((status, body.as_str()), (StatusCode::OK, "ok"));
        // Serving: the create reaches the handler (no session: 401).
        let (status, _) =
            status_and_body(app(&health)?, request(Method::POST, "/api/tables")?).await?;
        assert_ne!(status, StatusCode::SERVICE_UNAVAILABLE);

        health.start_draining();
        let (status, body) =
            status_and_body(app(&health)?, request(Method::GET, "/health")?).await?;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::SERVICE_UNAVAILABLE, "draining")
        );
        let (status, body) =
            status_and_body(app(&health)?, request(Method::POST, "/api/tables")?).await?;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let json: Value = serde_json::from_str(&body)?;
        assert_eq!(json["message"], DRAINING_NEW_TABLE);
        assert_eq!(json["error"]["code"], "unavailable");
        // Other routes still answer while draining.
        let (status, _) =
            status_and_body(app(&health)?, request(Method::GET, "/api/rivet/metadata")?).await?;
        assert_eq!(status, StatusCode::OK);
        Ok(())
    }

    #[tokio::test]
    async fn the_loopback_host_resolves_and_others_get_421() -> Result<()> {
        let health = Health::new();
        health.set_ready();
        let (status, body) =
            status_and_body(app(&health)?, request(Method::GET, "/api/rivet/metadata")?).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        let json: Value = serde_json::from_str(&body)?;
        assert_eq!(json["clientEndpoint"], "http://127.0.0.1:4100/rivet");
        let req = Request::builder()
            .uri("/api/rivet/metadata")
            .header("host", "evil.example:4100")
            .body(Body::empty())?;
        let (status, _) = status_and_body(app(&health)?, req).await?;
        assert_eq!(status, StatusCode::MISDIRECTED_REQUEST);
        Ok(())
    }

    #[test]
    fn loopback_origins_keep_a_configured_host() -> Result<()> {
        let mut origins = OriginPolicy::strict(vec!["https://localhost:4100".to_owned()])?;
        allow_loopback(&mut origins, SocketAddr::from(([127, 0, 0, 1], 4100)))?;
        assert_eq!(
            origins.public_origin_for_host("localhost:4100"),
            Some("https://localhost:4100")
        );
        assert_eq!(
            origins.public_origin_for_host("127.0.0.1:4100"),
            Some("http://127.0.0.1:4100")
        );
        assert!(origins.check(Some("http://127.0.0.1:4100")));
        assert!(!origins.check(Some("http://127.0.0.1:4101")));
        Ok(())
    }

    /// A request still open when the drain runs out gets `503` instead of
    /// holding the connection.
    #[tokio::test]
    async fn the_hard_stop_ends_open_requests_with_503() -> Result<()> {
        let health = Health::new();
        health.set_ready();
        let hard_stop = CancellationToken::new();
        let gate = Gate {
            health: health.clone(),
            hard_stop: hard_stop.clone(),
        };
        let app = axum::Router::new()
            .route(
                "/slow",
                axum::routing::get(|| async {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    "late"
                }),
            )
            .layer(middleware::from_fn_with_state(gate, drain_gate));
        let pending = tokio::spawn(status_and_body(app, request(Method::GET, "/slow")?));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!pending.is_finished());
        hard_stop.cancel();
        let (status, body) = tokio::time::timeout(Duration::from_secs(5), pending).await???;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let json: Value = serde_json::from_str(&body)?;
        assert_eq!(json["message"], RESTARTING);
        Ok(())
    }

    #[test]
    fn the_url_uses_the_loopback_of_the_bound_family() {
        assert_eq!(
            loopback_url(SocketAddr::from(([0, 0, 0, 0], 3000))),
            "http://127.0.0.1:3000"
        );
        assert_eq!(
            loopback_url(SocketAddr::from((Ipv6Addr::UNSPECIFIED, 3000))),
            "http://[::1]:3000"
        );
        assert_eq!(
            loopback_url(SocketAddr::from(([127, 0, 0, 1], 4100))),
            "http://127.0.0.1:4100"
        );
    }

    /// The first failure wins, flips `/health` and resolves `closed`; nothing is
    /// reported once shutdown started.
    #[tokio::test]
    async fn the_first_failure_closes_the_server() -> Result<()> {
        let health = Health::new();
        health.set_ready();
        let supervision = Supervision::new(health.clone());
        drop(RunnerWatch {
            supervision: supervision.clone(),
            reason: Some("boom".to_owned()),
        });
        assert!(!health.is_serving());
        assert!(supervision.closed.is_cancelled());
        supervision.report(ServerFailure::EngineExited {
            status: "signal: 9".to_owned(),
        });
        assert_eq!(
            supervision.failure.get(),
            Some(&ServerFailure::RunnerStopped {
                reason: "boom".to_owned()
            })
        );
        assert!(!supervision.engine_gone());

        let quiet = Supervision::new(Health::new());
        quiet.stopping.cancel();
        quiet.report(ServerFailure::EngineExited {
            status: "exit status: 0".to_owned(),
        });
        assert!(quiet.failure.get().is_none());
        assert!(!quiet.closed.is_cancelled());
        Ok(())
    }

    #[test]
    fn deployed_options_point_at_the_web_build() {
        assert_eq!(
            StartOptions::deployed().static_dir,
            Some(PathBuf::from("build/client"))
        );
        assert_eq!(StartOptions::default().static_dir, None);
        assert_eq!(StartOptions::default().downloads_dir, None);
        assert!(StartOptions::deployed().downloads_dir.is_some());
        assert_eq!(
            static_dir(&StartOptions {
                static_dir: Some(PathBuf::from("/definitely/not/here")),
                ..StartOptions::default()
            }),
            None
        );
    }
}
