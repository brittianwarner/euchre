# Rune on the web: network and serving

Lane NETWORK + SERVING, 2026-11-11. Goal: euchre.sh shows the Rune game compiled to
WebAssembly and drawn with WebGPU. A browser that cannot draw it gets the current Svelte
site. Both run on the same server, with the same cookies and the same match history.

Every number below was measured on this machine. Load average was 15 to 34 on 14 cores
while other lanes built, so treat build times as upper bounds. The probes live in
`/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/webspike/net/`
(called `net/` below). Nothing in this repo changed except this file.

## 1. Decisions

1. **Transport: the official rivetkit 3.1.0 JS client, behind a small bridge (option b).**
   A wasm-bindgen module snippet wraps `rivetkit/client`, configured exactly like
   `apps/web/src/lib/client/rivet.ts`. Rust sees five calls and three string callbacks.
2. **HTTP: `fetch` from web-sys with `credentials: "same-origin"`.** The browser keeps the
   HttpOnly guest cookie; Rust never touches a token except through the bridge.
3. **Serving: one URL space and two shells, picked by a cookie.**
   - `index.html` is the Rune loader. It is what every crawler and every new browser gets.
   - The Svelte build is served as `classic.html` when the browser has the `euchre_ui=classic*`
     cookie.
   - The loader sets that cookie and reloads when `navigator.gpu` is missing or
     `requestAdapter()` returns null.
   - If cookies are blocked, the loader swaps `classic.html` in place instead.
   - `/play/<id>`, `/games`, `/games/<id>` and `/settings` stay the same URLs in both shells.
4. **Rune files go under `/_rune/immutable/<hash>/`.** The server already serves them as
   `application/wasm` with brotli and a one-year immutable cache. Only the SPA fallback needs a
   change of about 15 lines.
5. **Build: a `rune-web` Docker stage parallel to `server`**, from the same cargo-chef image,
   plus a small bun stage for the bridge and a bun `site` stage that assembles and
   precompresses `build/client`.

## 2. How the wasm build talks to the server

### 2.1 The three options, measured

| Option | Result | Evidence |
| --- | --- | --- |
| (a) `rivetkit-client` 3.1.0 (Rust) for wasm32 | **Does not compile. Rejected.** | `net/probe-rivetkit/`, log `net/probe-rivetkit-wasm.log`. After the getrandom `js` / `wasm_js` fixes it still fails with 59 error lines: mio 48 ("This wasm target is unsupported by mio", pulled in through tokio `full`), rustls 7, rustls-native-certs 1. The crate hard-wires `WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>` (`common.rs:28`, `drivers/ws.rs`), `tokio::spawn` (`connection.rs`, `drivers/ws.rs`), `tokio::time::sleep` (`backoff.rs`) and `reqwest` (`remote_manager.rs`, `handle.rs`). A browser has no TCP socket, so this needs a fork of the client. |
| (b) Official rivetkit JS 3.1.0 client through a wasm-bindgen snippet | **Works end to end. Chosen.** | `net/web-probe/` (Rust) + `net/bridge-js/bridge.js`, driven by `net/driver/drive.mjs` against a real `euchre-server` (§2.6). |
| (c) The actor protocol over web-sys WebSocket + fetch (JSON) | Possible, rejected | We would re-implement what rivetkit owns: the `/rivet/connect` handshake with three subprotocols and the token in a subprotocol, action ids, event subscription, reconnect with backoff, and gateway-token refresh. No one upstream owns that copy, and the owner asked for the Rivet client. Keep it only as the fallback if a later rivetkit JS release breaks the bridge. |

Why (b) is the most reliable choice:

- It is the same client and the same configuration the live Svelte site uses today, so
  reconnect, token refresh and the wire format are already proven in production.
- Upgrades come from `bun update rivetkit` with no Rust changes.
- It costs 58.7 KB brotli (237 KB raw) of JS. That loads in parallel with the wasm.

### 2.2 The auth path (checked in `crates/euchre-http` and measured)

1. `POST /api/tables` with body `{stick, natural}` returns `201 {gameId}`. It mints the
   `euchre_guest` cookie if the browser has none, and adds the table to the
   `euchre_recent_tables` cookie.
   - Both cookies are `HttpOnly; SameSite=Lax; Path=/; Max-Age=34560000`, plus `Secure` when
     `APP_URL` is https. They are host-only.
   - Measured in Playwright: both cookies set, `httpOnly: true`, `sameSite: Lax`.
2. State changes from a cookie caller need `Origin` equal to the public origin
   (`require_same_origin`). The browser sends `Origin` on same-origin POST, PATCH and DELETE
   `fetch`; every POST in the probe passed.
