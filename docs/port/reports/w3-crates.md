# W3 report: lane S2-CRATES

**Date:** 2026-11-11
**Crates touched:** `euchre-server-core` (config only), `euchre-http` (`ApiConfig::from_server_config` and the trusted-proxy path), `euchre-profile-model` (D30 only). `euchre-table` is unchanged; the 64 KiB check did not need a change.

## Result

| Item | State |
| --- | --- |
| Trusted proxy (`requests/server-http-trusted-proxy.md`) | Done. The request is closed and moved to `requests/done/`. |
| D30 `forgetEpisode` | The model side is done and tested. Two callers outside this lane still use the old guard, so D30 is not live until they switch. That work is in `requests/server-profile-d30-forget-episode-callers.md`. |
| 64 KiB limit | Measured with the real types and through a real 3.0.7 engine. Neither body can exceed the limit today, so `euchre-table` is unchanged. Hazards and numbers are below. |

## 1. Trusted proxy (D11 buckets and the engine Guard)

### Settings

| Env | Default | Meaning |
| --- | --- | --- |
| `EUCHRE_TRUST_FORWARDED_FOR` | off | `1` honors the last `X-Forwarded-For` entry. Unset, empty or `0` means off. Any other value fails at boot. |
| `EUCHRE_TRUSTED_PROXIES` | unset | An optional comma-separated list of IPs or CIDRs, such as `10.0.0.0/8, fd00::/8`. When set, the header is honored only if the socket peer is in the list. Setting it without the flag fails at boot. |

The default is off in every mode: local, desktop, production, and an external engine. The setting no longer depends on `production` or `engine.mode`. If a deployment boots with the flag off, it logs one `warn`: every client behind an edge then shares the edge's bucket. That fails safe, because limits get tighter, not looser.

**Railway (W4):** set `EUCHRE_TRUST_FORWARDED_FOR=1` on the euchre service in `.railway/railway.ts`. Leave `EUCHRE_TRUSTED_PROXIES` unset unless the edge's address range is known. This is documented in 05 §6 (env table) and §12.4. The repo has no `.env.example` for the Rust server; the only one is `apps/web/.env.example`, which is out of scope. Until the flag is set, production keys every bucket on the edge IP.

### Code

- **`euchre-server-core/src/config.rs`:**
  - `HttpConfig` has two new fields, `trust_forwarded_for: bool` and `trusted_proxies: Vec<IpCidr>`, plus `HttpConfig::trusts_forwarded_for_from(peer)`.
  - New `forwarded_for_trusted(enabled, proxies, peer)`.
  - New `IpCidr`:
    - `new`, `host`, `contains`, `parse_list`, `FromStr` and `Display`.
    - `new` rejects host bits and folds an IPv4-mapped IPv6 network to IPv4.
    - `contains` matches an IPv4 peer against an IPv6 network through its mapped form.
  - `ServerConfig::local` sets the setting off.
- **`euchre-http/src/config.rs`:** `ApiConfig` has a new `trusted_proxies` field and `trusts_forwarded_for_from(peer)`. `from_server_config` copies both settings from `cfg.http`.
- **`euchre-http/src/api/mod.rs`:** two new functions.
  - `forwarded_client_ip(state, headers, socket)` returns the trusted client IP. It applies only when trust is on for this peer. It reads the last entry of the last header line, strips any port, and canonicalizes the address.
  - `pin_forwarded_for` rewrites the header before `/rivet/**`. When trust is on, it leaves exactly that one entry. Otherwise it removes the header, so the proxy falls back to the socket IP.

  As a result, the D11 bucket and the engine Guard key on the same address, and a direct client cannot pick either one.
- **`euchre-http/src/gateway/public.rs`:** `rivet_entry` calls `pin_forwarded_for` before the gateway bucket.

### API compatibility

All changes are additive. Code that builds `HttpConfig` or `ApiConfig` with a struct literal outside these crates needs the new fields (`trusted_proxies: Vec::new()` and, for `HttpConfig`, `trust_forwarded_for: false`). Today no such code exists: `apps/server` and the desktop crate build neither. `cargo check -p euchre-server -p euchre-profile -p euchre-table -p euchre-ai-seat --all-targets` is clean.

