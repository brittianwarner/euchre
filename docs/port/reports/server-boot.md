# Lane S1-BOOT report: the `euchre-server` composition root

**Status:** done. `apps/server` boots the real server: a spawned (or external) engine 3.0.7, one registry with `euchreTable`, `aiSeat`, `playerProfile` and `health`, and the HTTP API with the `/rivet` gateway in front. The end-to-end gate passed four times in a row. The last three runs used the final code.

The W3 review fixes on 2026-11-11 also changed crates under `crates/`. See "Review fixes" at the end.

**Files:**

- `apps/server/Cargo.toml`
- `apps/server/src/{lib,main,server,engine,process,error,health_actor}.rs`
- `apps/server/tests/e2e.rs`
- 05 §5.4 (text only)
- `W3-checklist.md` (ticks only)

`Cargo.lock` changed because the crate gained dependencies.

Not touched:

- `apps/desktop/**` and `apps/web/**`
- every crate under `crates/`
- the root `Cargo.toml`

## Gates

Run with `CARGO_TARGET_DIR=.cargo-targets/w3-boot` and `--offline`.

| Gate | Result |
|---|---|
| `cargo fmt -p euchre-server --check` | clean |
| `cargo clippy -p euchre-server --all-targets -- -D warnings` | clean |
| `cargo test -p euchre-server` | 11 unit tests passed. Both e2e tests print `SKIP` without `EUCHRE_E2E=1`. The doctest is `ignore`. |
| `EUCHRE_E2E=1 cargo test -p euchre-server --test e2e` | 2 passed, 4 runs in a row (timings below) |

Afterwards, `lsof -nP -iTCP:6860-6870 -sTCP:LISTEN` shows nothing listening, and no process from `.cargo-targets/w3-boot` is running.

## Boot order (`start` / `start_with`)

1. **Process lease.** Only one server runs per process. A second `start` while one is live returns `BootError::AlreadyRunning` at once. Then `assert_rivet_env_safety` runs, self-hosted for an external engine.
2. **Engine settings.**
   - Token: the configured token is used. `Spawn` without one generates a random one. `External` without one fails.
   - `Spawn` sets the endpoint to `http://127.0.0.1:<engine_port>` and the namespace to `LOCAL_ENGINE_NAMESPACE`.
3. **Bind the HTTP listener**, so the port is known. A local server (spawned engine, not production) then allows its own loopback origins: `http://127.0.0.1:<port>`, `http://localhost:<port>`, and `[::1]` when bound to IPv6. A host that is already configured keeps its origin. Without these origins every request gets `421 Unknown host`. D1 is unchanged: a present `Origin` must be on the list, and an absent one passes.
4. **Build the registry before any engine runs**, so a conflict leaves nothing behind:
   - `euchre_profile::register`: `KeysConflict` becomes `BootError::KeysConflict`.
   - `euchre_table::register` with `TableConfig::from_server_config` and the process-wide table client slot: `ConfigConflict` becomes `BootError::TableConfigConflict`.
   - One shared JSON client: `build_client_with_pool`, G26.
   - `euchre_ai_seat::register(.., AiSeatRuntime::from_config(SeatClient::new(client), &cfg))`. `dev_from_process_env` is never used.
   - `health`: TS `{pings}` state in SQLite (`JsonTable`), `ping() -> {ok, ts}`.
5. **Spawn the engine** (`EngineMode::Spawn`), in this order:
   1. Free-disk check: less than 4 GiB fails with `BootError::LowDisk`.
   2. Port check: a held port fails with `BootError::EnginePortInUse`. This check is skipped when the data dir records an engine pid, so `LocalEngine::start` can reclaim an orphan of ours.
   3. `LocalEngine::start` with the admin token and data under `<data_dir>/engine`. Download is off.
   4. If the start fails, the cause is diagnosed again: busy port, low disk, missing binary, or "the local engine did not start".
6. **Namespace bootstrap:** `ensure_rivet_namespace` with the default `AdminDialect::V3`:
   - `GET /namespaces`
   - `POST /namespaces {name}`
   - `PUT /pools/{pool}` with the flat body
