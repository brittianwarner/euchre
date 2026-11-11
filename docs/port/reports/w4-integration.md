# W4 integration report

Written 2026-11-11. This lane ran the root gates, the deploy smoke and the browser matches, and audited the definition of done (ARCHITECTURE §7) for W4.

It made no git writes, ran no railway commands and did not edit `apps/desktop`. Every gate ran with `--filter='!@euchre/desktop'`, `CARGO_TARGET_DIR=.cargo-targets/w4-web` and `NO_PROXY=127.0.0.1,localhost,::1`. All servers and engines ran on port block 6751.

## Result

| Step | Result |
|---|---|
| `bun install` | No changes; the lockfile is frozen-clean |
| `bun run check` | 12/12. svelte-check found 0 errors and 0 warnings for `@euchre/web` and `@rivetkit/svelte` |
| `bun run test` | 12/12. Rust: 1079 passed, 6 ignored, in 67 binaries. Web: 420/420. `@rivetkit/svelte`: 106/106 |
| `bun run build` | 12/12 in 1m48s, including the release `euchre-server`. The only output is vite's chunk-size warning |
| `bun run lint` | 11/11 |
| Final re-run of check + lint after the last edits | 23/23 |
| `EUCHRE_E2E=1 cargo test -p euchre-server --test e2e` (port 6751) | 2 passed in 76 s |
| Docker image build + container smoke | **Blocked: the Docker daemon is broken on this machine.** An emulation passed; see below |
| HTTP smoke, production-shaped | Pass |
| Browser full match, production-shaped (production tempo) | Pass: 426 s, `complete`, `resumed`, 0 errors, 0 bad responses |
| Browser full match, local mode (fast tempo) | Pass: 56 s, `complete`, `resumed`, 0 errors, 0 bad responses |
| Definition of done audit | Pass for W4. Open items are listed below |

## Docker

On this machine every container create or pull fails:

```
docker: Error response from daemon: write /var/lib/desktop-containerd/daemon/io.containerd.metadata.v1.bolt/meta.db: read-only file system
```

The error was the same at the start and at the end of this lane. Containers from other sessions are still running (the `sched-m0-*` engines). Restarting Docker Desktop would stop them, and this lane may not stop what it did not start. So it did not restart Docker, and it created no containers or networks.

### What ran instead

The emulation follows the Dockerfile stage by stage:

1. **Build context.** `rsync` with the `.dockerignore` patterns gave a 16 MB context holding the expected files.
2. **Web stage.** In the context copy, `bun install --frozen-lockfile` and `bun run build` in `apps/web` wrote `build/client` with `index.html` (8.3 MB).
3. **Server stage.**
   - The planner's toolchain check passed: `rust-toolchain.toml` says 1.99.1, matching `ARG RUST_VERSION=1.99.1`.
   - `ARG BUN_VERSION=1.4.4` matches `packageManager`.
   - `cargo build --release --locked -p euchre-server` passed.
4. **Runtime layout.**
   - The release binary and `build/client` were placed in one directory, as in `/app`, and the server was started from there.
   - It ran under `env -i`, with only the production env from DEPLOY.md and `.railway/railway.ts`: `NODE_ENV=production`, `PORT`, `EUCHRE_TRUST_FORWARDED_FOR=1`, `APP_URL`, `ALLOWED_ORIGINS`, `RIVET_ENDPOINT`, `RIVET_NAMESPACE=euchre`, `RIVET_POOL=default`, `RIVET_TOKEN`, `EUCHRE_ACTOR_JWT_SECRET`, `EUCHRE_INTERNAL_TOKEN` and `EUCHRE_IDENTITY_SECRET`.
   - Secrets were generated on the spot into a 0600 scratch file outside the repo. That file was deleted at the end, and a scan found none of those values in the repo.
