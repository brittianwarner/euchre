# 05 - Profile, Auth, Server, Deploy: porting spec

Lane: PLAYER PROFILE actor, AUTH, SERVER (HTTP routes + gateway), DEPLOY.
Source tree (READ-ONLY): `/Users/brittianwarner/goods/euchre` (TS, rivetkit 2.3.23).
Target: Rust server `apps/server` (axum + rivetkit Rust SDK 3.1.0), desktop client `apps/desktop` (Rune + rivetkit-client 3.1.0), web app retargeted later.

All `file:line` references are to the TS source under the repo root above. "TS" means the existing code, "Rust" means the proposed port. Parts 1-9 are as-built facts. Part 10 onwards is the proposed design. Anything marked **UNVERIFIED** was not proven against the Rust SDK 3.1.0 source or a running engine and must be probed first.

---

## 0. Executive summary of facts that shape the port

1. There are **two independent JWT families** and **two independent shared secrets** (plus one "dev fallback" per family). Do not merge them.
   - Guest family: `jose`, HS256, `iss=euchre-guest`, claim `kind=guest`, `sub=guest-<uuid>`. Two audiences: `euchre-session` (cookie, 400d) and `euchre-actors` (table connection, 2h). Used by `euchreTable` (via `verifyPlayer`) and by the SvelteKit routes. `src/lib/actors/auth/guest.ts`.
   - Profile family: hand-rolled HS256 (Web Crypto), no `iss`, no `kind`, `aud=euchre-actors`, `sub=<sha256 hex of email OR guest-uuid>`, lifetime 30 min, hard cap 1 h. Used only by `playerProfile`. Minted in `src/routes/games/_lib/session.server.ts:90`, verified in `src/lib/actors/player-profile/auth.ts:199`.
   - Internal family: not a JWT. `EUCHRE_INTERNAL_TOKEN` shared secret compared in constant time (SHA-256 digests) on every actor-to-actor call into `playerProfile`.
2. The **email "identity" is an unsigned cookie** (`euchre_email`, plain lowercase email). `userId = sha256hex(email)`. Anyone who sets that cookie becomes that profile. It is explicitly "a nickname, not an account" (session.server.ts:1-27). The Rust port must keep behavioural parity for the web app but should fix this (Part 11.4).
3. `better-auth` + `authStore` actor + `resend` + `drizzle` (`src/lib/server/auth/**`, `src/lib/server/db/**`) is **dead code**: `authStore` is not in `registry.ts`, there is no `hooks.server.ts`, no `/api/auth` route. Do NOT port it. Listed in Part 9 only so nothing is missed.
4. The production process is one Bun process: SvelteKit (`build/server/server.js`) + Bun WebSocket gateway proxy (`/rivet/*`) + rivetkit registry envoy (`server/index.ts`). The Rivet engine is a separate private Railway service. The browser never gets an engine admin credential; it gets an **engine-issued, actor-scoped gateway JWT** (`table.issueToken`) plus a guest actor JWT in connection params.
5. **rivetkit Rust 3.1.0 differs from the TS SDK in ways that change the actor design** (verified by reading `~/.cargo/registry/src/*/rivetkit-3.1.0/src`):
   - No queues for Rust actors: `start.rs:501` "Rust actors have no queue handlers"; any `QueueSend` is rejected. The five `playerProfile` queues must become actions.
   - No `c.state` write-through proxy and no `createConnState`. Actor has `kv()`, `sql()` (SQLite), `ctx.conn()` + `ConnCtx::params()` (CBOR-decoded `ConnParams`), `on_before_connect`, `on_connect`, `on_disconnect`, `on_fetch`, `schedule()`, `cron()`, `client()` (actor-to-actor).
   - No `issueToken` in `rivetkit-client` / `rivetkit` 3.1.0 (grep found none). Gateway tokens must be minted by calling the engine HTTP API `POST /auth/tokens` (as the TS client does internally) or by our own gateway. See Part 12.3.
   - Client sends the gateway/engine token as WS subprotocol `rivet_token.<token>` and in the URL path `/gateway/<actorId>@<token>/connect` (`rivetkit-client-3.1.0/src/remote_manager.rs:640,730-760`), not only on the single `/connect` path the TS entry permits. The Rust gateway must allow both (Part 12.2).
6. `apps/server` must keep the TS actor names, keys, action names, event names and JSON shapes: actor `playerProfile` key `["user", userId]`; actions `getSettings`, `updateSettings`, `getPersonas`, `getActiveGame`, `getStats`, `listGames`, `getReplay`, `listEpisodes`, `forgetEpisode`, `getServerReplay`, `getEpisodesForRole`, `pruneEpisodes`; and (as actions, not queues) `recordGame`, `recordHands`, `recordEpisodes`, `setActiveGame`, `recordTokens`.

---

## 1. Source map

| Area | Files |
|---|---|
| Profile actor | `src/lib/actors/player-profile/{index,auth,guards,repository,schema,settings,types,protocol-constants}.ts` |
| Guest JWT | `src/lib/actors/auth/guest.ts`, `src/lib/actors/auth/guest.test.ts` |
| Table-side token verify | `src/lib/actors/auth/verify.ts` (guest HS256 path; the JWKS/Better Auth path is dead) |
| Server helpers | `src/lib/server/{guest,recent-tables,personas,rivet}.ts` |
| Email nickname identity + profile token mint | `src/routes/games/_lib/session.server.ts` |
| Routes | `src/routes/play/+page.server.ts`, `play/[gameId]/+page.server.ts`, `games/+page.server.ts`, `games/[matchId]/+page.server.ts`, `settings/+page.server.ts`, `api/table-session/[gameId]/+server.ts`, `api/rivet/[...all]/+server.ts`, `+layout.server.ts`, `+layout.ts` |
| Presets | `src/routes/settings/presets.ts` |
| Production entry | `server/index.ts`, `server/rivet-gateway-proxy.ts`, `server/bootstrap-rivet-namespace.ts`, `server/public-origin.ts`, `server/README.md`, tests alongside |
| Dev | `vite-plugin-rivet-dev.ts`, `src/lib/actors/registry.ts` |
| Deploy | `Dockerfile`, `.railway/railway.ts`, `.railway/README.md`, `.env.example`, `scripts/actor-security.mjs`, `scripts/deployment-smoke.ts` |
| Browser client wiring | `src/lib/client/rivet.ts` |
| Protocol constants used here | `src/lib/protocol/index.ts` (lines cited inline) |

---

## 2. Identity and token model (as built)

### 2.1 Secrets (env vars) and their fallbacks

| Purpose | Env var(s), lookup order | Min length | Dev fallback | Where |
|---|---|---|---|---|
| Guest JWT signing key (`signGuest`/`verifyGuest`) | `EUCHRE_ACTOR_JWT_SECRET` \|\| `BETTER_AUTH_SECRET` \|\| `AUTH_SECRET` (JS `||`, so empty string falls through) | **32** | `'euchre-local-only-guest-signing-key'` only when `NODE_ENV !== 'production' && !RIVET_ENDPOINT`; else throw `Set EUCHRE_ACTOR_JWT_SECRET to at least 32 characters` | `guest.ts:5-12` |
| Profile JWT signing/verify key | `EUCHRE_ACTOR_JWT_SECRET` \|\| `BETTER_AUTH_SECRET` \|\| `AUTH_SECRET` (verifier uses `??`; minter uses `\|\|`) | **16** | `'euchre-dev-insecure-secret'` when `RIVET_ENDPOINT` unset; else verifier throws `invalid_token` "no signing secret configured"; minter throws `no signing secret configured for playerProfile tokens (set EUCHRE_ACTOR_JWT_SECRET)` | `player-profile/auth.ts:64-71`, `session.server.ts:56-62` |
| Internal actor-to-actor secret | `EUCHRE_INTERNAL_TOKEN` | **16** in actors (`internalSecret()`, `profileInternalToken()`) | `'euchre-dev-insecure-secret'` when `RIVET_ENDPOINT` unset; else `null` -> caller skips write / callee refuses | `player-profile/auth.ts:84-88`, `euchre-table/index.ts:176-181`, `ai-seat/index.ts` |
| Production boot guard | `EUCHRE_ACTOR_JWT_SECRET` and `EUCHRE_INTERNAL_TOKEN` | **32** each, or process exits | none | `server/index.ts:18-23`: `Deployment requires independent guest and internal signing secrets` |
| Optional issuer check on profile tokens | `EUCHRE_JWT_ISSUER` | n/a | unset = no check | `player-profile/auth.ts:235-238`. NOTE: the minter never sets `iss`, so setting this var rejects every minted token. Keep unset; port as "if configured, require `iss` equal" but have the Rust minter set `iss` when configured. |

"Is local dev" = `RIVET_ENDPOINT` unset (`isLocalDev()`), in three places (`auth.ts:58`, `euchre-table/index.ts:157`, `session.server.ts:52` via `$app/env/private`).

Cross-family fact: the guest dev key and the profile dev key are **different strings**, so in dev a guest token is not a valid profile token and vice versa even before claim checks.

### 2.2 Guest identity (anonymous, per browser)

Cookie: `euchre_guest` (`src/lib/server/guest.ts`).

- Value: a JWT (jose `SignJWT`), header exactly `{"alg":"HS256"}` (jose adds no `typ`), payload `{ "kind":"guest", "sub":"guest-<crypto.randomUUID()>", "iss":"euchre-guest", "aud":"euchre-session", "iat":<now s>, "exp":<now+400d s> }`.
- `signGuest(userId, audience)` (`guest.ts:15-25`): `audience` is `'euchre-session'` (expires `'400d'`) or `'euchre-actors'` (expires `'2h'`). `iat` = now (seconds). Subject = userId.
- `verifyGuest(token, audience)` (`guest.ts:27-39`): `jwtVerify` with `issuer: 'euchre-guest'`, `audience`, `algorithms: ['HS256']`, no clock tolerance override (jose default 0), then requires `payload.kind === 'guest'` and `payload.sub` startsWith `'guest-'`, else throws `Invalid guest identity`. Returns `sub`.
- A session token cannot be replayed as an actor token and vice versa (aud differs). Test: `guest.test.ts`.
- Cookie attributes (`guest.ts:18-24`): `path=/`, `httpOnly`, `sameSite=lax`, `secure = !dev` (SvelteKit `dev` flag), `maxAge = 400*86400` (34,560,000 s).
- `guestIdentity(cookies)`: if cookie present and `verifyGuest(.., 'euchre-session')` succeeds, return `sub`; if present but invalid/expired, silently mint a **new** identity (old games become unreachable); else mint. Side effect: sets cookie on first call. Hashing: none (the identity is the signed `sub`).
- "Hashing" in guest identity = HMAC-SHA256 (JWT HS256) over `base64url(header).base64url(payload)`. No cookie value hashing; no rotation.

### 2.3 Table connection token (guest actor token)

Minted server-side per request, never stored: `signGuest(userId, 'euchre-actors')`, 2 hours (`recent-tables.ts:41`, `api/table-session/[gameId]/+server.ts:19`). Passed as connection params `{ token }`.

Verification inside `euchreTable.createConnState` (`euchre-table/index.ts:1659-1700`) -> `verifyPlayer` (`actors/auth/verify.ts:46-62`):
1. `token` string non-empty and `!== 'dev'` (`DEV_PLAYER_TOKEN`), else `missing_or_invalid_token`.
2. `decodeProtectedHeader(token).alg === 'HS256'` -> `verifyGuest(token,'euchre-actors')` -> `{userId, email:''}`.
3. Any other alg goes to the Better Auth JWKS path (`jwtVerify` against `${APP_URL}/api/auth/jwks`, issuer = APP_URL, audience `euchre-actors`, `clockTolerance: 5`, claims `id|sub` + `email` required). **This path is dead** (no JWKS endpoint exists). Rust: reject any non-HS256 token.
4. Failure -> `UserError('Invalid or expired token', {code:'invalid_token'})`.
5. Ownership: if `state.ownerUserId === ''` the first authenticated user claims the table (persisted immediately); else must equal `userId` or `UserError('Not your game',{code:'forbidden'})`.

Table `onBeforeConnect` origin gate (`index.ts:1628`): connections carrying the per-match `internalToken` bypass; else `Origin` header must be in `ALLOWED_ORIGINS` (comma list, trimmed, empties dropped); empty list = allow only when local dev (`RIVET_ENDPOINT` unset) else `UserError('Origin not allowed',{code:'origin_not_allowed'})`.

### 2.4 Email-nickname identity (`/games`, `/settings`)

`src/routes/games/_lib/session.server.ts`.

- Cookie `euchre_email`, value = normalized email, `httpOnly`, `sameSite=lax`, `secure=!dev`, `path=/`, `maxAge = 60*60*24*400` = 34,560,000 s. **Not signed.**
- `normalizeEmail(raw)`: non-string -> null; `raw.trim().toLowerCase()`; null if length 0 or `> 254`; must match `/^[^\s@]+@[^\s@]+\.[^\s@]+$/`.
- `deriveUserId(normalizedEmail)` = lowercase hex SHA-256 (64 chars) of the UTF-8 bytes of the normalized email. No salt, no pepper.
- `readIdentity(cookies)`: `normalizeEmail(cookie)` then `{ email, userId }`; null if unset/invalid.
- `Identity = { email: string; userId: string }`. For a guest browsing the scrapbook, code builds `{ userId: <guestUserId>, email: '' }`.
- `setIdentityCookie`, `clearIdentityCookie` (`cookies.delete('euchre_email', {path:'/'})`).
- `connectProfile(identity)`: mints a profile token then `client.playerProfile.getOrCreate(['user', identity.userId], { params: { token } })`. Stateless handle: each action call carries the token as conn params; no long-lived socket. Token freshly minted on every `connectProfile` call.

### 2.5 Profile JWT (HS256, hand-rolled)

Mint (`session.server.ts:90-106`): header `{"alg":"HS256","typ":"JWT"}`; payload `{"sub":userId,"aud":"euchre-actors","iat":<now s>,"exp":<iat+1800>}` (key order as written); signature = HMAC-SHA256(secret utf-8, `b64url(header).b64url(payload)`), base64url no padding. TTL `TOKEN_TTL_SECONDS = 30*60`.

Verify `verifyProfileToken(token, nowMs)` (`player-profile/auth.ts:199-285`), in this exact order, every failure = `UserError(message,{code:'invalid_token'})`:
1. secret present (see 2.1), else `no signing secret configured`.
2. exactly 3 `.` segments, else `malformed token`.
3. header and payload segments base64url-decode to JSON objects (non-array), else `token segments are not JSON` / `token header is not an object` / `token payload is not an object`.
4. `header.alg === 'HS256'` exactly, else `unsupported token algorithm` (`alg:none` defence).
5. HMAC-SHA256 verify (WebCrypto `verify`, constant-time) over `headerSeg + '.' + payloadSeg`; malformed signature base64 -> `malformed signature`; mismatch -> `signature rejected`.
6. `aud` equals `'euchre-actors'` (string) or array containing it, else `wrong audience`.
7. If `EUCHRE_JWT_ISSUER` set: `iss` must equal it, else `wrong issuer`.
8. `exp` required finite number; `expiresAt = exp*1000`; reject if `expiresAt + 60_000 < now` (`token expired`). Skew `CLOCK_SKEW_MS = 60_000`.
9. `nbf` optional; reject if `nbf*1000 - 60_000 > now` (`token not yet valid`).
10. `iat` optional; if present: reject if `iat*1000 - 60_000 > now` (`token issued in the future`) and if `expiresAt - issuedAt > 3_600_000` (`token lifetime exceeds the maximum`, `MAX_TOKEN_LIFETIME_MS = 60*60_000`). Note a guest actor token (2 h, has iat) would FAIL this on a profile.
11. `sub` must be string, length 1..=128, else `token subject is missing or unusable`.
Returns `{ userId: sub, issuedAt, expiresAt }`.

`JWT_AUDIENCE = 'euchre-actors'` is shared by guest actor tokens and profile tokens; `iss`/`kind` are what distinguish them.

### 2.6 Internal token

`assertInternalToken(token)` (`auth.ts:129-142`): expected = `internalSecret()`; if null -> `internal_token_mismatch` "EUCHRE_INTERNAL_TOKEN is not configured; internal writes are refused"; token must be non-empty string else "internal token missing"; `constantTimeEqual(token, expected)` compares **SHA-256 digests** (32 bytes each, XOR accumulate) else "internal token rejected". Single code `internal_token_mismatch` by design (no oracle). Queue handlers use `tokenOk()` (boolean) and **silently drop forged messages** with a `warn` log `dropped forged <name>`; action handlers throw.

### 2.7 Per-match internal token (not profile related, for completeness)

`GameState.internalToken = crypto.randomUUID()` minted in `euchreTable.createState`, shared with the three `aiSeat`s; accepted as connection param `{internalToken}` by the table (`onBeforeConnect`/`createConnState` -> `role:'internal'`). Unrelated to `EUCHRE_INTERNAL_TOKEN`. Owned by the table lane.

---

## 3. `playerProfile` actor

### 3.1 Identity of the actor

