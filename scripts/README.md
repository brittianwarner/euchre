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
- `or-smoke.ts` — smallest possible live provider check: does the configured
  model return a schema-valid, legal move on both tiers.
- `shot-table.mjs` — Playwright: opens `/play`, gets past the cut so a hand
  exists, and screenshots the 3D table. Needs the dev server on :5199.
  `node scripts/shot-table.mjs <outdir> [port] [w] [h]` — port defaults to 5173

Also `/cardtest` (a dev route) renders `Card.svelte` in isolation — the fastest
way to tell "the card component is broken" from "the scene is placing it wrong".
