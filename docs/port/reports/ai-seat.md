# Report: SERVER-AI-SEAT

Crate: `crates/euchre-ai-seat` (the `aiSeat` actor on rivetkit 3.0.7). Owner lane: SERVER-AI-SEAT.
Spec: 04 §14, §15, §18; 03 §10.2, §16.7; contract §2.4; 08 §5, §6, §12, G25, G26, G32, G35; D17, D19, D20, D27, D29.

No other crate changed. The pure handlers are `euchre_ai::ai_seat::handlers`; this crate is the glue.

## Gates

Run with `CARGO_TARGET_DIR=/Users/brittianwarner/goods/euchre-rune/.cargo-targets/ai-seat`.

| Gate | Result |
| --- | --- |
| `cargo fmt -p euchre-ai-seat -- --check` | clean |
| `cargo clippy -p euchre-ai-seat --all-targets -- -D warnings` | clean |
| `cargo test -p euchre-ai-seat` | 17 unit passed; 3 `profile_bodies` passed; 6 e2e skipped (no `EUCHRE_E2E`); 1 doc test ignored (the `register` example) |

The library paths have no `unwrap`/`expect`/`todo!`/`unsafe`. Every public item has a doc comment. Logs never carry a token: `AiSeatRuntime`'s `Debug` prints `<set>` for the profile secret, and a malformed create input is logged without the serde message, which can quote values.

## Engine e2e

```text
EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6771 \
EUCHRE_TEST_STORAGE=/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/rivet-store-ai-seat \
perl -e 'alarm 500; exec @ARGV' cargo test -p euchre-ai-seat --test ai_seat_e2e -- --nocapture
```

Result (after the review fixes): `6 passed; 0 failed`, in 11-13 s. It ran three times in a row and passed every time. A heuristic `decide` reached `aiDecision` in about 0.4 s, well inside its 30 s deadline.

The engine was started only through `euchre_server_core::testing::TestEngine`, on port 6771, with no conflict. Afterwards, `lsof -iTCP:6771-6780 -sTCP:LISTEN` showed no listeners.

The other actors are stand-ins in the test file, with the real registry and action names (D29):

- **`euchreTable`:** `relay` calls the seat through its own `ctx.client()`, with `create_with_input` and params `{internalToken}`, as 03 §16.7 says the table does. It records each `aiDecision` and `aiSay` it receives, with its params.
- **`playerProfile`:** records `recordEpisodes` and `recordTokens`. The test feeds those bodies to the real `euchre_profile_model` parsers.

| Test | Covers |
| --- | --- |
| `decide_end_to_end` | **Decide:** a heuristic `decide` returns `null` at once. The `AIDecision` reaches `aiDecision` before `deadlineAt`. Its keys come in TS order (`internalToken, gameId, seat, turnId, moveId, source, rationale, latencyMs`), the `moveId` is legal, `source` is `fallback`, and the table params are `{internalToken}`.<br>**Wrong-token callers:** refused with `internal_token_mismatch` through the relay and on a token-less `decide` (exact message). A token-less `getStatus` is answered, as in TS.<br>**Dropped messages:** a forged body token, another seat, another match and a redelivered `turnId` are all dropped. `tokenRejections`, `seatRejections` and `staleTurns` are each 1, and the forged and wrong-seat warnings are logged. Malformed bodies follow the TS check order on the raw JSON: no `seat` counts as another seat, a redelivered `turnId` as stale, another match drops silently. One that passes every check logs `aiSeat queue handler failed` with `name: "decide"`, exactly once. A later turn acts as a FIFO barrier.<br>**Reads:** `getStatus`, `getPersona` and `getDecisionLog`.<br>**Sleep and wake:** idle sleep (`sleep_timeout` 2 s, seen by the `test_support` counter), then a wake (start count +1). `memory`, `lastTurnId` and the counters are unchanged, and a redelivery of the last turn is still dropped as stale. |
| `lifecycle_end_to_end` | **`handEnd`:** the hand's episode goes to `playerProfile.recordEpisodes` at the create input's `profileKey`, carrying the profile secret. `parse_record_episodes` accepts it: role, matchId, handNo, and the summary `h<n>: I called it and made the point`.<br>**`resetHand`:** advances the memory.<br>**`gameEnd` with no model spend:** sends no `recordTokens`, as in TS. The buffer, notes and `notedHands` are persisted. |
| `scripted_llm_end_to_end` | **Decision:** `euchre_ai::testing::ScriptedJev` and `ScriptedTalk` through `jev_factory` and `LlmProvider::from_factory`, with no network. The play decision has `source: "llm"`, and the Jev client was called once.<br>**Table talk:** one line reaches `aiSay`, exactly `{internalToken, gameId, seat, turnId, handNo, msgId: "<turnId>:talk:<seat>", text: "Mine.", final: true}` with params `{internalToken}`. `banter.lastHandNo` is persisted.<br>**`gameEnd`:** sends `recordTokens` with the scripted usage (`tokensIn` 10). `TokenIncrement::from_body` reads it back and it matches the seat budget. |

