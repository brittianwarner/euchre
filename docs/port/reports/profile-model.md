# Report: SERVER-PROFILE / PROFILE-MODEL

Crate: `crates/euchre-profile-model` (pure, no I/O). Owner lane: SERVER-PROFILE.
Spec: 05 §3.4-§3.5, the pure parts of §3.7, §3.10, D13, D29.

## Gates

Run with `CARGO_TARGET_DIR=/Users/brittianwarner/goods/euchre-rune/.cargo-targets/profile`.

| Gate | Result |
| --- | --- |
| `cargo fmt -p euchre-profile-model -- --check` | clean |
| `cargo clippy -p euchre-profile-model --all-targets -- -D warnings` | clean |
| `cargo test -p euchre-profile-model` | 21 passed, 0 failed (11 unit, 10 golden) |

There are no engine tests. The stage is pure, so nothing listened on 6721..6730 and no rivet-engine was started.

The library paths have no `unwrap`/`expect`/`todo!`/`unsafe`. Every public item has a doc comment.

## Tests: TS vs Rust

| Source | Count |
| --- | --- |
| TS tests for this logic (`~/goods/euchre` server/profile, settings, presets) | 0 (none exist, so none to port) |
| Rust unit tests (`src/*.rs`) | 11 |
| Rust golden tests (`tests/golden.rs`) | 10 tests over 240+ TS-generated cases |

The goldens cover the following cases:

| Area | Cases |
| --- | --- |
| sanitize | 25 (6 errors) |
| settings patch | 55 (31 errors), chained |
| normalize | 13, each also checked for idempotence |
| defaults, persona views, `personasFromSettings`, presets, default engine cfg | one each |
| match records | 42 (26 errors) |
| hand batches | 24 (17 errors) |
| episode batches | 12 (9 errors) |
| `clampLimit` | 11 |
| `utcDay` | 9 |
| episode ids | one list |
| repository writes | 11 (insert, update, finished-row no-op, hands, episodes) |
| history lists | 21 (9 cursor errors) |
| cursor walk | 4 pages |
| replays | 4 (client redaction and server full, including missing) |
| stats | from the SQL aggregate and from an in-memory fold |
| episode ranking | 6 |
| `listEpisodes` | one |

Every comparison is byte for byte against the `JSON.stringify` text. That includes error `{code, message}`, the SQL text and the bound args the TS repository sent.

### Regenerating the fixtures

```
bun run crates/euchre-profile-model/scripts/gen-fixtures.ts
```

The script imports the TS read-only from `~/goods/euchre`; `EUCHRE_TS_SRC` overrides the path. The clock is fixed at `NOW = 1762819200000`. The TS repository runs on `bun:sqlite` behind a RawAccess shim that logs each call's `{sql, args, rows}`. Output goes to `crates/euchre-profile-model/tests/fixtures/{sanitize,settings,records,repository}.json`.

## Public API (for the actor and HTTP lanes)

All fallible functions return `ProfileResult<T> = Result<T, ProfileError>`, where `ProfileError { code: ProtocolErrorCode, message: String }`. Map it to `UserError` with the same code and message.

Protocol types are reused and never redefined: `PersonaConfig`, `PersonaView`, `PersonaAssignment`, `MatchRecord`, `MatchSummary`, `MatchListPage`, `HandJournalEntry`, `LifetimeStats`, `TokenUsage`, `MatchStats`, `EpisodeKind`, `Seat`, `Suit` and the rest.

### Settings (`settings`, `presets`)

```rust
pub fn default_settings(now: i64) -> ProfileSettings;
pub fn apply_settings_patch(current: &ProfileSettings, patch: &Value, now: i64) -> ProfileResult<ProfileSettings>;
pub fn normalize_settings(raw: &Value, now: i64) -> ProfileSettings;   // repair a stored blob
pub fn persona_view(persona: &PersonaConfig, seat: Seat) -> PersonaView;
pub fn persona_views(settings: &ProfileSettings) -> Vec<PersonaView>;
pub fn personas_from_settings(settings: &ProfileSettings) -> Vec<PersonaAssignment>;
pub fn seed_to_persona(seed: &PersonaSeed, house_prompt: &str) -> PersonaConfig;
pub const PERSONA_PRESETS: [PersonaPreset; 7];
pub fn persona_preset(id: &str) -> Option<&'static PersonaPreset>;
// ProfileSettings { schema, version: i64, updated_at: i64, house_prompt, personas: [PersonaConfig; 3], table: TablePrefs }
// Typed patch builders: SettingsPatch { house_prompt, personas: Vec<PersonaPatchEntry>, table } -> .to_value()
```

### Text and id guards (`guards`)