3. `POST /api/table-session/{gameId}` returns `{actorId, token, gatewayToken, expiresAt}`
   with `Cache-Control: no-store`.
4. The bridge dials `<origin>/rivet` and passes these to rivetkit:
   - `getToken({forceRefresh})` returns `gatewayToken`;
   - `getParams()` returns `{token}`.
   The cache is the same as `rivet.ts`: refresh 10 s before `expiresAt`, single-flight.
5. Gateway admission (`gateway/public.rs`), in order:
   - the path is `/rivet/connect` with three subprotocols;
   - the Origin is allowlisted;
   - a browser (Origin present) needs a guest session (cookie);
   - then the per-IP bucket.

   Measured: every socket was `ws://127.0.0.1:6938/rivet/connect`.

### 2.3 Bridge API (`apps/desktop/web/rivet-bridge.ts`, bundled by bun)

The prototype is `net/bridge-js/bridge.js`. The bundled output is `net/web-probe/js/rivet-bridge.js`.

```js
openTable(origin, gameId, onEvent, onStatus, onError) -> {
  ready(): Promise<actorId>;  // rejects with error JSON (a table-session failure carries `status`)
  action(name, argsJson): Promise<string /* result JSON */>;
  // rejects with {group, code, message, metadata, status}
  // rejects at once with {group:"client", code:"closed"} unless connStatus === "connected"
  status(): "idle" | "connecting" | "connected" | "disconnected";
  dispose(): Promise<void>;
}
```

- `onEvent(name, json)` fires for each of `sync`, `thinking`, `chat`, `chatDelta` and
  `presence`.
- `onStatus(status)` fires on rivetkit `onStatusChange`.
- `onError(json)` fires on rivetkit `onError`. A refused re-mint during a reconnect also calls
  it, as `{group:"session", code:"http_<status>", status}`.
