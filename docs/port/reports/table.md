# Report: SERVER-TABLE (final)

Crate: `crates/euchre-table`, the `euchreTable` actor on rivetkit 3.0.7. Owner lane: SERVER-TABLE.

- Stages: TABLE-CORE (the actor, its state, timers, auth and the heuristic seat) and TABLE-AI-PROFILE (real `aiSeat` dispatch and `playerProfile` recording). Stage detail is in `docs/port/reports/table-core.md`.
- TS source of truth (read-only): `src/lib/actors/euchre-table/{index,queues,tempo,send-bounded}.ts`, `decision-delivery.test.ts`, `review.test.ts`, `send-bounded.test.ts`.
- Spec: `docs/port/03-table-actor.md` §8.4–§8.6, §10, §11, §13, §14, §16. Decisions D6–D10, D17–D24, D27, D29.

No other crate changed, and neither did the root `Cargo.toml`. The crate gained one dev-dependency, `euchre-profile-model.workspace = true`, so tests can run the profile's own body parsers.

Change requests filed by this lane:

- `docs/port/requests/server-table-profile-bodies.md`: `euchre-protocol` should own the `recordGame` / `recordHands` message bodies.
- `docs/port/requests/server-table-ai-seat-callback-retry.md`: the aiSeat port should retry `aiDecision` / `aiSay`, because an action to a table in its sleep grace fails with `stopping`, where the TS queue send was durable.

Requests honoured: `server-ai-seat-table-params.md` (`{ internalToken }` params on every aiSeat call) and `server-ai-seat-dispatch-order.md` (ordered per-seat delivery).

## Gates

Run with `CARGO_TARGET_DIR=/Users/brittianwarner/goods/euchre-rune/.cargo-targets/table`.

| Gate | Result |
| --- | --- |
| `cargo fmt -p euchre-table -- --check` | clean |
| `cargo clippy -p euchre-table --all-targets -- -D warnings` | clean |
| `cargo test -p euchre-table` | 67 unit tests passed. The 8 e2e tests (4 + 4) print `SKIP` without `EUCHRE_E2E`. 2 doc tests are ignored. |

Code rules: no `unwrap`, `expect`, `todo!()` or `unsafe` in library paths; doc comments on every public item; no token in any log line (error logs carry a 200-unit excerpt); user-facing strings and log event names copied from TS (D27).

## Engine e2e

Both suites start their engine only through `euchre_server_core::testing::TestEngine`, on port 6711 with storage `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/rivet-store-table`. Port 6711 was free before every run. After every run, `lsof -iTCP:6711 -sTCP:LISTEN` showed no listener.

```text
EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6711 \
EUCHRE_TEST_STORAGE=/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/rivet-store-table \
perl -e 'alarm 500; exec @ARGV' cargo test -p euchre-table --test table_e2e -- --nocapture

EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6711 \
EUCHRE_TEST_STORAGE=/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/rivet-store-table \
perl -e 'alarm 500; exec @ARGV' cargo test -p euchre-table --test table_ai_e2e -- --nocapture
```

| Suite | Result |
| --- | --- |
| `table_e2e` (TABLE-CORE: auth, moves, match, review + sleep) | `4 passed; 0 failed`, about 52 s |
| `table_ai_e2e` (TABLE-AI-PROFILE) | `4 passed; 0 failed`, about 100–137 s per run (see the run history below) |

### Run history of `table_ai_e2e`

The full suite was run 18 times while it was hardened, plus 3 runs of the sleep test alone. The failures found the problems below, each fixed in the test's stand-ins (none needed a table change):

- **`chatDelta` text.** The table screens a delta as TS does, so `"Good "` arrives as `"Good"`. The test expected the raw text.
- **`chat` key.** The wire key is `final`, not `isFinal` (the TABLE-CORE event table had it wrong; corrected here).
- **`thinking` counts.** TS sends `off` after forced moves and on the watchdog, so `off >= on`, not `off == on`.
- **Late callbacks from an earlier test.** A late answer from one test can land after its engine is gone, so callback errors are now filtered per match.
- **A stand-in seat going to sleep during a call** failed that call with "Actor queue delivery interrupted", and the next lifecycle call overtook it. The table logged it and moved on (best effort, as TS). The stand-ins now stay awake for the whole test, so the order checks see only the table's order.
- **The sleep test.** An answer landing in the table's sleep grace failed with `stopping`, or hung while the grace lasted, and the table did not wake within 15 s. The stand-in now bounds each try at 2 s and retries 5 times (`requests/server-table-ai-seat-callback-retry.md`).

