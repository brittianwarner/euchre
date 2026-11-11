# Request to SERVER-FOUNDATION: the auth fixture scripts import deleted TS code

From: W4 integration, 2026-11-11. Nothing is broken in any gate; this is a dev tool.

## What

`crates/euchre-server-core/scripts/` regenerates and cross-checks the auth goldens with the real TS auth code:

- `auth-fixture-env.ts` sets `WEB = <repo>/apps/web/src`.
- `gen-auth-fixtures.ts` imports `${WEB}/lib/actors/auth/guest.ts` and `${WEB}/lib/actors/player-profile/auth.ts`.
- `verify-rust-tokens.ts` also imports `${WEB}/routes/games/_lib/session.server.ts` and `${WEB}/../node_modules/jose/...`.

W4 removed all of these from `apps/web` (no TS actor code may remain, ARCHITECTURE §7), and `jose` is no longer a web dependency. So `bun run --cwd crates/euchre-server-core fixtures` (and the `EUCHRE_WRITE_RUST_TOKENS` verify step) now fail at import. The committed fixtures in `tests/fixtures/auth` are unchanged, and `cargo test -p euchre-server-core` still passes against them.

## Suggested fix (server-core lane)

The goldens are frozen TS behaviour, so the generator only makes sense against the TS original:

1. Read the TS source root from an env var, for example `EUCHRE_TS_SRC` (the read-only checkout's `src`, `~/goods/euchre/src`), and fail with a clear message when it is unset or missing.
2. Resolve `jose` from that checkout's `node_modules`.
3. Say in the script header and in the crate docs that the fixtures are frozen and need the TS checkout only to regenerate.

Or delete the scripts and the `fixtures` package script, and keep the fixtures as frozen goldens.
