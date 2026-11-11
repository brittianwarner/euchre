# 10. Rune on the web: build plan

Planner, 2026-11-11. This is the build contract for putting the animated Rune game on euchre.sh. It runs
**after the desktop network wave lands** (`apps/desktop/src/net/{mod,bridge,remote,sim,embedded}.rs`
final, its gates green). Where this file and the lane docs disagree, this file wins (§10 lists the
corrections). ARCHITECTURE.md D35 records the decision.

Inputs, both checked by the planner:

- [web-rune/spike.md](web-rune/spike.md): the desktop app on wasm played a full hand against the sim in
  headless Chromium (Metal adapter), zero console errors. Checked: `app_bg.wasm` is a real module
  (12,969,061 B art-by-URL, 16,358,535 B art-embedded, as claimed), `proof.json` shows `v` 2 → 34,
  hand 0 → 1, score 0–1, and the screenshots show Home, the deal, the bid panel, the sweep, the hand
  result, and the broken phone layout and suit boxes the doc reports.
- [web-rune/network-and-serving.md](web-rune/network-and-serving.md): the rivetkit JS bridge, the
  `WebBackend` probe against a real `euchre-server`, and the cookie-picked shells. Checked: the probe
  screenshot shows every `Backend`/`TableCalls` call and the reconnect; the no-WebGPU screenshot is the
  Svelte `/games` at the same URL; `net/static_files.patch` is the change described.

## 0. Done means

1. A WebGPU browser at `https://euchre.sh/<any route>` gets the Rune game, playing against the same
   server, cookies and match history as the Svelte site.
2. Every other browser gets today's Svelte site at the same URL, with no dead end and no reload loop.
3. Deep links (`/`, `/new`, `/play`, `/play/<id>`, `/games`, `/games/<id>`, `/settings`) mean the same
   thing in both.
4. The image is one `railway up` with no Railway config change, and rolls back by redeploying the
   previous deployment.
5. The §5 proofs pass and the §6 budgets hold.

## 1. Decisions

| # | Decision | Rejected (one line each) |
|---|---|---|
| W-1 | Build `apps/desktop` itself for `wasm32-unknown-unknown` (cdylib + `src/web.rs` start entry). One UI codebase; no screen, view or motion file forks. | A separate web crate: forks the UI. |
| W-2 | One async-runtime seam, `net/rt.rs` (`spawn`, `sleep`, `timeout`), tokio on native, `spawn_local` + `setTimeout` on wasm. Futures stay `Send`; `async_trait` unchanged. | Making the bridge `?Send`: rewrites every trait. |
| W-3 | Network: `WebBackend` (`net/web/`) over `fetch` with `credentials: "same-origin"` (the HttpOnly guest cookie) plus the official **rivetkit 3.1.0 JS client** through a bun-bundled bridge. `!Send` JS values live inside `Local<T>` (unsafe `Send`/`Sync` only on single-threaded wasm). | (a) Rust `rivetkit-client` on wasm: 59 compile errors, raw TCP. (c) Own actor protocol over web-sys WebSocket: copies handshake, reconnect and token refresh with no upstream owner. |
| W-4 | Serving: one URL space, two shells. `index.html` = Rune loader; the unchanged Svelte build = `classic.html`; the server picks by the `euchre_ui=classic*` cookie (`Vary: Cookie`). | Svelte under `/classic` via `kit.paths.base`: redirect loop today, ~25 Svelte link edits, links diverge. |
| W-5 | The loader sends a browser to classic when: no `navigator.gpu`; `requestAdapter()` null or throws; the adapter is a fallback (software) adapter; the viewport is under 720×520 CSS px; no `runepresent` within 10 s of import; Rune reports a lost device. `?rune=1` skips the fallback-adapter and size gates (tests only). | Trusting any adapter: SwiftShader loses the device after frame one. |
| W-6 | Cookie values: `classic-auto` (detected, 7 days), `classic-small` (viewport, session cookie), `classic` (chosen in Rune Settings, 400 days). Cookies blocked → the loader swaps `classic.html` in place. Not HttpOnly: a UI preference, not a credential. | A server-side UA sniff: cannot see WebGPU. |
| W-7 | Phones (under 720×520) get classic at launch. A compact Rune table is W4 stretch work; when it passes the 390×844 proof, the size gate drops. | Shipping the desktop layout to phones: overlapping table, colliding bid buttons (spike §6.2). |
| W-8 | Art by **absolute** URL, not embedded: `ImageCache::url` on `/_rune/immutable/art-<arthash>/…` on wasm; `include_bytes!` stays on native. Saves 3.33 MB brotli (43 %) and ~210 ms to first frame. | Embedded art: 7.69 MB brotli. Spike's page-relative `art/…`: breaks on `/play/<id>` and collides with Svelte's `/art/cards/AS.png` (different Ace, D31 S-2). |
| W-9 | Rune files under `/_rune/immutable/<hash>/` (one-year immutable cache, brotli/gzip siblings). `<hash>` = first 16 hex of sha256 over `app_bg.wasm`, `app.js` and the bridge snippet. | Fixed names: stale caches. |
| W-10 | No `wasm-opt` (brotli 2–5 % worse on every build measured). No Rune `--html` static export (needs a second native build; a table is not crawler text). | — |
| W-11 | Our own `apps/desktop/scripts/web-build.sh` runs cargo + wasm-bindgen with Rune's release profile, copies Rune's Inter faces, hashes and fills the loader. Docker and turbo both call it. No Python. | Rune's `web-build.sh`: needs Python ≥ 3.11 and misses Inter with a git dependency. |
| W-12 | `Backend` gains `identify(email)` and `sign_out()` (default `Err(NetError::NotWired)`), implemented on web. `WebBackend` reads `profile=email` when the session has an email identity, else `profile=browser`. | Leaving identity to the Svelte site: Rune users could never attach an email. |
| W-13 | Fallback (software) adapters go to classic (`adapter.info.isFallbackAdapter`, or `adapter.isFallbackAdapter` on older Chromium). | Letting Rune draw on the CPU. |
| W-14 | Sound off by default, WebAudio when on (same synthesized cues). Prefs in `localStorage["euchre:desktop-prefs"]`, same JSON. The turn halo (~20 fps while it is your turn, 1 frame per 3 s with reduced motion) is accepted. | — |
| W-15 | Proofs drive the **shipping** build through Rune's DOM mirror (W4 keys every control) and check results through `/api/*`. The spike's `?proof` probe moves behind cargo feature `web-proof`, never shipped, never used by a done-proof. | Shipping the `?proof` hook. |
| W-16 | No CSP, COOP, COEP or Railway variable change. If a `script-src` is ever added it must carry `'wasm-unsafe-eval'` and hashes for the loader and Svelte bootstrap scripts. | — |

