//! The HTTP API + rivetkit-client backend (D28, lane N1). It serves both modes: a remote
//! server (`EUCHRE_SERVER_URL`) and the embedded one (lane N2 builds it with
//! [`RemoteConfig::embedded`] once `euchre_server::start` returns).
//!
//! The build contract is `docs/port/reports/desktop-net-contract.md` (§N1). In short:
//!
//! - [`http`]: one `reqwest` client, `Authorization: Bearer <device session>` on every
//!   call, each status mapped to a [`NetError`] the player can read.
//! - [`device`]: the device identity in `device.json` (`DeviceRecord`, mode 0600, atomic):
//!   enroll once, refresh at 80 % of its lifetime, enroll again on a `401`. One session per
//!   backend, single-flight; a call that answers `401` recovers once and runs again.
//! - [`table`]: one table connection: the table session mint, the `/rivet` gateway
//!   WebSocket (json), the five events, the four actions and the reconnect supervisor.
//! - [`recent`]: the tables this device opened, kept in `recent-tables.json`.
//!
//! [`RemoteBackend::unconfigured`] (also `Default`) answers [`NetError::NotWired`] for good:
//! it is the backend with no server address. Every call runs on the bridge's tokio thread;
//! file reads and writes go through `spawn_blocking`.

pub mod device;
pub mod http;
pub mod recent;
mod session;
pub mod table;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use euchre_profile_model::{ClientMatchReplay, ProfileSettings, ProfileStats, SettingsPatch};
use euchre_protocol::{MatchListPage, TableSessionResponse, is_game_id};

use self::device::DeviceStore;
use self::http::{Api, wire};
use self::recent::RecentStore;
use self::session::{Creds, Session};
use self::table::{Dialed, Dialer, OPEN_TIMEOUT, TableHandle};
use super::{Backend, Identity, NetError, RecentTable, RecentTables, TableConfig, TableConn};

/// `device.json`'s `serverUrl` for the embedded server. Its loopback port changes every
/// launch, so the record is keyed by this name instead of by the URL.
pub const EMBEDDED_SERVER_KEY: &str = "embedded";

/// Set to `1` to allow a plain `http://` server that is not on this machine (review fix
/// LDS-9). Off by default: the device session would cross the network in the clear.
pub const ALLOW_INSECURE_ENV: &str = "EUCHRE_ALLOW_INSECURE";

/// Whether `host` is this machine (`localhost`, `*.localhost`, `127.0.0.0/8`, `::1`).
fn is_loopback(host: &url::Host<&str>) -> bool {
    match host {
        url::Host::Domain(name) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            name == "localhost" || name.ends_with(".localhost")
        }
        url::Host::Ipv4(ip) => ip.is_loopback(),
        url::Host::Ipv6(ip) => ip.is_loopback(),
    }
}

/// Whether `endpoint` is on the origin `base` (scheme, host and port).
fn same_origin(endpoint: &str, base: &str) -> bool {
    match (url::Url::parse(endpoint), url::Url::parse(base)) {
        (Ok(endpoint), Ok(base)) => endpoint.origin() == base.origin(),
        _ => false,
    }
}

/// Where a [`RemoteBackend`] talks and where it keeps its files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteConfig {
    /// The HTTP API origin, no trailing slash: `http://127.0.0.1:<port>` (embedded,
    /// `RunningServer::url()`) or the normalized `EUCHRE_SERVER_URL`.
    pub base_url: String,
    /// The stable name of the server: `device.json`'s `serverUrl` and the recent-tables
    /// owner. [`EMBEDDED_SERVER_KEY`] for the embedded server, the normalized base URL for a
    /// remote one. A record with another key is never used (the device enrolls again).
    pub server_key: String,
    /// The directory that holds `device.json` and `recent-tables.json` (created 0700 on
    /// first write).
    pub state_dir: PathBuf,
}

