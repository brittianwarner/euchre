# Desktop net wave: integration

Lanes N1 (`RemoteBackend`) and N2 (`EmbeddedBackend` and the app) now work end to end. A plain
launch boots the embedded server while Home is already live. One full match plays to game over
through the real UI:

- the bridge thread;
- the `TableStore`;
- `RemoteBackend` over HTTP and the `/rivet` gateway;
- the embedded server on a real engine.

Your games then lists the match and its saved table, and the match detail opens to its hands.
Quit stops the engine and frees every port.

The lane reports are `desktop-net-remote.md` (N1) and `desktop-net-embedded.md` (N2). The
contract is `desktop-net-contract.md`, and §3 there is this agent's brief.

## Gates

Run on the final code, with `CARGO_TARGET_DIR=.cargo-targets/net-int`:

| Gate | Result |
|---|---|
| `cargo fmt -p euchre-desktop --check` | ok |
| `cargo clippy -p euchre-desktop --all-targets -- -D warnings` | ok |
| `cargo test -p euchre-desktop` | 238 passed, 0 failed, engine-free without `EUCHRE_E2E` (the 4 live tests print "skipped"); `tests/motion.rs` frame budgets 25/25 |
| `EUCHRE_E2E=1 … --test live` (port 6870) | **3 of 3 green in a row on the final code** (runs G, H, I below) |
| `EUCHRE_E2E=1 … --test embedded_live` (port 6870) | green: boot 5485 ms, death noticed in 3353 ms, Restart ready in 13926 ms, shutdown 3166 ms (+17 ms until every port closed), port in use reported in 172 ms |
| `EUCHRE_E2E=1 … --test remote_live` (port 6870) | green: boot 5371 ms, enrol 13 ms, create 22 ms, open and first sync 169 ms, move ack 44 ms, rejected ack 5 ms, reconnect 345 ms, shutdown 3124 ms |

## What the live test proves (`tests/live.rs`)

It is gated by `EUCHRE_E2E=1`, has one test, `live_match_through_the_app`, and uses port 6870 by
default. Its root is `$EUCHRE_TEST_TMP/integration-<pid>` and is kept. The human seat plays
`euchre_ai::top_move` through `AppState`'s own input methods, never through an `Intent`. In order,
the test proves:

1. **Boot.** A table opened while the server boots shows "A seat at the table." and then deals.
   The device enrols.
2. **Illegal move refused by the server.** At the player's first trick-play turn, a second
   client of the same device sends a made-up move id. The server answers `illegal_move` with the
   turn's legal set, and the app's table does not move (same `v`, same turn, controls on).
3. **Review gate.** With review on, the next finished trick holds for 2 s. Continue moves it on,
   then review goes off at the server.
4. **Reconnect mid-hand.** In hand 1, `interrupt_tables()` drops the socket. "Reconnecting…"
   shows and the controls go off. The same turn comes back.
5. **Engine stop and Restart.** In hand 2 the engine gets SIGTERM. The table ends with "The game
   server stopped. Restart to keep playing." under the felt, and the last view stays. Restart
   brings a new engine and the same turn.
6. **Game over.** The status is complete, someone reached 10, the winner matches the score, the
   match-end panel shows and every move acked Ok.
7. **Your games.** The match is listed and counted in the stats. The saved table reads "Open game
   · Hand N · a–b" at the final score, and the match detail lists its hands.
8. **Quit.** The engine pid is gone and every port is closed.

It writes `timings.json` and the real syncs as `scenes::LiveFixture` files. It then renders
them headless with the app binary (`CARGO_BIN_EXE_euchre --scene live-*`) at 1280x800, scale 1.

## Timings

Apple silicon, debug build. Each run is one full match against a real engine (`fast_tempo`,
offline AI).

