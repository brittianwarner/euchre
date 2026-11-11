# Lane W-A WEB report: the web app on rivetkit 3.0.7 and the Rust server

**Status:** done. `apps/web` is a static SvelteKit SPA (D4). It has no server code. Every page load, form action and table session now calls the Rust server's `/api/*`, and the table connects through `/rivet` with rivetkit 3.0.7. In the browser, a full match against the `euchre-server` binary passed twice in a row. A third full match passed through the `vite dev` proxy.

Gates, all run after the final edits:

| Gate                                                      | Result                                                     |
| --------------------------------------------------------- | ---------------------------------------------------------- |
| `bun run --cwd apps/web check`                            | 0 errors, 0 warnings (1853 files)                          |
| `bun run --cwd apps/web test`                             | 17 files, 420 tests pass (Chromium browser project + node) |
| `bun run --cwd apps/web lint`                             | clean                                                      |
| `bun run --cwd apps/web build`                            | `build/client` with `index.html` fallback                  |
| `packages/rivetkit-svelte`: `bun run test`, `check-types` | 10 files, 106 tests pass; 0 type errors                    |
| `bun.lock`                                                | only `rivetkit@3.0.7` and `@rivetkit/*@3.0.7`; no 2.x      |

No git writes. Nothing under `apps/desktop/**`, `apps/server/**` or `crates/**` was edited.

## 1. Rivet JS 3.0.7

- `apps/web/package.json`: `rivetkit` is `3.0.7` (was `2.3.23`).
- `packages/rivetkit-svelte/package.json`: the peer and dev dependency are both `3.0.7`. The source needed no change; the package's tests and type check pass on 3.0.7. `AGENTS.md` now says 3.0.7 and lists `bun run` commands.
- Root `package.json`: removed `overrides` and `patchedDependencies`. `railway` moved to the root `devDependencies` (`3.12.0`). The `patches/` directory and `patches/rivetkit@2.3.23.patch` are deleted.
- `bun install` regenerated `bun.lock`.

## 2. What moved where

| TS (removed)                                                 | Now                                                                                                                                                                   |
| ------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `routes/+layout.server.ts` (session into every page)         | `routes/+layout.ts` is only `ssr = false`; pages that need the session call `GET /api/session`                                                                        |
| `routes/play/+page.server.ts` (create table, redirect)       | `routes/play/+page.ts`: `POST /api/tables {stick, natural}` → `redirect(302, /play/{gameId})`                                                                         |
| `routes/play/[gameId]/+page.server.ts` (owner check, cookie) | `routes/play/[gameId]/+page.ts` passes `gameId`; the server checks access at `POST /api/table-session/{id}` and refreshes the recent-tables cookie there (W4-notes 2) |
| `routes/games/+page.server.ts` + `_lib/session.server.ts`    | `routes/games/+page.ts`: session, recent tables, stats, games list (email, plus a browser-profile list when the guest differs)                                        |
| `routes/games/[matchId]/+page.server.ts`                     | `routes/games/[matchId]/+page.ts`: `GET /api/games/{id}?profile=browser\|email`; 401 or no identity redirects to `/games`                                             |
| `routes/settings/+page.server.ts` (load + save action)       | `routes/settings/+page.ts` loads; the page's `onsubmit` builds the same patch and `PUT /api/profile/settings`                                                         |
| identify / sign-out form actions                             | `IdentityGate.svelte` → `POST /api/identity/identify`; `PlayingAs.svelte` → `DELETE /api/identity`; both then reload the page                                         |
| `routes/api/table-session/[gameId]/+server.ts`               | Rust `POST /api/table-session/{gameId}`                                                                                                                               |
| `routes/api/rivet/[...all]/+server.ts`, `server/*` gateway   | Rust `/rivet/**` gateway                                                                                                                                              |
| `lib/actors/**` registry types                               | `lib/client/table-registry.ts`: a type-only `Registry` for `euchreTable` (actions `snapshot`, `submitMove`, `reviewTrick`, `getHandReview`, typed events)             |
| profile types from `lib/actors/player-profile`               | `lib/protocol/profile.ts` (copied unchanged)                                                                                                                          |

New files: `src/lib/client/api.ts` (the JSON API calls and `ApiError`), `src/lib/client/table-registry.ts`, `src/lib/protocol/profile.ts`, the four `+page.ts` loads above, `apps/web/README.md`, `apps/web/.gitignore`.

