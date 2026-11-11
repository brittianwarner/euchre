//! Engine-backed test harness (ARCHITECTURE §5). Owned by lane SERVER-FOUNDATION.
//!
//! One test binary shares **one** private `rivet-engine` ([`crate::rivet::LocalEngine`])
//! on its lane's port and storage. Each [`TestEngine::start`] leases that engine
//! (starting it for the first lease), serves its own registry on its own runner pool
//! (a fresh `test-<uuid>` pool by default), and exposes the endpoint, namespace,
//! admin token and one shared [`Client`] (G26) for that pool. So the `#[tokio::test]`s
//! of a binary can run in parallel against one engine. Use unique actor keys per test
//! (for example with a uuid) so tests never share actors.
//!
//! Dropping a `TestEngine`, or [`TestEngine::shutdown`], stops its registry and
//! releases its lease; the last lease stops the engine (and removes a temp storage
//! dir). Nothing else is ever stopped. An engine orphaned by a killed run is reclaimed
//! by the next start on the same storage (see [`crate::rivet::LocalEngine::start`]).
//!
//! Environment (used by [`TestEngineOptions::from_env`]):
//!
//! | Var | Default |
//! |---|---|
//! | `EUCHRE_E2E` | unset: engine tests print `SKIP` and return (see [`e2e_enabled`]) |
//! | `EUCHRE_TEST_ENGINE_PORT` | `6751` |
//! | `EUCHRE_TEST_STORAGE` | a fresh dir under the system temp dir, removed when the engine stops |
//! | `RIVET_ENGINE_BINARY_PATH` | the rivetkit cache (`~/.rivetkit/engine/v3/3.1.0/…`), else auto-download |
//!
//! Engine tests are plain `#[tokio::test]`s that start with
//! `if !e2e_enabled() { e2e_skip("name"); return Ok(()); }`, so a plain `cargo test`
//! stays engine-free and prints an explicit `SKIP` line, and
//! `EUCHRE_E2E=1 cargo test -p euchre-server --test <file>` runs them.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use rivetkit::client::Client;
use rivetkit::{EngineSpawnMode, Registry, ServeConfig};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::Secret;
use crate::rivet::{
    LOCAL_ENGINE_NAMESPACE, LocalEngine, LocalEngineOptions, build_client_with_pool,
};

/// Default engine port for integration tests (the INTEGRATION lane's port).
pub const DEFAULT_TEST_ENGINE_PORT: u16 = 6751;

/// True when `EUCHRE_E2E=1`: engine tests run only then.
pub fn e2e_enabled() -> bool {
    std::env::var("EUCHRE_E2E").is_ok_and(|v| v == "1")
}

/// Print the explicit skip line for an engine test that did not run.
pub fn e2e_skip(test: &str) {
    eprintln!("SKIP {test}: engine test; set EUCHRE_E2E=1 to run it");
}

/// Registers the actors under test.
pub type RegistryBuilder = Box<dyn FnOnce(&mut Registry) + Send>;

/// How to start a [`TestEngine`].
pub struct TestEngineOptions {
    /// Engine guard port (the engine also binds `port + 1` and `port + 5`). Every
    /// `TestEngine` alive at once in a process must use the same port and storage.
    pub port: u16,
    /// rivetkit storage path; `None` uses a fresh temp dir that is removed when the
    /// shared engine stops.
    pub storage_path: Option<PathBuf>,
    /// Registers the actors to serve.
    pub actors: RegistryBuilder,
    /// Runner pool this registry serves and the client targets. Unique per test unless
    /// two tests deliberately share a registry's actors.
    pub pool_name: String,
}

impl std::fmt::Debug for TestEngineOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestEngineOptions")
            .field("port", &self.port)
            .field("storage_path", &self.storage_path)
            .field("pool_name", &self.pool_name)
            .finish_non_exhaustive()
    }
}

impl TestEngineOptions {
    /// Options from `EUCHRE_TEST_ENGINE_PORT` / `EUCHRE_TEST_STORAGE`, with a fresh
    /// `test-<uuid>` pool.
    pub fn from_env(actors: impl FnOnce(&mut Registry) + Send + 'static) -> Result<Self> {
        let port = match std::env::var("EUCHRE_TEST_ENGINE_PORT") {
            Ok(v) => v
                .parse()
                .with_context(|| format!("EUCHRE_TEST_ENGINE_PORT `{v}` is not a port"))?,
            Err(_) => DEFAULT_TEST_ENGINE_PORT,
        };
        let storage_path = std::env::var_os("EUCHRE_TEST_STORAGE")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        Ok(Self {
            port,
            storage_path,
            actors: Box::new(actors),
            pool_name: format!("test-{}", uuid::Uuid::new_v4().simple()),
        })
    }
}

