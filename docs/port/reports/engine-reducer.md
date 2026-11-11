# engine-reducer lane report

Lane: **engine-reducer**, which finishes `crates/euchre-engine`.
Spec: `docs/port/01-engine.md`. TS source of truth: `~/goods/euchre/src/lib/euchre`. Where the two disagree, the TS wins.

## Gates

All three gates pass, run with `CARGO_TARGET_DIR=.cargo-targets/engine-reducer`.

| Gate | Result |
| --- | --- |
| `cargo fmt -p euchre-engine -- --check` | pass |
| `cargo clippy -p euchre-engine --all-targets -- -D warnings` | pass, with the example and every test target included |
| `cargo test -p euchre-engine` | pass, 401 tests, 0 failed, about 12 s in a debug build (387 before the review fixes) |

## What is done

| Item | Contents |
| --- | --- |
| `src/reduce.rs` | Spec functions: `create_game`, `new_game`, `start_hand`, `apply` and `advance`. Fallible twins: `try_create_game`, `try_start_hand`, `try_apply` and `try_advance`. Helpers: `move_id` and `say_for_bid`. Internals: the hash-chained `turn_id` (`mint`), the derived `internal_token`, and `rotate_to_next_hand`. Every transition is here: cut and deal, both bid rounds (throw-in streak capped at `misdeal_limit`), order-up and call (alone or not), dealer discard, play, trick resolve, hand score and game over. |
| `apply` validation ladder | Same order as the TS, step for step. **A:** `game_over` / `stale_turn` / `sitting_out` / turn null / `not_your_turn`. **B:** phase and kind (`bidding_closed` once the hand is past bidding). **C:** `dealer_must_call`, calling the turned-down suit (`illegal_move`), `card_not_in_hand`, `must_follow_suit`. **D:** the `find_legal_move` backstop. `RuleError` carries the legal set exactly where the TS does. |
| `src/invariants.rs` | `assert_invariants`, `assert_legal_non_empty` and `InvariantError`. Each message is byte-identical to the TS message, checked against 26 TS-generated cases. |
| Goldens (`tests/reducer_golden.rs`) | The `createGame` and `advance` goldens, the run-'em and bump `apply` goldens, and the four spec 01 §15 full-match replays (`abc`, `full-10`, `x1`, `nat`). All are exact. |
| Differential (`tests/differential.rs`) | The main parity guarantee; see the next section. |
| `examples/e2e.rs` | Port of `scripts/e2e-play.ts`. Run it with `cargo run -p euchre-engine --example e2e [N] [quiet]`. With 300 games, its stdout is byte-identical to `bun run scripts/e2e-play.ts` except for the `elapsed` line. It exits 1 if any check fails. `tests/e2e.rs` runs it inside the gate (see Review fixes). |

### Differential test

The generator is `crates/euchre-engine/scripts/differential.ts`. It imports the TS engine read-only by absolute path. Set `EUCHRE_TS_SRC` to point it somewhere else. (Rewritten in the review fixes; the first version is described there.)

- **Games: 169 full matches.**
  - 160 games: the full cross product of `gameTo` {5, 10} × `stickTheDealer` × `requireNaturalTrump` × first dealer {drawn, 0, 1, 2, 3} × picker {random, passHeavy, alone, randomAlt}.
  - 5 extra rows: `gameTo` 7, `gameTo` 11, `misdealLimit` 1 and 5 (with a 98%-pass picker so the cap binds), and an explicit `internalToken`.
  - The four §15 replays.
- **Points checked: 32,831.** That is the initial state plus every transition of every match.
- **The point string**, built the same way on both sides:
  - `JSON.stringify({views, steps, legal})`, where `views` is `project`, `steps` is `projectSteps` and `legal` is `legalMoveIds`, for all 4 seats;
  - then `JSON.stringify(state)`;
  - then the rejection battery: for every seat, a stale-`turnId` pass plus a fixed list of candidate moves (every move kind, each held card, cards not held), each recorded as `ok:<new turnId>;` or `<code>:<legal ids>;`.
- **What is compared:**
  - per point, the engine `fnv1a` (32-bit) of the point string, which locates a mismatch to one transition;
  - per game, the SHA-256 of every point string in order. This is what makes the check byte-identical: 231 MB of TS output is compared through 169 SHA-256 digests;
  - the final `turnId` and transition count of every match.
- **Fixture:** `tests/fixtures/differential.json`, 403 KB. The Rust test replays the matches in 8 parallel shards, about 1.5 s.
- **On a mismatch:** the test prints the seed, the transition index and the Rust point string. It then runs `bun run scripts/differential.ts --dump <seed> <index>` to print the TS point string.
- **Regenerate:** `bun run crates/euchre-engine/scripts/differential.ts` from the repo root. The output is deterministic.
- `scripts/invariants.ts` regenerates `tests/fixtures/invariants.json` (29 cases, 45 KB) the same way.

The lane's fixtures add 462 KB (`differential.json` 403 KB, `invariants.json` 45 KB, the two e2e goldens 14 KB). All fixtures in the crate total about 1.04 MB, including engine-core's.

## Test counts

