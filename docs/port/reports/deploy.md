# Lane W-B DEPLOY report

Written 2026-11-11. This lane covers the Docker image, the Railway file, the turbo pipeline, and the README, CLAUDE.md and deploy docs. It made no git writes and ran no railway commands.

## What changed

| File | Change |
|---|---|
| `Dockerfile` (root) | New multi-stage image. Stages: cargo-chef planner and cook, release build of `euchre-server`, a Bun stage that builds the adapter-static web app, and a `debian:trixie-slim` runtime. The runtime runs as non-root uid 10001, honours `PORT` (default 3000), has a `HEALTHCHECK` on `/health`, and contains no secrets. |
| `.dockerignore` | Rewritten. It drops `.git`, agent tooling, every `target`, `.cargo-targets`, `node_modules`, build output, `.env*`, local engine state, `docs` and root `assets`. Patterns without `**/` match the root only, so `apps/desktop/assets` stays in the context. |
| `apps/web/Dockerfile` | Deleted with `rm`, because the root Dockerfile replaces it. The index still holds someone else's staged rename (`R  Dockerfile -> apps/web/Dockerfile`); the index was left alone. |
| `.railway/railway.ts` | Engine on `rivetdev/engine:3.0.7@sha256:ddda8fb8…db81fb` with the 3.x env and a start command. The `euchre` env now uses the names `ServerConfig` reads. The volume is unchanged. |
| `.railway/README.md` | Rewritten for this project. It uses bun instead of `npm install railway`; the rules for agents versus humans and the type-check command are included. |
| `turbo.json` | Rust-aware inputs and env passthrough; the build task is not cached by turbo (see Turbo below). |
| `docs/DEPLOY.md` | New operator runbook. |
| `docs/port/05-profile-auth-server.md` | New 6.1 "Rust deployment (as built)". 12.4 is rewritten to match what was built. |
| `README.md`, `CLAUDE.md` | Rewritten to describe the repo as it is now. |

Package scripts were not changed. The scripts in `apps/server` and `crates/*` were already right: build, clippy check, test, fmt lint.

## Decisions

- **Web stage (D4).** The web stage runs `bun install --frozen-lockfile` and `bun run build` in `apps/web`. It copies `build/client` (or `build`), but only if that directory holds `index.html`; otherwise the stage fails and names D4. The runtime serves it from `/app/build/client`, which is the server's `DEFAULT_STATIC_DIR` relative to WORKDIR `/app`. W-A's current `vite.config.ts` writes `pages: 'build/client'` with `fallback: 'index.html'`, and its build at 04:20 contains `build/client/index.html`.
- **Toolchain.** The image is `lukemathwalker/cargo-chef:0.1.78-rust-1.99.1-slim-trixie` with `RUSTUP_TOOLCHAIN=1.99.1`.
  - The rust slim images install the minimal rustup profile. Without this variable, `rust-toolchain.toml` would make rustup download clippy and rustfmt on every build.
  - The planner stage fails when `rust-toolchain.toml` and `ARG RUST_VERSION` differ, so the pin cannot drift silently.
- **No BuildKit cache mounts.** Railway needs `id=s/<service-id>-…` mount ids, so caching comes from cargo-chef layers instead.
- **Engine 3.0.7 env.** 3.x reads config with `deny_unknown_fields`. Proven against the 3.0.7 binary:
  - `RIVET__FEATURES__GUARD_GATEWAY_V3__*` gives `unknown field \`features\``.
  - `RIVET__FILE_SYSTEM__PATH` gives `unknown field \`file_system\``.

  Both stop the boot, so the apply removes them. The new storage env follows the proven `engine_env` profile in `crates/euchre-server-core/src/rivet.rs`, with shutdown timings that fit Railway's 120 s drain.
- **Engine port.** `PORT` changes from 6421 to 6420. In 3.x the guard on `[::]:6420` serves `/health` on any `Host`, while the 6421 API peer is loopback-only and returns 404 for `/health`. Proven: `/health` with `Host: healthcheck.railway.app` returned 200 `{"status":"ok","version":"3.0.7"}`, and the peer port returned 404.
- **Start command.** The 3.0.7 image (amd64 config read from the registry) runs as `root` with entrypoint `/usr/bin/rivet-engine`. It has no `start` subcommand, and the engine refuses a log directory that does not exist yet. The start command is therefore `/bin/sh -c "mkdir -p /data/v3/log /data/v3/objects && exec /usr/bin/rivet-engine"`.
- **Volume (D10).** One volume per service, and omitting it would delete it, so `rivet-data` stays.
  - Engine 3 writes only under `/data/v3`. The 2.3 RocksDB stays at the volume root for rollback.
  - 3.x cannot read 2.3 storage. Rivet's `migrate_from_v2` path needs a separate cluster, and D10 does not migrate in any case.
  - Result: TS games and history do not carry over to the Rust deployment.