7. **Serve the registry** with `ServeConfig::from_env()`, overridden with:
   - endpoint, token, namespace and pool
   - `EngineSpawnMode::Never`
   - `engine_auto_download = false` and `start_services = false`
   - for `Spawn`: loopback `engine_host` / `engine_port` and a fresh `version`, which always increases within a process
8. **Wait for the envoy.** `health.ping` must answer through the shared client (`wait_ready`, 60 s, G36). If the registry task exits first, boot fails with its error.
9. **Bind the table client slot and build the router.**
   - `ApiConfig::from_server_config` with `engine_token = Some(token)`, which turns on the Option A gateway token issuer.
   - The static SPA dir is used only when it exists. `index.html` is the fallback, D4.
   - The drain gate is layered on.
   - Then `set_ready`, and `axum::serve` with `ConnectInfo<SocketAddr>` and graceful shutdown.

If any step from 6 on fails, the partly built server is stopped in shutdown order and the error is returned.

## Shutdown

`RunningServer::shutdown().await`:

1. `Health::start_draining`: `/health` returns `503 draining`, and `POST /api/tables` returns `503 {"message": DRAINING_NEW_TABLE}`. Everything else still answers, including table sessions for existing tables.
2. HTTP stops accepting. In-flight requests finish, bounded at 10 s. Requests still open then get `503 {"message": RESTARTING}` and their connections close (2 s more), and only then is the serve task aborted (review fix O9).
3. The registry stops: 90 s grace when deployed or on an external engine, 20 s locally, 5 s once the engine is gone, then abort.
4. The table client slot is unbound.
5. `LocalEngine::shutdown().await`.

Dropping a `RunningServer` cancels everything at once, without the drain. `LocalEngine`'s own `Drop` still stops the engine: SIGTERM, then SIGKILL after 15 s.

`run(cfg)` is the deployed binary's entry point (`main.rs`). It calls `start_with(cfg, StartOptions::deployed())`, which serves `build/client`, then waits for SIGTERM, ctrl-c or `closed()` and shuts down. A second signal during the drain stops at once. A failure or a cut drain returns an error, so the exit is non-zero. `main.rs` logs to stderr with `RUST_LOG`, and reads its configuration with `ServerConfig::from_env()`. The default is `info`, with rivetkit's per-action targets at `warn` (review fix O8).

## Public API

```rust
pub async fn start(cfg: ServerConfig) -> anyhow::Result<RunningServer>;            // StartOptions::default()
pub async fn start_with(cfg: ServerConfig, options: StartOptions) -> anyhow::Result<RunningServer>;
pub async fn run(cfg: ServerConfig) -> anyhow::Result<()>;                         // deployed binary
pub struct StartOptions { pub static_dir: Option<PathBuf> }                        // ::deployed() = build/client
pub struct RunningServer { pub http_addr, pub engine_endpoint, pub namespace, .. }
impl RunningServer { fn url(), fn health() -> &Health, fn engine_pid(),
                     async fn closed(&self) -> ServerFailure, fn failure() -> Option<ServerFailure>,
                     async fn shutdown(self) }
pub enum BootError { AlreadyRunning, EnginePortInUse{port, ports},
                     EnginePortChanged{recorded, requested}, HttpPortInUse{addr},
                     LowDisk{path, free_bytes}, EngineBinaryMissing, KeysConflict,
                     TableConfigConflict, NotReady(Duration) }
pub enum ServerFailure { EngineExited{status}, RunnerStopped{reason} }
pub const MIN_FREE_DISK_BYTES: u64;      // 4 GiB
pub const DEFAULT_STATIC_DIR: &str;      // "build/client"
pub const DRAINING_NEW_TABLE: &str;
pub const RESTARTING: &str;              // 503 body once the HTTP drain runs out
pub mod health_actor;                    // HEALTH_ACTOR, health_key(), Ping, PingReply, register()
pub use euchre_server_core::ServerConfig;
```

