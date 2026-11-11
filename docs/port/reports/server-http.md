# server-http lane report

Lane: **SERVER-HTTP**, crate `crates/euchre-http`.
Contract: `docs/port/ARCHITECTURE.md` (D1, D2, D3, D5, D11, D27, D29), then 05 §11–§12 and §5.4.
TS source of truth (read-only):
- `~/goods/euchre/server/{index,rivet-gateway-proxy,bootstrap-rivet-namespace,public-origin}.ts`
- the SvelteKit `/api/*` routes and `/play`

## Gates

All gates were run with `CARGO_TARGET_DIR=.cargo-targets/http`.

| Gate | Result |
| --- | --- |
| `cargo fmt -p euchre-http -- --check` | pass |
| `cargo clippy -p euchre-http --all-targets -- -D warnings` | pass |
| `cargo test -p euchre-http` | pass: 111 unit tests; the 4 e2e tests skip with an explicit `SKIP` line; 1 doc example is ignored |
| e2e (below) | pass, 4/4, about 25 s |

Code checks:
- Library code has no `unsafe`, `unwrap` or `todo!()`.
- The only `expect` calls are on the five constant regexes in `LazyLock` statics, each under `#[allow(clippy::expect_used)]`.
- Every public item has a doc comment.
- No other crate and not the root `Cargo.toml` was edited. All dependencies are `workspace = true`.

### E2E command and result

```
EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6731 \
EUCHRE_TEST_STORAGE=/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/rivet-store-http \
perl -e 'alarm 400; exec @ARGV' cargo test -p euchre-http --test http_e2e -- --test-threads=1 --nocapture
```

Result: `4 passed; 0 failed` in 25 s. Afterwards `lsof -nP -iTCP:6731-6740 -sTCP:LISTEN` was empty. Each test shuts down its `TestEngine`.

| e2e test | Proves |
| --- | --- |
| `table_session_mints_a_gateway_token_the_gateway_and_engine_accept` | Full path: enroll → `POST /api/tables` → `POST /api/table-session` (see the list below) |
| `profile_endpoints_reach_the_profile_actor` | `GET /api/profile/stats` mints a profile token and reaches `playerProfile ["user", id]` |
| `bootstrap_ensures_the_namespace_and_pool_on_engine_3` | V3 dialect on a real engine: the existing namespace is kept, a new one is created, a second run is idempotent, and a wrong token fails fast |
| `create_table_is_refused_for_a_cross_site_cookie_caller` | A browser create sets `euchre_guest` and `euchre_recent_tables`; a cross-site Origin gets `403` |

The first test checks these steps in order:
1. `POST /api/table-session` returns `{actorId, token, gatewayToken, expiresAt}` in TS key order, and `expiresAt` is the engine's `expires_ts`.
2. Another guest gets `403 "This table belongs to another browser"`, from the table's own `forbidden`.
3. `GET /api/tables/recent` reads the snapshot.
4. A real rivetkit-client 3.0.7 succeeds through `/rivet/**` with the gateway token, over **HTTP and WebSocket**.
5. The same token on actor B is refused with `auth/insufficient_permissions`.
6. A request with no session and no token gets the gateway's `401 Unauthorized`.
7. A browser that has a token but no session also gets `401`.
8. A cookie-session browser gets `200`.
9. A bogus token is refused with `auth/invalid_token`.
10. A `getOrCreate` query target gets `404 Not found`.
11. Directly against the engine, the D3 acceptance table holds: the issued token works, no token gives `invalid_token`, and the admin API answers `403` to the issued token.

## Endpoints

All `/api/*` errors use `{"message", "error": {"code", "message"}}` with `Cache-Control: no-store`. CSRF means `Origin == the public origin`, checked only for ambient (cookie or anonymous) callers.