With the final stand-ins, 4 full-suite runs in a row all passed, except one sleep-test run that played `fallback` under the then-strict `llm` check, which is now accepted (see below). The last edit only relaxed that check and comments. A re-run after it was not possible: the machine's disk filled (1.6–2.9 GiB free), and the engine refuses to start without 4 GiB free for its log shard ("configured log shard capacity of 4294967296 bytes exceeds the ... bytes free on its filesystem"). The `table_e2e` suite last passed with the final library code.

### `table_ai_e2e`

The other actors are stand-ins defined in the test, registered as `aiSeat` and `playerProfile` with the real action names. The stand-in seat acks `decide` at once and answers later through the table's `aiDecision` / `aiSay` with params `{ internalToken }`, picking the `euchre-ai` heuristic top move with `source: "llm"`. The stand-in profile records every body and refuses a wrong token. The tests run one at a time, each on its own pool.

**`table_ai_full_match_end_to_end`** (a player against three stand-in seats, about 35–60 s)
- The match plays to `status: "complete"`: about 50–75 player moves and 150–220 AI turns.
- `recordGame` arrives exactly once. Its keys are `["internalToken", "record"]`, `euchre_profile_model::parse_record_game` accepts it, and `matchId`, `userId`, `status`, `handsPlayed`, `winnerTeam` and `outcome` match the table's state. The table then sets `recordedAt`, arms the reap (`ledger.reapAt`) and clears `ledger.profileRetryAt`.
- `recordHands` arrives once per finished hand, throw-ins included, all before `recordGame`. Keys are `["internalToken", "matchId", "hands"]`, one entry per call, hand numbers strictly increasing, and `parse_record_hands` accepts each. No move is `auto`. Nearly every AI move is `llm` (for example 111 llm, 0 fallback).
- Each seat: every call carries the match's `internalToken` as params. The create input keys start `gameId, seat, internalToken, fenceNonce, persona, dossier, matchId`, and `profileKey` is `["user", <owner>]`. With a 300 ms `handEnd` at the seat, the order is still `resetHand(N)`, then each `decide` of hand N, then `handEnd(N)`, then `resetHand(N+1)`, and `gameEnd` is last, exactly once, after `recordGame`.
- Chat: seat 2's `aiSay` delta arrives as `chatDelta` (`delta: "Good"`, screened as TS trims it) and its final line as `chat` (`kind: "banter"`, `msgId` ending `:talk:2`). Seat 1's `banter` on a decision arrives as `chat`. Every `chat` has `final: true`.
- `thinking`: every event has `extended: false` (D23). There is at least one `on`, and `off >= on` (TS also sends `off` after forced moves and on the watchdog).

**`table_ai_misbehaving_seats_end_to_end`** (5 matches at once; first 3 asked AI turns each)
- Late (2.3 s, past the 1.5 s fast-tempo cap), wrong `turnId`, never answers: each turn plays the parked heuristic move as `fallback`, at the cap (slowest turn about 1.54 s, under cap + 1.5 s).
- Answers twice: the first answer plays as `llm`; the second is dropped.
- Illegal move id: `fallback` with the first legal move, about 0.15–0.25 s (logs `aiSeat returned a move outside the legal set`).
- Every asked turn has one `decide` and exactly one move.

**`table_ai_sleep_between_dispatch_and_decision_end_to_end`**
- Only HTTP calls reach the table (no socket keeps it up). The test hook makes the table call `ctx.sleep()` right after its first `decide` is sent. While an AI seat is to play, the test stops calling the table.
- The table sleeps (`sleeps` goes up). The test then lets the seat answer. The table wakes (`starts` goes up), the seat's callback succeeds, and the asked turn plays exactly once with the asked move. The game then plays on over a socket.
- The source is `llm` when the answer and the wake beat the 1.5 s fast-tempo cap, which was most runs. Under load the wake took longer in one run, and the table played the same parked move as `fallback` at the deadline. Both are accepted.