Boot errors a caller can act on come inside the `anyhow::Error` as `err.downcast_ref::<BootError>()`, with the underlying cause still in the chain. Every message names the cause and what to change. Example: "engine port 6865 on 127.0.0.1 is already in use (the engine binds 6865, 6866, 6870); pick free ports with ServerConfig::with_engine_port or RIVET_RUN_ENGINE_PORT".

### Process-wide rules

`euchreTable` and `playerProfile` keep their config in process-wide statics, so:

- One server runs per process at a time (`AlreadyRunning`).
- Every server in a process must use one secrets set. A different set fails with `KeysConflict` before any engine starts.
- The table config also compares the allowed origins and the client slot:
  - The client slot is one static `TableClient`, re-bound on each start.
  - A local server re-binds its previous loopback HTTP address on a restart in the same process, so the loopback origins stay the same. This is the "sticky" port, and it applies only when the configured bind port is 0.
- Fast tempo must also stay the same across starts.

## End-to-end tests (`apps/server/tests/e2e.rs`)

The tests drive the server only as a real client would:

- `reqwest` with no `Origin` and a bearer session.
- rivetkit-client 3.0.7 with `EncodingKind::Json`, aimed at `/api/rivet/metadata`'s `clientEndpoint` (`<base>/rivet`) and `clientNamespace`.
- The engine-issued `gatewayToken` from the table session, and conn params `{token, protocolVersion: 1}`.

Both tests share one runtime and run one after the other, behind a mutex. Every wait is bounded:

| Wait | Bound |
|---|---|
| Call | 15 s |
| Next event | 30 s |
| View condition | 90 s |
| Profile record | 30 s |
| Failing boot | 90 s |
| Busy-port failure | 5 s |
| Whole test | 480 s |

Every server uses `ServerConfig::try_local(tempdir).with_engine_port(..)`, with offline AI and fast tempo. Each data dir gets the same `secrets.json`, written with mode 0600.

**`full_match_through_the_server`** (engine 6860):

1. Boot. `/health` returns `200 ok`. A concurrent second `start` gets `AlreadyRunning`.
2. `POST /api/device/enroll`, then `POST /api/tables` (3 AI, bounded retry on 503), then `POST /api/table-session/{id}`, which carries a `gatewayToken`. Connect through `/rivet`. The first `sync` has empty steps and `you: 0`.
3. **Review gate:**
   1. `reviewTrick set true` returns `{queued:true}`, and the view shows `reviewTricks`.
   2. **Illegal move:** at the first player turn, `not-a-move` is refused with `user/illegal_move` and `metadata.legal` equal to the view's legal moves. The same turn then takes a legal move.
   3. Play to the first finished trick. It holds (`trick_resolve`, `awaitingTrickReview`) for 800 ms.
   4. `continue` with its `turnId` moves the game on.
   5. `set false`.
4. **Reconnect:** at a player turn mid-hand, disconnect, then reconnect through the same gateway client. The first `sync` has the same `turnId` and `handNo`.
5. **Drain and restart mid-match:**
   1. At a player turn in hand 2 or later, start draining.
   2. `/health` returns `503 draining`, `POST /api/tables` returns `503 DRAINING_NEW_TABLE`, and a table session still works.
   3. `shutdown()`. Ports 6860, 6861, 6865 and the HTTP port close.
   4. `start` on the same data dir, on the same HTTP port.
   5. Mint a new table session and reconnect. The first `sync` has the same `turnId`, `handNo` and `score`.
6. Play to `status: "complete"`.
7. **Profile over HTTP:**
   - `GET /api/games` has one row: `complete`, with the outcome and score of the final view.
   - `GET /api/profile/stats?profile=browser` matches 1 game with the right win or loss.
   - `GET /api/games/{matchId}` returns `complete` with at least 1 hand.
8. `shutdown()`, and every port closes.

**`boot_failures_and_a_second_server`** (engine 6865):

1. A second server in the same binary, with the shared secrets, a new data dir and the 6865 block, boots, enrolls, creates a table, mints a session and shuts down.
2. A fresh data dir with its own secrets gets `KeysConflict` in about 10 ms, and nothing listens on 6865.
3. With 6865 held by a plain `TcpListener`, boot fails with `EnginePortInUse { port: 6865 }` in under 100 ms.
4. The shared secrets then boot again, which shows the lease was released.