- On a `forbidden` or `invalid_token` error, the bridge drops its cached session, so the next
  reconnect mints fresh tokens (the desktop's `Control::Refresh`).
- The client is created with `encoding:"json"`, `devtools:false` and
  `gateway:{skipReadyWait:true}`.
- Rust imports it with `#[wasm_bindgen(module = "/js/rivet-bridge.js")]`. wasm-bindgen reads
  the file at compile time and emits it as `snippets/<crate>-<hash>/js/rivet-bridge.js`
  beside `app.js`. So the bundle must exist before `cargo build`. Generate it; do not commit
  it (§4).

### 2.4 `WebBackend`: the desktop traits on wasm

The source is `net/web-probe/src/{local,time,bridge,web}.rs`. `net.rs` is a byte-for-byte
copy of the trait shapes in `apps/desktop/src/net/mod.rs`. It compiles for
`wasm32-unknown-unknown` with the real `#[async_trait] Backend: Send + Sync` and
`TableCalls: Send + Sync` bounds, and `Arc<dyn Backend>` is used as is.

**Send + Sync.** `JsValue`, `JsFuture` and `Closure` are `!Send`. Every method body that holds
one across an `.await` runs inside `Local`:

```rust
pub struct Local<T>(pub T);
#[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
unsafe impl<T> Send for Local<T> {}   // one thread: nothing can be sent anywhere
#[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
unsafe impl<T> Sync for Local<T> {}
impl<F: Future> Future for Local<F> { /* structural pin of field 0 */ }

async fn open_table(&self, game_id: &str) -> Result<TableConn, NetError> {
    Local(WebTable::open(&self.origin, game_id)).await   // without Local: "future cannot be sent"
}
```

The compiler checked this: without the wrapper, `open_table` failed with "future cannot be
sent between threads safely". The impls exist only on single-threaded wasm, so a native or
threaded-wasm build fails to compile instead of becoming unsound. Use this in place of the
`send_wrapper` crate, which is not in the lockfile.

**Timers** (`time.rs`):

- `setTimeout` is imported from the global scope, so it works in a window or a worker.
- `sleep_ms(ms)` is a Promise around `setTimeout`.
- `timeout(ms, fut)` uses `futures::future::select`.
- `now_ms()` is `performance.now()`; `wall_ms()` is `Date::now()`.
- Background loops use `wasm_bindgen_futures::spawn_local`.
- On wasm, tokio is only `default-features = false, features = ["sync"]`, for `mpsc`.

**HTTP** (`exchange()` in `web.rs`):

- `RequestInit` with the method, `RequestCredentials::SameOrigin`,
  `AbortSignal::timeout_with_u32(15_000)`, `Accept: application/json` and a JSON body.
- A non-2xx reply reads `error.code` from `{message, error:{code, message}}`.
- The status maps through the desktop's own `status_error` (copied unchanged).
- A network failure maps to `Offline`. A `TimeoutError` DOMException maps to `Timeout`.

| `Backend` method | Request (cookie) | Measured (ms) |
| --- | --- | --- |
| `name()` | `"web"` | |
| `identity()` | `GET /api/session` → `{guestUserId, identity:{email,userId}}`. Returns the email identity if present, else the guest, else `Err(Unauthorized)`. | 1–9 (returned `Unauthorized` before the first table, a guest after it) |
| `create_table(cfg)` | `POST /api/tables {stick, natural}` → `201 {gameId}`. Retries `503` with backoff from 250 ms up to 2 s, for at most 20 s, like the desktop. | 9–31 |
| `open_table(id)` | `bridge.openTable` → `ready()` within 10 s → `TableConn` | session ready 94–114, connected 106–131, first `sync` 107–135 |
| `list_games(cursor)` | `GET /api/games?profile=browser&limit=10[&cursor]` | 76–136 |
| `match_detail(id)` | `GET /api/games/{id}?profile=browser` | 3–5 (`NotFound` for an unknown id) |
| `stats()` / `get_settings()` | `GET /api/profile/{stats,settings}?profile=browser` | 4–6 |
| `patch_settings(p)` | `PATCH /api/profile/settings?profile=browser` | (same route as the desktop; not run) |
| `recent_tables()` | `GET /api/tables/recent` with no `ids`, so the server reads the recent-tables cookie | 24–201 |
| `host()` | `None` (default) | |

**`TableCalls`.** Each call fails at once with `Closed` while the socket is down (measured
0 ms). Otherwise it calls `bridge.action(name, JSON(args))` with a 10 s bound.

| Call | Action | Measured (ms) |
| --- | --- | --- |
| `snapshot()` | `snapshot` with `[]` | 2–5 |
| `submit_move(req)` | `submitMove` with `[req]` | legal 35–48 (`Ok { v }`); illegal 5–6 (`Rejected { IllegalMove, .. }`) |
| `review_trick(req)` | `reviewTrick` with `[req]` | 8–23 (`queued: true`) |
| `get_hand_review(n)` | `getHandReview` with `[n]` | 3–6 |
| `close()` | marks the table ended, then `dispose()` (2 s bound), then drops the callbacks. Dropping them after dispose means JS never calls a freed closure. | 1–6; a call after close returns `Closed` |

**Error mapping.** These copy `remote/table.rs`, but read the JS error object instead of
`parse_rivet_error`:

- `move_rejection`: a `user/<ErrorCode>` error that is not `forbidden` becomes
  `MoveAck::Rejected { code, legal: metadata.legal or [] }`.
- `action_error`:
  - `forbidden` → `Forbidden`;
  - any other `user/*` → `Invalid(code)`;
  - `client/closed` → `Closed`;
  - any other structured error → `Unavailable`;
  - no error → `Closed`.

**Events and status.**

- Events: the `onEvent(name, json)` callback runs `TableEvent::decode` and sends
  `ConnEvent::Event`. A bad payload logs a console warning.
- Status: `onStatus` sends `ConnEvent::Status(true)` on the first `connected`, and
  `Status(false)` once when a connected socket leaves `connected`.
- Final errors: a `{group:"session"}` error with a 403, 404/400 or 401 ends the table with
  `Ended(Forbidden | NotFound | Unauthorized)`. rivetkit retries everything else by itself.
- Watchdog: rivetkit JS retries forever, so a `spawn_local` watchdog ends the table with
  `Ended(Unavailable)` after `GIVE_UP_MS = 120_000` without a connection. That is the desktop
  supervisor's limit.
- The desktop's own backoff, re-dial and per-reconnect client are not needed. The JS client
  re-runs `getToken` and `getParams` on every reconnect (the reason the desktop rebuilds its
  client).

### 2.5 What `apps/desktop` must change to build for wasm (for the lanes that own it)

- `net::now_ms()` uses `std::time::Instant` and `wall_ms()` uses `SystemTime`. Both panic on
  wasm32-unknown-unknown. Use `rune::time::Instant` (or the `time.rs` above) and
  `js_sys::Date::now()`.
- `bridge.rs` runs a tokio multi-thread runtime on its own thread and uses `tokio::spawn`,
  `tokio::time`, `spawn_blocking`, signals and `std::thread`. On wasm, the same Intent/Reply
  loop runs on `spawn_local`, with the `sleep_ms` / `timeout` shims. `UiSink`
  (`AppHandle::update`) is unchanged; on web it takes `Send` closures, and those are
  satisfied the same way.
- `remote/table.rs` `move_rejection` / `action_error` depend on
  `euchre_server_core::rivet::parse_rivet_error`. On wasm, use the JS-object versions above.
- Gate these Cargo deps off wasm with `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`:
  - `euchre-server`, `euchre-server-core`, `rivetkit-client`, `rivetkit-engine-process`;
  - `reqwest`, `rodio`, `directories`;
  - tokio `full`.

  Add for wasm: `wasm-bindgen = "=0.2.130"` (the lockfile version), `wasm-bindgen-futures`,
  `js-sys`, and `web-sys` with features `Window`, `Location`, `Document`, `Element`,
  `Headers`, `RequestInit`, `RequestCredentials`, `Response`, `AbortSignal`, `Performance`,
  `Crypto` and `DomException`. Also tokio with `sync` only.
- `[lib] crate-type = ["cdylib", "rlib"]` and a `#[wasm_bindgen(start)]` entry point that
  mounts with `App::full_viewport()` and `same_tab_router`. `BackendKind::build` returns
  `WebBackend` on wasm.
- `crypto.randomUUID()` (web-sys `Crypto`) makes `clientMoveId`. 127.0.0.1 and https are
  secure contexts, so it is available.

### 2.6 End-to-end proof

`net/driver/drive.mjs`, `forbid.mjs` and `nocookie.mjs` run headless Chromium from the
Playwright 1.62.0 in `apps/web`, with `--enable-unsafe-webgpu`, against the deploy build of
`euchre-server` (port 6938, engine on 6930/6931/6935, `EUCHRE_FAST_TEMPO=1`). The wasm probe
used the cookie path for every `Backend` and `TableCalls` method.

- **Game:** the probe created a table, connected, received `sync` (and `thinking`, `chat` and
  `presence`), took a snapshot, waited for its turn and submitted a legal move (`Ok { v }`).
  An illegal move came back `Rejected(IllegalMove)`.
- **Reconnect** (the lane's own server, restarted with SIGTERM like a deploy):
  - the restart took 14.3 s;
  - `Status(false)` arrived 47 ms after the restart began;
  - calls returned `Closed` while the socket was down;
  - `Status(true)` and a fresh `sync` arrived 15.96 s after the restart began (about 1.6 s
    after the server was healthy again);
  - `snapshot` after the reconnect took 5 ms.

  Playwright `setOffline(true)` did **not** drop the WebSocket in 90 s, so test reconnects
  with a server restart, not with offline emulation.
- **Ownership:**
  - another guest's table: `open_table` → `Forbidden`;
  - an unknown id → `Unavailable` (the server answers 503, the same mapping as the desktop);
  - your own table → opens.
- **Console noise:** rivetkit JS logs "unhandled action promise rejection" for a rejected move,
  and "failed to reconnect" during a restart. This is harmless, but it is noise in the
  console.

### 2.7 Gaps to decide

- **Identity.** The Svelte site has "Remember me here" (`POST /api/identity/identify {email}`)
  and "Not you?" (`DELETE /api/identity`). `Backend` has neither.
  - Proposal: add `identify(email)` and `sign_out()` to `Backend`, defaulting to
    `Err(NotWired)`, and implement both on web.
  - Until then, Rune web users cannot attach an email. Cookies set from the Svelte site still
    apply, because the server reads them.
- **Profile choice.** The desktop always reads `profile=browser`. On web, a user who
  identified in Svelte sees their email scrapbook today.
  - The server defaults to the email profile for settings and stats, and the Svelte `/games`
    page also lists the browser guest's games.
  - Proposal: `WebBackend` sends `profile=email` when `identity()` saw an email, else
    `profile=browser`, so the history matches what the Svelte site shows.

## 3. Serving on euchre.sh

### 3.1 Layout of `build/client`

```
index.html                         Rune loader (no-cache; crawlers and new browsers get this)
classic.html{,.br,.gz}             the Svelte adapter-static fallback, renamed (unchanged build)
_app/immutable/...                 Svelte assets (unchanged, absolute /_app paths)
art/..., robots.txt                Svelte static files (unchanged)
_rune/immutable/<hash>/app.js{,.br,.gz}
_rune/immutable/<hash>/app_bg.wasm{,.br,.gz}
_rune/immutable/<hash>/snippets/<crate>-<hash>/js/rivet-bridge.js{,.br,.gz}
_rune/immutable/<hash>/Inter-{Regular,Medium,SemiBold}.ttf{,.br,.gz}
og.png                             link-preview image
```

The Svelte fallback references `/_app/immutable/...` with `base: ""` (checked in
`apps/web/build/client/index.html`). So it boots unchanged from any URL under any file name.
The Svelte build needs **no source change**.

`<hash>` is the first 16 hex characters of the sha256 of `app_bg.wasm`, `app.js` and the
snippet. web-build.sh writes fixed names, so the directory carries the hash.

### 3.2 What already works with no server change (curl against the live code)

| Request | Result |
| --- | --- |
| `GET /_rune/immutable/probe/app_bg.wasm` | `content-type: application/wasm`, `cache-control: public,max-age=31536000,immutable` |
| same, with `Accept-Encoding: br, gzip` | `content-encoding: br`, 119,960 B (raw 527,853), `vary: Accept-Encoding` |
| same, with `Accept-Encoding: gzip` | `content-encoding: gzip`, 150,536 B |
| `app.js`, `rivet-bridge.js` with br | `text/javascript`, br, immutable |
| `/play/abc`, `/games`, `/settings`, `/` | the root `index.html`, `no-cache`, `frame-ancestors 'none'`, `X-Frame-Options: DENY` |

### 3.3 The one server change: the shell picked by a cookie

The diff was applied in a scratch copy and built: the native release build of `euchre-server`
took 2 min 3 s, rebuilding rivetkit and the euchre crates. The diff is `net/static_files.patch`.

```rust
pub const UI_COOKIE: &str = "euchre_ui";          // "classic" (chosen) | "classic-auto" (detected)
pub const CLASSIC_SHELL: &str = "classic.html";

let classic = crate::cookies::read_cookie(req.headers(), UI_COOKIE)
    .is_some_and(|v| v.starts_with("classic"));
// in the SPA-fallback branch:
let shell = if classic && self.dir.join(CLASSIC_SHELL).is_file() { CLASSIC_SHELL } else { "index.html" };
// ...and on the fallback response:
res.headers_mut().append(header::VARY, HeaderValue::from_static("Cookie"));
// finish(): `.insert(VARY, "Accept-Encoding")` becomes `.append(...)` so both Vary values survive.
```

- Check `classic.html` once in `StaticFiles::new`, not per request. The prototype calls
  `is_file()` per request for brevity.
- Measured with the patch:
  - `/games/m1` with no cookie → the Rune loader, `vary: Cookie`;
  - with `euchre_ui=classic-auto` → the Svelte shell;
  - with br → `content-encoding: br`, `vary: Cookie, Accept-Encoding`;
  - `/index.html` fetched by name → always the loader;
  - `/_app/...` → unchanged, immutable.
- Tests to add in `static_files.rs` / `api/tests.rs`:
  - no cookie → loader;
  - `classic` and `classic-auto` → shell;
  - a cookie with no `classic.html` on disk → loader;
  - `Vary` carries both values;
  - HEAD has the same headers.

### 3.4 The loader (`index.html`), as tested

```html
<script>
(function () {
  function classic(why) {
    var secure = location.protocol === "https:" ? "; Secure" : "";
    document.cookie = "euchre_ui=classic-auto; Path=/; Max-Age=604800; SameSite=Lax" + secure;
    if (document.cookie.indexOf("euchre_ui=classic") < 0) {      // cookies blocked
      fetch("/classic.html", { credentials: "same-origin" }).then(function (r) { return r.text(); })
        .then(function (html) { document.open(); document.write(html); document.close(); });
      return;
    }
    try {                                                         // reload-loop guard
      if (sessionStorage.getItem("euchre.reloaded")) return;
      sessionStorage.setItem("euchre.reloaded", why);
    } catch (e) {}
    location.reload();                                           // server now sends classic.html
  }
  if (!("gpu" in navigator)) return classic("no-gpu");
  window.__adapter = navigator.gpu.requestAdapter().then(function (a) {
    if (!a) classic("no-adapter");
    return a;
  }, function () { classic("adapter-error"); return null; });
})();
</script>
<!-- then: <script type="module"> await window.__adapter; import("/_rune/immutable/<hash>/app.js") -->
```

- The detected cookie lasts 7 days, so a browser that gains WebGPU later moves to Rune.
- An explicit "Classic table" choice in Rune settings writes `euchre_ui=classic` for 400
  days.
- A "Try the new table" link on the Svelte site clears the cookie (`Max-Age=0`) and reloads.
  That link is the one optional Svelte edit.
- The cookie is not HttpOnly on purpose: it is a UI preference, not a credential.

Measured in headless Chromium:

| Case | Result |
| --- | --- |
| WebGPU on: `/`, `/play/abc123`, `/games`, `/games/m1`, `/settings` | The Rune page stays at the same URL; adapter ready 12–49 ms after navigation |
| No `navigator.gpu`: first visit to `/games` | Cookie set, one reload, Svelte `/games` rendered after 206 ms, same URL |
| No `navigator.gpu`: later visits to `/`, `/games`, `/settings` | The server sends Svelte directly: 20–71 ms to text |
| `requestAdapter()` returns null | Svelte at the same URL |
| No WebGPU and cookie writes dropped | In-place swap: Svelte `/games` after 128 ms; clicking Settings routes to `/settings` in the client |
| Headless Chromium adapter | `swiftshader`, `isFallbackAdapter: true` (see the risks) |

Screenshots, all read: `net/shots/probe.png` (full probe report), `nogpu-1..5.png` (Svelte
`/games`, `/`, `/games`, `/settings`, `/play/not-a-table` at the same URLs) and
`nocookie.png`.

### 3.5 Rejected: Svelte under `/classic` via `kit.paths.base`

- **The current server loops on it.** Its SPA fallback answers `/classic/*` with the root
  `index.html`, which is the loader. Measured: the no-WebGPU redirect went `/` →
  `/classic/` → `/classic/classic/` → … for 12 or more navigations. A prefix-aware fallback
  plus a loop guard would be required.
- **The Svelte source would need about 25 edits.** It hard-codes root paths: `href="/new"`,
  `"/games"`, `"/settings"`, `"/"`, `` `/play/...` ``, `action="/play"`, and `` src={`/art...`} ``
  in `+page.svelte`, `GlobalNav`, `ScoreBoard`, `HandReview`, `TableStage` and `CardRack`.
  Each would need `base` or `resolve()`.
- **Links would diverge.** A link copied from the classic site (`/classic/games/x`) keeps a
  WebGPU user on Svelte, and the reverse.
- **Every first visit pays a redirect.** The cookie design pays one reload on the first visit
  only.

### 3.6 Deep links

- Rune routes itself on the same paths with `App::same_tab_router` (pushState, WEB.md).
  `/play/<id>` opens `open_table(id)`; `/games`, `/games/<id>` and `/settings` map to the
  game's screens.
- The server returns the loader for any path with no extension. A route that does not exist
  is handled by the app, as it is in Svelte today.
- One catch: `/games/xyz.1` (a dot in the last segment) is a 404 at the server today in both
  shells. Match ids are UUIDs, so this is not a problem.

### 3.7 CSP

- Today HTML gets only `frame-ancestors 'none'`. There is no `script-src`, so wasm compiles.
- If a `script-src` is ever added, it needs:
  - `'wasm-unsafe-eval'`, or `WebAssembly.instantiateStreaming` is blocked;
  - a hash or nonce for the loader's inline script and the Svelte bootstrap script;
  - `connect-src 'self'` (fetch and `ws(s)://` to the same origin are covered by `'self'` in
    current browsers).
- No COOP or COEP headers are needed (WEB.md: no SharedArrayBuffer).

### 3.8 SEO and link previews

- Crawlers and preview bots never have the `euchre_ui` cookie, so they always get the loader.
  Put `<title>`, `description`, `og:title`, `og:description`, `og:image` (an absolute
  `https://euchre.sh/og.png`) and `twitter:card` in the loader template, plus a short
  `<noscript>` line.
- Rune's `--html` export (WEB.md "A static first frame") is built for `web_document()` pages.
  It runs the app natively, so the image would need a second, native Rune build. A game
  table's first frame is also not useful crawler text. Do not use it for the game. Revisit
  only if a landing page becomes a Rune `web_document`.
- A JS-rendering crawler without WebGPU follows the fallback: it gets the Svelte content, or
  stops at the loader with its meta if it neither keeps cookies nor runs `fetch`.

### 3.9 Accessibility

- Rune keeps a DOM mirror above the canvas, also for full-viewport apps (`bridge.js` lines
  66–90, `[data-rune-a11y]`, `role=document`). It has landmarks, headings, links (`El::href`
  gives a real `<a href>`), and buttons reached with Tab.
- The game screens should key their elements and use `role(...)` and `El::href`, so that the
  bid and play controls are real buttons in the mirror.
- Set `--rune-sans` and `--rune-mono` to the fonts the game draws with.
- The Svelte site remains the full fallback for browsers that cannot draw WebGPU.

## 4. Build and deploy

### 4.1 Dockerfile stages (to add)

```dockerfile
# Rivet bridge for the Rune web build: rivetkit 3.1.0 browser client, bundled by bun.
FROM oven/bun:${BUN_VERSION}-slim@${BUN_DIGEST} AS rune-bridge
WORKDIR /src
COPY . .
RUN bun install --frozen-lockfile \
 && bun build apps/desktop/web/rivet-bridge.ts --target browser --format esm --minify \
      --outfile /out/rivet-bridge.js

# Rune web build: apps/desktop for wasm32 (WebGPU). Runs in parallel with `server`.
FROM chef AS rune-web
ARG RUST_VERSION
# Must equal the wasm-bindgen version in Cargo.lock (0.2.130 today).
ARG WASM_BINDGEN_VERSION=0.2.130
RUN rustup target add wasm32-unknown-unknown --toolchain ${RUST_VERSION} \
 && cargo install wasm-bindgen-cli --version ${WASM_BINDGEN_VERSION} --locked --root /usr/local
# scripts/web-build.sh's release profile; identical for cook and build so the cook layer is reused.
ENV CARGO_PROFILE_RELEASE_OPT_LEVEL=s CARGO_PROFILE_RELEASE_LTO=thin \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_DEBUG=0 \
    CARGO_PROFILE_RELEASE_STRIP=debuginfo
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --target wasm32-unknown-unknown -p euchre-desktop \
      --recipe-path recipe.json
COPY . .
COPY --from=rune-bridge /out/rivet-bridge.js apps/desktop/js/rivet-bridge.js
RUN set -eu; \
    cargo build --release --locked --target wasm32-unknown-unknown -p euchre-desktop --lib; \
    art=target/wasm32-unknown-unknown/release/euchre_desktop.wasm; \
    wasm-bindgen "$art" --out-dir /out/rune --target web --out-name app --remove-name-section; \
    cut=$(tr ' ' '\n' < "${art%.wasm}.d" | grep -m1 '/assets/web/first/Inter-Regular\.ttf$'); \
    cp "${cut%/first/*}"/Inter-*.ttf /out/rune/; \
    hash=$(cat /out/rune/app_bg.wasm /out/rune/app.js | sha256sum | cut -c1-16); \
    mkdir -p /out/client/_rune/immutable; mv /out/rune "/out/client/_rune/immutable/$hash"; \
    sed "s/__RUNE_HASH__/$hash/g" apps/desktop/web/index.html > /out/client/index.html

# Site: the Svelte build as classic.html beside the Rune loader, all precompressed.
FROM oven/bun:${BUN_VERSION}-slim@${BUN_DIGEST} AS site
WORKDIR /src
COPY --from=web /out/client /out/client
RUN mv /out/client/index.html /out/client/classic.html \
 && for e in br gz; do mv /out/client/index.html.$e /out/client/classic.html.$e; done
COPY --from=rune-web /out/client /out/client
COPY apps/desktop/scripts/precompress.ts /tmp/precompress.ts
# Writes .br (q11) and .gz (9) siblings for js/wasm/ttf/html under _rune and index.html (node:zlib).
RUN bun /tmp/precompress.ts /out/client
# runtime: COPY --from=site /out/client /app/build/client   (replaces COPY --from=web)
```

- The `web` stage is unchanged.
- The wasm-bindgen CLI layer depends only on the version ARG, so Railway's layer cache keeps
  it across builds. Locally it took 17.8 s to compile, in `net/tools/install.log`. A pinned
  prebuilt release tarball with a sha256 check would be faster on a cold builder.
- `wasm-opt`: optional. On the probe, `-Oz` (binaryen 129, 1 s) cut the raw size 10 %
  (530,279 → 474,976 B) but made the brotli size 5 % larger (120,523 → 126,878 B).
  Leave it out unless the real app measures a brotli win; Rune's showcase is measured in §4.2.
- No Python is needed: the Dockerfile pins the wasm-bindgen version itself, where web-build.sh
  read it from Cargo.lock with Python's `tomllib` (it needs Python 3.11 or newer; this
  machine has 3.9.6).
- `.dockerignore` already admits `apps/desktop/**`. The root `assets` is excluded, but
  `apps/desktop/assets` is not.

### 4.2 Cost

| Item | Measured |
| --- | --- |
| Rune `web-showcase` for wasm32, cold, web-build.sh profile, fresh target dir | COLD_BUILD |
| Same, after touching the crate's `lib.rs` | WARM_BUILD |
| Rune `web-showcase` output | SHOWCASE_SIZES |
| Probe wasm build (incremental) / `wasm-bindgen` | 2.5–3 s / 0.03 s |
| Probe output: `app_bg.wasm` / `app.js` / bridge | 530,279 / 23,953 / 237,243 B raw; brotli 120,523 / 4,391 / 58,745 B |
| Bridge bundle (`bun build --minify`, 163 modules) | 93 ms |
| `euchre-server` native release after the static_files change | 2 min 3 s (rebuilding rivetkit + euchre crates) |
| wasm init in the browser (cached) | 19–57 ms |

Railway build time:

- The Rune stage runs beside `server` under BuildKit. The extra wall time is about how much
  longer `rune-web` takes than `server`, plus CPU contention between the two.
- On a cold cache, expect the cold wasm build (above) plus the wasm-bindgen install, on top
  of today's 4.5 min if the builder has few cores. On a warm cache (only euchre code changed),
  expect the touch rebuild plus link and LTO.
- Image size grows by the Rune output only: about 0.5–3 MB raw plus `.br` and `.gz` siblings.

### 4.3 Turbo task

```jsonc
// apps/desktop/package.json
"build:web": "bun build web/rivet-bridge.ts --target browser --format esm --minify --outfile js/rivet-bridge.js && sh scripts/web-build.sh"
// turbo.json
"@euchre/desktop#build:web": {
  "dependsOn": [],
  "inputs": ["$TURBO_DEFAULT$", "$TURBO_ROOT$/crates/*/src/**", "$TURBO_ROOT$/crates/*/Cargo.toml"],
  "outputs": ["build/web/**"],
  "passThroughEnv": ["CARGO_TARGET_DIR", "CARGO_HOME", "RUSTUP_TOOLCHAIN"]
}
```

- `scripts/web-build.sh` here is the euchre wrapper of the Dockerfile `RUN` above: cargo,
  wasm-bindgen, fonts, hash and the loader template, writing into `apps/desktop/build/web/`.
- Add `apps/desktop/js/rivet-bridge.js` to `.gitignore`.
- Local serving: copy or symlink `build/web/` into `apps/web/build/client/` with
  `index.html` → `classic.html`, and run `euchre-server` from `apps/web`.

## 5. Risks

1. **SwiftShader and fallback adapters.** Headless Chromium returned a software adapter
   (`isFallbackAdapter: true`). A real browser with a blocklisted GPU may do the same, and
   Rune would then draw on the CPU. Proposal: treat `adapter.info.isFallbackAdapter` as
   classic, with a `?rune=1` override for tests. Not measured on a real low-end device.
2. **Two clients, two copies of the rules.** The bridge copies the `rivet.ts` credentials
   cache, and `WebBackend` copies the desktop `status_error` / `action_error` mappings. Put
   the Rust mapping in one shared module that both the desktop and the web backends use.
3. **rivetkit JS behavior.**
   - Reconnect, backoff and token refresh are rivetkit's, not ours; the 120 s give-up is our
     watchdog.
   - A rivetkit major bump can change `onStatusChange` / `onError` shapes. Pin `rivetkit` to
     the same exact version as the engine, and keep the bridge tiny.
4. **Unsafe `Send`/`Sync`.** This is sound only on single-threaded wasm. The `cfg` makes a
   threaded build fail to compile. Keep it that way, and never enable `atomics`.
5. **`document.write` swap.** It is only used when cookies are blocked. A Chrome intervention
   blocks `document.write` of cross-origin parser-blocking scripts on slow connections. The
   Svelte bootstrap is same-origin and module-based, so it is not affected. Tested once.
6. **Cookie-varied HTML.** The fallback HTML now varies by cookie. `Vary: Cookie` is sent and
   the HTML is `no-cache`. If a CDN is ever put in front, it must respect `Vary` or bypass
   HTML.
7. **Cold builds.** The wasm stage roughly doubles the Rust compile work on a cold cache.
   Railway's layer cache keeps the cook layer while Cargo.lock does not change.
8. **Identity and profile gaps** (§2.7). Without them, Rune web users cannot attach an email
   and may see the guest scrapbook where Svelte showed the email one.

## 6. Record

- **Installed (user locations only):** `wasm-bindgen-cli 0.2.130` with
  `cargo install --locked --root net/tools`, binary at `net/tools/bin/wasm-bindgen`
  (17.8 s). The `wasm32-unknown-unknown` target was already installed for 1.99.1. No Python
  was installed. `~/.cargo/bin/wasm-bindgen` is 0.2.129 (Rune's own lock), and it was left
  alone.
- **Cargo target dir:** `.cargo-targets/webnet`, with the cold-build subdir
  `.cargo-targets/webnet/coldwasm`.
- **Probes:**
  - `net/probe-rivetkit/` — option (a);
  - `net/web-probe/` — the `WebBackend`;
  - `net/bridge-js/` — the bridge;
  - `net/serve/` — the server, loader and `build/client`;
  - `net/driver/` — the Playwright drivers and JSON results `run1.json` / `run2.json`;
  - `net/repo-serve/` — the patched server source;
  - `net/static_files.patch`;
  - `net/shots/` — screenshots.
- **Processes:** the local `euchre-server` and its engine (ports 6930/6931/6935/6938) were
  started and stopped by this lane. Nothing is left running.
