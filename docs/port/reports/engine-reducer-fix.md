# engine-reducer-fix lane report

Lane: **engine-reducer-fix**. It fixes the adversarial-review findings on the engine-reducer lane in `crates/euchre-engine`. The per-finding verdicts and evidence are in the "Review fixes" section of `docs/port/reports/engine-reducer.md`.

## Gates

All three gates pass, run with `CARGO_TARGET_DIR=.cargo-targets/engine-reducer`.

| Gate | Result |
| --- | --- |
| `cargo fmt -p euchre-engine -- --check` | pass |
| `cargo clippy -p euchre-engine --all-targets -- -D warnings` | pass |
| `cargo test -p euchre-engine` | pass: 401 tests, 0 failed, about 12 s wall in a debug build |

## What is done

7 findings are fixed. Finding 7 is partly fixed; its V9 second branch and V16 parts are rejected with evidence. There are 8 findings in all.

| File | Change |
| --- | --- |
| `tests/reduce.rs` | The frozen-state test now compares the live input state before and after `apply`/`advance` (finding 1). |
| `scripts/differential.ts`, `tests/differential.rs`, `tests/fixtures/differential.json` | The fixture now has 169 games: the full cross product of config × first dealer (drawn, 0, 1, 2, 3) × picker, plus rows for `gameTo` 7 and 11, `misdealLimit` 1 and 5, and an explicit `internalToken`. Each of the 32,831 points also runs a rejection battery, which covers every `RuleCode` and its legal set. Each game also gets a SHA-256 over all its point strings. The replay runs in 8 parallel shards. Findings 2, 4 and 5. |
| `tests/reducer_golden.rs` | The §15 replays assert invariants and a non-empty legal set at every step. They also check the `firstDealer` column (finding 3). |
| `examples/e2e.rs`, `tests/e2e.rs`, `tests/fixtures/e2e-20*.txt` | The harness now writes to a sink through `pub fn run`. The gate runs it narrated and quiet, and compares both outputs with TS goldens (finding 6). |
| `scripts/invariants.ts`, `tests/invariants.rs`, `tests/fixtures/invariants.json` | Adds cases for the V18 `trickLog` and `bids` length branches. Adds a V8-over-V9 precedence case and a test that V16 holds for every trump suit (finding 7). |
| `tests/game.rs` | A missing maker is now collected as a violation instead of panicking (finding 8). |
| `Cargo.toml` (crate) | Adds the `sha2` and `hex` workspace dev-dependencies. |

## Test counts

| | Count |
| --- | --- |
| TS tests in scope (unchanged) | 118 |
| This lane's Rust tests, before → after | 137 → 151 |
| Crate total | 401 (engine-core 250 + reducer 151) |

## Fixture budget

The lane's fixtures total 462 KB, under the 2 MB limit:

- `differential.json`: 403 KB
- `invariants.json`: 45 KB
- the two e2e goldens: 14 KB

## Deviations from TS

None in engine behaviour. The rewritten differential found no mismatch, and neither did the rejection battery or the e2e goldens.

## TS assertions that cannot apply in Rust

- **`Object.isFrozen(final)`** in the reduce frozen test. Rust has no freezing, and `&GameState` already forbids writes through the input.
- **V9 "the sitting seat won a trick"** cannot be reached in TS either. V8 fires first, as the new fixture case shows.
- **V16** cannot be reached from any state. It depends only on the constant `DECK` and the trump suit.

## Open issues

None. One note: `examples/e2e.rs` now writes through `fmt::Write`. `main` passes a sink that prints straight to stdout, so the CLI behaviour is unchanged.
