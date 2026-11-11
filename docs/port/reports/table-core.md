# Report: SERVER-TABLE / TABLE-CORE

Crate: `crates/euchre-table`, the `euchreTable` actor on rivetkit 3.0.7. Owner lane: SERVER-TABLE.

- TS source of truth (read-only): `src/lib/actors/euchre-table/{index,queues,tempo,send-bounded}.ts` and `src/lib/actors/auth/*`.
- Spec: `docs/port/03-table-actor.md`, including §15 (code wins) and §16 (the Rust mapping).
- Binding decisions: `ARCHITECTURE.md` D6–D10, D17–D24, D27 and D29.
- Cookbook: G1–G38.

No other crate changed, and neither did the root `Cargo.toml`. This stage filed no new change request. The table reads the existing `docs/port/requests/server-ai-seat-table-params.md` and honours it in the next stage (see "AI seam").

## Gates

Run with `CARGO_TARGET_DIR=/Users/brittianwarner/goods/euchre-rune/.cargo-targets/table`.

| Gate | Result |
| --- | --- |
| `cargo fmt -p euchre-table -- --check` | clean |
| `cargo clippy -p euchre-table --all-targets -- -D warnings` | clean |
| `cargo test -p euchre-table` | 53 unit tests passed. The 4 e2e tests print `SKIP` without `EUCHRE_E2E`. 1 doc test is ignored. |

Code rules:

- Library paths have no `unwrap`, `expect`, `todo!()` or `unsafe`.
- Every public item has a doc comment.
- Logs carry no tokens. Error logs carry a 200-unit excerpt (`err_excerpt`).
- User-facing strings are copied byte for byte from TS (D27). This covers error messages, chat lines and log event names.

## Engine e2e

```text
EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6711 \
EUCHRE_TEST_STORAGE=/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/rivet-store-table \
perl -e 'alarm 500; exec @ARGV' cargo test -p euchre-table --test table_e2e -- --nocapture
```

**Result:** `4 passed; 0 failed`, in about 62 s. I ran it twice and both runs were green. The full match takes about 53 s: 11 hands, about 70 player moves and about 350 `sync` events.

**Engine hygiene:**

- Engines were started only through `euchre_server_core::testing::TestEngine`, which gives each test its own pool.
- Port 6711 was free before the run.
- After each run, `lsof -iTCP:6711 -sTCP:LISTEN` showed no listener.

**Known log line:** one `ERROR rivetkit_core::actor::context: scheduled event execution failed error=stopping` (`onTempoGate`) appears at the end of the review test. It comes from `TestEngine::shutdown` stopping an actor while a tempo fire is still queued. It is not a table failure.

The AI seats are the in-actor heuristic, so no `aiSeat` actor is involved. Coverage of each test:

**`table_auth_end_to_end`**
- **Claim:** a refused first visitor does not claim the table. The first valid guest Actors JWT claims it, over HTTP and then over WebSocket.
- **Bad tokens:** each is refused with the TS code and message, on HTTP and at WebSocket connect (fast `CallError::Rejected`). Cases:
  - a bad token, an expired token (iat − 7300 s), a `Session`-audience token and no token: `invalid_token` "Invalid or expired token";
  - seat 1, seat 3 and seat `"0"`: `seat_mismatch` "Seat 0 is the only seat a player may occupy" (seat 0 is accepted);
  - protocol version 2, `"1"` or `null`: `protocol_version_mismatch` "Unsupported protocol version", checked before the token;
  - another user's valid token: `forbidden` "Not your game".
- **Origin:** a client sending `Origin: https://evil.example` is refused with `origin_not_allowed` "Origin not allowed". The allowlisted Origin passes, and the internal token passes from any Origin.
- **Internal token:**
  - It can read: `snapshot` and `lastTrick` work.
  - It cannot act as a player: `submitMove`, `getHandReview` and `reviewTrick` return `forbidden` "Not a player connection".
  - A wrong internal token is `invalid_token`.
