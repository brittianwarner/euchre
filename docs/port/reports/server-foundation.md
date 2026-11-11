# Lane SERVER-FOUNDATION report: config, auth, rivet helpers, test engine

**Status:** done. All deliverables are built and tested, and the engine probe passes against a real rivet-engine 3.0.7.

## Gates

Run with `CARGO_TARGET_DIR=.cargo-targets/foundation`.

| Gate | Result |
|---|---|
| `cargo fmt -p euchre-server -- --check` | **Fails only on `src/server.rs`**, which belongs to another lane (one diff at line 18). Every lane-owned file is formatted: I ran `rustfmt --edition 2024` on my own files, because `cargo fmt` would also rewrite `server.rs`. |
| `cargo clippy -p euchre-server --all-targets -- -D warnings` | clean |
| `cargo test -p euchre-server` | lib 46 passed; `foundation_auth` 7 passed; `foundation_probe` 1 passed (it skips without `EUCHRE_E2E=1`) |
| `EUCHRE_E2E=1 … --test foundation_probe` | passed twice on port 6701 (about 9-11 s each). The engine was stopped afterwards and no listener was left on 6701-6710. |
| `bun apps/server/scripts/verify-rust-tokens.ts` | `ok: TS verified 7 Rust-minted tokens` |

## What is done

| File | Contents |
|---|---|
| `src/config.rs` | `ServerConfig::{from_env, from_map, from_lookup, local, try_local, with_engine_port}`. Also `Secret` (redacting), `AuthSecrets`/`ResolvedSecret`/`SecretSource`, `EngineConfig`/`EngineMode`, `HttpConfig`, `AiConfig`, `OriginPolicy` and `LocalSecrets::load_or_create`, which writes `secrets.json` with mode 0600. Helpers: `write_private_file_atomic`, `write_new_private_file`, `random_secret`, `js_len`, `js_trim`, `is_js_whitespace`, `url_origin`. |
| `src/auth/mod.rs` | `AuthError` (`code()`, `into_user_error()`), `now_secs`/`now_ms`, `AuthKeys`, and the `Authenticator` facade (each method has an `_at` variant for a fixed clock). |
| `src/auth/guest.rs` | Guest JWT (TS `guest.ts`): `sign`, `verify`, `verify_player` and `new_guest_id`, with `GuestAudience::{Session, Actors}`. |
| `src/auth/profile.rs` | Profile actor token (TS `player-profile/auth.ts` and `session.server.ts`): `mint` and `verify`. It keeps the exact TS reasons and order, an `atob`-compatible decoder, lossy UTF-8 with the BOM stripped, and UTF-16 subject length. |
| `src/auth/identity.rs` | Signed email nickname (D2): `normalize_email`, `derive_user_id`, `sign`, `verify` and `resolve`. |
| `src/auth/internal.rs` | `assert_internal_token` (SHA-256 then constant-time compare, TS messages), `match_token_ok` (TS `tokenOk`, a UTF-16 XOR fold) and `new_match_token`. |
| `src/auth/device.rs` | Desktop device enrollment: `enroll`, `refresh`, `DeviceEnrollment`, and `DeviceRecord` (`load`/`save` to `device.json` with mode 0600, written atomically, refreshed at 80 %). |
| `src/rivet.rs` | Helpers for errors, schedule, conn params, SQLite JSON rows, the client, bounded calls, event forwarding, the readiness wait and the actor lane. Also `LocalEngine` (see API). |
| `src/testing.rs` | `TestEngine`, `TestEngineOptions` and `e2e_enabled`. |
| `scripts/auth-fixture-env.ts`, `gen-auth-fixtures.ts`, `verify-rust-tokens.ts` | Cross-language fixtures that run the real TS code. Output is deterministic (verified by md5 across two runs). |
| `tests/foundation_auth.rs` | Replays the cases below. |
| `tests/foundation_probe.rs` | Engine e2e for the probe actor. |

Fixture cases replayed by `foundation_auth.rs`:

- 26 guest cases;
- 41 profile cases;
- 25 email normalizations;
- 6 internal-token cases and 7 `tokenOk` cases;
- byte-identical checks of the TS `signGuest` (session and actors), `mintProfileToken` and a jose `SignJWT` identity token.

The rejected variants include:

- tampered payload and tampered signature;
- expired (at `exp`), and for profile tokens past the skew;
- wrong aud and wrong iss;
- `alg: none` and HS512;
- wrong key and wrong kind;
- `nbf` and `iat` in the future, and a lifetime that is too long;
- a bad subject, including a 65-character astral subject;
- non-object header or payload, bad base64, and the wrong number of parts.

## Public API

### `auth`

```rust
pub fn guest::sign(key: &[u8], user_id: &str, aud: GuestAudience, iat: i64) -> Result<String, AuthError>;
pub fn guest::verify(key: &[u8], token: &str, aud: GuestAudience, now: i64) -> Result<String, AuthError>;
pub fn guest::verify_player(key: &[u8], token: &str, now: i64) -> Result<PlayerClaims, AuthError>;
pub fn profile::mint(key: &[u8], user_id: &str, iat: i64, issuer: Option<&str>) -> Result<String, AuthError>;
pub fn profile::verify(key: &[u8], token: &str, now_ms: i64, issuer: Option<&str>) -> Result<ProfileClaims, AuthError>;
pub fn identity::normalize_email(raw: &str) -> Option<String>;
pub fn identity::derive_user_id(normalized_email: &str) -> String;
pub fn identity::sign(key: &[u8], identity: &Identity, iat: i64) -> Result<String, AuthError>;
pub fn identity::verify(key: &[u8], token: &str, now: i64) -> Result<Identity, AuthError>;
pub fn identity::resolve(key: &[u8], value: &str, allow_unsigned: bool, now: i64) -> Option<Identity>;
pub fn internal::assert_internal_token(expected: Option<&Secret>, token: Option<&str>) -> Result<(), AuthError>;
pub fn internal::match_token_ok(expected: &str, got: Option<&str>) -> bool;
pub fn device::enroll(key: &[u8], iat: i64) -> Result<DeviceEnrollment, AuthError>;
pub fn device::refresh(key: &[u8], bearer: &str, now: i64) -> Result<DeviceEnrollment, AuthError>;
impl Authenticator {
    pub fn from_config(cfg: &ServerConfig) -> Self;
    // sign_guest, verify_guest, verify_player, mint_profile_token, verify_profile_token,
    // assert_internal_token, internal_token_ok, identity_for_email, issue_identity_token,
    // verify_identity_token, resolve_identity, enroll_device, refresh_device, verify_session_bearer
    // (+ `_at` variants taking the clock)
}
```

### `rivet`