### Timings

| Run | Boot | Restart: shutdown / boot | Match to game over | Replay hands | Busy port | Keys conflict | Suite |
|---|---|---|---|---|---|---|---|
| 1 | 4.99 s | 3.13 s / 4.87 s | 51.1 s | 11 | 65.1 s (before the fix) | 8 ms | 141.1 s |
| 2 | 4.78 s | 3.13 s / 4.50 s | 34.6 s | 7 | 90 ms | 11 ms | 58.8 s |
| 3 | 5.33 s | 3.15 s / 4.51 s | 72.9 s | 18 | 89 ms | 10 ms | 98.3 s |
| 4 | 5.09 s | 3.11 s / 4.47 s | 45.6 s | 11 | 89 ms | 14 ms | 70.6 s |

- **Second-server boot** took 4.8–5.4 s.
- **Final shutdown** took about 3.1–3.2 s each time.
- **Match length** varies with the deal, from 7 to 18 hands.

**The busy-port fix.** Run 1 found that a busy port took 65 s to fail. The engine does not exit when it cannot bind, so the start waited out its 60 s ready timeout. `engine::spawn` now checks the ports before the start, unless the data dir records an engine pid (the orphan reclaim case). The ports are still checked again after any failed start. The unit test `a_busy_port_fails_fast_without_an_orphan_record` covers this without an engine.

**Expected warnings.** Every warning and error in a run is expected:

- the logged `illegal_move` dispatch
- `playerProfile keys already configured` from the conflict case
- `notifyAiSeats failed` and `dispatch to aiSeat failed; heuristic stands` (`actor_stopped_while_waiting`, "queue delivery interrupted") from tables that were still dealing while the second server shut down

None appeared around the mid-match restart.

## Left for W4

1. **Desktop wiring.** `RemoteBackend`'s embedded mode should call `euchre_server::start(ServerConfig::local(app_data_dir))`, use `server.url()` as the base, and `shutdown().await` on exit. Dropping the server skips the drain, and `LocalEngine`'s Drop blocks for up to 15 s.
   - A second window or instance in the same process gets `AlreadyRunning`.
   - Map each `BootError` to a plain UI message. `LowDisk` and `EnginePortInUse` already say what to change.
2. **Dev mode.** `run` covers the deployed binary through `ServerConfig::from_env()`, which needs the production variables. A `--dev` / `--local <dir>` flag on `main.rs` that runs `ServerConfig::local` with `StartOptions::deployed()` would give local dev a one-command server. It is not built.
3. **Docs to sync.** Describe the static dir (`build/client`, relative to the working directory) and the boot and shutdown order above in `apps/server` docs and ARCHITECTURE §2.4. The env table in 05 §6 needs no change.
4. **Pool config in local mode.** For a loopback endpoint, rivetkit-core 3.0.7 (`registry/runner_config.rs`) re-PUTs `/pools/{pool}` with only `{"drain_on_version_upgrade": true}` after our bootstrap. A local engine therefore loses the eviction pacing fields. That is harmless with one runner per process. A deployed (non-loopback) endpoint keeps our body. If pacing matters locally, run the bootstrap again after the runner is up.
5. **The sticky HTTP port.** It holds only within one process, and only while nothing else takes the port between a shutdown and the next start. If another process takes it, the restart fails with `HttpPortInUse`, not with a new port. A new port would change the loopback origins and hit `TableConfigConflict`.
6. **`/health` during boot.** The listener is bound at step 3 but served only after the runner answers. A health check during engine boot therefore waits in the accept backlog instead of getting `503`. This matches the 05 §12.1 order. Serve a health-only router first if the Railway check needs an early 503.
7. **Gateway WebSockets on shutdown.** `axum` graceful shutdown does not wait for upgraded `/rivet` WebSockets. They close when the registry and engine stop, and the client reconnects, as the e2e restart shows.
8. **Not covered here.** Live headless snapshots against the embedded server stay open on the checklist. S2-CRATES did the `recordGame` 64 KiB check (`reports/w3-crates.md` §3). This e2e plays to 10 only.