- **Former queues and schedule targets:**
  - Former queues reject players with `internal_only` "This action is internal.".
  - Schedule targets reject every client connection, internal included, with the same error.
  - A malformed `review` body gets `internal_error` "Invalid review request".
- **`getHandReview` validation:** `-1`, `1.5`, `"0"`, `99` and an unfinished hand all return `null`.
- **Presence:** an internal watcher gets no `sync`. It sees `{seat:0, online:true}` on the first player socket, nothing for the second socket or for HTTP calls, nothing when one of two sockets closes, and `{seat:0, online:false}` when the last one closes.

**`table_moves_end_to_end`**
- **Initial sync:** `steps: []`, with the current view.
- **Illegal move:** `illegal_move` "That move is not legal right now", with `metadata.legal` equal to the view's legal set.
- **Missing `clientMoveId`:** checked after legality, giving `internal_error` "Missing clientMoveId".
- **Stale turn:** `{ok:false, code:"stale_turn", legal}`, and nothing is applied.
- **Concurrent submits:** 5 concurrent submits of one move (3 over HTTP, 2 over WebSocket) apply exactly one move. `appliedSeq` goes up by 1 and the journal has one seat-0 entry. The other submits get idempotent acks or are refused because the turn moved on.
- **Duplicate:** on the next turn, the remembered `clientMoveId` gets `{ok:true}` and applies nothing.

**`table_match_end_to_end`**
- **One full hand:** `handsPlayed >= 1`, and `getHandReview(1)` returns the hand.
- **Whole match:** played to `status: "complete"`. The winner has at least 10 points, the persisted status is `complete`, and the reviews of the first and last hands are present.
- **After game over:** a move gets `game_complete` "This match is over".
- `sync` versions never go backwards, and `sync.v == view.v`.

**`table_review_sleep_end_to_end`**
- **The gate:**
  - `reviewTrick set` returns `{queued:true}`, and `reviewTricks: true` arrives in `sync`.
  - A bad request gets `internal_error` "Invalid review request".
  - The first finished trick is held at `trick_resolve` with `awaitingTrickReview: true` for more than 0.8 s.
  - A Continue for another turn is ignored, and `lastTrick` returns the held trick.
- **Sleep at the gate:** the table sleeps mid-hand at the gate (the `sleeps` counter goes up) and wakes (`starts + 1`). The persisted `game` and `journal` are unchanged. The first `sync` after the wake has `steps: []`, the same turn and the same `v`.
- **Continue after the wake:** the trick clears, and the AI reveal timers armed after the wake play on.
- **Sleep at a player turn:** with the gate off, play to a player turn mid-hand and sleep there. The table wakes on the same turn, and the hand is then played to its end.

The pacing profile is `fast_tempo` (`Tempo::FAST`, the same as TS `EUCHRE_FAST_TEMPO=1`), and `sleep_timeout` is 1 s.

**Limit of the sleep test:** fast-tempo timers (at most 250 ms) always fire before a 1 s idle sleep. So the e2e cannot catch an engine sleep with a timer still due. That re-arm path is proven by unit tests on the ledger instead (`core/tests.rs`):
- A wake resumes the read pause with 2300 ms left.
- An early fire re-arms the remainder.
- A stale fire does nothing.
- The reap ledger re-arms.

## Unit tests (53)

- **`core/tests.rs`**, against a fake `Host`:
  - Ports of `review.test.ts` (cases 1–11), `state-proxy` and the pure parts of `decision-delivery` (cases 1–7).
  - `dispatchAi` create-input key order. A parked heuristic move is played at the think floor.
  - `submitMove` check order.
  - `on_move` is idempotent, and it returns `stale_turn`.
  - Tempo: observation rounding (1130 → 1250) and wake resume.
  - Ledger: early-fire re-arm, stale-fire no-op, reap re-arm and destroy (not destroyed when the match is unrecorded).
  - Persistence: repeated commits (`v + 64`, journal capped at 40); a failed save leaves the live state unchanged.