// ------------------------------------------------------------------ shared engine

/// The process-wide engine and its lease count.
struct Shared {
    engine: LocalEngine,
    requested_storage: Option<PathBuf>,
    temp_storage: Option<PathBuf>,
    users: usize,
}

/// Guarded by a blocking mutex so the last lease can stop the engine from `Drop`. The
/// engine stop (bounded, 15 s) runs while it is held, so a concurrent first lease
/// waits for the old engine to exit before starting a new one on the same port.
static SHARED: parking_lot::Mutex<Option<Shared>> = parking_lot::const_mutex(None);
/// Serializes engine starts (held across the async start, never inside `SHARED`).
static STARTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// What a lease hands to its `TestEngine`.
struct Lease {
    endpoint: String,
    token: Secret,
    pid: Option<u32>,
}

async fn acquire(port: u16, storage: Option<PathBuf>) -> Result<Lease> {
    let _starting = STARTING.lock().await;
    {
        let mut shared = SHARED.lock();
        if let Some(s) = shared.as_mut() {
            if s.engine.port() != port || s.requested_storage != storage {
                bail!(
                    "the shared test engine runs on port {} with storage {:?}; a TestEngine asked for port {port} with storage {storage:?}",
                    s.engine.port(),
                    s.requested_storage
                );
            }
            s.users += 1;
            return Ok(lease_of(&s.engine));
        }
    }
    let (storage_path, temp_storage) = match &storage {
        Some(p) => (p.clone(), None),
        None => {
            let p =
                std::env::temp_dir().join(format!("euchre-test-engine-{}", uuid::Uuid::new_v4()));
            (p.clone(), Some(p))
        }
    };
    std::fs::create_dir_all(&storage_path)
        .with_context(|| format!("create {}", storage_path.display()))?;
    let engine = match LocalEngine::start(LocalEngineOptions::new(port, &storage_path)).await {
        Ok(engine) => engine,
        Err(e) => {
            if let Some(dir) = temp_storage {
                let _ = std::fs::remove_dir_all(dir);
            }
            return Err(e);
        }
    };
    let lease = lease_of(&engine);
    *SHARED.lock() = Some(Shared {
        engine,
        requested_storage: storage,
        temp_storage,
        users: 1,
    });
    Ok(lease)
}

fn lease_of(engine: &LocalEngine) -> Lease {
    Lease {
        endpoint: engine.endpoint().to_owned(),
        token: engine.admin_token().clone(),
        pid: engine.pid(),
    }
}