## 2. Shape of the result

```
build/client/ (served by euchre-server from /app/build/client)
  index.html{,.br,.gz}                  Rune loader (no-cache). Crawlers and new browsers get this.
  classic.html{,.br,.gz}                Svelte adapter-static fallback, renamed, otherwise unchanged
  _app/immutable/…, art/…, robots.txt   Svelte, unchanged
  og.png                                link preview
  _rune/immutable/<hash>/app.js{,.br,.gz}, app_bg.wasm{,.br,.gz}
  _rune/immutable/<hash>/snippets/euchre-desktop-<h>/js/rivet-bridge.js{,.br,.gz}
  _rune/immutable/<hash>/Inter-{Regular,Medium,SemiBold}.ttf{,.br,.gz}
  _rune/immutable/art-<arthash>/cards/*.png, euchre-hero.webp   (no .br: PNG/WebP do not shrink)
```

Request flow: `GET /games/<id>` → `static_files.rs` SPA fallback → cookie `euchre_ui=classic*` and
`classic.html` on disk ? `classic.html` : `index.html`. The loader runs the W-5 gates, then
`import("/_rune/immutable/<hash>/app.js")`; `web.rs::start` reads `location.pathname`, routes to the
match screen, and `WebBackend` calls `/api/*` and `/rivet` with the browser's cookies.

## 3. Lanes, ownership and order

Ownership rules from ARCHITECTURE §1 apply: edit only owned paths; ask other lanes through
`docs/port/requests/<lane>-<topic>.md`. No git writes, bun only, never stop a process you did not
start. Each lane builds in its own target dir, an APFS clone of the network wave's dir
(`cp -c -R .cargo-targets/<network-wave-dir> .cargo-targets/web-wN`). Engines for local proofs use
the overflow blocks: W2 6860, W3 6870, W4 6880, W5 6890 (P..P+9 each).

### 3.1 Order

