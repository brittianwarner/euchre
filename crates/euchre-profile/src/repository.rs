//! The 05 §3.7 repository as SQL over `ctx.sql()`: the TS `repository.ts` and
//! `schema.ts` functions, one for one. Statements, bind order and row codecs
//! come from `euchre_profile_model::sql`; this module only runs them.
//!
//! Every write here runs under the actor lane (one writer at a time), so a
//! read-then-write such as [`upsert_match`] cannot interleave with another
//! write. Multi-row writes use `execute_batch`, which is atomic.

use std::borrow::Cow;

use anyhow::Result;
use euchre_profile_model::js::MAX_SAFE_INTEGER;
use euchre_profile_model::sql::{
    self, CLIENT_REPLAY_HANDS_SQL, CLIENT_REPLAY_MATCH_SQL, EPISODE_CANDIDATES_SQL,
    EPISODE_DELETE_SQL, EPISODE_EXISTS_SQL, EPISODE_INSERT_SQL, EPISODES_LIST_SQL,
    EPISODES_PRUNE_SQL, LEDGER_DDL, LEDGER_INSERT_SQL, LEDGER_SELECT_SQL, MATCH_EXISTING_SQL,
    MIGRATIONS, RecordMatchOutcome, SERVER_REPLAY_HANDS_SQL, SERVER_REPLAY_MATCH_SQL, UpsertPlan,
    col_text,
};
use euchre_profile_model::{
    ClientMatchReplay, Column, HandRow, MatchRow, ProfileError, ProfileStats, Row,
    ServerMatchReplay, StoredEpisode, build_page, client_replay_from_rows, decode_cursor,
    list_matches_query, rank_episodes, server_replay_from_rows, stats_from_row,
};
use euchre_protocol::{MatchListPage, TokenUsage};
use euchre_server_core::rivet::user_error;
use rivetkit::{BindParam, ColumnValue, QueryResult, SqliteBatchStatement, SqliteDb};
use serde_json::Value;

/// A [`ProfileError`] as the public `user/<code>` error TS throws (D21).
pub fn profile_error(error: ProfileError) -> anyhow::Error {
    user_error(error.code.as_str(), error.message)
}

/// A repository failure that keeps bad stored data apart from a failing
/// database, so a former queue can drop the first (TS logged and completed
/// the message) and return the second for a retry.
#[derive(Debug)]
pub enum RepoError {
    /// A stored row failed to decode. Deterministic: a retry fails the same way.
    Data(ProfileError),
    /// SQLite itself failed.
    Storage(anyhow::Error),
}

impl From<anyhow::Error> for RepoError {
    fn from(e: anyhow::Error) -> Self {
        RepoError::Storage(e)
    }
}

impl From<RepoError> for anyhow::Error {
    /// Data errors become the public `user/<code>` error, as [`profile_error`].
    fn from(e: RepoError) -> Self {
        match e {
            RepoError::Data(e) => profile_error(e),
            RepoError::Storage(e) => e,
        }
    }
}

/// A model value as a SQLite bind parameter.
pub fn bind(column: Column) -> BindParam {
    match column {
        Column::Null => BindParam::Null,
        Column::Integer(v) => BindParam::Integer(v),
        Column::Float(v) => BindParam::Float(v),
        Column::Text(v) => BindParam::Text(v),
        Column::Blob(v) => BindParam::Blob(v),
    }
}

/// Bind parameters for a list of model values.
pub fn binds(columns: impl IntoIterator<Item = Column>) -> Vec<BindParam> {
    columns.into_iter().map(bind).collect()
}

/// A SQLite result value as a model value.
pub fn column(value: ColumnValue) -> Column {
    match value {
        ColumnValue::Null => Column::Null,
        ColumnValue::Integer(v) => Column::Integer(v),
        ColumnValue::Float(v) => Column::Float(v),
        ColumnValue::Text(v) => Column::Text(v),
        ColumnValue::Blob(v) => Column::Blob(v),
    }
}

/// A query result as named model rows (TS `db.execute` result objects).
pub fn rows(result: QueryResult) -> Vec<Row> {
    let QueryResult { columns, rows } = result;
    rows.into_iter()
        .map(|values| {
            columns
                .iter()
                .cloned()
                .zip(values.into_iter().map(column))
                .collect()
        })
        .collect()
}

async fn select(db: &SqliteDb, sql: &str, params: Vec<Column>) -> Result<Vec<Row>> {
    let params = if params.is_empty() {
        None
    } else {
        Some(binds(params))
    };
    Ok(rows(db.query(sql, params).await?))
}