5. **Engine.** The stand-in for the `rivetdev/engine:3.0.7` container was the engine 3.0.7 binary on 6751, reached over `RIVET_ENDPOINT` with the admin token. The production-shaped server bootstrapped the `euchre` namespace and pool on its own.

### Owed by a human

A real image build and a container run are still owed:

1. Restart Docker Desktop.
2. Follow DEPLOY.md §7: build the root Dockerfile, run it next to `rivetdev/engine:3.0.7` on a private network, and use host ports 6940-6949.
3. Repeat the smoke below and `apps/web/scripts/e2e-browser.mjs` against the container.

## HTTP smoke (production-shaped server on 6754)

| Check | Result |
|---|---|
| `/health` | 200 `ok` |
| `/` | 200 `text/html`, `cache-control: no-cache` |
| `/_app/immutable/...js` | 200, `public,max-age=31536000,immutable` |
| SPA fallback `/play/<uuid>`, `/games` | 200, 200 |
| Card art `/art/cards/AS.png` | 200 |
| `/.env` | 404 |
| `/api/rivet/metadata` | `{"clientEndpoint":"http://localhost:6754/rivet","clientNamespace":"euchre"}` |
| Guest enroll (`POST /api/device/enroll`) | Returns `userId` `guest-…`, a session token and `expiresAt` |
| Create table with bearer | 201 `{"gameId":…}` |
| `POST /api/table-session/<id>` | 200, with tokens and `expiresAt` |
| `Host: evil.example` | `/health` 200 (any host, for Railway's healthcheck); `/` 421 |
| Enroll from `Origin: https://evil.example` | 403 |
| `/rivet/actors`, `/api/rivet/actors` | 404, 404 |
| Browser-like table create (Origin, no bearer) | 201 |

The server logs for the production-shaped and local-mode runs have no WARN or ERROR lines. The AI seats played through `source="fallback"` (`ai_no_provider`), as expected with no model key.

## Browser matches

Both runs used `apps/web/scripts/e2e-browser.mjs`. It covers home, identify, settings, new game, bidding, every trick through the review gate, hand results and the hand-review dialog. It also reloads mid-hand to test resume, then covers the phone layout, game over, `/games` with a match detail page, and switching profile.

| Run | Server | Bids / plays / discards | Review continues | Hand results | Hands in replay | Time |
|---|---|---|---|---|---|---|
| prodshape | Release binary, `NODE_ENV=production`, external engine, production tempo, port 6754 | 10 / 45 / 2 | 20 | 7 | 9 | 426 s |
| local | `euchre-server` local mode (spawns its own engine on 6751) serving `apps/web/build/client`, `EUCHRE_FAST_TEMPO=1`, port 6753 | 9 / 35 / 2 | 10 | 7 | 7 | 56 s |

Both runs ended `complete: true`, `resumed: true`, `errors: []` and `badResponses: []`, with no idle draw calls over 1.5 s.

### Script fixes

The first two production-tempo runs failed in the script, not the server. Earlier runs had only used fast tempo, which hid both races.

1. **The tour covered the settings click.** The first-run tour opens once cards are dealt. At production tempo that comes seconds after the table renders, so it opened after the script had already dismissed it. It then intercepted the review-gate checkbox.
   - Fix: `skipTour(waitMs)` now waits up to `waitMs` for the tour's Skip button.
   - The script also waits for a card in the hand, or an enabled `.bids` button, and only then touches settings.
2. **The player can cut first.** When seat 0 cuts first, no card arrives until the player cuts, so the 60 s wait for a card timed out.
   - Fix: the wait accepts an enabled `.bids` button (the cut) as well as a card. The match loop then dismisses the tour.

`apps/web/scripts/README.md` now says a match takes about a minute at fast tempo and about seven at production tempo.

## Definition of done audit (ARCHITECTURE §7, W4 scope)

| Item | Status |
|---|---|
| Root `check`, `test` and `build` pass (turbo runs cargo + web) | Pass. All three were run without desktop: the desktop wave is in progress, and this lane may not touch `apps/desktop` |
| Engine goldens, AI tests, profile/auth goldens | Pass, as part of `bun run test` (1079 Rust tests) |
| `EUCHRE_E2E=1 cargo test -p euchre-server --test e2e` | Pass (2 tests, real 3.0.7 engine, port 6751) |
| Desktop launches and plays with animations; scenes render headless | Not audited: desktop lane |
| Web app plays a full match against the Rust backend (browser) | Pass, twice (above) |
| No TS actor code | Pass. Two files import from `rivetkit`, both type-only: `apps/web/src/lib/client/table-registry.ts` (`import type { ActorDefinition, Registry, event }`) and `packages/rivetkit-svelte/src/lib/internal/framework-base.ts` (`import type`). Nothing calls `actor(` or `setup(` |
| No rivetkit 2.x in package.json files | Pass. `apps/web` and `packages/rivetkit-svelte` pin `rivetkit` at exact `3.0.7` |
| No rivetkit 2.x in `bun.lock` | Pass. Every `rivetkit` and `@rivetkit/*` entry is 3.0.7, platform packages included (20 entries). The exceptions are the independent `@rivetkit/bare-ts@0.6.2` and `@rivetkit/on-change@6.0.1` |
| No patches | Pass. There is no `patches/` directory, and no `patchedDependencies`, `overrides` or `resolutions` in any package.json |
| Rust `Cargo.lock` | Pass. All 14 rivet crates are at 3.0.7 |
| Railway engine image | Pass. `rivetdev/engine:3.0.7@sha256:fd679281d8a357d20081a6639933d1fcaecdc6fc087604f7a05ee0905a8db731`. The engine's `/health` reports `3.0.7` |

## Left open

1. **Docker image build and container smoke.** Blocked by the Docker daemon; owed by a human (see Docker above).
2. **Orphaned 2.3.23 packages in `node_modules/.bun`** in this checkout: `@rivetkit+engine-cli`, `engine-api-full`, `rivetkit-napi` and others. They are not in `bun.lock`, and a fresh install does not create them. `rm -rf node_modules && bun install` clears them. This lane left them because another session may be using this checkout's store.
3. **The auth fixture generator imports deleted TS code.** `crates/euchre-server-core/scripts` imports `apps/web/src/lib/actors/...` and `jose`. The committed fixtures and the cargo tests are unaffected. Filed as `requests/server-foundation-auth-fixture-scripts.md`.
4. **Provenance comments still cite rivetkit 2.3.13.** They are in `packages/rivetkit-svelte/src/lib/internal/framework-base.ts:4` and `src/lib/shared.svelte.ts:162`. They record where the code came from and are not a dependency. Reword them if "2.x appears nowhere" should cover comments too.
5. **A stray root file `--at`** (267 KB, written 04:01), probably from a mistyped flag. It is a rune desktop motion trace (`"launch":"rune"`), so it comes from the desktop lane. `.dockerignore` does not exclude it, so it lands in the build context. Its owner should delete it.
6. **Hard-coded ports in three helper scripts:** `apps/web/scripts/shot-portrait-tmp.mjs` (5199), `browser-review.mjs` (5173) and `touch-smoke.mjs` (5173). They are dev helpers, not gates. `shot-portrait-tmp.mjs` looks temporary.
7. **Desktop items:** the LowDisk UI (`W3-checklist.md`, line 14) and the desktop line of §7 belong to the desktop lane.
8. **Known parity quirk, kept on purpose:** match detail shows "Tricks won 0–0" (`reports/web-retarget.md`). It matches TS.

`EUCHRE_PRODUCTION` does not exist. `ServerConfig::from_env` turns on the deployment guards with `NODE_ENV=production`, which Railway sets as a literal on `euchre`.

## Files changed by this lane

| File | Change |
|---|---|
| `turbo.json` | `"cacheDir": ".turbo/cache"`. Without it, turbo 2.11 shares the main git worktree's cache across worktrees and restores outputs that hold another checkout's absolute paths. Verified: the cache now lands in this checkout, and the original's cache stays empty |
| `packages/rivetkit-svelte/package.json` | Added a `check` script (svelte-check), so root `bun run check` type-checks the package. `check-types` stays as an alias |
| `packages/rivetkit-svelte/AGENTS.md` | Scripts row mentions `check` |
| `packages/rivetkit-svelte/README.md` | RivetKit `3.0.7` (exact) instead of `^2.3.17-layerr.0`. bun commands instead of npm |
| `README.md` (root) | The check line names both svelte-check targets. Adds a note on the worktree-local turbo cache |
| `apps/web/scripts/e2e-browser.mjs` | Tour and cut-first waits for production tempo (above). Prettier-clean |
| `apps/web/scripts/README.md` | Match timings at fast and production tempo |
| `docs/port/W3-checklist.md` | Line 18 marks `EUCHRE_TRUST_FORWARDED_FOR` as done (a literal `"1"` on `euchre`) |
| `docs/port/requests/server-foundation-auth-fixture-scripts.md` | New request (item 3 above) |
| `docs/port/reports/w4-integration.md` | This report |

The player-facing UI and copy are unchanged.

## Cleanup

- Both servers were stopped with SIGTERM and shut down gracefully. The local-mode server stopped its engine.
- Nothing listens on 6750-6760.
- No `euchre-server`, `rivet-engine` or headless browser from this lane is running. The one `rivet-engine` still running belongs to another checkout and was left alone.
- No Docker containers or networks were created.
- The scratch secrets file is deleted.

## Review fixes

Written 2026-11-11 by the W4 fix agent, after three reviews of this lane: web-flows, deploy-security and done. Each finding was checked against the code before anything changed.

The same rules held: no git writes, no railway commands, nothing in `apps/desktop`, and bun only. Gates ran with `--filter='!@euchre/desktop'`, `CARGO_TARGET_DIR=.cargo-targets/w4-web` and `NO_PROXY=127.0.0.1,localhost,::1`. Servers and engines ran on port block 6990-6999.

### Findings

| # | Review | Sev | Finding | Outcome |
|---|---|---|---|---|
| 1 | web-flows | P1 | Turning on Hints crashes the table (`state_unsafe_mutation`), again after every reload. Already in TS | **Fixed.** See "Hint crash" below |
| 2 | web-flows | P3 | Match detail and stats always show "Tricks won 0–0" | **Skipped.** `parse_stats` copies TS on purpose (its doc comment says so), and the profile model crate is frozen for W4. Changing it departs from TS, which needs an orchestrator call like D30. Filed `requests/profile-tricks-won.md` |
| 3 | web-flows | P3 | At phone width the fixed nav covers the top of `/games` and the match detail Back link. Already in TS | **Skipped.** W4 keeps the player-facing UI identical to TS, and this is a design fix, not a port fix. Filed `requests/web-nav-covers-games-top.md` for the UI fidelity wave |
| 4 | deploy-security | P1 | DEPLOY.md assumed a branch auto-deploy, but `euchre` has no source and ships with `railway up` | **Fixed** in `docs/DEPLOY.md`. See "Release model" below |
| 5 | deploy-security | P2 | The SPA shipped no precompressed assets (about 4x more JS and CSS than TS) | **Fixed.** `adapter-static` now runs with `precompress: true`. The server already sends `.br` and `.gz` siblings |
| 6 | deploy-security | P3 | Base images pinned by tag, not digest | **Fixed.** All three FROM lines pin the multi-arch index digest, through `CHEF_DIGEST`, `BUN_DIGEST` and `DEBIAN_DIGEST`. Each digest was checked against Docker Hub and a registry HEAD. A comment says to bump tag and digest together |
| 7 | deploy-security | P3 | `curl` in the runtime image only for a Docker HEALTHCHECK that Railway ignores | **Fixed.** The runtime installs `ca-certificates` only, and the HEALTHCHECK is gone. Railway probes `/health` itself. DEPLOY §7 now waits on `curl` from the host instead of `docker inspect` health |
| 8 | deploy-security | P3 | No security headers on HTML or API responses | **Fixed.** HTML gets `Content-Security-Policy: frame-ancestors 'none'` and `X-Frame-Options: DENY`. Every response gets `X-Content-Type-Options: nosniff` and `Referrer-Policy: strict-origin-when-cross-origin` (`if_not_present`, so a handler can still set its own) |
| 9 | deploy-security | P3 | `/index.html` fetched directly was cached for a day | **Fixed.** Any `.html` file, matched case-insensitively, is `no-cache`, like `/` and SPA routes |
| 10 | deploy-security | P3 | DEPLOY §3.1's secret recipe contradicted its "out of shell history" claim | **Fixed.** The dashboard is now the preferred way. The CLI form puts the value in a variable, sets it with `--skip-deploys` and unsets it, and notes that the value is briefly in the process list |
| 11 | done | P2 | The documented dev setup fails on a machine without a cached engine binary | **Fixed.** New `apps/server/scripts/dev.sh` (the `dev` script) resolves the engine from the `@rivetkit/engine-cli` 3.0.7 devDependency when neither `RIVET_ENGINE_BINARY_PATH` nor `RIVET_ENDPOINT` is set, then runs `cargo run -p euchre-server` with your args. README and `apps/web/README.md` explain how the binary is found. Checked with an empty `HOME`: it resolves `node_modules/.bun/@rivetkit+engine-cli-darwin-arm64@3.0.7/.../rivet-engine` (`Rivet 3.0.7`) from `apps/server` and from `apps/web` |
| 12 | done | P2 | The index stages 102 renames whose targets are deleted on disk, 50 of them TS actor or server code | **Skipped: no git writes in this wave.** Verified read-only: `git status --short` shows 102 `RD` entries, 50 under `src/lib/actors`, `src/lib/server` or `apps/web/server`. See "At commit time" below |
| 13 | done | P3 | 05 §6.1 contradicted itself and the code on `BETTER_AUTH_SECRET` / `AUTH_SECRET` | **Fixed** in the spec. The code (`ACTOR_SECRET_VARS` in `config.rs`) still reads both as fallbacks when `EUCHRE_ACTOR_JWT_SECRET` is unset or empty, and §6.1 now says so. Only `BETTER_AUTH_URL` and the other `BETTER_AUTH_*` names are removed |
| 14 | done | P3 | 05 §13 and related lines described a `--dev` / `EUCHRE_DEV` / auto-download mode | **Fixed.** 05 secret loading, §11.3, §12.1, §12.4 and §13 now describe what was built: `ServerConfig::local` with `secrets.json`, `LocalEngine` on `RIVET_RUN_ENGINE_PORT` with downloads off, `dev.sh`, `Secure` cookies exactly when `APP_URL` is https, and the digest-pinned, curl-free image |
| 15 | done | P3 | TS-era docs 00-06 claim authority with no superseded banner | **Fixed.** Each of `docs/00`-`06` opens with a banner: TS-era, the port wins, read `port/ARCHITECTURE.md` first. The README's docs/ row says the same |
| 16 | done | P3 | Protocol comments still describe the removed TS backend | **Fixed, comments only.** The module header names `crates/euchre-protocol` and the Rust actor crates. Queues became actions (`decide`, `aiDecision`, `resetHand`, `handEnd`, `gameEnd`, `recordGame`, `recordEpisodes`). The form action, `jose`, `canPublish` and "serverless migration" notes now name the Rust checks. No types or code changed |
| 17 | done | P3 | Five resolved requests were not moved to `done/` | **Fixed.** `gateway-token`, `integration-local-engine`, `server-ai-seat-table-params`, `server-http-engine-admin-api` and `server-profile-auth-scope` moved with plain `mv`, each with a "Resolved" line. Live references now point at `done/`: ARCHITECTURE D3, `W3-checklist.md`, and doc comments in `euchre-http` (`token.rs`, `bootstrap.rs`) and `euchre-table` (`remote.rs`). Older reports keep their original paths, as earlier moves did. `integration-local-engine.md` was a byte-identical copy of `done/foundation-local-engine.md`; both are kept so the checklist link works |
| 18 | done | P3 | Broken parent and related links in `packages/rivetkit-svelte/AGENTS.md` | **Fixed.** They point at the root `README.md`, `apps/web/README.md` and `docs/port/ARCHITECTURE.md` |

Also from "Left open" item 4: the comment in `packages/rivetkit-svelte/src/lib/shared.svelte.ts` now says that rivetkit 3.0.7, like 2.3.13, omits `on` from `ActorConn`. That was checked against the 3.0.7 `.d.ts`. The `framework-base.ts` provenance line still names 2.3.13, because that is where the code came from.

### Hint crash

**Cause.** `gloss()` ran inside the template and wrote `$state.raw` (`explainedTerms`). Svelte 5 throws `state_unsafe_mutation` on any state write during render. So the first hint with a glossed term ("ruff", "lay off", "second hand", ...) crashed the table, and since the toggle persists, so did every reload.

**Fix** (`apps/web/src/lib/ui/Hint.svelte`).
- The explained-term set is plain, non-reactive bookkeeping, because nothing has to re-render when a term is marked.
- A per-turn memo (`turnId` + `why`) returns the same text when the same turn re-renders, so a fresh `view` object does not drop the explanation it just gave.
- The markup, copy and styles are unchanged. `svelte-autofixer` reports no issues.

**Test** (`apps/web/src/lib/ui/Hint.svelte.spec.ts`, browser project).
- It drives heuristic self-play to find two seat-0 decisions on different turns whose hints use the same term.
- It renders the first and expects "— meaning". It re-renders the same turn with a fresh view object, and the meaning stays. It renders the later turn, and the meaning is not repeated.
- Red before the fix: it threw `state_unsafe_mutation` against the original component. Green after.

**In a real browser.** A hints-on full match against the local-mode server played 52 moves to game over. It showed 17 distinct hints and explained "second hand" and "lay off" once each. The page logged no errors.

### Release model (deploy P1)

DEPLOY.md now matches how `euchre` really ships. This follows the TS release flow (`railway up` with the project, environment and service ids, `--detach`).
- The rules say the service has no git source, so a merge deploys nothing. Only a human runs `railway config apply` and `railway up`.
- §3.3 (the window):
  1. Check out the merged commit on a clean tree.
  2. Run `railway config plan`.
  3. A human applies. The outage starts here.
  4. A human runs `railway up` with the full ids.
  5. Smoke. The outage lasts about as long as the image build.
- §6 rollback is config first:
  1. Restore the TS `railway.ts`.
  2. Plan and apply.
  3. Redeploy the last TS deployment from the dashboard, or `railway up` from a TS worktree.
  4. Revert the merge.
  5. Smoke.
- New §8 covers releases after the move.
- **Not chosen:** adding a GitHub source in `.railway/railway.ts`. It cannot be checked without `railway config plan`, it would change the deploy model, and it needs the GitHub app.

### At commit time (finding 12)

Before committing the port, stage the deletions as well as the additions. One way is `git add -A` over the port paths. Then check that this prints nothing:

```
git diff --cached --name-only | grep -E 'apps/web/(server|src/lib/(actors|server))|drizzle|\.env\.example'
```

If only what is staged now is committed, the commit brings back the TS actor and server code, `drizzle.config.ts` and dead scripts. `git status --short` also shows ` D patches/rivetkit@2.3.23.patch` (deleted, not staged), and the stray root file `--at` from the desktop lane is still untracked.

### Gates

| Step | Result |
|---|---|
| `bun install --frozen-lockfile` | No changes. `bun.lock` gained one line, the `@rivetkit/engine-cli` 3.0.7 devDependency of `apps/server`; frozen-clean after |
| `bun run check` | 12/12. svelte-check: 0 errors, 0 warnings for `@euchre/web` and `@rivetkit/svelte`. clippy `-D warnings` clean |
| `bun run test` | 12/12 in 33 s. Rust: 1080 passed, 0 failed, 6 ignored, in 67 binaries (+1: the new headers test). Web: 421/421 (+1: the Hint test). `@rivetkit/svelte`: 106/106 |
| `bun run build` | 12/12 in 13 s, including the release `euchre-server`. `build/client` now holds 76 `.br` and 76 `.gz` files |
| `bun run lint` | 11/11 |
| `EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6990 cargo test -p euchre-server --test e2e` | 2 passed in 97 s |
| `cargo test -p euchre-http --lib static` | 7 passed, including `static_html_is_no_cache_and_unframeable_and_every_response_is_nosniff` |
| Docker image build + container smoke | **Still blocked.** `docker build .` and `docker create` both fail with `write .../io.containerd.metadata.v1.bolt/meta.db: read-only file system` (Docker 29.7.2). No container, network or image was created. The emulation below passed |

### Docker emulation (same steps as above)

1. **Context.** `rsync` through `.dockerignore`: 16 MB, with no `build`, `node_modules` or `.svelte-kit`.
2. **Web stage.** In the context copy, `bun install --frozen-lockfile`, then `bun --bun run build` in `apps/web`. That runs vite on the bun runtime, as the node-less `oven/bun` image does. Precompress works there: 76 `.br` files, `index.html.br` included.
3. **Server stage.**
   - `rust-toolchain.toml` 1.99.1 equals `ARG RUST_VERSION`, and `ARG BUN_VERSION` 1.4.4 equals `packageManager`.
   - `cargo build --release --locked -p euchre-server` passed. Cargo reused the release artifacts, because the sources are byte-identical with mtimes kept.
   - Every `include_str!` / `include_bytes!` path is inside `crates/`, so the context omits nothing the build reads.
4. **Runtime.**
   - The binary and `build/client` were placed in one directory, as in `/app`, and run under `env -i` with only the production env from DEPLOY.md.
   - The secrets were generated into a 0600 scratch file, deleted at the end.
5. **Engine.** The engine was 3.0.7 on 6990. The production-shaped server bootstrapped namespace `euchre` and pool `default` itself (`created=true attempts=1`).

### HTTP smoke (production-shaped server on 6994)

| Check | Result |
|---|---|
| `/health` | 200 `ok` |
| `/` with `Accept-Encoding: br, gzip` | 200 `text/html`, `content-encoding: br`, `cache-control: no-cache`, `vary: Accept-Encoding`, CSP `frame-ancestors 'none'`, `x-frame-options: DENY`, `nosniff`, `referrer-policy` |
| `/index.html` | 200, `no-cache`, CSP, DENY |
| `/_app/immutable/entry/app.*.js` | `br`: 950 bytes (the body is the `.br` file byte for byte). `gzip`: 1020. Identity: 2711. Each is `public,max-age=31536000,immutable` with `vary: Accept-Encoding`, `nosniff`, no CSP |
| SPA fallback `/play/<uuid>`, `/games` | 200 `text/html`, `no-cache` |
| `/art/cards/AS.png`, `/robots.txt` | 200, `max-age=86400` |
| `/.env`, `/missing.js` | 404 (with `nosniff`) |
| `/api/rivet/metadata` | `{"clientEndpoint":"http://localhost:6994/rivet","clientNamespace":"euchre"}` |
| Guest enroll | `userId` `guest-…`, `sessionToken`, `expiresAt` |
| Create table with bearer, then `POST /api/table-session/<id>` | 201 `{gameId}`, then 200 with `actorId`, `expiresAt`, `gatewayToken`, `token` |
| `Host: evil.example` | `/health` 200, `/` 421 (still `nosniff` and `referrer-policy`) |
| Enroll from `Origin: https://evil.example` | 403 |
| `/rivet/actors`, `/api/rivet/actors`, `/api/nope` | 404, 404, 404 |

### Browser matches (`apps/web/scripts/e2e-browser.mjs`)

| Run | Server | Bids / plays / discards | Review continues | Hand results | Hands in replay | Time |
|---|---|---|---|---|---|---|
| prodshape | Release binary from the context build, under `env -i` with `NODE_ENV=production`, external engine, production tempo, port 6994 | 11 / 45 / 2 | 15 | 8 | 9 | 420 s |
| local | Release `euchre-server` in local mode (spawned engine 3.0.7 on 6990), serving `apps/web/build/client`, `EUCHRE_FAST_TEMPO=1`, port 6993 | 14 / 65 / 3 | 21 | 11 | 13 | 101 s |

Both runs ended `complete: true`, `resumed: true`, `errors: []` and `badResponses: []`, with 0 idle draw calls over 1.5 s. Neither server log has a WARN or ERROR line. A third run, the hints-on probe above, also reached game over with no page errors.

### Files changed by the fixes

| File | Change |
|---|---|
| `apps/web/src/lib/ui/Hint.svelte` | Crash fix (finding 1) |
| `apps/web/src/lib/ui/Hint.svelte.spec.ts` | New browser test |
| `apps/web/vite.config.ts` | `precompress: true` |
| `apps/web/src/lib/protocol/index.ts` | Comments only (finding 16) |
| `crates/euchre-http/src/static_files.rs` | HTML `no-cache` by extension, CSP + `X-Frame-Options` on HTML |
| `crates/euchre-http/src/api/mod.rs` | `nosniff` + `Referrer-Policy` on every response |
| `crates/euchre-http/src/api/tests.rs` | Headers and caching test |
| `crates/euchre-http/src/{token,bootstrap}.rs`, `crates/euchre-table/src/remote.rs` | Doc-comment paths to `requests/done/` |
| `Dockerfile` | Digest pins, no `curl`, no HEALTHCHECK |
| `docs/DEPLOY.md` | Release model, rollback, secret recipe, image notes, §7 wait, §8 |
| `apps/server/scripts/dev.sh` (new), `apps/server/package.json`, `bun.lock` | Engine binary resolution for dev |
| `README.md`, `apps/web/README.md` | Dev engine binary, docs pointer, `railway up` |
| `docs/port/05-profile-auth-server.md` | §6.1, secret loading, §11.3, §12.1, §12.4, §13 |
| `docs/00`-`06` | TS-era banners |
| `docs/port/ARCHITECTURE.md`, `docs/port/W3-checklist.md` | Request paths to `done/` |
| `docs/port/requests/done/*` (5 moved), `requests/profile-tricks-won.md`, `requests/web-nav-covers-games-top.md` (new) | Findings 2, 3, 17 |
| `packages/rivetkit-svelte/AGENTS.md`, `src/lib/shared.svelte.ts` | Links; version in one comment |

The player-facing UI and copy are unchanged.

### Cleanup

- Both servers stopped on SIGTERM and shut down gracefully. The local-mode server stopped its engine.
- Nothing listens on 6990-6999. No `euchre-server`, `rivet-engine` or headless browser from this agent is running.
- No Docker container, network or image was created, because every create failed.
- The scratch secrets file is deleted. A scan found none of its values in the repo.
- `.cargo-targets/w4-deploy` (18 GB) and `w4-server` (14 GB) are deleted. No process had a file open in them. No `w4-review-*` directory existed.
- The `dev.sh` check used a stand-in `cargo` that only printed its env, under `env -i` with an empty `HOME`. No server was started by it.