## Actions

Every action is a newtype over the whole positional argument array (G32). The handler reads argument 0, so a malformed body is logged as in TS rather than failing as a masked decode error (G14). The four message actions require a connection whose params hold the match's `internalToken`; a call with no connection is allowed. A refused caller gets `user/internal_token_mismatch`. The three reads are open, as in TS.

| Action | Args | Returns | Effect |
| --- | --- | --- | --- |
| `decide` | `[AIDecideRequest]` | `null`, at once | Queued for the worker. Answered by a call to `euchreTable.aiDecision`, then possibly `aiSay`. |
| `resetHand` | `[AiSeatLifecycleMessage]` | `null`, at once | Queued. Runs `on_reset_hand`, then saves. |
| `handEnd` | `[AiSeatLifecycleMessage]` | `null`, at once | Queued. Notes the hand and buffers its episode, then saves. Then `recordEpisodes` and `complete_episode_flush`, then saves again. |
| `gameEnd` | `[AiSeatLifecycleMessage]` | `null`, at once | Queued. `recordEpisodes` and `complete_episode_flush`, then `recordTokens`, then saves. |
| `getStatus` | `[]` | `AiStatus` | Read. |
| `getPersona` | `[]` | `PersonaView` | Read. |
| `getDecisionLog` | `[]` | `DecisionRecord[]` | Read. |

Messages are handled one at a time, in arrival order, by a worker task that `on_start` registers. The worker holds the `ActorLane`, so this is the TS single run loop. Each accepted message carries a keep-awake region until it is handled (G25), so the seat cannot sleep with work pending.

The decide order is the TS order:

1. `on_decide` (token, match, seat, `lastTurnId`, absorb, ladder, record, usage).
2. Save.
3. `aiDecision`.
4. The banter gate. If it opens: save, then a background task (keep-awake) runs `run_banter_job`, then calls `aiSay`.

A dropped decide is saved only when the state changed (`state_changed`). Failures log the TS strings:

- `aiSeat queue handler failed` with `{name, err}`.
- `optional table talk could not be delivered`.
- `flushEpisodes failed; will retry at the next boundary` with `{err}`.
- `flushTokens failed` with `{err}`.

`err` is cut to 200 UTF-16 units.

## Cross-actor calls

All calls are stateless actions by name, through the runtime's shared JSON `rivetkit::client::Client` (D19, G26), each bounded by `DEFAULT_CALL_TIMEOUT` (5 s). No other actor crate's types are used (D29).

| Target | Key | Params | Action | Body |
| --- | --- | --- | --- | --- |
| `euchreTable` | `["table", gameId]` | `{internalToken}` | `aiDecision` | `AIDecision` |
| `euchreTable` | `["table", gameId]` | `{internalToken}` | `aiSay` | `AiSayMsg` (`final: true`) |
| `playerProfile` | `profileKey` from the flush | none | `recordEpisodes` | `RecordEpisodesBody` |
| `playerProfile` | `profileKey` from the flush | none | `recordTokens` | `RecordTokensBody` |

