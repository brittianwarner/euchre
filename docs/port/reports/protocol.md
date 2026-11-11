# Lane PROTOCOL report: `crates/euchre-protocol`

**Status:** done. All three gates pass:
- `cargo fmt -p euchre-protocol -- --check`
- `cargo clippy -p euchre-protocol --all-targets -- -D warnings`
- `cargo test -p euchre-protocol`: 106 tests (after the review fixes below; 87 before)

## What is done

| Module | Contents |
|---|---|
| `lib.rs` | `PROTOCOL_VERSION = 1`. Re-exports the engine wire vocabulary (never `GameState`) and every protocol type. |
| `errors.rs` | `ProtocolErrorCode` (13 codes, `PROTOCOL_ERROR_CODES`). `ErrorCode = Rule(RuleCode) \| Protocol(..)` as a bare string, aliased as `MoveRejectionCode`. `ConnectionErrorCode`, `ProtocolError`, `UserErrorPayload` (+ `illegal_move_legal()`). |
| `conn.rs` | `TableConnectParams`, `ConnRole`, `TableConnState`. `TableSessionResponse` + `is_stale` (10 s margin, same rule as `client/rivet.ts`). |
| `events.rs` | `SyncEvent`, `ThinkingEvent`, `ChatKind`, `ChatEvent` (`final: true` literal), `ChatDeltaEvent`, `PresenceEvent`. `TableEventName`, `TABLE_EVENT_NAMES`, `PER_CONNECTION_EVENTS`. `TableEvent::decode(name, payload)`. |
| `actions.rs` | `SubmitMoveRequest`. `MoveAck`, discriminated by `ok` with a manual Serialize and Deserialize that require each arm's fields. `ReviewTrickRequest` (tag `action`), `ReviewTrickAck`, `hand_review_number` (the `getHandReview` guard). `TableCreateInput`. Name constants for actors, table actions (client and schedule targets), table queues, aiSeat queues and actions, and playerProfile queues and actions (05 §0.6). `is_game_id`, `table_key`, `ai_seat_key`, `profile_key`, `parse_move_id`. |
| `internal.rs` | `InternalEnvelope`, `token_ok` (constant time), `MoveMsg`, `ReviewAction`, `ReviewMsg`, `TickKind`, `TickMsg`. |
| `ai.rs` | `AIDecisionKind`, `RankedMove`, `AIDecideRequest`, `AIDecision`, `AiSeatLifecycleKind`, `AiSeatLifecycleMessage`, `AiSayMsg`. |
| `persona.rs` | `AnthropicModelId` (+ `ModelId`), `MODEL_PLAY`, `MODEL_BID`, `NO_SAMPLING_MODELS`, `Difficulty`, `DIFFICULTIES`. All caps and dial/temperature bounds. `PersonaConfig`, `PersonaAssignment`, `PersonaView`, `default_personas()`, `persona_style`, `table_players`. |
| `records.rs` | `MatchOutcome`, `MatchStats`, `TokenUsage`, `MatchRecord`, `HandJournalEntry`, `MatchSummary`, `MatchListPage`, `LifetimeStats`, `PlayerProfileState`, `EpisodeKind`, `EpisodeRecord`. |
| `pacing.rs` | `TABLE_MOTION`, `TRICK_PRESENTATION_MS` (3300). |
| `hand_review.rs` | `CompletedHandReview` and `build_hand_review(&HandState)`, which follows the `archiveHand` rules exactly. |
| `wire.rs` | `WireView`, `serialize_view`, `view_to_value`. These write a view with the five actor metadata keys last, as TS does. |
| `store.rs` | Pure `TableStore` (02 §6). It holds the TS store (`bind`, `apply_sync`, `apply_chat`, `resync`, `play`) plus the page wiring (`attach`, `set_connected`, `on_sync_event`, `on_chat_event`, `submit_move`, `review_trick`, `open_hand_review`, `choose_hand_review`, `tick`, `next_deadline_ms`), the pre-view loading copy (`loading_notice`, `loading_error_line`) and the hand-review panel state (`HandReview.svelte`). Effects come back as `StoreEffects { commands: Vec<StoreCommand>, events: Vec<StoreEvent> }`; settlements carry the `RequestId` they settle, refusals are separate events. Replies come in through `on_snapshot`, `on_submit`, `on_review` and `on_hand_review`. Time is injected as monotonic `now_ms`; every TS timeout (4 s snapshot, 12 s action, 10 s poll, 6.5 s talk) fires from `tick`. |
| `serde_util.rs` | `string_enum!`, `True`, `js_f64` + `js_number_string` (ECMAScript `Number::toString`: `12`, `0.0000015`, `1e+21`), `double_option`, `lenient` (TS-style field reads for untrusted client input). |
| `profile/mod.rs` | Reserved for lane SERVER-PROFILE (doc line only). |

