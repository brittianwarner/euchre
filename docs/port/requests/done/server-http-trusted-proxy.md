# Explicit trusted-proxy setting for X-Forwarded-For (SERVER-HTTP)

**For:** the owner of `crates/euchre-server-core` (`ServerConfig`), and 05 §11 / D11.
**Status:** worked around inside `crates/euchre-http`. The real fix needs a `ServerConfig` field.

## Problem

The D11 per-IP buckets (enroll, identify, tables, table-session, gateway) key on the
last `X-Forwarded-For` entry when `ApiConfig.trust_forwarded_for` is on. That entry is
only trustworthy when every request reaches the server through a proxy that appends it
(the Railway edge). Reached directly, a client writes the header itself and picks a
fresh bucket per request.

`ApiConfig::from_server_config` cannot know the topology. It used to infer trust from
`engine.mode == External`. That says where the engine runs, not whether a proxy sits in
front of the server.

## What `euchre-http` does now

- `from_server_config` sets `trust_forwarded_for` from the config's own "deployment"
  rule: `production || engine.mode == External`. A local or desktop server never trusts
  the header.
- `ApiConfig.trust_forwarded_for` stays a plain public field, so a boot crate can set it
  explicitly.
- The `/rivet/**` proxy still stamps the engine's `X-Forwarded-For` from the last entry,
  as TS did. That is the engine Guard's key and is unchanged.

## Ask

Add an explicit setting to `ServerConfig`, for example `http.trust_forwarded_for: bool`
read from `EUCHRE_TRUST_FORWARDED_FOR=1`, or a list of trusted proxy CIDRs that the socket
peer must match before the header is honored. `euchre-http` will then read it in
`from_server_config` instead of inferring it.

## Resolution (lane S2-CRATES, 2026-11-11)

Done as asked, with both options available.

**Settings.** `ServerConfig.http` has two new fields, `trust_forwarded_for` and `trusted_proxies`:
- `EUCHRE_TRUST_FORWARDED_FOR=1` turns trust on. Unset, empty or `0` is off; any other value fails at boot.
- `EUCHRE_TRUSTED_PROXIES` is an optional list of IPs or CIDRs. When set, the header is honored only if the socket peer is in the list. Setting it without the flag fails at boot.
- Trust is off by default in every mode. A deployment that boots with it off logs a `warn`.

**`euchre-http`.**
- `ApiConfig::from_server_config` copies both settings and no longer infers trust from `production` or the engine mode.
- `ApiConfig` has a new `trusted_proxies` field.
- One function, `forwarded_client_ip`, feeds both the D11 buckets and the `/rivet/**` Guard header. When trust is on it uses the last entry of the last line. Otherwise the proxy falls back to the socket IP, so a direct client cannot pick either key.

**Tests and docs.**
- Tests for parsing, the default, the proxy list, and a direct client rotating the header. The details are in `reports/w3-crates.md` §1.
- 05 §6 and §12.4 document the Railway value (`1`).

**Owed (W4).** Add `EUCHRE_TRUST_FORWARDED_FOR = "1"` to the euchre service in `.railway/railway.ts`.

Moved to `done/`.