## Review fixes (2026-11-11)

The W3 server review raised 4 security, 9 operations and 2 web-compat findings. Every one was checked before any code changed. The P1 was reproduced with a throwaway probe test, since deleted: a table session got `200` on engine port 6910, and the same storage restarted on 6912 got `503` for good. Every P0–P2 is fixed except the web-side P2, which is listed for W4. Every P3 is fixed, though O4 only partly: one client cannot be changed and needs `NO_PROXY`. D30's callers are done too.

**Files changed:**

- `apps/server/src/{lib,main,server,engine,process,error}.rs` and `apps/server/tests/e2e.rs`
- `crates/euchre-server-core/src/rivet.rs` and `crates/euchre-server-core/src/config.rs`. `rivet.rs` is outside this lane's listed files, but O1, O3 and O4 live in `LocalEngine`, so the fix had to go there.
- `crates/euchre-http/src/{static_files.rs,gateway/mod.rs,gateway/tests.rs,api/tables.rs,api/profile.rs,api/tests.rs}` and `crates/euchre-http/tests/http_e2e.rs`
- `crates/euchre-profile/src/actor.rs` and `crates/euchre-profile/tests/profile_e2e.rs`
- Docs: `W4-notes.md` (new), `W3-checklist.md` (the D30 tick), and `requests/done/server-profile-d30-forget-episode-callers.md` (moved there, with a resolution).

`Cargo.lock` and `Cargo.toml` did not change.

### Security

| # | Finding | Outcome |
|---|---|---|
| S1 | P3: the static dir serves dotfiles | **Fixed.** `static_files::is_hidden` gives `404 Not found` to any decoded path with a segment that starts with `.`, before `ServeDir` runs. That covers `/.env`, `/%2eenv`, `/.git/config` and `..`. `/.well-known/*` is refused too; the web build has none. |
| S2 | P3: the `.map` block is bypassed by letter case or a trailing slash | **Fixed.** The extension is compared case-insensitively after trailing `/` and `\` are trimmed (`.MAP`, `.Map`, `.map/`, `.map//`). |
| S3 | P3: `//<dir>` redirects to a scheme-relative `Location` | **Fixed.** `ServeDir` now has `append_index_html_on_directories(false)`, so it never redirects. A directory path is a client route and gets `index.html` with `no-cache`. A test checks that `/art`, `//art`, `/art/`, `//_app` and `/` get `200` and no `Location`. |
| S4 | P3: engine internal 5xx errors pass through (metadata leak) | **Fixed.** On an upstream 5xx, `passthrough_http` reads the body, up to 64 KiB. A JSON object loses its `metadata` and keeps `group`, `code` and `message`. A body that is too big or cannot be read becomes `502 upstream_error`. A non-JSON body passes unchanged. Test: `http_passthrough_scrubs_engine_error_metadata`. Not covered: a CBOR error body (clients use JSON, D19) and WebSocket frames. |

### Operations

