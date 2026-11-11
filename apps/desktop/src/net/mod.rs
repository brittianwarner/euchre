//! Everything the desktop says to a table server, behind one [`Backend`] trait (D28).
//!
//! - [`Backend`] covers identity, creating and opening tables, the match history and
//!   the profile settings.
//! - [`Backend::open_table`] returns a [`TableConn`]: a stream of protocol
//!   [`TableEvent`]s plus the four table actions ([`TableCalls`]).
//! - [`sim::SimBackend`] is the in-process dev double (engine + a pluggable decider).
//!   [`remote::RemoteBackend`] is the HTTP + rivetkit-client backend (lane N1).
//!   [`embedded::EmbeddedBackend`] runs `euchre_server` in-process on loopback and talks to it
//!   through a `RemoteBackend` (lane N2; `docs/port/reports/desktop-net-contract.md`). It is the
//!   default backend ([`BackendKind::choose`]).
//! - [`bridge`] runs every call on the tokio thread and reports back through
//!   `AppHandle::update`, so the UI thread never blocks. A backend with a local server
//!   ([`Backend::host`]) has it booted, watched and stopped by the bridge.
//! - [`rt`] is the one async-runtime seam: tokio in the window, the page's event loop in the
//!   browser build (wasm32), where only the sim and the web backend exist.
//!
//! The profile calls speak the `euchre_profile_model` shapes the server returns:
//! [`ProfileSettings`] and [`SettingsPatch`] for settings, [`ProfileStats`] for the lifetime
//! stats and [`ClientMatchReplay`] for one match.

pub mod bridge;
#[cfg(not(target_arch = "wasm32"))]
pub mod embedded;
#[cfg(not(target_arch = "wasm32"))]
pub mod fetch;
#[cfg(not(target_arch = "wasm32"))]
pub mod local;
#[cfg(not(target_arch = "wasm32"))]
pub mod remote;
// The rivet error mapping both server backends share, and the browser's server backend.
pub mod rivet_error;
pub mod rt;
pub mod sim;
#[cfg(target_arch = "wasm32")]
pub mod web;

use std::sync::{Arc, OnceLock};

// `std::time::Instant::now()` panics on wasm32; Rune's is `performance.now()` there and
// `std::time::Instant` on native.
use rune::time::Instant;

use async_trait::async_trait;
use euchre_profile_model::{ClientMatchReplay, ProfileSettings, ProfileStats, SettingsPatch};
use euchre_protocol::{
    CompletedHandReview, EngineConfigOverrides, GameStatus, MatchListPage, MoveAck, PublicGameView,
    ReviewTrickAck, ReviewTrickRequest, SubmitMoveRequest, TableEvent,
};
use tokio::sync::{mpsc, watch};

/// The match length every table plays to (06 §4: `gameTo: 10`).
pub const GAME_TO: u8 = 10;

/// Environment variable choosing the backend: `sim`, `embedded` or `remote` (any case). Unset,
/// the app runs `remote` when [`SERVER_URL_ENV`] names a server and `embedded` (the game server
/// in-process on loopback) otherwise; scene and headless runs are always `sim`.
pub const BACKEND_ENV: &str = "EUCHRE_DESKTOP_BACKEND";

/// Environment variable naming a remote server (`EUCHRE_SERVER_URL`, ARCHITECTURE §2.5).
pub const SERVER_URL_ENV: &str = "EUCHRE_SERVER_URL";

/// The server a release download plays against when the environment names none (owner decision
/// 4: `https://euchre.sh`). The release script sets `EUCHRE_DEFAULT_SERVER_URL` at build time;
/// a dev build has none and keeps the embedded server as its default. `EUCHRE_SERVER_URL` and
/// `EUCHRE_DESKTOP_BACKEND` still win at run time.
pub const DEFAULT_SERVER_URL: Option<&str> = option_env!("EUCHRE_DEFAULT_SERVER_URL");