| Route | Auth | Notes |
| --- | --- | --- |
| `GET /health` | none, any host | `ok`, or `503 draining` before `set_ready` and after `start_draining` |
| any route on an unknown `Host` | – | `421 Unknown host` (text). `X-Forwarded-Host` is never read |
| `GET /api/session` | bearer → cookie; identity header → cookie | `{guestUserId, identity}`; sets no cookies |
| `POST /api/device/enroll` | D1 Origin (absent passes) | rate limited; `{userId, sessionToken, expiresAt}` |
| `POST /api/device/refresh` | bearer only | `401 "Please start a game first"` |
| `POST /api/identity/identify` | CSRF unless bearer; invalid bearer `401` | rate limited; `{identity, identityToken}`; sets `euchre_email` for cookie callers |
| `DELETE /api/identity` | CSRF unless bearer | `204`; clears `euchre_email` |
| `POST /api/tables` | CSRF for ambient; invalid bearer `401` | rate limited; mints a guest and its cookie when needed; `201 {gameId}`; sets the recent cookie for ambient callers |
| `GET /api/tables/recent[?ids=]` | session | `{tables:[{gameId,status,score,handNo}], unavailable}`; 5 s per snapshot |
| `POST /api/table-session/{gameId}` | CSRF for ambient; session | rate limited; `400 Invalid table`, `401`, `403`, `503`; gateway token per D3 Option A |
| `GET/PUT/PATCH /api/profile/settings` | identity (`?profile=`); writes need CSRF unless bearer | see the error mapping below |
| `GET /api/profile/personas`, `/api/profile/stats` | identity | same mapping as settings |
| `GET /api/games[?cursor&limit]`, `GET /api/games/{matchId}` | identity, else browser guest | a `null` replay gives `404`; a bad id gives `400` |
| `GET /api/episodes[?limit]`, `DELETE /api/episodes/{id}` | identity; delete needs CSRF unless bearer | `{"forgotten": bool}` |
| `GET /api/presets` | none | the 7 `PERSONA_PRESETS` |
| `GET /api/rivet/metadata` | none | `{clientEndpoint: <public origin>/rivet, clientNamespace}` |
| other `/api/rivet*` | – | `404 Not found` (text) |
| other `/api/*` | – | JSON `404 not_found` |
| `/rivet`, `/rivet/**` | see "Gateway" below | – |
| everything else | – | static build (below), or `404 Not found` when no dir is configured |

Profile error mapping:
- `invalid_persona` → `400`.
- `unauthorized`, `invalid_token` or `forbidden` → `401`.
- Any other user error → `400`.
- A transport failure → `503` with the family's copy.

Rate limits (D11) are per-IP token buckets, keyed on the trusted last `X-Forwarded-For` entry when `trust_forwarded_for` is set, else on the socket IP:

| Route class | Burst / per minute |
| --- | --- |
| enroll | 10 / 10 |
| identify | 20 / 20 |
| tables | 30 / 30 |
| table-session | 60 / 120 |
| gateway | 600 / 1200 |

CORS applies to `/api/*` only:
- origins from `ALLOWED_ORIGINS` (the `OriginPolicy`), with credentials
- methods GET/POST/PUT/PATCH/DELETE/OPTIONS
- headers content-type and authorization

Cookies follow §11.3:
- `Path=/; HttpOnly; SameSite=Lax; Max-Age=34560000`, plus `Secure` when `secure_cookies` is set, and no `Domain`.
- Recent tables are written URI-encoded (`a%2Cb`), at most 12.

Static build (`ServeDir`):
- Serves br/gzip precompressed files and adds `Vary: Accept-Encoding` when encoded.
- `Cache-Control` is `public,max-age=31536000,immutable` for `/immutable/` paths and `public,max-age=86400` for everything else.
- An extensionless miss falls back to `index.html` with `no-cache`.
- `.map` files, non-GET/HEAD requests and misses get `404 Not found`. A bad percent-decode gets `400 Bad path`.

## Gateway decision

**D3 Option A**, recorded in `requests/gateway-token.md`:
- The server mints actor-scoped engine tokens with `POST /auth/tokens` (`GatewayTokenIssuer`, 900 s).
- The gateway forwards client tokens verbatim and never injects the admin token.

`/rivet/**` admission runs in this order (`gateway::public`):
1. `/rivet/metadata` → the `{}` stub.
2. Tightening: only a concrete actor target passes, as either `/rivet/connect` with the three subprotocols and no query, or `/rivet/gateway/<actorId>[@token]/…` with no `rvt-*` query key. Anything else gets `404 Not found`.
3. The proxy allowlist (`classify_proxy_path`, the TS regexes verbatim): a refused path gets `404 {"error":"not_found"}`.
4. Origin allowlist → `403 {"error":"origin_not_allowed"}`.
5. Caller check, else `401 Unauthorized`:
   - A browser (Origin present) needs a session.
   - A native caller needs a session, or a gateway token (`@token`, `rivet_token.*` or `x-rivet-token`) together with a bearer that is not invalid.
6. The gateway bucket → `429`.

