# Report: SERVER-SEAMS

Stage SERVER-SEAMS. It checks every cross-actor call between `euchreTable`, `aiSeat` and `playerProfile` in both directions, then proves them on one engine.

- Owner lane: `crates/euchre-table/**` and `crates/euchre-ai-seat/**`.
- `euchre-profile` and every other crate are frozen and were not changed, and neither was the root `Cargo.toml`.
- The actors talk only by actor name and action name over the shared client (D29). None imports another's actor type at runtime.

## Cross-actor calls

All calls are stateless: `get_or_create(name, key, { params, create_with_input })` then `.action(name, [body])`, JSON encoding, one shared `Arc<Client>`.

Tokens:

- **Per-match token:** `TableState.game.internal_token`. The table hands it to each seat in the create input.
- **Deployment token:** `EUCHRE_INTERNAL_TOKEN` = `Authenticator` `keys.internal`. On the table it is `TableConfig.profile_token`; on the seat it is `AiSeatRuntime.profile_token`.

"Code" means checked by reading both sides. "e2e" means exercised by `seams_e2e` against the real callee.

| Caller → callee | Action | Key | Params | Arg (one positional) | Auth on the callee | Budget | Verified |
| --- | --- | --- | --- | --- | --- | --- | --- |
| table → aiSeat | create | `["table", gameId, "seat", "<n>"]` | `{ internalToken }` | create input `{ gameId, seat, internalToken, fenceNonce, persona, dossier, matchId, profileKey? }`, `profileKey = ["user", profileUserId ?? ownerUserId]` only with an owner; decodes as `AiSeatCreateInput` | none at create; the token is stored in seat state | n/a | code + e2e: all 3 seats persisted `profileKey ["user", <guest>]` and the table's `matchId` |
| table → aiSeat | `decide` | same | same | `AIDecideRequest` | `require_internal`: params `internalToken` == state token | `max(1 ms, deadlineAt − now)` | code + e2e: 73 and 104 model-sourced moves (`source: "llm"`) in the two passing runs |
| table → aiSeat | `resetHand`, `handEnd`, `gameEnd` | same | same | `AiSeatLifecycleMessage { internalToken, gameId, kind, handNo? }` | same | 1 s each | code + e2e: `handEnd` produced the episodes, `gameEnd` the token flush |
| aiSeat → table | `aiDecision` | `["table", gameId]` | `{ internalToken }` | `AIDecision` | `origin_gate` lets the internal token through; `identify` gives Internal; `require_internal`; `decode_body::<AIDecision>` | per try ≤ 2 s, ≤ 5 tries 100 ms apart, all within the seat's call timeout (5 s); retry new in this stage | code + e2e: decisions applied as `llm`, no `aiSeat queue handler failed` |
| aiSeat → table | `aiSay` | same | same | `AiSayMsg` (TS producer key order) | same, `decode_body::<AiSayMsg>` | same retry | code + e2e: 72 and 78 chat lines, no `BANTER_UNDELIVERED` |
| table → playerProfile | `recordHands` | `["user", userId]` | none (Anonymous) | `{ internalToken, matchId, hands: [entry] }` | body `internalToken` == `keys.internal`, else `internal_token_mismatch` | 1 s (TS parity) | code + e2e: `getReplay` hands == `handsPlayed` + throw-ins (8 and 11 hands) |
| table → playerProfile | `recordGame` | same | none | `{ internalToken, record }` | same | 1 s, with ledger retry | code + e2e: `listGames` row matches the table on matchId, status, outcome, handsPlayed and score; `getStats` matches, won and lost |
| aiSeat → playerProfile | `recordEpisodes` | `profileKey` from the create input | none | `{ internalToken, episodes: [{ role, matchId, handNo, ts, kind, summary, salience }] }` | same | 5 s single try; on failure the buffer is kept and resent at the next boundary | code + e2e: `listEpisodes` equals each seat's notes, per persona id and match (10 and 9 episodes) |
| aiSeat → playerProfile | `recordTokens` | same | none | `{ internalToken, tokensIn, tokensOut, cacheReadTokens, calls }` | same | 5 s single try (not idempotent, so no retry) | code + e2e: profile `dailyTokens` equals the sum of the 3 seat budgets (760 / 76 calls and 1100 / 110 calls) |
| test (owner) → playerProfile | `listGames`, `getStats`, `getReplay`, `listEpisodes` | `["user", userId]` | `{ token: mint_profile_token(userId), protocolVersion: 1 }` | as TS | owner JWT | 15 s (test) | e2e |

The deployment token is the same `keys.internal` on all three sides because one `ServerConfig` builds them all. No call logs a token.

## Fixes

### aiSeat → table callbacks now retry

This is in `crates/euchre-ai-seat/src/calls.rs` (`send_to_table`), and it implements `requests/server-table-ai-seat-callback-retry.md`.

**The problem.** TS sent `aiDecision` and `aiSay` on a durable queue. The Rust port sends them as actions (D6). An action that reaches a table in its sleep grace fails with `actor/stopping` or hangs. The port tried once, so the decision was lost and the table had to wait for its hard cap and play the fallback.

**The fix:**