### Tests

**`euchre-server-core` config tests:**
- `forwarded_for_is_off_unless_set_in_every_mode`
- `forwarded_for_flag_parsing`
- `trusted_proxies_parse_and_gate_the_peer`
- `ip_cidr_matching`
- `forwarded_for_trusted_needs_the_flag`

**`euchre-http` config tests:**
- `a_local_server_never_trusts_forwarded_for`, extended.
- `from_server_config_reads_the_explicit_setting_not_the_mode`

**`euchre-http/src/api/tests.rs`:**
- `a_direct_client_cannot_pick_its_bucket_when_trust_is_off`: rotating the header does not reset the bucket.
- `trusted_forwarded_for_keys_on_the_last_entry_of_the_last_line`
- `trusted_proxies_honor_forwarded_for_only_from_the_proxy`
- `the_engine_guard_gets_the_same_client_ip_as_the_buckets`: an echo upstream shows the header the engine receives.

## 2. D30: `forgetEpisode` accepts stored ids

- **`euchre-profile-model/src/guards.rs`:** new `EPISODE_ID_PATTERN_TEXT`, `is_valid_episode_id` and `require_episode_id`, re-exported from `lib.rs`. `require_id`, `is_valid_id` and `ID_PATTERN_TEXT` are unchanged.
  - **Accepted:** exactly `role.matchId.handNo.kind`. `role` and `matchId` match the existing id pattern. `handNo` is a canonical decimal in `0..=10000`. `kind` is one of the five episode kinds.
  - **Refused:** `/`, `%`, whitespace, `..`, leading zeros, extra or missing parts, and unknown kinds.
  - The error is `internal_error` with the message `episodeId must match <role>.<matchId>.<handNo>.<kind>`.
  - The guard is separate from `ID_PATTERN`. Widening that pattern would let `.` into `matchId` and `role` and make episode ids ambiguous.
- **`records.rs`:** `MAX_EPISODE_HAND_NO = 10_000` is shared by the parser and the guard.
- **Tests:**
  - `an_episode_is_forgotten_by_the_id_it_was_stored_under`: records an episode through `parse_record_episodes` and forgets it by the stored id.
  - `every_storable_episode_id_is_forgettable`
  - `malformed_episode_ids_are_still_refused`
- `reports/profile-model.md` records this as deviation 10, a deliberate deviation from TS (D30).

**The actor crate needs a change, contrary to the expectation that it would not.** `crates/euchre-profile/src/actor.rs` still calls `require_id(.., "episodeId")`, and `profile_e2e.rs` pins the TS bug. `euchre-http` `forget_episode` also pre-checks with `is_valid_id`, so `DELETE /api/episodes/{id}` answers 400 for every stored id. Neither caller is in this lane. The exact diffs and gates are in `requests/server-profile-d30-forget-episode-callers.md`. The D30 checklist line stays open until both callers switch.

## 3. The 64 KiB limit and `recordGame` / `recordHands`

### Where the limit comes from (rivetkit 3.0.7)

- `rivetkit-core-3.0.7/src/actor/config.rs:24` sets `DEFAULT_MAX_INCOMING_MESSAGE_SIZE = 65_536`. The queue limit is also 65,536, and outgoing messages are capped at 1 MiB.
- The limit is enforced on the HTTP action body (`registry/http.rs:214`, `:336`) and on each WebSocket message (`registry/websocket.rs:297`). An oversized body fails with `message/incoming_too_long`, HTTP 400.
- The table sends with `handle.action(name, vec![body])` (`euchre-table/src/remote.rs:175`) over the server client's JSON encoding (`euchre-server-core/src/rivet.rs:602`). The wire body is `serde_json::to_vec(&json!({"args": [body]}))` (`rivetkit-client-3.0.7/src/protocol/codec.rs:37`).

### How the worst case was built

The bodies come from the real types: a maxed `apply_settings_patch` → `personas_from_settings` → `MatchRecord` / `HandJournalEntry` → the table's body shape. Every text field sits at its sanitize cap and uses a 4-byte character (an emoji), since the caps count characters, not bytes. The match settings are `gameTo` 15 and `misdealLimit` 255. Ids are at the 128-character id cap, seeds at the 256-unit cap, and `handsPlayed` at the profile's 10,000 cap. Each body was also run back through `parse_record_game` / `parse_record_hands`.