fn statement(sql: impl Into<String>, params: Vec<Column>) -> SqliteBatchStatement {
    SqliteBatchStatement {
        sql: sql.into(),
        params: Some(binds(params)),
    }
}

/// TS `applyMigrations(db)`: create the ledger, then run every migration the
/// ledger lacks, each with its ledger row in one atomic batch. Idempotent; run
/// on every start.
pub async fn apply_migrations(db: &SqliteDb, now: i64) -> Result<Vec<&'static str>> {
    db.exec(LEDGER_DDL).await?;
    let applied: Vec<String> = select(db, LEDGER_SELECT_SQL, Vec::new())
        .await?
        .iter()
        .filter_map(|r| col_text(r.get("id")).map(str::to_owned))
        .collect();
    let mut ran = Vec::new();
    for migration in MIGRATIONS {
        if applied.iter().any(|id| id == migration.id) {
            continue;
        }
        let mut batch: Vec<SqliteBatchStatement> = migration
            .statements
            .iter()
            .map(|sql| SqliteBatchStatement {
                sql: (*sql).to_owned(),
                params: None,
            })
            .collect();
        batch.push(statement(
            LEDGER_INSERT_SQL,
            vec![migration.id.into(), now.into()],
        ));
        db.execute_batch(batch).await?;
        ran.push(migration.id);
    }
    Ok(ran)
}

/// The largest token counter a match row stores: JS `Number.MAX_SAFE_INTEGER`.
pub const MAX_STORED_TOKENS: u64 = MAX_SAFE_INTEGER.unsigned_abs();

/// `record` with each token counter clamped to [`MAX_STORED_TOKENS`].
///
/// W3 follow-up. A record's counters parse to at most `i64::MAX`
/// (`value_to_int` saturates) and were stored as SQLite integers, so two
/// near-maximum matches made `STATS_SQL`'s integer `SUM(tokens_in)` overflow.
/// [`compute_stats`] runs on every `recordGame`, so every later record for the
/// profile failed and its table retried forever. TS bound such a count as a
/// float (a JS number past 2^53 is no integer), so it never overflowed. The
/// port clamps it to the largest integer a JS reader holds exactly, which a
/// real match never reaches, and sums with [`SATURATING_STATS_SQL`].
pub fn clamp_tokens(record: &MatchRow) -> Cow<'_, MatchRow> {
    let t = &record.tokens;
    let counters = [t.tokens_in, t.tokens_out, t.cache_read_tokens, t.calls];
    if counters.iter().all(|v| *v <= MAX_STORED_TOKENS) {
        return Cow::Borrowed(record);
    }
    let clamp = |v: u64| v.min(MAX_STORED_TOKENS);
    let mut clamped = record.clone();
    clamped.tokens = TokenUsage {
        tokens_in: clamp(t.tokens_in),
        tokens_out: clamp(t.tokens_out),
        cache_read_tokens: clamp(t.cache_read_tokens),
        calls: clamp(t.calls),
    };
    Cow::Owned(clamped)
}

/// TS `upsertMatch(db, record, now)`: insert a new match, advance a live one,
/// never touch a finished one. Token counters are stored clamped
/// ([`clamp_tokens`]).
pub async fn upsert_match(
    db: &SqliteDb,
    record: &MatchRow,
    now: i64,
) -> Result<RecordMatchOutcome, RepoError> {
    let record = clamp_tokens(record);
    let record = record.as_ref();
    let existing = select(
        db,
        MATCH_EXISTING_SQL,
        vec![record.match_id.as_str().into()],
    )
    .await?;
    let plan =
        sql::upsert_plan(existing.first().map(|r| r.get("ended_at"))).map_err(RepoError::Data)?;
    let outcome = |inserted, updated| RecordMatchOutcome {
        match_id: record.match_id.clone(),
        inserted,
        updated,
    };
    match plan {
        UpsertPlan::Skip => Ok(outcome(false, false)),
        UpsertPlan::Update => {
            db.run(
                sql::match_update_sql(),
                Some(binds(sql::match_update_params(record, now))),
            )
            .await?;
            Ok(outcome(false, true))
        }
        UpsertPlan::Insert => {
            let values = sql::match_columns(record, now).into_iter().map(|(_, v)| v);
            db.run(sql::match_insert_sql(), Some(binds(values))).await?;
            Ok(outcome(true, false))
        }
    }
}