**`table_ai_missing_seat_end_to_end`** (no `aiSeat` registered)
- Every `decide` and lifecycle call fails (`dispatch to aiSeat failed; heuristic stands`, `notifyAiSeats failed`). Every AI turn plays the heuristic `fallback` within the cap (slowest about 1.54 s). The game never stalls.

**Known log lines.** `ERROR rivetkit_core::actor::context: scheduled event execution failed error=stopping` appears when an engine stops an actor (sleep or `TestEngine::shutdown`) while a schedule is due. It is not a table failure: the ledger re-arms that timer on the next wake (D20).

## Actions

Registry name `euchreTable`, key `["table", gameId]`. Create input `TableCreateInput` (`null` means the defaults). Every action is a newtype over the positional argument array (G32). The caller is re-derived from `ctx.conn().params()` on every call (D17): **player** (guest JWT owner at seat 0, params `{ token, protocolVersion: 1 }`), **internal** (params `{ internalToken }`, the per-match token), **schedule** (no connection).

| Action | Arguments (JSON) | Returns | Allowed |
| --- | --- | --- | --- |
| `snapshot` | none | `PublicGameView`, redacted for seat 0 | player, internal |
| `submitMove` | `{ moveId, turnId?, clientMoveId }` | `{ ok: true, v }` or `{ ok: false, code, legal }` | player |
| `reviewTrick` | `{ action: "set", enabled }` or `{ action: "continue", turnId }` | `{ queued: true }` | player |
| `getHandReview` | `handNo` | `CompletedHandReview` or `null` | player |
| `lastTrick` | none | `Trick` or `null` | player, internal |
| `move` | `MoveMsg` `{ moveId, turnId?, clientMoveId, ... }` | `MoveAck` | player |
| `review` | `ReviewMsg` | `null` | internal, schedule |
| `aiDecision` | `{ internalToken, gameId, seat, turnId, moveId, source, banter?, rationale?, latencyMs }` | `null` | internal, schedule |
| `aiSay` | `{ internalToken, gameId, seat, turnId, handNo?, msgId, delta?, text?, final }` | `null` | internal, schedule |
| `tick` | `TickMsg` | `null` | internal, schedule |
| `onTempoGate` / `releaseAiMove` | `turnId` | `null` | schedule |
| `onAiTimeout` | `turnId, seat` | `null` | schedule |
| `onNudge` / `onAbandon` | `turnId` | `null` (legacy no-ops, D7) | schedule |
| `flushProfile` / `onReap` | none | `null` | schedule |

`aiDecision` runs the §8.4 checks in TS order and drops (returns `null`) on the first that fails: internal token (`dropped forged aiDecision`, code `internal_token_mismatch`); `gameId` and `status: "active"`; `turnId` is the current turn; the turn seat is an AI seat; `seat` matches it (`dropped aiDecision with a mismatched seat`, code `seat_mismatch`); a pending decision exists; it is not already answered or forced; past the deadline it releases the parked move (`releasePending(force)`); an id outside the legal set is logged (`aiSeat returned a move outside the legal set`) and the first legal move is taken instead, as `fallback`; then banter is screened and the answer is held behind the think floor (`revealAt`) or played at once.

`aiSay` (§8.6): token, `gameId`, AI seat, and the line must belong to this hand (`handNo`) or, without `handNo`, this turn. A `final` line is screened and sent as `chat` (`kind: "banter"`); a delta is screened and sent as `chatDelta`, and an empty delta is dropped.

## Events

| Event | Payload (JSON, key order as sent) | Delivery |
| --- | --- | --- |
| `sync` | `{ v, view: PublicGameView, steps }` | Per connection, redacted for its seat; `steps: []` on connect. Never to internal connections. |
| `thinking` | `{ seat, on, extended: false }` | Broadcast. `on` when a seat is asked; `off` after every AI move and on the watchdog. |
| `chat` | `{ msgId, seat, kind: "call" \| "banter" \| "system", text, final: true }` | Broadcast |
| `chatDelta` | `{ msgId, seat, delta }` | Broadcast |
| `presence` | `{ seat, online }` | Broadcast on 0→1 and 1→0 live player sockets |

## Calls to other actors

All calls go through the one shared `rivetkit::client::Client` (JSON encoding) in `TableConfig.client`, as stateless actions: `get_or_create(name, key, { params, create_with_input })` then `action(name, [body])`. They are by name only (D29). Each runs off the lane, on its target's ordered outbox, in a `register_task` holding `keep_awake_region` (G25), bounded by `send_bounded` against `ctx.abort_signal()`.

