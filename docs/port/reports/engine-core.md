# engine-core lane report

Lane: **engine-core**, the foundation of `crates/euchre-engine`.
Spec: `docs/port/01-engine.md`. TS source of truth: `~/goods/euchre/src/lib/euchre`.

## Gates

All three gates pass, run with `CARGO_TARGET_DIR=.cargo-targets/engine-core`.

| Gate | Result |
| --- | --- |
| `cargo fmt -p euchre-engine -- --check` | pass |
| `cargo clippy -p euchre-engine --all-targets -- -D warnings` | pass |
| `cargo test -p euchre-engine` | pass, 250 tests, 0 failed |

## What is done

These modules are complete. The crate has no `unsafe`, no `unwrap`/`expect` in library paths, and no `todo!()`. Every public item has a doc comment.

| Module | Contents |
| --- | --- |
| `types.rs` | Every engine type, with exact serde per 01 §2. Unions are tagged on `"t"`. `GamePhase` uses explicit renames. TS `?` fields are skipped when `None`. `hands` serializes as the `"0".."3"` object. `Card` serializes as its 2-char id. `Seat` and `Team` are range-checked `u8` newtypes. |
| `cards.rs` | `DECK` and `CARD_IDS` as const `[Card; 24]`, plus effective suit, trump and plain ranks, bowers and colour mates. |
| `seats.rs` | Seat arithmetic, roles, bidding order and active-seat helpers. |
| `deal.rs` | `fnv1a` over UTF-16 code units, `mulberry32` with JS uint32 semantics, shuffle, cut (JS slice semantics on degenerate lengths), deal plan, `deal_from`, `deal_hand`, `dealt_packets`, `pick_first_dealer`, `to_base36`. |
| `legal.rs` | Legal plays, `why_illegal` (same messages, em dash included), `legal_moves` (stick-the-dealer and the natural-trump house rule), and card and suit names. |
| `trick.rs` | `card_value`, `trick_winner` and in-progress trick helpers. |
| `score.rs` | `score_hand`, throw-in outcome, `apply_delta`, game-over and barn helpers. |
| `optimistic.rs` | `apply_optimistic` per §13, plus `OPTIMISTIC_FIELDS`. |
| `project.rs` | `project` and `project_steps`, ported from `index.ts:247-321`. |
| `lib.rs` | Re-exports the full §14 surface. |

`reduce.rs` and `invariants.rs` hold types and signatures only, as the lane brief asked: `ApplyResult`, `ApplyOptions`, `RuleError`, `GameInit`, `move_id` and `InvariantError`.

### Differential fixtures

`scripts/gen-fixtures.ts` imports the real TS engine read-only and writes two fixtures. Together they are 568 KB.

- **`tests/fixtures/deal.json`** holds the RNG and deal vectors:
  - fnv1a, base36, mulberry32 draws, deal plans and `cutAt`.
  - Ten seeds by four hands each of shuffle, cut, run and bump deals, including non-ASCII and emoji seeds.
- **`tests/fixtures/trace.json`** holds 71 `GameState`s sampled from three real matches:
  - Every phase except the transient `deal`, plus loners and sitting seats.
  - Throw-ins with the natural-trump rule on and stick-the-dealer off.
  - Scoring and game over.

`tests/trace.rs` checks these, byte for byte with key order included:

- The `GameState` JSON round-trips.
- `project` matches TS for all 4 seats.
- `apply_optimistic` matches TS for every legal move of the actor.
- `Step[]` round-trips, and `project_steps` matches TS for every seat.

To regenerate, run this from the repo root:

```sh
bun run crates/euchre-engine/scripts/gen-fixtures.ts
```

Set `EUCHRE_TS_SRC` to override the TS path.

## Test counts

The in-scope TS files have 209 static `it(` calls and 220 tests at runtime. The `score.test.ts` TABLE loop generates 12 tests.

| File | TS tests | Rust tests | Notes |
| --- | --- | --- | --- |
| cards | 37 | 37 | 36 ported, 1 frozen-object test omitted, 1 Rust-only card id/serde test added |
| seats | 38 | 39 | all ported (frozen half dropped), plus 1 serde range test |
| trick | 43 | 43 | all ported |
| score | 34 static / 45 runtime | 45 | 12 table rows via `check_row(i)` |
| legal | 57 | 58 | all ported, plus 1 printed-suit natural-trump test |
| golden (§15) | n/a | 21 | hardcoded §15 vectors plus differential checks against `deal.json` |
| trace | n/a | 7 | differential checks against `trace.json` |
| **Total** | **209 static / 220 runtime** | **250** | |