| Run (root) | Wall | Moves | Move ack p50 / p95 / max | Boot cold | Create → first turn | Reconnect | Stop noticed | Restart → ready / table back | Games / detail | Quit | Ports closed after quit |
|---|---|---|---|---|---|---|---|---|---|---|---|
| G (`integration-25381`) | 96.7 s | 82 | 32 / 61 / 163 ms | 5540 ms | 5763 ms | 453 ms | 4004 ms | 10631 / 10842 ms | 7 / 12 ms | 3171 ms | +5 ms |
| H (`integration-28968`) | 89.1 s | 76 | 31 / 54 / 72 ms | 4910 ms | 5274 ms | 379 ms | 3567 ms | 13648 / 13798 ms | 7 / 11 ms | 3178 ms | +4 ms |
| I (`integration-30421`) | 86.0 s | 64 | 31 / 41 / 163 ms | 5592 ms | 6008 ms | 419 ms | 3178 ms | 13572 / 15712 ms | 7 / 7 ms | 3886 ms | +11 ms |
| A (`integration-74829`) | 95.3 s | 86 | 32 / 61 / 111 ms | 4976 ms | 5251 ms | 382 ms | 3426 ms | 13632 / 13790 ms | 33 / 36 ms | 3213 ms | +3 ms |
| B (`integration-77263`) | 97.5 s | 75 | 34 / 77 / 90 ms | 5038 ms | 5330 ms | 388 ms | 3228 ms | 13665 / 13862 ms | 7 / 11 ms | 3159 ms | +7 ms |
| C (`integration-83158`) | 119.9 s | 98 | 42 / 101 / 175 ms | 9029 ms | 9505 ms | 461 ms | 3503 ms | 14303 / 14443 ms | 8 / 12 ms | 3164 ms | +7 ms |

More detail:

- **Move timings.** "Move ack" runs from submit to `Ack`. The move is on screen ("landed") 1–2 ms
  after its ack.
- **Refused move.** The server answers the made-up move in under 1 ms.
- **Review gate.** The gate held for 2000–2004 ms.
- **Runs A–C** used the same app code minus the saved-row fix, and an earlier test (refused move
  in hand 0 only). Run C's cold boot was slower because the machine was busy.
- **Restart → ready** is 13.6 s in most runs. It is 5 s of the server's registry grace against a
  dead engine, plus a boot that waits out the engine's 7 s lost threshold for the old runner.
  Both are server-side; see the requests below.

### Runs that failed, and why

| Run | Failure | Cause | Fix |
|---|---|---|---|
| final3, E, F | The finished match's saved table read "Resume game · Hand 3 · 4–1" | The app's cached `RecentTables` answer from the Restart moment, shown until the fresh read landed. In run F a fresh read was right at once. Run E's data dir, reopened in a new process, read `complete` 10–3. | `AppState::note_open_table` keeps the open table's row in step with the live view; test `the_saved_row_follows_the_table_on_screen` |
| D | "the made-up move was tried" never happened | Hand 0 gave the player no trick-play turn: it was thrown in, or the player sat out a partner's loner | The refused move now runs on the player's first trick-play turn in any hand. The test waits for review-on to reach the server before moving on. |
| final3 | The old server's engine client re-registered with the restarted engine (`reconnected after the envoy lost threshold…`) | rivet-envoy-client's detached connection loop survives the registry abort | Each embedded server runs on its own tokio runtime, dropped at stop. No connection attempt is logged after the old server stops in any run since. |

## Real window (release build, embedded server)

The release build took 3m31s. Each launch used fresh temp dirs: data and prefs under
`…/scratchpad/net/int/window-1/`.

| Launch | Spawn → ready (`boot_ms`) | Checks | Quit (SIGTERM to my pid) |
|---|---|---|---|
| Cold (empty data dir, engine cached) | 6592 ms (5402) | `device enrolled user_id=guest-…` in `<data>/logs/euchre.log`; `/health` 200 `ok`; engine on 60687, 60688, 60692; HTTP on 60689 | App exits in 3220 ms, engine in 3250 ms; nothing listening |
| Warm (same data dir) | 4756 ms (4612) | `/health` 200; the same engine ports (recorded and reused); a new HTTP port, 61316 | App exits in 3207 ms, engine in 3234 ms; nothing listening |

The log shows "quit signal: stopping the game server": the bridge handles SIGTERM. A first warm
attempt was quit while the server was still booting. That cancelled cleanly, and its engine exited
too.

## Gallery

`/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/net/gallery/`
(1280x800, scale 1; `overview.png` is the contact sheet).

