# Desktop net wave: build contract (lanes N1 and N2)

**Back to:** ARCHITECTURE §2.5, 02 §0–§6, 05 §11, 06 §14 and §17.1, the server reports
`server-http.md` and `server-boot.md`, and the requests in `docs/port/requests/desktop-net-*.md`.

**Goal.** The desktop plays for real against the Rust server. By default it starts the server in
this process (`euchre_server::start`) with its own engine and talks to it on loopback. With
`EUCHRE_SERVER_URL` it talks to a remote server instead. The in-process sim stays for tests,
scenes and `EUCHRE_DESKTOP_BACKEND=sim`.

**The bar.** Local play feels instant. Home is interactive on the first frame and never waits for
the server. Nothing janks. Every state is honest: booting, stopped, failed, reconnecting. A
reconnect loses nothing. No server, HTTP or rivet text ever reaches the player.

**Where the tree is now.** The skeleton is in place. It compiles, the gates are green (159 tests),
and every new call answers `NetError::NotWired`. The default backend is still `Sim` until N2 flips
it. Lane N1 fills `net/remote/**`. Lane N2 fills `net/embedded.rs` and the app. Then one
integration agent writes the live proof (§3).

---

## 0. Rules for every lane

| Rule | Detail |
|---|---|
| Files | Each lane edits only the files it owns (§1.3) and its report `docs/port/reports/desktop-net-<lane>.md`. Nothing outside `apps/desktop/**` or `docs/port/{reports,requests}/desktop-net*`. W4 is editing `apps/web`, `crates/**`, `apps/server/**`, `Dockerfile`, `.railway`, `package.json` and `turbo.json`. |
| Server gaps | Write `docs/port/requests/desktop-net-<topic>.md` (what, why, the exact change). Work around it in the desktop. |
| Git | No git writes of any kind: no add, commit, stash, checkout, restore or reset. |
| JS | Bun only, if JS is touched at all (it should not be). Never npm, npx or pnpm. |
| Cargo | `CARGO_TARGET_DIR=/Users/brittianwarner/goods/euchre-rune/.cargo-targets/net-<lane>`. Seed it with `cp -c -R .cargo-targets/net-contract .cargo-targets/net-<lane>`. Never `cargo clean`. Toolchain 1.99.1. No new crates. Every crate the wave needs is already in `apps/desktop/Cargo.toml`: `reqwest`, `url`, `futures`, `tokio-util`, `rivetkit-client`, `rivetkit-engine-process`, `euchre-server`, `euchre-server-core`, dev `wiremock` and `tempfile`. If a lane needs another, it says so in its report. |
| Player data | Never read or write `~/Library/Application Support/sh.euchre.Euchre`. Every run, the window, the binary and the tests alike, sets `EUCHRE_DATA_DIR` and `EUCHRE_PREFS_DIR` to a fresh dir under `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/net/<lane>/`. Live tests also set `EUCHRE_TEST_TMP` to that root (§4). |
| Engines | Only through `euchre_server::start(ServerConfig::try_local(..))`: through `EmbeddedBackend`, or directly in N1's live test. Each lane uses its own port block (§4). The engine binds P, P+1 and P+5. Engine tests run only when `euchre_server_core::testing::e2e_enabled()` (`EUCHRE_E2E=1`). The engine refuses to start with less than 4 GiB free disk. |
| One server per test binary | Server state is process-global: the process lease (`AlreadyRunning`), the table config (`TableConfigConflict`), the shared keys (`KeysConflict`) and the sticky HTTP port. So each live test binary has exactly one engine-backed `#[test]` and one data dir, and runs its steps in sequence inside it. |
| Proxy | `NO_PROXY=127.0.0.1,localhost,::1` and `no_proxy=` the same, for every command that talks to loopback. |
| Timeouts | Use the Bash tool `timeout` on every engine-touching command (600000 ms for live tests). Never `perl alarm`. |
| Processes | Signal only a pid you started, by exact pid. Never `pkill` or `killall`. The user is playing in `euchre-motion` (pid 748). Rivet engines that are not yours (pid 67721, W4's on 6751–6760 and 6930–6999) are off limits. Before you finish, nothing of yours is listening on your ports and no server, engine or window of yours is running. |
| UI thread | Never block it. All IO runs on the bridge runtime. Results come back through `UiSink` → `AppHandle::update`. |
| Copy | `NetError`'s `Display` is player copy in the 06 voice: plain, calm, short. Never put server, HTTP, rivet or serde text in a `NetError`, a toast or a label. Log that detail at `warn`, without tokens or bodies. |
| Gates | `cargo fmt -p euchre-desktop --check`; `cargo clippy -p euchre-desktop --all-targets -- -D warnings`; `cargo test -p euchre-desktop` (engine-free without `EUCHRE_E2E`, every existing test still green, motion frame budgets included); plus your live test with `EUCHRE_E2E=1`. |

---

## 1. The seams

### 1.1 Shared types (`src/net/mod.rs`, already in the tree)

N2 owns the file. A change to anything below needs a note in both lane reports.

| Item | Meaning |
|---|---|
| `NetError` | `NotWired`, `NotFound`, `Timeout`, `Closed`, `Unauthorized`, `Forbidden`, `Unavailable`, `Offline` (new: a remote server cannot be reached at all), `RateLimited`, `Invalid(code)`, `ServerStopped`, `Other(detail)`. The table store treats every failure as "no reply" (02 §6). |
| `ConnEvent` | `Status(bool)`, `Event(TableEvent)`, `Ended(NetError)`. `Ended` is the last item; the stream ends after it. |
| `RecentTable`, `RecentTables` | One `GET /api/tables/recent` row (`game_id`, `status: Option<GameStatus>`, `score: Option<[u8; 2]>`, `hand_no: Option<u32>`), and the list plus `unavailable`. |
| `ServerPhase` | `Starting`, `Downloading` (new: the first launch fetches the engine), `Ready`, `Stopped`, `Failed(BootProblem)`. |
| `BootProblem` | `PortInUse{port}`, `LowDisk{free_bytes, need_bytes}`, `EngineMissing`, `TooSlow`, `DataConflict`, `Relaunch` (new: this process cannot start its server again, a fresh process can), `Other`. |
| `trait ServerHost` | `phases() -> watch::Receiver<ServerPhase>`, `async boot()`, `async restart()`, `async shutdown()`. |
| `Backend` additions | `async recent_tables() -> Result<RecentTables, NetError>` (default: empty) and `host() -> Option<Arc<dyn ServerHost>>` (default: `None`). The bridge boots, forwards and stops the host. `BridgeConfig` is unchanged. |
| `BackendKind` | `Sim`, `Embedded`, `Remote`. |
| Consts | `BACKEND_ENV = "EUCHRE_DESKTOP_BACKEND"`, `SERVER_URL_ENV = "EUCHRE_SERVER_URL"`, `DATA_DIR_ENV = "EUCHRE_DATA_DIR"`; `fn data_dir() -> PathBuf`. |
| Bridge | `Intent::RecentTables`, `Intent::RestartServer`, `Reply::RecentTables(Result<RecentTables, NetError>)`, `Reply::Server(ServerPhase)`, `Reply::Quit` (a signal asked to quit). The bridge forwards every phase. At `Shutdown` it closes the tables, drains preferences, then runs `host.shutdown().await`. |
| `AppState` | `local_server: Option<ServerPhase>` (`None` for sim and remote), `recent_tables: Option<Result<RecentTables, String>>`, `restart_server()`. `on_conn_event(Ended)` marks the table `Failed(copy)` and disconnected. |

### 1.2 Constructors and calls (exact; a lane may add private items, never change these)

**N1 provides** (`net/remote.rs`, `net/remote/*.rs`):

```rust
pub const EMBEDDED_SERVER_KEY: &str = "embedded";
impl RemoteConfig {
    pub fn remote(base_url: &str, data_dir: &Path) -> Result<RemoteConfig, NetError>;
    pub fn embedded(url: String, data_dir: &Path) -> RemoteConfig;          // done
}
impl RemoteBackend {                      // Clone shares one inner state (Arc), Debug, Default
    pub fn new(config: RemoteConfig) -> Result<RemoteBackend, NetError>;  // no IO, no enroll
    pub fn unconfigured() -> RemoteBackend;                              // NotWired forever
    pub fn config(&self) -> Option<&RemoteConfig>;
    pub fn interrupt_tables(&self);       // tests: drop every live socket as a blip would
    pub fn end_tables(&self, err: NetError); // Ended(err) on every open table, stop them
}
impl Backend for RemoteBackend { /* name() == "remote" */ }
// remote::http
pub fn status_error(call: Call, status: Option<u16>, timed_out: bool, code: Option<&str>) -> NetError;
impl Api { pub fn new(base_url: &str) -> Result<Api, NetError>; pub fn with_create_retry(self, Duration) -> Api; /* one async fn per call, §N1.3 */ }
pub mod wire { CreateTableBody, CreateTableResponse, RecentTableRow, RecentTablesResponse,
               RivetMetadata, SessionResponse, ApiErrorBody, ApiErrorDetail }
// remote::device
impl DeviceStore { pub fn new(state_dir: &Path, server_key: &str) -> DeviceStore;
                   pub fn load(&self) -> Option<DeviceRecord>; pub fn save(&self, &DeviceRecord) -> Result<(), NetError>; }
// remote::recent
impl RecentStore { pub fn new(state_dir: &Path) -> RecentStore; pub fn load(&self) -> Vec<String>;
                   pub fn remember(&self, game_id: &str) -> Result<(), NetError>; }
// remote::table
impl RemoteTable { pub fn game_id(&self) -> &str; }   // impl TableCalls; N1 may change `new`
pub fn move_rejection(err: &anyhow::Error) -> Option<MoveAck>;
pub fn action_error(err: &anyhow::Error) -> NetError;
```

**N2 provides** (`net/embedded.rs`, the bridge and the app):

```rust
impl EmbeddedConfig {        // pub data_dir, engine_port: Option<u16>, fast_tempo, offline_ai
    pub fn new(data_dir: PathBuf) -> EmbeddedConfig;                       // the app
    pub fn for_tests(data_dir: PathBuf, engine_port: u16) -> EmbeddedConfig; // fast, offline AI
    pub fn server_dir(&self) -> PathBuf;                                   // <data_dir>/server
    pub fn remote_config(&self, url: String) -> RemoteConfig;
}
impl Supervisor { pub fn new(EmbeddedConfig) -> Supervisor; pub fn config(&self) -> &EmbeddedConfig;
                  pub fn phase(&self) -> ServerPhase; pub fn remote(&self) -> Option<Arc<RemoteBackend>>;
                  pub fn engine_pid(&self) -> Option<u32>; }        // impl ServerHost
impl EmbeddedBackend { pub fn new(EmbeddedConfig) -> EmbeddedBackend; pub fn supervisor(&self) -> &Arc<Supervisor>;
                       pub fn remote(&self) -> Option<Arc<RemoteBackend>>; } // impl Backend, name "embedded"
pub fn boot_problem(err: &anyhow::Error) -> BootProblem;
pub async fn engine_source() -> EngineSource;   // Env(p) | Bundled(p) | Cache(p) | Download
pub fn bundled_engine(exe: &Path) -> Option<PathBuf>;
pub async fn download_engine() -> Result<PathBuf, BootProblem>;
impl BackendKind { pub fn choose(backend: Option<&str>, server_url: Option<&str>) -> BackendKind; } // pure; stub = parse(backend)
impl AppState { pub fn continue_table(&mut self, game_id: String, tx: &mut Tx); }  // done: close, open id, Route::Table
// net::bridge: Reply::Quit: a signal asked the app to quit; AppState answers tx.quit() (done). N2 posts it.
```

### 1.3 File ownership

| Lane | Owns (edits only these) | Its new test files |
|---|---|---|
| **N1** | `src/net/remote.rs`, `src/net/remote/**` (`http.rs`, `device.rs`, `recent.rs`, `table.rs`, plus any new private module there) | `tests/remote_http.rs` (wiremock, always runs), `tests/remote_live.rs` (gated) |
| **N2** | `src/net/embedded.rs`, `src/net/embedded/**`, `src/net/mod.rs`, `src/net/bridge.rs`, `src/net/sim.rs`, `src/app.rs`, `src/lib.rs`, `src/main.rs`, `src/prefs.rs`, `src/scenes.rs`, `src/screens/**`, new `src/startup.rs`, the existing `tests/shell.rs` (new scene names only) | `tests/server_states.rs` (headless, always runs), `tests/embedded_live.rs` (gated) |
| **Integration** | `tests/live.rs`; after N1 and N2 land, any fix anywhere in `apps/desktop/**`, recorded in its report | `tests/live.rs` (gated) |

`apps/desktop/Cargo.toml` and `Cargo.lock` stay as they are. Each lane's in-file unit tests live in
the files it owns.

---

## N1. REMOTE: the HTTP API and rivetkit-client backend

`RemoteBackend` serves both modes. With a remote server it is built from
`RemoteConfig::remote(EUCHRE_SERVER_URL, data_dir)`. With the embedded server, N2 builds it from
`RemoteConfig::embedded(RunningServer::url(), data_dir)` once the server is ready.

### N1.1 Configuration

- `RemoteConfig::remote(base_url, data_dir)` trims the input and parses it with `url::Url`. It
  refuses (`Err(NetError::Invalid("server_url"))`) a scheme other than `http` or `https`, no host,
  a username or password, a path other than `""` or `"/"`, a query, or a fragment.
- `base_url` is `Url::origin().ascii_serialization()`: lowercase host, default port dropped, no
  trailing slash. `server_key` is `base_url`.
- `state_dir = <data_dir>/remote/<slug>`. `<slug>` is `base_url` with `://` replaced by `_` and
  every character outside `[A-Za-z0-9.-]` replaced by `_`. For example, `https://Euchre.example:443/`
  gives `https_euchre.example`, and `http://127.0.0.1:8080` gives `http_127.0.0.1_8080`.
- `RemoteConfig::embedded(url, data_dir)` keeps `state_dir = data_dir` and
  `server_key = "embedded"`. The loopback port can change between launches, so the device record
  is keyed by name, not by URL.
- `RemoteBackend::new` builds the HTTP client and nothing else: no IO, no enroll.
  `RemoteBackend::unconfigured()` (also `Default`) answers `NotWired` to every call, for good. The
  bridge's unit tests rely on that.

### N1.2 The HTTP client

- One `reqwest::Client` per backend, shared by its clones. It has: `connect_timeout(CONNECT_TIMEOUT)`
  (3 s), `timeout(REQUEST_TIMEOUT)` (10 s), `user_agent("euchre-desktop/<CARGO_PKG_VERSION>")`, no
  cookie store, `redirect::Policy::none()` (a redirect reads as `Unavailable`), and `.no_proxy()`
  when the host is `127.0.0.1`, `::1` or `localhost`.
- Headers: `Accept: application/json` on every call. `Content-Type: application/json` when there
  is a body. `Authorization: Bearer <sessionToken>` where §N1.3 says bearer. Never `Origin`,
  `Cookie` or `X-Euchre-Identity`. Bearer callers are exempt from the CSRF checks, enroll allows a
  missing `Origin`, and the desktop has no email identity in this wave.
- A 2xx body that does not decode into its type is `Unavailable`. Log the call and the serde
  error, never the body.
- An error body (`{"message","error":{"code","message"}}`, `wire::ApiErrorBody`) is read only for
  `error.code`, which goes into `status_error`. Its `message` is browser copy and is never shown.
- Logs: `warn!(call = ?Call::X, status, code, "...")`. Never a token, `device.json` content or a
  body.

### N1.3 Every call

All paths are relative to `base_url`. Types come from `euchre-server-core`, `euchre-protocol` and
`euchre-profile-model`, or from `http::wire` where the handler builds untyped JSON (request
`desktop-net-http-types.md`). Each `wire` type is pinned by a test against the exact JSON the
handler writes.

| `Call` | Request | Auth | Success | Decodes to | Used by |
|---|---|---|---|---|---|
| `Enroll` | `POST /api/device/enroll`, no body | none | 200 | `euchre_server_core::auth::DeviceEnrollment` (`userId`, `sessionToken`, `expiresAt` ms) | §N1.5 |
| `Refresh` | `POST /api/device/refresh`, no body | bearer | 200 | `DeviceEnrollment` | §N1.5 |
| `Session` | `GET /api/session` | bearer | 200 | `wire::SessionResponse` (`guestUserId: string\|null`) | `identity()` |
| `CreateTable` | `POST /api/tables`, body `wire::CreateTableBody {stick, natural}` | bearer | 201 | `wire::CreateTableResponse {gameId}` | `create_table` |
| `RecentTables` | `GET /api/tables/recent?ids=<id>,<id>…` (≤ 12, newest first) | bearer | 200 | `wire::RecentTablesResponse {tables:[{gameId,status,score,handNo}], unavailable}` | `recent_tables` |
| `TableSession` | `POST /api/table-session/{gameId}`, no body | bearer | 200 | `euchre_protocol::TableSessionResponse {actorId, token, gatewayToken, expiresAt}` | §N1.6 |
| `RivetMetadata` | `GET /api/rivet/metadata` | none | 200 | `wire::RivetMetadata {clientEndpoint, clientNamespace}`, cached per backend after the first success | §N1.6 |
| `ListGames` | `GET /api/games?profile=browser&limit=10[&cursor=<c>]` | bearer | 200 | `euchre_protocol::MatchListPage` | `list_games` |
| `MatchDetail` | `GET /api/games/{matchId}?profile=browser` (path segment encoded) | bearer | 200 | `euchre_profile_model::ClientMatchReplay` | `match_detail` |
| `Stats` | `GET /api/profile/stats?profile=browser` | bearer | 200 | `ProfileStats` | `stats` |
| `GetSettings` | `GET /api/profile/settings?profile=browser` | bearer | 200 | `ProfileSettings` | `get_settings` |
| `PatchSettings` | `PATCH /api/profile/settings?profile=browser`, body `SettingsPatch` | bearer | 200 | `ProfileSettings` | `patch_settings` |

`?profile=browser` is required. Settings, stats and personas default to the email profile, which
the desktop never has. Without it they answer `401 "Tell us who you are first."`. Games and replays
default to email-then-browser, so the query only makes them explicit.

### N1.4 Status → `NetError` (`status_error`)

Apply the first matching row:

| Status | Call | `NetError` |
|---|---|---|
| none, timed out | any | `Timeout` |
| none (refused, DNS, TLS, reset) | any | `Offline` (the embedded backend turns it into `ServerStopped`, §N2.3) |
| 400 | `TableSession`, `MatchDetail` (bad id) | `NotFound` |
| 400 | others | `Invalid(code or "bad_request")` |
| 401 | any | `Unauthorized`, but only after the one recovery in §N1.5 |
| 403 | `TableSession` | `Forbidden` |
| 403 | others (CSRF only, never seen with a bearer) | `Invalid("forbidden")` |
| 404 | any | `NotFound` |
| 413 | any | `Invalid("too_large")` |
| 429 | any | `RateLimited`, no retry |
| 421, 5xx, anything else | any | `Unavailable` (`CreateTable` retries first, §N1.8) |

`GET /api/tables/recent` never answers 401. It answers `200 {"tables":[]}` when the bearer does not
verify. So an empty `tables` list for a non-empty `ids` means "not verified": recover once (§N1.5)
and ask again. If it is still empty, report `RecentTables::default()`.

### N1.5 Device identity

- **File.** `<state_dir>/device.json` holds a `DeviceRecord` (`userId`, `sessionToken`,
  `expiresAt`, `serverUrl` = the server key). It is written with `DeviceRecord::save`, which
  writes atomically with mode 0600. `DeviceStore::save` creates `state_dir` first, mode 0700 on
  unix.
- **Load.** `DeviceStore::load()` returns `None` for any of these, and each one means "enroll": a
  missing file, an unreadable or unparsable one (log a warn), a `serverUrl` that is not this
  server key, or `is_expired(now)`.
- **One session per backend, single-flight.** A `tokio::sync::Mutex<Option<DeviceRecord>>` is held
  across the network call, so concurrent callers wait and then share the one enroll or refresh.
- **`ensure_session()`**:
  - no record → `Enroll` → `DeviceRecord::from_enrollment(e, server_key)` → save → use.
  - `needs_refresh(now)` (80 % of the lifetime gone) → `Refresh` with the current bearer →
    `apply_refresh` → save.
  - Refresh answers 401 → enroll again. This is a new guest: older tables now answer 403, and that
    is logged.
  - Refresh fails any other way → keep the current record (it is valid until `expiresAt`).
- **Recovery.** A call that answers 401 recovers once: `Refresh`; if that answers 401, `Enroll`;
  then the call runs one more time. A second 401 reports `Unauthorized`.
- **Save failures.** Log a warn and keep the record in memory. The device keeps playing this run
  and enrolls again next launch. Nothing is shown to the player.
- **Clock.** `SystemTime` in ms, as `i64`, matching the server's.
- **`identity()`.** `ensure_session()`, then `Session`.
  - `guestUserId == record.userId` → `Ok(Identity { user_id, display_name: None })`.
  - `null` or a mismatch → recover once and ask again; still not verified → `Unauthorized`.
  - This makes `Intent::Identify` at window ready a real check of the session.

### N1.6 One table connection (`remote::table`)

**Open** (`open_table(game_id)`):

1. `ensure_session()`.
2. `TableSession` → `TableSessionResponse`. A missing `gatewayToken` is `Unavailable`: the desktop
   never dials without one, and this server always issues one.
3. `RivetMetadata` (cached) → `clientEndpoint` (`<server origin>/rivet`) and `clientNamespace`.
4. `rivetkit_client::Client::new(ClientConfig::new(clientEndpoint).namespace(ns).encoding(EncodingKind::Json).disable_metadata_lookup(true).token(gatewayToken))`.
   A client's credentials are fixed for its lifetime, so each dial builds its own client.
5. `client.get_for_id(euchre_protocol::actor_names::EUCHRE_TABLE, &actorId, GetOptions { params: Some(json(TableConnectParams::player(token))) })`.
   `TableConnectParams::player` includes `protocolVersion: 1`. Never `get_or_create`: tables are
   created only by `POST /api/tables`. Never dial by key: the mint resolved
   `["table", gameId]` to `actorId`.
6. `let conn = handle.connect()`. Before waiting, add one `conn.on_event(name, ..)` for each of
   `euchre_protocol::TABLE_EVENT_NAMES` (`sync`, `thinking`, `chat`, `chatDelta`, `presence`).
   - Each callback runs `TableEvent::decode(name, args.first().cloned().unwrap_or(Value::Null))`.
     `Ok` → `ConnEvent::Event`. `Err` → warn with the event name only, and drop it.
   - Events that arrive before this connection's `Status(true)` has gone out are buffered, then
     sent right after it, in order.
7. Wait on `conn.status_receiver()` for `ConnectionStatus::Connected` within `OPEN_TIMEOUT`
   (10 s). If it does not come: `conn.disconnect()`, drop the client, `Err(Timeout)`.
8. `RecentStore::remember(game_id)` (a failure is logged), then return
   `TableConn { game_id, events, calls }` with `Status(true)` already queued as the first item.

**Status.** Only edges are sent: `Status(true)` on reaching `Connected`, `Status(false)` once when
an up connection leaves it. The store resyncs on the rising edge (02 §6, `store.attach(false)`, then
`set_connected(true)`).

**Reconnect supervisor** (one task per open table, spawned on the bridge runtime):

- **On a drop:**
  1. Send `Status(false)`.
  2. `conn.disconnect()`. This stops rivetkit's own retry, which would dial with the old
     credentials.
  3. Drop the client.
  4. Dial again with backoff: `RECONNECT_FIRST` (250 ms), doubling to `RECONNECT_CAP` (8 s), with
     ±20 % jitter.
- **Before each dial**, mint a fresh session if any of these hold:
  - the last session `is_stale(now)` (`expiresAt − SESSION_REFRESH_MARGIN_MS`, 10 s);
  - the previous dial failed;
  - an action answered `forbidden`.

  Otherwise reuse the last session.
- **On success:** `Status(true)`.
- **Mint failures that end the stream for good:** 403 → `Ended(Forbidden)`, or
  `Ended(SignedOut)` when this device was silently enrolled again under a new user (the
  `device-reset.json` marker; review fix PP-7); 400 or 404 → `Ended(NotFound)`, and the id leaves
  the recent tables (review fix PP-1); 401 after recovery → `Ended(Unauthorized)`.
- **Anything else** (`Offline`, `Unavailable`, `Timeout`, `RateLimited`): keep trying until
  `RECONNECT_GIVE_UP` (120 s after the drop), then `Ended(<last error>)`.

  An unknown table also reads as 503 today, so this is where it ends
  (`desktop-net-missing-table.md`).
- **When the stream ends:** after `Ended`, or when the `events` receiver is dropped, the supervisor
  closes its socket and exits.

**Session rotation, make-before-break (D33; supersedes D17's timed rotation, review fix
PP-4).** The gateway token lives 15 minutes and the actor token in the conn params 2 hours. Both
are checked only when the WebSocket upgrades: a live socket keeps its identity for as long as it
stays open. So nothing rotates on a timer. A fresh session is minted on every reconnect (as
above), and the supervisor rotates only when an action on the live socket is refused with
`user/forbidden`, no sooner than `ROTATE_FLOOR` after the last rotation:

1. Mint a new session. A refusal that is final (403, 404, 401 after recovery, or a signed-out
   device) is not retried: keep the current socket, and end the stream with that error at its
   next drop (review fix PP-3). Any other failure keeps the current socket and retries on the
   backoff; nothing is sent.
2. Dial a second connection (steps 4–7) with its events buffered.
3. When it reaches `Connected`, swap the active connection. The buffer flushes, and events from the
   old socket are dropped from that moment.
4. Let the old socket's in-flight calls finish, up to `CLOSE_TIMEOUT` (2 s), then disconnect it and
   drop its client.

No `Status` edge is sent, so the player sees nothing. Presence stays put, because the live
connection count goes 1 → 2 → 1 (D18). The fresh connection's first `sync` is a no-op for the
store when `v` is unchanged. A `chat` line the new socket redelivers (same `msgId`) is already in
the log: the app neither adds it again nor pops its table-talk bubble a second time (review fix
PP-4).

**Actions** (`RemoteTable: TableCalls`). Each one uses the active connection when it is
`Connected`, else fails at once with `Err(Closed)`. Never let rivetkit queue a call and send it
late.

| Call | Action and args | Ok | Err |
|---|---|---|---|
| `submit_move(req)` | `submitMove`, `[req]` | `MoveAck` (decode failure → `Unavailable`) | `move_rejection(e)` → `Ok(MoveAck::Rejected{code, legal})`, else `action_error(e)` |
| `review_trick(req)` | `reviewTrick`, `[req]` | `ReviewTrickAck` | `action_error(e)` |
| `snapshot()` | `snapshot`, `[]` | `Option<PublicGameView>` (`null` → `None`) | `action_error(e)` |
| `get_hand_review(n)` | `getHandReview`, `[n]` | `Option<CompletedHandReview>` | `action_error(e)` |
| `close()` | stop the supervisor; `disconnect()` within `CLOSE_TIMEOUT`; drop the client | the stream ends with no `Ended`; idempotent | — |

- **`move_rejection`.** The table throws its refusals rather than returning them. It throws
  `user/game_complete`, `user/illegal_move` with `metadata.legal`, and `user/internal_error`;
  `stale_turn` comes back as a normal ack. Parse with
  `euchre_server_core::rivet::parse_rivet_error`. A `user` error whose code is an
  `euchre_protocol::ErrorCode` becomes `MoveAck::Rejected { code, legal }`, with `legal` from
  `UserErrorPayload::illegal_move_legal` or else empty. That is exactly what the sim returns, so
  the illegal-move path behaves the same in both. `forbidden` is not a rejection.
- **`action_error`.**
  - `user/forbidden` → `Forbidden`, and the supervisor rotates now (D33: the only rotation), so
    the actor token is fresh.
  - Any other `user/<code>` → `Invalid(code)`.
  - Any other structured error → `Unavailable`.
  - No structured error (for example "Socket closed during rpc") → `Closed`.
- **Bounds.** The bridge bounds each call (`SNAPSHOT_TIMEOUT_MS` 4 s and `ACTION_TIMEOUT_MS` 12 s,
  each plus 1 s grace) and reports every failure as `NoAck`. `RemoteTable` adds no timers of its
  own.

**Test hooks.**
- `interrupt_tables()` disconnects each open table's active socket through the same path a network
  drop takes: `Status(false)`, then the reconnect.
- `end_tables(err)` sends `Ended(err)` on each open table and stops it. N2 calls it with
  `ServerStopped`.

### N1.7 Recent tables

- **File.** `<state_dir>/recent-tables.json` holds a JSON array of game ids: newest first,
  deduplicated, at most `RECENT_LIMIT` (12), lowercase UUIDs only. It is saved atomically
  (temp file, then rename) with mode 0600. Unreadable → empty.
- **Remembered** after `create_table` and after `open_table` succeed.
- **`recent_tables()`.** No ids → `Ok(RecentTables::default())`, with no call. Otherwise
  `ensure_session()`, then `RecentTables`. Rows come back in the asked order. `status` parses
  `"active"` or `"complete"`, and `null` stays `None`.
- **Pruning.** A bookmark grants no access; the server rechecks ownership on every row. Never
  prune on a `null` row (the table may just be asleep). The 12-entry cap is the only pruning.

### N1.8 Backend methods

| Method | Behaviour |
|---|---|
| `name()` | `"remote"` |
| `identity()` | §N1.5 |
| `create_table(cfg)` | `CreateTable` with `{stick: cfg.stick_the_dealer, natural: cfg.require_natural_trump}` → `gameId`. On 503, or a connect error (nothing was sent), it retries with backoff from 250 ms to 2 s until `Api::create_retry_for()` (20 s) has passed. The first requests after a boot can see 503 while the runner settles (G36). A read timeout is not retried, because the table may exist: it reports `Timeout`. 429 → `RateLimited`. Then `remember`. |
| `open_table(id)` | §N1.6 |
| `list_games(cursor)` | `ListGames`; the cursor goes in through reqwest's `query` |
| `match_detail(id)` | `MatchDetail` |
| `stats()`, `get_settings()`, `patch_settings(p)` | `Stats`, `GetSettings`, `PatchSettings` |
| `recent_tables()` | §N1.7 |
| `host()` | `None` |

### N1.9 N1 tests

**Unit tests** (in the files N1 owns):

- `http.rs`:
  - `status_error_maps_every_status_per_call` (table-driven over §N1.4)
  - `wire_bodies_match_the_handlers` (the literal JSON each `euchre-http` handler writes)
- `device.rs`:
  - `missing_corrupt_foreign_or_expired_record_loads_none`
  - `save_is_atomic_and_private` (0600 file, 0700 dir, no temp file left)
- `recent.rs`:
  - `remember_moves_to_front_dedupes_and_caps`
  - `bad_file_loads_empty`
  - `invalid_ids_are_ignored`
- `table.rs`:
  - `illegal_move_becomes_a_rejected_ack` (an error built the way rivetkit-client 3.0.7 surfaces a thrown `user/illegal_move` with metadata)
  - `forbidden_and_other_errors_map`
  - `a_closed_socket_is_closed`
- `remote.rs`:
  - `remote_config_normalizes_and_refuses` (schemes, paths, query, credentials, default ports, slug)
- The supervisor, over a private `Dialer` seam with `tokio::time::pause`:
  - `drop_sends_status_false_then_reconnects_with_backoff`
  - `stale_session_is_minted_again_before_a_dial`
  - `rotation_swaps_without_a_status_edge`
  - `rotation_lets_the_old_sockets_calls_finish`
  - `forbidden_mint_ends_the_stream`
  - `not_found_and_unauthorized_mints_end_the_stream_too`
  - `gives_up_after_two_minutes_with_the_last_error`
  - `nothing_rotates_without_a_refused_action` (D33)
  - `a_refused_rotation_keeps_the_socket_then_ends_at_the_drop` (PP-3)
  - `a_forbidden_action_rotates_the_session_now`
  - `events_before_status_true_are_buffered_in_order`
  - `actions_fail_fast_while_down`

**`tests/remote_http.rs`** (wiremock, no engine, always runs):

- `every_call_sends_its_method_path_headers_and_body` (bearer where listed, never `Origin`, `Cookie` or `X-Euchre-Identity`; `profile=browser`)
- `first_call_enrolls_and_saves_the_device`
- `refreshes_at_eighty_percent`
- `reenrolls_when_refresh_is_refused`
- `a_401_recovers_once_then_reports_unauthorized`
- `concurrent_calls_share_one_enroll` (8 parallel calls, 1 enroll hit)
- `create_retries_503_then_succeeds`
- `create_gives_up_after_its_window` (`with_create_retry(500 ms)`)
- `table_session_statuses_map`
- `recent_tables_skips_the_call_when_empty_and_keeps_order`
- `recent_tables_recovers_an_unverified_session`
- `transport_failures_map_to_offline_and_timeout`
- `redirects_are_not_followed`

**`tests/remote_live.rs`** (gated, one `#[test]`, `remote_backend_plays_against_a_real_server`).
It runs on port `EUCHRE_TEST_ENGINE_PORT`, default 6741. The data root is
`$EUCHRE_TEST_TMP/n1-<pid>`. The server is started directly with
`euchre_server::start(ServerConfig::try_local(root/server)?.with_engine_port(P))`,
`fast_tempo = true` and `ai = AiConfig::default()`. The backend is
`RemoteBackend::new(RemoteConfig::remote(&server.url(), &root)?)`; this is the remote path, so the
state lands in `root/remote/<slug>`. Steps:

1. `identity()` enrolls. `device.json` is 0600. A second `identity()` makes no new enroll and
   returns the same user id.
2. `create_table` → id. `open_table` → `Status(true)`, then a `sync` within 10 s.
3. On the player's turn, `top_move` from `euchre_ai::heuristic` → `Ok(MoveAck::Ok)`.
4. A made-up move id → `Ok(MoveAck::Rejected { code: illegal_move, legal })` with `legal`
   non-empty.
5. `interrupt_tables()` → `Status(false)`, then `Status(true)` within 10 s. `snapshot()` → `Some`,
   with `v` at least what it was before.
6. A second backend with its own state dir (another device) opens the same table →
   `Err(Forbidden)`.
7. `recent_tables()` lists the table: `Active`, `hand_no` is `Some`.
8. `list_games(None)`, `stats()` and `get_settings()` answer, and `patch_settings` round-trips one
   field.
9. `close()` ends the stream. `server.shutdown()`. P, P+1, P+5 and the HTTP port are closed within
   5 s.

---

## N2. EMBEDDED and APP: the local server, its lifetime, and what the player sees

### N2.1 Backend choice, data dir, preferences

`BackendKind::choose(backend, server_url)` is pure. `from_env()` feeds it the two env values. The
order:

1. **A scene or headless run** (`rune::is_headless()`, or `--scene` in the args) is `Sim`, always.
   `lib.rs` decides this before `from_env()`. A `--snap` must never boot an engine; the shell and
   motion tests run the binary.
2. **`EUCHRE_DESKTOP_BACKEND`** set to `sim`, `embedded` or `remote` (any case) picks that kind.
   Any other value: warn and fall through.
3. **`EUCHRE_SERVER_URL`** set and non-empty → `Remote`.
4. **Otherwise `Embedded`.** This is the new default; `BACKEND_ENV`'s doc says so.

`build()` makes:
- `Sim` → `SimBackend::default()`.
- `Embedded` → `EmbeddedBackend::new(EmbeddedConfig::new(data_dir()))`. No IO.
- `Remote` → `remote_from_env()`. A bad or missing URL gives an unconfigured backend.

N2 rewords `NotWired`'s `Display` to "This copy of Euchre has no game server set up." N2 also gives
`Other(String)` the fixed `Display` "Something went wrong. Please try again." The string stays for
`Debug` and logs; `sim.rs` moves its one player-facing `Other` to `NotFound`.

`data_dir()` resolves `EUCHRE_DATA_DIR` if it is non-empty. Otherwise it uses
`ProjectDirs("sh","euchre","Euchre").data_dir()` (macOS: `~/Library/Application Support/sh.euchre.Euchre`),
and if there is none, `euchre-data`. The layout:

```
<data_dir>/                    0700
  prefs.json                   PrefsStore (existing)
  device.json                  the embedded server's device record (N1)
  recent-tables.json           the embedded server's recent tables (N1)
  logs/euchre.log(.1)          §N2.8
  server/                      ServerConfig::try_local's data dir
    secrets.json
    engine/                    engine storage, ENGINE_PID_FILE, ENGINE_PORT_FILE
  remote/<slug>/               one remote server's device.json and recent-tables.json (N1)
```

Preferences: `PrefsStore::for_run(scene_run, override)`. The override is `EUCHRE_PREFS_DIR`, else
`EUCHRE_DATA_DIR` (new: one variable now isolates a whole run), else the platform dir, which is
the same dir as `data_dir()`.

### N2.2 The supervisor

One task owns the `RunningServer`. It is the server task. `Supervisor` talks to it over channels
and never holds a lock across an `.await`.

- **`boot()`.** The bridge calls it once, spawned, at start. The window draws in parallel.
  1. Phase `Starting`.
  2. `engine_source().await` (§N2.6). On `Download` → phase `Downloading` →
     `download_engine().await` → on error, `Failed(EngineMissing)`.
  3. `ServerConfig::try_local(config.server_dir())`. On error → `Failed(DataConflict)`. Never use
     `ServerConfig::local`: its fallback is ephemeral secrets, which would orphan the device on
     every launch and fail a restart with `KeysConflict`.
  4. If `engine_port` is set, `.with_engine_port(p)`. Then `cfg.fast_tempo = config.fast_tempo`;
     if `offline_ai`, `cfg.ai = AiConfig::default()`; for `Bundled(p)` or `Cache(p)`,
     `cfg.engine.binary_path = Some(p)`.
  5. `euchre_server::start(cfg)` under `BOOT_WAIT` (120 s; the server's own ready wait is 60 s).
     - `Err(e)` → `Failed(boot_problem(&e))`, with `{e:#}` logged at warn.
     - Elapsed → `Failed(TooSlow)`. Dropping the future drops whatever it started.
  6. `Ok(server)` → `remote = Arc::new(RemoteBackend::new(config.remote_config(server.url()))?)`,
     published to `remote()`, then phase `Ready`, then the server task starts.
- **The server task** runs `select!` over two arms:
  - `server.closed()`: warn with the `ServerFailure`, `remote.end_tables(NetError::ServerStopped)`,
    phase `Stopped`, then `server.shutdown()` under `STOP_BUDGET` to reap the engine and free the
    ports.
  - A stop request: `remote.end_tables(ServerStopped)`, then `server.shutdown()` under
    `STOP_BUDGET` (25 s). If that runs out, drop the server inside `spawn_blocking` and await it,
    up to 17 s. Its `Drop` sends SIGTERM to the engine, then SIGKILL after 15 s. Then reply.
- **`restart()`.** From `Stopped` or `Failed`, wait for the old server task to finish, then run
  `boot()` again. From `Ready`, `Starting` or `Downloading` it does nothing (logged).
  - The URL stays the same: the server binds its loopback HTTP port sticky per process
    (`bind_http`).
  - The engine comes back on its recorded port. The token signing ring matches, so the device
    record stays valid.
- **`shutdown()`.** Idempotent.
  1. Cancel a boot in flight with a `CancellationToken`.
  2. Send the stop request and await its reply.
  3. The phase is left as it is; the window is gone.
- **`engine_pid()`.** `RunningServer::engine_pid()` while `Ready`.
- **Allowed edges only:**
  - `Starting` → `Downloading` → `Starting`
  - `Starting` → `Ready` or `Failed`
  - `Ready` → `Stopped`
  - `Stopped` or `Failed` → `Starting`

### N2.3 Delegation (`EmbeddedBackend: Backend`)

- **Every method** first runs `let remote = self.ready().await?`:
  - It waits on `phases()` for `Ready`, up to `BOOT_WAIT`.
  - `Stopped`, `Failed` or the wait running out → `Err(ServerStopped)`.
  - Then it delegates.
- **Error mapping.** `Offline` coming back from the delegate becomes `ServerStopped`: the server
  went away mid-call, and `closed()` is about to say so. Every other error passes through.
- **Calls made during the boot just wait.** A New game started while the engine boots shows the
  table's loading state (§N2.5) until the table opens. Home never waits.
- **The rest.** `name()` is `"embedded"`. `host()` is `Some(supervisor)`. `remote()` is
  `supervisor.remote()`.

### N2.4 Boot problems: mapping and copy

`boot_problem(err)` finds the first `euchre_server::BootError` in `err.chain()`. No `BootError`
means `Other`. A `try_local` error is mapped in `boot()`, not here.

| Cause | `BootProblem` | Heading | Line | Button |
|---|---|---|---|---|
| `EnginePortInUse{port}`. This is also what a second copy of Euchre on the same data dir gets: its engine holds the ports. | `PortInUse{port}` | Euchre is already open. | Close the other copy of Euchre, then try again. | Try again |
| `LowDisk{free_bytes}` | `LowDisk{free_bytes, need_bytes: MIN_FREE_DISK_BYTES}` | Euchre needs more disk space. | The game server needs 4 GB free. Free up some space, then try again. | Try again |
| `EngineBinaryMissing`, or the first-start download failed | `EngineMissing` | One part of Euchre is missing. | The first start downloads it. Connect to the internet, then try again. | Try again |
| `NotReady(_)`, or `BOOT_WAIT` ran out | `TooSlow` | The game server is taking too long to start. | Try again in a moment. Your games are saved. | Try again |
| `try_local` failed (`secrets.json` unreadable or unwritable) | `DataConflict` | Euchre can’t open its saved data. | Check that your disk isn’t full or locked, then try again. | Try again |
| `AlreadyRunning`, `HttpPortInUse`, `KeysConflict`, `TableConfigConflict` | `Relaunch` | Euchre needs to reopen. | Reopen Euchre to start the game server again. Your games are saved. | Reopen Euchre (`tx.relaunch()`) |
| `EnginePortChanged` (only a pinned test port can cause it), anything else | `Other` | The game server didn’t start. | Try again. Your games are saved. | Try again |
| `ServerPhase::Stopped` (`closed()` fired) | — | The game server stopped. | Restart to keep playing. Your games are saved. | Restart |

Every button except Reopen calls `AppState::restart_server()`.

### N2.5 What the player sees

**Server notice.** This is one component, `screens/server.rs`. It shows on every route with nav
(Home, New game, Games, Match detail, Settings) while `local_server` is `Stopped` or `Failed`.
- Layout: a calm banner at the top of the content. It uses the 06 §14.2 notice style (bg
  `#fff1d9`, border `#d5b68c`, radius 8, text `#233e32`) and `role="status"`.
- Content: heading, line and button from §N2.4.
- Keys: `server-notice`, `server-notice-action`.
- It reserves no space while hidden and slides in with the existing notice motion, so there is no
  layout shift when it appears.

**Booting** (`Starting` or `Downloading`). Nothing on Home: the boot is quiet and Home is live.
New game works.
- The table's loading view (06 §14.1) keeps the H1 "A seat at the table.".
- Its line reads "Getting the table ready…".
- On `Downloading`, the line reads "Getting the table ready. The first start takes a minute.".
- Once `Ready`, the existing lines take over ("Connecting to your game…", then "Dealing your
  cards…").
- Games and Settings show their own loading state until their reply comes.

**A table that ended** (`ConnEvent::Ended`, stage `Failed(copy)`):
- The last good view stays on screen; it is never blanked.
- Controls stay disabled.
- The 06 §14.2 banner shows the ended copy instead of "Reconnecting…":

| `Ended(..)` | Banner line | Action |
|---|---|---|
| `ServerStopped` | The game server stopped. Restart to keep playing. | Restart (`restart_server`) |
| `Forbidden` | This table belongs to another device. Open Your games to find your table. | Your saved games (→ Games) |
| `NotFound` | That table could not be found. | Your saved games |
| `Unauthorized` | Please start a game on this device first. | New game |
| `Unavailable`, `Offline`, `Timeout`, `RateLimited` | We couldn’t reach this table. Your game is saved. | Reconnect to this game (`reconnect`) |

With no view yet, the same copy shows as the loading view's error line (existing), and the primary
button follows the same table.

**Recovery after a restart.** When `Reply::Server(Ready)` arrives after `Stopped` or `Failed`:
1. Send `Identify` and `RecentTables` again.
2. If a table is open, `reconnect()` it: new conn id, same game id. The store resyncs to the same
   turn.
3. Reload the current route's data: Games sends `ListGames` + `RecentTables`, Match detail sends
   `MatchDetail`, Settings sends `GetStats` + `GetSettings`.

**Saved tables on Games** (06 §17.1; "this browser" becomes "this device"). Entering Games sends
`Intent::RecentTables` along with `ListGames`. When `recent_tables` has rows, a cream band
(`#f4f3e9`, radius 12, padding 20) sits above the list:
- h2 "Your saved tables".
- p "Resume on this device, exactly where the table is waiting."
- One link row per table, min height 48:

  | Row status | Label |
  |---|---|
  | `Active` | Resume game · Hand {hand_no+1} · {a}–{b} → |
  | `Complete` | Open game · Hand {hand_no+1} · {a}–{b} → |
  | `None` | Reconnect saved table · {first 8 chars of id} → |

- When `unavailable` is true, a status line: "Some saved tables could not be opened. Their links
  are kept above; try reconnecting. Completed matches are also listed below."
- A row calls `AppState::continue_table(game_id, tx)`:
  1. Close the current table.
  2. A new conn id, and `TableScreen::creating(conn_id, TableConfig::default())` with
     `game_id = Some(id)` and stage `Opening`.
  3. `Intent::OpenTable`.
  4. Navigate to `Route::Table`.
- Keys: `saved-tables`, `saved-table-<n>`.

**Quit and signals.**
- `lib.rs` adds `.on_quit(app::on_quit)`, which sends `Intent::Shutdown` as soon as a quit starts
  (Cmd-Q, the Dock, logout, the last window closing). The server's stop then overlaps the window
  teardown.
- After `try_run` returns, `Intent::Shutdown` is sent again (idempotent), then
  `thread.join_within(QUIT_WAIT)`. `QUIT_WAIT` is 500 ms with no host and 45 s with a host
  (`STOP_BUDGET` plus the drop bound plus margin). By then the window is gone; only the process
  stays a little longer.
- On unix, the bridge runtime listens for SIGINT and SIGTERM (`tokio::signal::unix`):
  - The first one posts `Reply::Quit`, and `AppState::receive` answers `tx.quit()`. That is the
    same path as Cmd-Q, so the server stops cleanly.
  - A second one calls `std::process::exit(130)`. The next launch reclaims the engine.

**The engine never outlives the app.**
- Clean quit: the bridge's `Shutdown` runs `host.shutdown()`, which runs `RunningServer::shutdown`
  under `STOP_BUDGET` and then the drop. Both stop the engine.
- A panic in the server task drops the server, which stops the engine.
- `kill -9` or a crash leaves the engine orphaned. The next launch's `euchre_server::start`
  reclaims it before it spawns a new one: it reads `ENGINE_PID_FILE`, matches the `--config` path
  and the dead owner, then sends SIGTERM and after 15 s SIGKILL.
- The desktop never signals an engine pid itself, except in the live tests, on a pid they started.

### N2.6 The engine binary

`engine_source()` checks, in this order, without network:

1. **`Env(p)`**: `RIVET_ENGINE_BINARY_PATH` is set. `try_local` already reads it. A missing file
   gives `EngineBinaryMissing`.
2. **`Bundled(p)`**: `bundled_engine(current_exe)` looks for `<exe dir>/rivet-engine`, then the
   macOS bundle's `<exe dir>/../Resources/rivet-engine` (`.exe` on Windows). This wave does no
   bundling; the slot is there for packaging.
3. **`Cache(p)`**: `rivetkit_engine_process::resolve_engine_binary_path(&EngineResolverConfig::from_parts(&engine_endpoint, None, None, None, false))`
   returns `Ok(p)`. That is rivetkit's cache `~/.rivetkit/engine/v3/3.0.7/rivet-engine-<target>`
   or a dev-tree `target/{debug,release}/rivet-engine`. Do not set `RIVETKIT_STORAGE_PATH`.
4. **`Download`**: nothing local.

`download_engine()` calls the same resolver with `auto_download = true`. It downloads into
rivetkit's cache, verifies the checksum, and allows 60 s per request.
- On the first launch the phase is `Downloading` while this runs.
- Offline, it fails → `Failed(EngineMissing)`, with the "connect to the internet" copy and Try
  again.
- After one successful launch the engine is cached, so play is fully offline from then on.

### N2.7 `NO_PROXY` at process start (`src/startup.rs`, called first in `main`)

- **Why.** The reqwest client inside rivetkit-client, and the server's engine client, read the
  system proxy settings (`macos-system-configuration`). A configured proxy would capture loopback
  calls to the engine and the `/rivet` gateway.
- **What to set.** The merged list is `merged_no_proxy(existing)`: the entries already in
  `NO_PROXY` or `no_proxy`, plus `127.0.0.1`, `localhost` and `::1`. When nothing is missing, it
  returns `None` and nothing changes.
- **How.** `std::env::set_var` is `unsafe` in edition 2024, and the workspace forbids unsafe code.
  So when the list changes and `EUCHRE_REEXEC` is unset, `main` re-executes itself:
  `std::os::unix::process::CommandExt::exec` on `current_exe()`, with the same `args_os`, plus
  `NO_PROXY`, `no_proxy` and `EUCHRE_REEXEC=1`.
  - This happens before any thread, runtime, logger or `rune::prewarm_gpu()`.
  - The process image is replaced, and the pid stays the same.
  - If `exec` fails, or on non-unix, log a warn and go on. The desktop's own reqwest clients set
    `.no_proxy()` for loopback anyway.

### N2.8 Logging

- Filter: `RUST_LOG` when set, else `warn,euchre_desktop=info`. That is quiet; rivetkit and engine
  chatter stays at warn.
- Window runs log to `<data_dir>/logs/euchre.log`.
  - At start, a file over 5 MiB is renamed to `euchre.log.1`, replacing the old one.
  - The file is append-only, mode 0600, with `with_ansi(false)`, through tracing-subscriber's
    `Mutex<File>` writer.
  - Also log to stderr when stderr is a terminal (`IsTerminal`).
- Scene and headless runs log to stderr only, and never create the log dir.
- The engine's own output goes where `LocalEngine` sends it, not into our file.

### N2.9 Scenes (deterministic: no network, no engine)

**New entries in `SCENES`.** Each one is added to the `tests/shell.rs` snapshot list.

| Scene | State |
|---|---|
| `server-starting` | Route Table, table `Opening`, no view, `local_server = Some(Starting)` |
| `server-downloading` | Same, with `Downloading` |
| `server-stopped` | Home with `Some(Stopped)`: the notice and Restart |
| `server-stopped-table` | The `table-play` view, disconnected, stage `Failed(ServerStopped copy)`: the ended banner and Restart |
| `server-failed` | Home, `Failed(Other)` |
| `server-failed-disk` | Home, `Failed(LowDisk{free 2 GiB, need 4 GiB})` |
| `server-failed-port` | Home, `Failed(PortInUse{port})` |
| `server-relaunch` | Home, `Failed(Relaunch)` |
| `games-saved-tables` | Games, with rows `Active`, `Complete` and `None`, and `unavailable: true` |
| `live-table`, `live-games`, `live-match` | Read `EUCHRE_LIVE_FIXTURE` (§3). A missing or bad file falls back to `table-play`, `games` or `match-detail`, with a warn. |

**`LiveFixture`** (`pub`, serde, in `scenes.rs`):

```rust
pub struct LiveFixture {
    pub views: Vec<PublicGameView>,
    pub games: Option<MatchListPage>,
    pub replay: Option<ClientMatchReplay>,
    pub recent: Option<RecentTables>,
}
```

- `live-table` lands `views[0]` the way the fixture scenes land their replayed view.
- `views[1]`, when present, applies on the second frame through `TableScreen::showing_then`, so the
  moment animates as it did live (trick won, hand result).

### N2.10 N2 tests

**Unit tests** (in the files N2 owns):

- `mod.rs`:
  - `backend_kind_rules` (the matrix in §N2.1, scene override included)
  - `data_dir_prefers_the_override`
- `prefs.rs`:
  - `prefs_follow_the_data_dir_override`
- `startup.rs`:
  - `merged_no_proxy_adds_only_what_is_missing`
  - `log_file_rotates_past_five_mib`
- `embedded.rs`:
  - `every_boot_error_maps_to_its_problem` (an `anyhow` chain with each `BootError`)
  - `bundled_engine_checks_exe_dir_then_resources`
- `embedded.rs`, over a private `Starter` seam whose fake `Served` has a url, `closed` and
  `shutdown`:
  - `boot_reaches_ready`
  - `boot_error_fails_with_its_problem`
  - `closed_stops_and_ends_tables`
  - `restart_from_stopped_boots_again`
  - `shutdown_while_booting_cancels`
  - `calls_wait_for_ready_then_delegate`
  - `calls_after_a_failure_answer_server_stopped`
  - `shutdown_is_idempotent`
  - `phases_follow_only_the_allowed_edges`

**`tests/server_states.rs`** (headless runtime, a test `Backend` and `ServerHost` double, no
engine):

- `starting_shows_a_seat_at_the_table`
- `downloading_says_the_first_start_takes_a_minute`
- `stopped_shows_the_notice_and_restart_sends_the_intent`
- `ready_after_stopped_reconnects_the_open_table_and_reloads_the_route`
- `failed_copy_is_plain_for_every_problem`
- `relaunch_button_relaunches`
- `ended_keeps_the_last_view_and_says_why`
- `games_lists_saved_tables_and_opens_one`
- `quit_reply_quits`
- `home_is_interactive_while_the_server_boots`

**`tests/embedded_live.rs`** (gated, one `#[test]`, `embedded_server_boots_restarts_and_stops`).
It runs on port `EUCHRE_TEST_ENGINE_PORT`, default 6860, with
`EmbeddedConfig::for_tests(root, P)`, through a real `BridgeThread` and `UiSink::channel()`.
Steps:

1. `Reply::Server(Starting)`, then `Ready` within 120 s.
2. `Identify` → ok; `device.json` is 0600 under the root.
3. Create and open a table → first `sync`.
4. `kill -TERM <supervisor.engine_pid()>` (the engine this test started) → `Server(Stopped)`
   within 15 s, and the table's stream gives `Ended(ServerStopped)`.
5. `Intent::RestartServer` → `Starting`, then `Ready`, at the same URL. Reopening the same game id
   syncs the same `handNo`, `turnId` and score.
6. `Intent::Shutdown` → the bridge ends; the engine pid is gone; P, P+1, P+5 and the HTTP port are
   closed within 5 s.
7. Same data dir: hold `127.0.0.1:P` with a `TcpListener` and boot again →
   `Failed(PortInUse{port: P})`. Release it, `RestartServer` → `Ready`, then shut down.

---

## 3. Live proof (integration agent)

The test is **`tests/live.rs`**: gated, one `#[test]`, `live_match_through_the_app`. It runs on
port `EUCHRE_TEST_ENGINE_PORT`, default 6870, with root `$EUCHRE_TEST_TMP/integration-<pid>` (kept
afterwards: it holds the snapshots). It uses the same harness as `tests/table.rs`:

- `Bridge::channel()`, `UiSink::channel()`, and
  `BridgeThread::spawn(BridgeConfig { backend: Arc<EmbeddedBackend>, sink, prefs: None }, intents)`.
- The runtime is `App::new(spec()).init(..).headless_runtime(..)` over
  `AppState::new(Prefs { tour_seen: true, .. }, Some(bridge), "embedded")`.
- `pump` moves replies into `s.receive`.

**Flow.** Every step has a deadline and fails with the state it saw.

1. **Boot.** `local_server` reaches `Some(Ready)` within 120 s. Identify is ok.
2. **New game.** Click `hero-play`, settle, click `new-submit`. The route is `Table`, and the first
   view lands within 15 s.
3. **Play to game over** within 10 min of wall clock (fast tempo).
   - Whenever the player can act, the human seat plays `euchre_ai::heuristic::top_move` over the
     store's view, the same ranker the hint uses. It plays through `AppState`'s own input methods
     (`play_card`, the bid and discard calls the table tests use, and Continue), never through an
     `Intent`.
   - Record the move-ack latency (submit to `Ack`) for each move.
4. **Illegal move refused by the server.** At a trick-play turn, open a second connection for the
   same device (`EmbeddedBackend::open_table(game_id)` on a test runtime) and send `submit_move`
   with a made-up move id.
   - Expect `Ok(MoveAck::Rejected { code: illegal_move, legal })` with `legal` non-empty.
   - The app's view keeps its `v`.
   - Close that connection.
5. **Reconnect mid-hand.** In hand 2 or later, at a player turn, call
   `backend.remote().unwrap().interrupt_tables()`.
   - The table shows the "Reconnecting… Your table is saved." banner, and controls are disabled.
   - Then `Status(true)` and a resync to the same `turnId` and `handNo`. Controls come back.
   - Play on. Record the reconnect time.
6. **Review gate.** `set_review_tricks(true)`.
   - The next completed trick waits: `awaitingTrickReview` is true, and nothing moves on for 2 s.
   - Continue resumes it. Then turn it off again.
7. **Game over.** `status == Complete`, the match-end panel is on screen, and the winner team
   matches the score.
8. **Games list.** Navigate to `Route::Games` and re-send `ListGames` every 500 ms for up to 30 s:
   the profile records the match a moment after it ends.
   - The row shows the final score and the outcome.
   - The saved-tables band lists this game as "Open game · …".
9. **Match detail.** Click the row → `Route::MatchDetail(id)` → the replay has at least one hand.
10. **Quit.** `Intent::Shutdown`, then join within 45 s. The engine pid is gone, and the ports are
    closed.

**Snapshots** (live data, rendered headless by the real binary):

1. At each moment, write a `LiveFixture` to `root/fixtures/<name>.json`:

   | Name | Moment |
   |---|---|
   | `bid` | the first bid turn |
   | `play` | a trick-play turn |
   | `trick-won` | the view before and the view after a trick completes |
   | `hand-result` | before and after `hand_scored` |
   | `game-over` | the final view |
   | `games` | the `MatchListPage` plus `RecentTables` |
   | `match` | the `ClientMatchReplay` |

2. After quitting, render each one with `env!("CARGO_BIN_EXE_euchre") --scene live-table|live-games|live-match --snap root/snaps/live-<name>.png`.
   Set these env vars on the command:
   - `EUCHRE_LIVE_FIXTURE=<file>`
   - `EUCHRE_DESKTOP_BACKEND=sim`
   - `EUCHRE_DATA_DIR` and `EUCHRE_PREFS_DIR` = `root/scene`
   - `NO_PROXY`
3. Assert that each PNG exists and is over 10 KB.
4. Print the snapshot dir. The integration agent opens every PNG and checks it looks right: the
   live table, cards, scores, the trick-won and hand-result moments, the match-end panel, the games
   list with its saved-tables band, and the match detail.

**Report** (`docs/port/reports/desktop-net.md`):
- Cold and warm boot time; create to first view; move-ack p50 and p95; reconnect time; quit time.
- Every gate's result and every fix made.

---

## 4. Test plan and port blocks

| Seat | Port P (engine binds P, P+1, P+5) | Live test | Data root (`EUCHRE_TEST_TMP`) |
|---|---|---|---|
| N1 | 6741 | `tests/remote_live.rs` | `…/scratchpad/net/n1/` |
| N2 | 6860 | `tests/embedded_live.rs` | `…/scratchpad/net/n2/` |
| Integration | 6870 | `tests/live.rs` | `…/scratchpad/net/integration/` |
| Reviewers | 6880, 6890, 6900 | any of the above, with `EUCHRE_TEST_ENGINE_PORT` | `…/scratchpad/net/review-{a,b,c}/` |
| Fix | 6910 | any | `…/scratchpad/net/fix/` |
| Verify | 6920 | all three | `…/scratchpad/net/verify/` |

`…` is `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1`.
Each live test reads `EUCHRE_TEST_ENGINE_PORT` (its default above) and `EUCHRE_TEST_TMP` (default
`std::env::temp_dir()`). It creates `<tmp>/<seat>-<pid>/`, prints it, and leaves it there when it
fails. Only one live test runs per port at a time.

```sh
cd /Users/brittianwarner/goods/euchre-rune
export CARGO_TARGET_DIR=$PWD/.cargo-targets/net-<lane> NO_PROXY=127.0.0.1,localhost,::1 no_proxy=127.0.0.1,localhost,::1
export EUCHRE_DATA_DIR=<root>/data EUCHRE_PREFS_DIR=<root>/data EUCHRE_TEST_TMP=<root>
cargo fmt -p euchre-desktop --check
cargo clippy -p euchre-desktop --all-targets -- -D warnings
cargo test -p euchre-desktop                                        # engine-free; all green
EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=<P> cargo test -p euchre-desktop --test <live> -- --nocapture  # Bash timeout 600000
```

Before a live run, check: `df -g` shows at least 4 GiB free, and `lsof -nP -iTCP:<P> -sTCP:LISTEN`
(also P+1 and P+5) is empty. Afterwards, the same `lsof` is empty again.

---

## 5. Server gaps (requests written, desktop works around them)

| Request | Gap | Desktop workaround |
|---|---|---|
| `docs/port/requests/desktop-net-http-types.md` | `POST /api/tables` (request and 201), `GET /api/tables/recent`, `GET /api/rivet/metadata`, `GET /api/session` and the error body are built with `json!`. No shared type exists, so clients mirror them. | `remote::http::wire` mirrors, each pinned by a test against the handler's exact JSON |
| `docs/port/requests/desktop-net-missing-table.md` | `POST /api/table-session/{id}` for a table that does not exist answers 503, the same as a passing outage. Recent-table rows cannot tell "gone" from "asleep". `GET /api/tables/recent` answers 200 with an empty list to an unverified bearer, not 401. | Reconnect gives up after 120 s with `Ended(Unavailable)`; `null` rows are never pruned; an empty list for non-empty ids triggers one session recovery |

---

## 6. Done means

- A plain launch boots the embedded server in the background and Home is live on the first frame.
  A full match plays to game over, Games lists it, and the match detail opens.
- With `EUCHRE_SERVER_URL` pointing at a running server, the same flow works remotely, as N1's live
  test proves through `RemoteConfig::remote`.
- `EUCHRE_DESKTOP_BACKEND=sim` and every scene still run with no engine.
- Stopping the engine shows "The game server stopped." Restart brings the same table back on the
  same turn. Quitting stops the engine every time.
- The gates are green with every existing test kept. The three live tests pass with
  `EUCHRE_E2E=1`. The live snapshots look right.
- No process of the wave is left running, and no port is left listening.