**Profile bodies** (`requests/ai-profile-bodies.md`, seat side): `tests/profile_bodies.rs` serializes the bodies the seat sends and feeds them through `parse_record_episodes`, `TokenIncrement::from_body` and `body_internal_token` (a dev-dependency on `euchre-profile-model`).

- The key order matches TS.
- Every `EpisodeKind` is accepted.
- Salience, `ts` and `episodeId` come through as expected.
- They agree, so no `ai-seat-profile-bodies.md` mismatch request was needed.

## How to register

```rust
use euchre_ai_seat::{AiSeatRuntime, SeatClient};

let client = euchre_server_core::rivet::build_client(&endpoint, &namespace, token); // the one shared client
let runtime = AiSeatRuntime::from_config(SeatClient::new(client), &cfg);
euchre_ai_seat::register(&mut registry, runtime);            // or register_with(.., ActorConfig)
```

- **`from_config`** builds the model factory from `cfg.ai` with `model_factory_from_env_with`, then the provider with `LlmProvider::from_factory` (with `SystemClock` and `TracingLog`). The profile secret is `cfg.secrets.internal`, which is what the profile verifies in every mode (local mode keeps it in `secrets.json`).
  - With no model key, the provider is empty: decisions take the heuristic and banter is silent.
  - `dev_from_process_env` reads both from the process environment, with the TS dev fallback secret. It is for tools only, because a local server's profile refuses that secret.
- **`SeatClient::deferred()` plus `bind(client)`** suits hosts that get the client only after the registry serves. The e2e binds `TestEngine::client()` after start.
- **Options:** `with_provider`, `with_log` and `with_call_timeout`.
- **Display name and icon:** `register_with` fills an unset `ActorConfig.name` and `icon` with the TS `AI Seat` and `robot`.
- **The runtime is per registry, not per process.** `register_with` passes rivetkit's own `run_actor::<AiSeat>` to the deprecated `Registry::register_with` entry inside a `tokio::task_local!` scope, and `AiSeat::create` reads it there. So two registries in one test binary can use different providers. This is the per-registry injection that `requests/server-profile-auth-scope.md` asked FOUNDATION about; the profile crate could use the same pattern.

## TS tests ported

None 1:1. `src/lib/actors/ai-seat/` has no tests.

- The related tests (`euchre-table/decision-delivery.test.ts`, `review.test.ts`) test the table side: parked fallbacks, think floors, duplicate and forged answers at the table, and review pauses. They belong to the table lane.
- `ai/partnership.test.ts` is pure AI. Its cases are already ported in `euchre-ai` (`tests/fixtures/partnership.json`).

The seat behaviours those tests rely on are covered by the e2e above: an idempotent `lastTurnId`, forged tokens dropped, one banter line per hand after the decision, and lifecycle flushes.

## Deviations

1. **The ack is immediate and the inbox is not durable** (03 §16.7).
   - TS queues acked once the message was durable.
   - Here a message is acked into memory. A keep-awake region holds the seat awake until the message is handled, so idle sleep never drops it.
   - A process crash between the ack and the handling loses it. The table's pending ledger and watchdog fallback cover `decide`; the lifecycle messages are best-effort in both ports.
2. **The message gate differs from TS.**
   - TS denied WebSocket publishes (`externalDenied`) and left the reads open.
   - Rust cannot tell WebSocket from HTTP, so the four message actions require params `{internalToken}`. Connections and the reads stay open, as in TS (`scripts/ai-status.mjs` and `scripts/e2e-browser.mjs` read `getStatus` with no params).
   - The refusal message `aiSeat accepts actor-to-actor calls with the internal token only` is new; TS had no string for it. See `requests/server-ai-seat-table-params.md`; the TS table passed no params.