| File | Shows |
|---|---|
| `server-starting.png` | the boot line: "A seat at the table. Getting the table ready…" |
| `live-bid.png` | the player's bid: up card, Pass / Make clubs trump / Go alone |
| `live-play.png` | trick play with the "Must follow suit" hints |
| `live-trick-won.png` | "Your partner wins the trick." with the trick gathered |
| `live-trick-review.png` | the review gate: Continue on the trick |
| `live-hand-result.png` | the hand-result card ("Euchre! … +2") |
| `live-game-over.png` | the match-end panel: "A good game. Well played." at 8–10 with Play again |
| `live-games.png` | Your games: the saved tables band, the stats, the match row |
| `live-match.png` | match detail: totals and per-hand rows |
| `live-stopped.png` | a table under "The game server stopped. Restart to keep playing." |
| `server-stopped.png` | Home with the stopped notice and Restart |

The live PNGs are from run C. I opened each one and checked it. Every PNG of every run is also
kept in its run root under `snaps/`.

## Fixes made in integration

All of these are in `apps/desktop/**`.

- **`net/embedded.rs`:**
  - Each server runs on a `ServerRuntime` of its own, shut down with `shutdown_background` once
    its stop returns. This kills the old engine client.
  - `abort_bounded` handles the start and stop that run past their budget.
  - The `embedded_events` forwarder closes when its receiver goes.
- **`app.rs`:**
  - `note_open_table`: the saved-tables row of the open table follows the live view.
  - Layout settle (`layout_settle`, `note_drawn_size`). Rects placed from layout, such as the
    turn ring, read the previous commit. Two more commits now follow a new view or a resize, so
    nothing is left where an earlier pass put it. Before, a fresh table at 1280x800 drew the ring
    beside the wrong seat.
- **`screens/table/mod.rs`:** the "server stopped" and "Reconnecting…" banners sit at the foot of
  the table, over the controls they turn off. Before, they covered the partner's name and the
  hand number.
- **`screens/other.rs`:** the match page falls back to the per-hand trick counts when
  `stats.tricks` reads 0–0 (`profile-tricks-won.md`).
- **`scenes.rs`:**
  - `LiveFixture` carries syncs and stats.
  - The new `live-stopped` scene.
  - `live-games` shows the stats.
- **`tests/live.rs`** (new): the test described above.
- **`tests/server_states.rs`:** `the_saved_row_follows_the_table_on_screen`.
- **`README.md`** (new): Modes (embedded, remote, sim), the data dir, engine binary resolution,
  `NO_PROXY`, Quit, and how to run the live tests.

## Requests to the server (W4)

| File | Ask |
|---|---|
| `docs/port/requests/desktop-net-stale-registry.md` (new) | Stop the engine client with the registry. Retry `recordHand` and the aiSeat dispatch on `Actor queue delivery interrupted`. Shorten the Restart (lost threshold). Add an engine-crash case to the server e2e. |
| `docs/port/requests/desktop-net-dead-engine-grace.md` | 500 ms registry grace once the engine is gone, instead of 5 s |
| `docs/port/requests/desktop-net-missing-table.md` | 404 for a table that does not exist, instead of 503 |
| `docs/port/requests/desktop-net-http-types.md` | shared types for the `json!` bodies |
| `docs/port/requests/profile-tricks-won.md` | `stats.tricks` always reads 0–0 (worked around on the match page) |

## Open items

1. **After a Restart, a hand can go missing from the match history** (6 of 9 runs, always the
   hand in play when the engine died):
   `recordHand failed; hand journal entry dropped … Actor queue delivery interrupted`. The match
   result itself is recorded. Server-side; see the request.
2. **After a Restart, the AI seats play the heuristic for the rest of that hand** (every run).
   Server-side.
3. **Restart takes about 13.6 s** (5 s grace plus about 8.5 s boot). It should be about 5 s once
   both server requests land.
4. **The engine takes about 3.5 s to show as stopped** after SIGTERM: the server's 1 s poll plus
   the engine's own exit. Meanwhile the table reads "Reconnecting…".
5. **No engine is bundled yet.** A machine without rivetkit's cache downloads 58.7 MB on first
   launch. The bundle path (`<exe dir>/rivet-engine`, `../Resources/rivet-engine`) is wired and
   tested, but packaging is not done.