| Body | Wire bytes | Share of 65,536 |
| --- | ---: | ---: |
| `recordGame`, ASCII text, realistic ids | 11,218 | 17% |
| `recordGame`, 3-byte CJK text | 30,130 | 46% |
| `recordGame`, 4-byte emoji, realistic ids | 39,586 | 60% |
| **`recordGame`, emoji, 128/256-character ids, maxed counters (worst reachable)** | **39,965 to 40,190** | **61%** |
| `recordGame`, the same plus all three dossiers at the 1,600-character cap (no creator produces this) | 59,049 to 59,390 | 91% |
| `recordHands`, 1 hand × 40 moves (`JOURNAL_CAP`, what the table sends) | 5,139 to 5,739 | 9% |
| `recordHands`, 1 hand × 128 moves (profile `MAX_MOVES_PER_HAND`) | 15,636 to 15,859 | 24% |
| `recordHands`, 6 hands × 128 moves | 92,721 | 141%, refused |
| `recordHands`, 64 hands × 128 moves (profile batch bound; the table never sends this) | 987,382 | 1,507% |

The ranges reflect the parser probe and the engine probe, which differ slightly in their fixture strings.

- **Parser probe:** `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/size-probe/src/main.rs`.
- **Engine probe:** a scratch crate that runs a real `TestEngine` on port 6870 with `EUCHRE_E2E=1`: `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/engine-probe/tests/worst_case.rs`.

The engine probe passed, and showed:
- The worst-reachable `recordGame` (39,965 B) is stored: `getStats` counts it, and the replay holds three 32-character persona names.
- The dossier-maxed `recordGame` (59,049 B) is stored.
- `recordHands` at 40 and at 128 moves is stored.
- A parser-valid 92,721 B batch is refused with `message/incoming_too_long` and stores nothing.

### Conclusion

- `recordHands` is the per-hand path. The table sends one hand per call (`core.rs` `record_hand`), and `JOURNAL_CAP` bounds each hand at about 5.7 KB. It cannot reach the limit, so it needs no split.
- `recordGame` has about 25 KB of headroom at the worst reachable size. That holds even with the dossiers maxed, which nothing produces today. The table creator (`euchre-http` `create_table` → `personas_from_settings`) always sends an empty dossier, as TS does.
- `recordGame` carries no `hands` from the table. `RecordGameBody` has no hands field, and hands go through `recordHands` one per call.

### Hazards (recommendations, not changed)

1. **Unvalidated personas at table create.** The table copies `personas` from its create input without checking them (`euchre-table/src/state.rs:164`). Today's only creator sanitizes them. A future creator could pass oversized personas, such as filled dossiers or more text. That would turn every `recordGame` for the match into a deterministic 400, and `flushProfile` would retry it forever. Two fixes are recommended, either for the table owner or for W4:
   - Cap the serialized persona size at table create, or re-sanitize the personas through `euchre-profile-model`.
   - Add a regression test that asserts the worst-case `recordGame` wire size stays under 60,000 B.
2. **Counter sum overflow in the profile.** This is a parity edge outside this lane, and it was found by the engine probe. Out-of-range counters saturate: `value_to_int` truncates `u64::MAX` to `i64::MAX`. The second such match on one profile then makes `STATS_SQL`'s `SUM(tokens_in)` overflow. `recordGame` then fails with `sqlite/remote_execution_failed` on every later call for that profile.
   - TS stored these values as REAL and kept working.
   - Only the internal-token holder can send such values, and real lifetime totals are many orders of magnitude smaller.
   - The suggested fix, for the profile owner, is to clamp the token counters in `parse_tokens` to `MAX_SAFE_INTEGER`, the most TS could represent exactly.

## Gates

All gates were run with `CARGO_TARGET_DIR=.cargo-targets/w3-crates`.