| Phase | Runs | Starts when |
|---|---|---|
| 1 | **W1** ‖ **W3** ‖ **W2a** (bridge + `net/web/` in W2's scratch crate) | the network wave lands |
| 2 | **W2b** (wire into the app) ‖ **W4** | hand-off **H1**: W1 done (wasm check green, sim proof re-run) |
| 3 | **W5** proofs, fix loop with the owning lane | W2b, W3, W4 done |
| 4 | Deploy (§7) | W5 green, owner sign-off |

### 3.2 W1: desktop crate on wasm (lands the spike diff on the final net layer)

Re-apply spike §3 by intent, not by copying files: the scratch copy predates the network wave.

| Path | Change |
|---|---|
| `apps/desktop/Cargo.toml` | `[lib] crate-type = ["cdylib", "rlib"]`. Move `euchre-server`, `euchre-server-core`, `rivetkit-client`, `rivetkit-engine-process`, `directories`, `reqwest`, `rodio`, tokio `full` under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`. wasm: tokio `default-features = false, features = ["sync", "macros"]`, `uuid` `js`, `wasm-bindgen = "=0.2.130"`, `wasm-bindgen-futures`, `js-sys`, `web-sys` (console, Window, Location, History, Document, Element, Storage, Performance, Headers, Request, RequestInit, RequestCredentials, Response, AbortSignal, DomException, Crypto, AudioContext, AudioBuffer, AudioBufferSourceNode, AudioDestinationNode, AudioNode). Feature `web-proof = []`. Rune's `dev`, `hot`, `native-file-dialog` off on wasm. |
| `crates/euchre-ai/Cargo.toml`, `src/lib.rs` | cfg gates only: `ai_seat`, `banter`, `clock`, `decide`, `jev`, `model`, `talk`, `testing` native-only; tokio/reqwest native-only. `top_move`, `rank_moves`, `HeuristicOptions`, `RankParams` stay. **Needs orchestrator approval** (frozen crate, D28). |
| `apps/desktop/src/net/rt.rs` (new) | The seam S1 (§4). |
| `apps/desktop/src/net/mod.rs` | Gate `embedded`, `fetch`, `local`, `remote` native-only; `pub mod rt`. `now_ms()` on `rune::time::Instant`; `wall_ms()` on `js_sys::Date::now()` on wasm (std `Instant`/`SystemTime` panic there). `data_dir_from` constant on wasm. `BackendKind::build` on wasm: `Sim` arm only for now (W2b replaces it). |
| `apps/desktop/src/net/bridge.rs`, `net/sim.rs` | Every `tokio::spawn` / `tokio::time::{sleep,timeout}` → `rt::`. wasm `BridgeThread::spawn` runs the loop with `rt::spawn`, `join` no-op; prefs saved inline on wasm; signals native-only. |
| `apps/desktop/src/prefs.rs` | wasm `PrefsStore::platform()` over `localStorage["euchre:desktop-prefs"]`; every access in try/catch semantics (a throwing `localStorage` gives defaults). |
| `apps/desktop/src/sound.rs` | wasm `web_audio` module: thread-local `AudioContext`, `recipe(cue)` samples → `AudioBuffer` → `AudioBufferSourceNode`; context resumed on the first user gesture. |
| `apps/desktop/src/assets.rs` | wasm: no `include_bytes!` art; `Art::load` uses `cache.url(format!("{base}cards/{id}.png"))` with `base = web::asset_base()` (S2). |
| `apps/desktop/src/scenes.rs` | `PortInUse { port }` literal on wasm. |
| `apps/desktop/src/lib.rs` | `run()`/`startup` native-only; `#[cfg(target_arch = "wasm32")] pub mod web;` |
| `apps/desktop/src/web.rs` (new) | `#[wasm_bindgen(start)] pub fn start()`: prefs → backend → `Bridge::channel()` → `App::new(spec()).init(..)` sending Identify from `init` (Rune refuses `on_ready` on web) → `UiSink` over `handle.update` → `BridgeThread::spawn` → `.on_visibility(..).on_quit(..).try_run()`; an `Err` calls `globalThis.euchreFallback("start")` (S3). `pub fn asset_base() -> &'static str` (S2). |
| `apps/desktop/src/web_probe.rs` (new) | The spike's probe view and `euchre_probe()`, `#[cfg(all(target_arch = "wasm32", feature = "web-proof"))]`. Not in shipping builds. |
| `Cargo.lock` | Only what `cargo` writes for the above. |

W1 gates: native `cargo fmt/clippy/test -p euchre-desktop` as before; `cargo clippy -p euchre-desktop --lib --target wasm32-unknown-unknown -- -D warnings` clean; the spike proof (`web-proof` build, sim) re-run on the final net layer with zero console errors. Then **H1**.

### 3.3 W2: WebBackend and the rivet bridge

**W2a** (phase 1, scratch crate shaped like `net/web-probe/`, against the final trait shapes):

| Path | Change |
|---|---|
| `apps/desktop/web/rivet-bridge.ts` (new) | Port of `net/bridge-js/bridge.js`: `openTable(origin, gameId, onEvent, onStatus, onError)` → `{ready, action, status, dispose}` (S4). Credentials cache copied from `apps/web/src/lib/client/rivet.ts` (refresh 10 s before `expiresAt`, single-flight). `createClient({endpoint: origin + "/rivet", encoding: "json", devtools: false, gateway: {skipReadyWait: true}, getToken})`. Drops the cached session on `forbidden`/`invalid_token`. |
| `apps/desktop/package.json` | `"rivetkit": "3.1.0"` (exact, same as `packages/rivetkit-svelte`); scripts `"bridge": "bun build web/rivet-bridge.ts --target browser --format esm --minify --outfile js/rivet-bridge.js"`, `"build:web": "bun run bridge && sh scripts/web-build.sh --out build/web"`, and `check` gains the wasm clippy line from W1's gate. `bun.lock` as bun writes it. |
| `apps/desktop/src/net/web/{mod,http,table,local}.rs` (new) | `WebBackend` (`mod.rs`), `exchange()` + `status_error` reuse (`http.rs`), `WebTable` + `TableCalls` + events/status/120 s watchdog (`table.rs`), `Local<T>` (`local.rs`). Bridge import: `#[wasm_bindgen(module = "/js/rivet-bridge.js")]`. All timers via `rt::`. |
| `apps/desktop/src/net/rivet_error.rs` (new) | One mapping for both backends: `pub struct RivetError { group, code, message, metadata }`, `pub fn move_rejection(&RivetError) -> Option<MoveAck>`, `pub fn action_error(Option<&RivetError>) -> NetError`. |

**W2b** (after H1), edits listed exactly:

- `net/mod.rs`: `#[cfg(target_arch = "wasm32")] pub mod web;` and `pub mod rivet_error;`;
  `BackendKind::build` wasm arm → `Arc::new(web::WebBackend::from_page())` (Sim only with
  `web-proof` and `?backend=sim`); `Backend` gains `identify`/`sign_out` with `NotWired` defaults;
  `Identity` gains `email: Option<String>`.
- `net/remote/table.rs`: `move_rejection`/`action_error` call `rivet_error` after
  `parse_rivet_error` (one code path, same behaviour; its existing tests must pass unchanged).
- `net/bridge.rs`: `Intent::Identify { email }` / `Intent::SignOut` plumbed to the new trait methods.

W2 gates: probe-equivalent run against a local server (every `Backend` and `TableCalls` call, illegal
move `Rejected`, other browser `Forbidden`, reconnect after a server restart ≤ 5 s after healthy);
native remote tests unchanged.

### 3.4 W3: serving, build, deploy docs

| Path | Change |
|---|---|
| `crates/euchre-http/src/static_files.rs` | `UI_COOKIE = "euchre_ui"`, `CLASSIC_SHELL = "classic.html"`; `StaticFiles { dir, classic: bool }` with `classic` checked once in `new()`; SPA fallback serves `classic.html` when `read_cookie(UI_COOKIE)` starts with `classic`; fallback gets `Vary: Cookie`; `finish()` appends `Vary: Accept-Encoding` instead of replacing. Tests: no cookie → loader; `classic`, `classic-auto`, `classic-small` → classic; cookie without `classic.html` → loader; `/index.html` by name → loader; `Vary` has both values; HEAD same headers. |
| `apps/desktop/web/index.html` (new) | Loader template with `__RUNE_HASH__`, `__ART_BASE__`. Meta: title "Euchre", description, `og:*`, `twitter:card`, absolute `https://euchre.sh/og.png`, `viewport-fit=cover`, `<noscript>` line. Inline script = NET §3.4 plus the W-5/W-6 gates, `globalThis.__EUCHRE_RUNE__ = {art: "__ART_BASE__"}`, `globalThis.euchreFallback(why)`, a `runepresent` listener with the 10 s timer, and a `MutationObserver` that treats Rune's error `<pre>` ("The GPU device was lost", "WebGPU adapter unavailable") as `euchreFallback("lost")`. Reload-loop guard in `sessionStorage`. Body background = Home's page colour, wordmark text only, no spinner. `--rune-sans`/`--rune-mono` set to DM Sans / IBM Plex Mono. |
| `apps/desktop/scripts/web-build.sh` (new) | `cargo build --release --locked --target wasm32-unknown-unknown -p euchre-desktop --lib` with Rune's profile env (`OPT_LEVEL=s`, `LTO=thin`, `CODEGEN_UNITS=1`, `DEBUG=0`, `STRIP=debuginfo`) and `--features "$RUNE_FEATURES"`; checks `wasm-bindgen --version` equals the `wasm-bindgen` version in `Cargo.lock` (grep/sed); `wasm-bindgen --target web --out-name app --remove-name-section`; copies `Inter-*.ttf` found via the cdylib's `.d` file; copies art (`apps/web/static/art/cards/*.png`, `apps/web/static/art/euchre-hero.webp`, then `apps/desktop/assets/cards/AS.png` over the Ace) into `art-<arthash>/`; fills the loader. Output tree = §2 `_rune/` + `index.html`. |
| `apps/desktop/scripts/web-site.sh` (new), `apps/desktop/scripts/precompress.ts` (new) | `web-site.sh <svelte build/client> <rune out> <dest>`: copy Svelte, rename `index.html{,.br,.gz}` → `classic.html{…}`, copy Rune output, run `bun precompress.ts` (brotli q11 + gzip 9 via `node:zlib` for html/js/wasm/ttf). Used by Docker and by local proofs. |
| `Dockerfile` | `web` stage also runs `bun run --cwd apps/desktop bridge` (one `bun install`). New `rune-web` stage `FROM chef`: `rustup target add wasm32-unknown-unknown`, `cargo install wasm-bindgen-cli --version 0.2.130 --locked`, `cargo chef cook --release --locked --target wasm32-unknown-unknown -p euchre-desktop` (same profile env as the build), `COPY --from=web` the bridge, `ARG RUNE_FEATURES=""`, `sh apps/desktop/scripts/web-build.sh --out /out/rune`. New `site` stage (bun image) runs `web-site.sh`. Runtime copies `/out/client` from `site` instead of `web`. |
| `turbo.json`, root `package.json` | `@euchre/desktop#build:web` (outputs `build/web/**`, inputs `$TURBO_DEFAULT$` + `crates/*/src/**` + `crates/*/Cargo.toml` + `apps/web/static/art/**`); root `"build:web": "turbo run build:web"`. |
| `rust-toolchain.toml` | `targets = ["wasm32-unknown-unknown"]` (orchestrator approval: root file). |
| `.gitignore`, `.dockerignore` | Ignore `apps/desktop/js/` and `apps/desktop/build/`; confirm `.dockerignore` admits `apps/desktop/**` and `apps/web/static/art/**`. |
| `apps/web/static/og.png` (new); `apps/web/src/lib/…GlobalNav` | Preview image. Optional "Try the new table" link that clears `euchre_ui` (`Max-Age=0`) and reloads, shown only when `navigator.gpu` exists. |
| `docs/DEPLOY.md` | New section: the two shells, the `euchre_ui` cookie, the §7 smoke additions and §8 rollback. Image layout line in §1. |

W3 gates: `cargo test -p euchre-http`; `docker build` cold and warm timed and recorded; the image's
`/app/build/client` matches §2 (listing in the report); curl checks of §7 step 6 against the local
image. W3 can start before W1 by using the spike's `$S/web-fetch` output as a stand-in Rune tree.

### 3.5 W4: web UX (after H1)

| Path | Change |
|---|---|
| `apps/desktop/src/url_route.rs` (new, both targets, pure) | `pub enum Deep { Home, New, Create { stick: bool, natural: bool }, Play(String), Games, Match(String), Settings, Unknown(String) }`; `pub fn parse(path: &str, query: &str) -> Deep`; `pub fn path_for(route: &Route, game_id: Option<&str>) -> String`. Native unit tests for every Svelte route, trailing slashes, percent-encoding, non-UUID ids. |
| `apps/desktop/src/web_history.rs` (new) | wasm: `push(path)`, `replace(path)`, a `popstate` listener → `handle.update(|s, tx| s.open_deep(Deep, tx))`; native: no-ops. |
| `apps/desktop/src/web.rs` | After H1: `.same_tab_router(..)` routing same-origin links; initial `open_deep(parse(location))` after Identify; `/play?stick=&natural=` creates the table then `replace("/play/<id>")`. |
| `apps/desktop/src/app.rs` | `AppState::navigate` calls `web_history::push(path_for(..))` (replace for redirects); `open_deep`; a below-720×520 holding panel ("This table needs a bigger window." + "Use the classic table") instead of the overlapping table; Settings row "Classic table" writing `euchre_ui=classic` (400 days) and reloading. Identify / "Not you?" card on Games and Settings when `identity.email` is `None` on a backend that implements it (copy from the Svelte `/games` card, byte for byte). |
| `apps/desktop/src/screens/**` | `key`/`role("button")` on every rack card (`card-<id>`, disabled when not legal), bid button, tour button and nav item; `El::href` on navigation links, so the DOM mirror is a playable, Tab-able surface. Compact table layout for 360–719 px (stretch, does not block launch). |
| `apps/desktop/assets/fonts/` + `lib.rs` `FONTS` | A suits subset (♠♣♥♦ only, an OFL source such as Noto Sans Symbols 2) with its OFL text; registered in `FONTS` for both targets. |

W4 gates: no tofu in any screenshot (wordmark, "Spades are trump", Led chip, seat chip, footer,
watermark); `url_route` tests; back/forward and reload proofs (§5 P2); native scenes unchanged.

### 3.6 W5: proofs and budgets

Owns `apps/desktop/web-proof/` (new): `full-match.mjs`, `deep-links.mjs`, `fallback.mjs`,
`frames.mjs`, `sizes.mjs`, shared `lib.mjs` (Playwright from `apps/web/node_modules`, run with bun;
`channel: 'chromium'`, `--enable-unsafe-webgpu`; frame counters from the spike's init script), and the
report `docs/port/web-rune/proof.md`. Confirms first that Playwright 1.62.0's Chromium gets the Metal
adapter with `channel: 'chromium'` (the spike ran 1.64.1).

## 4. Seams

| # | Between | Contract |
|---|---|---|
| S1 | W1 → W2, W4 | `net/rt.rs`: `pub fn spawn<F: Future<Output = ()> + Send + 'static>(F) -> Task`; `Task::abort(&self)`, `Task::join(self)` (no-op wait on wasm); `pub async fn sleep(Duration)`; `pub async fn timeout<F: Future>(Duration, F) -> Result<F::Output, Elapsed>`. `net::now_ms()`/`wall_ms()` are safe on wasm. No `tokio::` calls outside `rt.rs` in `net/` (grep gate). |
| S2 | W1 ↔ W3 | Asset base: the loader sets `globalThis.__EUCHRE_RUNE__ = {art: "/_rune/immutable/art-<arthash>/"}` before importing `app.js`; `web::asset_base()` reads it once (fallback `"/_rune/art/"` logs a console warning). Art file names = today's `apps/web/static/art` names. |
| S3 | W1/W4 → W3 | `globalThis.euchreFallback(why: string)`: defined by the loader; sets `classic-auto` and reloads (or swaps in place). The app calls it on a failed `try_run()`. |
| S4 | W2 bridge ↔ `net/web/table.rs` | NET §2.3 API, unchanged: `openTable(origin, gameId, onEvent(name, json), onStatus(status), onError(json))` → `{ready(): Promise<actorId>, action(name, argsJson): Promise<resultJson>, status(), dispose()}`; errors `{group, code, message, metadata, status}`; `client/closed` when not connected. Bundle path `apps/desktop/js/rivet-bridge.js` must exist before `cargo build` (wasm-bindgen reads it at compile time). |
| S5 | W2b → W4 | `Backend::identify(&self, email: &str) -> Result<Identity, NetError>`, `Backend::sign_out(&self) -> Result<(), NetError>`, `Identity.email`. `NotWired` hides the card. |
| S6 | W4 → W5 | Mirror keys: `bid-buttons/bid-<id>`, `rack/card-<id>` (disabled when illegal), `nav-play`, `nav-games`, `nav-settings`, `tour-skip`. Stable across releases; renaming one is a breaking change for the proofs. |
| S7 | W3 → server | Cookie name and values, `classic.html` file name, `/_rune/immutable/` prefix. Changing any needs a loader + server change in one release. |

## 5. Proofs that count as done (W5, on the shipping artifact)

Production-shaped mode = the `docker build` image booted as DEPLOY §7 (`NODE_ENV=production`,
`APP_URL=http://localhost:3000`, separate 3.1.0 engine container, real gateway tokens, brotli from
`/app/build/client`), plus `EUCHRE_FAST_TEMPO=1` for time. Every screenshot is read. Allowed console
lines: the "[DOM] Password field" notice, and rivetkit "failed to reconnect" during P5 only.

| # | Proof | Pass |
|---|---|---|
| P1 | **Full match.** Fresh context, tour skipped via the mirror, play every bid and card through S6 mirror buttons to 10 points. | Game-over screen; `GET /api/games?profile=browser` from the page lists the match with the same final score; the same match opens at `/games/<id>` in Rune and, with `euchre_ui=classic`, in Svelte. Zero console errors. |
| P2 | **Deep links.** `/`, `/new`, `/play?stick=true&natural=false`, `/play/<own id>`, `/games`, `/games/<id>`, `/settings`, `/play/not-a-table`; then back, forward, reload on `/play/<id>`. | Right screen each time (mirror heading + screenshot); reload resumes the same table at `v` ≥ before; no 404 in the network log (catches page-relative art); another context on `/play/<id>` shows the "belongs to another browser" copy. |
| P3 | **Fallback.** (a) `navigator.gpu` deleted; (b) `requestAdapter` → null; (c) default headless shell (SwiftShader, fallback adapter); (d) `?rune=1` on SwiftShader (device lost after frame one); (e) cookies blocked; (f) 390×844 viewport; (g) Rune Settings → Classic table, then Svelte "Try the new table". | (a)–(d) Svelte at the same URL after at most one reload, cookie `classic-auto`; (e) Svelte in place, Svelte navigation works; (f) Svelte, session cookie `classic-small`; (g) cookie `classic` 400 days, then cleared and Rune again. Never a second reload, never a blank page. |
| P4 | **Frames.** Probe-free runs at 1280×800 DPR 2 against the server: the first deal, a trick sweep; idle windows on Home, New game, Games, Settings, a table at rest, your turn, your turn with reduced motion. | §6 frame rows. |
| P5 | **Reconnect.** `docker restart` the app container mid-hand. | Rune shows the reconnect state, play resumes ≤ 5 s after `/health` is ok, no move lost or doubled. |
| P6 | **Sizes and headers.** From the image's `/app/build/client` and curl against it. | §6 size rows; `application/wasm`, `content-encoding: br`, immutable cache on `_rune/immutable`; `Vary: Cookie, Accept-Encoding` on the shells; `classic.html` byte-identical to the Svelte build's `index.html`. |
| P7 | **Gates.** `bun run check`, `bun run test`, `bun run build`, `bun run build:web` from the root. | All green, including the wasm clippy line. |

## 6. Budgets (measured, with headroom)

| What | Budget | Spike / NET measurement |
|---|---|---|
| `app_bg.wasm` brotli | ≤ 4.6 MB | 4.36 MB (sim, art by URL) |
| `app.js` + bridge brotli | ≤ 80 KB | 16.5 KB + 58.7 KB |
| First frame, cold, localhost | ≤ 200 ms | 108–120 ms |
| First frame on euchre.sh | measured and recorded at deploy; no budget yet | — |
| Deal and sweep frames | p50 ≤ 16.8 ms, p95 ≤ 18 ms, 0 frames over 20 ms | p95 17.4–17.6, max 17.7–18.7 |
| At rest (Home, New game, Games, Settings, table waiting on others) | 0 GPU submits over 3 s | 0 |
| Your turn | ≤ 31 submits/s; reduced motion ≤ 1 per 3 s | ~20/s; 1 per 3 s |
| Docker build | cold and warm recorded by W3; warm (euchre code only) ≤ 2× today | cold wasm 72 s (spike, M4 Pro) |

## 7. Deploy

No `.railway/railway.ts` change: no new variable, no engine change. Only the image changes.

1. Checkout of the merged commit, clean tree. `bun install --frozen-lockfile`, then P7.
2. `docker build -t euchre-local .` and boot as DEPLOY §7. Run P1–P6 against `http://localhost:3000`.
3. Optional sanity: `railway config plan` shows no change.
4. A human runs the DEPLOY §3.3 step 4 `railway up … --detach` command. Railway builds, waits for
   `/health`, overlaps 60 s, drains the old server for 120 s. Tables live in the engine, so open
   games continue; sockets reconnect as on any deploy (~1.6 s after healthy, measured). Open
   Svelte tabs keep running until they reload, then follow the loader.
5. Smoke (DEPLOY §5 plus):
   - `curl -sI https://euchre.sh/games` → `vary: Cookie`, body is the loader.
   - `curl -s -H 'Cookie: euchre_ui=classic' https://euchre.sh/games | grep -c '/_app/immutable/'` → ≥ 1.
   - `curl -sI -H 'Accept-Encoding: br' https://euchre.sh/_rune/immutable/<hash>/app_bg.wasm` →
     `application/wasm`, `content-encoding: br`, `max-age=31536000,immutable`.
   - Browser: Chrome on a Mac plays a hand in Rune; Safari and Firefox land on whichever shell they
     can draw, with no blank page; a phone gets Svelte; "Your games" shows the same history in both.
6. Record first-frame time and wasm transfer time from euchre.sh in `docs/port/web-rune/proof.md`.

## 8. Rollback

- **Whole release:** Railway → `euchre` → Deployments → redeploy the previous deployment (Svelte only).
  No build, no config change. The old server ignores `euchre_ui`, every URL is a Svelte route, and
  games in progress continue.
- **One browser:** Rune Settings → Classic table, or a `euchre_ui=classic` cookie.
- **One browser family broken on Rune (for example Safari):** a one-line loader gate (UA or adapter
  vendor) shipped as a normal release.
- Then revert the merge on the main branch so a later `railway up` does not bring it back.

## 9. Risks

1. **Real-device WebGPU** (Safari, Firefox, Windows, older GPUs) is untested; only Chromium on Metal is.
   Mitigation: the W-5 gates, the manual §7 step 5 checks, the Classic setting, and the per-family
   loader gate.
2. **Download on slow networks.** 4.4 MB brotli before the first frame; art after. The loader shows the
   wordmark meanwhile. The wasm is one module and cannot be split; if the euchre.sh first frame is
   poor, the levers are the app fonts (1.9 MB raw / 0.42 MB brotli inside the wasm) and code size.
3. **Fallback detection relies on Rune's error `<pre>` text** for a lost device. A Rune upgrade can change
   it; P3(d) catches that. Request an `on_gpu_lost` hook upstream.
4. **Phones get Svelte** until the compact layout lands (W-7). This is a product limit, not a bug.
5. **The DOM mirror lags one layout commit** and hides the table while the tour is open; proofs wait for
   settle and skip the tour. Screen-reader users get a thinner surface than Svelte until W4's keys land.
6. **wasm breakage by native-only code.** Any later `std::time`, `tokio::spawn` or new native dependency in
   `apps/desktop` breaks the web build. The wasm clippy line in `check` catches it.
7. **Two copies of client rules** (bridge credential cache; error mapping). `rivet_error.rs` removes the
   second; pin `rivetkit` to the engine's exact version.
8. **Unsafe `Send`/`Sync` in `Local<T>`** is sound only on single-threaded wasm; the cfg makes a threaded
   build fail to compile. Never enable `atomics`.
9. **Frozen and root files** (`euchre-ai`, `rust-toolchain.toml`) need orchestrator approval before W1/W3.
10. **Railway build time** grows by the wasm compile on a cold cache; W3 measures locally first.
11. **Cookie-varied HTML** must not sit behind a CDN that ignores `Vary`. There is no CDN today.
12. **Turn-halo battery cost** (~20 fps while it is your turn) is accepted for now; revisit if reports
    come in.

## 10. Corrections to the lane docs

- network-and-serving §4.2 placeholders: Rune `web-showcase` cold wasm build 156 s, rebuild after a touch
  13 s (machine under load 19–29), output 10,268,333 B raw / 3,772,190 B brotli (checked).
- network-and-serving §2.4/§2.5 say tokio `sync` only; the bridge's `select!` needs `["sync", "macros"]`
  (spike, compiled).
- network-and-serving §2.5 says mount with `App::full_viewport()`; Rune's default canvas already covers the
  viewport, so neither `canvas_id` nor `full_viewport` is used.
- spike §3 `assets.rs` uses page-relative `art/…` and puts the desktop Ace at Svelte's `/art/cards/AS.png`;
  replaced by W-8.
- network-and-serving §6 says nothing of that lane is still running; a `rivet-engine` it started was
  still listening on 127.0.0.1:6930/6931/6935 at review time.

## 11. Tools

Already on this machine (no new installs for this plan): the `wasm32-unknown-unknown` target on 1.99.1,
wasm-bindgen-cli 0.2.130 at `~/.local/wasm-bindgen-0.2.130/bin` (spike; put it first on `PATH`),
`brotli`, bun 1.4.4, Playwright 1.62.0 in `apps/web/node_modules`. Python and `wasm-opt` are not needed.

## 12. Owner decisions after this plan (2026-11-11) — read before building

These amend W-4, §2 and §3.4 (ARCHITECTURE D36):

1. **`/` is the minimal homepage for every browser** (wordmark, cards picture, Play now, Download), built by `apps/web/src/routes/+page.svelte`. It is not the Rune loader and runs no WebGPU check on load. Rename the loader (e.g. `rune.html`); the server serves it for the app routes (`/new`, `/play/<id>`, `/games`, `/games/<id>`, `/settings`, unknown SPA paths) unless a `euchre_ui=classic*` cookie is set, and `classic.html` (the Svelte fallback page) otherwise. Check how adapter-static names the prerendered home and the SPA fallback after the homepage change before choosing file names.
2. **Play now** calls `startPlaying()` (the homepage lane's module). In this wave it runs the loader gates on click: pass → the Rune app's new-game route; fail → set the classic cookie and keep the Svelte flow.
3. **On the web the Rune app never shows its marketing Home**; its wordmark and Home link go to `/`.
4. **`/download/*`** (desktop downloads, `docs/port/downloads/serve.md`) must keep working; `static_files.rs` changes merge with the download routes, not replace them.
5. **Approved:** the `euchre-ai` cfg gates and the `rust-toolchain.toml` wasm target.
6. **Order:** this build starts after the homepage + downloads release is deployed, because both touch `apps/web`, `crates/euchre-http`, the `Dockerfile` and `apps/desktop`.
