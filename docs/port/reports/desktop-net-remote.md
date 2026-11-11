# Desktop net wave, lane N1: the remote backend

`RemoteBackend` is built to `desktop-net-contract.md` §N1. It is the HTTP API plus the
rivetkit-client 3.0.7 backend, and it serves both modes: a remote server
(`RemoteConfig::remote(EUCHRE_SERVER_URL, data_dir)`) and the embedded one
(`RemoteConfig::embedded(RunningServer::url(), data_dir)`, which N2 builds).

Gates: fmt (N1 files), clippy `-D warnings --all-targets`, and `cargo test -p euchre-desktop`
(234 tests, engine-free) are green. The gated live test passed 3 times in a row against a real
server on port block 6741.

## Files

| File | What it holds |
|---|---|
| `apps/desktop/src/net/remote.rs` | `RemoteConfig` (normalize, refuse, slug), `RemoteBackend` (`new`, `unconfigured`, `config`, `interrupt_tables`, `end_tables`), `impl Backend`, the real `Dialer` (`TableDialer`), the open-table registry |
| `apps/desktop/src/net/remote/http.rs` | `Api` (one `reqwest` client), `Call`, `status_error`, `wire` types, the 12 calls, the create retry |
| `apps/desktop/src/net/remote/device.rs` | `DeviceStore`: load (`None` = enroll), save atomically with mode 0600 into a 0700 dir |
| `apps/desktop/src/net/remote/session.rs` (new, private) | the device session: plan (enroll / refresh at 80 % / use), single-flight, `ensure`, `recover`, `authed` (one recovery per 401) |
| `apps/desktop/src/net/remote/recent.rs` | `RecentStore`: `recent-tables.json`, newest first, deduplicated, at most 12, lowercase game ids only |
| `apps/desktop/src/net/remote/table.rs` | one table: open, the reconnect supervisor, make-before-break rotation, `RemoteTable: TableCalls`, `move_rejection`, `action_error`, the rivetkit dial |
| `apps/desktop/tests/remote_http.rs` | 14 wiremock tests (always run) |
| `apps/desktop/tests/remote_live.rs` | the gated live test (`EUCHRE_E2E=1`) |

## The calls

Every path is relative to the normalized origin. Every call sends `Accept: application/json` and
the user agent `euchre-desktop/<version>`. Calls with a body add `Content-Type: application/json`.
No call sends `Origin`, `Cookie` or `X-Euchre-Identity`. Redirects are never followed, and
loopback hosts skip any proxy. Timeouts are connect 3 s and request 10 s.

| Call | Request | Auth |
|---|---|---|
| Enroll | `POST /api/device/enroll` | none |
| Refresh | `POST /api/device/refresh` | bearer |
| Session | `GET /api/session` | bearer |
| CreateTable | `POST /api/tables` `{stick, natural}` → 201 `{gameId}`. Retries 503 or an unsent request (250 ms doubling to 2 s, within 20 s). A read timeout is never retried. | bearer |
| RecentTables | `GET /api/tables/recent?ids=a,b,…` | bearer |
| TableSession | `POST /api/table-session/{gameId}` | bearer |
| RivetMetadata | `GET /api/rivet/metadata`, cached per backend after the first success | none |
| ListGames | `GET /api/games?profile=browser&limit=10[&cursor=]` | bearer |
| MatchDetail | `GET /api/games/{matchId}?profile=browser` (one encoded path segment) | bearer |
| Stats, GetSettings, PatchSettings | `GET` / `GET` / `PATCH /api/profile/{stats,settings}?profile=browser` | bearer |

`status_error` follows §N1.4 row for row:

- No status: `Timeout` if the request timed out, otherwise `Offline`.
- 400: `NotFound` for TableSession and MatchDetail; `Invalid(code or "bad_request")` for every
  other call.
- 401: `Unauthorized`, after the one recovery.
- 403: `Forbidden` for TableSession; `Invalid("forbidden")` for every other call.
- 404: `NotFound`. 413: `Invalid("too_large")`. 429: `RateLimited`.
- Any other status: `Unavailable`.

A 2xx body that does not decode is `Unavailable`. An error body is read only for `error.code`,
and only when it is 16 KiB or less. Logs never include a token, a body or `device.json`.

## Device session

