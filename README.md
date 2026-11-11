# Euchre

[Play at euchre.sh](https://euchre.sh)

Single-player euchre against three AI opponents. The game runs on Rivet actors, written in Rust, behind one Rust server. A SvelteKit web app and a native desktop app are its clients.

## Layout

| Path | What it is |
|---|---|
| `crates/euchre-engine` | The rules engine: a pure, deterministic reducer with no I/O |
| `crates/euchre-protocol` | Wire types shared by the server and both clients |
| `crates/euchre-ai` | AI decisions: heuristic, rollouts, Jev through OpenRouter, table talk |
| `crates/euchre-table` | The `euchreTable` actor: the authoritative match |
| `crates/euchre-ai-seat` | The `aiSeat` actor, one per AI opponent (three per table) |
| `crates/euchre-profile`, `crates/euchre-profile-model` | The `playerProfile` actor and its pure logic: history, stats, settings |
| `crates/euchre-http` | The HTTP API, the `/rivet/**` gateway and the engine bootstrap |
| `crates/euchre-server-core` | Config, secrets, auth tokens, the local engine and the test harness |
| `apps/server` | `euchre-server`: the binary that runs all of the above and serves the web build |
| `apps/web` | The web client: SvelteKit (Svelte 5) built as a static SPA, with Threlte for the 3D table |
| `apps/desktop` | The native desktop client (Rune), binary `euchre` |
| `packages/rivetkit-svelte` | Svelte bindings for rivetkit |
| `docs/` | `docs/port/` holds the Rust port: `ARCHITECTURE.md` (the contract), specs and reports. `docs/DEPLOY.md` is the deploy runbook. `docs/00`–`06` are the TS-era design notes, kept for history and superseded by the port. |

Actors run on rivetkit 3.1.0 against a Rivet engine 3.1.0. All durable state lives in the engine; there is no database.

## Requirements

- Bun 1.4.4. Use `bun` and `bunx` only, never npm or pnpm.
- Rust 1.99.1 with clippy and rustfmt. `rust-toolchain.toml` pins it, and rustup installs it on first use.

## Development

```sh
bun install
ALLOWED_ORIGINS=http://localhost:5173 bun run dev:server   # euchre-server on :3000, with its own engine on :6420
bun run dev:web                                            # Vite on :5173; proxies /api and /rivet to :3000
```

Open http://localhost:5173. The server reads its settings from the environment; it does not load `.env` files.

With `RIVET_ENDPOINT` unset, the server spawns a Rivet engine 3.1.0 and never downloads one. `bun run dev:server` (`apps/server/scripts/dev.sh`) points it at the binary `bun install` put in `node_modules` (`@rivetkit/engine-cli`). Run `cargo run -p euchre-server` directly, or the engine-backed tests, and the binary comes from `RIVET_ENGINE_BINARY_PATH` or rivetkit's cache at `~/.rivetkit/engine/v3/3.1.0/`; with neither, the server stops with `the rivet-engine 3.1.0 binary was not found`. `sh apps/server/scripts/dev.sh` does the same from any directory and passes its arguments to cargo (for example `--release`).

Two settings matter for local play:

- `ALLOWED_ORIGINS` must list the Vite origin. The proxy keeps the browser's `Host`, and an unlisted host gets `421`. The server always accepts its own loopback origin.
- Without `OPENROUTER_API_KEY`, the AI seats play the built-in heuristic and make no external calls. Set it to play against Jev.

`EUCHRE_FAST_TEMPO=1` shortens every table pause. The desktop app runs with `bun run dev:desktop`.

## Checks

```sh
bun run check   # clippy -D warnings for every crate and app; svelte-check for the web app and @rivetkit/svelte
bun run test    # cargo test for every crate and app; vitest for the web app
bun run build   # cargo builds, plus the web build
bun run lint    # cargo fmt --check; prettier --check
```

These run through Turborepo. Each workspace package has its own `check`, `test`, `build` and `lint` scripts, so one package runs with `--filter`, for example `bunx turbo run test --filter=@euchre/table`.

- Rust tasks cache on `Cargo.lock`, the toolchain file and every workspace crate's sources. Cargo builds are never cached by turbo.
- The turbo cache stays in this checkout (`cacheDir: .turbo/cache`). Without it, turbo would share the main git worktree's cache across worktrees, and restore outputs that hold another checkout's absolute paths.
- Engine-backed tests skip unless `EUCHRE_E2E=1`. With it set, each test binary starts a local engine on `EUCHRE_TEST_ENGINE_PORT` (default 6751). Run them one package at a time (`--concurrency=1`) so the engines do not collide. They find the engine binary as `cargo run` does (Development, above).
- Set `CARGO_TARGET_DIR` to keep a build separate from another checkout's. Turbo passes it through.

## Deploying

Railway runs two services, defined in `.railway/railway.ts`:

- `euchre`: the root `Dockerfile`, which builds `euchre-server` together with the static web build.
- `rivet-engine`: a private Rivet engine 3.1.0, with its storage on a volume.

`docs/DEPLOY.md` covers variables, secrets, plan review, releases, smoke tests and rollback. The `euchre` service has no git source: a human deploys with `railway up` and runs `railway config apply`.

## Art

The table models are Blender files in `assets/`. The card faces are Adrian Kennard's CC0 artwork, distributed by [letele/playing-cards](https://github.com/letele/playing-cards). The license is kept in `apps/web/static/art/cards/LICENSE.txt`. The burgundy card back is original artwork.