/// The server URL for this run: `EUCHRE_SERVER_URL` when set and non-empty, else the build's
/// [`DEFAULT_SERVER_URL`], else none.
pub fn server_url() -> Option<String> {
    std::env::var(SERVER_URL_ENV)
        .ok()
        .filter(|u| !u.trim().is_empty())
        .or_else(|| {
            DEFAULT_SERVER_URL
                .filter(|u| !u.trim().is_empty())
                .map(str::to_owned)
        })
}

/// Environment variable moving the app's data dir (device record, recent tables, the embedded
/// server, logs) away from the platform one. Tests and CLI runs always set it.
pub const DATA_DIR_ENV: &str = "EUCHRE_DATA_DIR";

/// Monotonic milliseconds since the first call in this process: the one clock the
/// table store, the bridge timers and the sim share.
pub fn now_ms() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = EPOCH.get_or_init(Instant::now);
    u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Wall-clock milliseconds since the Unix epoch, for dates people read (a match's
/// `playedAt`). Never for timers: those use [`now_ms`].
pub fn wall_ms() -> u64 {
    // `SystemTime::now()` panics on wasm32; the page's `Date.now()` is the same clock.
    #[cfg(target_arch = "wasm32")]
    {
        // An integral f64 well inside u64 (`as` saturates anything else).
        js_sys::Date::now() as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }
}

/// Why a backend call failed. The table store treats every failure as "no reply"
/// (02 §6), so these only shape logs, toasts and the table's failure line. `Display` is the
/// player's copy (06 voice): never put a server, HTTP or rivet error text in here.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    /// No server is configured for this backend (an unconfigured `RemoteBackend`, or a lane
    /// that has not landed).
    #[error("This copy of Euchre has no game server set up.")]
    NotWired,
    /// No such table or match (`400` bad id, `404`).
    #[error("That table could not be found.")]
    NotFound,
    /// The call did not answer in time.
    #[error("The table did not answer in time.")]
    Timeout,
    /// No live connection to the table (it closed, or it is reconnecting).
    #[error("The connection to the table closed.")]
    Closed,
    /// The device session was refused even after a refresh and a new enrollment (`401`).
    #[error("Please start a game on this device first.")]
    Unauthorized,
    /// The table belongs to another device (`403` on the table session).
    #[error("This table belongs to another device. Open Your games to find your table.")]
    Forbidden,
    /// The table answered `403` after this device was signed out and enrolled again (its
    /// refresh was refused, or its session ran out): the table belongs to the device's old
    /// identity, not to another device (review fix PP-7).
    #[error("This device was signed out, so older tables can’t be opened.")]
    SignedOut,
    /// The server or the table cannot answer just now (`5xx`, `421`, an unreadable body).
    #[error("Your table is temporarily unavailable. Please reconnect.")]
    Unavailable,
    /// A remote server could not be reached at all (refused connection, DNS, TLS). The
    /// embedded backend reports [`NetError::ServerStopped`] instead.
    #[error("Can’t reach the game server. Check your connection, then try again.")]
    Offline,
    /// The server is rate limiting this device (`429`).
    #[error("Too many requests. Try again in a minute.")]
    RateLimited,
    /// The server refused the request itself (`400`, `413`, or an action's `user/<code>`
    /// error). Carries the machine code for logs; the copy stays generic.
    #[error("That was not accepted. Please try again.")]
    Invalid(String),
    /// The embedded game server is not running (it stopped, or it could not start).
    #[error("The game server stopped. Restart to keep playing.")]
    ServerStopped,
    /// Anything else. The string is for logs and `Debug` only; the player reads a fixed line.
    #[error("Something went wrong. Please try again.")]
    Other(String),
}

/// Who is playing on this device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The stable user id the server knows this device by.
    pub user_id: String,
    /// The display name, when the player set one.
    pub display_name: Option<String>,
}

/// The options chosen on the New game screen (06 §4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableConfig {
    /// `cfg.stickTheDealer`.
    pub stick_the_dealer: bool,
    /// `cfg.requireNaturalTrump`.
    pub require_natural_trump: bool,
}