```rust
pub fn sanitize_value(value: Option<&Value>, opts: SanitizeOptions<'_>) -> ProfileResult<String>;
pub fn sanitize_text(value: &str, opts: SanitizeOptions<'_>) -> String;
pub fn require_id(value: Option<&Value>, field: &str) -> ProfileResult<String>;         // internal_error
pub fn require_client_id(value: Option<&Value>, field: &str) -> ProfileResult<String>;  // game_not_found
pub fn is_valid_episode_id(value: &str) -> bool;                                       // D30
pub fn require_episode_id(value: Option<&Value>) -> ProfileResult<String>;              // D30, internal_error
pub fn require_int(value: Option<&Value>, field: &str, min: i64, max: i64) -> ProfileResult<i64>;
pub fn require_boolean(value: Option<&Value>, field: &str) -> ProfileResult<bool>;
// SanitizeOptions::new(multiline, max_chars, field).with_code(code)
```

### Queue bodies (`records`)

```rust
pub fn owner_of(key: &[String]) -> ProfileResult<String>;            // ['user', userId]
pub fn body_internal_token(body: &Value) -> Option<&str>;           // compare, never log
pub fn parse_record_game(body: &Value, owner: &str, now: i64) -> ProfileResult<RecordGame>; // { record: MatchRow, hands: Option<Vec<HandRow>> }
pub fn parse_record_hands(body: &Value) -> ProfileResult<Vec<HandRow>>;
pub fn parse_record_episodes(body: &Value, now: i64) -> ProfileResult<Vec<StoredEpisode>>;
pub fn parse_set_active_game(body: &Value) -> Option<String>;
pub fn parse_match_record(raw: Option<&Value>, owner: &str, now: i64) -> ProfileResult<MatchRow>;
pub fn parse_hands(raw: Option<&Value>, match_id: &str) -> ProfileResult<Vec<HandRow>>;
pub fn parse_episodes(raw: Option<&Value>, now: i64) -> ProfileResult<Vec<StoredEpisode>>;
pub fn episode_id(role: &str, match_id: &str, hand_no: i64, kind: EpisodeKind) -> String;
impl TokenIncrement { pub fn from_body(body: &Value) -> TokenIncrement; }
pub const QUEUE_NAMES: [&str; 5];
pub fn forged_log_event(queue: &str) -> String;
pub const QUEUE_HANDLER_FAILED_EVENT: &str;
impl MatchRow { pub fn played_at(&self) -> i64; pub fn to_record(&self) -> ProfileResult<MatchRecord>; }
impl HandRow { pub fn to_entry(&self) -> ProfileResult<HandJournalEntry>; }
```

### Actor state (`state`)

```rust
pub fn initial_state(now: i64) -> ProfileActorState;
impl ProfileActorState { pub fn from_stored(raw: &Value, now: i64) -> ProfileActorState; }
impl DailyTokenUsage { pub fn record(&mut self, increment: TokenIncrement, now: i64); }
pub fn utc_day(now: i64) -> String;
pub fn lifetime_from_stats(stats: &ProfileStats) -> LifetimeStats;
pub fn require_authenticated(authenticated: bool) -> ProfileResult<()>;
```

### SQLite schema and row codecs (`sql`)

The rows are driver-neutral: `Column {Null, Integer, Float, Text, Blob}` and `Row(Vec<(String, Column)>)`. The actor lane converts its SQLite driver rows into these.

```rust
pub const LEDGER_DDL / LEDGER_SELECT_SQL / LEDGER_INSERT_SQL: &str;
pub const MIGRATIONS: &[Migration];                  // { id, sql } applied in order
pub const MATCH_EXISTING_SQL: &str;
pub fn upsert_plan(existing: Option<Option<&Column>>) -> ProfileResult<UpsertPlan>; // Insert | Update | Skip
pub fn match_insert_sql() -> String;  pub fn match_columns(record: &MatchRow, now: i64) -> Vec<(&'static str, Column)>;
pub fn match_update_sql() -> String;  pub fn match_update_params(record: &MatchRow, now: i64) -> Vec<Column>;
pub fn hand_insert_sql() -> String;   pub fn hand_columns(hand: &HandRow) -> Vec<(&'static str, Column)>;
pub const EPISODE_INSERT_SQL: &str;   pub fn episode_params(episode: &StoredEpisode) -> Vec<Column>;
pub const EPISODE_CANDIDATES_SQL / EPISODES_LIST_SQL / EPISODE_EXISTS_SQL / EPISODE_DELETE_SQL / EPISODES_PRUNE_SQL: &str;
pub fn episode_from_row(row: &Row) -> ProfileResult<StoredEpisode>;
pub const CLIENT_REPLAY_MATCH_SQL / CLIENT_REPLAY_HANDS_SQL / SERVER_REPLAY_MATCH_SQL / SERVER_REPLAY_HANDS_SQL / STATS_SQL: &str;
// RecordMatchOutcome { match_id, inserted, updated }
```

### History, replay, stats, episodes