### Golden generator

`crates/euchre-protocol/scripts/gen-fixtures.ts`, run with `bun run crates/euchre-protocol/scripts/gen-fixtures.ts`. It imports the real TS engine, pacing and personas from `EUCHRE_TS_SRC` (default `~/goods/euchre`), read-only. A small mirror of the table harness plays one deterministic match and writes 9 fixtures (76 KB) to `tests/fixtures/`:
- view
- sync
- steps (all 12 step types)
- events
- actions
- ai
- review (including the source `HandState`s)
- records
- pacing

Every entry is deserialized, re-serialized and compared byte for byte. Tests also check that optional fields are omitted and nullable fields are written as `null`. `build_hand_review` is checked byte for byte against the TS `archiveHand` output, for both a scored hand and a throw-in.

## Test counts

| Source | TS `it()` | Rust `#[test]` |
|---|---|---|
| `src/lib/game/table.test.ts` | 3 | 3 (1:1, same names in snake_case) |
| Added: §6 store rules (6.1, 6.2, 6.3, 6.5) | — | 43 |
| Added: engine cross-check (Rust `project(create_game(store-test))` equals the TS fixture) | — | 1 |
| Added: JSON goldens | — | 27 |
| Added: unit tests (codes, move ids, keys, guards, serde helpers) | — | 13 |
| Added in review fixes (see below) | — | 19 |
| **Total** | **3** | **106** |

TS tests in other lanes, not ported here:
- `lib/ui/table-presentation.test.ts` (UI lane; uses pacing).
- `lib/actors/euchre-table/decision-delivery.test.ts` (SERVER-TABLE lane).

## Deviations (Rust from TS or spec; TS wins where they disagree)

