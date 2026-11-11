# Euchre: notes for agents

`README.md` has the layout and commands. `docs/port/ARCHITECTURE.md` is the build contract for the Rust port. `docs/DEPLOY.md` is the deploy runbook.

## Rules

- Use Bun only (`bun`, `bunx`). Never npm, npx or pnpm.
- Rust 1.99.1 (`rust-toolchain.toml`). Each crate must pass `cargo fmt -p <crate> -- --check`, `cargo clippy -p <crate> --all-targets -- -D warnings` and `cargo test -p <crate>`. `bun run check` and `bun run test` run them for every crate.
- Actor names, keys, actions, events and JSON shapes stay byte-compatible with the web client (`crates/euchre-protocol`).
- Keep the player-facing copy and behaviour that the specs in `docs/port/` pin down.
- Assume other sessions work in this repo at the same time:
  - Use your own `CARGO_TARGET_DIR`, and never run `cargo clean`.
  - Never stop a process you did not start, including other sessions' `rivet-engine` processes.
  - Do not commit, stage or stash unless asked.

## Engines in tests

Engine-backed tests skip unless `EUCHRE_E2E=1`. Each test binary starts one private engine on `EUCHRE_TEST_ENGINE_PORT` (default 6751) through `euchre_server_core::testing::TestEngine`. That engine binds the port P, plus P+1 and P+5. Behind an HTTP proxy, set `NO_PROXY=127.0.0.1,localhost,::1`. The engine refuses to start with less than 4 GiB of free disk.

## Deploy

`.railway/railway.ts` is the whole Railway project, and omitting anything there deletes it. Agents edit that file and may run `railway config plan`. Only a human runs `railway config apply`. Never run `railway up`, `railway add` or `railway down`. Never put secret values in the file; use `preserve()`.

## Svelte

`apps/web` and `packages/rivetkit-svelte` are Svelte 5. The `svelte` MCP server (`.mcp.json`) serves the Svelte and SvelteKit docs (`list-sections`, then `get-documentation`). It also runs `svelte-autofixer`; run that on any Svelte code you write until it reports no issues.