`src/lib/client/rivet.ts` follows W4-notes 1 exactly: endpoint `<origin>/rivet`, `encoding: 'json'`, `getToken` always set (no `dev` branch, no `$app/env`), and `TableSession.gatewayToken: string`. W4-notes 2 needed no client change. W4-notes 3 needed no change either, since the client has no episode-id check.

`vite.config.ts` uses `@sveltejs/adapter-static` (`pages`/`assets` `build/client`, `fallback: 'index.html'`). `vite dev` and `vite preview` proxy `/api` and `/rivet` (WebSocket included) to `EUCHRE_SERVER_URL`, which defaults to `http://127.0.0.1:3000`.

## 3. Removed

From `apps/web`:

- `server/` (8 files: `index.ts`, the gateway proxy, public origin, namespace bootstrap and their tests, README).
- `src/lib/actors/` (27 files: `euchre-table`, `ai-seat`, `player-profile`, `auth`, `registry.ts`).
- `src/lib/server/` (15 files: auth, db, guest, personas, recent-tables, rivet).
- `src/routes/api/` (2), every `+page.server.ts` / `+layout.server.ts` (6), `routes/games/_lib/session.server.ts`.
- `src/env.ts`, `drizzle.config.ts`, `vite-plugin-rivet-dev.ts`, `.env.example`. The SPA reads no environment variables; the dev proxy reads `EUCHRE_SERVER_URL` from the shell.
- `src/lib/ai/`: 24 files. `heuristic.ts` stays because `lib/ui/Hint.svelte` imports `rankMoves`.
- Scripts that only exercised removed code: `ai-live.ts`, `ai-bidding-live.ts`, `ai-partnership-live.ts`, `ai-strategy-live.ts`, `ai-match-eval.ts`, `ai-context-bench.ts`, `ai-status.mjs`, `or-smoke.ts`, `actor-security.mjs`, `deployment-smoke.ts`, `fixtures/` (4).
- Dependencies: `@ai-sdk/anthropic`, `@openrouter/ai-sdk-provider`, `ai`, `better-auth`, `better-sqlite3`, `jose`, `resend`, `zod`, `@sveltejs/adapter-node`, `@types/better-sqlite3`, `@types/bun`, `drizzle-kit`, `drizzle-orm`. `railway` moved to the root.
- Scripts in `package.json`: `db:*`, `start`, `test:gateway`.

There was no `hooks.server.ts` in the TS app.

`apps/web/Dockerfile` (the TS server image) disappeared from the working tree during this run, but this lane did not delete it. The root `Dockerfile` already builds `apps/web` and copies `build/client` next to the binary. `docs/DEPLOY.md` §rollback still names it as what a revert brings back, which stays true.

## 4. Routes used

Pages: `/`, `/new`, `/play?stick=&natural=`, `/play/[gameId]`, `/games` (`?cursor=` paging), `/games/[matchId]` (`?profile=browser`), `/settings`, `/cardtest`.

API, all same-origin with cookies:

- `GET /api/session`
- `POST /api/identity/identify`, `DELETE /api/identity`
- `POST /api/tables`, `GET /api/tables/recent`, `POST /api/table-session/{gameId}`
- `GET|PUT /api/profile/settings`, `GET /api/profile/stats`
- `GET /api/games?profile=&cursor=&limit=`, `GET /api/games/{id}?profile=`
- `/rivet` (actor WebSocket, gateway token per D3)

## 5. Dev workflow

`apps/web/README.md` "Development" describes the two-terminal flow:

1. Start the server: `ALLOWED_ORIGINS=http://localhost:5173,http://127.0.0.1:5173 cargo run -p euchre-server`.
2. In a second terminal, run `bun run dev:web`.

The proxy keeps the browser's `Host`. The dev origin must therefore be in `ALLOWED_ORIGINS`, or the server answers 421.

`turbo run dev --filter=@euchre/web` resolves to `vite dev`. The flow was checked end to end with the server on 6764, and `vite dev` on 6767 proxying to it. That check is the third full match below.

## 6. Browser proof (§7)

Setup:

- The SPA was built with `bun run build`.
- The `euchre-server` debug binary was built in `.cargo-targets/w4-web`.
- The binary ran from `apps/web`, so it served `build/client`. It used `PORT=6764`, `RIVET_RUN_ENGINE_PORT=6761` (its own spawned engine, on ports 6761, 6762 and 6766), `EUCHRE_FAST_TEMPO=1`, `NO_PROXY=127.0.0.1,localhost,::1`, a scratch `RIVETKIT_STORAGE_PATH`, and no AI keys, so the AI seats used the heuristic fallback.
- `apps/web/scripts/e2e-browser.mjs` (Playwright Chromium) plays one full match per run:
  1. Home, then `/games`: the identity gate, identify by email.
  2. `/settings`: rename a seat, save, reload, and check it persisted.
  3. `/new` and Deal me in, then the table tour.
  4. Turn on the review gate (Wait for me after each trick).
  5. Bid, then play every trick, pressing Continue at each review gate.
  6. Hand results, then the hand-review dialog after hand 1.
  7. A reload mid-hand: the rack count and the score must match, and no idle redraws.
  8. The desktop and phone layouts, with no horizontal scroll at 390 px.
  9. Gate off after hand 2, then play to game over.
  10. `/games`: the saved table, the stats (matches = 1) and the match link.
  11. The match detail page (outcome heading and hand list).
  12. `/settings`: the rename is still there. Then Switch profile.
- Every run fails on any page error or any 4xx/5xx response.

| Run      | Target                   | Result | Hands | Bids | Plays | Discards | Continues | Reload  | Final | Time   |
| -------- | ------------------------ | ------ | ----- | ---- | ----- | -------- | --------- | ------- | ----- | ------ |
| run1     | server :6764 (built SPA) | pass   | 10    | 10   | 50    | 3        | 15        | resumed | 5–10  | 95.8 s |
| run2     | server :6764 (built SPA) | pass   | 8     | 8    | 40    | 2        | 11        | resumed | 3–10  | 65.0 s |
| devproxy | `vite dev` :6767 → :6764 | pass   | 7     | 9    | 35    | 2        | 16        | resumed | 2–10  | 73.0 s |

In every run, page errors were 0, bad responses were 0, and idle draw calls over 1.5 s after the resume were 0. "Hands" counts the hands in the match replay.

Screenshots, 19 per run, are in `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/w4/web-shots/`:

- `run1-*`, `run2-*` and `devproxy-*`, from `01-home` to `17-signed-out`.
- `try1-*` from the trial below.

A first trial (`try1`) failed in the test itself, not in the app. It clicked Continue again while the button was still fading out, and Playwright waited 30 s for a button that had left the DOM. The server log rules out a stall: the next seat had one legal card, and `dispatch_ai` takes a forced move without asking the seat (`crates/euchre-table/src/core.rs`), so the human's turn came up at once. The script now gives each review, bid and confirm click 3 s and loops again when one misses (`clickSoon`).

## 7. Deviations from TS (forced by the move to a client-side SPA)

- **Load errors.** A failed load shows the server's status and message on the error page. The TS server loads built these errors themselves. The match-detail 404 copy is the same in both: "No game like that in your scrapbook."
- **Opening someone else's table.** A fresh browser that opens another person's `/play/{id}` link now gets the table-session refusal with the 401 copy. The TS page load minted a guest cookie first, so the same case got 403.
- **Failed settings save.** The form keeps the player's edits and shows the same error copy. TS posted the form natively, so the page re-rendered from the stored settings.
- **Identify and sign-out.** Both reload the page with `window.location.assign`, which mirrors TS's 303 redirect so the seeded page state is fresh.

The player-facing markup, copy and styles are otherwise unchanged.

## 8. Left over

- **Root `README.md`.** It still describes the TS single-app setup. It is outside this lane and is already modified in the working tree.
- **Match detail "Tricks won" reads 0–0.** `crates/euchre-profile-model/src/records.rs` `parse_stats` keeps a TS quirk on purpose: an array `tricks` is read as `[0, 0]`. The per-hand trick counts are right. This is a backend parity call, not a web change.
- **Finished matches on `/games`.** The saved-tables card lists a finished match as "Open game · Hand N". That is the TS copy for a table that is not active.
- **Test timing.** The e2e needs `EUCHRE_FAST_TEMPO=1` to finish in about 1–2 minutes. Its `handResults` counter samples the instruction text, so it can miss a short hand-score window. The replay's hand count is the real figure.
- **Script ports.** `browser-review.mjs`, `touch-smoke.mjs` and `shot-portrait-tmp.mjs` still use hard-coded dev ports (5173 / 5199). They were left as they were.