- `device.json` lives in `<state_dir>`. For a remote server, `state_dir` is
  `<data_dir>/remote/<slug>` and the key is the origin. For the embedded server, `state_dir` is
  `data_dir` and the key is `"embedded"`.
- A record is discarded, and the device enrolls again, when it is missing, unreadable, for
  another server key, or expired.
- At 80 % of the record's lifetime the session refreshes. If the refresh answers 401, it enrolls
  again, which makes a new guest; this is logged. If the refresh fails any other way, the current
  record stays in use.
- One `tokio::sync::Mutex` is held across the network call, so 8 parallel first calls share one
  enroll. The test proves this.
- A call that answers 401 recovers once: refresh, and enroll if the refresh answers 401 too. Then
  the call runs again. A second 401 is `Unauthorized`. A caller that lost the race reuses the
  session the winner already recovered.
- Save failures are logged and the session stays in memory. All file IO goes through
  `spawn_blocking`.

## One table connection

**Open.**

1. Mint the table session, through `authed`.
2. Fetch the gateway metadata (cached).
3. Build a fresh `Client`: json encoding, `disable_metadata_lookup(true)`,
   `token(gatewayToken)`.
4. Call `get_for_id("euchreTable", actorId)` with `TableConnectParams::player(token)`, then
   `connect()`.
5. Add one `on_event` for each of the 5 event names.
6. Wait for `Connected` within 10 s.
7. Remember the table, then return the stream with `Status(true)` first. Events that arrive
   before it are buffered and follow it in order.

A session with no gateway token is `Unavailable`, and nothing is dialed. A failed or cancelled
dial disconnects its half-built connection and drops the client (`CloseOnDrop`).

**Supervisor.** It is one task per table, on the bridge runtime.

- **On a drop:** send `Status(false)`, then close the socket. Redial with backoff: 250 ms doubling
  to 8 s, ±20 % jitter.
- **Before a redial:** mint a fresh session if the old one is stale, the previous dial failed, or
  an action answered `forbidden` / `invalid_token`.
- **Mint errors that end the stream:** `Forbidden`, `NotFound` and `Unauthorized` end it at once.
- **Other errors:** keep trying for 120 s after the drop, then send `Ended(<last error>)`.
- **When the stream ends:** after `Ended`, `close()`, or a dropped `events` receiver, the
  supervisor closes its socket and exits.

**Rotation.** It runs at `expiresAt − 10 s`, while the table is up, make-before-break:

1. Mint and dial a second socket, with its events buffered.
2. Swap the active socket. From that moment, the old socket's unread events are dropped.
3. Let the old socket's in-flight calls finish (up to 2 s), then close it.

No `Status` edge is sent. A failed rotation keeps the current socket and retries on the backoff;
nothing is sent.

**Actions.** Each action uses the active socket only when it is `Connected`; otherwise it fails at
once with `Closed`. Each action races the socket's `lost()`, so a call in flight when the socket
drops answers `Closed` at once instead of waiting.

- `move_rejection` turns the table's thrown `user/<ErrorCode>` into
  `MoveAck::Rejected { code, legal }`, just as the sim returns it.
- `action_error`: `user/forbidden` is `Forbidden`. Any other `user/<code>` is `Invalid(code)`.
  Any other structured error is `Unavailable`. No structured error is `Closed`.
- `close()` is idempotent and ends the stream with no `Ended`.

**rivetkit-client 3.0.7.** The table connection works around two behaviours:

- **The client redials by itself.** The moment a socket drops, the client redials with the old
  credentials.
- **A late `disconnect()` hangs and leaks the socket.** It cannot abort a handshake already under
  way.

`dial_rivet` registers an `on_status_change` callback first, which holds a weak reference to the
connection. On the first `Disconnected`, the callback calls `disconnect().now_or_never()`. The
first poll of that future sets rivetkit's stop flag synchronously, before its retry loop checks
it. The supervisor then dials again with a fresh client of its own.

## Decisions and deviations

1. **The supervisor tests use a scaled real clock, not `tokio::time::pause`.** The workspace's
   tokio has only `full`, with no `test-util`, and `Cargo.toml` is frozen. So every supervisor
   duration goes through a private `Timing` seam: `Timing::REAL` holds the contract's numbers,
   and the tests run the same code on a copy scaled down.
   - The test values are: first 40 ms, cap 160 ms, give up 1.5 s, close 150 ms, rotation floor
     100 ms.
   - The tests assert backoff gaps of the base ±20 %, plus 60 ms of slack.
   - All 15 `table.rs` tests take about 1.5 s. Six repeat runs in a row were green.