- **`euchre` env.**
  - Literals: `NODE_ENV=production`, `PORT=3000` and `EUCHRE_TRUST_FORWARDED_FOR=1`.
  - `preserve()`: `APP_URL`, `ALLOWED_ORIGINS`, `RIVET_ENDPOINT`, `RIVET_NAMESPACE`, `RIVET_POOL`, `RIVET_TOKEN`, `EUCHRE_ACTOR_JWT_SECRET`, `EUCHRE_INTERNAL_TOKEN`, `EUCHRE_IDENTITY_SECRET` (new) and `OPENROUTER_API_KEY`.
  - Removed: `ORIGIN`, `RIVETKIT_RUNTIME_MODE`, `RIVET_ENGINE_SELFHOSTED`, `RIVET_EXPOSE_ERRORS`, `RIVET_INSPECTOR_DISABLE`, `RIVET_LOG_LEVEL` and `RIVET_PUBLIC_ENDPOINT`. The Rust server reads `RIVET_PUBLIC_ENDPOINT` only in a leak guard; browsers get their endpoint from `Host`.
  - DEPLOY.md section 2.3 lists every addition and removal.
- **Domains.** The file declares none, as before. If the plan shows a custom-domain removal, DEPLOY.md section 4 gives the block to add.

## Verification

- **Release build.** `cargo build --release --locked -p euchre-server` finished in 1m 37s, producing a 33 MB binary.
- **Docker Rust stages, emulated natively** (the Docker daemon was broken; see below).
  - Context: an rsync of the tree with the `.dockerignore` filter.
  - The toolchain check passed (`channel=1.99.1 match=yes`), and `cargo chef prepare` wrote the recipe.
  - `cargo chef cook --release --locked -p euchre-server` finished in 1m 37s.
  - The real build after the cook recompiled only the 10 workspace crates, in 22.6 s. The cache layer works.
- **Local mode.** The release binary ran with no deploy env and spawned an engine on 6930.
  - `/health` returned 200.
  - Index, SPA route and `/_app/immutable` all returned 200, with the expected cache headers.
  - Enroll returned 200. `POST /api/tables` returned 201. `POST /api/table-session/<id>` returned 200 with a `gatewayToken`. A bad `Host` returned 421.
- **Production-shaped run, with W-A's real web build.**
  - Setup: one release binary hosts a 3.0.7 engine on 6930. A second runs with `NODE_ENV=production`, the Railway env names, fresh 48-character secrets and `RIVET_NAMESPACE=euchre`, on 6938. It serves a copy of `apps/web/build/client` from 04:20.
  - Boot log: `namespace and runner pool ready namespace=euchre … created=true`, then `euchre server ready`.
  - Index returned 200 (`no-cache`). An immutable asset returned 200 (`max-age=31536000,immutable`). `/play/<id>` fell back with 200, card art returned 200, and `/.env` returned 404.
  - `/api/rivet/metadata` returned `{"clientEndpoint":"http://127.0.0.1:6938/rivet","clientNamespace":"euchre"}`.
  - Enroll, then table (201), then table-session (200). A bad `Host` returned 421, an evil `Origin` on enroll returned 403, and `/rivet/actors` returned 404.
  - Browser game, headless Chromium through Playwright, with no browser errors:
    - Home, then Play, then "Deal me in" reached `/play/<id>`, with the API responses `201 POST /api/tables`, `200 GET /rivet/metadata` and `200 POST /api/table-session/<id>`.
    - Bid "Make hearts trump", discarded, then played a card, and a trick completed ("Other team 1"). The AI seats used the heuristic, since no key was set.
    - After a reload, the page showed the same `/play/<id>`.
  - SIGTERM: both processes exited cleanly.
- **2.3 variable probe.** Each 2.3 variable makes the engine exit with `failed to deserialize config` and the `unknown field` errors quoted above.
- **`railway.ts`.**
  - `tsc --noEmit --strict --module preserve --moduleResolution bundler .railway/railway.ts` (TypeScript 6.0.3) exits 0.
  - Evaluating the program with `createRailwayContext({environment:"production"})` gives the expected graph: image digest, `startCommand`, healthcheck, replicas, mounts, every variable, and 10 `preserve` markers plus 3 literals on `euchre`.
- **Turbo,** with `--filter='!@euchre/web' --filter='!@euchre/desktop'`, a scratch `--cache-dir` and `CARGO_TARGET_DIR=.cargo-targets/w4-deploy`:
  - `check test build`: 32 of 32 tasks succeeded (9 crates plus `@euchre/server`, each running check, test and build, plus `@rivetkit/svelte` build and test).
  - Rerunning `check test` with no changes gave 21 of 21 cached ("FULL TURBO", 71 ms).
  - After the final `turbo.json` edit, `check test build` succeeded 32 of 32: the checks came from cache, and the tests reran because their task definition changed.
  - A dry run showed the Rust check and test inputs pulling 230–280 workspace Rust files, while the web inputs stay in-package.
  - `--dry=json` shows `ALLOWED_ORIGINS` passed through to `@euchre/server#dev`, and `EUCHRE_TEST_ENGINE_PORT` passed through to test.
