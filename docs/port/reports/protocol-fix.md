# Lane PROTOCOL-FIX report: `crates/euchre-protocol`

**Status:** done. All 11 adversarial-review findings were checked against the TS source and confirmed. All 11 are fixed, each with a new or stronger regression test. The finding-by-finding table, with TS evidence, is in the "Review fixes" section of `docs/port/reports/protocol.md`.

Gates (with `CARGO_TARGET_DIR=.cargo-targets/protocol`):
- `cargo fmt -p euchre-protocol -- --check`: pass
- `cargo clippy -p euchre-protocol --all-targets -- -D warnings`: pass
- `cargo test -p euchre-protocol`: 106 passed (17 unit, 30 golden, 59 store)

## What changed

- **`actions.rs` / `internal.rs`.** `SubmitMoveRequest` reads any JSON leniently (`from_wire`). That keeps the TS check order: forbidden, then game_complete, then `illegal_move {legal}`, then `internal_error`.
  - `turn_id: Option<String>`. Absent skips the stale check (D9). A present non-string reads as `Some("")`, which is stale.
  - `has_client_move_id()` is the TS client-move-id check.
  - `MoveMsg.turn_id` is optional.
- **`conn.rs`.** `TableConnectParams` keeps raw `seat` and `protocolVersion` claims as `Option<Value>`, with a present `null` kept.
  - New `claim_error()`, `seat_claim_ok()`, `protocol_version_ok()` and `with_seat()`.
  - `token` and `internalToken` read leniently, as TS does.
- **`serde_util.rs`.**
  - New `js_number_string`, which implements ECMAScript `Number::toString`.
  - `js_f64` now writes JS number text through `RawValue`. This needs the `raw_value` feature, enabled in this crate's `Cargo.toml` only.
  - New `lenient` field readers.
- **`ai.rs`.** `AiSayMsg` fields follow the key order of the real aiSeat producer. The generator builds `sayFinal` from a mirror of that literal.
- **`store.rs`.**
  - A 12 s action timeout for submit, review and hand review (`ACTION_TIMEOUT_MS`), plus `next_deadline_ms()`. `now_ms` is documented as monotonic.
  - Settlements carry their `RequestId`, and refusals are separate events (`PlayRefused`, `ReviewRefused`).
  - The latch re-runs when a preview is dropped. `reconnect_notice` is gated on a displayed view.
  - New loading-screen copy and `loading_error_line`.
  - Hand-review panel state: `GetHandReview`, `open_hand_review`, `choose_hand_review`, `on_hand_review`, `initial_review_hand`, tricks and totals helpers, and copy constants.
  - The docs now say hosts report every failure as `NoAck` (`throwOnError: false`).
- **Fixtures.** The generator is unchanged apart from `sayFinal`. It also writes a new `numbers.json` (13 KB, 246 TS-written numbers) for the `js_f64` differential test. The fixtures total 89 KB.

## Test counts

| Source | TS `it()` | Rust `#[test]` |
|---|---|---|
| `src/lib/game/table.test.ts` | 3 | 3 (unchanged, 1:1) |
| Lane total before fixes | — | 87 |
| Added by these fixes | — | 19 |
| **Total** | **3** | **106** |

Two Rust-only tests (not TS ports) were changed because they pinned non-TS behaviour that finding 2 removes:
- `seats_out_of_range_are_rejected`: a `seat: 7` connect claim now parses and gives `seat_mismatch`.
- `connect_params_round_trip`: `seat: null` is now kept and refused.

No TS-ported assertion was weakened.

## Deviations from TS

None new. Report deviation 8 (store API) is corrected: a refusal is now `PlayRefused`, `play`, `submit_move` and `review_trick` take `now_ms`, and the failure guidance is now "every failure is `NoAck`".

One known limit, documented on `js_f64`: `serde_json::to_value` keeps a number's value but not its JS text. A `Value` number prints with ryu, so byte-exact output needs `to_string` or `to_writer` on the typed struct.

## Open issues

1. **API changes for downstream lanes.** No other crate uses these types yet, so nothing is broken. The lanes that build on them must use the new shapes:
   - SERVER-TABLE: `SubmitMoveRequest.turn_id`, `MoveMsg.turn_id` and `TableConnectParams::claim_error`.
   - DESKTOP: the `StoreEvent` and `StoreEffects` changes, the `now_ms` arguments and the hand-review API.
2. `clientMoveId.slice(0, 64)` truncates by UTF-16 code units in TS. This crate has no helper for it, and SERVER-TABLE must implement it (D27).
3. The earlier open issues in `protocol.md` still apply: the engine field-order request, and the profile types owned by SERVER-PROFILE.
