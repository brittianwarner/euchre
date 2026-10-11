# Jev decision quality and release evaluation

Evaluated October 11, 2026. Model: `typesafe/jev-1.13`. Prompt version:
`jev-euchre-2`. Conversational models remain outside the game decision path.

## Confirmed problem

The reported table contained two unforced Rita calls recorded as `llm`, rather
than fallback: ordering hearts with no effective hearts and one outside ace,
and ordering an opponent's spade with only the trump ace and no outside ace.
Passing was legal in both cases. Neither was a forced-dealer call or a
left-bower counting issue. The displayed explanation was canned engine text,
not reasoning returned by Jev. The user's third reported occurrence was not
independently confirmed.

The previous request mixed conversation-oriented context and instructions with
strategy. The same ten-scenario live bidding corpus failed 30 of 60 baseline
checks. This demonstrated decision quality problems, not a provider outage.

## Implementation

- Give Jev native structured `state`, focused `instructions`, and explicit legal
  `criteria`. Compute card counts, effective suits, bowers, future dealer pickup
  hands, and secured partnership outcomes in code.
- Separate the suit/pass preference, support for each voluntary suit, and each
  conditional loner judgment. Batch these independent questions in one request;
  each sees shared facts and cannot see another question's answer. Code composes
  Jev's judgments; it does not replace an accepted judgment with a heuristic.
- Preserve the full legal bidding and play sets. A forced dealer cannot pass.
  Personal style affects plausible close calls, not known card facts. Player
  voice text cannot inject instructions into a decision.
- Supply approximate reference continuations: 32 possible bidding deals, or
  up to 24 remaining deals constrained by public played cards, proved voids,
  counts, and dealer pickup. Other actual hands and the private shuffle seed
  never enter this calculation. Jev chooses the actual move.
- Preserve partner's secured winners, distinguish exposed winners, prefer cheap
  equivalent plays, consider useful future leads, and pursue a possible sweep.
  Decision timing is weak evidence, not proof of particular cards.
- Validate every requested answer and its distribution, keep deadlines bounded,
  and log safe request/model/prompt metadata and rejection categories. Invalid
  answers retain the existing legal fallback so the game can finish.

The simulations use the existing expert heuristic for hypothetical future
play, uniform hidden deals, and small samples. Their outcomes are approximate
evidence, not calibrated win probabilities, actual hidden holdings, or solver
truth. Jev confidence is also not a probability of winning the hand.

## Final results

| Evaluation                                                             | Result                                   |
| ---------------------------------------------------------------------- | ---------------------------------------- |
| Unit suite                                                             | 518 tests passed, 29 files               |
| Svelte check                                                           | 0 errors, 0 warnings                     |
| Gateway suite                                                          | 54 tests passed                          |
| Ten bidding scenarios, three personas, four repetitions                | 120/120 accepted                         |
| Fifteen expanded bidding scenarios, three personas, two repetitions    | 90/90 accepted                           |
| Six lead/discard scenarios, three personas, two repetitions            | 36/36 accepted                           |
| Seven partnership scenarios, two repetitions                           | 14/14 accepted                           |
| Fresh whole games, 20 paired seeds                                     | Jev won 28/40; all 40 finished           |
| Additional whole games after final sampler edge fix, four paired seeds | Jev won 5/8; all 8 finished              |
| Local prompt preparation, 487 real projected positions                 | mean 4.56 ms, p95 12.96 ms, max 16.66 ms |

Bidding checks rotate/reverse legal options and reverse every batched question's
criteria. Lead/partnership checks also reverse options. Repetitions test
stability; they are not additional independent scenarios. Expanded bidding
fixtures were used during iteration and are an acceptance corpus, not an
untouched holdout. Lead acceptance sets were corrected to admit strategically
equivalent high-trump leads and comparable alternatives; their final pass rate
cannot be presented as improvement against the earlier narrower gold sets.

The 40-game benchmark swaps Jev's team on identical deals and first-dealer
positions, rotates personas, and varies both house rules. It asserts engine
invariants throughout. Across 465 hands, Jev was euchred on 66/316 calls;
the reference team on 25/149. There were 3,743 accepted model decisions,
1,898 forced decisions, two rejected distributions with legal fallback,
zero illegal applied moves, and zero failed games. Model decision latency was
220 ms mean and 273 ms p95; this measures the actor decision path, not animation
time or full browser turn latency.

The final eight-game edge run had 750 accepted model decisions, 426 forced
decisions, one rejected distribution with fallback, zero illegal moves, and
zero failed games. Safe diagnostics identified its rejection as
`choice_not_maximum`: the selected option was not the highest probability.
The earlier two rejections predated this finer diagnostic, so their exact
violations are unknown. Both full-match runs deliberately exit nonzero when
fallback occurs. Successful game completion is separate from that strict
provider-contract gate; do not report these runs as zero-fallback passes.

The baseline ten-case request averaged 5,489 input tokens. The final request
averaged 4,527 (about 18% less); mean decision latency was 228 ms. Additional
development experiments were discarded after weak calls, excessive loners,
or poor whole-game performance. A rule invariant exposed by simulation was
also corrected: a winning four-point loner can yield a score delta of three
when the scoreboard clamps at the target. This validator issue is not evidence
for the reported production HTTP 500 failures.

This is evidence of competitive play against this specific reference bot, not
mathematical optimality or expert-human acceptance. Possible-deal policy bias,
model option-order sensitivity, and rare malformed model distributions remain
limitations. Continue evaluating independently collected difficult hands before
claiming broader strength.

## Reproduction

Offline commands:

```sh
bunx vitest run
bun run check
bun run test:gateway
bun scripts/ai-context-bench.ts
bun run build
```

Live commands below require `OPENROUTER_API_KEY` and make paid requests. Never
print or commit the key. Synthetic trace output can contain the simulated
acting seat's cards and should stay in local evaluation artifacts.

```sh
bun scripts/ai-bidding-live.ts 4
bun scripts/ai-bidding-live.ts 2 --acceptance
bun scripts/ai-strategy-live.ts 2
bun scripts/ai-partnership-live.ts 2
bun scripts/ai-match-eval.ts 20 --fresh
bun scripts/ai-match-eval.ts 4 --edge
```

The seed labels identify reproducible cohorts. Re-running a cohort is not a new
holdout. Use new frozen cohorts for future comparisons. Full-match failures,
fallbacks, forced moves, legal moves, and completed games are separate counters.
Raw runs for this release were retained locally under `/tmp/euchre-*.jsonl`;
these temporary files are not durable repository artifacts.

## Official guidance and skill

The official [TypeSafe agent skill](https://github.com/typesafe-ai/skills/tree/main/skills/typesafe-ai)
exists and was read and applied for this work; it was not persistently installed.
Guidance used:

- [Jev 1.13 jaggedness](https://docs.typesafe.ai/model-jaggedness/jev-1.13):
  explicit counts, literal wording, focused judgments, and option-order tests.
- [State](https://docs.typesafe.ai/concepts/state) and
  [Choice](https://docs.typesafe.ai/primitives/choice): shared structured facts,
  independent batched questions, criteria, and distribution semantics.
- [Decisions API](https://docs.typesafe.ai/api) and
  [OpenRouter Jev overview](https://openrouter.ai/blog/insights/what-is-jev/):
  native decision requests and separation from text generation.

Local browser checks, deployed revision/readiness, and public browser checks
are distinct release evidence; passing this evaluation does not by itself prove
production deployment or customer acceptance.