Proxy behaviour (`GatewayProxy`, a port of `rivet-gateway-proxy.ts`):
- Hop-by-hop, credential and client-identity headers are stripped, and `X-Forwarded-For` is overwritten with the trusted derivation.
- reqwest passthrough has no redirects and a 120 s timeout; an upstream failure gives `404 upstream_unreachable`.
- The WebSocket relay echoes `rivet` and has a 16 MiB message/frame limit.
- Backpressure: both queues share a byte-accounted budget, and exceeding it closes both legs with `1013 backpressure limit`.
- Pings every 30 s and a 960 s idle close.
- Close codes are normalized (1005/1006/1015 become 1001 toward upstream and 1011 toward the browser), and reasons are capped at 123 UTF-8 bytes.
- Paths are logged with `@<redacted>` and `rvt-token=<redacted>`.

## Composition API

```rust
pub fn router(state: ApiState) -> axum::Router;
impl ApiState { pub fn new(config: ApiConfig, auth: Authenticator, client: Arc<rivetkit_client::Client>, health: Health) -> anyhow::Result<Self>; }
impl ApiConfig { pub fn from_server_config(cfg: &ServerConfig) -> Self; }   // then set static_dir
impl Health { pub fn new() -> Self; pub fn set_ready(&self); pub fn start_draining(&self); pub fn is_serving(&self) -> bool; }
pub async fn ensure_rivet_namespace(http: &reqwest::Client, opts: &EnsureRivetNamespaceOptions) -> Result<EnsureRivetNamespaceResult, RivetBootstrapError>;
pub fn assert_rivet_env_safety(env: &RivetEnv<'_>) -> Result<(), RivetEnvError>;
impl GatewayProxy { pub fn new(opts: GatewayProxyOptions, http: reqwest::Client) -> Self; pub fn router(&self) -> Router; pub async fn handle(&self, req: Request, socket: Option<IpAddr>) -> Response; }
```

Serve the router with `into_make_service_with_connect_info::<SocketAddr>()`, or every client shares one rate-limit bucket. `ApiConfig` fields:
- `origins`
- `engine_endpoint`, `namespace`, `pool_name`
- `engine_token`, which enables Option A
- `secure_cookies`
- `static_dir`: `None` serves no static routes
- `trust_forwarded_for`
- `rate_limits`, `gateway` tunables and call `timeouts`

The `lib.rs` module doc has the full boot sequence.

## TS tests ported 1:1

| TS file | Rust tests | Notes |
| --- | --- | --- |
| `rivet-gateway-proxy.test.ts` | `src/gateway/tests.rs` | Every pure case and every integration case, against an axum mock engine on loopback. The query-gateway case uses rivetkit-client 3.0.7. The Bun-internal FIFO test became an end-to-end ordering test: 200 frames sent while the upstream dial is in flight. Added: dial failure → 1011 with no address leaked, upstream unreachable, upstream status passthrough |
| `bootstrap-rivet-namespace.test.ts` | `src/bootstrap/tests.rs` | All 8 cases against wiremock in the V2 dialect, plus V3, `encodeURIComponent` and 7 env-safety cases |
| `public-origin.test.ts` | `src/origin.rs` tests | All cases, plus Host normalization and `X-Forwarded-Host` never trusted |

`src/api/tests.rs` drives the router through `tower::ServiceExt` with no network. It covers 33 cases: every endpoint, the bearer/cookie/identity precedence, CSRF, D1, each rate-limit class, the cookie attributes, the error-body bytes and key order, CORS, the gateway admission order, and the static cache and fallback rules.

## Deviations

1. **Gateway-token failure → `503`** with the table-unavailable copy. In TS an `issueToken` throw escaped as `500`.
2. **Table create failure → `503 "Unable to load the table. Please retry."`**, not the TS `500`.
3. **Bootstrap dialect V3 is the default**, because engine 3.0.7 has no `/runner-configs` (see `requests/server-http-engine-admin-api.md`). V2 keeps the TS behaviour.
4. **WebSocket upstream is plain `ws://` only.** tokio-tungstenite is built without TLS; the engine is private on Railway.
5. **The upstream dial happens after the browser upgrade.** Browser frames wait in the socket, and FIFO order is kept (tested).
6. **HTTP gateway bodies are buffered** up to 1 MiB (`413` above that). TS streamed them.
7. **Engine `statusText`** in bootstrap messages comes from `canonical_reason()`.
8. **The gateway checks Origin before the session.** Both are refused either way, but the response code can differ from TS when both are bad.
9. **The SPA fallback is `no-cache`.** TS served `index.html` with the 86400 default.
10. **`/api/rivetX`-style prefixes** route as JSON `404 not_found`, not the text 404. Only `/api/rivet` and `/api/rivet/**` get the text 404.
11. **No catch-panic `500` layer.** Handlers do not panic in library paths.
12. **New copy where TS had none:**
    - "Tell us who you are first."
    - "That id doesn’t look right."
    - "That request is not valid JSON."
    - "That request is too large."
    - "Unable to start a session just now. Please retry."