impl TableConfig {
    /// The engine overrides sent with `createTable`: `{ gameTo: 10, stickTheDealer,
    /// requireNaturalTrump }`.
    pub fn overrides(self) -> EngineConfigOverrides {
        EngineConfigOverrides {
            game_to: Some(GAME_TO),
            stick_the_dealer: Some(self.stick_the_dealer),
            require_natural_trump: Some(self.require_natural_trump),
            ..EngineConfigOverrides::default()
        }
    }
}

/// One item on a table connection's stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnEvent {
    /// The transport came up (`true`) or went down (`false`). The backend reconnects by
    /// itself after a `false`; the store resyncs on the next `true` (02 §6).
    Status(bool),
    /// A protocol event (sync, thinking, chat, chatDelta, presence).
    Event(TableEvent),
    /// The connection ended for good and will not come back by itself (the table is
    /// another device's, the device lost its session, or reconnecting gave up). It is the
    /// last item; the stream ends after it.
    Ended(NetError),
}

/// One table this device opened before, as `GET /api/tables/recent` reads it. Serde (camel
/// case, as the wire) lets a live fixture carry it (`scenes::LiveFixture`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentTable {
    /// The table's game id.
    pub game_id: String,
    /// Its status, or `None` when its snapshot was unavailable.
    pub status: Option<GameStatus>,
    /// The score by team, or `None` when unavailable.
    pub score: Option<[u8; 2]>,
    /// The current hand number (0-based), or `None` when unavailable.
    pub hand_no: Option<u32>,
}

/// The saved-tables band (06 §17.1): the tables this device opened, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RecentTables {
    /// Up to 12 tables, newest first.
    pub tables: Vec<RecentTable>,
    /// Some table could not be read just now.
    pub unavailable: bool,
}

/// Where the embedded game server is (lane N2). The bridge forwards every change as
/// `Reply::Server`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerPhase {
    /// Booting: the engine and the server are starting ("A seat at the table.").
    Starting,
    /// The first launch on this machine: the engine is downloading before the boot
    /// (contract §N2.6). Reads as [`ServerPhase::Starting`] with the first-start line.
    Downloading,
    /// The download is slow (past 45 s, or it had to resume); it keeps going.
    DownloadingSlow,
    /// Serving; every backend call goes through.
    Ready,
    /// It stopped by itself (`RunningServer::closed`): "The game server stopped. Restart to
    /// keep playing."
    Stopped,
    /// It could not start; the reason, for the player.
    Failed(BootProblem),
}

/// Why the embedded server could not start, as the player reads it (N2 maps every
/// `euchre_server::BootError`; the copy is in the contract).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootProblem {
    /// Another copy of Euchre holds this data dir (its lock, or the engine's recorded owner
    /// is alive) and did not let go within 20 s.
    AlreadyOpen,
    /// `EnginePortInUse` with no other copy of Euchre behind it: another program holds a
    /// port the engine needs. Never move ports (the engine's storage is tied to them).
    PortInUse {
        /// The engine port.
        port: u16,
    },
    /// `LowDisk`: the engine needs [`euchre_server::MIN_FREE_DISK_BYTES`] free.
    LowDisk {
        /// Bytes free where the server keeps its data.
        free_bytes: u64,
        /// Bytes the engine needs.
        need_bytes: u64,
    },
    /// `EngineBinaryMissing`: no engine binary was found and none could be downloaded (the
    /// release server was never reached).
    EngineMissing,
    /// The first-start download began, then nothing arrived for three minutes. The next try
    /// resumes it.
    DownloadStalled,
    /// `NotReady`: the engine or the actor runner did not come up in time.
    TooSlow,
    /// `KeysConflict` / `TableConfigConflict`, or `secrets.json` could not be read: the
    /// saved server data does not fit this run.
    DataConflict,
    /// `HttpPortInUse` or `AlreadyRunning` on a restart: this process cannot start its
    /// server again, a fresh one can ("Reopen Euchre", `Tx::relaunch`).
    Relaunch,
    /// Anything else (`EnginePortChanged`, other errors).
    Other,
}