3. **Malformed bodies.** Lifecycle bodies are read as TS read them: only `internalToken`, `gameId` and `handNo`, never `kind`. A `resetHand` whose `handNo` is not a whole `u32` logs `aiSeat queue handler failed`; TS stored it as is. A decide body that fails to decode goes through the TS checks on the raw JSON (token, match, seat, `turnId`) with the TS counters and logs. Only one that passes them all logs `aiSeat queue handler failed`; TS threw on the partial view at the same point.
4. **`handEnd` saves twice.** It saves before the `recordEpisodes` send as well as after, so a crash during the send cannot lose the buffered episode.
5. **A malformed create input wakes an unprovisioned seat.** The seat has an empty token, so nothing it is sent authenticates. It logs `aiSeat create input is malformed; the seat is unprovisioned`, which is a new string.
6. **Each call is bounded** (`DEFAULT_CALL_TIMEOUT` 5 s). A timed-out `aiDecision` logs `aiSeat queue handler failed` with `aiDecision timed out after 5s`, which is the TS path for a failed send.
7. **Out-of-order lifecycle messages are dropped.** The table's sends are detached (03 §16.7), so they can overtake one another; TS awaited them in order. A `handEnd` older than the memory's hand, or a `resetHand` for an older hand or one a decide already absorbed, is dropped. It logs `aiSeat dropped an out-of-order lifecycle message` (`{name, handNo, memoryHandNo}`), a new string. The real fix is an ordered outbox per seat in the table: `requests/server-ai-seat-dispatch-order.md`.
8. **Messages during sleep grace are handled inline.** rivetkit 3.0.7 fires the abort signal when grace starts but still dispatches actions. The worker then takes the lane, closes the inbox and drains it. A later message is handled inside its own action, under the lane, before the ack.

## Review fixes

Every finding was checked against the code, TS (`~/goods/euchre`) and rivetkit-core 3.0.7. All seven were real; each is fixed except where noted.