- **Cleanup.** Nothing is listening on 6930–6939, and no process of mine is running. No Docker containers or networks were created.

## Turbo design

- `build` has `cache: false`. Cargo is the artifact cache, and turbo cannot restore a target directory. `@euchre/web#build` (outputs `build/**`, `.svelte-kit/**`) and `@rivetkit/svelte#build` (`dist/**`) stay cached.
- Rust `check` and `test` add `$TURBO_ROOT$` globs for every crate's and `apps/server`'s `Cargo.toml`, `build.rs`, `src`, `tests`, `examples` and `benches`, plus `apps/web/static/art/**`, which is embedded in the crates.
  - The reason: the crate `package.json` files do not list all their Cargo path deps (ai-seat, http, profile, profile-model and table list none), and this lane may edit only scripts. So the cache keys on all workspace Rust sources.
  - `globalDependencies` adds `Cargo.toml`, `Cargo.lock` and `rust-toolchain.toml`.
- Strict env mode stripped `CARGO_TARGET_DIR`, which sent clippy into the shared root `target/`. `globalPassThroughEnv` now carries the cargo, rustup, proxy and Playwright variables.
  - `test` hashes `EUCHRE_E2E` and passes through `EUCHRE_TEST_ENGINE_PORT`, `EUCHRE_TEST_STORAGE` and `EUCHRE_WRITE_RUST_TOKENS`.
  - `dev` passes through the server's runtime variables (`NODE_ENV`, `PORT`, `APP_URL`, `PUBLIC_APP_URL`, `ALLOWED_ORIGINS`, `BUILD_NUMBER`, `RIVET_*`, `RIVETKIT_*`, `EUCHRE_*`, `OPENROUTER_*`, `ANTHROPIC_*`). Without this, `ALLOWED_ORIGINS=… bun run dev:server` never reached the server.
- Overrides: `@euchre/web#check` (`^check`), `@euchre/web#test` and `@rivetkit/svelte#test` keep their default inputs.

## Not verified, and open items

1. **The image was not built.** Docker on this machine failed with `read-only file system` on containerd `meta.db`, and `buildx` failed too. The Rust stages were emulated natively. The web stage was not run, because this lane may not run `bun install`. For the integration agent:
   - Run `docker build -t euchre-local .`, then the throwaway engine plus app recipe in DEPLOY.md section 7.
   - Use container names `euchre-local-*` and publish on your port block.
   - Watch the web stage: `bun install --frozen-lockfile` must accept W-A's final `bun.lock`.
2. **No `railway config plan` was run**, as the rules require. A human must:
   - set `EUCHRE_IDENTITY_SECRET` before the plan;
   - make sure `ALLOWED_ORIGINS` lists `https://www.euchre.sh` and every other serving host (unlisted hosts get 421);
   - check `rivet-data` usage first. Engine 3 needs 4 GiB for its log, beside the kept 2.3 data. Raise `sizeMB` if usage is above about 5 GB.
3. **The cutover has a short outage.** TS needs engine 2.3 and Rust needs 3.0.7, so apply immediately after the merge. DEPLOY.md section 6 is the rollback.
4. **`@rivetkit/svelte` has `check-types`, not `check`.** So `bun run check` does not type-check it. That is W-A's package, which this lane may not edit; renaming the script would fix it.
5. **`@euchre/web` check and test were not run here**, per the lane filter (W-A's work in progress).
6. **Side effect.** Before the passthrough fix, one turbo `check` of `@euchre/engine` ran clippy into the shared root `target/`, which updated euchre-engine fingerprints at about 04:11. Nothing was deleted, and the next build there recompiles that crate.
7. **Shared turbo cache.** Turbo 2.11 shares the local cache across git worktrees. This worktree's default cache is `/Users/brittianwarner/goods/euchre/.turbo/cache`, inside the read-only original. Every run here used `--cache-dir` in scratch, so nothing was written there.
   - Other lanes running `bun run check` without `--cache-dir` will write into the original.
   - Setting `"cacheDir"` in `turbo.json` may stop this, but that is unverified.
8. **A stray root file, `--at`** (266 KB, 04:01, untracked), is not from this lane and was left in place.
9. **Engine tests under turbo.** With `EUCHRE_E2E=1`, `turbo run test` runs crates in parallel, and each test binary starts an engine on the same port. README and CLAUDE.md say to use `--concurrency=1`.