13. **A rivetkit client with no token** sends the loopback `default` token to a local endpoint (rivetkit 3.0.7 behaviour). The gateway forwards it because it looks like a token, and the engine refuses it. Raw requests with no token get the gateway's own `401`.

## Requests filed

- `requests/gateway-token.md`: the D3 probe and decision (done).
- `requests/server-http-engine-admin-api.md`: engine 3.0.7 admin API differences and the V2/V3 dialects.

## Review fixes

| Finding | Result |
| --- | --- |
| P0 dot segments get past the `/rivet` allowlist | **Fixed.** New `gateway::is_canonical_path` refuses a `.` or `..` segment, also percent-encoded in any case (`%2e`, `.%2E`, ...), a raw `\`, an encoded `/` or `\` (`%2f`, `%5c`) outside the `@<token>` part of the actor segment, and a malformed `%` escape. `classify_proxy_path` checks it first, so the standalone `GatewayProxy` is covered. `admits_public_path` checks it too. Both HTTP and WS then build the upstream URL with `verified_upstream_url`, which parses it with `url::Url` and refuses (`404 not_found`) when parsing would change the classified path. Tests: classifier cases, raw-socket HTTP probes and WS probes against the mock upstream (it sees nothing, with a positive control), the router admission (`404 Not found`), and real-engine probes in `tests/http_e2e.rs` with the issued token: `../../../actors`, `%2e%2e/.../namespaces`, `.%2E/.../health`, `POST ../../../auth/tokens`, `../metadata`, `../inspector/state`. TS normalized and then classified. Refusing is stricter but keeps the classified and forwarded paths the same string. |
| P1 `rvt-*` query check ran on undecoded keys | **Fixed.** Each query key is form-decoded (`+` and `%XX`) before the case-insensitive `rvt-` test. A key with a malformed escape or a non-UTF-8 result refuses the request. Tests: `rvt%2D`, `%72vt-`, `RVT%2d`, fully encoded keys, malformed escapes, plus the review's exact `rvt%2Dnamespace...&rvt%2Dcrash-policy=sleep` query against the real engine (`404 Not found` from the gateway). |
| P2 X-Forwarded-For trust follows engine mode | **Partly fixed; request filed.** `from_server_config` now uses the config's "deployment" rule (`production` or a remote engine). A local server never trusts the header (tested). The topology can only be stated by an explicit `ServerConfig` setting, which this lane does not own: see `requests/server-http-trusted-proxy.md`. `ApiConfig.trust_forwarded_for` stays settable by hand. |
| P2 IPv6 per-address buckets and O(n) prune | **Fixed.** `rate_limit::client_key` keys IPv6 on its `/64` and keeps IPv4 (IPv4-mapped included) per address. A prune runs at most once per second above 50k buckets. Past a hard cap of 100k buckets, a client with no bucket draws from one shared overflow bucket per class instead of growing the map. Tests for all three. |
| P3 identity credential and forwarding headers reach the engine | **Fixed.** `x-euchre-identity`, `x-forwarded-host` and `x-forwarded-proto` are added to `HOP_BY_HOP_REQ`. A mock-upstream test asserts none arrive, and a unit test pins the list against `IDENTITY_HEADER`. |
| P2 bootstrap deadline cannot cut off a hanging request | **Fixed.** Each attempt runs under `tokio::time::timeout(remaining)`. An elapsed attempt is `Transient("timed out")`, so the loop ends with `RivetBootstrapError::Timeout`. Test: a wiremock `set_delay(30 s)` with a 300 ms deadline fails with the timeout in well under 5 s. |
| P3 empty pagination cursor | **Fixed.** `Some("")` counts as the last page, matching TS `if (!cursor) break`. The finding overstated the effect: the page loop is capped at 20, so it was 20 wasted fetches and never an endless loop. Test: an empty cursor with the namespace missing goes `GET`, `POST`, `PUT`. |

Gates after the fixes: `cargo fmt --check` and `cargo clippy --all-targets -D warnings` are clean. `cargo test -p euchre-http` passes 126 lib tests, and the 4 e2e tests return early without `EUCHRE_E2E` (1 doc test is ignored). The 4 engine e2e tests (`EUCHRE_E2E=1`, port 6731) pass, and nothing is left listening on 6731-6740.