/// TS `insertHands(db, hands)`: every hand in one atomic batch, existing
/// `(match_id, hand_no)` rows left alone. Returns the number attempted.
pub async fn insert_hands(db: &SqliteDb, hands: &[HandRow]) -> Result<usize> {
    if hands.is_empty() {
        return Ok(0);
    }
    let insert = sql::hand_insert_sql();
    let batch = hands
        .iter()
        .map(|h| {
            let values = sql::hand_columns(h).into_iter().map(|(_, v)| v).collect();
            statement(insert.as_str(), values)
        })
        .collect();
    db.execute_batch(batch).await?;
    Ok(hands.len())
}

/// TS `insertEpisodes(db, episodes)`: one atomic batch, duplicates ignored.
/// Returns the number attempted.
pub async fn insert_episodes(db: &SqliteDb, episodes: &[StoredEpisode]) -> Result<usize> {
    if episodes.is_empty() {
        return Ok(0);
    }
    let batch = episodes
        .iter()
        .map(|e| statement(EPISODE_INSERT_SQL, sql::episode_params(e)))
        .collect();
    db.execute_batch(batch).await?;
    Ok(episodes.len())
}

/// TS `listMatches(db, limit, cursor)`: one keyset page, newest first.
pub async fn list_matches(
    db: &SqliteDb,
    limit: i64,
    cursor: Option<&Value>,
) -> Result<MatchListPage> {
    let cursor = decode_cursor(cursor).map_err(profile_error)?;
    let (sql, params) = list_matches_query(limit, cursor.as_ref());
    let found = select(db, &sql, params).await?;
    build_page(&found, limit).map_err(profile_error)
}

/// TS `readClientReplay(db, matchId)`: the redacted replay, or `None`.
pub async fn read_client_replay(
    db: &SqliteDb,
    match_id: &str,
) -> Result<Option<ClientMatchReplay>> {
    let matches = select(db, CLIENT_REPLAY_MATCH_SQL, vec![match_id.into()]).await?;
    let Some(row) = matches.first() else {
        return Ok(None);
    };
    let hands = select(db, CLIENT_REPLAY_HANDS_SQL, vec![match_id.into()]).await?;
    client_replay_from_rows(row, &hands)
        .map(Some)
        .map_err(profile_error)
}

/// TS `readServerReplay(db, matchId)`: the full record and journal, or `None`.
pub async fn read_server_replay(
    db: &SqliteDb,
    match_id: &str,
) -> Result<Option<ServerMatchReplay>> {
    let matches = select(db, SERVER_REPLAY_MATCH_SQL, vec![match_id.into()]).await?;
    let Some(row) = matches.first() else {
        return Ok(None);
    };
    let hands = select(db, SERVER_REPLAY_HANDS_SQL, vec![match_id.into()]).await?;
    server_replay_from_rows(row, &hands)
        .map(Some)
        .map_err(profile_error)
}

/// `STATS_SQL` (the model's `computeStats` aggregate) with every
/// `COALESCE(SUM(x), 0)` as `MIN(TOTAL(x), 9007199254740991)`.
///
/// SQLite's integer `SUM` raises "integer overflow" past `i64::MAX`; `TOTAL`
/// sums as a float and never fails, and is exact while the total stays under
/// 2^53. Each total is capped at JS `Number.MAX_SAFE_INTEGER`. The aliases are
/// the model's, so `stats_from_row` reads it unchanged (it takes a float
/// column as its integer). A test keeps it in step with `STATS_SQL`.
pub const SATURATING_STATS_SQL: &str = "SELECT
\t\t\tCOUNT(*) AS matches,
\t\t\tMIN(TOTAL(CASE WHEN outcome = 'won' THEN 1 ELSE 0 END), 9007199254740991) AS won,
\t\t\tMIN(TOTAL(CASE WHEN outcome = 'lost' THEN 1 ELSE 0 END), 9007199254740991) AS lost,
\t\t\tMIN(TOTAL(CASE WHEN outcome = 'abandoned' THEN 1 ELSE 0 END), 9007199254740991) AS abandoned,
\t\t\tMIN(TOTAL(euchres_for), 9007199254740991) AS euchres_for,
\t\t\tMIN(TOTAL(euchres_against), 9007199254740991) AS euchres_against,
\t\t\tMIN(TOTAL(loners_attempted), 9007199254740991) AS loners_attempted,
\t\t\tMIN(TOTAL(loners_made), 9007199254740991) AS loners_made,
\t\t\tMIN(TOTAL(marches), 9007199254740991) AS marches,
\t\t\tMIN(TOTAL(throw_ins), 9007199254740991) AS throw_ins,
\t\t\tMIN(TOTAL(hands_played), 9007199254740991) AS hands_played,
\t\t\tMIN(TOTAL(tricks_us), 9007199254740991) AS tricks_us,
\t\t\tMIN(TOTAL(tricks_them), 9007199254740991) AS tricks_them,
\t\t\tMIN(TOTAL(tokens_in), 9007199254740991) AS tokens_in,
\t\t\tMIN(TOTAL(tokens_out), 9007199254740991) AS tokens_out,
\t\t\tMIN(TOTAL(cache_read_tokens), 9007199254740991) AS cache_read_tokens,
\t\t\tMIN(TOTAL(llm_calls), 9007199254740991) AS llm_calls
\t\t\tFROM matches";

