# D3 gateway token: probe result and decision (SERVER-HTTP)

**Resolved.** Option A shipped in W3 S1-BOOT (`reports/server-boot.md`, `W3-checklist.md`); moved to `done/` in the W4 review fixes.

**Decision: Option A.** The Rust server mints actor-scoped gateway tokens through the engine admin API (`POST /auth/tokens`). The gateway forwards them verbatim and never injects the admin credential. Option B (our own HS256 gateway JWT plus admin-token injection) is not implemented.

## Probe

- Engine: rivet-engine 3.0.7, started through `euchre_server_core::testing::TestEngine` on port 6731 with a random admin token.
- Actors: a stand-in `euchreTable` with one `snapshot` action. Actor A and actor B were created through the admin client.
- Harness: a temporary integration test in `crates/euchre-http/tests/`. Its cases now live in `tests/http_e2e.rs`.

### Request

```
POST {endpoint}/auth/tokens
Authorization: Bearer <RIVET_TOKEN>
Content-Type: application/json

{"namespace":"<ns>","subject":"guest-probe","duration":900,
 "grants":[{"resource":"actor_gateway","target":{"id":"<actorId A>"},"operations":["read"]}]}
```

### Response

`200 {"expires_ts":1794367709000,"issued_ts":1794366809000,"token":"eyJ0eXAiOiJyaXZldC1hdXRoK2p3dCIsImFsZyI6IkVkRFNBIi…"}`

- `expires_ts` and `issued_ts` are unix **milliseconds** (`expires_ts - issued_ts = 900000`).
- The token is an EdDSA JWT with header `typ: rivet-auth+jwt`. The server cannot verify it locally; the engine does.
- The keys are snake_case. The TS client (rivetkit 2.3) mapped them to `{token, issuedAt, expiresAt}`. The Rust issuer reads `expires_ts` and returns it as `TableSessionResponse.expiresAt`.

### Acceptance (rivetkit-client 3.0.7, JSON encoding, `get_for_id`)

| Client token | Target | Transport | Result |
|---|---|---|---|
| issued token | actor A | HTTP action | OK |
| issued token | actor A | WebSocket `connect()` + action | OK |
| issued token | actor B | HTTP action | `auth/insufficient_permissions` |
| none | actor A | HTTP action | `auth/invalid_token` |
| `bogus.token.value` | actor A | HTTP action | `auth/invalid_token` |
| none or bogus | actor A | WebSocket | the upgrade is refused; the client waits until its timeout |
| issued token | `GET /namespaces` (admin API) | HTTP | `403` |

So the issued token is actor-scoped, short-lived and refused by the admin API. That is exactly what the browser and desktop gateway paths need.

## Implementation

- `euchre_http::GatewayTokenIssuer` sends the request above with a shared `reqwest::Client` and a 10 s timeout. It is enabled when `ApiConfig.engine_token` is set (TS: "`RIVET_TOKEN` present"). It never logs the admin token or the issued token.
- `POST /api/table-session/{gameId}` returns `gatewayToken` and `expiresAt = expires_ts`. Without an engine token the field is omitted and `expiresAt = now + 900000`, as in TS.
- A failed issuance returns `503 "Your table is temporarily unavailable. Please reconnect."`. In TS an `issueToken` throw escaped as an unhandled 500.
- `tests/http_e2e.rs` re-checks the acceptance table above on every e2e run, both directly against the engine and through the `/rivet/**` gateway.