- **`auth.rs`:** check order, origin gate, owner claim, caller roles.
- **`actions.rs`:** names match `table_actions` / `table_queues`; `null` arguments read as absent.
- **`send_bounded.rs`:** port of `send-bounded.test.ts`.
- **`tempo.rs`:** constants copied verbatim, think floor with fnv jitter.
- **`text.rs`:** chat screening.
- **`test_support.rs`:** recording stays off until enabled.

## Actions

Registry name `euchreTable`, key `["table", gameId]` (`actor_key`). The create input is `TableCreateInput`; `null` means the defaults.

Each action is a newtype over the whole positional argument array, so any arity decodes (G32). The role is re-derived from `ctx.conn().params()` in every action (D17). The roles are:

- **player:** the verified guest JWT owner at seat 0;
- **internal:** the per-match `internalToken`;
- **schedule:** no connection.

| Action | Arguments | Returns | Allowed |
| --- | --- | --- | --- |
| `snapshot` | none | `PublicGameView` (wire JSON), redacted for seat 0 | player, internal |
| `submitMove` | `{ moveId, turnId?, clientMoveId }` | `MoveAck`: `{ ok: true, v }` or `{ ok: false, code, legal }` | player |
| `reviewTrick` | `{ action: "set", enabled }` or `{ action: "continue", turnId }` | `{ queued: true }` | player |
| `getHandReview` | `handNo` (integer, `0..=current`) | `CompletedHandReview \| null` | player |
| `lastTrick` | none | `Trick \| null` | player, internal |
| `move` (former queue) | `MoveMsg` | `MoveAck` | player |
| `review` (former queue) | `ReviewMsg` | `null` | internal, schedule |
| `aiDecision` (former queue) | `AIDecision` | `null` | internal, schedule |
| `aiSay` (former queue) | `AiSayMsg` | `null` | internal, schedule |
| `tick` (former queue) | `TickMsg` | `null` | internal, schedule |
| `onTempoGate` / `releaseAiMove` | `turnId` | `null` | schedule only |
| `onAiTimeout` | `turnId, seat` | `null` | schedule only |
| `onNudge` / `onAbandon` | `turnId` | `null`; legacy, with no game effect (D7) | schedule only |
| `flushProfile` / `onReap` | none | `null` | schedule only |

`submitMove` checks, in TS order:
1. role: `forbidden` "Not a player connection";
2. `game_complete` "This match is over";
3. `illegal_move` "That move is not legal right now", with metadata `{ legal }`;
4. `internal_error` "Missing clientMoveId".

The move is then applied under the lane. A `RuleError` becomes the rejected ack.

## Events

| Event | Payload | Delivery |
| --- | --- | --- |
| `sync` | `{ v, view: PublicGameView, steps }` | Per connection, redacted for that connection's seat. On connect it is sent with `conn.send` and `steps: []`. Internal connections never get it. |
| `thinking` | `{ seat, on, extended }` | broadcast |
| `chat` | `{ msgId, seat, kind: "call" \| "banter" \| "system", text, isFinal: true }` | broadcast |
| `chatDelta` | `{ msgId, seat, delta }` | broadcast |
| `presence` | `{ seat, online }` | Broadcast only on 0→1 and 1→0 live player WebSockets. |

## Runtime shape

**State**
- One JSON row (`table_state` / `state`) plus one row per archived hand (`hand_review` / `<handNo>`), in the actor's SQLite through `JsonTable`. There is no `ctx.kv()`.
- `create` reloads the state row on every wake. On the first wake it mints the match (TS `createState`) and saves it before anything else runs.
- A hand and its review are written in one `execute_batch`.

