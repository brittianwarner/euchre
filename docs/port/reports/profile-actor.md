# Report: SERVER-PROFILE / PROFILE-ACTOR

Crate: `crates/euchre-profile` (the `playerProfile` actor on rivetkit 3.0.7). Owner lane: SERVER-PROFILE.
Spec: 05 §3 and §10.3, contract §2.4, 08 §14 (G32), D17, D21, D27, G35.

`crates/euchre-profile-model` did not change in this stage.

## Gates

Run with `CARGO_TARGET_DIR=/Users/brittianwarner/goods/euchre-rune/.cargo-targets/profile`.

| Gate | Result |
| --- | --- |
| `cargo fmt -p euchre-profile -- --check` | clean |
| `cargo clippy -p euchre-profile --all-targets -- -D warnings` | clean |
| `cargo test -p euchre-profile` | 12 unit passed; 3 e2e skipped (no `EUCHRE_E2E`); 1 doc test ignored (the `register` example) |

The library paths have no `unwrap`/`expect`/`todo!`/`unsafe`. Every public item has a doc comment. Logs never carry tokens; queue-failure logs carry only the action name and a 200-unit error excerpt.

## Engine e2e

```text
EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6721 \
EUCHRE_TEST_STORAGE=/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/rivet-store-profile \
perl -e 'alarm 400; exec @ARGV' cargo test -p euchre-profile --test profile_e2e
```

Result: `3 passed; 0 failed`, in about 10 s.

The engine was started only through `euchre_server_core::testing::TestEngine`. Afterwards, `lsof -iTCP:6721-6730 -sTCP:LISTEN` showed no listeners.