| # | Finding | Outcome |
| --- | --- | --- |
| 1 (P2) | Messages that arrive during sleep grace are refused, or acked and dropped | **Fixed.** Confirmed in rivetkit-core 3.0.7: `start_grace` cancels the abort signal (task.rs:1748), and `accepting_dispatch` still accepts during `SleepGrace` (task.rs:2134). After the abort, `drain` takes the lane, calls `rx.close()` and drains what is queued with `try_recv`, then exits; a registered task drains grace, so the worker cannot simply stay. `accept` hands a message that `send` returns after the close to `run_message`, under the lane, inside the action. An in-flight action blocks finalize, so nothing acked is dropped, and the lane keeps the queued messages ahead of the inline ones. Tests: unit `a_closed_inbox_hands_the_message_back`; e2e `sleep_grace_end_to_end` (below). |
| 6 (P3) | Same as 1, durability lens | **Fixed** by the same change. A message that truly cannot be queued no longer returns `aiSeat inbox is closed`: it is handled. |
| 2 (P2) | Reads are token-gated, which breaks the TS tooling | **Fixed.** Confirmed: `scripts/ai-status.mjs:5` and `scripts/e2e-browser.mjs:108` call `getStatus()` with no params. `on_before_connect` is gone (the default accepts). The reads skip the token check, and only `decide`/`resetHand`/`handEnd`/`gameEnd` require it. Updated `decide_end_to_end`: a token-less `getStatus` is answered, and a token-less `decide` gets `user/internal_token_mismatch` with the exact message. The report and `requests/server-ai-seat-table-params.md` are corrected. |
| 3 (P3) | Strict body parsing turns TS no-ops and counters into `queue handler failed` | **Fixed.** `lifecycle_body` reads only `internalToken`, `gameId` and `handNo`; `kind` comes from the action. `malformed_decide` applies the TS order on the raw JSON: forged token (`tokenRejections`, warn), another match (silent), seat `!==` (`seatRejections`, warn `{mine, claimed}`, with `claimed` absent when TS logged `undefined`), `turnId === lastTurnId` (`staleTurns`, `null === null` included). Tests: unit `hand_numbers_read_like_ts`. e2e: `lifecycle_end_to_end` sends `handEnd` without `kind`, which still flushes, with no failure log. `decide_end_to_end` sends a seatless body (`seatRejections` 2), a stale malformed one (`staleTurns` 2), another match's (silent) and one that fails (exactly one `aiSeat queue handler failed`). |
| 4 (P3) | `from_process_env` defaults to a profile token the local server rejects | **Fixed.** New `AiSeatRuntime::from_config(client, &ServerConfig)`: models from `cfg.ai`, profile secret from `cfg.secrets.internal`. The env-only path is renamed `dev_from_process_env` and documented as tools-only. No other crate called the old name. Tests: `from_config_uses_the_servers_internal_secret`, `from_config_follows_a_generated_local_secret` (`ServerConfig::local`: the generated secret, not the dev one), `from_config_builds_the_model_provider_from_the_ai_settings`. |
| 5 (P2) | Detached table dispatch can reorder `handEnd`/`resetHand`/`decide` and corrupt episodes | **Fixed locally, with a request.** Confirmed: TS awaited `handEnd` (euchre-table/index.ts:986), then `resetHand` (:997), then `decide` (:847), and 03 §16.7 makes every Rust send detached. The seat now drops a `handEnd` older than `memory.handNo`, and a `resetHand` older than it or equal to it once a view is absorbed, logging `aiSeat dropped an out-of-order lifecycle message`. That prevents wrong notes and wiped memory. The overtaken hand's episode is still lost, though: only ordered delivery keeps it. `requests/server-ai-seat-dispatch-order.md` asks SERVER-TABLE for one ordered outbox per seat. Tests: unit `late_lifecycle_messages_are_out_of_order`; e2e `out_of_order_end_to_end` (`decide(N+1)`, then `handEnd(N)` and `resetHand(N+1)`, both dropped and logged; then `handEnd(N+1)` notes `hN+1` and the memory is intact). |
| 7 (P3) | No test covers the failure paths | **Fixed.** The stand-ins take per-actor modes (hang n times, fail n times). `failure_paths_end_to_end` (call timeout 800 ms): a 3 s hung `aiDecision` logs `aiSeat queue handler failed` with `aiDecision timed out` within 3 s, and the next decide is answered. Its failed `aiSay` logs `optional table talk could not be delivered`. A failed `recordEpisodes` logs `flushEpisodes failed; will retry at the next boundary` and keeps the buffer (`episodeBuffer` 1). `gameEnd` resends the same body and clears it. The banter keep-awake holding the seat past `sleep_timeout` is exercised by `sleep_grace_end_to_end`, where a 4 s hung `aiSay` keeps grace open. `flushTokens failed` is not separately tested (it is the same send-and-log shape as `recordEpisodes`). |

**What the grace e2e shows.** `sleep_grace_end_to_end` uses a test-only hook, `test_support::sleep_on_next_message`: a runtime switch that is off unless `test_support::enable()` ran. With it, the seat calls `ctx.sleep()` while it handles a message, and a 4 s hung `aiSay` keeps grace open. A burst of 20 messages across the start of grace, then a `handEnd` and a `decide`, all succeed and are all handled: `staleTurns` 20, `recordEpisodes` received, `aiDecision` for `s2`, no failure log.

On this engine, the requests sent after the sleep request were held and served by the next generation (`starts 1 -> 2`, 0 handled inline). So the in-grace dispatch window (requests already routed to the old generation) cannot be forced from a test. The inline path's channel semantics are covered by the unit test, and the e2e proves no message sent across a grace boundary fails or is lost.

**Gates after the fixes** (`CARGO_TARGET_DIR=.cargo-targets/ai-seat`):

- `cargo fmt -p euchre-ai-seat -- --check`: clean.
- `cargo clippy -p euchre-ai-seat --all-targets -- -D warnings`: clean.
- `cargo test -p euchre-ai-seat`: 17 unit tests and 3 `profile_bodies` tests passed; 6 e2e tests skipped; 1 doc test ignored.
- Engine e2e on port 6771 (`TestEngine`): `6 passed; 0 failed`, three runs in a row (11-13 s each).
- Afterwards, `lsof -iTCP:6771-6780 -sTCP:LISTEN` showed no listeners.

