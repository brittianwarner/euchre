# scripts/

- `e2e-play.ts` — headless fuzz harness over the pure rules engine. Drives the
  public API the way an actor does and re-derives every invariant from the rules
  rather than from the implementation. `bun run scripts/e2e-play.ts 300 quiet`
- `ai-live.ts` — live end-to-end AI check. Plays a real hand routing every AI
  seat through `decide()` with its own persona, asserts each move is legal, and
  reports `llm` vs `fallback` and latency. Needs `OPENROUTER_API_KEY`.
- `ai-partnership-live.ts` — opt-in Jev strategy regressions through the production
  actor ladder: secured partner tricks, vulnerable leads, bowers, cheap winners,
  and preserving a sweep. `bun scripts/ai-partnership-live.ts 2` also reverses
  option order. Requires `OPENROUTER_API_KEY` and makes paid API calls.
- `ai-bidding-live.ts` — bidding/pickup/loner regressions across all three personas,
  rotating legal choices and reversing every batched question's criteria.
  `bun scripts/ai-bidding-live.ts 4`; add `--acceptance` for the expanded corpus.
- `ai-strategy-live.ts` — lead/discard acceptance checks across all personas.
  `bun scripts/ai-strategy-live.ts 2` tests both option orders.
- `ai-match-eval.ts` — paired whole games against the expert reference heuristic,
  with team swaps, varied house rules, invariants, and separate model/forced/
  fallback/illegal/failed-game counters. `bun scripts/ai-match-eval.ts 20 --fresh`.
  Exits nonzero on any fallback, illegal move, or failed match. Optional `--trace`
  and `--trace-bids` output synthetic seat-visible evaluation evidence.
- `ai-context-bench.ts` — offline timing of prompt preparation on real engine
  positions. `bun scripts/ai-context-bench.ts`. No paid API calls.

The bidding, strategy, and match probes require `OPENROUTER_API_KEY` and make paid
requests. Results, method, official guidance, and limitations are documented in
[`docs/JEV-EVALUATION.md`](../docs/JEV-EVALUATION.md).

- `or-smoke.ts` — smallest possible live provider check: does the configured
  model return a schema-valid, legal move on both tiers.
- `shot-table.mjs` — Playwright: opens `/play`, gets past the cut so a hand
  exists, and screenshots the 3D table. Needs the dev server on :5199.
  `node scripts/shot-table.mjs <outdir> [port] [w] [h]` — port defaults to 5173

Also `/cardtest` (a dev route) renders `Card.svelte` in isolation — the fastest
way to tell "the card component is broken" from "the scene is placing it wrong".