/// TS `computeStats(db, now)`: lifetime totals over the whole history, each
/// saturating at JS `Number.MAX_SAFE_INTEGER` ([`SATURATING_STATS_SQL`]).
pub async fn compute_stats(db: &SqliteDb, now: i64) -> Result<ProfileStats, RepoError> {
    let found = select(db, SATURATING_STATS_SQL, Vec::new()).await?;
    stats_from_row(found.first(), now).map_err(RepoError::Data)
}

fn episodes_from_rows(found: &[Row]) -> Result<Vec<StoredEpisode>> {
    found
        .iter()
        .map(sql::episode_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(profile_error)
}

/// TS `selectEpisodesForRole(db, role, limit, now)`: live episodes for `role`,
/// most salient (half-life decayed) first. Ranks the newest 500 candidates in
/// Rust instead of SQLite `POWER()` (05 §3.7).
pub async fn select_episodes_for_role(
    db: &SqliteDb,
    role: &str,
    limit: i64,
    now: i64,
) -> Result<Vec<StoredEpisode>> {
    let found = select(db, EPISODE_CANDIDATES_SQL, vec![role.into(), now.into()]).await?;
    let candidates = episodes_from_rows(&found)?;
    let limit = usize::try_from(limit.max(0)).unwrap_or(0);
    Ok(rank_episodes(candidates, limit, now))
}

/// TS `listEpisodes(db, limit, now)`: live episodes, newest first.
pub async fn list_episodes(db: &SqliteDb, limit: i64, now: i64) -> Result<Vec<StoredEpisode>> {
    let found = select(db, EPISODES_LIST_SQL, vec![now.into(), limit.into()]).await?;
    episodes_from_rows(&found)
}

/// TS `deleteEpisode(db, episodeId)`: `true` when a row existed and is gone.
pub async fn delete_episode(db: &SqliteDb, episode_id: &str) -> Result<bool> {
    let before = select(db, EPISODE_EXISTS_SQL, vec![episode_id.into()]).await?;
    if before.is_empty() {
        return Ok(false);
    }
    db.run(EPISODE_DELETE_SQL, Some(binds([episode_id.into()])))
        .await?;
    Ok(true)
}

/// TS `pruneExpiredEpisodes(db, now)`: drop every episode past `expires_at`.
pub async fn prune_expired_episodes(db: &SqliteDb, now: i64) -> Result<()> {
    db.run(EPISODES_PRUNE_SQL, Some(binds([now.into()])))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use euchre_profile_model::sql::STATS_SQL;
    use euchre_profile_model::{MatchRow, parse_match_record};
    use serde_json::json;

    fn record_with_tokens(tokens: Value) -> MatchRow {
        let raw = json!({
            "userId": "u1",
            "matchId": "m1",
            "seed": "s",
            "startedAt": 1,
            "endedAt": 2,
            "status": "complete",
            "outcome": "won",
            "score": [10, 4],
            "winnerTeam": 0,
            "handsPlayed": 7,
            "personas": [],
            "stats": {},
            "tokens": tokens,
        });
        parse_match_record(Some(&raw), "u1", 3).unwrap_or_else(|e| panic!("record: {e:?}"))
    }

    #[test]
    fn the_saturating_stats_sql_is_stats_sql_with_capped_totals() {
        let derived = STATS_SQL
            .replace("COALESCE(SUM(", "MIN(TOTAL(")
            .replace("), 0) AS ", "), 9007199254740991) AS ");
        assert_eq!(SATURATING_STATS_SQL, derived);
        assert!(!SATURATING_STATS_SQL.contains("SUM("));
        assert_eq!(
            SATURATING_STATS_SQL.matches("TOTAL(").count(),
            STATS_SQL.matches("SUM(").count()
        );
        assert_eq!(
            i64::try_from(MAX_STORED_TOKENS).ok(),
            Some(MAX_SAFE_INTEGER)
        );
    }

    #[test]
    fn token_counters_are_stored_clamped_to_the_js_safe_range() {
        let normal = record_with_tokens(
            json!({ "tokensIn": 100, "tokensOut": 20, "cacheReadTokens": 5, "calls": 3 }),
        );
        assert!(matches!(clamp_tokens(&normal), Cow::Borrowed(_)));

        let huge = u64::MAX;
        let over = record_with_tokens(json!({
            "tokensIn": huge,
            "tokensOut": MAX_STORED_TOKENS + 1,
            "cacheReadTokens": MAX_STORED_TOKENS,
            "calls": 1e300,
        }));
        let clamped = clamp_tokens(&over);
        assert_eq!(
            clamped.tokens,
            TokenUsage {
                tokens_in: MAX_STORED_TOKENS,
                tokens_out: MAX_STORED_TOKENS,
                cache_read_tokens: MAX_STORED_TOKENS,
                calls: MAX_STORED_TOKENS,
            }
        );
        // Only the counters change.
        let mut expected = over.clone();
        expected.tokens = clamped.tokens;
        assert_eq!(clamped.into_owned(), expected);
    }

    #[test]
    fn float_totals_read_back_as_the_capped_integers() {
        // What `SATURATING_STATS_SQL` returns for a table whose sums passed 2^53.
        let cap = Column::Float(9_007_199_254_740_991.0);
        let row: Row = [
            ("matches", Column::Integer(2)),
            ("won", Column::Float(2.0)),
            ("tricks_us", Column::Float(9.0)),
            ("tricks_them", Column::Float(3.0)),
            ("tokens_in", cap.clone()),
            ("llm_calls", cap),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let stats = stats_from_row(Some(&row), 5).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!((stats.matches, stats.won, stats.tricks), (2, 2, [9, 3]));
        assert_eq!(stats.tokens.tokens_in, MAX_STORED_TOKENS);
        assert_eq!(stats.tokens.calls, MAX_STORED_TOKENS);
        assert_eq!(stats.tokens.tokens_out, 0);
    }

    #[test]
    fn values_round_trip_between_model_and_sqlite() {
        let cases = [
            Column::Null,
            Column::Integer(-7),
            Column::Float(0.25),
            Column::Text("é".into()),
            Column::Blob(vec![1, 2]),
        ];
        for c in cases {
            let back = match bind(c.clone()) {
                BindParam::Null => ColumnValue::Null,
                BindParam::Integer(v) => ColumnValue::Integer(v),
                BindParam::Float(v) => ColumnValue::Float(v),
                BindParam::Text(v) => ColumnValue::Text(v),
                BindParam::Blob(v) => ColumnValue::Blob(v),
            };
            assert_eq!(column(back), c);
        }
    }

    #[test]
    fn query_results_become_named_rows() {
        let result = QueryResult {
            columns: vec!["a".into(), "b".into()],
            rows: vec![
                vec![ColumnValue::Integer(1), ColumnValue::Text("x".into())],
                vec![ColumnValue::Null, ColumnValue::Float(1.5)],
            ],
        };
        let out = rows(result);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].get("a"), Some(&Column::Integer(1)));
        assert_eq!(out[0].get("b"), Some(&Column::Text("x".into())));
        assert_eq!(out[1].get("a"), Some(&Column::Null));
        assert_eq!(out[1].get("missing"), None);
    }

    #[test]
    fn repo_errors_keep_data_apart_from_storage() {
        let data: anyhow::Error = RepoError::Data(ProfileError::invalid_persona("bad row")).into();
        let parsed = euchre_server_core::rivet::parse_rivet_error(&data);
        let parsed = parsed.map(|p| (p.group, p.code, p.message));
        assert_eq!(
            parsed,
            Some((
                "user".to_owned(),
                "invalid_persona".to_owned(),
                "bad row".to_owned()
            ))
        );
        let storage: anyhow::Error = RepoError::from(anyhow::anyhow!("disk full")).into();
        assert_eq!(storage.to_string(), "disk full");
    }

    #[test]
    fn profile_errors_become_public_user_errors() {
        let e = profile_error(ProfileError::invalid_persona("name is too long"));
        let parsed = euchre_server_core::rivet::parse_rivet_error(&e);
        let parsed = parsed.map(|p| (p.group, p.code, p.message));
        assert_eq!(
            parsed,
            Some((
                "user".to_owned(),
                "invalid_persona".to_owned(),
                "name is too long".to_owned()
            ))
        );
    }
}