TS counts are runtime counts, so a loop or `it.each` row counts once per generated test.

| TS file | TS tests | Rust file | Rust tests |
| --- | --- | --- | --- |
| `__tests__/reduce.test.ts` | 57 | `tests/reduce.rs` | 57 |
| `__tests__/game.test.ts` | 31 | `tests/game.rs` | 31 |
| `__tests__/house-rules.test.ts` | 6 | `tests/house_rules.rs` | 6 |
| `__tests__/optimistic.test.ts` | 3 | `tests/optimistic.rs` | 3 |
| `__tests__/project.test.ts` | 21 | `tests/project.rs` | 21 |
| **In scope** | **118** | | **118** |
| (none) | | `tests/reducer_golden.rs` | 8 |
| (none) | | `tests/differential.rs` | 12 |
| (none) | | `tests/invariants.rs` | 5 |
| (none) | | `tests/engine_fault.rs` | 6 |
| (none) | | `tests/e2e.rs` | 2 |
| **Lane total** | **118** | | **151** |

The crate total is 401 Rust tests: engine-core's 250 plus this lane's 151.

## Deviations

- **Extra public items:**
  - `try_create_game`, `try_start_hand`, `try_apply` and `try_advance`, with `EngineFault` and `ApplyError`. A TS `invariant()` throw is an `EngineFault`, so a caller such as the table actor can handle it without a panic.
  - The plain spec functions return `Result<_, RuleError>` for rule errors. They panic with `euchre engine invariant violated: {msg}` only on an engine fault, which the TS also treats as a crash. `tests/engine_fault.rs` checks the fault messages against the TS.
- **Extra public item:** `Step::tag()` returns each variant's `"t"` name.
- **`PublicGameView` field order:** the actor metadata fields now come after `winner_team`, which matches the TS table view's wire order. This was the protocol lane's request.
- **`invariants.rs`:**
  - The V11 negative-score checks are omitted, because scores and deltas are `u8` and cannot be negative.
  - V13 is vacuous in the TS too, so it is only a comment.
  - The kitty is a fixed `[Card; 4]`, so the kitty-length check cannot fail.
  - Every other message matches the TS byte for byte.
- **Stronger than the TS:** `tests/game.rs` deep mode also runs `assert_invariants` and `assert_legal_non_empty` at every step. The e2e determinism check compares the whole summary, including the phase and move-id sets, which the TS `JSON.stringify` drops as `{}`.
- **`examples/e2e.rs`:**
  - A first argument that is not a number falls back to 300 games, where the TS gets `NaN` and plays 0 games.
  - The narration keeps the TS quirk that `lastTrickCount` never resets, so `→ X takes it.` lines appear only in hand 1.
  - In a debug build it runs about 5× slower than bun (37 ms per game against 7 ms), because the crate itself is compiled at `opt-level 0`. Use `--release` for timing.

## Not ported 1:1 (reason)

| TS test or assertion | Rust form | Reason |
| --- | --- | --- |
| reduce "never mutates a deep-frozen state" | snapshot the JSON of the live input state, call `apply`/`advance`, then compare that same input with the snapshot | `deepFreeze` / `Object.isFrozen` do not exist in Rust. The borrow checker already forbids mutation through `&GameState`. `Object.isFrozen(final)` has no Rust form. |
| project "never mutates a deep-frozen state", and the `projectSteps` freeze | clone and compare | Same reason. |
| `toBe` identity: `advance` identity, optimistic identity, `projectSteps` returning the same reference, `legalMoves` returning fresh arrays | value equality, or a check that the results mutate independently | Rust has no object identity for owned values. |
| `isRuleError(e)` / `expect(() => ...).toThrow(RuleError)` | `Result::Err(RuleError)` with a code check | Rule errors are values, not exceptions. |
| house-rules JS-array hand patches | the whole `hands[1]` is replaced | `Hands` is `[Vec<Card>; 4]`. |

No TS test was dropped. Each one in scope has a `#[test]` with the same intent and assertions that are equal or stricter.

## Open issues

- None blocking.
- **`docs/port/requests/protocol-view-field-order.md` is resolved.** The protocol lane asked the engine to move the five actor-metadata fields of `PublicGameView` to after `winner_team`. This lane made that change in `src/types.rs`.
  - Pure `project()` output is unchanged, because the five fields are skipped when `None`. The differential still passes.
  - `cargo test -p euchre-protocol` still passes (87 tests).
  - The protocol lane's `WireView` reorder helper is now a no-op. It can stay or go. Removing it is that lane's call. The request file itself was left as is, since it belongs to that lane.

## Review fixes

Eight findings from the adversarial review. Each one was checked against the TS source and the Rust tests before any change. Gates after the fixes: fmt pass, clippy pass, `cargo test -p euchre-engine` pass with 401 tests, 0 failed.