6. **`kill -9` of the app leaves the engine running** until the next launch reclaims it. Not
   exercised live here.
7. **At 720×520 the Home "server stopped" notice covers the heading.** It is fine at 1280×800.
   This is a layout follow-up for the shell.
8. **rune warnings in the live logs** (motion lane):
   - `on_hover_change` at `screens/table/hand.rs:506` commits the whole view for each rack card;
     it should use `El::hover` / `hover_group`.
   - `duplicate matched id "hcard-<seat>-<card>"`: two elements share one matched id, and rune
     matches the last one.

   Neither one failed a test or a frame budget.

## Review fixes

Every finding was checked against the code. None was refuted. Two are partly server-side
(PP-1, PX-2), and PX-5's remainder is too: the desktop side is done, and the rest is in a request
file. PX-9 is the same finding as LDS-3. All changes are in `apps/desktop/**`, plus the contract
(D33) and the request files.

| Id | Verdict | What changed | Test |
|---|---|---|---|
| PP-1 (P2) | Confirmed; server half requested | A 404 table-session mint ends with `NotFound` and drops the id from `recent-tables.json` (`RecentStore::forget`). A 403 never drops it. The server still answers 503 for an unknown table: `desktop-net-missing-table.md` (review update). | `remote_http::a_gone_table_is_forgotten_and_a_signed_out_device_says_so`, `recent::forget_drops_one_id_and_keeps_the_order` |
| PP-2 (P2) | Confirmed | `remote_live` checks more on a real server: a blank persona (`Invalid("invalid_persona")`, settings unchanged) and a stale turn id (`Rejected stale_turn`). It plays the match to the end, then checks a move after the end (`Rejected game_complete`), exactly 1 scrapbook row with `stats.matches == 1`, `match_detail` decoding with hands, and a random match id (`NotFound`). | `remote_live::remote_backend_plays_against_a_real_server` (E2E, green 3 times) |
| PP-3 (P3) | Confirmed | A rotation mint refused for good (403, 404, 401, signed out) is not retried. The socket stays, and the stream ends with that error at its next drop (`final_err`, `is_final`). | `table::a_refused_rotation_keeps_the_socket_then_ends_at_the_drop` |
| PP-4 (P3) | Confirmed | D33 supersedes D17: no timed rotation. The only rotation is after a `forbidden` action, at most once per `ROTATE_FLOOR`. A redelivered `chat` (same `msgId`) neither joins the log again nor pops the bubble again. The contract is updated. | `table::nothing_rotates_without_a_refused_action`, `table::a_forbidden_action_rotates_the_session_now`, `server_states::a_redelivered_chat_line_shows_once` (fails with the guard removed) |
| PP-5 (P3) | Confirmed | The Games pager sends one page at a time. `Reply::Games` carries its cursor, and only the page asked for lands. | `server_states::the_games_pager_waits_for_its_page` |
| PP-6 (P3) | Confirmed | After a create `Timeout`, Reconnect does nothing by itself, and the table offers New game. | `server_states::a_failed_create_says_so_and_never_creates_twice_by_itself` |
| PP-7 (P3) | Confirmed | A silent re-enrol under a new user writes `device-reset.json` (0600) with the previous user id. A later 403 on an older table reads "This device was signed out, so older tables can’t be opened." with New game. | `remote_http::a_gone_table_is_forgotten_and_a_signed_out_device_says_so` (marker kept across relaunch; expired record too), `server_states::ended_keeps_the_last_view_and_says_why` |
| LDS-1 (P2) | Confirmed | A watchdog shell waits on a pipe from the app. When the app dies, even by SIGKILL, the shell stops the engine, but only if its command line still names this storage's `--config`. | `local::the_watchdog_stops_its_engine_when_the_pipe_closes`; E2E `embedded_live::an_engine_stops_when_its_app_is_killed`: engine gone, ports free 3050–3172 ms after SIGKILL (4 runs) |
| LDS-2 (P2) | Confirmed | On relaunch, the app waits up to `INSTANCE_WAIT` (20 s) while the recorded owner is alive, then fails with `AlreadyOpen`. | `local::a_launch_waits_for_the_copy_that_is_quitting`, `embedded::a_second_copy_waits_for_the_first_then_says_it_is_open` |
| LDS-3 / PX-9 (P2) | Confirmed | `AlreadyOpen` ("Euchre is already open.") is now distinct from `PortInUse` ("Euchre can’t start its game server."). A first run takes a block in 20000–29990, step 10, with P, P+1 and P+5 all free, and records it. | `local::first_ports_stay_in_range_and_skip_busy_blocks`, `local::a_recorded_port_is_kept`, `server_states::failed_copy_is_plain_for_every_problem` |
| LDS-4 (P2) | Confirmed | The data and server dirs are 0700. Prefs and device saves go through `startup::create_private_dir`. | `startup::data_and_server_dirs_end_up_private`, `prefs::save_round_trips_with_owner_only_mode` (nested dir 0700) |
| LDS-5 (P2) | Confirmed | Each port block gets a stable root (`integration-<port>`, `n2-<port>`, `n2-owner-<port>`). A run first reclaims an orphan engine whose command line names that root's storage. Tests run with `quit_on_signals` off. A passing `live.rs` removes its root unless `EUCHRE_LIVE_KEEP=1`. | E2E: every passing run left no `integration-*` or `n2-*` root behind |
| LDS-6 (P3) | Confirmed | `flock` on `<data>/euchre.lock`, held for the server's life. | `local::the_lock_is_one_holder_at_a_time` |
| LDS-7 (P3) | Confirmed | The test `Drop`s and the mid-match kill check `local::is_engine_of(pid, storage)` (the command line) before any signal. | `live.rs`, `embedded_live.rs` (asserted before each kill) |
| LDS-8 (P3) | Confirmed | A corrupt `device.json` or `recent-tables.json` moves to `<name>.corrupt-<ms>` (0600). At most 2 copies are kept. The log names only the kind (`io:<kind>` / `json:<category>`). | `recent::a_corrupt_file_is_set_aside_and_only_two_copies_are_kept`, `device::missing_corrupt_foreign_or_expired_record_loads_none` |
| LDS-9 (P3) | Confirmed | Plain `http://` only for loopback, unless `EUCHRE_ALLOW_INSECURE=1`. A `clientEndpoint` on another origin is replaced by `<base>/rivet`, with a warning. | `remote::remote_config_normalizes_and_refuses`, `remote::a_gateway_on_another_origin_is_not_dialed` |
| LDS-10 (P3) | Confirmed | The envoy client's connection log is dropped unless its link is expected. A normal quit logs no ERROR line. | `startup::engine_client_logs_only_while_its_link_is_expected`; the live logs show no ERROR at quit |
| LDS-11 (P3) | Confirmed | SIGHUP quits like SIGINT and SIGTERM (a stop of the game server, then exit). | None automated (process signal) |
| PX-1 (P1) | Confirmed | `net/fetch.rs` is our own engine download. It streams to `.part` with a connect timeout and a per-read idle timeout, and no total cap. It resumes with `Range`, checks SHA-256, then renames. The new `DownloadingSlow` phase and the `DownloadStalled` copy ("The first-start download stopped. Check your connection, then try again. It picks up where it left off.") cover slow and stalled downloads. | `fetch::a_slow_cut_download_resumes_and_finishes`, `fetch::a_stalled_download_gives_up_and_nothing_reached_is_unreachable`, `embedded::a_slow_download_says_so_then_starts` |
| PX-2 (P1) | Confirmed; server-side | `recordHand` is dropped in `crates/euchre-table` (W4). `live.rs` now fails on any missing hand except the one the engine stopped in. 2 of 4 runs lost hand 2; 2 lost none. See `desktop-net-stale-registry.md` (review update). | `live::live_match_through_the_app` |
| PX-3 (P2) | Confirmed | A bounded 10 s wait for the match-end panel, recorded as `match_end_panel_ms` (43–82 ms). | `live::live_match_through_the_app` |
| PX-4 (P2) | Confirmed | Restart keeps the TableScreen and its last view, so `display_view` stays `Some`, with no loading screen. A failed-create screen is used only with no game id and no view. | `server_states::ready_after_stopped_reconnects_the_open_table_and_reloads_the_route` |
| PX-5 (P2) | Confirmed; the rest is server-side | A server whose engine died is dropped on its own runtime instead of shut down (`clear_dead`, `DEAD_STOP`). The stop takes 0–2 ms, down from 5001. Restart to ready is 9.3–10.3 s, down from 13.6–14.3. The remaining boot after a crash (9.3–10.3 s, against 4.5 s after a clean stop) is the engine waiting about one stop-timer period for the dead runner: `desktop-net-stale-registry.md`. | `embedded::closed_stops_and_ends_tables` (drop, never `shutdown`) |
| PX-6 (P2) | Confirmed | A failed create reads "We couldn’t start a game. Try again." with Try again, and never "Your game is saved." | `server_states::a_failed_create_says_so_and_never_creates_twice_by_itself` |
| PX-7 (P3) | Confirmed | The notice sits beside the nav when it fits. Otherwise it sits at the window's foot, or under the header on Home. | `server_states::the_notice_never_covers_the_heading`: Home, Games and Settings × Stopped and PortInUse × 720×520 and 1024×700 |
| PX-8 (P3) | Confirmed | While the table is Failed, the hand reads "Paused." and "Play picks up where it left off.", and the rack dims. The felt's line goes quiet, so the sentence shows once. | `server_states::ended_keeps_the_last_view_and_says_why` |