**One writer (`ActorLane`)**
- These all run a `Session` under the lane: every mutating action, the former queues, schedule fires, the owner claim in `on_before_connect`, `on_start` reconcile, and the `on_disconnect` save (spawned with `register_task` and `keep_awake_region`).
- A session edits a copy. Order is persist, then publish the live state, then emit, then ack.

**Timers**
- Timers are `ctx.schedule()` one-shots. Their due times are kept in the state's `ledger` (`tempo`, `reapAt`, `profileRetryAt`) and in the game's `turnDeadlineAt` / `pending.revealAt`.
- `on_start` re-arms them through `reconcile`.
- A fire more than 25 ms early re-arms the remainder.
- Schedule targets never return `Err` (D20). A failure is logged (`euchreTable schedule target failed`) and re-armed after 1 s, up to 5 times in a row.

## AI seam (this stage) and what the next stage replaces

`src/seams.rs` holds two private traits, `AiDispatch { asks, decide, notify }` and `ProfileSink { available, record_hands, record_game }`.

**This stage**
- `HeuristicSeat` (`asks() == false`): `dispatchAi` parks the `euchre-ai` heuristic top move in `game.pending` (D8). The reveal timer then plays it at the think floor (`thinkFloorMs` plus fnv jitter), with no network call.
- `NoProfile` (`available() == false`): the match is marked recorded at game over, as TS does when no deployment secret is set. So no reap is armed, which matches TS.
- Both seams already receive the exact bodies the next stage sends: `DecideCall` (the seat key `["table", gameId, "seat", "<n>"]`, the TS `aiSeatCreateInput` with key order kept, and `AIDecideRequest`) and `LifecycleCall`.

**Next stage (AI-SEAT wiring)** replaces both bodies. Nothing else in the core changes, and calls go through the client only (D29).

- **`AiDispatch`.** A client-backed implementation whose `asks()` is `true`.
  - `decide` and `notify` run detached: inside `ctx.register_task`, holding `ctx.keep_awake_region()` (G25).
  - Each makes a stateless call: `ctx.client().get_or_create("aiSeat", key, { params: { internalToken }, create_with_input })`, then `.action("decide" | "resetHand" | "handEnd" | "gameEnd", [body])`. The `{ internalToken }` params are what `requests/server-ai-seat-table-params.md` asks for.
  - Calls are bounded by `send_bounded`. The `decide` budget is `max(1, deadlineAt - now)`; lifecycle calls use the 1 s default.
  - Failures log `dispatch to aiSeat failed; heuristic stands` or `notifyAiSeats failed`.
  - The seat answers through `aiDecision` / `aiSay`, with params `{ internalToken }`.
  - The parked heuristic move and the `onAiTimeout` watchdog stay as the fallback.
- **`ProfileSink`.** A client-backed implementation that calls `playerProfile`:
  - `recordHands`, detached. A failure logs `recordHand failed; hand journal entry dropped`.
  - `recordGame`, bounded. An `Err` makes the core schedule `flushProfile` (30 s); success arms the 30-day reap.
  - Both carry the deployment internal token.

## Deviations from TS