1. **View key order.** The engine declares the five `PublicGameView` metadata fields first, but TS writes them last. `wire::WireView` fixes this, and `SyncEvent` and `AIDecideRequest` use it. The table's `snapshot()` must return `WireView(&view)`. Engine request: `docs/port/requests/protocol-view-field-order.md`.
2. **Spec 02 §6.3 vs TS.** The spec lists two review failure messages. TS throws "The table did not confirm…" and catches it, so the user only ever sees "Could not update the table. Please try again." The store follows TS and has one message.
3. **Spec 02 §7 `upCard`.** The spec suggests `Option<CardId>` that reads null as None. That cannot reproduce `upCard: null` versus an absent key byte for byte, so it is `Option<Option<CardId>>` (`double_option`).
4. **Spec 02 §13.5 tuples.** The spec says `[i32; 2]`. Scores, deltas and tricks follow the engine's `[u8; 2]`. `MatchStats.tricks` is `[u32; 2]`.
5. **`TableConnectParams.token`.** It is required in TS. In Rust a missing, `null` or non-string token reads as `""` (TS `params?.token ?? ''`), because the table's internal connections send `""` and a bad token must reach `invalid_token`, not a parse error.
6. **`final` is a reserved word in Rust.** `ChatEvent.is_final` and `AiSayMsg.is_final` are renamed back to `final` on the wire.
7. **`PersonaConfig` key order.** Rust writes interface order. TS `defaultPersonas()` writes spread order (`id, name, blurb, prompt, difficulty, aggression, risk, chattiness, version, housePrompt, …`). The fixture generator rewrites personas into interface order. JSON readers are not affected, but raw TS bytes will not equal Rust bytes for persona objects.
8. **Store API is pure.** These follow from the pure, injected-time design:
   - `bind(connected, now_ms)` arms the 10 s poll from `now_ms`.
   - `play(move_id, client_move_id)` takes the UUID from the caller.
   - A refused `play` returns `PlayRefused` straight away, in place of a resolved `false`. A settled play is `PlaySettled { id, ok }`, keyed by its `SubmitMove` id.
   - `play`, `submit_move` and `review_trick` take `now_ms` to arm the 12 s action timeout.
   - A snapshot reply that arrives exactly at its 4000 ms deadline counts as timed out.
   - A late poll tick fires once and keeps the interval's phase.
   - `console.error` is not modelled.
   - With `throwOnError: false` the web client sees every `submitMove` failure (a `UserError`, a transport error, a dropped connection, the 12 s timeout) as `undefined`. A host matches it by reporting every failure as `SubmitOutcome::NoAck`, which shows "The table could not confirm your move. Your game is saved. Please try again." `SubmitOutcome::Failed` (the TS `catch` branch, "Connection interrupted…") is kept for parity but is unreachable in the web client; hosts never report it. The same holds for `getHandReview` (`HandReviewOutcome::Loaded(None)` vs `Failed`).
9. **`hand_review_number` accepts `3.0`,** matching JS `Number.isInteger`.
10. **`TableEvent::Sync` is boxed** to satisfy clippy `large_enum_variant`.

## Fixture notes

- The `reviewGate` sync fixture is built as `onReview set` does it: v+1, with `reviewTricks: true`.
- `ackErr` is a real engine `RuleError` (`stale_turn`) with its `legal` set.
- `connectParamsInternal` is written in interface order.

## Open issues

1. **Engine field order** (request file above). Low severity; worked around.
2. **`StoredEpisode`, `ProfileStats`, `ClientMatchReplay` and replay redaction.** ARCHITECTURE §2.2 assigns them to this crate. They are 05 types tied to profile logic and are left to SERVER-PROFILE under `src/profile/`.
3. **`last_good_view` latch.** It is ported and re-latches on every display change, including a dropped preview. It cannot be observed through the public API while `view` never returns to `None` (the TS store never clears it either); in-module unit tests force `view = None` to cover it.

## Not ported (with reason)

- `personasFromSettings`, `ProfileSettings`, `TablePrefs`, patch, sanitize and normalize: reserved for lane SERVER-PROFILE (`src/profile/`).
- `screenLine`, `tempo.ts`, `tableView` assembly and `observeMove`: table-actor logic for lane SERVER-TABLE. Protocol provides `table_players`, `persona_style` and `WireView` for it.

## Review fixes

Each finding was checked against the TS source before any change. All 11 are confirmed and fixed. Gates after the fixes: fmt, clippy (`-D warnings`) and 106 tests pass.