| # | Finding | Outcome |
|---|---|---|
| O1 | P1: an engine port change on a used data dir makes the JWT ring refuse every gateway token, so table sessions get `503` for good | **Fixed, three ways.** (1) After ready, `LocalEngine::start` records the guard port in `<storage>/.rivetkit/var/engine/euchre-engine.port`. For a different port it refuses with `EnginePortMismatch` before anything is reclaimed or written. Storage from before the record falls back to the topology `public_url` in `config.json`. (2) `ServerConfig::local(dir)` reuses the recorded port (`recorded_engine_port`), so the desktop never moves. (3) `engine::spawn` checks first and returns `BootError::EnginePortChanged { recorded, requested }`. In the e2e, the restart now uses `ServerConfig::try_local(same dir)` with no port: it picks the same port and mints table sessions, gateway token included. A moved port is refused in 91–179 ms with nothing listening. Unit tests: `the_recorded_port_comes_from_the_record_then_the_topology`, `a_new_port_on_used_storage_is_refused`, `local_mode_reuses_the_recorded_engine_port`, `a_moved_port_is_refused_before_anything_starts`. |
| O2 | P2: nothing supervises the engine or the registry after boot | **Fixed.** A supervisor polls the spawned engine every second (`LocalEngine::try_wait`). A drop guard on the registry task reports when it returns, errors or panics. The first failure, unless shutdown already started, logs an `error`, turns `/health` to `503` for good and resolves `RunningServer::closed() -> ServerFailure` (`EngineExited` or `RunnerStopped`). `run` selects on it, stops, and returns the failure, so the exit is non-zero. Shutdown gives the registry 5 s once the engine is gone. In the e2e, the engine gets `kill -KILL`: the failure is noticed in 0.40–0.64 s, `/health` is `503`, and shutdown takes 5.0 s with every port closed. Limit: the old envoy keeps retrying the dead engine for its own stop budget, about 30 s, and logs `connection failed`. rivetkit 3.0.7 keeps the envoy handle `pub(crate)` (W4-notes §4). |
| O3 | P3: a stale pid file plus a busy port takes 65 s to fail | **Fixed.** `LocalEngine::start` checks all three ports after the orphan reclaim and fails at once. `diagnose` maps that to `EnginePortInUse`. Test: `a_held_port_fails_fast_after_a_stale_pid_record` (about 1 s, bounded at 5 s). |
| O4 | P3: `LocalEngine` probes go through `HTTP_PROXY` | **Fixed** for the probes and also for the gateway proxy and token issuer client (`.no_proxy()`). The engine admin client already had it. **Not fixable here:** the rivetkit-client 3.0.7 actor client builds `reqwest::Client::new()`, so `NO_PROXY=127.0.0.1,localhost,::1` must be set wherever a proxy is (W4-notes §6). |
| O5 | P3: the dev `[::]` bind starts beside a `127.0.0.1` squatter, and `url()` points at it | **Fixed.** A local wildcard bind on a known port is refused with `HttpPortInUse { 127.0.0.1:P }` when `127.0.0.1:P` is held. `url()` maps `[::]` to `[::1]` and `0.0.0.0` to `127.0.0.1`. Tests: `a_wildcard_bind_beside_a_loopback_squatter_is_refused`, `the_url_uses_the_loopback_of_the_bound_family`. |
| O6 | P3: the cached engine binary is not found when `RIVETKIT_STORAGE_PATH` is set | **Fixed.** With `RIVETKIT_STORAGE_PATH` set, no binary configured and no `RIVET_ENGINE_BINARY_PATH`, `engine::spawn` uses `$HOME/.rivetkit/engine/v3/3.0.7/rivet-engine-<target>` when it exists. Otherwise rivetkit's own order is unchanged. Test: `the_home_cache_is_used_when_nothing_else_names_a_binary`. |
| O7 | P3: a second ctrl-c during the drain is ignored | **Fixed.** `run` races the drain against a second SIGTERM or ctrl-c. On the second signal it logs a `warn` and drops the drain, which drops the server and stops the engine. It then returns an error. Not tested end to end, because a test cannot signal its own process safely. |
| O8 | P3: the default `info` log level logs every rivetkit action | **Fixed.** The default is `info,rivetkit_core::actor=warn,rivetkit_core::registry=warn,rivet_envoy_client=warn,rivet_envoy_client::connection=info,rivet_envoy_client::envoy=info`. `RUST_LOG` still overrides it. A test checks that it parses. |
| O9 | P3: the HTTP drain abort does not close in-flight connections | **Fixed.** After the 10 s drain, a hard-stop token makes `drain_gate` end every open handler with `503 {"message": RESTARTING}`. Graceful shutdown then closes those connections. The serve task is aborted only if they are still open 2 s later. Test: `the_hard_stop_ends_open_requests_with_503`. Limit: a response body that is already streaming is not cut. Gateway replies are short actor responses, and upgraded WebSockets close when the registry and engine stop (W4 item 7). |

### Web-compat