/// Drop one lease; the last one stops the engine (blocking, bounded) and removes a
/// temp storage dir.
fn release_blocking() {
    let mut shared = SHARED.lock();
    let last = match shared.as_mut() {
        Some(s) => {
            s.users = s.users.saturating_sub(1);
            s.users == 0
        }
        None => false,
    };
    if last && let Some(s) = shared.take() {
        drop(s.engine);
        if let Some(dir) = s.temp_storage {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

// ------------------------------------------------------------------ TestEngine

/// A leased engine plus a registry served against it on its own pool.
pub struct TestEngine {
    endpoint: String,
    namespace: String,
    pool_name: String,
    token: Secret,
    engine_pid: Option<u32>,
    client: Arc<Client>,
    shutdown: CancellationToken,
    serve: Option<JoinHandle<Result<()>>>,
    leased: bool,
}

impl std::fmt::Debug for TestEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestEngine")
            .field("endpoint", &self.endpoint)
            .field("namespace", &self.namespace)
            .field("pool_name", &self.pool_name)
            .field("engine_pid", &self.engine_pid)
            .finish_non_exhaustive()
    }
}

impl TestEngine {
    /// Lease the shared engine (starting it, healthy and ready, on first use), then
    /// serve the registry on `options.pool_name`.
    ///
    /// The registry is served from `ServeConfig::from_env()` (G31) with the endpoint,
    /// the engine's admin token, namespace, pool, a fresh version and
    /// `EngineSpawnMode::Never` set on top: the engine is already ours, and `Always`
    /// would let rivetkit spawn a second engine on the shared default storage if its
    /// probe ever missed ours.
    pub async fn start(options: TestEngineOptions) -> Result<Self> {
        let lease = acquire(options.port, options.storage_path).await?;
        let mut me = Self {
            client: build_client_with_pool(
                &lease.endpoint,
                LOCAL_ENGINE_NAMESPACE,
                Some(lease.token.expose()),
                Some(&options.pool_name),
            ),
            endpoint: lease.endpoint,
            namespace: LOCAL_ENGINE_NAMESPACE.to_owned(),
            pool_name: options.pool_name,
            token: lease.token,
            engine_pid: lease.pid,
            shutdown: CancellationToken::new(),
            serve: None,
            leased: true,
        };

        let mut registry = Registry::new();
        (options.actors)(&mut registry);
        let mut config = ServeConfig::from_env().context("ServeConfig::from_env")?;
        config.endpoint = me.endpoint.clone();
        config.token = Some(me.token.expose().to_owned());
        config.namespace = LOCAL_ENGINE_NAMESPACE.to_owned();
        config.pool_name = me.pool_name.clone();
        config.engine_spawn = EngineSpawnMode::Never;
        config.engine_host = Some("127.0.0.1".to_owned());
        config.engine_port = Some(options.port);
        config.engine_auto_download = false;
        config.start_services = false;
        config.version = u32::try_from(crate::auth::now_secs()).unwrap_or(u32::MAX);

        let token = me.shutdown.clone();
        me.serve = Some(tokio::spawn(async move {
            registry.serve_with_config(config, token).await
        }));
        // Fail fast when the registry could not even start (bad config, port, ...).
        tokio::time::sleep(Duration::from_millis(200)).await;
        if me.serve.as_ref().is_some_and(JoinHandle::is_finished) {
            let res = match me.serve.take() {
                Some(h) => h.await,
                None => Ok(Ok(())),
            };
            me.release().await;
            match res {
                Ok(Ok(())) => bail!("registry exited immediately"),
                Ok(Err(e)) => return Err(e.context("registry failed to start")),
                Err(e) => bail!("registry task panicked: {e}"),
            }
        }
        Ok(me)
    }

    /// `http://127.0.0.1:<port>`.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Engine namespace (`default`).
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Pool the registry serves.
    pub fn pool_name(&self) -> &str {
        &self.pool_name
    }

    /// The engine's admin token (random per engine start, never `default`).
    pub fn token(&self) -> &Secret {
        &self.token
    }

    /// The shared client for this pool (JSON encoding). Clone the `Arc`, never the
    /// `Client` (G26).
    pub fn client(&self) -> Arc<Client> {
        self.client.clone()
    }

    /// Process id of the shared engine, while running.
    pub fn engine_pid(&self) -> Option<u32> {
        self.engine_pid
    }

    /// Retry `probe` (typically a ping action) until the envoy serves it (G36), for up
    /// to 60 s. A `user/*` error ends it at once (see [`crate::rivet::wait_ready`]).
    pub async fn ready<T, F, Fut>(&self, probe: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        crate::rivet::wait_ready(Duration::from_secs(60), probe).await
    }

    /// Stop the registry, then release the engine lease (the last lease stops it).
    pub async fn shutdown(mut self) -> Result<()> {
        self.shutdown.cancel();
        if let Some(serve) = self.serve.take() {
            match tokio::time::timeout(Duration::from_secs(20), serve).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(e))) => {
                    tracing::warn!(error = %format!("{e:#}"), "registry exited with error")
                }
                Ok(Err(e)) => tracing::warn!(%e, "registry task panicked"),
                Err(_) => tracing::warn!("registry did not stop within 20 s"),
            }
        }
        self.release().await;
        Ok(())
    }

    async fn release(&mut self) {
        if std::mem::take(&mut self.leased)
            && let Err(e) = tokio::task::spawn_blocking(release_blocking).await
        {
            tracing::warn!(%e, "engine lease release panicked");
        }
    }
}

impl Drop for TestEngine {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if let Some(serve) = self.serve.take() {
            serve.abort();
        }
        if std::mem::take(&mut self.leased) {
            // Blocking and bounded (15 s) when this is the last lease.
            release_blocking();
        }
    }
}
