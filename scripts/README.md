# scripts/

- `e2e-play.ts` — headless fuzz harness over the pure rules engine. Drives the
  public API the way an actor does and re-derives every invariant from the rules
  rather than from the implementation. `bun run scripts/e2e-play.ts 300 quiet`
- `ai-live.ts` — live end-to-end AI check. Plays a real hand routing every AI
  seat through `decide()` with its own persona, asserts each move is legal, and
  reports `llm` vs `fallback` and latency. Needs `OPENROUTER_API_KEY`.
- `or-smoke.ts` — smallest possible live provider check: does the configured
  model return a schema-valid, legal move on both tiers.
- `shot-table.mjs` — Playwright: opens `/play`, gets past the cut so a hand
  exists, and screenshots the 3D table. Needs the dev server on :5199.
  `bunx vite dev --port 5199` then `node scripts/shot-table.mjs <outdir>`

Also `/cardtest` (a dev route) renders `Card.svelte` in isolation — the fastest
way to tell "the card component is broken" from "the scene is placing it wrong".