```rust
pub fn list_games_args(req: Option<&Value>) -> (i64, Option<&Value>);
pub fn clamp_limit(raw: Option<&Value>) -> i64;
pub fn decode_cursor(raw: Option<&Value>) -> ProfileResult<Option<DecodedCursor>>;
pub fn encode_cursor(played_at: i64, match_id: &str) -> String;
pub fn list_matches_query(limit: i64, cursor: Option<&DecodedCursor>) -> (String, Vec<Column>);
pub fn build_page(rows: &[Row], limit: i64) -> ProfileResult<MatchListPage>;
pub fn client_replay_from_rows(row: &Row, hands: &[Row]) -> ProfileResult<ClientMatchReplay>;  // no seed, no deck, redacted moves
pub fn server_replay_from_rows(row: &Row, hands: &[Row]) -> ProfileResult<ServerMatchReplay>;  // internal only
pub fn redact_moves(moves: &[Value]) -> Vec<JsJson>;
pub fn stats_from_row(row: Option<&Row>, now: i64) -> ProfileResult<ProfileStats>;
pub fn compute_stats(rows: &[MatchRow], now: i64) -> ProfileStats;
pub fn rank_episodes(candidates: Vec<StoredEpisode>, limit: usize, now: i64) -> Vec<StoredEpisode>;
pub fn episode_score(salience: f64, ts: i64, now: i64) -> f64;
```

To fetch episodes for a role, the actor runs `EPISODE_CANDIDATES_SQL` (`ORDER BY ts DESC LIMIT 500`) and then `rank_episodes`.

Opaque JSON (`cfg`, `personas`, `moves`) is carried as `JsJson(Value)`. It serializes exactly as `JSON.stringify` would: JS number text and JS key order.

## Deviations

1. **Persona `version` saturates to u32.** Protocol `PersonaConfig.version` is `u32`, so `normalize_settings` saturates out-of-range stored values. TS keeps any integer. `ProfileSettings.version` and `updatedAt` stay `i64`.
2. **Summary scores saturate to 0..=255.** `MatchSummary.score` is protocol `[u8; 2]`, so stored scores are clamped. Real euchre scores never exceed 255, so no protocol change was requested.
3. **Seed truncation keeps surrogate pairs whole.** It never splits a pair. Rust strings cannot hold the lone high surrogate that TS `slice` can leave.
4. **A non-string stored `activeGameId` reads as `None`.**
5. **Corrupt JSON columns read as `[]`.** When a stored JSON column decodes to the wrong shape, it reads as `[]` or an array of strings and values. TS returns whatever parsed.
6. **Token counters saturate** at `u32`/`u64` instead of growing past them.
7. **Episode ranking caps the candidate set.** It ranks the 500 most recent candidates in Rust, as the spec allows, where TS ranks every row in SQL with `POWER`. The score formula is identical, including SQLite integer division of the age. The goldens confirm the order.
8. **Rust additions with no TS counterpart.** Three helpers exist only here:
   - `ProfileActorState::from_stored`, which repairs a stored state;
   - `compute_stats`, an in-memory equivalent of the SQL aggregate, proven equal by the goldens;
   - the typed `SettingsPatch` builders.
9. **TS quirks preserved deliberately.**
   - `parseStats` reads `tricks` with `pick`, so an array `[5,3]` yields `[0,0]` and only an object `{0:..,1:..}` counts.
   - `clampLimit` and cursor messages are copied verbatim.
10. **`forgetEpisode` accepts stored episode ids (D30, W3).** TS checked the id with `requireId` (`/^[A-Za-z0-9_-]{1,128}$/`), but every stored id is `role.matchId.handNo.kind` (`episode_id`), so TS could never forget an episode. `guards::require_episode_id` / `is_valid_episode_id` are the fix:
    - The id must have exactly four `.`-separated parts.
    - `role` and `matchId` must each match the TS id pattern.
    - `handNo` must be a canonical decimal in `0..=MAX_EPISODE_HAND_NO` (10,000): no sign, no leading zero, ASCII digits only.
    - `kind` must be one of `EPISODE_KINDS`.

    So `/`, `%`, whitespace, empty parts, `..`, extra segments and unknown kinds are still refused. An id that passes can only name a row in the owner's own `episodes` table, where `EPISODE_DELETE_SQL` binds it as the primary key. The error is `internal_error` "`episodeId must match <role>.<matchId>.<handNo>.<kind>`". `require_id` and `ID_PATTERN_TEXT` are unchanged, so every other id keeps the TS contract (D34).

    Tests in `guards::tests`:
    - `an_episode_is_forgotten_by_the_id_it_was_stored_under`: record, then forget by the stored id.
    - `every_storable_episode_id_is_forgettable`: every kind, `handNo` edges, and 128-char parts.
    - `malformed_episode_ids_are_still_refused`.

    The two callers still use the old guard and must switch. See `requests/server-profile-d30-forget-episode-callers.md`:
    - the actor, `euchre-profile`;
    - the route, `euchre-http`.