2. **Rotation floor of 30 s (`ROTATE_FLOOR`).** A rotation never starts sooner than 30 s after
   the last one, or after the open. Without the floor, a desktop clock far from the server's
   would see every session as stale and spin the rotation. A reconnect still mints at once when
   the session is stale.
3. **`invalid_token` also triggers a fresh mint.** It does so as well as `forbidden`, because that
   is what an expired actor token answers. The player still gets `Invalid("invalid_token")` for
   that one call; the next call uses the fresh session.
4. **`Api::with_timeouts(connect, request)`.** This is an additive public method, like
   `with_create_retry`. The transport test uses it so a timeout takes 300 ms, not 10 s.
5. **`recent_tables`.** An empty list for non-empty ids, or a 401, triggers one recovery and one
   more ask. If the list is still empty, the result is `RecentTables::default()`. The server
   answers one row per asked id once the bearer verifies, so an empty list only means "not
   verified".
6. **`open_table` with a non-game id** answers `NotFound` with no call. This stops a
   path-shaped id from ever reaching the HTTP layer.
7. **`end_tables(err)`** ends the tables open now. It does not refuse later opens, because N2
   builds a new backend for each boot.

## Tests

**Unit tests (27, in the N1 files).**

| File | Tests |
|---|---|
| `http.rs` | `status_error_maps_every_status_per_call`, `wire_bodies_match_the_handlers`, `urls_encode_segments_and_keep_the_origin` |
| `device.rs` | `missing_corrupt_foreign_or_expired_record_loads_none`, `save_is_atomic_and_private` |
| `recent.rs` | `remember_moves_to_front_dedupes_and_caps`, `bad_file_loads_empty`, `invalid_ids_are_ignored` |
| `session.rs` | `plan_refreshes_at_eighty_percent_and_enrolls_when_expired`, `creds_debug_hides_the_bearer` |
| `remote.rs` | `remote_config_normalizes_and_refuses`, `unconfigured_is_not_wired_and_new_does_no_io` |
| `table.rs`, error mapping | `illegal_move_becomes_a_rejected_ack`, `forbidden_and_other_errors_map`, `a_closed_socket_is_closed` |
| `table.rs`, the contract's 7 supervisor tests | `drop_sends_status_false_then_reconnects_with_backoff`, `stale_session_is_minted_again_before_a_dial`, `rotation_swaps_without_a_status_edge`, `forbidden_mint_ends_the_stream`, `gives_up_after_two_minutes_with_the_last_error` (on the scaled clock), `events_before_status_true_are_buffered_in_order`, `actions_fail_fast_while_down` |
| `table.rs`, 5 more supervisor tests | `rotation_lets_the_old_sockets_calls_finish`, `not_found_and_unauthorized_mints_end_the_stream_too`, `interrupt_and_end_drive_the_same_paths`, `a_dropped_stream_stops_the_supervisor`, `a_forbidden_action_rotates_the_session_now` |

**`tests/remote_http.rs` (14).** The contract's 13, plus one more:

- `every_call_sends_its_method_path_headers_and_body`
- `first_call_enrolls_and_saves_the_device`
- `refreshes_at_eighty_percent`: at 79 % nothing happens, at 81 % it refreshes, and a 503 on
  refresh keeps the session.
- `reenrolls_when_refresh_is_refused`
- `a_401_recovers_once_then_reports_unauthorized`: three cases.
- `identity_rechecks_an_unverified_session` (the extra test)
- `concurrent_calls_share_one_enroll`
- `create_retries_503_then_succeeds`
- `create_gives_up_after_its_window`: also checks that 429, 400 and 500 are not retried.
- `table_session_statuses_map`: also checks, through the backend, that a 403 open is
  `Forbidden`, that a session with no gateway token is `Unavailable` with no dial, that a
  non-game id makes no call, and that a failed open remembers nothing.
- `recent_tables_skips_the_call_when_empty_and_keeps_order`
- `recent_tables_recovers_an_unverified_session`
- `transport_failures_map_to_offline_and_timeout`
- `redirects_are_not_followed`

