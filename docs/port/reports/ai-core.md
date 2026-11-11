# ai-core lane report

Lane: **AI-CORE** (Lane AI), the pure, deterministic half of `crates/euchre-ai`.
Contract: `docs/port/ARCHITECTURE.md` (D8, D23, D24, D27, D29), then `docs/port/04-ai.md`. Where the TS code and the spec disagree, the TS code wins.
TS source of truth (read-only): `~/goods/euchre/src/lib/ai/*` and `src/lib/actors/ai-seat/*`.

## Gates

All three gates pass, run with `CARGO_TARGET_DIR=.cargo-targets/ai`.

| Gate | Result |
| --- | --- |
| `cargo fmt -p euchre-ai -- --check` | pass |
| `cargo clippy -p euchre-ai --all-targets -- -D warnings` | pass |
| `cargo test -p euchre-ai` | pass, 74 tests, 0 failed (about 25 s in debug; the differential takes about 24 s of that) |

Library code has no `unsafe`, no `unwrap`/`expect` and no `todo!()`. Every public item has a doc comment. The engine and protocol crates are untouched, as is the root `Cargo.toml`. No change requests were needed.

`Cargo.lock` was updated by cargo itself. It holds entries for crates from other lanes; nothing was edited by hand.

## Modules

| Module | Port of |
| --- | --- |
| `heuristic` | `ai/heuristic.ts`. Has 4 presets through `HeuristicOptions` and `from_persona`. The table actor shares it (D8). |
| `notation` | `ai/notation.ts`: `encode_for_llm`, `public_state_only`, `estimate_tokens`. |
| `bid_rollouts`, `play_rollouts`, `rng` | `ai/bid-rollouts.ts`, `ai/play-rollouts.ts`, and `randomFor`, which hashes the first UTF-16 unit of each code point. |
| `partnership`, `table_context` | `ai/partnership.ts`, `ai/table-context.ts` |
| `decision_context` | `ai/decision-context.ts` and the Jev request body from `ai/jev.ts` |
| `schema` | `ai/schema.ts`, as a validator |
| `screen::ai`, `screen::seat` | `ai/screen.ts` (including the phrasebook) and `actors/ai-seat/screen.ts` |
| `config`, `types` | `ai/config.ts`, `ai/types.ts` |
| `js` | JS semantics: UTF-16 length and prefix, `Math.round`, `toFixed`, `\s` trim, and `JSON.stringify` with JS number formatting |
| `ai_seat::{state, memory, difficulty, ladder, status}` | The pure parts of `actors/ai-seat/*`: state, memory, difficulty, ladder, budget and status |
| `lib` | Re-exports, plus `heuristic_outcome`, the heuristic rung of the ladder |

## Public API (key signatures)