- `cargo fmt -p euchre-server-core -p euchre-http -p euchre-profile-model --check`: clean.
- `cargo clippy -p euchre-server-core -p euchre-http -p euchre-profile-model --all-targets -- -D warnings`: clean.
- `cargo test -p euchre-server-core -p euchre-http -p euchre-profile-model`: all pass. That includes 61 config and core unit tests, 131 `euchre-http` lib tests, and the `euchre-profile-model` suites.
- `EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6870 cargo test -p euchre-http --test http_e2e`: 4 passed.
- `EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6870 cargo test -p euchre-server-core`: `foundation_probe` 2 passed.
- The engine probe above passed.
- Dependents still compile: `cargo check -p euchre-server -p euchre-profile -p euchre-table -p euchre-ai-seat --all-targets`.

## Files

- `crates/euchre-server-core/src/config.rs`
- `crates/euchre-http/src/config.rs`, `src/api/mod.rs`, `src/api/tests.rs`, `src/gateway/public.rs`, `tests/http_e2e.rs` (the new field in the config literal)
- `crates/euchre-profile-model/src/guards.rs`, `src/records.rs`, `src/lib.rs`
- `docs/port/05-profile-auth-server.md` (§6 and §12.4)
- `docs/port/reports/profile-model.md` (deviation 10)
- `docs/port/requests/server-profile-d30-forget-episode-callers.md` (new)
- `docs/port/requests/done/server-http-trusted-proxy.md` (resolved and moved)
- `docs/port/W3-checklist.md`

## 4. W-C follow-ups: create-input bounds and saturating stats

Lane W-C (W4 server follow-ups) closed both hazards from §3. It owned `crates/euchre-table/**` and `crates/euchre-profile/**` for this work. `euchre-profile-model` was not edited.

### Hazard 1: the table bounds its create input

`TableState::create` now runs the new `crates/euchre-table/src/create_input.rs` before it builds the game. TS trusted its creator (03 §2: "3 entries, seats 1..3"; TS `TableState`: text "capped by `$lib/protocol`"). The port enforces those bounds.

