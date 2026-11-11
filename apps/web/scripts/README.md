# scripts/

The browser scripts need a running app: either the Rust server serving the web
build, or `vite dev` proxying to it (see `../README.md`). They use Playwright's
Chromium (`bunx playwright install chromium` once).

- `e2e-browser.mjs` — the browser proof against the Rust backend. Plays one
  full match: home (no nav, manifest read), identify, settings save, /new,
  a game started with the home page's Play now, bidding, every trick
  through the review gate, hand results, the hand-review dialog, a reload
  mid-hand (resume), the phone layout, game over, `/games` and a match detail
  page, then switches profile. Asserts no page errors and no 4xx/5xx responses,
  and prints a JSON summary. With the server's fast tempo
  (`EUCHRE_FAST_TEMPO=1`) a match takes about a minute; at production tempo it
  takes about seven.

  ```sh
  EUCHRE_URL=http://127.0.0.1:3000 E2E_SHOTS=/tmp/euchre-e2e E2E_LABEL=run1 \
    bun scripts/e2e-browser.mjs
  ```

  `E2E_MATCH_BUDGET_MS` caps the match (default 15 minutes).

- `e2e-play.ts` — headless fuzz harness over the pure rules engine
  (`src/lib/euchre`). Drives the public API the way the table does and
  re-derives every invariant from the rules rather than from the
  implementation. `bun run scripts/e2e-play.ts 300 quiet`
- `shot-table.mjs` — opens `/play`, gets past the cut so a hand exists, and
  screenshots the 3D table. `bun scripts/shot-table.mjs <outdir> [port] [w] [h]`
  (port defaults to 5173).
- `browser-review.mjs`, `touch-smoke.mjs`, `shot-portrait-tmp.mjs` — quick
  layout and touch checks against a dev server on a fixed port.
- `raster-card-art.mjs`, `fetch-card-art.py`, `build-table.py` — regenerate the
  card art and the table assets in `static/art`.

Also `/cardtest` (a dev route) renders `Card.svelte` in isolation — the fastest
way to tell "the card component is broken" from "the scene is placing it wrong".