| # | Finding | Verdict | Fix and regression test |
|---|---|---|---|
| 1 | `SubmitMoveRequest` requires `turnId` and `clientMoveId` (major) | **Fixed.** Confirmed: `euchre-table/index.ts:1861-1912` checks forbidden, then game_complete, then `illegal_move` with `{legal}`, then `internal_error` for `clientMoveId`; `reduce.ts:515` skips the stale check for `turnId === undefined`. | `SubmitMoveRequest` now deserializes from any JSON value (`from_wire`). `turn_id` is `Option<String>`: absent is `None` (no stale check, D9), and a present non-string reads as `Some("")`, which is stale as TS `null !== turnId` is. `move_id` and `client_move_id` read non-strings as `""`. `has_client_move_id()` is the TS check. `MoveMsg.turn_id` is optional too. Test: `submit_move_reads_leniently_for_the_ts_check_order`, covering both missing-field payloads, a numeric `clientMoveId`, `turnId: null` and non-object requests. |
| 2 | `TableConnectParams` types `seat`/`protocolVersion` (minor) | **Fixed.** Confirmed: `index.ts:1659-1678` refuses any present value that is not `=== 0` / `=== 1`, including `null`. | `seat` and `protocol_version` are raw `Option<Value>` that keep a present `null`. `internalToken` reads non-strings as `None` (TS `tokenOk` takes `unknown` and returns false), and `token` reads non-strings as `""`. New `seat_claim_ok()`, `protocol_version_ok()`, `claim_error()` (TS order: version, then seat) and `with_seat()`. Tests: `connect_claims_follow_the_ts_guard` (16 cases), plus updated `connect_params_round_trip` and `seats_out_of_range_are_rejected`. Those two Rust-only tests had pinned the old non-TS behaviour (seat 7 failing to parse, `seat: null` dropped); they now assert the TS outcome. |
| 3 | `js_f64` falls back to serde_json's float format (minor) | **Fixed.** Confirmed with bun: `JSON.stringify(0.0000015)` is `0.0000015`. | New `js_number_string` implements ECMAScript `Number::toString` over Rust's shortest round-trip digits. `js_f64` writes integers up to 2^63 as `i64` on every serializer. It writes other finite values as raw JSON text through `serde_json::value::RawValue`; this enables the `raw_value` feature in this crate's `Cargo.toml` only, and the root `Cargo.toml` is untouched. Binary serializers get `f64`, and NaN gives `null`. Tests: extended `js_f64_prints_like_json_stringify` (1e-6, 1.5e-6, 2^53, 1e19, 1e20, 1e21), `js_number_string_matches_ecmascript_layout`, `js_f64_keeps_the_value_through_to_value_and_writes_null_for_nan`, and the differential `js_numbers_match_json_stringify` over 246 TS-written numbers (`tests/fixtures/numbers.json`, 13 KB, written by the generator). Note: `serde_json::to_value` keeps the value but not the text, because a `Value` number prints with ryu. Byte-exact output needs `to_string`/`to_writer` on the typed value. |
| 4 | `AiSayMsg` key order differs from the real producer (minor) | **Fixed.** Confirmed: `ai-seat/index.ts:400-409` writes `internalToken, gameId, seat, turnId, handNo, msgId, text, final`. | Fields reordered to match the producer, with `delta` after `msgId`. The generator now builds `sayFinal` from a mirror of the aiSeat literal (`aiSayFinal`) instead of by hand. Test: `ai_say_and_lifecycle_round_trip` asserts the exact key order. |
| 5 | No 12 s deadline for `submitMove`/`reviewTrick` (major) | **Fixed.** Confirmed: `client/rivet.ts:57` `actionDefaults: { timeout: 12_000, throwOnError: false }`; `rivetkit.svelte.js` times out and resolves `undefined` (line 236). | `ACTION_TIMEOUT_MS = 12_000`. The submit (in `AwaitingAck`), the review and the new hand-review load each carry a deadline. `tick` treats an expired submit as `NoAck` (it goes on to resync), an expired review as `on_review(id, None)` and an expired load as no archive. A late reply is ignored. Tests: `a_submit_that_never_answers_times_out_after_12s_as_a_lost_ack`, `a_timed_out_submit_whose_move_landed_counts_as_landed`, `a_review_that_never_answers_times_out_after_12s`, `a_hand_review_that_never_answers_times_out_as_no_archive`. |
| 6 | Transport failure mapped to `Failed` / "Connection interrupted" (major) | **Fixed (docs + guidance).** Confirmed: with `throwOnError:false`, `failGuard`, `settleFailure` and the timeout all resolve `undefined`, so `table.svelte.ts:132-136` (`catch`) is unreachable in production. | The module docs, `SubmitOutcome` and `MSG_CONNECTION_INTERRUPTED` now say hosts report every failure as `NoAck`, and that `Failed` is a dead TS-parity branch. Deviation 8 is corrected. `a_thrown_submit_with_the_same_turn_reports_an_interruption` stays as the parity test for the TS `catch` branch. The new timeout tests pin the `NoAck` copy for the common failure path. |
| 7 | Settlement events carry no correlation id (minor) | **Fixed.** Confirmed in `store.rs` as described. | `PlaySettled { id, ok }`, `ReviewSettled { id, ok }` and `ResyncSettled { id: Option<RequestId> }` now carry ids. Immediate refusals are separate `PlayRefused` and `ReviewRefused` events. `StoreEffects::play_settled(id)`, `review_settled(id)`, `play_refused()`, `review_refused()`, `submit_id()` and `review_id()` replace the first-match helpers. Tests: `a_refused_play_never_reads_as_the_in_flight_play_settling`, `a_refused_review_never_reads_as_the_in_flight_review_settling`, plus id assertions in existing tests. |
| 8 | Time contract underspecified (minor) | **Fixed.** Confirmed: TS uses `setTimeout` and `setInterval` (monotonic). | The module docs define `now_ms` as monotonic store-clock ms. `TableTalk.until_ms` is documented on that clock. New `next_deadline_ms()` returns the earliest of the snapshot, submit, review and hand-review deadlines, the talk expiry and the poll. Test: `next_deadline_is_the_earliest_pending_timer`, plus deadline asserts in the timeout tests. |
| 9 | Last-good-view latch misses a dropped preview (minor) | **Fixed.** Confirmed: `+page.svelte:171-174` re-latches on every `displayView` change. | `fail_page_preview` re-latches after clearing `optimistic`. That covers both the refusal path and the failed-settle path. The latch stays eager (one view clone per change), which is acceptable for a client. Tests (in-module, forcing `view = None`): `a_refused_submit_relatches_the_live_view_not_the_rejected_preview` and `a_play_refused_while_submitting_relatches_too`. |
| 10 | `reconnect_notice()` shows before any view (minor) | **Fixed.** Confirmed: `+page.svelte:357-367` loading branch, notice at 390-392 inside `{:else}`. | `reconnect_notice()` is gated on `display_view().is_some()`. New `loading_notice()` returns the connected and disconnected heading and body. New `loading_error_line(transport_error, session_error)` mirrors `{#if table.lastError \|\| connectionError}` and `connectionError ?? …`. New constants cover the four strings, the error line, `Unable to open this table.` and the two button labels. Tests: `before_the_first_view_the_loading_screen_shows_and_not_the_reconnect_notice`, `the_loading_error_line_follows_the_page`. |
| 11 | Hand-review loading not in the store (minor) | **Fixed.** Confirmed: `HandReview.svelte:20-55`. | New `StoreCommand::GetHandReview { id, hand_no }`, `open_hand_review` (with `initial_review_hand`: the previous hand when the current one has no tricks and no result), `choose_hand_review`, `on_hand_review(id, HandReviewOutcome)` (latest request wins; `This older hand was not saved by the previous version.` only for a hand other than the displayed one; `Could not load this hand. Please try again.` on the parity-only `Failed`), and `HandReviewPanel` state. `hand_review_tricks()` and `hand_review_totals()` port the `$derived` fallbacks. `MSG_OPENING_HAND` is the busy copy. Tests: `the_review_opens_on_the_previous_hand_until_the_current_one_has_play`, `the_latest_hand_review_request_wins`, `a_missing_archive_reads_as_unsaved_only_for_an_older_hand`, `a_hand_review_that_never_answers_times_out_as_no_archive`. |

API changes other lanes must pick up:
- `SubmitMoveRequest.turn_id` and `MoveMsg.turn_id` are now `Option<String>`.
- `TableConnectParams.seat` and `.protocol_version` are now `Option<serde_json::Value>`. Use `claim_error()`.
- `TableStore::play`, `submit_move` and `review_trick` now take `now_ms`.
- `StoreEvent` variants changed, and `StoreEffects::play_settled` and `review_settled` now take an id.
- No other crate used these types at the time of the fix.