### Gates after the review fixes

Run on the final code, with `CARGO_TARGET_DIR=.cargo-targets/net-fix`, port block 6910–6919:

| Gate | Result |
|---|---|
| `cargo fmt -p euchre-desktop --check` | ok |
| `cargo clippy -p euchre-desktop --all-targets -- -D warnings` | ok |
| `cargo test -p euchre-desktop` | 264 passed, 0 failed, 1 ignored (`engine_owner_child`, run only by its parent test). Engine-free without `EUCHRE_E2E`. `tests/motion.rs` 25/25. |
| `EUCHRE_E2E=1 … --test embedded_live` | Green twice on the final code (rounds 4 and 5; also green in rounds 2 and 3). Boot 5552 and 5499 ms. Death noticed in 3473 and 4032 ms. Restart ready in 9844 and 9349 ms. Owner SIGKILL to engine gone in 3050 and 3137 ms. Port in use reported in 107 and 88 ms. |
| `EUCHRE_E2E=1 … --test remote_live` | Green twice on the final code (also green in rounds 1 and 3). Boot 4738 and 4896 ms. Move ack 35 and 59 ms. Stale ack 13 and 4 ms. Reconnect 324 and 393 ms. Match to the end 29.2 and 36.8 s. After-end ack 16 and 13 ms. Shutdown 3137 and 3148 ms. |
| `EUCHRE_E2E=1 … --test live` | Green twice on the final code (also green in rounds 2 and 3). Wall time 101.9 and 95.4 s. Move ack p50 31 ms, p95 43 and 39 ms. Restart ready in 9342 and 10344 ms. Match-end panel up after 43 and 60 ms. Quit in 3158 and 3164 ms. `missing_hands` `[]` and `[2]`. |

### Gallery after the review fixes

These files were re-rendered into the gallery directory above. The `live-*.png` files come from
round 4 of `live.rs`, run with `EUCHRE_LIVE_KEEP=1`. I looked at each of them.

- `server-stopped-table.png` and `live-stopped.png`: "Paused." over the dimmed rack. The felt line
  is quiet.
- `server-stopped-games-720x520.png`: the notice sits at the foot, clear of "Your games".
- `server-failed-port.png` and `server-failed-port-720x520.png`: "Euchre can’t start its game
  server." The notice sits beside the nav at 1280 and under the header at 720, clear of the H1.
- `server-relaunch-1024x700.png`, `server-downloading.png`, `server-failed.png`,
  `server-stopped-games.png` and `games-saved-tables.png`.

### Open items, updated

- Items 3, 6 and 7 above are fixed: Restart is 9.3–10.3 s, `kill -9` takes the engine with it,
  and the notice clears the H1.
- Item 1 is PX-2 and stays server-side.
- Item 2 still holds. The AI seats fall back for the rest of the hand after a Restart.
- At 720×520 the Home notice covers the hero's eyebrow line ("The table is yours"), but not the H1.