/// A local server the bridge owns (embedded mode). Sim and remote backends have none.
#[async_trait]
pub trait ServerHost: Send + Sync {
    /// The phase now and every change after it (newest wins).
    fn phases(&self) -> watch::Receiver<ServerPhase>;
    /// Starts the server: `Starting`, then `Ready` or `Failed`. The bridge calls it once when
    /// it starts; a second call while booting or ready does nothing.
    async fn boot(&self);
    /// Stops whatever runs (bounded), then boots again: the Restart and Try again actions.
    async fn restart(&self);
    /// Stops the server within its budget. The engine never outlives the app.
    async fn shutdown(&self);
}

/// The four table actions (02 §5). Every method is bounded by the caller.
#[async_trait]
pub trait TableCalls: Send + Sync {
    /// `submitMove`.
    async fn submit_move(&self, request: SubmitMoveRequest) -> Result<MoveAck, NetError>;
    /// `reviewTrick`.
    async fn review_trick(&self, request: ReviewTrickRequest) -> Result<ReviewTrickAck, NetError>;
    /// `snapshot`: the current view, or `None` when the table has none yet.
    async fn snapshot(&self) -> Result<Option<PublicGameView>, NetError>;
    /// `getHandReview(handNo)`: a finished hand's archive, if kept.
    async fn get_hand_review(&self, hand_no: u32) -> Result<Option<CompletedHandReview>, NetError>;
    /// Closes the connection; the event stream ends.
    async fn close(&self);
}

/// An open table: its id, the event stream and the actions.
pub struct TableConn {
    /// The table's game id.
    pub game_id: String,
    /// Status changes and protocol events, in order.
    pub events: mpsc::UnboundedReceiver<ConnEvent>,
    /// The table actions.
    pub calls: Arc<dyn TableCalls>,
}

impl std::fmt::Debug for TableConn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableConn")
            .field("game_id", &self.game_id)
            .finish_non_exhaustive()
    }
}

/// The desktop's one door to a table server (D28).
#[async_trait]
pub trait Backend: Send + Sync {
    /// A short name for logs (`"sim"`, `"remote"`).
    fn name(&self) -> &'static str;
    /// Enrolls the device on first run, then returns who is playing.
    async fn identity(&self) -> Result<Identity, NetError>;
    /// Creates a table with these rules; returns its game id.
    async fn create_table(&self, cfg: TableConfig) -> Result<String, NetError>;
    /// Connects to a table.
    async fn open_table(&self, game_id: &str) -> Result<TableConn, NetError>;
    /// One page of finished and saved matches, newest first.
    async fn list_games(&self, cursor: Option<String>) -> Result<MatchListPage, NetError>;
    /// One match's replay: totals and the list of hands (`getReplay`).
    async fn match_detail(&self, match_id: &str) -> Result<ClientMatchReplay, NetError>;
    /// The lifetime stats over every recorded match (`getStats`).
    async fn stats(&self) -> Result<ProfileStats, NetError>;
    /// The player's profile settings: the three personas, the house prompt, table prefs.
    async fn get_settings(&self) -> Result<ProfileSettings, NetError>;
    /// Merges a patch into the profile settings (`updateSettings`); returns what was saved.
    async fn patch_settings(&self, patch: SettingsPatch) -> Result<ProfileSettings, NetError>;
    /// The tables this device opened before, newest first (06 §17.1). Backends without a
    /// server (the sim) remember none.
    async fn recent_tables(&self) -> Result<RecentTables, NetError> {
        Ok(RecentTables::default())
    }
    /// The local server this backend runs, if any: the bridge boots it, reports its phase,
    /// restarts it on request and stops it at quit.
    fn host(&self) -> Option<Arc<dyn ServerHost>> {
        None
    }
}

/// Which backend a run uses ([`BackendKind::choose`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    /// The in-process [`sim::SimBackend`]: scenes, headless renders and `sim` by choice.
    Sim,
    /// The game server in-process on loopback: [`embedded::EmbeddedBackend`] (the default).
    Embedded,
    /// The HTTP + rivetkit [`remote::RemoteBackend`] against `EUCHRE_SERVER_URL`.
    Remote,
}