```rust
pub fn user_error(code: &str, message: impl Into<String>) -> anyhow::Error;
pub fn user_error_with_metadata<M: Serialize + ?Sized>(code: &str, message: impl Into<String>, metadata: &M) -> anyhow::Error;
pub struct UserError { pub group: String, pub code: String, pub message: String, pub metadata: Option<Value> } // is_user(), is(code)
pub fn parse_rivet_error(error: &anyhow::Error) -> Option<UserError>;   // both client formats + downcast
pub fn parse_rivet_error_str(text: &str) -> Option<UserError>;
pub async fn schedule_after<A: Actor, T: Serialize + ?Sized>(ctx: &Ctx<A>, delay: Duration, action: &str, args: &T) -> Result<String>;
pub async fn schedule_action<A: Actor, M: Action>(ctx: &Ctx<A>, delay: Duration, action: &M) -> Result<String>;
pub fn is_scheduled_fire<A: Actor>(ctx: &Ctx<A>) -> bool;
pub fn require_scheduled_fire<A: Actor>(ctx: &Ctx<A>) -> Result<()>;      // user/internal_only
pub fn decode_params<T: DeserializeOwned>(params: &Value) -> Result<T>;    // user/bad_request
pub fn conn_params<A: Actor<ConnParams = Value>, T: DeserializeOwned>(conn: &ConnCtx<A>) -> Result<T>;
pub fn caller<A: Actor, C>(ctx: &Ctx<A>, verify: impl FnOnce(&A::ConnParams) -> Result<C>) -> Result<C>; // user/unauthorized when no conn
pub struct JsonTable; // new(name) -> Result, name, schema_sql, ensure(db), load::<T>(db, key), save_statement(key, &T), save(db, key, &T), delete(db, key)
pub fn build_client(endpoint: &str, namespace: &str, token: Option<&str>) -> Arc<Client>;
pub fn build_client_with_pool(endpoint: &str, namespace: &str, token: Option<&str>, pool: Option<&str>) -> Arc<Client>;
pub enum CallError { Timeout { what: String, after: Duration } }
pub async fn connect_bounded<A: Actor>(handle: &TypedActorHandle<A>, timeout: Duration) -> Result<TypedActorConnection<A>>;
pub async fn call_with_timeout<A: Actor, M: Action>(conn: &TypedActorConnection<A>, action: M, timeout: Duration) -> Result<M::Output>;
pub async fn stateless_call_with_timeout<A: Actor, M: Action>(handle: &TypedActorHandle<A>, action: M, timeout: Duration) -> Result<M::Output>;
pub async fn forward_events<A: Actor, E: Event>(conn: &TypedActorConnection<A>) -> (SubscriptionHandle, mpsc::UnboundedReceiver<E>);
pub async fn wait_ready<T, F, Fut>(deadline: Duration, probe: F) -> Result<T>;
pub fn is_startup_error(error: &anyhow::Error) -> bool;
pub struct ActorLane; // new(), enter().await -> LaneGuard<'_>, run(fut).await
pub const LOCAL_ENGINE_TOKEN: &str = "default"; pub const LOCAL_ENGINE_NAMESPACE: &str = "default";
pub const PEER_PORT_OFFSET: u16 = 1; pub const METRICS_PORT_OFFSET: u16 = 5; pub const ENGINE_VERSION: &str = "3.0.7";
pub struct LocalEngineOptions { pub port, pub storage_path, pub binary_path, pub auto_download, pub ready_timeout } // new(port, storage_path)
pub struct LocalEngine; // start(opts).await, endpoint, port, pid, storage_root, stderr_log, stdout_log, shutdown().await; Drop stops it
pub fn engine_env(port: u16, storage_root: &Path) -> Vec<(String, String)>;
pub fn engine_config_json(endpoint: &str, port: u16) -> Result<String>;
```

### `testing`

```rust
pub const DEFAULT_TEST_ENGINE_PORT: u16 = 6751;
pub fn e2e_enabled() -> bool;
pub type RegistryBuilder = Box<dyn FnOnce(&mut Registry) + Send>;
pub struct TestEngineOptions { pub port: u16, pub storage_path: Option<PathBuf>, pub actors: RegistryBuilder, pub pool_name: String }
impl TestEngineOptions { pub fn from_env(actors: impl FnOnce(&mut Registry) + Send + 'static) -> Result<Self>; }
impl TestEngine {
    pub async fn start(options: TestEngineOptions) -> Result<Self>;
    pub fn endpoint(&self) -> &str; pub fn namespace(&self) -> &str; pub fn pool_name(&self) -> &str;
    pub fn client(&self) -> Arc<Client>; pub fn engine(&self) -> Option<&LocalEngine>;
    pub async fn ready<T, F, Fut>(&self, probe: F) -> Result<T>;   // 60 s wait_ready
    pub async fn shutdown(self) -> Result<()>;                      // Drop also cleans up
}
```

## Fixture regeneration

Run from the repo root:

```sh
bun apps/server/scripts/gen-auth-fixtures.ts                 # TS -> tests/fixtures/auth/{guest,profile,identity,internal}.json
EUCHRE_WRITE_RUST_TOKENS=1 cargo test -p euchre-server --test foundation_auth rust_minted   # Rust -> rust-minted.json
bun apps/server/scripts/verify-rust-tokens.ts                # TS verifies the Rust tokens
```

The scripts import `apps/web/src` read-only. They shim `$app/env`, `$app/env/private` and `server/rivet` with a bun plugin, pin `Date`, and load jose from `apps/web/node_modules`.