| Test | Covers |
| --- | --- |
| `profile_auth_end_to_end` | **Owner token:** accepted over HTTP and WebSocket, and accepted with no `protocolVersion`.<br>**Bad token:** `invalid_token` over HTTP; `CallError::Rejected` over WebSocket, quickly.<br>**Another user's valid token:** `forbidden` "token does not belong to this profile".<br>**`protocolVersion` 2, `"1"` or `null`:** `protocol_version_mismatch` "unsupported protocol version", checked before the token.<br>**No token:** connects, but every browser action (HTTP and WebSocket) gets `forbidden` "this connection is not authenticated".<br>**Internal reads:** with no token, `internal_token_mismatch` "internal token missing"; with a wrong token, "internal token rejected"; with the right one, they run from an anonymous connection. |
| `profile_actions_end_to_end` | Every action:<br>**Settings:** save, patch-error messages (four) and personas; a refused patch leaves the settings unchanged.<br>**`setActiveGame`/`getActiveGame`:** set and read; a forged token is refused.<br>**`recordGame` + `recordHands`:** a live record, then hands, then the final record; a second final record leaves the finished match unchanged. Checked through `getStats` and the persisted `lifetime` cache.<br>**Bodies:** a forged token gets `internal_token_mismatch`; an invalid body (another user's record, a bad `matchId`) is logged and dropped, as in TS.<br>**`listGames`:** 6 matches, `limit` 2, 3 pages in newest-first order; the default page and `listGames(null)` match; a bad cursor gets `internal_error` "cursor is malformed".<br>**`getReplay` vs `getServerReplay`:** the client view hides seeds, `deckOrder` and seat 1 discards, and keeps seat 0's; the server view has all of them. An unknown id, a bad id and a missing id are checked.<br>**Episodes:** record, list, `listEpisodes(1)`, role ranking by salience; expired rows are hidden; `pruneEpisodes` is proven by re-recording an expired id, which is ignored before the prune and lands after it.<br>**`recordTokens`:** truncation and negative clamping in `dailyTokens`. |
| `profile_durability_end_to_end` | **Concurrent writes:** 24 parallel `recordTokens`, 6 racing first inserts of one match with hands, and 3 parallel settings patches. The lane serializes them: counts are exact, there is one match row, and all 3 patches land.<br>**Sleep and wake:** idle sleep (`sleep_timeout` 1 s, seen by the `test_support` sleep counter), then a wake (start count +1). Settings, the active game, stats, history, the replay, episodes and the persisted state are all unchanged. |

## TS tests ported

None. The TS repo has no tests of the `playerProfile` actor itself. The only player-profile references in its tests are table-side mocks, in `table-context.test.ts`, `decision-delivery.test.ts` and `review.test.ts`, and those belong to the table lane. The e2e above was written new against the TS source (`src/lib/actors/player-profile/index.ts`, `repository.ts`, `guards.ts`).

## How to register

```rust
let auth = euchre_server_core::auth::Authenticator::from_config(&cfg);
euchre_profile::register(&mut registry, auth)?;
// or with a custom rivetkit ActorConfig:
euchre_profile::register_with(&mut registry, auth, rivetkit::ActorConfig::default())?;
```

- `register` installs the `Authenticator` process-wide, then registers `PlayerProfile` as `playerProfile`. The `Authenticator` holds the profile JWT key and `EUCHRE_INTERNAL_TOKEN`.
- **One key set per process.** Calling it again with the same keys returns `Ok`. Different keys return `ConfigureError::KeysConflict` and log `playerProfile keys already configured`; the first keys stay and nothing is registered. See `docs/port/requests/server-profile-auth-scope.md`.
- Call it before the registry serves. An actor that wakes unconfigured fails to start with "playerProfile is not configured: call euchre_profile::register before serving".
- Key: `euchre_profile::actor_key(user_id)` gives `["user", userId]`. Any other key shape fails `create` with "playerProfile must be keyed ['user', userId]".
- **Browser connections** send params `{ token, protocolVersion: 1 }`, where `token` comes from `Authenticator::mint_profile_token`.
- **Actors** (the table) call with no token and pass `internalToken`. For the former queues it goes in the body; for the internal reads it is the first argument.
- **Typed client:** `client.get_or_create_typed::<PlayerProfile>("playerProfile", actor_key(id), opts)`, then `.call(euchre_profile::RecordGame::new(body))` and so on.

## Actions

Arguments are positional, as in TS (08 §14). Each action type is a `Vec<Value>` newtype with a constructor, for example `GetReplay::new(id)`.

**Owner connection only.** Without the owner token: `forbidden` "this connection is not authenticated".

| Action | Args | Returns |
| --- | --- | --- |
| `getSettings` | none | `ProfileSettings` |
| `updateSettings` | `patch` | saved `ProfileSettings`. Errors: `invalid_persona` with the TS message |
| `getPersonas` | none | `PersonaView[]` |
| `getActiveGame` | none | `string \| null` |
| `getStats` | none | `ProfileStats` |
| `listGames` | `{ limit?, cursor? }?` | `MatchListPage`. Cursor errors: `internal_error` |
| `getReplay` | `matchId` | `ClientMatchReplay \| null` (redacted) |
| `listEpisodes` | `limit?` | `StoredEpisode[]` (unexpired) |
| `forgetEpisode` | `episodeId` | `boolean` (see parity findings) |

**Internal token only.**

| Action | Args | Returns |
| --- | --- | --- |
| `getServerReplay` | `internalToken, matchId` | `ServerMatchReplay \| null` |
| `getEpisodesForRole` | `internalToken, role, limit?` | `StoredEpisode[]`, ranked |
| `pruneEpisodes` | `internalToken` | `null` |

**Former queues, internal token in the body.**

| Action | Body | Returns |
| --- | --- | --- |
| `recordGame` | `{ internalToken, record, hands? }` | `null` |
| `recordHands` | `{ internalToken, matchId, hands }` | `null` |
| `recordEpisodes` | `{ internalToken, episodes }` | `null` |
| `setActiveGame` | `{ internalToken, gameId }` | `null` |
| `recordTokens` | `{ internalToken, tokensIn, tokensOut, cacheReadTokens, calls }` | `null` |

**Errors.** Every error is a public `user_error` (D21), with codes and messages copied from TS (D27). The internal-token errors are:

- "internal token missing"
- "internal token rejected"
- "EUCHRE_INTERNAL_TOKEN is not configured; internal writes are refused"

## Design notes

- **Storage:** state lives in SQLite through `JsonTable` (`profile_state`/`state`), never `ctx.kv()`. Migration `0001_initial` is the 05 §3.6 schema verbatim, applied through a ledger in `create`. The repository is SQL over `ctx.sql()`. Multi-row writes use the atomic `execute_batch`.
- **Single writer:** every write runs under `ActorLane::enter()`, in the order persist, then emit, then ack. The actor declares no events, so nothing is emitted. Reads do not take the lane.
- **Persist, then publish (08 §2):** every state change is made on a copy, the copy is saved, and only a saved copy becomes the live state (`commit_with`). If the save fails, memory is left as it was. No read serves an unsaved value, and nothing unsaved reaches disk with a later save.
- **Auth (D17):** each action re-derives auth from `ctx.conn().params()`. A missing conn is treated as internal (anonymous), so it must pass the internal token. `on_before_connect` runs the same check, so a bad token or a wrong owner is refused at connect.
- **Internal token:** compared in constant time by `Authenticator`.
- **`on_start`:** normalizes the stored settings and persists them. There is no run handler (G35).

## Deviations

1. **The `Authenticator` is a process-wide static.** rivetkit's `Actor::create(ctx)` takes no factory argument, so `register` stores the keys in a `RwLock`. There is one key set per process: a second, different set is refused (`ConfigureError::KeysConflict`), never swapped in.
2. **Arguments are `Vec<Value>` newtypes, not typed tuples.** This mirrors TS `unknown` arguments: a missing or `null` argument reads as absent, and each handler validates its own arguments with the TS messages.
3. **A forged internal token on a former queue gets an error.** TS dropped it silently. Here the action fails with `internal_token_mismatch` and logs the warning "dropped forged <name>" (10.3).
4. **Other queue failures:** a body that fails validation, or a stored row that fails to decode (`RepoError::Data`), is logged ("playerProfile queue handler failed") and acknowledged, as TS dropped it. A SQLite failure returns an error, so the caller retries. A retry is safe because a failed save publishes nothing. The SQL writes are idempotent (`INSERT OR IGNORE`, the match upsert), and so are `setActiveGame` (an absolute set) and the lifetime cache (recomputed from SQL). `recordTokens` is **not** idempotent: it adds. A retry adds it once only because the failed attempt left memory and disk unchanged. If a save succeeds but the ack is lost, the retry counts the tokens twice. TS had no retry path, so it could not double-count.
5. **No explicit SQL transaction around the match upsert.** The lane serializes the read-then-write. The e2e race test shows 6 racing first inserts give one row.
6. **`updateSettings` persists before it returns.** TS relied on auto-save of `c.state`.
7. **Settings repair:** settings are normalized when decoded in `create` and again in `on_start`, which persists the repaired state.
8. **Test hook:** `test_support` records starts, sleeps and the last persisted state per owner, so tests can read `lifetime` and `dailyTokens`, which have no read action. It records nothing until a test calls `test_support::enable()`. This is a runtime switch, not a Cargo feature, so feature unification cannot turn it on in a server build.

## Parity findings, ported as-is (D27)

- **`forgetEpisode` can never delete a stored episode.** TS validates `episodeId` with `requireId` (`/^[A-Za-z0-9_-]{1,128}$/`), but `episodeId()` builds `role.matchId.handNo.kind`, with dots. Every real id gets `internal_error` "episodeId must match /^[A-Za-z0-9_-]{1,128}$/". The e2e pins this behaviour. The fix belongs to the spec owner (allow `.` in this one pattern). It was not changed here.
- **An array `stats.tricks` reads as `[0, 0]`.** `stats.tricks` is read with `pick(tricks, "0")`, which only sees plain objects, so an array `[a, b]` reads as `[0, 0]`. The model documents this; the e2e uses the object form `{ "0": a, "1": b }`.

## Risks

- **Argument size cap:** rivetkit's 64 KiB incoming-argument cap can reject a `recordGame` that bundles many hands. Callers should send hands through `recordHands` in chunks.

## Files

- `/Users/brittianwarner/goods/euchre-rune/crates/euchre-profile/Cargo.toml`
- `/Users/brittianwarner/goods/euchre-rune/crates/euchre-profile/src/lib.rs`: register API
- `/Users/brittianwarner/goods/euchre-rune/crates/euchre-profile/src/actor.rs`
- `/Users/brittianwarner/goods/euchre-rune/crates/euchre-profile/src/actions.rs`
- `/Users/brittianwarner/goods/euchre-rune/crates/euchre-profile/src/auth.rs`
- `/Users/brittianwarner/goods/euchre-rune/crates/euchre-profile/src/repository.rs`
- `/Users/brittianwarner/goods/euchre-rune/crates/euchre-profile/src/test_support.rs`
- `/Users/brittianwarner/goods/euchre-rune/crates/euchre-profile/tests/profile_e2e.rs`

Request: `/Users/brittianwarner/goods/euchre-rune/docs/port/requests/server-profile-auth-scope.md` (one key set per process).

## Review fixes

Gates after the fixes (`CARGO_TARGET_DIR=.cargo-targets/profile`), for both crates:
- `cargo fmt -- --check`: clean.
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo test -p euchre-profile`: 17 unit tests pass; the 3 e2e tests return early as skips (no `EUCHRE_E2E`).
- `cargo test -p euchre-profile-model`: 12 unit tests and 10 integration tests pass.

Engine e2e, with `EUCHRE_E2E=1` on port 6721: 3 of 3 pass (auth, actions, durability). Afterwards nothing listened on 6721 to 6730.

| Finding | Outcome |
| --- | --- |
| P2: `recordTokens` adds the increment in memory before the save and never rolls it back, so a retry double-counts | **Fixed.** `recordTokens` now commits through `commit_with`, which edits a copy and publishes it only after the save. A failed save leaves memory unchanged, so the retry adds the increment once. Deviation 4 now says which writes are idempotent. Test: `actor::tests::failed_save_leaves_state_and_retry_counts_tokens_once` injects a failing save, then retries. |
| P2: former-queue handlers change state before the save and never roll back (`setActiveGame`, the lifetime cache, `updateSettings` visible before its save) | **Fixed.** Every state write goes through `commit_with`: `updateSettings`, `on_start`'s settings repair, `setActiveGame`, `recordGame`'s lifetime cache and `recordTokens`. The old `persist` that saved whatever was in memory is gone, so a later save cannot write an unsaved value to disk. Reads that skip the lane now see only saved state. Test: `actor::tests::failed_edit_publishes_nothing`, plus the test above. |
| P3: in multiline mode, `\r` followed by U+2028, U+2029 or U+0085 gives two newlines (TS gives one) | **Fixed.** The exotic-break mapping now runs on the char stream before the CR fold peeks at the next char, which is the TS order. Test: `guards::tests::cr_then_exotic_break_folds_like_crlf`. The expected values were checked against TS `sanitizeText` under bun: `"a\nb"` for each break, `"a\n\nb"` for `a\r\r\u2028b`, and `"a b"` in single-line mode. |
| P2: the `Authenticator` is one global per process, so a second `register()` swaps the keys | **Fixed.** `configure`, `register` and `register_with` return `Result<(), ConfigureError>`. The same keys again return `Ok`. Different keys return `KeysConflict`: the error is logged, the first keys stay and nothing is registered. The crate docs and deviation 1 say "one key set per process". Filed `docs/port/requests/server-profile-auth-scope.md` for INTEGRATION and FOUNDATION. Test: `tests::configure_keeps_one_key_set_per_process`. |
| P3: deterministic data errors (`upsert_plan`, `stats_from_row` decode) become Storage, so the caller retries forever | **Fixed.** `repository::upsert_match` and `compute_stats` return a typed `RepoError::{Data, Storage}`. In a former queue, `Data` becomes `QueueFailure::Invalid`: it is logged and acknowledged, as TS did. Only SQLite errors are returned for a retry. `getStats` still surfaces a data error as the same `user/<code>` error. Test: `repository::tests::repo_errors_keep_data_apart_from_storage`. |
| P3: the self dev-dependency turns on the `test-support` feature in workspace-wide builds | **Fixed.** The feature and the self dev-dependency are removed. Recording is behind a runtime `test_support::enable()` (an `AtomicBool`, off by default), which only the e2e calls. Feature unification cannot turn it on. Test: `test_support::tests::records_nothing_until_enabled`. |
