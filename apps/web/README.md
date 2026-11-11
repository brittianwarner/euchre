# @euchre/web

The euchre web client: a SvelteKit (Svelte 5) static SPA (D4). It has no server
code. The Rust server (`apps/server`, binary `euchre-server`) owns `/api/*`, the
`/rivet` actor gateway and every actor, and serves this app's build.

## How it talks to the server

- **Pages load in the browser.** `ssr = false` at the root; every load is a
  `+page.ts` that calls the JSON API through `src/lib/client/api.ts`
  (`/api/session`, `/api/identity/*`, `/api/tables*`, `/api/profile/*`,
  `/api/games*`). Requests are same-origin with cookies.
- **The table** connects with the RivetKit browser client
  (`src/lib/client/rivet.ts`): it asks `POST /api/table-session/{gameId}` for the
  actor id and tokens, then opens the socket at `<origin>/rivet` (D3, JSON
  encoding per D19). `rivetkit` is pinned to exactly 3.1.0.
- **Build output** is `build/client` with a fallback `index.html`. The server
  serves that directory relative to its working directory and answers every
  extensionless miss with `index.html`.
- **The home page** (`src/routes/+page.svelte`) shows the wordmark, the
  fanned-cards picture, Play now and Download. Play now calls `startPlaying()`
  (`src/lib/client/start-playing.ts`): the same `POST /api/tables` that /new
  makes, then the table. Download reads `GET /download/manifest.json` and offers
  only the builds it lists (`src/lib/home/downloads.ts`). Because every path
  gets the SPA shell, the title, description, social tags and a `<noscript>`
  copy of the home page live in `src/app.html`.

## Development

Two terminals. The server spawns its own local Rivet engine when
`RIVET_ENDPOINT` is unset; `EUCHRE_FAST_TEMPO=1` shortens the table pauses.
`apps/server/scripts/dev.sh` runs `cargo run -p euchre-server` with the engine
binary that `bun install` put in `node_modules`. Plain `cargo run` needs
`RIVET_ENGINE_BINARY_PATH` or rivetkit's cache in `~/.rivetkit` (root
README, "Development").

```sh
# Terminal 1, from the repo root.
ALLOWED_ORIGINS=http://localhost:5173,http://127.0.0.1:5173 \
NO_PROXY=127.0.0.1,localhost,::1 \
  bun run dev:server

# Terminal 2, from the repo root
bun run dev:web
```

Open http://localhost:5173. `vite dev` proxies `/api`, `/download` and `/rivet`
(WebSocket included) to `http://127.0.0.1:3000`; set `EUCHRE_SERVER_URL` in the shell to
point it elsewhere. The proxy keeps the browser's `Host`, so the server must
list the dev origin in `ALLOWED_ORIGINS`, or it answers 421 and refuses cookie
writes. `NO_PROXY` matters only behind an HTTP proxy.

To run the production shape locally, build and let the server serve it:

```sh
bun run build                       # from apps/web → build/client
NO_PROXY=127.0.0.1,localhost,::1 sh ../server/scripts/dev.sh
# open http://127.0.0.1:3000
```

The server serves `build/client` relative to its working directory, so run
it from `apps/web` here.

## Checks

```sh
bun run check    # svelte-kit sync + svelte-check
bun run test     # vitest: browser project (Chromium) + node project
bun run lint     # prettier --check
bun run build    # static build
```

The browser tests need Playwright's Chromium once:
`bunx playwright install chromium`. The full-match browser proof is
`scripts/e2e-browser.mjs` (see `scripts/README.md`).

## Layout

| Path                         | What                                                            |
| ---------------------------- | --------------------------------------------------------------- |
| `src/routes`                 | Pages: `/`, `/new`, `/play`, `/games`, `/settings`, `/cardtest` |
| `src/lib/client`             | JSON API calls and the Rivet table client                       |
| `src/lib/protocol`           | Wire types (mirrored by `crates/euchre-protocol`)               |
| `src/lib/euchre`             | Pure rules engine (UI helpers, tests, `scripts/e2e-play.ts`)    |
| `src/lib/ai/heuristic.ts`    | Move ranking for the in-browser coach hint                      |
| `src/lib/game`, `src/lib/ui` | Table state and UI components                                   |
| `src/lib/three`              | Threlte table scene                                             |