| Target | Key | Params | Action | Body (JSON) | Budget | When | On failure |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `aiSeat` | `["table", gameId, "seat", "<n>"]` | `{ internalToken }` | `decide` | `{ internalToken, gameId, seat, turnId, kind, view, legal, ranking?, deadlineAt }` | `max(1, deadlineAt - now)` ms | An asked AI turn opens (after persist) | warn `dispatch to aiSeat failed; heuristic stands` `{ seat, e }`; the parked move and watchdog play the turn |
| `aiSeat` | same | same | `resetHand` | `{ internalToken, gameId, kind: "resetHand", handNo }` | 1 s | A new hand is dealt | warn `notifyAiSeats failed` `{ seat, kind, e }` |
| `aiSeat` | same | same | `handEnd` | `{ internalToken, gameId, kind: "handEnd", handNo }` | 1 s | A hand is scored | same |
| `aiSeat` | same | same | `gameEnd` | `{ internalToken, gameId, kind: "gameEnd" }` | 1 s | After the match is recorded (or at once without a profile) | same |
| `playerProfile` | `["user", userId]` | none | `recordHands` | `{ internalToken, matchId, hands: [HandJournalEntry] }` | 1 s | Each finished hand, before the seats hear `handEnd` | warn `recordHand failed; hand journal entry dropped` `{ handNo, err }` |
| `playerProfile` | same | none | `recordGame` | `{ internalToken, record: MatchRecord }` | 1 s | Game over, once | warn `recordGame failed; will retry` `{ err }`; `flushProfile` sends it again after 30 s |

The `aiSeat` create input is the TS `aiSeatCreateInput`: `{ gameId, seat, internalToken, fenceNonce, persona, dossier, matchId, profileKey? }`, with `profileKey: ["user", userId]` only when the match has an owner. `recordGame` and `recordHands` carry the deployment internal token (`TableConfig.profile_token`); `decide` and the lifecycle calls carry the per-match token. Neither is logged.

**Order.** Each AI seat has its own outbox, and so does the profile. A seat therefore sees `handEnd(N)`, `resetHand(N+1)`, `decide(N+1)` in the order the table made them, and a slow seat delays only its own later calls. Outboxes live in memory. After a crash, `pending` plus the watchdog re-derive a decision, and the `flushProfile` ledger re-derives the match record (§13).

**Budgets (D24).** The table's turn deadline is `ai_hard_cap_ms` (4000 ms; 1500 ms in fast tempo). `extended` thinking starts at 2200 ms in TS but is never emitted (D23). The 1400 / 400 / 300 ms rungs belong to the aiSeat ladder. Whatever the seat does, the parked heuristic move plays at the cap.

## Deviations from TS

The 17 TABLE-CORE deviations stand (`table-core.md`). This stage adds:

1. **`recordGame` is sent off the lane and settled in a later lane turn** (§16.7). The record snapshot is saved first. The outcome comes back through `on_record_game`, which sets `recordedAt`, notifies `gameEnd` and arms the reap. TS awaited the call inside the run loop.
2. **The `flushProfile` retry is armed before the send**, not after a failure. A crash between send and outcome still re-sends on wake. A failure keeps the armed retry and does not schedule a second one. Only one `recordGame` is in flight at a time; the armed retry covers any send it skipped.
3. **Ordered per-seat outboxes** replace TS's in-loop awaits (`requests/server-ai-seat-dispatch-order.md`). TS awaited `handEnd` up to its budget before the next message; here the order is the same, but the table never waits.
4. **aiSeat calls carry `{ internalToken }` connection params** (`requests/server-ai-seat-table-params.md`). TS aiSeat had no connection auth.
5. **The `decide` budget is computed when the call is sent**, from `deadlineAt`, not when it was queued.
6. **The client slot is rebindable** (`TableClient`). A server binds its client after the registry exists (the engine client is built before the registry closure runs). With no client bound, the table plays the heuristic seat, and a match record waits on the `flushProfile` retry until a client is bound. Only a deployment with no profile token marks matches recorded without sending, as TS does without a deployment secret (changed in review, see Review fixes 3).
7. **Callbacks are actions, not durable queue sends (D6).** An `aiDecision` that reaches a table in its sleep grace can fail with `stopping`, or hang while the grace lasts. The seat must bound each try and retry (`requests/server-table-ai-seat-callback-retry.md`). The table never stalls on it while a player is connected: the watchdog plays the turn.
8. **Test-only hook** `test_support::request_sleep_after_dispatch` makes the table sleep right after one `decide`. It does nothing unless `test_support::enable()` ran.

