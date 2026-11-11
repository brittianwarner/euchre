# Engine 3.0.7 admin API differs from the TS bootstrap (SERVER-HTTP)

**Resolved.** `AdminDialect::V3` is the default and 05 §5.4 describes it (W3 S1-BOOT, `W3-checklist.md`); moved to `done/` in the W4 review fixes.

**For:** the owner of server boot (`euchre-server` / INTEGRATION), and 05 §5.4.
**Status:** worked around inside `crates/euchre-http`. No other crate needs a change. The spec text needs an update.

## What differs

`server/bootstrap-rivet-namespace.ts` targets the rivet-engine 2.3 admin API. The engine this port runs (3.0.7, started through `TestEngine`) answers differently. These results come from the D3 probe and from `tests/http_e2e.rs`:

| Call | Engine 2.3 (TS) | Engine 3.0.7 |
| --- | --- | --- |
| `GET /namespaces?name=<ns>` | `{namespaces:[…]}` | same |
| `POST /namespaces` | `{name, display_name}` | `{name}`. A duplicate gives `409`/conflict, which counts as "exists" |
| Runner pool config | `PUT /runner-configs/{pool}?namespace=` with `{datacenters:{default:{normal:{…}}}}` | `/runner-configs` returns `404`. Use `PUT /pools/{pool}?namespace=` with the flat body `{drain_on_version_upgrade, actor_eviction_period, actor_eviction_rate}` |
| `POST /auth/tokens` | n/a | `{namespace, subject, duration, grants}` → `{expires_ts, issued_ts, token}` in ms (see `gateway-token.md`) |

## What `euchre-http` does

- `AdminDialect::{V2, V3}` on `EnsureRivetNamespaceOptions.dialect`. **V3 is the default.**
  - V2 is the TS behaviour, byte for byte. The 8 ported TS tests run against it.
  - V3 sends `POST /namespaces {name}` and `PUT /pools/{pool}` with the flat body. The error messages keep the TS wording and name `/pools/<pool>` instead of `/runner-configs/<pool>`.
- The retry loop, the deadline, the fatal codes (401/403/400/422 and the `namespace.not_found` quirk) and the never-retried setup errors are the same in both dialects.
- In e2e against engine 3.0.7:
  - The existing namespace returns `namespace_created: false` on attempt 1.
  - A new namespace returns `namespace_created: true` on attempt 1.
  - A second run is idempotent.
  - A wrong admin token fails at once with the TS message (`engine admin auth rejected the token on GET /namespaces (401 auth.invalid_token: …)`).

## Ask

- Update 05 §5.4 to name `PUT /pools/{pool}` and the flat body for engine 3.
- The server boot should call `ensure_rivet_namespace` with the default dialect. Pass `AdminDialect::V2` only against a 2.3 engine.