impl BackendKind {
    /// Parses an explicit backend name (`sim`, `embedded`, `remote`, any case, trimmed);
    /// anything else is `None`.
    pub fn parse(value: &str) -> Option<BackendKind> {
        let value = value.trim();
        [
            ("sim", BackendKind::Sim),
            ("embedded", BackendKind::Embedded),
            ("remote", BackendKind::Remote),
        ]
        .into_iter()
        .find_map(|(name, kind)| value.eq_ignore_ascii_case(name).then_some(kind))
    }

    /// The backend for a window run, from [`BACKEND_ENV`] and [`SERVER_URL_ENV`] (pure; contract
    /// §N2.1): an explicit `sim`, `embedded` or `remote` wins; another value is ignored with a
    /// warning; else a non-empty server URL is `Remote`; else `Embedded`. Scene and headless runs
    /// never get here (`lib.rs` runs them on the sim).
    pub fn choose(backend: Option<&str>, server_url: Option<&str>) -> BackendKind {
        if let Some(value) = backend.filter(|v| !v.trim().is_empty()) {
            match BackendKind::parse(value) {
                Some(kind) => return kind,
                None => tracing::warn!(
                    value,
                    "{BACKEND_ENV} is not sim, embedded or remote; ignoring it"
                ),
            }
        }
        if server_url.is_some_and(|url| !url.trim().is_empty()) {
            BackendKind::Remote
        } else {
            BackendKind::Embedded
        }
    }

    /// The backend for this process: [`BackendKind::Sim`] for a scene or headless run
    /// (`scene_run`), else [`BackendKind::choose`] over the environment.
    pub fn for_run(scene_run: bool) -> BackendKind {
        if scene_run {
            BackendKind::Sim
        } else {
            BackendKind::from_env()
        }
    }

    /// Reads [`BACKEND_ENV`] and [`SERVER_URL_ENV`] into [`BackendKind::choose`].
    pub fn from_env() -> BackendKind {
        BackendKind::choose(
            std::env::var(BACKEND_ENV).ok().as_deref(),
            server_url().as_deref(),
        )
    }

    /// Builds the backend. In the browser build there is no embedded server and no native
    /// remote backend: `Remote` and `Embedded` are the [`web::WebBackend`] against the page's
    /// own origin. Only a `web-proof` build with `?backend=sim` in the URL plays the sim.
    pub fn build(self) -> Arc<dyn Backend> {
        match self {
            // The app's computer seats play the euchre-ai heuristic; tests keep the sim's own.
            BackendKind::Sim => Arc::new(sim::SimBackend::new(sim::SimOptions {
                decider: Arc::new(sim::AiDecider::neutral()),
                ..sim::SimOptions::default()
            })),
            #[cfg(not(target_arch = "wasm32"))]
            BackendKind::Embedded => Arc::new(embedded::EmbeddedBackend::new(
                embedded::EmbeddedConfig::new(data_dir()),
            )),
            #[cfg(not(target_arch = "wasm32"))]
            BackendKind::Remote => Arc::new(remote_from_env()),
            #[cfg(target_arch = "wasm32")]
            BackendKind::Embedded | BackendKind::Remote => {
                #[cfg(feature = "web-proof")]
                if web_wants_sim() {
                    return BackendKind::Sim.build();
                }
                Arc::new(web::WebBackend::from_page())
            }
        }
    }
}

/// True when the page URL carries `backend=sim` (the `web-proof` build only).
#[cfg(all(target_arch = "wasm32", feature = "web-proof"))]
fn web_wants_sim() -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|q| {
            q.trim_start_matches('?')
                .split('&')
                .any(|kv| kv == "backend=sim")
        })
}

/// The app's data dir: [`DATA_DIR_ENV`] when set and non-empty, else the platform one
/// (`ProjectDirs("sh", "euchre", "Euchre").data_dir()`; macOS
/// `~/Library/Application Support/sh.euchre.Euchre`), else a relative `euchre-data`. It holds
/// `prefs.json`, `device.json`, `recent-tables.json`, `logs/` and the embedded server's
/// `server/` (contract §N2.1); preferences follow [`DATA_DIR_ENV`] when `EUCHRE_PREFS_DIR` is
/// unset.
pub fn data_dir() -> std::path::PathBuf {
    data_dir_from(std::env::var_os(DATA_DIR_ENV))
}