## E2E

```sh
EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6701 \
EUCHRE_TEST_STORAGE=/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/rivet-store-foundation \
perl -e 'alarm 240; exec @ARGV' cargo test -p euchre-server --test foundation_probe -- --nocapture
```

The latest run printed `probe ok: endpoint=http://127.0.0.1:6701 … wake 1 -> 2 laned=[1, 2] unlaned=[1, 1]` and `test result: ok. 1 passed` in 8.81 s.

It covers:

- JSON round-trip, stateless and over WebSocket;
- `user/probe_failed` with metadata parsed from both client formats;
- identity read per call: alice and bob on one actor, and a bad token refused as `user/unauthorized`;
- the `on_connect` `conn.send` sync, and a `ctx.emit` broadcast from a stateless call to a WebSocket;
- the `schedule_after` fire after 500 ms, and direct WebSocket and HTTP `tick` calls refused as `user/internal_only`;
- two concurrent laned bumps returning `[1, 2]`, against an unlaned control that returns `[1, 1]`;
- a SQLite save, `ctx.sleep()`, then wake: `create()` ran again and the state reloaded identically.

## Cargo.toml changes (`apps/server/Cargo.toml` only)

```toml
rivet-error = "=3.0.7"
rivetkit-engine-process = "=3.0.7"
```

Both crates were already in `Cargo.lock`, and the lock file is unchanged. See `requests/foundation-rivet-deps.md`.

## Deviations from TS (deliberate)

1. Guest and identity tokens **require `exp`**; jose treats it as optional. Fixture: `guest_missing_exp`.
2. Guest and identity tokens reject non-canonical base64url signatures. jsonwebtoken does this; jose decodes loosely. Fixture: `guest_sig_noncanonical_trailing_bits`. No signer produces such a signature. Profile tokens keep the loose `atob` behavior (`profile_sig_noncanonical_ok`).
3. For guest and identity tokens, Rust reproduces jose's *claim* and *header* (`crit`, `b64`, `alg`) messages verbatim. Signature and encoding failures are rejected with jsonwebtoken's text instead. Profile messages match TS exactly in every case.
4. The profile secret uses `||` (empty means unset) in both the minter and the verifier. The TS verifier uses `??`. The deployment guard is stricter.
5. `iss` is appended last when `EUCHRE_JWT_ISSUER` is set, which matches the TS mint order.
6. An absent `Origin` header passes `OriginPolicy::check` (D1).
7. The engine metrics port is `P+5`, not rivetkit's `P+10`.
8. `TestEngine` launches its own engine and uses `EngineSpawnMode::Never` (see `requests/foundation-local-engine.md`).
9. The TS Better Auth JWKS path is not ported.
10. Profile JSON edge cases: `serde_json` rejects lone surrogates and out-of-range floats that `JSON.parse` accepts.

## Gotchas found (for the TABLE, PROFILE and HTTP lanes)

- **Single-field action structs.** rivetkit decodes args as a sequence first. If a struct's only field accepts any JSON (`Value`, `Vec<_>`), it swallows the whole argument object (`{value: {value: …}}`). Give such actions a second field, or a typed field. `Tick { label: String }`-style structs are fine.
- **Stateless calls and `on_connect`.** Stateless HTTP calls run `on_connect`. A `conn.send` there logs `ERROR … Connection event sender is not configured` (it is harmless, and `send` still returns `Ok`). rivetkit 3.0.7 has no way to tell the two connection kinds apart.
- **Listeners and the initial sync.** Listeners registered right after `handle.connect()`, before `Connected`, do receive the `on_connect` sync. `ctx.emit` from `on_connect` does not reach the new connection.

## Open items

- `src/server.rs` fails `cargo fmt --check`. That file belongs to its owner's lane.
- Requests filed:
  - `requests/foundation-rivet-deps.md`
  - `requests/foundation-engine-ports.md`
  - `requests/foundation-local-engine.md`

  The last one covers server.rs local mode: use `LocalEngine`, and the default port 6420 collides with other engines.