```rust
// heuristic
pub fn rank_moves(params: RankParams<'_>) -> Vec<RankedMove>;
pub fn top_move(params: RankParams<'_>) -> Option<LegalMoveId>;
impl RankParams { new(view), with_opts(HeuristicOptions), with_legal(&[LegalMove]), with_hand(&[Card]) }
pub fn from_persona(&PersonaConfig) -> HeuristicOptions;
pub fn void_reads(&PublicGameView) -> [Vec<Suit>; 4];
pub fn hand_strength(HandStrengthParams) -> f64;  pub fn best_discard(&[Card], Suit) -> Option<Card>;
// rollouts
pub fn bid_rollouts(BidRolloutParams) -> Result<Vec<(LegalMoveId, BidRollout)>, RolloutError>;
pub fn play_rollouts(PlayRolloutParams) -> Result<Vec<(LegalMoveId, PlayRollout)>, RolloutError>;
pub fn possible_remaining_deals(PlayRolloutParams) -> Result<Vec<GameState>, RolloutError>;
// evidence
pub fn analyze_partnership(PartnershipParams) -> Option<PartnershipAnalysis>;
pub fn table_context(&PublicGameView) -> TableContext;
pub fn holding_facts(&[Card], Suit) -> HoldingFacts;
pub fn build_decision_question(DecisionQuestionParams{view, legal, persona}) -> Result<DecisionQuestion, RolloutError>;
pub fn jev_request_body(&DecisionQuestion) -> String;   // byte-identical to TS JSON.stringify
pub const DECISION_PROMPT_VERSION: &str = "jev-euchre-2"; pub const JEV_MODEL: &str = "typesafe/jev-1.13";
// notation
pub fn encode_for_llm(EncodeParams{view, legal, ranking, notes}) -> String;
pub fn public_state_only(&PublicGameView) -> String;  pub fn estimate_tokens(&str) -> u64;
// schema
pub fn decision_schema(DecisionSchemaParams{kind, ids}) -> Result<MoveSchema, EmptyCandidateSetError>;
impl MoveSchema { pub fn validate(&self, &DecisionObject) -> Result<(), SchemaViolation> }
// screens
pub fn screen::ai::screen_banter(&str, &PublicGameView) -> ScreenResult;
pub fn screen::ai::screen_text(ScreenTextParams) -> ScreenResult;
pub fn screen::ai::clean_rationale(CleanRationaleParams) -> String;
pub fn screen::ai::phrasebook_line(&str, i64) -> &'static str;
pub fn screen::seat::{screen_banter(&str, &[Card]) -> Option<String>, clean_rationale, sanitize_persona, public_only_view, publicly_named_cards};
// ai-seat (pure)
pub fn create_state(CreateStateParams{key, input}) -> Result<AiSeatState, SeatKeyError>;
pub fn absorb_public_state(&mut AiSeatHandMemory, &PublicGameView);
pub fn remember_hand(RememberParams) -> Option<String>;  pub fn episode_for(EpisodeParams) -> Option<AiSeatEpisode>;
pub fn moment_salience(&PublicGameView, f64) -> f64;  pub fn situation_line(&PublicGameView, Seat) -> String;
pub fn narrow_by_difficulty(NarrowParams) -> Vec<LegalMove>;  pub fn turn_rng(&str, Seat) -> Mulberry32;
pub fn ranking_for(&AIDecideRequest, &PersonaConfig) -> Vec<RankedMove>;
pub fn heuristic_choice(&[RankedMove], &[LegalMove]) -> Option<HeuristicChoice>;
pub fn budget_for(BudgetParams{nominal, deadline_at, now}) -> f64;
pub fn record_decision(RecordDecisionParams);  pub fn apply_usage(&mut AiSeatState, LlmUsage);
pub fn get_status(&AiSeatState) -> AiStatus;  pub fn get_persona(&AiSeatState) -> PersonaView;
pub fn heuristic_outcome(HeuristicOutcomeParams{legal, ranking, set}) -> Option<DecideOutcome>;
```

## Differential test (mandatory)

- **Generator:** `crates/euchre-ai/scripts/differential.ts`. It imports the real TS modules read-only. Run it from the repo root with `bun run crates/euchre-ai/scripts/differential.ts`. To print the TS side of one point, use `--dump <seed> <index> [part]`.
- **Fixture:** `tests/fixtures/differential.json`, 415 KB. It holds 28 games and **6106 decision points**. Each point records the FNV-1a of six parts:
  - `rank`: the heuristic across 4 presets
  - `notation`
  - `bid` rollouts
  - `play` rollouts
  - `jev`: the full request body
  - `memory`: memory absorb, `situationLine` and `momentSalience`
- **Per-game checks:** a SHA-256 over all of the game's parts, and the final `turnId`.
- **How the games vary:** they rotate configs, dealers and pickers (random, heuristic, mixed, passHeavy, alone). Two thirds of the games also carry `rules` and a 40-entry observation log with human-observed and computer-paced timing.
- **Test:** `tests/differential.rs` replays every game in 8 shards. `fixture_covers_every_decision_kind` pins the coverage minimums below.

| Coverage | Points |
| --- | --- |
| cutting | 286 |
| bid_round_1 | 802 |
| bid_round_2 | 324 |
| dealer_discard | 179 |
| trick_play | 4515 |
| sitting seat (loner in play) | 1154 |
| loner question | 1051 |
| support question | 1032 |
| stick-the-dealer forced bid | 19 |
| natural-trump rule | 444 |
| with observations | 4099 |

Every hash and every SHA-256 matches. That proves byte parity, including the typographic ’ and JS number text.

## Fixtures

- **Generator:** `crates/euchre-ai/scripts/fixtures.ts` (`bun run crates/euchre-ai/scripts/fixtures.ts`). It writes `bidding.json`, `bidding-acceptance.json`, `partnership.json` and `strategy.json`, holding **10, 15, 7 and 6** cases.
- **Case format:** `{name, view, expected, rank, jev}`. `rank` and `jev` are the TS outputs, and the Jev body is built with persona Rita.
- **Test:** `tests/fixtures.rs` checks the counts, legality, rank-line parity and Jev-body parity. It also checks that the Rust ports of `playView` and `bidView` (in `tests/common/mod.rs`) rebuild the TS fixtures exactly.
- **Total size:** all fixtures together come to about 1 MB.

## Tests: TS vs Rust