## Not portable (approximated or dropped)

These checks rely on JavaScript object behaviour that Rust does not have. No assertion was weakened beyond what that requires.

- **`Object.isFrozen` checks.** The cards test "DECK and SUITS and RANKS are frozen" is omitted. The frozen halves of the SEATS and SUIT_NAME tests are dropped. Rust consts and arrays cannot be mutated, so these checks have no equivalent.
- **Reference identity.** `CARD_IDS toBe DECK` and the `not.toBe` fresh-array and fresh-object checks have no Rust meaning. They became value equality, plus mutation-independence checks where TS asserted a fresh copy.
- **`deepFreeze` purity.** Replaced with clone-and-compare: the input is snapshotted before the call and compared after it.

## Not ported (reducer lane)

These tests need `createGame`, `apply` or `advance`:

- `__tests__/reduce.test.ts` (57)
- `__tests__/game.test.ts` (14)
- `__tests__/house-rules.test.ts` (5)
- `__tests__/optimistic.test.ts` (3). `apply_optimistic` itself is covered differentially by `trace.rs`.
- `__tests__/project.test.ts` (21). `project` and `project_steps` are covered differentially by `trace.rs`, and redaction is covered by `views_never_carry_foreign_cards` and `project_steps_never_leaks_a_foreign_dealt_packet`.
- The createGame goldens in §15 (`turnId`, full-match replays) and `scripts/e2e-play.ts`. The `deriveToken` vector *is* checked here, through `fnv1a` and `to_base36`.

## Deviations from the spec or TS

Where the spec and TS disagreed, TS won.

- **Errors are `Result`s, not throws.**
  - `trick_winner` returns `EmptyTrickError` on an empty trick.
  - `deal_from`, `deal_hand` and `assert_deck_order` return `DealError`, whose `Display` matches the TS messages.
  - `RuleError` and `InvariantError` are error types.
- **Params structs** for functions with several arguments: `ScoreHandParams`, `WhyIllegalParams` and `DealHandParams`.
- **`bidding_order` returns `[Seat; 4]`** instead of a fresh `Vec`.
- **`score_hand` delta saturates at 0** when `score > game_to`. TS would go negative, but V11 makes that unreachable. `apply_delta` also saturates.
- **`Suit` and `Rank` have no `Ord`.** `Card`'s `Ord` is lexicographic on the 2-char id, which matches JS `.sort()`.
- **`AiDecisionSource`** is the type name, with `AIDecisionSource` as an alias.
- **`GameInit.cfg`** is `EngineConfigOverrides`, a partial config.
- **Name tables** (`SUIT_NAME`, `SUIT_NOUN`, `RANK_NAME`) are arrays indexed by discriminant, with `suit_name()`, `suit_noun()` and `rank_name()` accessors.
- **Extra public items:**
  - `color_mate`, `to_base36`, and the `Mulberry32` struct, which also implements `Iterator`.
  - `DealtHands`, `DealError`, `EmptyTrickError`, `OutOfRange`, `ParseCardError`, `ParseEnumError`, `SchemaV1`, `EngineConfigOverrides`, `TableRules`.
  - `Move::{id, as_bid, is_bid, tag}` and `Bid::{id, is_alone}`.
- **`project` sets the actor-only `PublicGameView` fields to `None`.** These are `players`, `observations`, `review_tricks`, `rules` and `awaiting_trick_review`, and TS `project` also leaves them absent.

## Open issues

- **Reducer functions are absent.** `create_game`, `start_hand`, `apply`, `advance`, `say_for_bid` and `new_game` belong to the reducer lane. Their supporting types are already in `reduce.rs`.
- **Invariant checks are absent.** `assert_invariants` and `assert_legal_non_empty` belong to the reducer lane. `InvariantError` exists.
- **Full-match goldens are not checked yet.** The createGame and turnId replay goldens from §15 can only be checked once the reducer lands. `trace.json` already holds post-transition states and steps that the reducer lane can reuse as expected outputs.