- `rivet::LocalEngine` start/stop is covered by the e2e test; orphan reclaim has a unit test with a fake engine process (`reclaim_orphan_stops_only_our_orphans`).

## Review fixes

Every finding was checked against the code (and, for parity findings, the TS / jose / cookie sources in `node_modules`) before it was changed.

| # | Sev | Finding | Outcome |
|---|-----|---------|---------|
| 1 | P2 | Embedded engine uses the well-known admin token `default` | **Fixed.** `secrets.json` holds a 4th secret, `RIVET_TOKEN` (43-char random), added in place to older files on load. `local()` puts it in `cfg.engine.token`; `LocalEngineOptions` takes `admin_token` (random by default, `with_admin_token`); the engine env and the readiness probe use it. `LOCAL_ENGINE_TOKEN` removed; `DEFAULT_ENGINE_TOKEN` exists only to assert against. Tests: local-mode config test, e2e asserts the test engine's token is not `default`. |
| 2 | P2 | `ServerConfig::local` lets any present Origin through | **Fixed.** Local mode is strict (D1: present Origin must be allowlisted, absent passes). `OriginPolicy::strict` / `allow` added, `is_local_dev` renamed `allows_unlisted`. Test `origin_policy_strict_and_allow`. |
| 3 | P2 | Legacy percent-encoded `euchre_email` cookie not decoded | **Fixed.** `identity::decode_cookie_value` reproduces cookie@2 `decode` (no `%` → as is; else `decodeURIComponent`, raw on any failure). `resolve` applies it on the legacy branch only. New fixture block `identity.json` `legacy_cookies` (8 cases, generated through SvelteKit's own `cookie` package) replayed by `legacy_cookies_decode_like_sveltekit`. |
| 4 | P2 | Deployment does not require `RIVET_TOKEN` / `RIVET_NAMESPACE` (G37) | **Fixed** with #13 in `config::rivet_env_guard`. |
| 5 | P2 | Local mode hardcodes engine port 6420 | **Fixed.** `config::pick_local_engine_port` binds a free loopback port P and checks P+1 and P+5 are free too (32 tries; warns and falls back to 6420). Test `local_engine_port_block_is_free`. |
| 6 | P3 | Claim presence order is aud then iss; jose checks iss then aud | **Fixed.** Fixture `guest_missing_iss_and_aud` (TS: `missing required "iss" claim`). |
| 7 | P3 | Unknown `crit` header accepted | **Fixed.** `jwt::validate_header` runs jose's `validateCrit` + `validateB64` + `alg` allowlist + unencoded-payload rule, in jose's order and with its messages, before jsonwebtoken. Like jose's verify path, duplicate `crit` entries are not rejected (jose only checks that when signing). 7 new fixtures (`guest_crit_*`), all matching TS verbatim; the `alg none`/`HS512` messages now match too. |
| 8 | P3 | `secrets.json` creation needs hard links; permissions not rechecked | **Fixed.** Hard link first; on any link error except AlreadyExists, `create_new` 0600 fallback. Existing files with group/other bits are warned about and chmodded 0600. Parent dir fsync after create and rename. Tests `legacy_secrets_file_gains_engine_token_and_owner_only_mode`, `new_private_file_never_overwrites`. |
| 9 | P3 | Non-workspace pinned deps in `apps/server/Cargo.toml` | **Rejected.** Promoting them needs the root `Cargo.toml`, which this lane may not edit; `requests/foundation-rivet-deps.md` already asks the integration wave to do it. |
| 10 | P2 | An interrupted test orphans the engine (G30) | **Fixed.** `LocalEngine` writes `euchre-engine.pid` (`<engine pid> <owner pid>`) in its storage root. The next `start` on that root reclaims it: a dead pid or a process that is not `rivet-engine --config <this root's config>` just clears the record; an engine whose owner is still alive is refused, never killed; an orphan gets SIGTERM, then SIGKILL after 15 s. Unit test `reclaim_orphan_stops_only_our_orphans` (fake engine process) and `pid_file_and_command_matching`. |
| 11 | P2 | No shared engine, tests in one binary collide | **Fixed.** `testing` keeps one refcounted engine per process (start serialized by an async mutex; port/storage mismatch refused); each `TestEngine` gets its own pool `test-<uuid>`; the last lease stops the engine and removes the temp dir. The probe now has two tests that run in parallel against it. |
| 12 | P2 | 6420 hardcode (duplicate of #5) | **Fixed** by #5. |
| 13 | P2 | `deployment_guard` lacks TS `assertRivetEnvSafety` checks | **Fixed.** Always refuse a `RIVET_PUBLIC_ENDPOINT` containing the token; in a deployment require `RIVET_TOKEN`, `RIVET_NAMESPACE`, `RIVET_ENDPOINT`, refuse userinfo in `RIVET_ENDPOINT` and in a public endpoint not on `api.rivet.dev` / `*.rivet.dev`. Tests `deployment_requires_rivet_token_namespace_and_endpoint`, `deployment_refuses_userinfo_and_leaked_tokens`. |
| 14 | P2 | `connect_bounded` untested, loses group/code | **Fixed.** It now waits on the status watch for `Connected` or `Disconnected` and fails fast with the new `CallError::Rejected { message }` (rivetkit-client 3.0.7 hands error callbacks only a bare message, so group/code cannot be recovered; documented: make one stateless call to classify). E2e: a bad-token WebSocket connect returns `Rejected` in ~4 ms (was: the full timeout) and the same handle's stateless call returns `user/unauthorized`. |
| 15 | P3 | `wait_ready` retries deterministic errors and overruns its deadline | **Fixed.** Each attempt is clamped to the time left; a `user/*` error returns at once. Other errors still retry (the transient set is wider than `is_startup_error`). Unit tests plus an e2e check (bad token, 60 s deadline, returns in ms). |
| 16 | P3 | `JsonTable` accepts keyword and reserved names | **Fixed.** Names are double-quoted in every statement, and names starting with `sqlite_` or `_rivet_` (any case) are rejected. |
| 17 | P3 | `forward_events` doc is wrong | **Fixed** (doc rewritten to match the code). |
| 18 | P3 | Skipped e2e reports a pass; no masked-bail control; no WS bad-token case | **Fixed.** Skips print `SKIP <test>: engine test; set EUCHRE_E2E=1 to run it` (`testing::e2e_skip`); `#[ignore]` was not used because `EUCHRE_E2E=1 cargo test` would then still skip them (`requests/foundation-e2e-skip.md` asks for the ARCHITECTURE wording). New `probe_rejections_end_to_end`: a plain `bail!` reaches HTTP and WS clients as `core/internal_error` with its text hidden; the bad-token WS and HTTP cases above; `wait_ready` on a user error. |

**Dependencies.** No change to `apps/server/Cargo.toml` in this pass.

**Requests.** `requests/foundation-local-engine.md` updated (admin token, free port, orphan reclaim, and the API changes other lanes will see: `allows_unlisted`, `LOCAL_ENGINE_TOKEN` and `TestEngine::engine()` removed, `CallError::Rejected`, strict local origins, the new deployment requirements). New `requests/foundation-e2e-skip.md`.

**Gates after the fixes** (`CARGO_TARGET_DIR=.cargo-targets/foundation`):

- `cargo clippy -p euchre-server --all-targets -- -D warnings`: pass.
- `cargo test -p euchre-server`: pass (56 lib, 8 `foundation_auth`, 2 `foundation_probe` printing SKIP).
- `cargo fmt -p euchre-server -- --check`: every file of this lane is clean; the only diff is `src/server.rs:18` (another lane's file, unchanged open item above).
- E2e (`EUCHRE_E2E=1`, engine port 6701, scratch storage, 300 s alarm): both probe tests pass in parallel on one shared engine (~9 s); no engine left on 6701-6706.
- Fixtures: `gen-auth-fixtures.ts` regenerated (34 guest, 41 profile, 25 email, 8 legacy cookie, 13 internal cases); `rust_minted` wrote the Rust tokens; `verify-rust-tokens.ts`: TS verified 7 Rust-minted tokens.