/// [`data_dir`] over the value of [`DATA_DIR_ENV`].
fn data_dir_from(value: Option<std::ffi::OsString>) -> std::path::PathBuf {
    if let Some(dir) = value.filter(|d| !d.is_empty()) {
        return std::path::PathBuf::from(dir);
    }
    // The browser has no filesystem; nothing reads this path there.
    #[cfg(target_arch = "wasm32")]
    {
        std::path::PathBuf::from("euchre-data")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        directories::ProjectDirs::from("sh", "euchre", "Euchre").map_or_else(
            || std::path::PathBuf::from("euchre-data"),
            |dirs| dirs.data_dir().to_path_buf(),
        )
    }
}

/// The remote backend `EUCHRE_SERVER_URL` names, or an unconfigured one (every call
/// [`NetError::NotWired`]) when it is unset or unusable.
#[cfg(not(target_arch = "wasm32"))]
fn remote_from_env() -> remote::RemoteBackend {
    let Some(url) = server_url() else {
        return remote::RemoteBackend::unconfigured();
    };
    match remote::RemoteConfig::remote(&url, &data_dir()).and_then(remote::RemoteBackend::new) {
        Ok(backend) => backend,
        Err(err) => {
            tracing::warn!(error = ?err, "remote server not usable");
            remote::RemoteBackend::unconfigured()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_rules() {
        use BackendKind::{Embedded, Remote, Sim};
        let url = Some("https://euchre.example");
        // An explicit choice wins, in any case and with spaces around it.
        for (backend, kind) in [("sim", Sim), (" Remote ", Remote), ("EMBEDDED", Embedded)] {
            assert_eq!(BackendKind::choose(Some(backend), None), kind, "{backend}");
            assert_eq!(
                BackendKind::choose(Some(backend), url),
                kind,
                "{backend} + url"
            );
        }
        // Another value (or an empty one) is ignored: the URL decides, else embedded.
        assert_eq!(BackendKind::choose(Some("other"), None), Embedded);
        assert_eq!(BackendKind::choose(Some("other"), url), Remote);
        assert_eq!(BackendKind::choose(Some(""), url), Remote);
        // No choice: a non-empty URL is remote, else the embedded server is the default.
        assert_eq!(BackendKind::choose(None, url), Remote);
        assert_eq!(BackendKind::choose(None, Some("  ")), Embedded);
        assert_eq!(BackendKind::choose(None, None), Embedded);
        // A scene or headless run is always the sim, whatever the environment says.
        assert_eq!(BackendKind::for_run(true), Sim);
        assert_eq!(BackendKind::parse("sim"), Some(Sim));
        assert_eq!(BackendKind::parse("nope"), None);
    }

    #[test]
    fn data_dir_prefers_the_override() {
        let dir = std::path::PathBuf::from("/private/tmp/euchre-test-data");
        assert_eq!(data_dir_from(Some(dir.clone().into_os_string())), dir);
        // An empty override is no override: the platform dir (or the relative fallback).
        let platform = data_dir_from(None);
        assert_eq!(data_dir_from(Some(std::ffi::OsString::new())), platform);
        assert_ne!(platform, dir);
    }

    #[test]
    fn player_copy_never_carries_internal_text() {
        assert_eq!(
            NetError::Other("rivet: actor/not_found at 127.0.0.1:6420".to_owned()).to_string(),
            "Something went wrong. Please try again."
        );
        assert_eq!(
            NetError::NotWired.to_string(),
            "This copy of Euchre has no game server set up."
        );
    }

    #[test]
    fn config_overrides_play_to_ten() {
        let o = TableConfig {
            stick_the_dealer: true,
            require_natural_trump: false,
        }
        .overrides();
        assert_eq!(o.game_to, Some(10));
        assert_eq!(o.stick_the_dealer, Some(true));
        assert_eq!(o.require_natural_trump, Some(false));
    }

    #[test]
    fn clock_is_monotonic() {
        let a = now_ms();
        let b = now_ms();
        assert!(b >= a);
    }
}