| # | Finding | Outcome |
|---|---|---|
| W1 | P2: the web dev build sends no gateway token | **Skipped here, with a reason.** The fix is in `apps/web/src/lib/client/rivet.ts`, which this wave must not edit. ARCHITECTURE D3 keeps the token required in every mode, so the server does not change. W4-notes §1 has the exact change: always pass `getToken`, use the `/rivet` endpoint, and use `encoding: 'json'`. |
| W2 | P3: `table_session` does not refresh the recent-tables cookie | **Fixed on the server.** For cookie (ambient) callers, `POST /api/table-session/{gameId}` sets `euchre_recent_tables` with that game first, deduplicated, under the same cap as create. This replaces the TS page-load `rememberTable`, which a static SPA never runs. Bearer callers get no cookie. `http_e2e` checks the order after two creates and a session. |

### D30 (`forgetEpisode` callers)

**Done.** The work follows `requests/done/server-profile-d30-forget-episode-callers.md`:

- the actor uses `require_episode_id`;
- `DELETE /api/episodes/{id}` uses `is_valid_episode_id`;
- `api/tests.rs` uses `verna.m1.0.bid` and refuses `ep-1`, `verna.m1.03.bid` and `verna.m1.0.nope`;
- `profile_e2e` forgets a stored id and then re-records it.

The checklist line is ticked.

### Also fixed

`euchre-server-core`'s `config::tests::local_engine_port_block_is_free` failed about 1 run in 4, before these changes and without them. Other tests in the binary pick ports at the same time. It now passes if any of 5 picks is free when checked, and it passed 10 runs in a row.

### Gates after the fixes

Run with `CARGO_TARGET_DIR=.cargo-targets/w3-boot` and `--offline`. `fmt --check` and `clippy --all-targets -- -D warnings` are clean for each crate.

| Crate | `cargo test` without `EUCHRE_E2E` |
|---|---|
| `euchre-server` | 17 lib and 1 bin unit tests pass. The 2 e2e tests print `SKIP`. The doctest is `ignore`. |
| `euchre-server-core` | 65 lib and 8 `foundation_auth` tests pass. The 2 `foundation_probe` tests print `SKIP`. |
| `euchre-http` | 134 lib tests pass. The 4 `http_e2e` tests print `SKIP`. 1 doctest is `ignore`. |
| `euchre-profile` | 17 lib tests pass. The 3 `profile_e2e` tests print `SKIP`. 1 doctest is `ignore`. |

The engine suites ran with `EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6910`, one at a time. The server e2e now reads the same variable: its first engine binds P, P+1 and P+5, and its second binds P+2, P+3 and P+7. Every suite passed twice in a row on the final code.

| Suite | Run 1 | Run 2 |
|---|---|---|
| `euchre-server --test e2e` | 2/2, 90.3 s | 2/2, 72.4 s |
| `euchre-table --test table_e2e` | 4/4, 49.7 s | 4/4, 53.0 s |
| `euchre-table --test table_ai_e2e` | 4/4, 127.5 s | 4/4, 100.8 s |
| `euchre-table --test seams_e2e` | 1/1, 74.3 s | 1/1, 79.1 s |
| `euchre-ai-seat --test ai_seat_e2e` | 6/6, 12.9 s | 6/6, 13.1 s |
| `euchre-http --test http_e2e` | 4/4, 7.2 s | 4/4, 7.2 s |
| `euchre-profile --test profile_e2e` | 3/3, 8.3 s | 3/3, 10.5 s |

Two earlier server e2e runs also passed, in 93.3 s and 98.3 s, before the last edits, which were the `RESTARTING` export and the `RIVETKIT_STORAGE_PATH` check for the binary fallback.

Server e2e on the final code:

- boot: 4.75 s and 5.40 s;
- restart: shutdown 3.16 s, boot 4.49–4.51 s;
- busy port: 88–110 ms;
- keys conflict: 10–11 ms;
- the dead engine was noticed in 0.40–0.58 s.

Afterwards nothing listens on 6910–6919, and no process from `.cargo-targets/w3-boot` runs. The reviewers' `.cargo-targets/w3-review-*` and `.cargo-targets/w3-crates` dirs were already gone.