- Registry name: `playerProfile` (display `options: { name: 'Player Profile', icon: 'id-badge' }`).
- Key: `["user", userId]` where `userId` is `sha256hex(email)` (64 hex) or `guest-<uuid>`. `ownerOf(key)` (`auth.ts:304-316`): `key[0] === 'user'` and `key[1]` non-empty string else `protocolError('internal_error', "playerProfile must be keyed ['user', userId]")`. Called in `createState` (fails creation on bad key), `createConnState`, and every queue handler.
- Callers use `client.playerProfile.getOrCreate(['user', id], {params})` (web) and `tableClient(c).playerProfile.getOrCreate(['user', profileUserId ?? ownerUserId])` (table/seat; `euchre-table/index.ts:1010`, `ai-seat/index.ts:451`). Match history is filed under `profileUserId ?? ownerUserId`.
- Storage tiers: `c.state` (settings, activeGameId, dailyTokens, lifetime cache) + `c.db` SQLite (matches, hands, episodes, ledger). Rationale in `schema.ts` header.

### 3.2 Connection (`createConnState`)

`params: ProfileConnectParams = { token: string; protocolVersion?: number }` (untrusted). Algorithm (`index.ts:227-243`):
1. If `protocolVersion !== undefined && !== PROTOCOL_VERSION (1)` -> `protocol_version_mismatch` "unsupported protocol version".
2. `owner = ownerOf(c.key)`.
3. If `token` not a non-empty string -> `{authenticated:false, userId:null, since: now}` (NOT an error: this is the shape of connectionless internal calls).
4. Else `verifyProfileToken`; if `claims.userId !== owner` -> `forbidden` "token does not belong to this profile". A bad token throws (`invalid_token`) and never connects.
5. Else `{authenticated:true, userId:owner, since:now}`.

`ProfileConnState = {authenticated:true,userId:string,since:number} | {authenticated:false,userId:null,since:number}`.

Every browser-facing action begins `requireAuthenticated(c.conn.state)` -> `forbidden` "this connection is not authenticated". No action takes a `userId` argument: the key is the ACL.

### 3.3 Actor state (`c.state`)

```ts
interface ProfileActorState {
  schema: 1;
  activeGameId: string | null;
  dailyTokens: { tokensIn: number; tokensOut: number; cacheReadTokens: number; calls: number; day: string /* YYYY-MM-DD UTC */ };
  lifetime: { matches: number; won: number; lost: number; abandoned: number; euchresFor: number; euchresAgainst: number; lonersMade: number; handsPlayed: number; updatedAt: number };
  settings: ProfileSettings;
}
```
JSON (camelCase, exactly these keys). `createState` initial: `schema:1, activeGameId:null, dailyTokens = zeros + day=utcDay(now), lifetime = zeros + updatedAt=now, settings = defaultSettings(now)`. `utcDay(now) = new Date(now).toISOString().slice(0,10)`.

`lifetime` is a cache overwritten (never incremented) by `lifetimeFromStats(await computeStats(db, now))` after each `recordGame`; fields copied: matches, won, lost, abandoned, euchresFor, euchresAgainst, lonersMade, handsPlayed, updatedAt.

`run` (start of the run loop, every wake): `c.state.settings = normalizeSettings(c.state.settings, Date.now())` (repair, Part 3.5.4), then iterate the queue.

Observed facts about callers (grep of `src/`): `setActiveGame`, `getActiveGame`, `getPersonas`, `getServerReplay`, `getEpisodesForRole`, `listEpisodes`, `forgetEpisode`, `pruneEpisodes` have **no callers** outside the actor; `dailyTokens` has no reader. `aiSeat` sends `recordEpisodes`/`recordTokens`; the table sends `recordGame`/`recordHands`. Dossier in persona assignments is always `''`. Port all of them anyway (cheap, and the web Settings screen is expected to grow), but do not invent behaviour.

### 3.4 Constants

| Name | Value | Source |
|---|---|---|
| `PROTOCOL_VERSION` | `1` | protocol/index.ts:121 |
| `PERSONA_NAME_MAX_CHARS` | 32 | protocol:586 |
| `PERSONA_BLURB_MAX_CHARS` | 120 | :589 |
| `PERSONA_PROMPT_MAX_CHARS` | 2000 | :597 |
| `HOUSE_PROMPT_MAX_CHARS` | 1000 | :600 |
| `EPISODE_SUMMARY_MAX_CHARS` | 140 | :620 |
| `PERSONA_DOSSIER_MAX_CHARS` | 1600 | :608 |
| `DIAL_MIN` / `DIAL_MAX` | 0 / 1 | :623/:626 |
| `DIFFICULTIES` | `['rookie','casual','expert']` | :583 |
| `MODEL_PLAY` | `'claude-haiku-4-5'` | :555 |
| `MODEL_BID` | `'claude-opus-5'` | :558 |
| `AnthropicModelId` | `'claude-haiku-4-5' \| 'claude-opus-5'` | :552 |
| `ANTHROPIC_MODEL_IDS` (allowed set) | `[MODEL_PLAY, MODEL_BID]` | settings.ts:57 |
| `PACES` | `['brisk','normal','relaxed']` | settings.ts:61 |
| `AI_SEATS` | `[1,2,3]` (persona index i <-> seat i+1) | types.ts:57 |
| `MAX_PERSONA_PATCHES` | 3 | settings.ts:64 |
| `JWT_AUDIENCE` | `'euchre-actors'` | auth.ts:36 |
| `CLOCK_SKEW_MS` | 60_000 | auth.ts:39 |
| `MAX_TOKEN_LIFETIME_MS` | 3_600_000 | auth.ts:42 |
| `MAX_HANDS_PER_CALL` | 64 | repository.ts:94 |
| `MAX_EPISODES_PER_CALL` | 32 | :95 |
| `MAX_DECK_CARDS` | 24 | :96 |
| `MAX_MOVES_PER_HAND` | 128 | :97 |
| `MAX_LIST_LIMIT` | 50 | :98 |
| `DEFAULT_LIST_LIMIT` | 20 | :99 |
| `EPISODE_TTL_MS` | 90*24*3600*1000 = 7_776_000_000 | :792 |
| Episode half-life | 14 days = 1_209_600_000 ms | :901 |
| `ID_PATTERN` | `^[A-Za-z0-9_-]{1,128}$` | guards.ts:131 |
| `LEDGER_TABLE` | `_profile_migrations` | schema.ts |
| Match seed bound | 1..=256 chars | repository.ts:214 |
| Hand seed bound | truncated to 256 chars, `''` if not string | :274 |
| `PROTOCOL_ERROR_CODES` | `origin_not_allowed, invalid_token, forbidden, seat_mismatch, internal_token_mismatch, game_not_found, game_complete, duplicate_move, rate_limited, protocol_version_mismatch, invalid_persona, ai_unavailable, internal_error` | protocol:922-965 |

`DEFAULT_ENGINE_CONFIG` (types.ts:306): `{deckVariant:'24', gameTo:10, stickTheDealer:true, requireNaturalTrump:false, loner:'makerOnly', defendAlone:false, superEuchre:false, farmersHand:'off', railroading:false, dealerMustHoldTrump:false, misdealLimit:3}`. Used as `cfg` fallback when a record has no `cfg` object, and as decode fallback for `cfg_json`. (Owned by the rules-engine lane; reuse `euchre-engine::EngineConfig::default()`.)

### 3.5 Settings model

#### 3.5.1 Types and JSON shape

```ts
interface ProfileSettings { schema: 1; version: number; updatedAt: number; housePrompt: string; personas: PersonaConfig[] /* exactly 3 */; table: TablePrefs }
interface TablePrefs { pace: 'brisk'|'normal'|'relaxed'; banter: boolean; showRationale: boolean; sound: boolean; reduceMotion: boolean }
interface PersonaConfig { id: string; version: number; name: string; blurb: string; prompt: string; housePrompt: string; difficulty: 'rookie'|'casual'|'expert'; aggression: number; risk: number; chattiness: number; temperature: number; modelId: 'claude-haiku-4-5'|'claude-opus-5'; bidModelId: 'claude-haiku-4-5'|'claude-opus-5' }
interface PersonaView { id; version; name; blurb; seat: 1|2|3; difficulty; chattiness: number; degraded: boolean }
```
`cloneSettings`/`clonePersona` copy field by field in the order above (so JSON key order = declaration order; consumers must not rely on it, but golden tests may). `getSettings` and `updateSettings` return `ProfileSettings`; `getPersonas` returns `PersonaView[]` (`degraded` always `false`; `seat = AI_SEATS[index]`).

Rust: `#[serde(rename_all="camelCase")]` structs; `schema` as `u8` constant 1; dials `f64`; enums `#[serde(rename_all="lowercase")]` for `Difficulty` and `Pace`; model id enum with `#[serde(rename="claude-haiku-4-5")]` / `"claude-opus-5"`.

#### 3.5.2 Defaults (verbatim; the Unicode apostrophe in the house prompt is U+2019 and must be preserved)

`defaultSettings(now)` -> `schema:1, version:1, updatedAt:now, housePrompt: DEFAULT_HOUSE_PROMPT, personas:[seed(marge), seed(dutch), seed(verna)], table: DEFAULT_TABLE_PREFS`.

`DEFAULT_HOUSE_PROMPT` =
`This is a friendly Thursday-night euchre game in somebody’s kitchen. Keep the table talk short, dry and good-natured. No coaching, no gloating that lasts.`
(exactly two literals concatenated: `'...somebody’s kitchen. '` + `'Keep the table talk short, dry and good-natured. No coaching, no gloating that lasts.'`)

`DEFAULT_TABLE_PREFS` = `{pace:'normal', banter:true, showRationale:true, sound:true, reduceMotion:false}`.

Persona seeds (index i -> seat i+1). Each persona is built by `seedToPersona(seed, housePrompt)`: `version:1`, `housePrompt` = the house prompt, `modelId: MODEL_PLAY ('claude-haiku-4-5')`, `bidModelId: MODEL_BID ('claude-opus-5')`.