| TS file | TS cases | Rust file | Rust tests | Not ported (needs a scripted Jev model) |
| --- | --- | --- | --- | --- |
| bid-rollouts.test.ts | 5 | bid_rollouts.rs | 5 | none |
| play-rollouts.test.ts | 5 | play_rollouts.rs | 5 | none |
| banter-safety.test.ts | 7 (6 `each` + 1) | banter_safety.rs | 7 | none |
| partnership.test.ts | 25 (15 evidence incl. 4 `each`; 6 `each` + 2 conservation; 2 Jev) | partnership.rs | 18 (the 6 conservation cases run in one loop) | "Jev integration" (2) |
| table-context.test.ts | 8 | table_context.rs | 6 | `buildLayers` conversational prompt; `personasFromSettings` snapshot |
| decision-context.test.ts | 9 | decision_context.rs | 6 | "Jev owns bid and loner choices" (3) |
| jev.test.ts | 6 | none | 0 | all 6: HTTP client, cancellation, retry and response validation |
| none | none | fixtures.rs | 8 | none |
| none | none | differential.rs | 9 | none |
| unit (`src/`) | none | rng, schema, screen | 10 | none |

The table-context observations test runs as a pure variant. It checks the state from `build_decision_question`, which is exactly the state Jev receives. The LLM stage still owes the `callModel` path for that test.

## Deviations, with reasons

1. **UTF-16 caps never split a surrogate pair.** `utf16_prefix` stops one unit short instead of emitting a lone surrogate, which Rust `str` cannot hold. Output is the same for any text without astral characters at the boundary.
2. **Errors are values, not throws.** Rollouts and `build_decision_question` return `Result<_, RolloutError>`. `heuristic_choice`, `heuristic_outcome` and `top_move` return `Option`. `seat_from_key` and `create_state` return `Result<_, SeatKeyError>`, and `move_id_enum` returns `EmptyCandidateSetError`. In each case TS would throw.
3. **Rollout maps are ordered `Vec<(id, value)>`**, in legal order, which matches the key order of the TS object. Tests that used `toEqual` compare them without regard to order.
4. **A play id that cannot be parsed scores 0, with an empty why,** in the heuristic. TS would produce `NaN` (shown as 0) and keep the why of whichever `base()` branch the garbage card falls into. Engine-made ids never take that path, and the ranking order matches.
5. **Schema is a validator, not zod.** `MoveSchema::validate` enforces the id enum, the `why` cap (110 for bids, 90 for plays) and confidence in 0..1. Rendering the JSON schema for a provider belongs to the LLM stage.
6. **Screen regexes fail closed.** If a pattern fails to compile, the text is rejected. A unit test pins that every pattern compiles. Patterns use ASCII `\b` and case folding, plus a JS `\s` class, to match V8.
7. **`phrasebook_line` takes `n: i64`.** It uses the JS `%` sign semantics, so -6 maps to index 1.
8. **Fixture views are put in projection key order.** The hand-built TS fixture tricks list `winnerSeat` before `plays`. The Jev body copies tricks verbatim, and the typed Rust view always serializes in engine projection order. So the generator reorders `trick` and `trickLog` before recording the TS outputs. Engine-made views, which are what production uses, already have this order; the differential proves it.

## What the LLM stage must do

- **Jev HTTP client** (`ai/jev.ts`):
  - Send the body from `jev_request_body` to `JEV_MODEL`.
  - Support cancellation and do not retry provider errors.
  - Classify failures without keeping response bodies.
  - Validate the response in TS order: `missing_answer`, `unknown_choice`, `option_keys`, `probability_total` (|sum − 1| > 0.02), then `choice_not_maximum` (with a 1e-6 tolerance).
  - Resolve the answer through `DecisionQuestion.{choices, alone_choices, bid_checks}`: a support answer of pass becomes the pass move, and a loner answer of alone becomes the alone twin.
- **`callModel` and `runLadder`, async** (`ai/decide.ts`, `actors/ai-seat/decide.ts`):
  - Rung order and escalation.
  - `budget_for` deadlines.
  - The `jev_decision` log, with no private fields.
  - Fall back to `heuristic_outcome`.
  - Record results with `record_decision` and `apply_usage`.
- **Banter writer and prompt layers:**
  - `buildLayers`, using `sanitize_persona` with a nonce.
  - Run output through `screen::ai::screen_banter`, `screen::seat::screen_banter` and `clean_rationale`.
  - Use `phrasebook_line` as the fallback.
- **`personasFromSettings`**, a snapshot that never aliases settings.
- **Port the 13 Jev-dependent tests listed above** against a scripted mock model. No network and no real keys.