## Gaps and risks

- **No TS-emitted `TableState` fixture exists** in `euchre-protocol/tests/fixtures`, so the `TableState` round trip (§16.8) is checked by key-order and null-versus-absent unit tests in `state.rs`, not against TS output. `sync` and `MoveAck` are checked against the TS goldens.
- **A schedule due during a sleep grace is lost by rivetkit** (`scheduled event execution failed error=stopping`). The ledger re-arms it on the next wake, but a table that no caller wakes stays parked. In practice a live game has a player socket, which keeps the table awake.
- **The engine e2e cannot catch an engine sleep with a fast-tempo timer due**, because fast timers fire before the 1 s idle sleep. The re-arm paths are covered by unit tests.

## Review fixes

The review found 14 issues. 12 are fixed, each with a regression test. One is rejected, and one is fixed in part. Request filed: `requests/server-table-d17-amendment.md` (D17, D20 and D22 text in ARCHITECTURE.md).

| # | Sev | Finding | Outcome |
| --- | --- | --- | --- |
| 1 | P1 | Token expiry re-checked on every action breaks games longer than the 2 h guest token on one socket | **Fixed.** `on_connect` stores the identity each connection proved (`ConnEntry`). Later actions on that socket only run `auth::authorize` (role against owner). A stateless HTTP call has no connection, so it is still checked in full on every call. This is how TS worked: `createConnState` ran once. Tests: `auth.rs` `caller_is_rederived_and_roles_gate_actions` (a kept identity authorizes after expiry), and the e2e `auth_checks` case (a token 3 s from expiry still works on its socket 5 s later, while a stateless call fails `invalid_token` "Invalid or expired token"). |
| 2 | P2 | `submitMove` validated and its seq computed outside the lane | **Fixed.** `submitMove` and `move` build their body inside the lane (`apply_move(MoveInput::Submit / Queue)`), so the second of two concurrent moves is checked against the state after the first. It gets `illegal_move`, as in TS, where `submitMove` validated and committed synchronously. Test: `a_second_move_validated_against_the_same_view_is_refused_once_the_first_applied`. |
| 3, 10 | P2 | An unbound `TableClient` is read as "no profile", so a match is marked recorded without being sent | **Fixed.** `ActorHost` now always holds the `Remote`. The profile seam is the remote whenever a profile token is configured. An unbound client makes the send fail, so `on_record_game` keeps the armed `flushProfile` retry. Only a deployment with no profile token uses `NoProfile`. Deviation 6 now says this. Test: `actor.rs` `an_unbound_client_is_not_a_missing_profile`. |
| 4, 7 | P2 | After a partial commit, the D20 retry re-fires a stale turn, and no next turn is armed | **Fixed.** `Session` counts its saves and can roll back to the saved copy. When a session fails after its first save, the table rolls back and spawns one recovery task (`spawn_recover`: keep-awake, up to `FIRE_RETRY_LIMIT` tries with a growing delay, abort-aware). It runs `reconcile`, which re-derives the open turn's timers from saved state. The same-args retry is kept only for a session that failed before any save. A move whose save landed is acknowledged `{ ok: true, v }` (`move_persisted`), so a client retry never sees an error for a move that took effect. In `on_ai_decision`'s early-answer path, the old watchdog and reveal are now cancelled only after the new reveal is saved, as TS did. `on_start` also spawns recovery if its reconcile fails. Tests: `a_turn_that_fails_after_its_first_save_is_recovered_by_reconcile`, `an_early_answer_keeps_the_old_timers_until_its_reveal_is_saved`. |
| 5 | P3 | Reader paths return `invalid_token` / `protocol_version_mismatch` instead of `forbidden` | **Rejected.** TS returned the same codes: `createConnState` threw `invalid_token` and `protocol_version_mismatch` before any action ran, and only role checks threw `forbidden`. A stateless call here fails the same way in `on_before_connect`. The e2e asserts these codes, and `euchre-http` maps them as it does today. With finding 1 fixed, a live socket no longer reaches this path after connect. |
| 6 | P3 | Minor input handling: `aiSay` `handNo: null`, create input, `read_params` | **Fixed in part.** `aiSay` with an explicit `handNo: null` is now dropped as a mismatch, as in TS (test: `a_line_with_a_null_hand_no_is_dropped_as_ts_did`). **Rejected:** create input validation, because TS `createState` has no validation and no error to copy (D27), and failing create on malformed input is the safer outcome. **Rejected:** `read_params`, because the decoder is the protocol crate's `TableConnectParams`. Its lenient fields give the same outcomes as TS (`?? ''` leads to `invalid_token`, a non-1 version to `protocol_version_mismatch`), and this lane must not redefine it. |
| 8 | P2 | A kill between the move's save and `hand_boundary`'s save loses a thrown-in hand, and records go out before their bookkeeping is saved | **Fixed.** `hand_boundary` is now synchronous. It does the bookkeeping on the working copy and returns the calls it owes (`Boundary`). The move and the boundary land in one save, and `send_boundary` sends `recordHands` and the seat notices only after that save, in order. Test: `a_thrown_in_hand_lands_in_the_same_save_as_the_move_and_is_sent_after_it`. |
| 9 | P2 | `on_connect` runs before rivetkit registers the conn, so a later sync can be missed and an aborted connect leaves a phantom in presence | **Fixed.** `on_connect` records the connection as pending and spawns `register_player` (register_task, keep-awake). That task waits until `ctx.conns()` lists the connection (it gives up if the entry was removed, or after 10 s). Then, in the lane, it marks the connection live, sends `sync` from current state, and broadcasts `presence` online on the first live player. Pending entries never count as present, and `on_disconnect` broadcasts offline only when the last live player leaves. Test: `actor.rs` `only_registered_player_sockets_count_as_present`. Engine e2e covers the connect, sync and presence flow. |
| 11 | P2 | A `flushProfile` fire lost in the sleep grace leaves a finished match unrecorded and never reaped | **Fixed.** While `profile_retry_at` is set, the table holds a `KeepAwakeRegion` (`hold_for_profile`, updated on every persist), so it cannot go idle-asleep before the record lands. `on_sleep` drops the hold, so a destroy or engine stop still finishes. The 30-day reap is not held. A lost reap only leaves a recorded table parked until its next wake, which re-arms it (D20). |
| 12 | P3 | The former `tick` queue lets an internal caller fire any timer at once, bypassing the ledger's early-fire guard | **Fixed.** `tick` now goes through `on_tick_message` to `fire`, which applies the ledger's due-time guard. Test: `an_internal_tick_cannot_fire_a_timer_before_it_is_due`. |
| 13 | P3 | The `move` action trusts client `seq` and `clientMoveId` (overflow, stuck game, state growth) | **Fixed.** `queue_move_body` rebuilds the body in the lane: server seq (`applied_seq.saturating_add(1)`), `clientMoveId` cut to 64 UTF-16 units, and the caller's own user id. `submit_move_body` also saturates. Test: `the_move_action_body_is_rebuilt_server_side`. |
| 14 | P3 | An internal-token reader gets seat 0's hidden hand | **Fixed.** `snapshot` for an internal caller returns `internal_view`: the seat 0 view with `hand` and `legal` cleared. Test: `an_internal_reader_never_sees_a_private_card`. |

### Gates and e2e after the fixes

- `cargo fmt -p euchre-table -- --check`: clean.
- `cargo clippy -p euchre-table --all-targets -- -D warnings`: clean.
- `cargo test -p euchre-table`: 77 lib tests pass. The e2e binaries skip without `EUCHRE_E2E`.
- `table_e2e` (port 6711): 4 passed in 71.65 s.
- `table_ai_e2e` (port 6711): 4 passed in 99.75 s.
- The engine refused to start while the disk had under 4 GiB free ("configured log shard capacity of 4294967296 bytes exceeds the ... bytes free"). `RIVET__STORAGE__LOG_SHARD__MAX_DISK_BYTES` did not lower that cap, so the runs above went through once free space rose above 4 GiB. Nothing from these runs listens on 6711-6720.