| idx/seat | id | name | blurb | difficulty | aggression | risk | chattiness | temperature |
|---|---|---|---|---|---|---|---|---|
| 0 / seat 1 | `marge` | `Marge` | `Counts every card and lets you know it.` | expert | 0.35 | 0.25 | 0.4 | 0.5 |
| 1 / seat 2 (human's partner; teams are `seat & 1`) | `dutch` | `Dutch` | `Your partner. Steady, and slightly deaf.` | casual | 0.5 | 0.45 | 0.5 | 0.6 |
| 2 / seat 3 | `verna` | `Verna` | `Orders it up on anything red.` | casual | 0.8 | 0.75 | 0.75 | 0.7 |

Prompts (verbatim, `+`-concatenated halves joined into one string):
- marge: `You have played in this kitchen for thirty years. You are precise, patient, and faintly amused by anybody who bids on three small trump. You speak in short sentences.`
- dutch: `You are the player’s partner. You are steady, encouraging and never second-guess a call out loud. When you are pleased you say so in four words or fewer.` (U+2019 in "player’s")
- verna: `You bid too often and enjoy it. You are cheerful about being euchred and insufferable about a march. You never explain your reasoning.`

#### 3.5.3 `server/personas.ts`: the OTHER default set (used by `/play` for guests and as the offline fallback)

`defaultPersonas()` (no profile identity, or settings fetch failed/timed out): three assignments seat 1,2,3 with `dossier: ''`, `persona = {...player, version:1, housePrompt: HOUSE, temperature:0.6, modelId:'claude-haiku-4-5', bidModelId:'claude-haiku-4-5'}`. Note both model ids are Haiku here (differs from settings defaults, which use Opus for bids).

HOUSE (`personas.ts:6`): `A friendly kitchen-table euchre game. Warm, dry humour; one short line when something actually happens. Never coach or signal to a partner.`

| seat | id | name | blurb | difficulty | aggression | risk | chattiness |
|---|---|---|---|---|---|---|---|
| 1 | `west-rita` | Rita | `Patient. Remembers every trump.` | expert | 0.3 | 0.25 | 0.35 |
| 2 | `north-ned` | Ned | `Your steady, encouraging partner.` | expert | 0.5 | 0.45 | 0.45 |
| 3 | `east-grandpa` | Grandpa | `Takes a chance. Enjoys the story.` | expert | 0.8 | 0.8 | 0.65 |

Prompts:
- rita: `You have played euchre for forty years. Count public cards carefully, favour a dependable point over a speculative loner, and preserve partner winners. Your humour is understated: a raised eyebrow in a few words. Never claim to know hidden cards.`
- ned: `You are a dependable experienced partner. Make sound calls, protect vulnerable partner winners, and save strength when their trick is secure. You celebrate good team play in a few warm words, never coach, scold, or give hints about your cards.`
- grandpa: `You are a seasoned kitchen-table player with a mischievous streak. Take a plausible close call, value a well-supported loner and play for the march when it is still available. You take calculated chances, never deliberately misplay or spend a bower when a small trump does the same job. Your short, good-natured quips sound like an old friend; no caricatures, coaching or hints at hidden cards.`
  (source text ends "...no caricatures, coaching or hidden-card hints." - COPY FROM `src/lib/server/personas.ts:44` rather than from this paragraph; the implementer must treat the TS file as the byte-exact source for every long prompt string in this spec.)

`personasFromSettings(settings)` (profile present and fetched within 2 s): for each persona i -> `{seat:i+1, persona:{...clonePersona(persona), housePrompt: settings.housePrompt}, dossier:''}`.

`/play` guards the settings fetch with `Promise.race([connectProfile(identity).then(p=>p.getSettings()), timeout 2000ms])`; on any failure logs `console.warn('Persona settings unavailable; starting with kitchen-table players')` and uses `defaultPersonas()`.

#### 3.5.4 Validation: `applySettingsPatch(current, rawPatch, now)` (pure; commit only on success)

Input `SettingsPatch = { housePrompt?: string; table?: Partial<TablePrefs>; personas?: {seat:number; patch: PersonaPatch}[] }`. Errors are `invalid_persona` unless stated.

1. `rawPatch` must be a non-array object ("settings patch must be an object"). Unknown top-level keys ignored.
2. `next = cloneSettings(current)`.
3. If `'housePrompt' in rawPatch`: `sanitizeText(value, {multiline:true, maxChars:1000, field:'housePrompt'})` (non-string -> "housePrompt must be a string"). Empty string allowed.
4. If `'table' in rawPatch`: must be object ("table must be an object"). `pace` (if present) via `requireOneOf(PACES)` -> "table.pace must be one of: brisk, normal, relaxed". For each of `banter, showRationale, sound, reduceMotion` present: must be boolean ("table.<flag> must be a boolean"). Absent keys keep current values.
5. If `'personas' in rawPatch`: must be array ("personas must be an array"); length <= 3 ("at most 3 persona patches per call"); each entry a non-array object ("each persona patch must be an object"); `seat` must `===` one of 1,2,3 ("seat must be one of: 1, 2, 3"; note strict equality so `"1"` fails); duplicate seat -> "duplicate seat in persona patches". Apply `applyPersonaPatch(next.personas[idx], entry.patch, seat)`.
6. House-prompt mirror: `next.personas = next.personas.map(p => p.housePrompt === next.housePrompt ? p : {...clonePersona(p), housePrompt: next.housePrompt, version: p.version + 1})`.
7. `next.version = current.version + 1; next.updatedAt = now` **always** (even a no-op patch).

`applyPersonaPatch(current, rawPatch, seat)`: patch must be object ("patch for seat N must be an object"). Field label `personas[<seat>].<name>` (uses the SEAT number, not the index). A local `changed` flag flips only when a post-validation value `!==` the current value.
- `name`: `sanitizeText(single-line, max 32)`; empty after sanitising -> "personas[N].name is empty after sanitisation".
- `blurb`: single-line, max 120; empty allowed.
- `prompt`: multiline, max 2000; empty allowed ("no character, just play").
- `difficulty`: one of rookie/casual/expert else "personas[N].difficulty must be one of: rookie, casual, expert".
- `aggression`, `risk`, `chattiness`, `temperature`: `clampNumber(value, 0, 1, field)`: must be a finite JS number (NaN/Infinity/strings -> "<field> must be a finite number"), then clamped to [0,1].
- `modelId`, `bidModelId`: one of `claude-haiku-4-5`, `claude-opus-5`.
- If `changed`: `next.version = current.version + 1`.
- Cannot patch: `schema`, `version`, `updatedAt`, persona `id`, persona `housePrompt` (derived).
- Quirk to preserve (golden-test it): if a call changes both a persona field and `housePrompt`, that persona's version increases by 2 (once in step 5, once in the mirror step).

`sanitizeText(value, {multiline, maxChars, field, code='invalid_persona'})` (`guards.ts:222-253`), in order:
1. non-string -> `<code>` "`<field>` must be a string".
2. `value.normalize('NFC')`.
3. Replace `[  \u0085]` with `'\n'` if multiline else `' '`.
4. `\r\n?` -> `\n`.
5. Delete `[­؜᠎​-‏‪-‮⁠-⁤⁦-⁯﻿]`.
6. Delete `[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]` (keeps `\t \n \r`; `\r` already gone).
7. multiline: `[^\S\n]+` -> `' '` (tabs become spaces), ` *\n *` -> `\n`, `\n{3,}` -> `\n\n`; single-line: `\s+` -> `' '`.
8. `trim()`.
9. If code-point count (`Array.from`) > `maxChars`: take the first `maxChars` code points, join, `trim()` again.
Rust: use `unicode-normalization` (`nfc`), `regex` with the same classes, `chars().take(n)`. JS `\s` = Unicode White_Space plus `﻿`; Rust regex `\s` is Unicode White_Space (no `﻿`, which step 5 already removed) so behaviour matches.

`normalizeSettings(raw, now)` (repair on every wake, never throws): start from `defaultSettings(now)`; if `raw` not an object return defaults. `housePrompt = safeText(raw.housePrompt, 1000, default, multiline=true)`; for each persona index: if `raw.personas[i]` not an object -> `{...defaultPersona, housePrompt}`; else `id = safeId` (regex `^[a-z0-9_-]{1,48}$`, else default id), `version = safeInt(…,1)` (finite number truncated), `name/blurb/prompt = safeText` (type check, sanitise, fall back on failure; max 32/120/2000; name/blurb single-line, prompt multiline), `housePrompt` = the (re)sanitised house prompt, `difficulty = safeEnum`, dials `safeDial` (non-finite -> default, else clamp [0,1]), `modelId/bidModelId = safeEnum`. `table`: each flag boolean else default; `pace` safeEnum. Result `{schema:1, version: safeInt(raw.version,1), updatedAt: safeInt(raw.updatedAt, now), housePrompt, personas, table}`. Note: unlike patch, an empty persona `name` is NOT replaced if sanitising yields `''` (safeText only falls back on type error/throw).

#### 3.5.5 Preset library (`routes/settings/presets.ts`) - UI-only data

Seven presets; applying one fills blurb, prompt, difficulty and dials in the form (never name, never saved until "Save changes"). <!-- critic: added difficulty; settings/+page.svelte:75-85 applyPreset sets difficulty too (06 section 18 already says so) --> Not read by any actor. Port as static JSON served by `GET /api/presets` (or compiled into the web app unchanged). Fields: `id, label, hint, blurb, prompt, difficulty, aggression, risk, chattiness`.

| id | label | hint | blurb | difficulty | aggr | risk | chat |
|---|---|---|---|---|---|---|---|
| grandpa | Grandpa | Experienced, warm, takes a calculated chance. | Takes a chance. Enjoys the story. | expert | 0.8 | 0.8 | 0.65 |
| counter | The Counter | Precise, patient, plays it by the book. | Counts every card and rarely misplays one. | expert | 0.3 | 0.25 | 0.25 |
| steady | Steady Partner | Encouraging, dependable, never second-guesses out loud. | Solid and supportive — never rattled. | casual | 0.45 | 0.4 | 0.45 |
| bold | Bold Bidder | Orders it up often, plays for the march. | Bids first, asks questions never. | casual | 0.85 | 0.8 | 0.6 |
| chatty | Chatterbox | Warm, talkative, keeps the table lively. | Has something to say about every hand. | casual | 0.55 | 0.5 | 0.85 |
| quiet-pro | Quiet Pro | Expert-level play, almost never talks. | Says little. Plays like it matters. | expert | 0.5 | 0.45 | 0.1 |
| wildcard | Wildcard | Unpredictable and a little reckless. | You never quite know what this one will do. | rookie | 0.7 | 0.85 | 0.65 |

Prompts must be copied byte-exact from `presets.ts` (e.g. grandpa: `You are a seasoned kitchen-table player with a mischievous streak. Take plausible close calls and well-supported loners, then play sound partnership euchre. Never deliberately misplay, waste secured winners, coach, or hint at your cards. Your humour is warm and understated, like an old friend.`). Dash in steady blurb is U+2014; apostrophes in "partner’s", "doesn’t", "wouldn’t" are U+2019.

### 3.6 SQLite schema (`c.db`) - verbatim, migration `0001_initial`

Migration ledger: `CREATE TABLE IF NOT EXISTS _profile_migrations (id TEXT PRIMARY KEY, applied_at INTEGER NOT NULL)`. `applyMigrations(db)` (runs on EVERY actor start via `db({ onMigrate })`): create ledger; `SELECT id FROM _profile_migrations`; for each migration not applied run its statements in order, then `INSERT INTO _profile_migrations (id, applied_at) VALUES (?, ?) ON CONFLICT(id) DO NOTHING` with `Date.now()`. All DDL is `IF NOT EXISTS`. Never edit a shipped migration; append only. rivetkit wraps `onMigrate` in a savepoint.

```sql
CREATE TABLE IF NOT EXISTS matches (
  match_id TEXT PRIMARY KEY,
  schema INTEGER NOT NULL,
  user_id TEXT NOT NULL,
  seed TEXT NOT NULL,
  cfg_json TEXT NOT NULL,
  first_dealer INTEGER NOT NULL,
  started_at INTEGER NOT NULL,
  ended_at INTEGER,
  played_at INTEGER NOT NULL,
  status TEXT NOT NULL,
  outcome TEXT NOT NULL,
  score_us INTEGER NOT NULL,
  score_them INTEGER NOT NULL,
  winner_team INTEGER,
  hands_played INTEGER NOT NULL,
  personas_json TEXT NOT NULL,
  opponents_json TEXT NOT NULL,
  euchres_for INTEGER NOT NULL DEFAULT 0,
  euchres_against INTEGER NOT NULL DEFAULT 0,
  loners_attempted INTEGER NOT NULL DEFAULT 0,
  loners_made INTEGER NOT NULL DEFAULT 0,
  marches INTEGER NOT NULL DEFAULT 0,
  throw_ins INTEGER NOT NULL DEFAULT 0,
  tricks_us INTEGER NOT NULL DEFAULT 0,
  tricks_them INTEGER NOT NULL DEFAULT 0,
  tokens_in INTEGER NOT NULL DEFAULT 0,
  tokens_out INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens INTEGER NOT NULL DEFAULT 0,
  llm_calls INTEGER NOT NULL DEFAULT 0,
  recorded_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_matches_history ON matches (played_at DESC, match_id DESC);
CREATE INDEX IF NOT EXISTS idx_matches_outcome ON matches (outcome);
CREATE TABLE IF NOT EXISTS hands (
  match_id TEXT NOT NULL,
  hand_no INTEGER NOT NULL,
  seed TEXT NOT NULL,
  dealer_seat INTEGER NOT NULL,
  deck_order_json TEXT NOT NULL,
  moves_json TEXT NOT NULL,
  trump TEXT,
  maker_seat INTEGER,
  alone_seat INTEGER,
  tricks_us INTEGER NOT NULL,
  tricks_them INTEGER NOT NULL,
  result TEXT,
  delta_us INTEGER,
  delta_them INTEGER,
  ended_at INTEGER NOT NULL,
  PRIMARY KEY (match_id, hand_no)
);
CREATE TABLE IF NOT EXISTS episodes (
  episode_id TEXT PRIMARY KEY,
  role TEXT NOT NULL,
  match_id TEXT NOT NULL,
  hand_no INTEGER NOT NULL,
  ts INTEGER NOT NULL,
  kind TEXT NOT NULL,
  summary TEXT NOT NULL,
  salience REAL NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_episodes_role ON episodes (role, salience DESC, ts DESC);
CREATE INDEX IF NOT EXISTS idx_episodes_expiry ON episodes (expires_at);
```
No FK from `hands` to `matches` on purpose (hands are journalled at each hand boundary; the match row is only written at game over). All timestamps are Unix milliseconds (INTEGER). JSON columns (`cfg_json`, `personas_json`, `opponents_json`, `deck_order_json`, `moves_json`) are `JSON.stringify(value ?? null)` text; decoding falls back on any fault (`decodeJson(value, fallback)` returns fallback for non-string, `''`, parse error, or JSON `null`).

Enumerations: `status` in `active|complete|abandoned`; `outcome` in `won|lost|abandoned`; hand `result` in `point|march|lone_point|march|lone_march|euchre|throw_in` (exact set: `point, march, lone_point, lone_march, euchre, throw_in`); `trump` in `S,H,D,C`; episode `kind` in `bid,play,loner,euchre,read`.

### 3.7 Repository behaviour (every function)

Value rules: all values are bound parameters; the only interpolated strings are column names from literals.

**Parsers** (`repository.ts:105-189`): `parseSeat` int in [0,3]; `parseTeamOrNull` int in [0,1]; `parsePair` array of exactly 2 ints each in [-1000,1000] ("<field> must be a pair"); `parseTimestamp(raw, field, fallback)`: null/undefined -> fallback else int in [0, MAX_SAFE_INTEGER]; `parseCount` = `toInt(raw,0)` clamped at >= 0; `parseStats(raw)` -> `{euchresFor, euchresAgainst, lonersAttempted, lonersMade, marches, throwIns, tricks:[t0,t1]}` via `parseCount`; `parseTokens` -> `{tokensIn, tokensOut, cacheReadTokens, calls}`. `requireInt` rejects non-integers (`internal_error`, "<field> must be an integer in [min, max]"). `toInt` accepts number, bigint, numeric string; null/undefined -> fallback; anything else -> `internal_error` "unexpected non-integer column value".

**`opponentNames(personas)`**: map seat->sanitised name (single-line, max 32, code `internal_error`) for assignments with numeric `seat` and string `persona.name`; returns array for seats `[1,2,3]`, missing -> `''`.

**`parseMatchRecord(raw, owner, now)`**: raw must be object ("record must be an object", `internal_error`); `raw.userId` string and `=== owner` else `forbidden` "record does not belong to this profile"; `matchId = requireId` (`internal_error`); `seed` string length 1..=256; `startedAt` default `now`; `endedAt` null/undefined -> null; `personas = asArray(raw.personas).slice(0,3)`; `cfg = isRecord(raw.cfg) ? raw.cfg : DEFAULT_ENGINE_CONFIG`; `firstDealer = parseSeat(raw.firstDealer ?? 0)`; `status`, `outcome` via `requireOneOf` (`internal_error`); `score = parsePair`; `winnerTeam = parseTeamOrNull`; `handsPlayed = requireInt(raw.handsPlayed ?? 0, 0, 10_000)`; `stats = parseStats`; `tokens = parseTokens`. Output `schema:1`, `userId: owner`.

**`matchColumns`** mapping (column <- value): `match_id<-matchId, schema<-1, user_id, seed, cfg_json<-json(cfg), first_dealer, started_at, ended_at, played_at<-(endedAt ?? startedAt), status, outcome, score_us<-score[0], score_them<-score[1], winner_team, hands_played, personas_json<-json(personas), opponents_json<-json(opponentNames(personas)), euchres_for, euchres_against, loners_attempted, loners_made, marches, throw_ins, tricks_us<-stats.tricks[0], tricks_them<-stats.tricks[1], tokens_in, tokens_out, cache_read_tokens, llm_calls<-tokens.calls, recorded_at<-now, updated_at<-now`.

**`upsertMatch(db, record, now)`** in ONE transaction:
- `SELECT ended_at FROM matches WHERE match_id = ? LIMIT 1`.
- row exists and `ended_at` non-null -> **no-op** `{inserted:false, updated:false}` (finished history is immutable; late retries cannot rewrite).
- row exists and `ended_at` null -> `UPDATE matches SET <all columns except match_id, recorded_at> = ? ... WHERE match_id = ? AND ended_at IS NULL` -> `{updated:true}`.
- else `INSERT ... ON CONFLICT(match_id) DO NOTHING` -> `{inserted:true}`.

**`parseHands(raw, matchId)`**: `asArray`, <= 64 entries; each entry object; `entry.matchId ?? matchId` must equal `matchId`; `deckOrder` array <= 24 (string entries kept, others dropped); `moves` array <= 128 (stored as-is); fields: `handNo` int [0,10000], `seed` (string -> first 256 chars else `''`), `dealerSeat` int[0,3] default 0, `trump` null or in suits, `makerSeat`/`aloneSeat` null or seat, `tricksWon` pair default `[0,0]`, `result` null or in results, `delta` null or pair, `endedAt` timestamp default 0.

**`insertHands`**: empty -> 0; one transaction; per hand `INSERT INTO hands (...) VALUES (...) ON CONFLICT(match_id, hand_no) DO NOTHING` (columns: match_id, hand_no, seed, dealer_seat, deck_order_json<-json, moves_json<-json, trump, maker_seat, alone_seat, tricks_us<-tricksWon[0], tricks_them<-tricksWon[1], result, delta_us/delta_them (null when delta null), ended_at). Returns attempted count (not actually-new count).

**Cursor**: `encodeCursor = "<playedAt>:<matchId>"`; `decodeCursor(raw)`: null/undefined/`''` -> none; non-string or length > 160 -> `internal_error` "cursor is not a string"; no `:` at index > 0 -> "cursor is malformed"; prefix must match `^\d{1,15}$` -> "cursor timestamp is malformed"; suffix `requireId`. **`clampLimit(raw)`**: null/undefined/non-number/non-finite -> 20; else `min(50, max(1, trunc(raw)))`.

**`listMatches(db, limit, rawCursor)`**: columns `match_id, played_at, status, outcome, score_us, score_them, hands_played, opponents_json`. With cursor: `WHERE played_at < ? OR (played_at = ? AND match_id < ?) ORDER BY played_at DESC, match_id DESC LIMIT ?(limit+1)`; without: same without WHERE. `page = rows.slice(0,limit)`; `cursor = rows.length > limit && page.length>0 ? encodeCursor(last.playedAt, last.matchId) : null`. Returns `MatchListPage = {rows: MatchSummary[], cursor: string|null}`; `MatchSummary = {matchId, playedAt, status, outcome, score:[us,them], handsPlayed, opponents:string[]}`. `seed` is never selected.

**`readClientReplay(db, matchId)`** -> `ClientMatchReplay | null`:
```ts
{ matchId, startedAt, endedAt: number|null, playedAt, status, outcome, score:[us,them], winnerTeam: 0|1|null, handsPlayed, opponents: string[],
  stats: {euchresFor, euchresAgainst, lonersAttempted, lonersMade, marches, throwIns, tricks:[us,them]},
  hands: ClientHandReplay[] }
ClientHandReplay = { handNo, dealerSeat, trump: Suit|null, makerSeat, aloneSeat, tricksWon:[n,n], result: HandResult|null, delta:[n,n]|null, endedAt, moves: PlayerAction[] }
```
Match query selects only `match_id, started_at, ended_at, played_at, status, outcome, score_us, score_them, winner_team, hands_played, opponents_json, euchres_for, euchres_against, loners_attempted, loners_made, marches, throw_ins, tricks_us, tricks_them`; hands query `SELECT hand_no, dealer_seat, moves_json, trump, maker_seat, alone_seat, tricks_us, tricks_them, result, delta_us, delta_them, ended_at FROM hands WHERE match_id = ? ORDER BY hand_no ASC`. **Redaction rule**: `seed` and `deck_order_json` are never selected; `moves` filtered by `isClientVisibleMove`: drop an action whose `move.t === 'discard'` unless `action.seat === 0` (dealer's buried card stays secret even after the hand). A hand with `delta` columns where either is null -> `delta:null`.

**`readServerReplay(db, matchId)`** -> `{match: MatchRecord, hands: HandJournalEntry[]}` or null: `SELECT *` from both tables; full unredacted (`seed`, `deckOrder`, `userId`, `cfg`, `personas`, `tokens`). Reachable only with a valid internal token.

**`computeStats(db, now)`**: single aggregate over `matches`:
```sql
SELECT COUNT(*) AS matches,
 COALESCE(SUM(CASE WHEN outcome='won' THEN 1 ELSE 0 END),0) AS won,
 COALESCE(SUM(CASE WHEN outcome='lost' THEN 1 ELSE 0 END),0) AS lost,
 COALESCE(SUM(CASE WHEN outcome='abandoned' THEN 1 ELSE 0 END),0) AS abandoned,
 COALESCE(SUM(euchres_for),0) AS euchres_for, COALESCE(SUM(euchres_against),0) AS euchres_against,
 COALESCE(SUM(loners_attempted),0) AS loners_attempted, COALESCE(SUM(loners_made),0) AS loners_made,
 COALESCE(SUM(marches),0) AS marches, COALESCE(SUM(throw_ins),0) AS throw_ins,
 COALESCE(SUM(hands_played),0) AS hands_played,
 COALESCE(SUM(tricks_us),0) AS tricks_us, COALESCE(SUM(tricks_them),0) AS tricks_them,
 COALESCE(SUM(tokens_in),0) AS tokens_in, COALESCE(SUM(tokens_out),0) AS tokens_out,
 COALESCE(SUM(cache_read_tokens),0) AS cache_read_tokens, COALESCE(SUM(llm_calls),0) AS llm_calls
FROM matches
```
Result `ProfileStats = {matches, won, lost, abandoned, winRate, euchresFor, euchresAgainst, lonersAttempted, lonersMade, marches, throwIns, handsPlayed, tricks:[us,them], tokens:{tokensIn,tokensOut,cacheReadTokens,calls}, updatedAt:now}` with `winRate = decided==0 ? 0 : won/(won+lost)` (abandons excluded; f64). In practice `tokens.*` are 0 (the table writes zeros; seats flush to `dailyTokens` instead).

**Episodes**: `episodeId(role, matchId, handNo, kind) = "<role>.<matchId>.<handNo>.<kind>"`. `parseEpisodes(raw, now)`: <= 32; each object; `role`, `matchId` via `requireId`; `handNo` int [0,10000]; `kind` in set; `ts` default now; `summary = sanitizeText(single-line, 140, code internal_error)`; `salience = clamp(toFloat(raw,0),0,1)`; `expiresAt` = given or `ts + 90d`. `insertEpisodes`: one transaction, `INSERT ... ON CONFLICT(episode_id) DO NOTHING`, returns attempted count. `selectEpisodesForRole(db, role, limit, now)`: `SELECT * FROM episodes WHERE role = ? AND expires_at > ? ORDER BY salience * POWER(0.5, (? - ts) / ?) DESC, ts DESC LIMIT ?` binding `role, now, now, 1_209_600_000, limit`. **Portability warning**: stock SQLite has no `POWER()` unless built with `SQLITE_ENABLE_MATH_FUNCTIONS`; **UNVERIFIED** whether the rivetkit Rust SQLite exposes it. Safe port: `SELECT ... WHERE role=? AND expires_at>? ORDER BY ts DESC LIMIT 500`, then rank in Rust by `salience * 0.5^((now-ts)/1.2096e9)` desc, `ts` desc, take `limit`. `listEpisodes(db, limit, now)`: `SELECT * FROM episodes WHERE expires_at > ? ORDER BY ts DESC LIMIT ?`. `deleteEpisode(id)`: select-then-delete, returns bool. `pruneExpiredEpisodes(now)`: `DELETE FROM episodes WHERE expires_at <= ?`.
`StoredEpisode = {episodeId, role, matchId, handNo, ts, kind, summary, salience, expiresAt}`.

### 3.8 Actions (client-facing; require authenticated conn)

| Action | Args | Returns | Notes |
|---|---|---|---|
| `getSettings` | none | `ProfileSettings` | `cloneSettings(state.settings)` |
| `updateSettings` | `patch: unknown` | `ProfileSettings` | `applySettingsPatch`; assigns to state; errors `invalid_persona` |
| `getPersonas` | none | `PersonaView[]` | seats 1..3 |
| `getActiveGame` | none | `string \| null` | `state.activeGameId` |
| `getStats` | none | `ProfileStats` | `computeStats(db, now)` |
| `listGames` | `req?: {limit?: number; cursor?: string\|null}` | `MatchListPage` | `clampLimit(req?.limit)`, cursor `req?.cursor ?? null` |
| `getReplay` | `matchId: unknown` | `ClientMatchReplay \| null` | `requireId(matchId,'matchId')` (bad id -> `internal_error`) |
| `listEpisodes` | `limit?: unknown` | `StoredEpisode[]` | `clampLimit` |
| `forgetEpisode` | `episodeId: unknown` | `boolean` | `requireId` |

Internal-only actions (explicit `internalToken` first arg; `assertInternalToken` BEFORE any read; throw `internal_token_mismatch`):

| Action | Args | Returns |
|---|---|---|
| `getServerReplay` | `(internalToken, matchId)` | `ServerMatchReplay \| null` |
| `getEpisodesForRole` | `(internalToken, role, limit?)` | `StoredEpisode[]` (`selectEpisodesForRole`) |
| `pruneEpisodes` | `(internalToken)` | `void` |

### 3.9 Queues (writes) - TS names, bodies, handlers

`canPublish = externalDenied(c) => c.conn === undefined || c.request?.method === 'POST'` (comment says "fails open for the stateless HTTP handle every actor-to-actor call uses"; the token compare is the real guard). Handlers run serially in `run`; each message ends with `message.complete()`; exceptions are caught and logged `playerProfile queue handler failed {name, err: String(err).slice(0,200)}`; forged tokens are logged `dropped forged <name>` and dropped (still completed).

| Queue | Body | Handler |
|---|---|---|
| `recordGame` | `{internalToken, record: MatchRecord, hands?: HandJournalEntry[]}` | token ok? `owner=ownerOf(key)`; `record=parseMatchRecord(record, owner, now)`; `upsertMatch`; if `hands !== undefined` `insertHands(parseHands(hands, record.matchId))`; `state.lifetime = lifetimeFromStats(computeStats)`; `saveState({immediate:true})` |
| `recordHands` | `{internalToken, matchId, hands: HandJournalEntry[]}` | token ok? `matchId=requireId`; `insertHands(parseHands(hands, matchId))` |
| `recordEpisodes` | `{internalToken, episodes: EpisodeRecord[]}` | token ok? `insertEpisodes(parseEpisodes(episodes, now))` (aiSeat flushes with `role = persona.id`) |
| `setActiveGame` | `{internalToken, gameId: string\|null}` | token ok? `state.activeGameId = gameId` (not validated); save immediate |
| `recordTokens` | `{internalToken, tokensIn, tokensOut, cacheReadTokens, calls}` | token ok? if `dailyTokens.day !== utcDay(now)` reset bucket; add `nonNegativeInt(x)` (finite number > 0 truncated else 0) to each field; save immediate |

Senders: table -> `sendBounded(profile,'recordGame',body, abort)` (1 s budget; on failure schedules `flushProfile` retry via `TEMPO.profileRetryMs`); `recordHands` per hand boundary (failure logged, dropped); `aiSeat.flushEpisodes` -> `recordEpisodes` (buffer retained on failure); `aiSeat.flushTokens` -> `recordTokens` best-effort.

### 3.10 `MatchRecord` and `HandJournalEntry` wire shapes (profile-consumed)

```ts
MatchRecord { schema:1; matchId; userId; seed; cfg: EngineConfig; firstDealer: Seat; startedAt; endedAt: number|null; status: GameStatus; outcome: MatchOutcome; score:[number,number]; winnerTeam: Team|null; handsPlayed; personas: PersonaAssignment[]; stats: MatchStats; tokens: TokenUsage }
HandJournalEntry { matchId; handNo; seed; dealerSeat; deckOrder: CardId[]; moves: PlayerAction[]; trump: Suit|null; makerSeat: Seat|null; aloneSeat: Seat|null; tricksWon:[n,n]; result: HandResult|null; delta:[n,n]|null; endedAt }
PersonaAssignment { seat: 1|2|3; persona: PersonaConfig; dossier: string }
```
`outcome` for the table: derived by `outcomeOf(game)` (table lane). `PlayerAction`, `CardId`, `Suit`, `Seat`, `Team`, `GameStatus`, `HandResult`, `EngineConfig` belong to the engine/protocol lanes; `moves` are stored opaque.

---

## 4. HTTP surface (SvelteKit, as built)

Status of SvelteKit error responses: `error(status, msg)` yields JSON `{"message": msg}` for `fetch`/`Accept: application/json`, HTML otherwise. `fail(status, data)` re-renders the page with `form` data (form actions).

### 4.1 Page loads / routes

| Method + path | Auth | Behaviour | Responses |
|---|---|---|---|
| `GET /` | none | landing page (client only, no server code) | 200 |
| `GET /play?stick=true&natural=true` (`routes/play/+page.server.ts`) | guest cookie (created if absent) | **Creates a table.** If `RIVET_ENDPOINT` unset: `fetch('/api/rivet/metadata')` must be ok else `throw new Error('The local table service could not start')` (500). `gameId = crypto.randomUUID()` (lowercase hyphenated v4). `identity = readIdentity`. Personas: see 3.5.3. `client.euchreTable.create(['table', gameId], { input: { ownerUserId: guestIdentity(cookies), personas, profileUserId: identity?.userId, cfg: { gameTo: 10, stickTheDealer: searchParams.stick === 'true', requireNaturalTrump: searchParams.natural === 'true' } } })`. `rememberTable(cookies, gameId)`. | `302 Location: /play/<gameId>` (Set-Cookie euchre_guest, euchre_recent_tables); 500 on actor create failure |
| `GET /play/[gameId]` (`+page.server.ts`, `+page.ts: ssr=false`) | guest cookie (created) | `guestIdentity(cookies)`; `rememberTable(cookies, gameId)` (only if gameId matches the UUID regex); returns `{gameId}`. No validation that the table exists (client retries). | 200 |
| `GET /games` | guest + email cookies | see below | 200 |
| `POST /games?/identify` | none | form field `email`; `normalizeEmail`; invalid -> `fail(400, {error:'That doesn’t look like an email address yet.'})`; else sets `euchre_email` and `redirect(303, url.pathname)` | 400 / 303 |
| `POST /games?/signout` | none | `clearIdentityCookie`; `redirect(303, url.pathname)` | 303 |
| `GET /games/[matchId]?profile=browser` | email cookie, or guest cookie when `profile=browser` | `guestUserId = profile==='browser' ? readGuestUserId : null`; `identity = guestUserId ? {userId:guestUserId, email:''} : readIdentity`; none -> `redirect(303,'/games')`; `connectProfile(identity).getReplay(matchId)`; null -> `error(404,'No game like that in your scrapbook.')`; returns `{identity, replay}`. A bad matchId (not matching `ID_PATTERN`) surfaces as an unhandled actor error (500). | 200 / 303 / 404 / 500 |
| `GET /settings` | email cookie | no identity -> `{identity:null}`; else `connectProfile.getSettings()`; failure -> `{identity, settings:null, loadError:true}`; ok -> `{identity, settings, loadError:false}` | 200 |
| `POST /settings?/identify`, `?/signout` | | same as /games | 400 / 303 |
| `POST /settings?/save` | email cookie | no identity -> `fail(401,{error:'Tell us who you are before saving.'})`. Builds patch from form (below); `profile.updateSettings(patch)`; ok -> `{settings, saved:true}`; any throw -> `fail(400,{error:'Couldn’t save that — one of the fields may be too long or an odd value.'})` | 200 / 400 / 401 |
| `POST /api/table-session/[gameId]` | guest cookie + same Origin | see 4.2 | 200/400/401/403/503 |
| `ALL /api/rivet/*` (`routes/api/rivet/[...all]/+server.ts`) | n/a | local dev only: `registry.handler(request)` (serverless start/metadata). In production `server/index.ts` answers `/api/rivet/metadata` itself and returns 404 for every other `/api/rivet*` path. | |
| layout | | `+layout.server.ts` returns `{rivetPublicEndpoint: env.RIVET_PUBLIC_ENDPOINT || undefined}`; `+layout.ts` returns `{rivetPublicEndpoint, rivetMountPath:'/api/rivet'}` (`_RIVET_MOUNT_PATH`) | |

`/games` load (`games/+page.server.ts`): `recent = loadRecentTables(cookies)`; `identity = readIdentity`; `guestUserId = readGuestUserId`. If `guestUserId && guestUserId !== identity?.userId`: `browserGames = connectProfile({userId: guestUserId, email:''}).listGames({limit: PAGE_SIZE})` (errors swallowed). No identity -> `{identity:null, recent, browserGames}`. Else `cursorParam = searchParams.cursor`, `backStack = searchParams.back` split on `,` (kept entries: all non-empty, plus `''`... effectively `split(',')`), `Promise.all([getStats(), listGames({limit: 10, cursor: cursorParam})])`. `olderHref = page.cursor ? hrefFor(page.cursor, [...backStack, cursorParam ?? '']) : null`; `newerHref = backStack.length>0 ? hrefFor(last, rest) : cursorParam ? '/games' : null`; `hrefFor(cursor, back)` = `'?' + URLSearchParams{cursor (if non-empty), back (joined ',' if non-empty)}` or `'/games'` when empty. `PAGE_SIZE = 10`. Error -> `{recent, browserGames, identity, stats:null, page:null, olderHref:null, newerHref:null, loadError:true}`.

`/settings?/save` form -> patch mapping (`settings/+page.server.ts:51-71`):
```
housePrompt = String(form.housePrompt ?? '')
table = { pace: String(form.pace ?? 'normal'), banter: form.banter==='on', showRationale: form.showRationale==='on', sound: form.sound==='on', reduceMotion: form.reduceMotion==='on' }
personas = [1,2,3].map(seat => ({ seat, patch: {
   name: String(form[`seat${seat}_name`] ?? ''), blurb: ..._blurb, prompt: ..._prompt,
   difficulty: String(form[`seat${seat}_difficulty`] ?? 'casual'),
   aggression: Number(form[`seat${seat}_aggression`] ?? 0.5), risk: ... ?? 0.5, chattiness: ... ?? 0.5 } }))
```
(`temperature`, `modelId`, `bidModelId` are never patched by the UI.) `Number('')` is 0 and `Number('abc')` is NaN -> actor rejects -> 400.

### 4.2 `POST /api/table-session/[gameId]` (the only JSON API)

Request: no body. Headers: `Origin` required; `Cookie: euchre_guest=<jwt>`.
Algorithm (`+server.ts`):
1. `request.headers.get('origin') !== url.origin` -> `error(403,'Origin not allowed')` (missing Origin also fails).
2. `gameId` must match `^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$` else `error(400,'Invalid table')`.
3. `userId = verifyGuest(cookie, 'euchre-session')`; failure -> `error(401,'Please start a game first')`.
4. `token = signGuest(userId, 'euchre-actors')` (2 h).
5. `table = client.euchreTable.get(['table', gameId], { params: {token}, signal: AbortSignal.timeout(12_000) })`; `await table.snapshot()` (runs origin+owner checks in the actor). On throw: `code = cause.code`; `code === 'forbidden'` -> `error(403,'This table belongs to another browser')`; else `console.warn('[table-session] snapshot unavailable', {gameId, code})` and `error(503,'Your table is temporarily unavailable. Please reconnect.')`.
6. `issued = process.env.RIVET_TOKEN ? await table.issueToken({subject: userId, expiresIn: 900}) : undefined`. (Engine `POST /auth/tokens`, grants `[{resource:'actor_gateway', target:{id: actorId}, operations:['read']}]`, namespace, subject, duration seconds.)
7. Response 200, header `Cache-Control: no-store`, body:
```json
{ "actorId": "<await table.resolve()>", "token": "<guest actor jwt, 2h>", "gatewayToken": "<engine jwt or absent>", "expiresAt": <issued.expiresAt ?? Date.now()+900000> }
```
Client behaviour (`src/lib/client/rivet.ts`): caches the session, refreshes when `now >= expiresAt - 10_000` or on `forceRefresh`; `createClient({endpoint: origin + '/api/rivet', devtools:false, gateway:{skipReadyWait:true}, getToken: forceRefresh => credentials(forceRefresh).gatewayToken})` (only when not `dev`; throws `Missing table gateway token` if absent); `getParams = () => ({token})`; `actionDefaults: {timeout:12000, throwOnError:false, guardConnection:true}`. User-facing errors: 403 -> `This table belongs to another browser. Open Your games to find your table.`, 401 -> `Please start a game in this browser first.`, else `Your table is temporarily unavailable. Please reconnect.`.

### 4.3 Recent-tables cookie (`src/lib/server/recent-tables.ts`)

- Cookie `euchre_recent_tables`: comma-joined list of lowercase UUIDs (`^[0-9a-f]{8}-...-[0-9a-f]{12}$`), newest first, max 12, deduped. Options same as guest cookie (httpOnly, lax, path `/`, `secure=!dev`, maxAge 400 d). Not signed (it is a bookmark; each table re-checks ownership).
- `recentTableIds(cookies)`: split `,`, filter regex, slice 12.
- `rememberTable(cookies, id)`: ignore if invalid; `[id, ...existing.filter(x=>x!==id)].slice(0,12)`.
- `loadRecentTables(cookies)`: needs a valid `euchre_guest` (else `{tables:[], unavailable:false}`); mints one actor token; for each id `Promise.allSettled` -> `client.euchreTable.get(['table', id], {params:{token}, signal: AbortSignal.timeout(5000)}).snapshot()` -> `{gameId, status: view.status, score: view.score, handNo: view.handNo}`; rejected -> `{gameId, status:null, score:null, handNo:null}`; `unavailable = any rejected`.
- `readGuestUserId(cookies)`: `verifyGuest` or `null`.

---

## 5. Gateway and server process (as built)

### 5.1 Process layout (`server/index.ts`)

Boot sequence (any throw = crash):
1. `assertRivetEnvSafety({...env, selfhosted:true})`.
2. `origin = new URL(APP_URL).origin` (APP_URL required: `new URL('')` throws).
3. `EUCHRE_ACTOR_JWT_SECRET.length >= 32 && EUCHRE_INTERNAL_TOKEN.length >= 32` else throw.
4. `RIVET_ENVOY_VERSION` defaults to `String(Date.now())` (new version per deploy so the engine drains old envoys).
5. `allowedOrigins = ALLOWED_ORIGINS.split(',').trim().filter(Boolean)`; must include `origin`. `createPublicOriginResolver(allowedOrigins)` (public-origin.ts): origins must be exact `http(s)` origins (`url.origin === origin`: no path/userinfo); one origin per host; resolver maps `url.host` -> configured origin (so `www.euchre.sh:444` and `euchre.sh.attacker.example` are rejected).
6. `await ensureRivetNamespace({endpoint: RIVET_ENDPOINT, token: RIVET_TOKEN, namespace: RIVET_NAMESPACE, runnerPool: RIVET_POOL ?? 'default'})`.
7. `registry.startAndWait()` (actors register as an **envoy runner** in pool `default`), then SvelteKit `server.init`, static asset map from `build/client/**` (skip `*.map`).
8. `Bun.serve({hostname:'::', port: PORT ?? 3000, maxRequestBodySize: 1 MiB, idleTimeout: 120, websocket: proxy.websocket, fetch, error})`. Error handler returns `500 "Unable to load the table. Please retry."`.
9. SIGTERM/SIGINT: `draining=true`, `registry.shutdown()`, `app.stop(true)`, exit 0. While draining `/health` returns `503 draining`. Registry `shutdown: {disableSignalHandlers:true, gracePeriodMs:90000}` in prod.

### 5.2 Request routing in `fetch` (order matters)

1. `GET /health` (any method): `draining ? 503 'draining' : registry.routes.health()`.
2. `publicOrigin = publicOriginFor(url)`; none -> `421 "Unknown host"`.
3. `/api/rivet/metadata`: `200 {"clientEndpoint": "<publicOrigin>/rivet", "clientNamespace": "<RIVET_NAMESPACE>"}` + `Cache-Control: no-store`.
4. other `/api/rivet*` -> `404 "Not found"`.
5. `/rivet*`:
   - `/rivet/metadata` -> `proxy.handleRequest` (static stub `{}`).
   - else must satisfy ALL of: `pathname === '/rivet/connect'`, `url.search === ''`, `Sec-WebSocket-Protocol` contains `rivet_target.actor`, some entry matches `^rivet_actor\.[a-z0-9_-]{20,64}$`, some matches `^rivet_token\.[A-Za-z0-9_.-]+$`; otherwise `404 "Not found"` (actor create/getOrCreate, arbitrary namespaces and actors are unreachable).
   - Cookie `euchre_guest` extracted by naive split on `;` and `startsWith('euchre_guest=')`, `.slice(13)`; `verifyGuest(cookie,'euchre-session')` else `401 "Unauthorized"`. (Note: this is session-validity only, not ownership; ownership is enforced by the actor's conn params.)
   - `proxy.handleRequest(request, server)`.
6. GET/HEAD static asset lookup (decodeURIComponent; `400 "Bad path"`): `Cache-Control: public,max-age=31536000,immutable` when path contains `/immutable/`, else `public,max-age=86400`; `.br`/`.gz` sibling by `Accept-Encoding` (br preferred) with `Content-Encoding` + `Vary: Accept-Encoding`; HEAD returns no body.
7. SvelteKit: `kit.respond(new Request(publicOrigin + path + search, request), {getClientAddress: () => deriveTrustedClientIp(x-forwarded-for, socket ip) ?? '127.0.0.1'})`.

### 5.3 Gateway proxy (`server/rivet-gateway-proxy.ts`)

**Classification** `classifyProxyPath(pathname, method, isUpgrade, prefix='/rivet')` returns `{kind: 'metadata'|'http'|'ws'|'deny', upstreamPath}`; path outside prefix -> deny; `path = pathname.slice(prefix.length) || '/'`:
- `GET /metadata` -> `metadata`.
- `path === '/connect'` or matches `^/gateway/[^/]+(?:/connect|/websocket(?:/.*)?)$` -> `ws` only if the request is an Upgrade, else `deny`.
- matches `^/gateway/[^/]+(?:/action/[^/]+|/queue/[^/]+|/request(?:/.*)?)$` -> `http` only if NOT an upgrade, else deny. (Any HTTP method including OPTIONS.)
- everything else (including `/actors`, `/namespaces`, `/runner-configs`, `/datacenters`, `/ui`, `/gateway/x/inspector/...`, `/gateway/x/health`, `/gateway/x/dynamic/reload`, `/metadata` POST) -> deny -> `404 {"error":"not_found"}`.
`/metadata` is NEVER forwarded; answered locally `200 {}` with `Content-Type: application/json`, `Cache-Control: no-store` (empty object = "keep configured endpoint").
`GATEWAY_SEG = [^/]+` (actor name or `actorId[@token]`).

**Origin gate** `isOriginAllowed(origin|null, allowed|'*')`: null passes (non-browser; actor HMAC is the boundary); `'*'` passes; else must be in list. Rejection `403 {"error":"origin_not_allowed"}` (warn log). Applies to HTTP passthrough and WS upgrades.

**Client IP for the engine's rate limit** `deriveTrustedClientIp(xff, socketAddr)`: take the LAST entry of `X-Forwarded-For` (appended by the nearest trusted hop, Railway edge; first entry is client-forgeable), strip `ip:port` / `[v6]:port`, must be a valid IP (`net.isIP`); else (warn once per 60 s with printable-ASCII 64-char entry) fall back to the socket IP if valid; else null. Stamped onto the upstream as `X-Forwarded-For` (overwritten; deleted if null). The engine Guard rate-limits per client IP (10,000 req/60 s + 2,000 in flight, hardcoded in 2.3.x).

**HTTP passthrough**: build `headers` = inbound minus hop-by-hop and credential/identity headers: `connection, keep-alive, transfer-encoding, upgrade, host, content-length, cookie, authorization, forwarded, x-real-ip, x-client-ip, true-client-ip, cf-connecting-ip, x-cluster-client-ip`; set `x-forwarded-for`. `fetch(upstreamHttp + upstreamPath + search, {method, headers, body (non GET/HEAD), redirect:'manual', signal: AbortSignal.timeout(120_000)})`. Upstream 429 logs `[guard-503] rivet proxy: upstream 429 (guard rate limit)`. Response headers minus `connection, keep-alive, transfer-encoding`; status + statusText preserved. Failure -> `404 {"error":"upstream_unreachable"}`. **No admin credential is injected**: the browser's engine-issued gateway JWT and actor conn-params HMAC are the auth.

**WebSocket relay**: `Sec-WebSocket-Protocol` offered list (split `,` trim) forwarded verbatim upstream; if it contains `rivet` the 101 response echoes `Sec-WebSocket-Protocol: rivet`. Upstream URL `ws(s)://<engine>` + upstreamPath + search. Upstream dial headers: `Origin` (browser's, if any) and `X-Forwarded-For`. Dial timeout 10 s -> close both `1011 "upstream dial timeout"`. Constants: `HIGH_WATER_BYTES = 4 MiB`, `LOW_WATER_BYTES = 1 MiB`, `DEFAULT_MAX_BUFFERED_BYTES = 16 MiB`, `DEFAULT_MAX_PAYLOAD_BYTES = 16 MiB`, `UPSTREAM_DRAIN_POLL_MS = 25`, Bun ws `idleTimeout: 960`, `sendPings: true`, `perMessageDeflate: false`, `closeOnBackpressureLimit: true`. Frames browser->upstream queue (FIFO) until the upstream opens or while `upstream.bufferedAmount > HIGH_WATER` (flush when below LOW_WATER via a 25 ms poll); upstream->browser frames queue once the browser leg is above HIGH_WATER and flush on `drain`; total queued bytes over 16 MiB -> close both `1013 "backpressure limit"`. Frame order is preserved strictly (the Rivet wire protocol is ordered). Close mirroring: upstream close -> browser close with `normalizedCloseCode(code, 1011)`, reason `event.reason || 'upstream closed'`; browser close -> upstream close with `normalizedCloseCode(code, 1001)`, reason `reason || 'client closed'`; upstream error -> `1011 "upstream error: <msg>"`. `isSendableCloseCode`: 1000-1014 except 1004,1005,1006, or 3000-4999; anything else becomes the fallback. Reason capped at **123 UTF-8 bytes** (`capWsCloseReason`: encode, truncate bytes, decode, strip trailing U+FFFD).

### 5.4 Namespace bootstrap (`server/bootstrap-rivet-namespace.ts`)

`ensureRivetNamespace({endpoint, token, namespace, runnerPool, additionalRunnerPools?, timeoutMs=60_000, retryDelayMs=1_000})`: loop until deadline; each attempt `ensureNamespaceOnce`:
1. `GET {base}/namespaces[?cursor=<c>]` with `Authorization: Bearer <token>`; up to 20 pages following `pagination.cursor`; found if some `namespaces[].name === namespace`. 401/403 -> fatal (`RivetBootstrapFatalError`, no retry, message "engine admin auth rejected the token on GET /namespaces (...) — check RIVET_TOKEN matches the engine's RIVET__AUTH__ADMIN_TOKEN"). Non-ok -> retryable error.
2. If missing: `POST {base}/namespaces` JSON `{"name": ns}`; ok -> created; body `code === 'name_not_unique'` -> treated as success (concurrent boot); 401/403 -> fatal; else retryable.
3. For each unique non-empty trimmed pool in `[runnerPool, ...additional]`: `PUT {base}/pools/<encodeURIComponent(pool)>?namespace=<ns>` with the flat JSON body
`{"drain_on_version_upgrade":true,"actor_eviction_period":60,"actor_eviction_rate":1}` (full replace; an empty body would drain all old envoys immediately on each deploy; do NOT send `actor_eviction_delay`). 401/403 fatal; other non-ok retryable.

Steps 2 and 3 are the rivet-engine 3.1.0 admin API, which the port runs on: `euchre_http::ensure_rivet_namespace` with `AdminDialect::V3` (the default), called by `euchre-server` at boot (`apps/server/src/server.rs`, `docs/port/reports/server-boot.md`). The TS source spoke the engine 2.3 API, which `AdminDialect::V2` keeps for an engine still on 2.3: step 2 sends `{"name": ns, "display_name": ns}`, and step 3 is `PUT {base}/runner-configs/<pool>?namespace=<ns>` with the same three fields nested as `{"datacenters":{"default":{"normal":{...}}}}`. Step 1 is the same on both.

Engine error body shape `{group?, code?, message?}` formatted `"<status> <group ?? 'api'>.<code ?? 'error'>: <message ?? statusText>"`. Timeout error: `rivet bootstrap: engine at <endpoint> did not accept namespace/runner-config provisioning within <ms>ms (<n> attempts): <last>`. Returns `{namespaceCreated, runnerConfigEnsured:true, attempts}`.

`assertRivetEnvSafety({RIVET_ENDPOINT, RIVET_PUBLIC_ENDPOINT, RIVET_TOKEN, RIVET_NAMESPACE, selfhosted})`:
- always: if token and public endpoint both set and the public endpoint string contains the token -> throw "RIVET_PUBLIC_ENDPOINT contains the RIVET_TOKEN value ...".
- selfhosted: missing any of `RIVET_TOKEN, RIVET_NAMESPACE, RIVET_ENDPOINT` -> throw naming all missing; `RIVET_ENDPOINT` must not embed userinfo; `RIVET_PUBLIC_ENDPOINT` userinfo allowed only if host is `api.rivet.dev` or ends `.rivet.dev`.

### 5.5 Registry wiring (`src/lib/actors/registry.ts`)

`setup({ shutdown: isLocalDev ? undefined : {disableSignalHandlers:true, gracePeriodMs:90000}, use: {health, euchreTable, aiSeat, playerProfile}, startEngine: isLocalDev ? true : undefined, startServices: false, configurePool: isLocalDev ? {url: RIVET_DEV_SERVERLESS_URL ?? 'http://127.0.0.1:5173/api/rivet'} : undefined })`. `health` actor: state `{pings:0}`, action `ping() -> {ok:true, ts}` (increments). Dev: Vite plugin `rivetDevMiddleware` serves `/api/rivet/*` with raw bodies (works around empty `request.arrayBuffer()` for Rivet's binary `/start`), sets `RIVETKIT_STORAGE_PATH ??= resolve('.rivetkit')` when `RIVET_ENDPOINT` unset. Dev mode is serverless-callback; production is envoy. The Rust port has no serverless dev loop (Part 13).

`server/rivet.ts` client defaults: `endpoint: RIVET_ENDPOINT || 'http://127.0.0.1:6420'`, `namespace: RIVET_NAMESPACE || 'default'`, `token: RIVET_TOKEN`, `devtools:false`, `headers: {Origin: APP_URL || 'http://localhost:5173'}`; `getRivetPublicEndpoint() = RIVET_PUBLIC_ENDPOINT || undefined`.

---

## 6. Environment variables (complete)

| Var | Meaning | Default | Required where |
|---|---|---|---|
| `NODE_ENV` | `production` in Docker; disables guest dev key | unset | prod (Dockerfile sets) |
| `PORT` | HTTP listen port (Bun), engine uses its own `PORT=6421` | 3000 | optional |
| `APP_URL` | canonical public origin; table `appUrl()`; boot guard; Rivet client Origin header | `http://localhost:5173` (actors) / required in `server/index.ts` | prod (required, else boot throws) |
| `ORIGIN` | SvelteKit adapter-node origin | n/a | prod (set same as APP_URL) |
| `ALLOWED_ORIGINS` | comma list of exact origins; WS/Origin allowlist (table `onBeforeConnect`, gateway) | empty (deny in prod, allow in local dev) | prod; must include `APP_URL`'s origin |
| `PUBLIC_APP_URL` | fallback for `APP_URL` | n/a | optional |
| `RIVET_ENDPOINT` | private engine URL (`http://rivet-engine.railway.internal:6420`); unset = local dev with bundled engine | unset | prod |
| `RIVET_NAMESPACE` | namespace (`euchre`) | `default` (client) | prod (selfhosted requires) |
| `RIVET_POOL` | runner pool name | `default` | prod |
| `RIVET_TOKEN` | engine admin token (`${{rivet-engine.RIVET__AUTH__ADMIN_TOKEN}}`); presence also toggles `issueToken` in the session route | unset | prod |
| `RIVET_PUBLIC_ENDPOINT` | `https://<origin>/rivet`; browsers' client endpoint (only exposed via layout data) | unset | prod (must not contain token) |
| `RIVET_ENGINE_SELFHOSTED` | documented flag; server hard-codes `selfhosted:true` | `true` | prod |
| `RIVETKIT_RUNTIME_MODE` | `envoy` (prod) vs serverless | envoy | prod |
| `RIVET_ENVOY_VERSION` | monotonically changing version per deploy | `Date.now()` | optional |
| `RIVET_INSPECTOR_DISABLE` | `1` hides inspector | unset | prod |
| `RIVET_LOG_LEVEL` | `warn` | | prod |
| `RIVET_EXPOSE_ERRORS` | `0` in prod | | prod |
| `RIVET_DEV_SERVERLESS_URL` | dev-only callback URL | `http://127.0.0.1:5173/api/rivet` | dev |
| `RIVETKIT_STORAGE_PATH` | dev storage dir set by Vite plugin | `.rivetkit` | dev |
| `EUCHRE_ACTOR_JWT_SECRET` | guest + profile JWT HMAC secret | none | prod, >= 32 chars |
| `EUCHRE_INTERNAL_TOKEN` | actor-to-actor shared secret | none | prod, >= 32 chars |
| `EUCHRE_JWT_ISSUER` | optional `iss` requirement on profile tokens | unset | never |
| `BETTER_AUTH_SECRET`, `AUTH_SECRET` | legacy fallbacks for the JWT secret (and Better Auth, dead) | | no |
| `OPENROUTER_API_KEY` | Jev decisions via OpenRouter Decisions API (`typesafe/jev-1.13`) | | prod for AI opponents |
| `OPENROUTER_MODEL_TALK` | conversational banter model (e.g. `google/gemini-3.6-flash`) | | optional |
| `OPENROUTER_BASE_URL` | override | | optional |
| `ANTHROPIC_API_KEY`, `ANTHROPIC_BASE_URL` | banter fallback only, never moves | | optional |
| `EUCHRE_FAST_TEMPO` | `1` compresses table pauses | unset | optional |
| `EUCHRE_TRUST_FORWARDED_FOR` | (Rust server, W3) `1` keys the D11 per-IP buckets and the engine Guard on the last `X-Forwarded-For` entry; `0`/unset keys them on the socket peer. Any other value refuses to boot. Never inferred from `NODE_ENV` or the engine mode; a deployment with it off logs a warning | off (always off in `ServerConfig::local`) | prod on Railway: `1` |
| `EUCHRE_TRUSTED_PROXIES` | (Rust server, W3) comma list of CIDRs or bare IPs (`100.64.0.0/10, fd00::/8`); when set, the header counts only on requests whose socket peer is inside one. Requires `EUCHRE_TRUST_FORWARDED_FOR=1` | unset (any peer) | optional; leave unset on Railway unless the edge's peer range is known |
| `DATABASE_URL` | legacy drizzle scaffold (`local.db`); slated for removal | | no |
| `BETTER_AUTH_URL`, `EMAIL_FROM`, `RESEND_API_KEY`, `VERCEL_URL`, `VERCEL_PROJECT_PRODUCTION_URL` | dead better-auth/Vercel code | | no |
| engine svc: `PORT=6421`, `RIVET__FILE_SYSTEM__PATH=/data`, `RIVET__AUTH__ADMIN_TOKEN`, `RIVET__FEATURES__GUARD_GATEWAY_V3__MODE=on`, `RIVET__FEATURES__GUARD_GATEWAY_V3__PERCENTAGE=100` | engine config | | engine |

The table above records the TS app. The Rust deployment's set follows in 6.1.

### 6.1 Rust deployment (as built, W4)

`ServerConfig::from_env` (`crates/euchre-server-core/src/config.rs`) reads these names. `.railway/railway.ts` sets them, and `docs/DEPLOY.md` is the runbook.

| Var | Rust server | Railway `euchre` |
|---|---|---|
| `NODE_ENV` | `production` turns on the deployment guards: three distinct secrets of at least 32 characters, none a dev key; the engine token, namespace and endpoint; `APP_URL` in `ALLOWED_ORIGINS`; `RIVET_EXPOSE_ERRORS=1` refused | literal `production` |
| `PORT` | listens on `[::]:PORT` (default 3000) | literal `3000` |
| `EUCHRE_TRUST_FORWARDED_FOR` | as in the table above | literal `1` |
| `APP_URL`, `ALLOWED_ORIGINS` | as above. `ALLOWED_ORIGINS` also maps each `Host` to its public origin; an unlisted host gets `421`, and `X-Forwarded-Host`/`-Proto` are never read. It must list every serving host (`euchre.sh`, `www.euchre.sh`). | `preserve()` |
| `RIVET_ENDPOINT`, `RIVET_NAMESPACE`, `RIVET_POOL`, `RIVET_TOKEN` | as above; the server creates the namespace and pool on boot (engine 3: `POST /namespaces`, `PUT /pools/{pool}`) | `preserve()` |
| `EUCHRE_ACTOR_JWT_SECRET`, `EUCHRE_INTERNAL_TOKEN` | as above | `preserve()` |
| `BETTER_AUTH_SECRET`, `AUTH_SECRET` | still read, as in TS: when `EUCHRE_ACTOR_JWT_SECRET` is unset or empty, the first non-empty of these two signs guest and profile tokens instead (`ACTOR_SECRET_VARS` in `config.rs`). Leave both unset | unset; `railway.ts` does not list them, so apply deletes any stale value |
| `EUCHRE_IDENTITY_SECRET` | **new**: the identity-token secret, at least 32 characters, distinct from the other two | `preserve()`, set by hand before the first plan |
| `OPENROUTER_API_KEY` | as above | `preserve()` |
| `RUST_LOG` | log filter (default `info` with rivetkit actor and envoy chatter at `warn`) | unset |
| `BUILD_NUMBER` | envoy version (`u32`); then `RIVET_ENVOY_VERSION`; else unix seconds at boot | unset |
| `NO_PROXY` | only where `HTTP_PROXY` is set: `127.0.0.1,localhost,::1`, plus the engine host when external. The rivetkit-client actor client honours the proxy variables and cannot be configured (W4-notes 6). | unset (Railway has no proxy) |
| `RIVET_PUBLIC_ENDPOINT` | read only by the env guard (it must not contain the token); the browser endpoint comes from the request `Host` (`/api/rivet/metadata`) | removed |
| `EUCHRE_JWT_ISSUER`, `EUCHRE_TRUSTED_PROXIES`, `OPENROUTER_MODEL_TALK`, `OPENROUTER_BASE_URL`, `ANTHROPIC_*`, `EUCHRE_FAST_TEMPO`, `EUCHRE_ALLOW_UNSIGNED_EMAIL` | optional; the last two stay unset in production | unset |
| `RIVETKIT_STORAGE_PATH`, `RIVET_RUN_ENGINE_PORT`, `RIVET_ENGINE_BINARY_PATH` | local mode only (spawned engine) | unset |

Removed with the TS app (the Rust server does not read them): `ORIGIN`, `RIVETKIT_RUNTIME_MODE`, `RIVET_ENGINE_SELFHOSTED`, `RIVET_INSPECTOR_DISABLE`, `RIVET_LOG_LEVEL`, `RIVET_EXPOSE_ERRORS` (refused when `1`), `RIVET_DEV_SERVERLESS_URL`, `BETTER_AUTH_URL` (and every other `BETTER_AUTH_*` name except `BETTER_AUTH_SECRET`, above), `DATABASE_URL`, `EMAIL_FROM`, `RESEND_API_KEY`, `VERCEL_*`. `EUCHRE_GATEWAY_SECRET` was never needed (12.3 Option A).

Engine service, 3.1.0. The config is `deny_unknown_fields`, so the 2.3 `RIVET__FEATURES__*` and `RIVET__FILE_SYSTEM__PATH` stop the boot (`unknown field`) and are removed:

`PORT=6420` (the guard port, which serves `/health`; the 6421 API peer is loopback-only in 3.x), `RIVET__AUTH__ADMIN_TOKEN` (`preserve()`), `RIVET__STORAGE__LOG_DIRECTORY=/data/v3/log`, `RIVET__OBJECT_STORAGE__PRIMARY__FILE_SYSTEM__PATH=/data/v3/objects`, `RIVET__STORAGE__BALANCING__ENABLED=false`, `RIVET__STORAGE__AUTO_PROVISION_PARTITIONS=1`, `RIVET__STORAGE__LOG_SHARD__MAX_DISK_BYTES=4294967296`, `RIVET__STORAGE__COLD_STORE__DISK_CACHE_BYTES=0`, `RIVET__STORAGE__STOP_TIMEOUT_MS=80000`, `RIVET__STORAGE__STOP_TEARDOWN_MS=20000`, `RIVET__RUNTIME__GUARD_SHUTDOWN_DURATION=60`, `RIVET__RUNTIME__FORCE_SHUTDOWN_DURATION=110`. Start command `/bin/sh -c "mkdir -p /data/v3/log /data/v3/objects && exec /usr/bin/rivet-engine"`: the image entrypoint is the engine, and 3.x has no `start` subcommand and refuses a missing log directory.

---

## 7. Deployment (as built)

### 7.1 Railway topology (`.railway/railway.ts`, `server/README.md`)

Project `euchre` id `8a81234f-6775-4d79-8bab-5db9da547d57`; production environment `4e5631d6-9139-47e0-aaf3-2e41dad8d801`; app service id `71db2501-0a70-4eca-983e-acbb64479a59`. Region `us-east4-eqdc4a`.

| Service | Source | Public | Network | Resources | Deploy |
|---|---|---|---|---|---|
| `rivet-engine` | image `rivetdev/engine:2.3.23@sha256:e6d1cbfe85847bc7c3c6ed7e5b3dc6cf9aaed32e8e42d6778ac9423bd928f2bb` | NO (no domain) | private `rivet-engine.railway.internal:6420` (Guard/API), health on `6421` via `PORT=6421`, healthcheck path `/health`, timeout 120 s | cpu 2, memory 2,000,000,000 B | 1 replica, `drainingSeconds 120`, `overlapSeconds 0`, restart ALWAYS, volume `rivet-data` mounted at `/data` |
| `euchre` | Dockerfile build (`builder DOCKERFILE`, `buildEnvironment V3`) | YES, `https://euchre.sh`, `https://www.euchre.sh`, Railway preview origin | port 3000, healthcheck `/health`, timeout 120 s | cpu 2, mem 2,000,000,000 B | 1 replica, `drainingSeconds 120`, `overlapSeconds 60`, restart ALWAYS |
| volume `rivet-data` | | | | `sizeMB 10000`, `allowOnlineResize true`, region as above, usage alerts 80/95/100 % | keep one engine replica with its attached RocksDB volume |

Engine env (preserve secrets): `PORT, RIVET__AUTH__ADMIN_TOKEN, RIVET__FEATURES__GUARD_GATEWAY_V3__MODE, RIVET__FEATURES__GUARD_GATEWAY_V3__PERCENTAGE, RIVET__FILE_SYSTEM__PATH`. App env (all `preserve()`): `ALLOWED_ORIGINS, APP_URL, EUCHRE_ACTOR_JWT_SECRET, EUCHRE_INTERNAL_TOKEN, NODE_ENV, OPENROUTER_API_KEY, ORIGIN, PORT, RIVETKIT_RUNTIME_MODE, RIVET_ENDPOINT, RIVET_ENGINE_SELFHOSTED, RIVET_EXPOSE_ERRORS, RIVET_INSPECTOR_DISABLE, RIVET_LOG_LEVEL, RIVET_NAMESPACE, RIVET_POOL, RIVET_PUBLIC_ENDPOINT, RIVET_TOKEN`. Actor version upgrades drain 1 actor per 60 s. Host must match a configured origin; keep each game on the hostname where it was created (guest cookies are host-only).

Release checks (README): `bun run check`, `bun run test:gateway`, `bun run build`, `railway up --project ... --environment ... --service ... --detach`; then smoke: create game, play, reload, create another; unrelated guest cannot get a token; `/rivet/actors` and `/api/rivet/start` return 404. Project rules (Railway): never use `railway add`, service-delete, or `railway down`; `railway config apply` is human-only.

### 7.2 Dockerfile

Three stages from `oven/bun:1.4.2`: (1) build: copy `package.json bun.lock patches rivetkit-svelte/package.json`, `bun install --frozen-lockfile --ignore-scripts`, copy all, `bun run build`; (2) dependencies: `bun install --production --frozen-lockfile --ignore-scripts`; (3) runtime: `ENV NODE_ENV=production PORT=3000`, copy package files, prod `node_modules`, `rivetkit-svelte`, `build`, `src`, `server`; `USER bun`; `EXPOSE 3000`; `CMD ["bun","server/index.ts"]`. (`src` ships because actors run from TS source.)

### 7.3 Smoke / security scripts

- `scripts/actor-security.mjs <gameId>`: against local engine `http://127.0.0.1:6420`, for params `{}`, `{token:'dev'}`, `{internalToken:'forged-match-token'}`: `euchreTable.get(['table', id], {params}).snapshot()` must reject and `table.send('aiDecision', {internalToken:'forged-match-token', moveId:'pass'})` must reject. Prints `Six unauthorized table reads/publishes rejected.`
- `scripts/deployment-smoke.ts [origin=http://localhost:5280] [--play-hand] [--review-tricks]`: (1) `GET /api/rivet/metadata` `clientEndpoint === origin + '/rivet'`; (2) `GET /play/__data.json` (redirect manual) -> `location` + `set-cookie`; (3) `POST /api/table-session/<id>` with Origin+Cookie -> ok; (4) rivetkit client at `origin + '/api/rivet'` with `token: gatewayToken`, `gateway:{skipReadyWait:true}`, `getForId(actorId,{params:{token}})`, `.connect()`; patched WebSocket to assert origin and send Cookie/Origin; `snapshot` action; optional play loop (`submitMove {moveId, turnId, clientMoveId}`; pass preferred; `reviewTrick` set/continue); (5) negative checks: anonymous session POST -> 401; other guest -> 403; cross-origin -> 403; `/rivet/actors`, `/rivet/namespaces`, `/api/rivet/start` -> 404.

### 7.4 Gateway token timing quirks to preserve

`expiresAt` is 15 min (900 s); browser refreshes 10 s early; actor token 2 h; WS auth happens at connect time (conn params), so a long-lived socket outlives the 15-minute gateway token (engine only checks at upgrade).

---

## 8. Test inventory to port (behavioural spec)

`server/rivet-gateway-proxy.test.ts` (classify allow/deny matrices; trusted-IP derivation incl. `ip:port`, `[v6]:port`; reason cap by bytes; Origin gate; FIFO order under backpressure; WS echo text+binary with protocols+origin forwarded; upstream close code/reason mirrored incl. 1001; abrupt browser termination closes upstream; one frame cannot overshoot buffer budget; bad Origin rejected pre-dial; HTTP passthrough forwards method/query/body/headers without injecting credentials; XFF overwritten; client-identity headers stripped; `/metadata` stub; admin surfaces denied by proxy (404 not engine 401); HTTP Origin gate), `server/bootstrap-rivet-namespace.test.ts` (skip-POST when exists; POST when missing; name_not_unique race; cursor pagination; 403 fatal; 5xx retry; deadline exhaustion; env-safety matrix), `server/public-origin.test.ts`, `src/lib/actors/auth/guest.test.ts` (session vs actor audience separation, tamper rejection, short-secret refusal in production). Port each as Rust unit/integration tests.

---

## 9. Dead / not to port

`src/lib/server/auth/**` (Better Auth config `createAuth`, `authStore` actor keyed `["auth"]`, drizzle schema tables `user, session, account, verification, rateLimit`, magic-link via Resend, `internal-token.ts` label `euchre:authStore:internal:v1`, session 7 d/update 1 d, magic link 10 min, rate rules 5/300 s link request, 20/300 s verify, 120/60 s global), `src/lib/server/db/**`, `drizzle.config.ts`, the JWKS branch of `verifyPlayer`, the `dev` token literal (`DEV_PLAYER_TOKEN='dev'` is REJECTED by `verifyPlayer`; `DEV_USER_ID='dev-user'` is unused). Do not register an `authStore`.

---

## 10. Rust architecture (proposal)

### 10.1 Crates and layout

```
Cargo.toml (workspace)  +  turbo.json / package.json (Turborepo wrappers)
packages/protocol      -> wire types (shared with 01/02 lanes): PersonaConfig, ProfileSettings, TablePrefs, MatchRecord, ...
packages/engine, packages/ai
apps/server
  src/main.rs                bootstrap: env, namespace ensure, registry.serve_with_config, axum
  src/config.rs              env parsing + validation (envy/serde; boot guards of 5.1)
  src/auth/{mod,guest,profile_token,internal,identity,secrets}.rs
  src/actors/{player_profile,euchre_table,ai_seat,health}/...
  src/http/{mod,health,tables,session,profile,identity,presets,metadata}.rs
  src/gateway/{mod,classify,proxy_http,proxy_ws,origin,client_ip}.rs
  src/bootstrap/engine_namespace.rs
apps/desktop              Rune client + device identity store
```
Key crates: `axum` (with `ws`), `tokio`, `tower-http` (cors, trace, set-header, services::ServeDir), `jsonwebtoken` 9, `hmac`+`sha2` (internal token digest, email hash), `subtle` (`ConstantTimeEq`), `rand`/`getrandom`, `uuid` (v4), `reqwest` (rustls) for engine admin + OpenRouter, `tokio-tungstenite` for the upstream WS leg, `serde`, `serde_json`, `regex`, `unicode-normalization`, `thiserror`, `anyhow`, `tracing`, `directories` (desktop), `cookie` (parsing/serialising `Set-Cookie`), `time`.

### 10.2 `jsonwebtoken` token code (exact parity)

```rust
// guest
#[derive(Serialize, Deserialize)]
struct GuestClaims { kind: String, sub: String, iss: String, aud: String, iat: u64, exp: u64 }
const GUEST_ISS: &str = "euchre-guest";
fn sign_guest(user_id: &str, aud: GuestAud) -> Result<String> {
    let now = unix_secs();
    let exp = now + match aud { GuestAud::Session => 400*86_400, GuestAud::Actors => 2*3600 };
    let claims = GuestClaims { kind:"guest".into(), sub:user_id.into(), iss:GUEST_ISS.into(), aud: aud.as_str().into(), iat: now, exp };
    encode(&Header::new(Algorithm::HS256) /* typ: set header.typ = None to match jose's bare {"alg":"HS256"} */, &claims, &EncodingKey::from_secret(key()?))
}
fn verify_guest(token: &str, aud: GuestAud) -> Result<String> {
    let mut v = Validation::new(Algorithm::HS256);
    v.set_issuer(&[GUEST_ISS]); v.set_audience(&[aud.as_str()]);
    v.leeway = 0; v.validate_exp = true; v.required_spec_claims = ["exp","iss","aud","sub"].into_iter().map(String::from).collect();
    let data = decode::<GuestClaims>(token, &DecodingKey::from_secret(key()?), &v)?;
    if data.claims.kind != "guest" || !data.claims.sub.starts_with("guest-") { bail!("Invalid guest identity") }
    Ok(data.claims.sub)
}
```
Notes: `jsonwebtoken::Header::new` sets `typ: Some("JWT")`; set `header.typ = None` for byte parity (verifiers accept either). `jsonwebtoken` never auto-adds `iat`; set it explicitly. Verify must pin `algorithms = [HS256]`; reject others before decode (alg-confusion defence).

Profile token (hand-rolled checks; use `jsonwebtoken` only for signature+aud+exp then re-apply the extra rules, or decode with `Validation::insecure_disable_signature_validation` after a manual HMAC check). Required claims struct: `{ sub: String, aud: OneOrMany<String>, exp: i64, iat: Option<i64>, nbf: Option<i64>, iss: Option<String> }`. Enforce in the same order and with the same error messages as 2.5 (tests assert messages only at log level; the wire code is `invalid_token`). Use `Validation { leeway: 60, validate_nbf: true }` for skew and then explicitly check `exp - iat <= 3600` and `sub.len() in 1..=128`.

```rust
fn mint_profile_token(user_id: &str) -> Result<String> {  // 30 minutes
    let now = unix_secs();
    let claims = json!({ "sub": user_id, "aud": "euchre-actors", "iat": now, "exp": now + 1800 }); // + "iss" only if EUCHRE_JWT_ISSUER set
    encode(&Header { typ: Some("JWT".into()), alg: Algorithm::HS256, ..Default::default() }, &claims, &EncodingKey::from_secret(profile_secret()?))
}
```

Internal token: `subtle::ConstantTimeEq` on `Sha256(expected)` vs `Sha256(supplied)`; single error `internal_token_mismatch`.

Secret loading (as built, `euchre_server_core::config`): `ServerConfig::from_env` resolves every secret at boot with the Part 5.1 rules, as TS does. The 16-character minimum and the fixed dev keys apply only in local dev, which means `RIVET_ENDPOINT` unset; the guest and identity dev keys also need `NODE_ENV` other than `production`. A deployment (`NODE_ENV=production`, or a remote engine) needs three distinct secrets of at least 32 characters. There is no `--dev` flag and no `EUCHRE_DEV`. `ServerConfig::local` (desktop and embedded) never uses the dev keys: it generates its secrets once and keeps them in `data_dir/secrets.json` (0600).

### 10.3 `playerProfile` in Rust (rivetkit 3.1.0)

Register: `registry.register_actor::<PlayerProfile>("playerProfile")`. Key `["user", userId]` (`ActorKey`). Because the Rust SDK differs from TS:

| TS feature | Rust realisation |
|---|---|
| `c.state` (settings, activeGameId, dailyTokens, lifetime) | One KV key `b"state"` holding the JSON of `ProfileActorState`; load in `Actor::create`, hold `parking_lot::Mutex<ProfileActorState>`, write-through with `ctx.kv().put` after every mutation (probe in `scratchpad/rivet-probe` proves the pattern). Run `normalize_settings` in `create` (the TS `run` start). <!-- critic: superseded by 08 section 2 and ARCHITECTURE 2.4: `ctx.kv()` is deprecated in 3.0 (value cap 131072 bytes); store the state JSON in a single-row SQLite table next to the 3.6 schema. A `parking_lot::Mutex` cannot be held across the `.await` of a write-through (08 G3); use a `tokio::sync::Mutex` lane that is held across persist, as the table does. --> |
| `c.db` SQLite + `onMigrate` | `ctx.sql()` (`SqliteDb`) with `SqliteDbExt::transaction`; run `apply_migrations` in `create` or `on_start` before serving (idempotent ledger as 3.6). Exact bind-parameter API is **UNVERIFIED** (`db_query(sql, params: Option<&[u8]> /* CBOR */)` exists; confirm in `rivetkit-core` `SqliteDb`). <!-- critic: verified in 08 section 2: `query/run/execute(sql, Option<Vec<BindParam>>)`, `execute_batch(Vec<SqliteBatchStatement>)` for atomic multi-statement writes, positional `?` placeholders. --> |
| `createConnState` | No equivalent. Authenticate in `on_before_connect(params)`: verify profile token and `claims.sub == key[1]`, returning `Err` to refuse; token-less connection allowed (internal calls). Per-action authority: re-derive from `ctx.conn().and_then(|c| c.params().ok())`; cache `conn_id -> authenticated bool` in a `DashMap`, remove in `on_disconnect`. **UNVERIFIED**: whether `ctx.conn()` is `Some` for stateless HTTP action calls in 3.1.0; if `None`, browser-facing actions must carry the token as an explicit first argument or the server (HTTP API) must call them with the internal token. <!-- critic: answered by 08 section 1 and G5: `ctx.conn()` is Some for stateless HTTP calls (a temporary conn; on_before_connect and on_connect run on every call), None only for scheduled/internal dispatch. --> |
| queues `recordGame` etc. | Rust actors have no queues. Implement as **actions** with the same names and bodies (`recordGame(body)` etc., `internalToken` inside the body exactly as TS). Idempotent repository functions make caller retries safe (table already retries via `flushProfile`). Serialize via an async `Mutex` inside the actor (replaces the single run-loop lane). Forged tokens return `internal_token_mismatch` (TS silently dropped queue messages; an action error is strictly safer). |
| `UserError(code)` | Use `rivet_error` / the SDK's user-error type so the client sees `{group:'user', code:'<protocol code>', message}`. **UNVERIFIED** exact API <!-- critic: verified in 08 section 8: `rivet_error::RivetError { kind: RivetErrorKind::Dynamic { group: "user", code, default_message }, .. }`; do not use `#[derive(RivetError)]` (G15) -->; the desktop/web clients key off `code` (`invalid_token`, `forbidden`, `invalid_persona`, `internal_token_mismatch`, `internal_error`, ...). |
| `c.conn.state` | n/a (see above) |
| action arg decoding | actions take `serde_json::Value` first, then validate by hand like `guards.ts` (never rely on serde strictness for caller errors; keep `invalid_persona` vs `internal_error` code mapping). <!-- critic: per 08 G32 a `struct X(Value)` or `struct X { v: Value }` action binds the WHOLE positional args array, not the first arg. Use the verified `struct X((Value,))` shape (08 section 11 and section 14 rows for updateSettings/getReplay). --> |

Action names, argument order, return JSON and error codes: exactly as 3.8/3.9. `ConnParams`: `#[derive(Deserialize, Default)] struct ProfileConnParams { token: Option<String>, protocol_version: Option<u32> /* rename protocolVersion */ }`.

Settings, sanitiser, repository: port 1:1 into `apps/server/src/actors/player_profile/{settings,guards,repository,schema}.rs` with the Part 3 golden vectors (Part 14). Put pure logic (sanitize, apply patch, normalize, clamp, cursor, stats math, redaction) in `packages/protocol` or a `euchre-profile` crate so it is unit-testable without an engine and reusable by the desktop for offline settings validation.

Rust persona types:
```rust
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all="camelCase")]
pub struct PersonaConfig { pub id: String, pub version: u32, pub name: String, pub blurb: String, pub prompt: String, pub house_prompt: String, pub difficulty: Difficulty, pub aggression: f64, pub risk: f64, pub chattiness: f64, pub temperature: f64, pub model_id: ModelId, pub bid_model_id: ModelId }
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)] #[serde(rename_all="lowercase")] pub enum Difficulty { Rookie, Casual, Expert }
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)] #[serde(rename_all="lowercase")] pub enum Pace { Brisk, Normal, Relaxed }
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)] pub enum ModelId { #[serde(rename="claude-haiku-4-5")] ClaudeHaiku45, #[serde(rename="claude-opus-5")] ClaudeOpus5 }
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)] #[serde(rename_all="camelCase")]
pub struct TablePrefs { pub pace: Pace, pub banter: bool, pub show_rationale: bool, pub sound: bool, pub reduce_motion: bool }
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)] #[serde(rename_all="camelCase")]
pub struct ProfileSettings { pub schema: u8, pub version: u32, pub updated_at: i64, pub house_prompt: String, pub personas: [PersonaConfig; 3] /* serde arrays up to 32 ok */, pub table: TablePrefs }
```
Patches deserialize from `serde_json::Value` manually (`apply_settings_patch(&ProfileSettings, &Value, now) -> Result<ProfileSettings, ProfileError>`), so type errors map to `invalid_persona` with the exact TS messages. Numbers: accept any JSON number as `f64` (JS `Number`); reject non-finite (JSON cannot carry NaN, so only strings/null/bools hit the "must be a finite number" branch; `Number('abc')` NaN from the web form arrives as `null` through JSON.stringify and must also map to `invalid_persona`).

Timestamps: `i64` ms. Version counters: `u32`. `winnerTeam`/`firstDealer`/seats: small ints.

### 10.4 Where Rust-hosted HTTP routes call actors

`apps/server` holds a `rivetkit-client` `Client` (`ClientConfig::new(RIVET_ENDPOINT).token(RIVET_TOKEN).namespace(NS).pool_name(POOL).encoding(Cbor)`), the same way TS `getRivetClient()` does, and calls actors with `getOrCreate`/`get`/`create` and per-call `params: {token}`. The HTTP layer never touches the engine's data plane except through this client and the engine admin API.

---

## 11. Rust HTTP API (axum) for desktop AND web

Goal: JSON endpoints that cover every capability the SvelteKit server code had (form actions, page loads, session mint), so the web app can drop `+page.server.ts` and call these from the browser (or SvelteKit `load` via `fetch`).

### 11.1 Authentication of API callers

An API caller is identified by `euchre_guest` cookie **or** `Authorization: Bearer <guest session jwt>` (aud `euchre-session`, same token). Browser callers use the cookie; desktop uses the bearer header. Resolution order: Bearer, then cookie. `identity` (email nickname) travels the same two ways: cookie `euchre_email` (web) or a signed identity token (11.4).

CSRF: state-changing browser (cookie-authenticated) requests require `Origin == public origin` (exact match against the resolved origin for the request host, as the TS session route does with `url.origin`). Bearer-authenticated requests are exempt (not ambient). CORS: `tower_http::cors` limited to `ALLOWED_ORIGINS` with `allow_credentials(true)`, methods GET/POST/PUT/PATCH/DELETE/OPTIONS, headers `content-type, authorization`.

Error body (all non-2xx JSON): `{ "error": { "code": "<snake_code>", "message": "<plain text>" } }`; SvelteKit-compatible `message` also present at top level for the unchanged web client: `{ "message": "...", "error": {...} }`. Codes reuse `PROTOCOL_ERROR_CODES` plus `bad_request`, `unauthorized`, `forbidden`, `not_found`, `unavailable`.

### 11.2 Endpoints

| Method + path | Purpose | Request | Success | Errors |
|---|---|---|---|---|
| `GET /health` | liveness/readiness (503 `draining` while shutting down or before the envoy connected) | | `200 text "ok"` (match `registry.routes.health()` body; capture it from a running TS instance) | 503 |
| `GET /api/rivet/metadata` | gateway discovery (browser + desktop `lookup_metadata`) | | `200 {"clientEndpoint":"<publicOrigin>/rivet","clientNamespace":"<ns>"}` `Cache-Control: no-store`; host must resolve via public-origin resolver else `421 "Unknown host"` | 421 |
| `GET /api/session` | who am I (replaces implicit cookie reads) | auth (cookie or bearer) optional | `200 {"guestUserId": "guest-..."|null, "identity": {"email": "...", "userId": "<hex>"}|null}` (never sets cookies) | |
| `POST /api/device/enroll` | desktop first run: mint a device guest identity | `{}` (optional `{"deviceName":"..."}`) | `200 {"userId":"guest-<uuid>","sessionToken":"<jwt aud euchre-session, 400d>","expiresAt":<ms>}` | 429 (rate limit) |
| `POST /api/device/refresh` | rotate session token before expiry | Bearer valid session token | `200` same shape (new token, same `userId`) | 401 |
| `POST /api/identity/identify` | email nickname (replaces `?/identify`) | `{"email":"..."}` | `200 {"identity":{"email","userId"},"identityToken":"<signed>"}`; also sets `euchre_email` cookie for web (see 11.4) | 400 `{"error":{"code":"bad_request","message":"That doesn’t look like an email address yet."}}` |
| `DELETE /api/identity` | sign out of nickname (replaces `?/signout`) | | `204`; clears cookie | |
| `POST /api/tables` | create a table (replaces `GET /play`) | `{"stick": bool?, "natural": bool?}` (JSON; also accept query `?stick=true&natural=true`) | `201 {"gameId":"<uuid v4>"}` + `Set-Cookie euchre_recent_tables` (web) | 401 not applicable (guest minted on demand for cookie callers; bearer callers must be enrolled), 503 |
| `GET /api/tables/recent` | recent tables (replaces `loadRecentTables`) | cookie list, or `?ids=a,b,c` for bearer clients (desktop keeps its own list locally) | `200 {"tables":[{"gameId","status","score","handNo"}],"unavailable":bool}` | |
| `POST /api/table-session/{gameId}` | mint connection credentials | none (Origin check only for cookie callers) | `200` body exactly as 4.2 with `Cache-Control: no-store` | 400 `Invalid table`, 401 `Please start a game first`, 403 `Origin not allowed` / `This table belongs to another browser`, 503 `Your table is temporarily unavailable. Please reconnect.` |
| `GET /api/profile/settings` | settings (replaces `/settings` load) | identity required | `200 ProfileSettings` | 401 (no identity), 503 |
| `PUT /api/profile/settings` (alias `PATCH`) | save (replaces `?/save`) | `SettingsPatch` JSON (3.5.4) | `200 ProfileSettings` | 400 `invalid_persona` with the actor's message, 401 `Tell us who you are before saving.` |
| `GET /api/profile/personas` | PersonaView[] | identity | `200 PersonaView[]` | |
| `GET /api/profile/stats` | ProfileStats | identity | `200 ProfileStats` | |
| `GET /api/games?cursor=&limit=` | scrapbook page (replaces `/games` data) | `profile=browser|email` (default: email identity if present else guest) | `200 MatchListPage` (`limit` default 10 for UI parity, clamped 1..50 by actor) | 400 bad cursor, 401 |
| `GET /api/games/{matchId}?profile=browser` | replay | | `200 ClientMatchReplay` | 404 `No game like that in your scrapbook.`, 400 bad id |
| `GET /api/episodes?limit=` / `DELETE /api/episodes/{episodeId}` | list/forget memories | identity | `200 StoredEpisode[]` / `200 {"forgotten": bool}` | |
| `GET /api/presets` | persona presets (3.5.5) | none | `200 PersonaPreset[]` | |
| `ANY /rivet/**` | public gateway to the private engine (Part 12) | | | |

Profile resolution (`profile=`): `email` -> `userId = sha256hex(email)`; `browser` -> `guestUserId`. Both mint a 30-min profile token internally (server-side only; never returned) and call `playerProfile` with `params:{token}` (or the Rust server may call internal-token actions on the owner's behalf, avoiding the token round trip - but keep the token path for web parity).

Web migration notes: `GET /play` remains a thin redirect route for the old UI (`POST /api/tables` then `302 /play/<id>`); `POST /games?/identify` etc. map to the endpoints above; cookie names/attributes stay identical so an old and a new server can share a browser session during cutover.

### 11.3 Cookies (Rust, via the `cookie` crate)

| Cookie | Value | Attributes |
|---|---|---|
| `euchre_guest` | guest session JWT | `Path=/; HttpOnly; SameSite=Lax; Max-Age=34560000; Secure` (`Secure` exactly when `APP_URL` starts with `https://`; never in `ServerConfig::local`) |
| `euchre_recent_tables` | up to 12 comma-joined lowercase UUIDs | same attrs |
| `euchre_email` | normalized email (legacy) OR signed identity token | same attrs |

Host-only (no `Domain`). The server derives the public origin from the Host header via the exact-match resolver (never trust `X-Forwarded-Host`/proto), as `createPublicOriginResolver` does.

### 11.4 Identity: web email nickname (fix) and desktop device identity

**Problem**: the unsigned `euchre_email` cookie lets anyone claim any email's profile. **Proposal** (backwards compatible):
- Keep `userId = sha256hex(normalizeEmail(email))` so existing profile actors/history stay reachable.
- On `identify`, issue `identityToken` = HS256 JWT `{kind:"email-nick", sub:<userId>, em:<normalized email>, iss:"euchre-identity", aud:"euchre-identity", iat, exp (400d)}` signed with a THIRD secret `EUCHRE_IDENTITY_SECRET` (>= 32 chars) and set it as `euchre_email` cookie value (or `euchre_identity`). Accept the legacy plain-email cookie only when `EUCHRE_ALLOW_UNSIGNED_EMAIL=1` (default off after migration), mapping it to the same userId. Still not proof of mailbox ownership; adding a mailbox challenge (magic link) is future work (the dead better-auth path shows the earlier intent).
- This changes the semantics deliberately; flag as an owner decision (Open question Q2).

**Desktop device identity** (no browser, no cookie jar):
1. First launch: client calls `POST /api/device/enroll`. Server mints `userId = "guest-" + uuid_v4` and a session JWT (aud `euchre-session`, 400 d, identical to the cookie value) and returns it.
2. Desktop persists `{ userId, sessionToken, expiresAt, serverUrl }` as JSON in the OS app-data dir via `directories::ProjectDirs::from("sh","euchre","Euchre")` -> `data_dir()` (macOS `~/Library/Application Support/sh.euchre.Euchre/`, Windows `%APPDATA%\euchre\Euchre\data`, Linux `$XDG_DATA_HOME/euchre`). File `device.json`, created with mode `0600` (Unix) / user-only ACL (Windows), written atomically (temp + rename). Optional hardening: store `sessionToken` in the OS keychain (`keyring` crate) and keep only `userId` on disk.
3. Every API call: `Authorization: Bearer <sessionToken>`. Refresh at 80 % of lifetime via `POST /api/device/refresh`; on 401 re-enroll (new identity; old history is orphaned, same as clearing browser cookies; offer export/import of `device.json` for migration).
4. The `guest-` prefix is mandatory: the table's `verifyGuest` requires `sub.startsWith('guest-')`, and the same user id family keeps the web `/games?profile=browser` flow working.
5. Optional stronger variant (recommended if tokens leaking from disk is a concern): client generates an Ed25519 keypair (`ed25519-dalek`), stores the private key in the app-data dir/keychain, and enrolls `{publicKey}`; the server stores `publicKey` in a `deviceRegistry` actor keyed `["device", userId]` and authenticates `POST /api/device/refresh` with a signed timestamp challenge. The device "secret" then never crosses the wire after enrollment.
6. Email nickname on desktop: `POST /api/identity/identify` returns `identityToken`; the desktop stores it next to `sessionToken` and sends header `X-Euchre-Identity: <identityToken>` (server accepts header or cookie).
7. Actor connections from the desktop (`rivetkit-client`): `GetOrCreateOptions/GetOptions { params: Some(json!({"token": <actorToken>})) }` where `actorToken` comes from `POST /api/table-session/{gameId}` (the `token` field; renew when `expiresAt` nears). The returned `gatewayToken` is set as `ClientConfig::token(...)` so the client sends it as `rivet_token.<jwt>` and in `/gateway/<actorId>@<jwt>/connect` (Part 12.2). `getForId(actorId, ...)` against `endpoint = <publicOrigin>/rivet`, `.disable_metadata_lookup(true)` (the server's `/rivet/metadata` is a `{}` stub), `encoding(Cbor)`.

---

## 12. Rust gateway in front of a private engine

### 12.1 Process composition

One Rust process:
- `tokio::spawn(registry.serve_with_config(ServeConfig { endpoint: RIVET_ENDPOINT, token: Some(RIVET_TOKEN), namespace: RIVET_NAMESPACE, pool_name: RIVET_POOL, version: <u32 build/deploy number>, engine_spawn: EngineSpawnMode::Never /* prod */, start_services: false, ..Default::default() }, shutdown))` registers the actor runner (envoy) with the private engine. Replace TS `RIVET_ENVOY_VERSION=Date.now()` with a `u32` that increases per deploy (e.g. `BUILD_NUMBER` env or `Utc::now().timestamp() as u32`). As built, local mode (`RIVET_ENDPOINT` unset) does not use rivetkit's `EngineSpawnMode::Always`: `apps/server` starts the engine itself through `euchre_server_core::rivet::LocalEngine` on `RIVET_RUN_ENGINE_PORT` (default 6420), with downloads off. The binary comes from `RIVET_ENGINE_BINARY_PATH` or rivetkit's cache at `~/.rivetkit/engine/v3/3.1.0/`; `bun run dev:server` sets the first to the binary `bun install` put in `node_modules` (`apps/server/scripts/dev.sh`). Section 13 has the rest.
- `axum::serve` on `[::]:PORT` serving: `/health`, JSON API (Part 11), `/rivet/**` gateway, static web assets (`tower_http::services::ServeDir` with `precompressed_br/gzip`, `Cache-Control: public,max-age=31536000,immutable` for `/immutable/` paths else `public,max-age=86400`).
- Boot order: validate env/secrets (5.1 guards) -> `ensure_rivet_namespace` (reqwest, 5.4, retry 1 s, deadline 60 s, fatal on 401/403) -> start registry and wait until connected (health stays 503 until then) -> start axum. SIGTERM: flip `draining`, `registry` shutdown (grace 90 s), axum graceful shutdown (`with_graceful_shutdown`) , exit 0.
- Port the namespace bootstrap and env-safety functions with the same signatures and the same unit tests (use `wiremock`/`httpmock` for the engine).

### 12.2 Gateway behaviour (axum + tokio-tungstenite)

Router: `.route("/rivet/metadata", get(metadata_stub))` and `.route("/rivet/*rest", any(rivet_gateway))`. `rivet_gateway` runs `classify_path` (port the regexes verbatim, plus the Rust-client path forms), then per kind:

**Allowed paths** (strictly an allowlist; all else `404 {"error":"not_found"}`):
- WS: `/connect` (TS browser form, requires subprotocols below), `/gateway/<seg>/connect`, `/gateway/<seg>/websocket[/...]` where `<seg>` is `[^/]+` (the Rust client uses `/gateway/<actorId>@<token>/connect`; the token in the path segment must be treated as a credential: never log the path; redact `@...`).
- HTTP: `/gateway/<seg>/action/<name>`, `/gateway/<seg>/queue/<name>`, `/gateway/<seg>/request[/...]`.
- Tighten vs TS (recommended): for the public gateway, require that `<seg>` is a concrete actor id (`^[a-z0-9_-]{20,64}(@[A-Za-z0-9._~%-]+)?$`) so name-based/query targets (which could create or getOrCreate actors) are unreachable; `/rivet/connect` (TS form) additionally requires the three subprotocol predicates of 5.2 (`rivet_target.actor`, `rivet_actor.<id matching ^[a-z0-9_-]{20,64}$>`, `rivet_token.<[A-Za-z0-9_.-]+>`) and an empty query string. Never forward `/actors`, `/namespaces`, `/runner-configs`, `/datacenters`, `/ui`, `/auth/tokens`, inspector, health, dynamic reload.

**Admission checks, in order**: (1) Origin allowlist (absent Origin passes; native desktop sends no Origin); (2) for browser callers, `euchre_guest` cookie must be a valid session JWT (TS `server/index.ts`); for bearer callers, `Authorization: Bearer <session jwt>` or a valid gateway token suffices - the gateway token itself is actor-scoped and short-lived, and the actor re-checks ownership from conn params; (3) rate-limit per trusted client IP (add `tower_governor` or a token bucket; the engine Guard limit is a backstop, not a feature).

**Forward**: strip hop-by-hop and credential headers (list in 5.3: `connection, keep-alive, transfer-encoding, upgrade, host, content-length, cookie, authorization, forwarded, x-real-ip, x-client-ip, true-client-ip, cf-connecting-ip, x-cluster-client-ip`), overwrite `X-Forwarded-For` with `derive_trusted_client_ip` (LAST XFF entry, strip port, validate `IpAddr`, fallback to `ConnectInfo` socket addr; warn once per minute on unparseable). HTTP passthrough via a shared `reqwest::Client` (no redirects, 120 s timeout, streaming bodies, 429 logged as `[guard-503]`). WS relay: `axum::extract::ws::WebSocketUpgrade` with `.protocols(["rivet"])` (echo `rivet` when offered), `max_message_size(16 MiB)`, `max_frame_size(16 MiB)`; dial `tokio_tungstenite::connect_async` with the offered subprotocol list verbatim (`Sec-WebSocket-Protocol`), `Origin` and trusted `X-Forwarded-For` headers; 10 s dial timeout -> close `1011 upstream dial timeout`; two `tokio::spawn` pumps with bounded `mpsc` channels sized to enforce the 16 MiB total buffer (account bytes; over limit -> close both with `1013 backpressure limit`); ordering preserved; ping/pong handled by the library (idle 960 s: use `tokio::time::timeout` on read); close mirroring with `normalize_close_code` (1000-1014 except 1004/1005/1006, or 3000-4999, else fallback 1011 for upstream->client and 1001 for client->upstream) and reasons truncated on a UTF-8 char boundary to <= 123 bytes. On either side ending, close the other.

### 12.3 Gateway token strategy (what replaces `table.issueToken`)

Constraint: Rust `rivetkit`/`rivetkit-client` 3.1.0 has no `issue_token`. Two options:

- **Option A (parity, preferred)**: the Rust server calls the engine admin API exactly like the TS client does: `POST {RIVET_ENDPOINT}/auth/tokens` with `Authorization: Bearer <RIVET_TOKEN>` and JSON `{"namespace":"<ns>","subject":"<userId>","duration":900,"grants":[{"resource":"actor_gateway","target":{"id":"<actorId>"},"operations":["read"]}]}`; response gives `{token, issuedTs, expiresTs}` (TS maps to `{token, issuedAt, expiresAt}` in ms). The gateway forwards the token verbatim; the engine validates it. **UNVERIFIED for engine 3.1.0** (`/auth/tokens` shape observed in TS client `rivetkit@2.3.23`); probe against the 3.1.0 engine binary at `~/.rivetkit/engine/v3/3.1.0/` (scratchpad `rivet-probe`) before committing. Resolve `actorId` via `client.get(name,key).resolve()` (`handle.resolve()` exists in `rivetkit-client` 3.1.0).
- **Option B (engine-independent)**: mint our own HS256 gateway JWT `{iss:"euchre-gateway", aud:"euchre-gateway", sub:userId, act:<actorId>, iat, exp<=900s}` signed with `EUCHRE_GATEWAY_SECRET`; the gateway verifies it, checks `act` equals the actor id in the path/subprotocol, then **replaces** the credential upstream with `RIVET_TOKEN` (rewrite the `rivet_token.` subprotocol entry and the `@<token>` path segment). Costs: the gateway now injects the admin credential, so the allowlist and actor-id check become the only barrier between a client and the engine; keep the Part 12.2 tightening mandatory.

Both keep `expiresAt = now + 900 s` and the 10 s client-side early refresh. In both, the guest actor JWT in conn params remains the actor-level authorization (ownership enforced inside `euchreTable`).

### 12.4 Railway / Docker for the Rust server (as built, W4)

- Services unchanged in shape. The `euchre` app service is public and has no git source: a human deploys it with `railway up` (`docs/DEPLOY.md` sections 3.3 and 8), and Railway builds the root `Dockerfile`, with healthcheck `/health` on `PORT` 3000, 1 replica, `overlapSeconds 60` and `drainingSeconds 120`. `rivet-engine` is private: image `rivetdev/engine:3.1.0@sha256:fd679281d8a357d20081a6639933d1fcaecdc6fc087604f7a05ee0905a8db731`, volume `rivet-data` at `/data`, healthcheck `/health` on 6420, 1 replica, `overlapSeconds 0`. The env lists are in 6.1. The engine and `rivetkit` are both 3.1.0.
- Engine storage (D10). Engine 3 cannot read 2.3 storage, and the TS tables are not migrated, so engine 3 starts fresh under `/data/v3` on the same volume. The 2.3 RocksDB stays at the volume root for rollback. On used storage, never change the guard port or add a topology config: engine 3 ties its token-signing ring to them.
- Root `Dockerfile` (every base image pinned by multi-arch index digest, tag kept for reading):
  - `chef` stage: `lukemathwalker/cargo-chef:0.1.78-rust-1.99.1-slim-trixie`, with `RUSTUP_TOOLCHAIN=1.99.1` so rustup does not fetch clippy and rustfmt.
  - `planner` stage: fails if `rust-toolchain.toml` and `ARG RUST_VERSION` differ, then runs `cargo chef prepare`.
  - `server` stage: `cargo chef cook --release --locked -p euchre-server`, then `cargo build --release --locked -p euchre-server`.
  - `web` stage: `oven/bun:1.4.4-slim` runs `bun install --frozen-lockfile` and `bun run build` in `apps/web`. It takes `build/client` (or `build`) only when it holds `index.html`, else it fails (D4). The adapter precompresses (`.br`, `.gz`), as TS adapter-node did, and the server sends those to clients that accept them.
  - Runtime: `debian:trixie-slim` with `ca-certificates` only; non-root user `euchre` (uid/gid 10001); WORKDIR `/app` holding `/app/euchre-server` and `/app/build/client`; `ENV PORT=3000`; `CMD ["/app/euchre-server"]`. No `curl` and no Docker `HEALTHCHECK`: Railway runs its own healthcheck on `/health`.
  - No secrets and no source tree in the image, and no BuildKit cache mounts (Railway wants `s/<service-id>` mount ids).
  - `apps/web/Dockerfile`, the TS image, is deleted.
- The web app is a static SPA (`adapter-static`, fallback `index.html`), served from `build/client` by the Rust server. There is no second runtime.

---

## 13. Dev experience (Rust)

As built (W3/W4). This replaces the planned `--dev` flag, `EUCHRE_DEV=1` and `engine_auto_download`, none of which exist.

- `euchre-server` takes no arguments. `RIVET_ENDPOINT` unset means local mode, exactly as TS `isLocalDev()`. The server then starts its own engine on `RIVET_RUN_ENGINE_PORT` (default 6420; it also binds that port + 1 and + 5), with storage under `RIVETKIT_STORAGE_PATH` (default rivetkit's own), and stops it on shutdown. It never downloads the engine: the binary comes from `RIVET_ENGINE_BINARY_PATH` or rivetkit's cache at `~/.rivetkit/engine/v3/3.1.0/`, and boot fails with a plain error otherwise. `bun run dev:server` runs `apps/server/scripts/dev.sh`, which sets `RIVET_ENGINE_BINARY_PATH` to the binary `bun install` put in `node_modules` (`@rivetkit/engine-cli` 3.1.0) unless it is already set.
- Local mode uses the fixed dev keys when no secrets are set, as TS does, unless `NODE_ENV=production` turns on the deployment guards. `Secure` cookies follow `APP_URL` (`https://` only). `ALLOWED_ORIGINS` must list the Vite origin (`http://localhost:5173`); the server always accepts its own loopback origin.
- No Vite serverless middleware is needed: the Rust process runs as an envoy runner against the engine it spawned. `vite dev` proxies `/api` and `/rivet` to it.
- Turborepo: root `turbo.json` pipelines `build/test/lint` wrapping `cargo` commands via `package.json` scripts in `apps/server` and `apps/desktop` (`"build": "cargo build -p euchre-server"`, `"test": "cargo test -p euchre-server"`), plus existing `apps/web` (Bun/Vite) tasks.

---

## 14. Golden test vectors (write before porting)

1. `normalize_email`: `"  Foo@Bar.COM "` -> `foo@bar.com`; `"a@b"` -> none; length 255 -> none; `"a b@c.d"` -> none. `derive_user_id("foo@bar.com")` = lowercase hex SHA-256 of the 11 ASCII bytes `foo@bar.com` (compute in a test against `sha2`; cross-check with `printf 'foo@bar.com' | shasum -a 256`).
2. Guest JWT: header decodes to exactly `{"alg":"HS256"}`; payload has exactly keys `kind,sub,iss,aud,iat,exp`; session `exp-iat = 34_560_000`; actors `exp-iat = 7200`; session token rejected as actors token and vice versa; `sub` without `guest-` rejected; 31-char secret rejected in production mode.
3. Profile JWT: a token minted by the TS `mintProfileToken` with a fixed secret/`iat` verifies in Rust and vice versa (cross-language fixture committed under `apps/server/tests/fixtures/`); `alg:none`, wrong aud, expired (> 60 s skew), `nbf` in future, `iat` in future, lifetime 3601 s, `sub` 129 chars, 2-segment token each yield `invalid_token`.
4. Internal token: equal passes; different length / empty / non-string -> `internal_token_mismatch`.
5. Settings: defaults serialise to the exact JSON of `defaultSettings(0)` (fixture); patch scenarios: name `"  ​Hi\u0000  there  "` -> `Hi there`; prompt with 3+ blank lines collapses to one blank line; 33-code-point name truncated to 32 and re-trimmed; surrogate pair at the boundary not split; `aggression: 1.7` -> 1; `aggression: "0.5"` -> `invalid_persona`; seat `"1"` -> `invalid_persona`; duplicate seat; 4 patches; patch changing both a persona field and `housePrompt` gives persona version +2 and settings version +1; no-op patch still bumps `version` and `updatedAt`; `normalizeSettings` repairs garbage (`id: "Bad Id"` -> default id, dial `NaN`/2 -> default/1, non-string name -> default).
6. Repository: upsert idempotence (insert, in-progress update, finished immutable); hands `ON CONFLICT DO NOTHING`; episode id derivation; cursor round trip and malformed cursors; `listMatches` ordering with equal `played_at`; `computeStats` winRate with 0 decided matches = 0; replay redaction removes non-seat-0 `discard` moves and never exposes `seed`/`deckOrder`; episode ranking = TS ordering on a fixed fixture (this validates the POWER replacement in 3.7).
7. Gateway: port every assertion in 8; plus actor-id-only allowlist, subprotocol predicates, header stripping, WS close-code/reason mapping, `1013` on overrun, Origin gate on both HTTP and WS.
8. Bootstrap: port `bootstrap-rivet-namespace.test.ts` cases verbatim against a mock engine.
9. End-to-end (CI with the spawned 3.1.0 engine): create table via `POST /api/tables`, `POST /api/table-session/:id`, connect with `rivetkit-client` using `gatewayToken` + conn params, `snapshot` works; second guest gets 403; anonymous gets 401; cross-origin gets 403; `/rivet/actors`, `/rivet/namespaces`, `/api/rivet/start` give 404.

---

## 15. Risks (porting hard parts)

1. **Rust actors have no queues and no connection-state hook.** Profile writes become actions; per-action auth must be rebuilt from `ConnCtx::params()`. Whether stateless HTTP action calls expose a `ctx.conn()` is unverified; if not, the whole "token in conn params" authorization model for HTTP-invoked actions needs redesign (explicit auth arg or server-side internal-token calls).
2. **Gateway token minting is not in the Rust SDK**; relying on engine `POST /auth/tokens` (3.1.0) is unverified; fallback Option B means the gateway holds the engine admin credential.
3. **SQLite `POWER()`** in episode ranking is likely unavailable; rank in Rust (spec'd) and test equivalence.
4. **Cross-family JWT parity**: guest vs profile JWTs differ subtly (`typ`, `kind`, `iss`, secret minimum length 32 vs 16, dev fallbacks, 2 h vs 30 min, profile cap 1 h). Easy to merge accidentally; keep separate modules and tests, and ship cross-language fixtures.
5. **Unsigned email cookie** is a real impersonation hole for profile history/settings; fixing it changes behaviour (migration plan needed).
6. **WebSocket relay correctness** under backpressure and close semantics (ordering, 123-byte reason cap, 1013 handling) is where regressions hide; port the tests first.
7. **Token in URL path** (`/gateway/<id>@<token>/connect`) used by the Rust client: must be excluded from access logs, tracing spans and error messages.
8. **Engine/SDK version coupling**: Rust SDK 3.1.0 vs TS rivetkit 2.3.23 clients: the web app and Rust server must speak the same protocol to the 3.1.0 engine; the retargeted web app needs rivetkit TS >= the 3.0.x line. Engine version pin and digest TBD.
9. **Idempotence and ordering without queues**: concurrent `recordGame`/`recordHands` actions must serialize (actor-level async mutex) or rely on SQL guards only; verify rivetkit Rust action concurrency model.
10. **Cookie/host semantics**: guest cookies are host-only and per-origin; `www` vs apex produce different identities (documented behaviour); keep exact-match origin resolution.

## 16. Open questions

- Q1. Does `ctx.conn()` return `Some` for connectionless HTTP actions in rivetkit-rust 3.1.0, and how are `UserError`-style coded errors raised from Rust actions so clients see `code`?
- Q2. Owner decision: fix the unsigned `euchre_email` cookie (signed identity token, optional later magic link) or keep strict parity?
- Q3. Gateway token: Option A (engine `/auth/tokens`) vs Option B (own JWT and admin injection)? Needs a probe on the 3.1.0 engine.
- Q4. Web hosting after retarget: static SPA served by the Rust server vs separate SvelteKit service?
- Q5. Exact 3.1.0 engine image tag and sha256 digest for `.railway/railway.ts`.
- Q6. Should desktop device identity use a bearer session token (simple, mirrors cookie) or Ed25519 enrollment (stronger)? Default proposed: bearer token in app-data dir (0600) with optional keychain.
- Q7. Should the Rust profile expose `dailyTokens` and `activeGameId` read APIs (none exist today and no caller reads them), and should `setActiveGame` be wired into the table at all?
- Q8. Keep `GET /play` side-effect create for the legacy web UI or require `POST /api/tables` immediately?
- Q9. Rate limiting of `POST /api/device/enroll`, `identify`, `table-session` (none in TS beyond the engine Guard).
- Q10. Is the `euchre-profile` pure-logic crate shared with the desktop (offline settings editing/validation), and where do `MatchRecord`/`HandJournalEntry` types live relative to the protocol/engine lanes (must be one definition)?