| # | Finding | Verdict | What changed |
| --- | --- | --- | --- |
| 1 | `never_mutates_a_deep_frozen_state_across_a_whole_match` compared a private clone with its own snapshot, so it could never fail. | **Fixed** | `tests/reduce.rs`: `on_state` now snapshots the JSON of the live state before it goes to `apply`/`advance`. `on_transition` runs after that call with the same input, re-serialises it and compares it with the snapshot. It also asserts every transition was checked (`checked == states - 1`) and that the result differs from the input. `Object.isFrozen(final)` has no Rust form and is listed under "Not ported 1:1". |
| 2 | The differential replayed accepted moves only. Rejection codes and the `RuleError` legal sets were outside the gate, except for 4 codes. | **Fixed** | Every point string now ends with a rejection battery. For each seat it runs a stale-`turnId` pass plus a fixed list of candidates, and records `ok:<turnId>` or `<code>:<legal ids>`. That covers 32,831 points, the game-over state included. A new test, `the_battery_reaches_every_rule_code_and_game_over_rejects_everything`, asserts the battery produces all 9 `RuleCode`s and that a finished match returns only `wrong_phase` with no `ok`. The reviewer's probe found no mismatch. The gated version found none either. |
| 3 | `check_replay` did not assert invariants at each step, and did not check the §15 `firstDealer` column. | **Fixed** | `tests/reducer_golden.rs`: `assert_invariants` runs on the initial state and after every transition. `assert_legal_non_empty` runs after every transition. Each of the 4 rows carries `want_first_dealer` (abc 1, full-10 1, x1 2, nat 0), checked every step and at the end. |
| 4 | In the config matrix, the fixed first dealer was tied to `gameTo` and `stick`. `gameTo` 7/11, `misdealLimit` and `internalToken` were never used. The coverage test did not check the full cross product. | **Fixed** | `scripts/differential.ts` now builds the full cross product: 8 configs × 5 first dealers (drawn, 0, 1, 2, 3) × 4 pickers, 160 games. It adds rows for `gameTo` 7, `gameTo` 11, `misdealLimit` 1, `misdealLimit` 5 and an explicit `internalToken`. `fixture_covers_the_required_combinations` asserts every (config, dealer, picker) tuple, the extra `gameTo` values, both `misdealLimit` values, the token row and a default-config row. A new test, `the_misdeal_limit_rows_reach_their_cap`, checks that the misdeal rows actually reach their cap (1 and 5). An earlier 95% pass rate never got past a streak of 1, so the rate is now 98%. |
| 5 | The differential compared 32-bit `fnv1a` hashes but claimed byte-identical JSON. | **Fixed** | Each game now also records the SHA-256 of all its point strings in order (`sha2` and `hex` are dev-dependencies, from the workspace). The Rust test recomputes it. The per-point `fnv1a` is kept only to locate a mismatch. I checked by hand that a corrupted digest fails with the SHA message and a corrupted point hash fails with the seed, the index and the TS dump. I did not store full JSON samples, because the SHA-256 covers every byte (231 MB) and keeps the fixture at 403 KB. |
| 6 | `examples/e2e.rs` was outside the gate. | **Fixed** | The example now writes through a `say!` macro into a `&mut dyn fmt::Write` sink, and `main` streams it to stdout. Its new `pub fn run(args, out) -> bool` replaces `process::exit` inside the harness; `main` still exits 1 on failure. `tests/e2e.rs` includes the example with `#[path]` and runs it two ways: narrated with 20 games, and quiet with 20 games. Each run must report success, and its output minus the `elapsed` line must equal the TS goldens `tests/fixtures/e2e-20.txt` and `e2e-20-quiet.txt`, generated with `bun run scripts/e2e-play.ts 20 [quiet] \| grep -v '^elapsed '`. A 300-game run still matches TS except for `elapsed`. |
| 7 | The invariant fixture missed V9 "the sitting seat won a trick", the V18 `trickLog` and `bids` lengths, and V16. | **Fixed for V18, rejected for V9's second branch and V16** | New fixture cases: `V18 trickLog length` (an empty sixth trick, giving "trickLog has 6 entries") and `V18 bids length` (9 bids, giving "bids has 9 entries"). Both messages match TS. **V9 "sitting seat won a trick" cannot be reached in TS either.** It needs `next.plays[0].seat === prev.winnerSeat === sittingSeat`, and V8 runs first over the same `allTricks` and rejects any play by the sitting seat. The new case `V9 sitting seat leads the next trick (V8 fires first)` shows this: TS and Rust both give "V8 violated: sitting seat 3 played in trick 1". **V16 cannot be reached from any state.** It depends only on the constant `DECK` and the trump suit, and all four suits pass. The new test `the_v16_trump_census_holds_for_every_trump` sets each trump on a valid state and asserts `Ok`. The new test `covers_the_v18_length_branches_and_the_v8_over_v9_precedence` pins all three messages. |
| 8 | `play_match` panicked inside a hook when `maker_seat` was missing. TS collects every failure. | **Fixed** | `tests/game.rs`: a scored hand with no maker is now noted as a violation, collected with the rest and asserted empty at the end, as in TS. No other hook panics. No regression test was added, because the case needs a broken engine to trigger. |

TS test count is unchanged at 118. This lane's Rust tests went from 137 to 151: `differential.rs` from 2 to 12 (8 replay shards plus 4 coverage tests), `invariants.rs` from 3 to 5, and `e2e.rs` is new with 2.