- Each try is bounded by `TABLE_TRY_TIMEOUT` (2 s) and made on a fresh handle.
- There are up to `TABLE_TRIES` (5) tries, `TABLE_RETRY_BACKOFF` (100 ms) apart, all inside the runtime's call timeout.
- A refusal (`user/*`, `auth/*`) is final.
- An unbound client fails at once.
- When every try timed out, the error is still `CallError::Timeout { what: <action>, after: <call timeout> }`. The existing failure test, which uses an 800 ms timeout and expects `aiDecision timed out`, still passes.

**Why a retry is safe.** The table drops a duplicate or stale decision (turn fence, first answer wins) and keys chat by `msgId`.

**Where it is used.** `on_decide` (`aiDecision`) and `spawn_banter` (`aiSay`). Profile writes are unchanged.

**Tests.** There are new unit tests for the unbound client and for the refusal classification. Compile-time checks confirm the bounds fit inside `DEFAULT_CALL_TIMEOUT`.

### Test fixture: the owner goes in the create input

This is in `crates/euchre-table/tests/seams_e2e.rs`, not a product change.

**What went wrong.** The first run created the table without `ownerUserId` and let the first connect claim the owner, as `table_ai_e2e` does. The seats were created before the claim, so they had `profileKey: null` and silently dropped their episodes and tokens.

**What production does.** The lobby (`euchre-http` `POST /api/tables`, TS `/play`) always passes `ownerUserId`, so the test now does too. TS behaves the same for an owner claimed later: a seat's create input is fixed at its first call. No change was made.

### Dev-dependencies

`crates/euchre-table/Cargo.toml` gained `euchre-ai-seat = { workspace = true }` and `euchre-profile = { workspace = true }` as dev-dependencies, for the seams test only.

## The proof: `crates/euchre-table/tests/seams_e2e.rs`

`seams_full_match_end_to_end` registers the real `euchreTable`, `aiSeat` and `playerProfile` on one `TestEngine`.

**Setup:**

- Table and seats sleep after 1–2 s idle.
- The seats run a scripted model from `euchre_ai::testing`, with no network. It always answers `move_1` at 10 input tokens per call and says "Mine.".

**The match.** A guest plays a full match against three AI seats over the WebSocket, to `status: "complete"`.

**The checks, read through an owner profile token:**

- `listGames`
- `getStats`
- `getReplay`: hand count, header, and model-sourced AI moves.
- `listEpisodes`: exactly each seat's noted hands, by persona id.
- `dailyTokens`, from `test_support`, because there is no read action for it (D12).
- No seat logged `aiSeat queue handler failed`, `BANTER_UNDELIVERED`, `FLUSH_EPISODES_FAILED` or `FLUSH_TOKENS_FAILED`.

A seat notes a hand only when the last view it was asked about already shows the outcome. Forced moves are never asked, as in TS, so a match can have no episode. When that happens the test plays another match for the same user, up to 4, and checks all of them.

### Command

```text
export CARGO_TARGET_DIR=/Users/brittianwarner/goods/euchre-rune/.cargo-targets/table
EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6841 \
EUCHRE_TEST_STORAGE=<scratchpad>/rivet-store-seams \
perl -e 'alarm 500; exec @ARGV' cargo test -p euchre-table --test seams_e2e -- --nocapture
```

### Result

Two consecutive passing runs on port 6841. Both played one match.

| Run | Hands | Player moves | AI moves (llm / fallback / forced) | Chat lines | Episodes | `dailyTokens` | Time |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 8 | 48 | 73 / 34 / 53 | 72 | 10 | `{"tokensIn":760,"tokensOut":0,"cacheReadTokens":0,"calls":76}` | 50.2 s |
| 2 | 11 | 65 | 104 / 32 / 69 | 78 | 9 | `{"tokensIn":1100,"tokensOut":0,"cacheReadTokens":0,"calls":110}` | 60.0 s |

The fallback moves are the scripted model's own failures, not lost calls. All 64 `jev_failure` lines in run 2 have reason `transport_or_adapter` (`move_1` is not a candidate there), two attempts per fallback.

**Other e2e runs:**

- Re-run against the retry change on the same port: `cargo test -p euchre-ai-seat --test ai_seat_e2e`, 6/6 passed, including `failure_paths_end_to_end` and `sleep_grace_end_to_end`.
- Afterwards `lsof -nP -iTCP:6841-6850 -sTCP:LISTEN` shows nothing listening.

## Gates

| Gate | euchre-table | euchre-ai-seat |
| --- | --- | --- |
| `cargo fmt -p <crate> -- --check` | clean | clean |
| `cargo clippy -p <crate> --all-targets -- -D warnings` | clean | clean |
| `cargo test -p <crate>` | 77 unit passed; e2e binaries (1 + 4 + 4) skip without `EUCHRE_E2E`; 2 doc tests ignored | 19 unit + 6 + 3 passed; 1 doc test ignored; e2e skips without `EUCHRE_E2E` |

## Notes

- **Disk space.** At the start the data volume had under 1.3 GiB free, and the engine needs 4 GiB (`RIVET__STORAGE__LOG_SHARD__MAX_DISK_BYTES`, hard-coded in `euchre-server-core`). This lane removed only its own incremental cache (`.cargo-targets/table/debug/incremental`) and three stale test binaries of its own. It built with `CARGO_INCREMENTAL=0`. Space later freed up elsewhere.
- **No new change requests.** `euchre-profile` compiled and accepted every body as sent, so no stand-in and no `seams-profile.md` request were needed.