**`tests/remote_live.rs`.** One test, `remote_backend_plays_against_a_real_server`. It starts the
server with `euchre_server::start(ServerConfig::try_local(root/server)?.with_engine_port(P))`,
`fast_tempo`, and offline AI. The data root is `$EUCHRE_TEST_TMP/n1-<pid>`. The test runs all 9
contract steps:

1. Enroll. `device.json` is 0600 and its dir 0700. A second `identity()` leaves the file
   byte-for-byte unchanged.
2. Create a table, then open it: `Status(true)`, then a `sync`.
3. Play `top_move` and get `MoveAck::Ok`.
4. Submit a made-up move on the next turn and get `Rejected { illegal_move, legal }`, where
   `legal` equals the turn's legal set.
5. `interrupt_tables()` gives `Status(false)`, then `Status(true)`. The snapshot `v` is not behind.
6. Another device's open is `Forbidden`.
7. `recent_tables()` lists the table as Active, with a hand number.
8. `list_games`, `stats` and `get_settings` answer, and `patch_settings` round-trips
   `housePrompt`.
9. `close()` ends the stream with no `Ended`, and a later call answers `Closed`. Shutdown
   completes, and P, P+1, P+5 and the HTTP port are closed within 5 s.

Run it with:

```text
EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6741 EUCHRE_TEST_TMP=<scratch> NO_PROXY=127.0.0.1,localhost,::1 \
  no_proxy=127.0.0.1,localhost,::1 cargo test -p euchre-desktop --test remote_live -- --nocapture
```

## Timings (3 green live runs, debug build, loopback, ms)

| | run 1 | run 2 | run 3 |
|---|---|---|---|
| server boot | 4980.5 | 5291.1 | 4983.8 |
| enroll (first `identity()`, incl. `/api/session`) | 10.5 | 9.1 | 12.0 |
| `identity()` again | 0.2 | 0.2 | 0.1 |
| create table | 21.5 | 22.4 | 22.5 |
| open (mint + metadata + dial to `Connected`) | 184.1 | 212.9 | 192.8 |
| first sync (from the open's start) | 221.5 | 212.9 | 192.9 |
| move ack | 79.4 | 14.2 | 36.8 |
| rejected-move ack | 5.1 | 0.6 | 5.4 |
| reconnect (interrupt → `Status(true)`, includes the 250 ms ±20 % backoff) | 378.7 | 323.6 | 319.1 |
| another device's open refused | 11.0 | 10.2 | 11.6 |
| recent tables | 0.9 | 0.9 | 1.0 |
| close | 0.1 | 0.1 | 0.1 |
| server shutdown | 3074.4 | 3133.2 | 3158.9 |

Each run took about 9.5 s in all. Before and after the runs, nothing listened on 6741, 6742 or
6746. No engine of N1's was left running.

## Notes for N2 and the integrator

- **Constructing the backend.** `RemoteBackend::new` does no IO, and its clones share one state,
  so `Arc<RemoteBackend>` is fine. Connection refused is `Offline`, which §N2.3 maps to
  `ServerStopped`. `end_tables(NetError::ServerStopped)` sends `Ended` on every open table and
  stops them.
- **Writing a live test.** After a `MoveAck::Ok { v }`, wait for a view with `v` at least the ack's
  before deciding it is the player's turn again. The cached view still shows the turn the player
  just took, so a move sent against it is refused with `illegal_move` and `legal: []`. The first
  live run failed this way, and `remote_live.rs` now waits.
- **fmt.** The N1 files are rustfmt-clean. When I last ran it, crate-wide
  `cargo fmt -p euchre-desktop --check` still reported diffs in N2's files: `net/embedded.rs`,
  `net/mod.rs`, `net/sim.rs`, `scenes.rs`, `screens/other.rs`, `screens/server.rs`,
  `startup.rs` and `tests/server_states.rs`. I did not touch them.
- **Server log noise.** In one run the server logged `aiDecision timed out after 5s` under offline
  AI and fast tempo. It is server-side, and the table played on.
- **Server gaps.** None are new. The two requests already written (`desktop-net-http-types.md`,
  `desktop-net-missing-table.md`) cover what the desktop works around.