/// `base` as one path segment: `"://"` → `"_"`, then every char outside `[A-Za-z0-9.-]` →
/// `"_"` (`http://127.0.0.1:8080` → `http_127.0.0.1_8080`).
fn slug(base: &str) -> String {
    base.replacen("://", "_", 1)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl RemoteConfig {
    /// A remote server at `base_url`; its files live in `<data_dir>/remote/<slug>`, so a
    /// remote identity never replaces the embedded one. Fails on a URL that is not
    /// `http(s)://host[:port]` (a path, query, fragment or credentials are refused), and on
    /// plain `http://` to a host off this machine unless `EUCHRE_ALLOW_INSECURE=1`. The base
    /// URL is the origin: lowercase host, no default port, no trailing slash.
    pub fn remote(base_url: &str, data_dir: &Path) -> Result<RemoteConfig, NetError> {
        let allow_insecure = std::env::var_os(ALLOW_INSECURE_ENV).is_some_and(|v| v == "1");
        Self::remote_with(base_url, data_dir, allow_insecure)
    }

    /// [`RemoteConfig::remote`] with the insecure escape given.
    fn remote_with(
        base_url: &str,
        data_dir: &Path,
        allow_insecure: bool,
    ) -> Result<RemoteConfig, NetError> {
        let refused = || NetError::Invalid("server_url".to_owned());
        let url = url::Url::parse(base_url.trim()).map_err(|_| refused())?;
        let plain = matches!(url.scheme(), "http" | "https")
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(url.path(), "" | "/")
            && url.query().is_none()
            && url.fragment().is_none();
        if !plain {
            return Err(refused());
        }
        let loopback = url.host().is_some_and(|host| is_loopback(&host));
        if url.scheme() == "http" && !loopback && !allow_insecure {
            return Err(NetError::Invalid("insecure_server_url".to_owned()));
        }
        let base = url.origin().ascii_serialization();
        Ok(RemoteConfig {
            state_dir: data_dir.join("remote").join(slug(&base)),
            server_key: base.clone(),
            base_url: base,
        })
    }

    /// The embedded server at `url` (`RunningServer::url()`); its files live in `data_dir`
    /// itself (ARCHITECTURE §2.5: `<data_dir>/device.json`).
    pub fn embedded(url: String, data_dir: &Path) -> RemoteConfig {
        RemoteConfig {
            base_url: url,
            server_key: EMBEDDED_SERVER_KEY.to_owned(),
            state_dir: data_dir.to_path_buf(),
        }
    }
}

/// One configured backend's state, shared by its clones.
struct Inner {
    config: RemoteConfig,
    api: Api,
    session: Session,
    recent: RecentStore,
    /// Serializes `recent-tables.json` read-modify-writes.
    recent_lock: tokio::sync::Mutex<()>,
    /// `GET /api/rivet/metadata`, after its first success.
    metadata: tokio::sync::Mutex<Option<wire::RivetMetadata>>,
    /// The open tables (pruned as they stop).
    tables: parking_lot::Mutex<Vec<TableHandle>>,
}

impl Inner {
    /// Remembers `game_id` in `recent-tables.json` (a failure is logged by the store).
    async fn remember(&self, game_id: &str) {
        let _serial = self.recent_lock.lock().await;
        let store = self.recent.clone();
        let game_id = game_id.to_owned();
        let _ = tokio::task::spawn_blocking(move || store.remember(&game_id)).await;
    }

    /// Drops `game_id` from `recent-tables.json`: the server says it does not exist.
    async fn forget(&self, game_id: &str) {
        let _serial = self.recent_lock.lock().await;
        let store = self.recent.clone();
        let game_id = game_id.to_owned();
        let _ = tokio::task::spawn_blocking(move || store.forget(&game_id)).await;
    }

    /// The remembered ids, newest first.
    async fn remembered(&self) -> Vec<String> {
        let _serial = self.recent_lock.lock().await;
        let store = self.recent.clone();
        tokio::task::spawn_blocking(move || store.load())
            .await
            .unwrap_or_default()
    }

    /// The gateway's address, fetched once. A `clientEndpoint` on another origin is never
    /// dialed (the device session would go to a host the player did not choose): the
    /// server's own `/rivet` is used instead (review fix LDS-9).
    async fn metadata(&self) -> Result<wire::RivetMetadata, NetError> {
        let mut cached = self.metadata.lock().await;
        if let Some(meta) = cached.as_ref() {
            return Ok(meta.clone());
        }
        let mut meta = self.api.rivet_metadata().await?;
        if !same_origin(&meta.client_endpoint, &self.config.base_url) {
            tracing::warn!(
                endpoint = %meta.client_endpoint,
                "gateway address is on another origin; using the server's own"
            );
            meta.client_endpoint = format!("{}/rivet", self.config.base_url);
        }
        *cached = Some(meta.clone());
        Ok(meta)
    }

    /// `GET /api/session` answers this session's own guest id.
    async fn verified(&self, creds: &Creds) -> Result<bool, NetError> {
        match self.api.session(&creds.bearer).await {
            Ok(session) => Ok(session.guest_user_id.as_deref() == Some(creds.user_id.as_str())),
            Err(NetError::Unauthorized) => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// `GET /api/tables/recent?ids=` as the player's rows, in the asked order.
    async fn recent_rows(&self, creds: &Creds, ids: &[String]) -> Result<RecentTables, NetError> {
        let body = self.api.recent_tables(&creds.bearer, ids).await?;
        Ok(RecentTables {
            tables: body
                .tables
                .into_iter()
                .map(|row| RecentTable {
                    game_id: row.game_id,
                    status: row.status,
                    score: row.score,
                    hand_no: row.hand_no,
                })
                .collect(),
            unavailable: body.unavailable,
        })
    }
}

/// The real [`Dialer`] of one table: the backend's session and gateway metadata.
struct TableDialer {
    inner: Arc<Inner>,
    game_id: String,
}

#[async_trait]
impl Dialer for TableDialer {
    /// The table session. A table that does not exist leaves `recent-tables.json` (review
    /// fix PP-1); a `403` after this device was signed out reads as
    /// [`NetError::SignedOut`] (review fix PP-7).
    async fn mint(&self) -> Result<TableSessionResponse, NetError> {
        let api = &self.inner.api;
        let game_id = self.game_id.as_str();
        let minted = self
            .inner
            .session
            .authed(|creds| async move { api.table_session(&creds.bearer, game_id).await })
            .await;
        match minted {
            Err(NetError::NotFound) => {
                self.inner.forget(game_id).await;
                Err(NetError::NotFound)
            }
            Err(NetError::Forbidden) if self.inner.session.was_reset().await => {
                Err(NetError::SignedOut)
            }
            other => other,
        }
    }

    async fn dial(&self, session: &TableSessionResponse) -> Result<Dialed, NetError> {
        let meta = self.inner.metadata().await?;
        table::dial_rivet(&meta, session, OPEN_TIMEOUT).await
    }

    fn now_ms(&self) -> u64 {
        super::wall_ms()
    }
}

/// The remote backend. Cheap to build (no IO): the device enrolls on the first call. Clones
/// share one state (HTTP client, device session, open tables).
#[derive(Clone, Default)]
pub struct RemoteBackend {
    inner: Option<Arc<Inner>>,
}

impl std::fmt::Debug for RemoteBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteBackend")
            .field("config", &self.config())
            .finish_non_exhaustive()
    }
}

impl RemoteBackend {
    /// A backend for `config`. Builds the HTTP client only; never blocks, never enrolls.
    pub fn new(config: RemoteConfig) -> Result<RemoteBackend, NetError> {
        let api = Api::new(&config.base_url)?;
        let store = DeviceStore::new(&config.state_dir, &config.server_key);
        Ok(RemoteBackend {
            inner: Some(Arc::new(Inner {
                session: Session::new(api.clone(), store),
                recent: RecentStore::new(&config.state_dir),
                recent_lock: tokio::sync::Mutex::new(()),
                metadata: tokio::sync::Mutex::new(None),
                tables: parking_lot::Mutex::new(Vec::new()),
                api,
                config,
            })),
        })
    }

    /// The backend with no server address: every call answers [`NetError::NotWired`].
    pub fn unconfigured() -> RemoteBackend {
        RemoteBackend { inner: None }
    }

    /// The configuration, unless unconfigured.
    pub fn config(&self) -> Option<&RemoteConfig> {
        self.inner.as_ref().map(|inner| &inner.config)
    }

    fn inner(&self) -> Result<&Arc<Inner>, NetError> {
        self.inner.as_ref().ok_or(NetError::NotWired)
    }

    /// The open tables' handles (pruning the stopped ones).
    fn live_tables(&self) -> Vec<TableHandle> {
        let Some(inner) = self.inner.as_ref() else {
            return Vec::new();
        };
        let mut tables = inner.tables.lock();
        tables.retain(TableHandle::is_live);
        tables.clone()
    }

    /// Tests: drops every live table socket as a network blip would; each table emits
    /// `Status(false)` and reconnects by itself (the live tests' mid-hand reconnect). Does
    /// nothing with no table open.
    pub fn interrupt_tables(&self) {
        for table in self.live_tables() {
            table.interrupt();
        }
    }

    /// Ends every open table for good: each emits `ConnEvent::Ended(err)`, closes its socket
    /// and stops reconnecting. The embedded supervisor calls it with
    /// [`NetError::ServerStopped`] when the server stops.
    pub fn end_tables(&self, err: NetError) {
        for table in self.live_tables() {
            table.end(err.clone());
        }
    }
}

#[async_trait]
impl Backend for RemoteBackend {
    fn name(&self) -> &'static str {
        "remote"
    }

    async fn identity(&self) -> Result<Identity, NetError> {
        let inner = self.inner()?;
        let creds = inner.session.ensure().await?;
        let creds = if inner.verified(&creds).await? {
            creds
        } else {
            tracing::warn!("device session not verified by the server; recovering");
            let creds = inner.session.recover(&creds.bearer).await?;
            if !inner.verified(&creds).await? {
                return Err(NetError::Unauthorized);
            }
            creds
        };
        Ok(Identity {
            user_id: creds.user_id,
            display_name: None,
        })
    }

    async fn create_table(&self, cfg: TableConfig) -> Result<String, NetError> {
        let inner = self.inner()?;
        let api = &inner.api;
        let body = wire::CreateTableBody {
            stick: cfg.stick_the_dealer,
            natural: cfg.require_natural_trump,
        };
        let game_id = inner
            .session
            .authed(|creds| async move { api.create_table(&creds.bearer, body).await })
            .await?;
        inner.remember(&game_id).await;
        Ok(game_id)
    }

    async fn open_table(&self, game_id: &str) -> Result<TableConn, NetError> {
        let inner = self.inner()?;
        if !is_game_id(game_id) {
            return Err(NetError::NotFound);
        }
        let dialer = Arc::new(TableDialer {
            inner: Arc::clone(inner),
            game_id: game_id.to_owned(),
        });
        let (conn, handle) = table::open(game_id, dialer).await?;
        {
            let mut tables = inner.tables.lock();
            tables.retain(TableHandle::is_live);
            tables.push(handle);
        }
        inner.remember(game_id).await;
        Ok(conn)
    }

    async fn list_games(&self, cursor: Option<String>) -> Result<MatchListPage, NetError> {
        let inner = self.inner()?;
        let api = &inner.api;
        let cursor = cursor.as_deref();
        inner
            .session
            .authed(|creds| async move { api.list_games(&creds.bearer, cursor).await })
            .await
    }

    async fn match_detail(&self, match_id: &str) -> Result<ClientMatchReplay, NetError> {
        let inner = self.inner()?;
        let api = &inner.api;
        inner
            .session
            .authed(|creds| async move { api.match_detail(&creds.bearer, match_id).await })
            .await
    }

    async fn stats(&self) -> Result<ProfileStats, NetError> {
        let inner = self.inner()?;
        let api = &inner.api;
        inner
            .session
            .authed(|creds| async move { api.stats(&creds.bearer).await })
            .await
    }

    async fn get_settings(&self) -> Result<ProfileSettings, NetError> {
        let inner = self.inner()?;
        let api = &inner.api;
        inner
            .session
            .authed(|creds| async move { api.get_settings(&creds.bearer).await })
            .await
    }

    async fn patch_settings(&self, patch: SettingsPatch) -> Result<ProfileSettings, NetError> {
        let inner = self.inner()?;
        let api = &inner.api;
        let patch = &patch;
        inner
            .session
            .authed(|creds| async move { api.patch_settings(&creds.bearer, patch).await })
            .await
    }

    async fn recent_tables(&self) -> Result<RecentTables, NetError> {
        let inner = self.inner()?;
        let ids = inner.remembered().await;
        if ids.is_empty() {
            return Ok(RecentTables::default());
        }
        let ids = &ids;
        let creds = inner.session.ensure().await?;
        // The handler answers `200 {"tables": []}` (never `401`) when the bearer does not
        // verify, and one row per asked id otherwise: an empty list means "not verified".
        // Recover once (a `401` from a proxy in front counts the same) and ask again.
        match inner.recent_rows(&creds, ids).await {
            Ok(rows) if !rows.tables.is_empty() => return Ok(rows),
            Ok(_) | Err(NetError::Unauthorized) => {
                tracing::warn!("recent tables not verified; recovering the device session");
            }
            Err(err) => return Err(err),
        }
        let creds = inner.session.recover(&creds.bearer).await?;
        let rows = inner.recent_rows(&creds, ids).await?;
        if rows.tables.is_empty() {
            return Ok(RecentTables::default());
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_config_normalizes_and_refuses() -> anyhow::Result<()> {
        let data = Path::new("/data");
        let ok = |url: &str| RemoteConfig::remote(url, data);
        let c = ok("https://Euchre.example:443/")?;
        assert_eq!(c.base_url, "https://euchre.example");
        assert_eq!(c.server_key, "https://euchre.example");
        assert_eq!(
            c.state_dir,
            data.join("remote").join("https_euchre.example")
        );
        // Plain http only to this machine, unless the escape is set (review fix LDS-9).
        let insecure = NetError::Invalid("insecure_server_url".into());
        for far in [
            "http://euchre.example",
            "http://10.0.0.7:8080",
            "http://192.168.1.5",
            "http://[fe80::1]:8080",
            "http://notlocalhost",
        ] {
            assert_eq!(ok(far), Err(insecure.clone()), "{far:?}");
            assert!(
                RemoteConfig::remote_with(far, data, true).is_ok(),
                "{far:?}"
            );
        }
        assert_eq!(ok("http://127.8.9.10:81")?.base_url, "http://127.8.9.10:81");
        assert_eq!(
            ok("http://game.localhost:81")?.base_url,
            "http://game.localhost:81"
        );
        let c = ok("  http://127.0.0.1:8080  ")?;
        assert_eq!(c.base_url, "http://127.0.0.1:8080");
        assert_eq!(c.state_dir, data.join("remote").join("http_127.0.0.1_8080"));
        assert_eq!(ok("http://LOCALHOST:80")?.base_url, "http://localhost");
        assert_eq!(
            ok("https://euchre.example:8443")?.base_url,
            "https://euchre.example:8443"
        );
        assert_eq!(
            ok("http://[::1]:8080/")?.state_dir,
            data.join("remote").join("http____1__8080")
        );
        let refused = NetError::Invalid("server_url".into());
        for bad in [
            "",
            "euchre.example",
            "ftp://euchre.example",
            "ws://euchre.example",
            "file:///tmp/x",
            "https://user@euchre.example",
            "https://user:pw@euchre.example",
            "https://euchre.example/api",
            "https://euchre.example/?x=1",
            "https://euchre.example/?",
            "https://euchre.example/#top",
            "http://",
        ] {
            assert_eq!(ok(bad), Err(refused.clone()), "{bad:?}");
        }
        // The embedded config keeps its own key and the data dir itself.
        let e = RemoteConfig::embedded("http://127.0.0.1:4100".into(), data);
        assert_eq!(e.server_key, EMBEDDED_SERVER_KEY);
        assert_eq!(e.state_dir, data);
        Ok(())
    }

    #[test]
    fn a_gateway_on_another_origin_is_not_dialed() {
        assert!(same_origin(
            "http://127.0.0.1:4100/rivet",
            "http://127.0.0.1:4100"
        ));
        assert!(same_origin(
            "https://euchre.example:443/rivet",
            "https://euchre.example"
        ));
        for other in [
            "https://evil.example/rivet",
            "http://euchre.example/rivet",
            "https://euchre.example:8443/rivet",
            "not a url",
        ] {
            assert!(!same_origin(other, "https://euchre.example"), "{other}");
        }
    }

    #[test]
    fn unconfigured_is_not_wired_and_new_does_no_io() -> anyhow::Result<()> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let none = RemoteBackend::default();
        assert_eq!(none.config(), None);
        assert_eq!(none.name(), "remote");
        assert_eq!(rt.block_on(none.identity()), Err(NetError::NotWired));
        assert_eq!(rt.block_on(none.recent_tables()), Err(NetError::NotWired));
        none.interrupt_tables();
        none.end_tables(NetError::ServerStopped);
        // `new` touches no disk: the state dir does not appear.
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("nowhere");
        let backend =
            RemoteBackend::new(RemoteConfig::embedded("http://127.0.0.1:9".into(), &state))?;
        assert!(!state.exists());
        assert!(format!("{backend:?}").contains("embedded"));
        // No remembered tables: no call at all (port 9 would refuse it).
        assert_eq!(
            rt.block_on(backend.recent_tables()),
            Ok(RecentTables::default())
        );
        assert!(matches!(
            rt.block_on(backend.open_table("nope")),
            Err(NetError::NotFound)
        ));
        Ok(())
    }
}