1. **Caller re-checked on every action (D17).** Each action re-derives the caller from its own connection params and re-checks the token, seat and owner. TS trusted the connection state set at connect.
2. **An unclaimed table refuses player actions** with `forbidden` "Not your game". A table is always claimed at connect, so this happens only to a token that changes between connect and call.
3. **`turnOpened.connected` is cleared only on 1→0**, when the last live player socket closes. TS cleared it on any disconnect.
4. **HTTP stateless calls are not counted as presence** and get no `sync`. rivetkit opens a conn for them, while TS counted only sockets.
5. **`reviewTrick` runs inline** under the lane, because there is no queue to put it on. It still returns `{ queued: true }`.
6. **Former queues are actions** (D6). Players calling `review`, `aiDecision`, `aiSay` or `tick` get `internal_only` "This action is internal.". The `move` action is player-only.
7. **Body validation messages are new:** `"Invalid <name> request"` (`internal_error`). TS dropped malformed queue bodies silently.
8. **`snapshot` and `lastTrick` are role-gated.** A schedule fire, which has no connection, is refused.
9. **Timer due times live in a ledger.** A fire more than 25 ms early re-arms the remainder.
10. **A tempo pause resumes where it was** on wake, instead of restarting from zero.
11. **An old reveal or watchdog is cancelled** when it is re-armed, so no orphan fires are left.
12. **AI dispatch is detached.** Nothing holds the lane across a network round trip.
13. **Schedule fires are retried (D20).** A failed fire is logged with a new event name, `euchreTable schedule target failed`, and retried after 1 s up to 5 times. TS let the run loop rethrow.
14. **A reconcile failure in `on_start` is logged, not returned**, so the table still starts.
15. **`screen_line` keeps surrogate pairs whole** when it truncates.
16. **No reap is armed without a profile.** This is the same as TS, but worth noting because an unrecorded table now stays forever.
17. **`onNudge` / `onAbandon` are no-ops** (D7). They are kept only so that schedules from older versions resolve.

## TABLE-AI-PROFILE

This stage replaces the two seam bodies of "AI seam" above with client-backed calls. The full lane report, with every action, event and cross-actor call, is `docs/port/reports/table.md`.

**What changed in the crate**

- `src/remote.rs` (new): one ordered outbox per AI seat and one for the profile, the `deliver` function that makes each call, and the `recordGame` / `recordHands` bodies.
- `src/actor.rs`: `Remote` implements `AiDispatch` (`asks() == true`) and `ProfileSink` over the outboxes when a client is bound. Without a client the table falls back to `HeuristicSeat` and `NoProfile`, as in TABLE-CORE.
- `src/core.rs`: `record_match_if_needed` saves the record snapshot and a pre-armed `flushProfile` retry under the lane, then hands the send to the outbox. The outcome re-enters the lane through `on_record_game`.
- `src/lib.rs`: `TableConfig` gains `client: TableClient` (a rebindable slot for the shared `rivetkit::client::Client`) and `profile_token` (the deployment internal token).
- `src/seams.rs`: `ProfileSink` is now three sync, detached calls taking `RecordHandsCall` / `RecordGameCall`.
- `src/test_support.rs`: a one-shot "sleep right after the next `decide`" hook for the crash test.

**Calls (all by actor name and action name, D29)**

- `aiSeat` key `["table", gameId, "seat", "<n>"]`, params `{ internalToken }`, create input = the TS `aiSeatCreateInput`: `decide` (budget `max(1, deadlineAt - now)`), `resetHand`, `handEnd`, `gameEnd` (budget 1 s).
- `playerProfile` key `["user", userId]`, no params: `recordHands` `{ internalToken, matchId, hands: [entry] }` once per finished hand, `recordGame` `{ internalToken, record }` once per match (1 s budget each).
- Each call is bounded by `send_bounded` against the actor's abort signal. A slow or missing target delays only its own outbox.

**Unit tests added (53 → 67)**

- `remote/tests.rs` (6): per-seat outboxes, the drainer start rule, a slow `handEnd` is never overtaken and never blocks another seat (`requests/server-ai-seat-dispatch-order.md`), one `recordGame` in flight, the `decide` budget, the seat params.
- `core/tests.rs` (8): the match record is sent once off the lane then settles and arms the reap; a failed record retries on `flushProfile` and never reaps unrecorded; a crash before the outcome sends again on wake; no owner means nothing is recorded; each finished hand goes to the profile, then seats hear `handEnd`, `resetHand`, `decide` in order; the forged-publish test (§14, removing the token check makes a forged call succeed); `sync` and `MoveAck` goldens against the protocol fixtures.

**Engine e2e** (`tests/table_ai_e2e.rs`, 4 tests, with stand-in `aiSeat` and `playerProfile` actors): commands, results and run history are in `table.md`. Note: the `chat` payload key is `final`, not `isFinal` as the Events table above says.