| Input | Rule | Kind |
| --- | --- | --- |
| `personas` | The first entry per AI seat (1..=3), in input order. A seat-0 entry or a repeated seat is dropped. `persona_for` already read the first entry. | clamp |
| persona `name` / `blurb` | `sanitize_text`, single line, 32 / 120 | clamp |
| persona `prompt` / `housePrompt` / `dossier` | `sanitize_text`, multiline, 2000 / 1000 / 1600 | clamp |
| persona `id` | First 128 UTF-16 units (D27, the profile's id cap) | clamp |
| seed | First 256 UTF-16 units (D27, the profile's `MAX_SEED_UNITS`). The actor uses random hex for an empty seed, as for an absent one. `create` refuses an empty seed. | clamp |
| match id (key) | Must match the profile's `^[A-Za-z0-9_-]{1,128}$`, or the create fails. The profile refuses any other `record.matchId`. | refuse |
| `ownerUserId` / `profileUserId` | At most 128 UTF-16 units, or the create fails. The table cannot shorten an identity. An empty `profileUserId` is treated as absent. | refuse |

- The text caps count code points. That is what TS `sanitizeText` (`Array.from`) and the profile-model sanitizer count, so the creator's output is a fixed point. A unit test proves this for `default_personas()`, `personas_from_settings(default_settings)`, and a settings patch maxed in 4-byte characters.
- D27's UTF-16 rule applies where TS cuts by `.length`: the seed and the id caps.
- A refused create fails `Actor::create` before anything persists. Every real creator passes ids that fit: a uuid key, a `guest-<uuid>` owner (42 characters), and a profile id from `derive_user_id` (64 hex characters).

**Size at the cap.** The regression test `worst_case_record_game_fits_the_action_limit` (in `core/tests.rs`) builds an oversized input and drives a real match to game over. The input has:

- five personas, with every text field at 3x its cap in emoji;
- persona ids, the profile id and the seed made of control characters, which cost 6 JSON bytes per unit;
- a 128-character match id.

The test takes the `RecordGameCall` the table sends and widens every number to the most digits the profile accepts. It serializes the body as it goes on the wire (`{"args":[{"internalToken","record"}]}`, with a 64-character token). It then asserts the size against `rivetkit::ActorConfig::default().max_incoming_message_size`, which is 65,536, and runs the body through `parse_record_game`.

| Body | Wire bytes | Share of 65,536 |
| --- | ---: | ---: |
| Personas alone, as the oversized input sends them | over 65,536 | over 100% |
| **Worst-case `recordGame` after the bounds** | **63,617** | **97%** |

- The test also requires at least 1 KiB of headroom, so a new record field shows up there first.
- §3's suggested 60,000 B target fits every input a creator produces today. It does not fit this deliberately hostile one, whose control-character ids and seed alone add about 3 KB.

### Hazard 2: the profile cannot overflow its stats

- **Clamped storage.** `repository::upsert_match` stores each match token counter clamped to `MAX_STORED_TOKENS`, which is JS `Number.MAX_SAFE_INTEGER` (`clamp_tokens`). It clones the row only when a counter is over the cap.
- **Saturating sums.** `repository::compute_stats` runs `SATURATING_STATS_SQL` instead of `STATS_SQL`. This is the model's statement with each `COALESCE(SUM(x), 0)` replaced by `MIN(TOTAL(x), 9007199254740991)`, with the same aliases.
  - `TOTAL` sums as a float, so it never raises "integer overflow". It is exact while a total stays under 2^53.
  - `stats_from_row` reads a float column as its integer.
  - A unit test derives the statement from `STATS_SQL` and asserts the two match.
- Rows stored before this fix, with `i64::MAX` counters, now sum without error too.

**Deviations, both deliberate.**

- TS bound a past-2^53 count as a REAL and kept working. The port stores it clamped instead, and caps every lifetime total at `MAX_SAFE_INTEGER`.
- §3 suggested clamping in `parse_tokens`. That function is in the frozen `euchre-profile-model`, so the clamp happens at storage in `euchre-profile`.

**Tests.**

- Unit tests:
  - `the_saturating_stats_sql_is_stats_sql_with_capped_totals`;
  - `token_counters_are_stored_clamped_to_the_js_safe_range`;
  - `float_totals_read_back_as_the_capped_integers`.
- The engine test `profile_token_overflow_end_to_end`:
  - records two matches with counters at `u64::MAX`, `MAX_SAFE_INTEGER + 1`, `1e300` and `i64::MAX`, then a normal match;
  - checks that every `recordGame` lands and that `getStats` counts 3 matches with exact counters and token totals at `MAX_SAFE_INTEGER`;
  - checks that `getServerReplay` shows the stored counters clamped and the normal match unchanged;
  - checks that a fourth record still lands.
- With the old `upsert_match` and `STATS_SQL` restored, the engine test fails at `recordGame m_huge_2`. With the fix it passes.

### Gates

All runs used `CARGO_TARGET_DIR=.cargo-targets/w4-server`. Engine tests used `EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6970` and ran one at a time.

- `cargo fmt -p euchre-table -p euchre-profile -- --check`: clean.
- `cargo clippy -p euchre-table -p euchre-profile --all-targets -- -D warnings`: clean.
- `cargo test -p euchre-table -p euchre-profile`: all pass. `euchre-table` lib has 84 tests and `euchre-profile` lib has 20. The engine tests skip without `EUCHRE_E2E`.
- `table_e2e`: 4 passed.
- `seams_e2e`: 1 passed on the second run. The first run failed mid-match with "no sync within 20s" while the machine's load average was about 12 (other sessions' engines). The match had already been created, so create was not the cause.
- `profile_e2e`: 4 passed, including the new test.
- Dependents still compile: `cargo check -p euchre-server -p euchre-http --all-targets`.
- `Cargo.lock` is unchanged. `euchre-profile-model` was already in the lock for `euchre-table`; it moved from dev-dependencies to dependencies.

### Files

- `crates/euchre-table/Cargo.toml` (`euchre-profile-model` moved to `[dependencies]`)
- `crates/euchre-table/src/create_input.rs` (new)
- `crates/euchre-table/src/lib.rs`, `src/state.rs`, `src/actor.rs`, `src/core/tests.rs`
- `crates/euchre-profile/src/repository.rs`, `tests/profile_e2e.rs`
- `docs/port/W3-checklist.md`
