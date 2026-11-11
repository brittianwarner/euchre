//! The `playerProfile` SQLite surface, driver-neutral: the schema, every
//! statement, the bound parameters for each write, and the decoders that turn
//! result rows back into records (`schema.ts`, `repository.ts`).
//!
//! Rows are [`Row`]s of [`Column`]s so this crate needs no SQLite driver; the
//! actor lane converts its driver's values at the edge. Every value is bound,
//! never interpolated; the only strings ever joined into a statement are the
//! column-name literals here.

use euchre_protocol::serde_util::js_number_string;
use serde::{Serialize, Serializer};

use crate::error::ProfileResult;
use crate::guards::{decode_json, encode_json, non_integer};
use crate::js::{js_string_to_number, js_trim, trunc_i64};
use crate::records::{HandRow, MatchRow, StoredEpisode, opponent_names};

/* -------------------------------------------------------------------------- */
/* Values and rows                                                             */
/* -------------------------------------------------------------------------- */

/// One SQLite value.
#[derive(Clone, PartialEq, Debug)]
pub enum Column {
    /// `NULL`.
    Null,
    /// `INTEGER`.
    Integer(i64),
    /// `REAL`.
    Float(f64),
    /// `TEXT`.
    Text(String),
    /// `BLOB`.
    Blob(Vec<u8>),
}

impl From<i64> for Column {
    fn from(v: i64) -> Column {
        Column::Integer(v)
    }
}

impl From<Option<i64>> for Column {
    fn from(v: Option<i64>) -> Column {
        v.map_or(Column::Null, Column::Integer)
    }
}

impl From<u32> for Column {
    fn from(v: u32) -> Column {
        Column::Integer(i64::from(v))
    }
}

impl From<u64> for Column {
    fn from(v: u64) -> Column {
        Column::Integer(i64::try_from(v).unwrap_or(i64::MAX))
    }
}

impl From<f64> for Column {
    fn from(v: f64) -> Column {
        Column::Float(v)
    }
}

impl From<String> for Column {
    fn from(v: String) -> Column {
        Column::Text(v)
    }
}

impl From<&str> for Column {
    fn from(v: &str) -> Column {
        Column::Text(v.to_owned())
    }
}

impl Serialize for Column {
    /// JSON form: numbers print as JS numbers, a blob as an array of bytes.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Column::Null => serializer.serialize_unit(),
            Column::Integer(i) => serializer.serialize_i64(*i),
            Column::Float(f) => euchre_protocol::serde_util::js_f64::serialize(f, serializer),
            Column::Text(s) => serializer.serialize_str(s),
            Column::Blob(b) => b.serialize(serializer),
        }
    }
}

/// One result row: column names with values, in select order.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Row(pub Vec<(String, Column)>);

impl Row {
    /// The value of column `name`, or `None` when the row has no such column.
    pub fn get(&self, name: &str) -> Option<&Column> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

impl FromIterator<(String, Column)> for Row {
    fn from_iter<I: IntoIterator<Item = (String, Column)>>(iter: I) -> Row {
        Row(iter.into_iter().collect())
    }
}

/* -------------------------------------------------------------------------- */
/* Column readers (`toInt`, `toFloat`, `toStringColumn`, ...)                  */
/* -------------------------------------------------------------------------- */

/// `toInt(value, fallback)` on a column: an integer, a truncated finite real, a
/// numeric non-blank text; `NULL` or a missing column is `fallback`; anything
/// else is `internal_error` "unexpected non-integer column value".
pub fn col_int(value: Option<&Column>, fallback: i64) -> ProfileResult<i64> {
    match value {
        Some(Column::Integer(i)) => Ok(*i),
        Some(Column::Float(f)) if f.is_finite() => Ok(trunc_i64(*f)),
        Some(Column::Text(s)) if !js_trim(s).is_empty() => {
            let n = js_string_to_number(s);
            if n.is_finite() {
                Ok(trunc_i64(n))
            } else {
                Err(non_integer())
            }
        }
        None | Some(Column::Null) => Ok(fallback),
        _ => Err(non_integer()),
    }
}

/// `toIntOrNull(value)`.
pub fn col_int_or_null(value: Option<&Column>) -> ProfileResult<Option<i64>> {
    match value {
        None | Some(Column::Null) => Ok(None),
        v => col_int(v, 0).map(Some),
    }
}

/// `toFloat(value, fallback)`; never fails.
pub fn col_float(value: Option<&Column>, fallback: f64) -> f64 {
    match value {
        Some(Column::Integer(i)) => *i as f64,
        Some(Column::Float(f)) if f.is_finite() => *f,
        Some(Column::Text(s)) if !js_trim(s).is_empty() => {
            let n = js_string_to_number(s);
            if n.is_finite() { n } else { fallback }
        }
        _ => fallback,
    }
}

/// `toStringColumn(value, '')`: text as is, a number as `String(n)`, else `''`.
pub fn col_string(value: Option<&Column>) -> String {
    match value {
        Some(Column::Text(s)) => s.clone(),
        Some(Column::Integer(i)) => i.to_string(),
        Some(Column::Float(f)) => js_number_string(*f),
        _ => String::new(),
    }
}

/// `toStringOrNull(value)`.
pub fn col_string_or_null(value: Option<&Column>) -> Option<String> {
    match value {
        None | Some(Column::Null) => None,
        v => Some(col_string(v)),
    }
}

/// `pair(a, b)`.
pub fn col_pair(a: Option<&Column>, b: Option<&Column>) -> ProfileResult<[i64; 2]> {
    Ok([col_int(a, 0)?, col_int(b, 0)?])
}

/// The text of a JSON column, for `decodeJson`.
pub fn col_text(value: Option<&Column>) -> Option<&str> {
    match value {
        Some(Column::Text(s)) => Some(s),
        _ => None,
    }
}

/* -------------------------------------------------------------------------- */
/* Schema                                                                      */
/* -------------------------------------------------------------------------- */

/// `LEDGER_TABLE`.
pub const LEDGER_TABLE: &str = "_profile_migrations";

/// The ledger DDL, run before any migration.
pub const LEDGER_DDL: &str = "CREATE TABLE IF NOT EXISTS _profile_migrations (
\t\t\tid TEXT PRIMARY KEY,
\t\t\tapplied_at INTEGER NOT NULL
\t\t)";

/// `SELECT id FROM _profile_migrations`.
pub const LEDGER_SELECT_SQL: &str = "SELECT id FROM _profile_migrations";

/// Records an applied migration; binds `(id, applied_at)`.
pub const LEDGER_INSERT_SQL: &str = "INSERT INTO _profile_migrations (id, applied_at) VALUES (?, ?)
\t\t\t\tON CONFLICT(id) DO NOTHING";

/// One ordered migration.
#[derive(Clone, Copy, Debug)]
pub struct Migration {
    /// Ledger id.
    pub id: &'static str,
    /// Statements, run in order.
    pub statements: &'static [&'static str],
}

/// `MIGRATIONS`, in order.
pub const MIGRATIONS: &[Migration] = &[Migration {
    id: "0001_initial",
    statements: &[
        "CREATE TABLE IF NOT EXISTS matches (
\t\t\t\tmatch_id TEXT PRIMARY KEY,
\t\t\t\tschema INTEGER NOT NULL,
\t\t\t\tuser_id TEXT NOT NULL,
\t\t\t\tseed TEXT NOT NULL,
\t\t\t\tcfg_json TEXT NOT NULL,
\t\t\t\tfirst_dealer INTEGER NOT NULL,
\t\t\t\tstarted_at INTEGER NOT NULL,
\t\t\t\tended_at INTEGER,
\t\t\t\tplayed_at INTEGER NOT NULL,
\t\t\t\tstatus TEXT NOT NULL,
\t\t\t\toutcome TEXT NOT NULL,
\t\t\t\tscore_us INTEGER NOT NULL,
\t\t\t\tscore_them INTEGER NOT NULL,
\t\t\t\twinner_team INTEGER,
\t\t\t\thands_played INTEGER NOT NULL,
\t\t\t\tpersonas_json TEXT NOT NULL,
\t\t\t\topponents_json TEXT NOT NULL,
\t\t\t\teuchres_for INTEGER NOT NULL DEFAULT 0,
\t\t\t\teuchres_against INTEGER NOT NULL DEFAULT 0,
\t\t\t\tloners_attempted INTEGER NOT NULL DEFAULT 0,
\t\t\t\tloners_made INTEGER NOT NULL DEFAULT 0,
\t\t\t\tmarches INTEGER NOT NULL DEFAULT 0,
\t\t\t\tthrow_ins INTEGER NOT NULL DEFAULT 0,
\t\t\t\ttricks_us INTEGER NOT NULL DEFAULT 0,
\t\t\t\ttricks_them INTEGER NOT NULL DEFAULT 0,
\t\t\t\ttokens_in INTEGER NOT NULL DEFAULT 0,
\t\t\t\ttokens_out INTEGER NOT NULL DEFAULT 0,
\t\t\t\tcache_read_tokens INTEGER NOT NULL DEFAULT 0,
\t\t\t\tllm_calls INTEGER NOT NULL DEFAULT 0,
\t\t\t\trecorded_at INTEGER NOT NULL,
\t\t\t\tupdated_at INTEGER NOT NULL
\t\t\t)",
        "CREATE INDEX IF NOT EXISTS idx_matches_history
\t\t\t\tON matches (played_at DESC, match_id DESC)",
        "CREATE INDEX IF NOT EXISTS idx_matches_outcome ON matches (outcome)",
        "CREATE TABLE IF NOT EXISTS hands (
\t\t\t\tmatch_id TEXT NOT NULL,
\t\t\t\thand_no INTEGER NOT NULL,
\t\t\t\tseed TEXT NOT NULL,
\t\t\t\tdealer_seat INTEGER NOT NULL,
\t\t\t\tdeck_order_json TEXT NOT NULL,
\t\t\t\tmoves_json TEXT NOT NULL,
\t\t\t\ttrump TEXT,
\t\t\t\tmaker_seat INTEGER,
\t\t\t\talone_seat INTEGER,
\t\t\t\ttricks_us INTEGER NOT NULL,
\t\t\t\ttricks_them INTEGER NOT NULL,
\t\t\t\tresult TEXT,
\t\t\t\tdelta_us INTEGER,
\t\t\t\tdelta_them INTEGER,
\t\t\t\tended_at INTEGER NOT NULL,
\t\t\t\tPRIMARY KEY (match_id, hand_no)
\t\t\t)",
        "CREATE TABLE IF NOT EXISTS episodes (
\t\t\t\tepisode_id TEXT PRIMARY KEY,
\t\t\t\trole TEXT NOT NULL,
\t\t\t\tmatch_id TEXT NOT NULL,
\t\t\t\thand_no INTEGER NOT NULL,
\t\t\t\tts INTEGER NOT NULL,
\t\t\t\tkind TEXT NOT NULL,
\t\t\t\tsummary TEXT NOT NULL,
\t\t\t\tsalience REAL NOT NULL,
\t\t\t\texpires_at INTEGER NOT NULL
\t\t\t)",
        "CREATE INDEX IF NOT EXISTS idx_episodes_role
\t\t\t\tON episodes (role, salience DESC, ts DESC)",
        "CREATE INDEX IF NOT EXISTS idx_episodes_expiry ON episodes (expires_at)",
    ],
}];

/// `MIGRATION_IDS`.
pub fn migration_ids() -> Vec<&'static str> {
    MIGRATIONS.iter().map(|m| m.id).collect()
}

/* -------------------------------------------------------------------------- */
/* matches writes                                                              */
/* -------------------------------------------------------------------------- */

/// `matchColumns` names, in TS order (31 columns).
pub const MATCH_COLUMN_NAMES: [&str; 31] = [
    "match_id",
    "schema",
    "user_id",
    "seed",
    "cfg_json",
    "first_dealer",
    "started_at",
    "ended_at",
    "played_at",
    "status",
    "outcome",
    "score_us",
    "score_them",
    "winner_team",
    "hands_played",
    "personas_json",
    "opponents_json",
    "euchres_for",
    "euchres_against",
    "loners_attempted",
    "loners_made",
    "marches",
    "throw_ins",
    "tricks_us",
    "tricks_them",
    "tokens_in",
    "tokens_out",
    "cache_read_tokens",
    "llm_calls",
    "recorded_at",
    "updated_at",
];

/// `matchColumns(record, now)`: the 31 bound values, in [`MATCH_COLUMN_NAMES`] order.
pub fn match_columns(record: &MatchRow, now: i64) -> Vec<(&'static str, Column)> {
    let personas: Vec<serde_json::Value> = record.personas.iter().map(|p| p.0.clone()).collect();
    let opponents: Vec<serde_json::Value> = opponent_names(&record.personas)
        .into_iter()
        .map(serde_json::Value::String)
        .collect();
    let s = &record.stats;
    let t = &record.tokens;
    let values: [Column; 31] = [
        record.match_id.as_str().into(),
        Column::Integer(1),
        record.user_id.as_str().into(),
        record.seed.as_str().into(),
        encode_json(Some(&record.cfg.0)).into(),
        record.first_dealer.into(),
        record.started_at.into(),
        record.ended_at.into(),
        record.played_at().into(),
        record.status.as_str().into(),
        record.outcome.as_str().into(),
        record.score[0].into(),
        record.score[1].into(),
        record.winner_team.into(),
        record.hands_played.into(),
        encode_json(Some(&serde_json::Value::Array(personas))).into(),
        encode_json(Some(&serde_json::Value::Array(opponents))).into(),
        s.euchres_for.into(),
        s.euchres_against.into(),
        s.loners_attempted.into(),
        s.loners_made.into(),
        s.marches.into(),
        s.throw_ins.into(),
        s.tricks[0].into(),
        s.tricks[1].into(),
        t.tokens_in.into(),
        t.tokens_out.into(),
        t.cache_read_tokens.into(),
        t.calls.into(),
        now.into(),
        now.into(),
    ];
    MATCH_COLUMN_NAMES.into_iter().zip(values).collect()
}

/// `SELECT ended_at FROM matches WHERE match_id = ? LIMIT 1`; binds `(matchId)`.
pub const MATCH_EXISTING_SQL: &str = "SELECT ended_at FROM matches WHERE match_id = ? LIMIT 1";

/// The insert for a new match; binds the 31 [`match_columns`] values.
pub fn match_insert_sql() -> String {
    let placeholders = vec!["?"; MATCH_COLUMN_NAMES.len()].join(", ");
    format!(
        "INSERT INTO matches ({}) VALUES ({placeholders})\n\t\t\t\tON CONFLICT(match_id) DO NOTHING",
        MATCH_COLUMN_NAMES.join(", ")
    )
}

fn is_update_column(name: &str) -> bool {
    name != "match_id" && name != "recorded_at"
}

/// The update for a still-live match (every column but `match_id` and
/// `recorded_at`); binds [`match_update_params`].
pub fn match_update_sql() -> String {
    let assignments: Vec<String> = MATCH_COLUMN_NAMES
        .iter()
        .filter(|n| is_update_column(n))
        .map(|n| format!("{n} = ?"))
        .collect();
    format!(
        "UPDATE matches SET {} WHERE match_id = ? AND ended_at IS NULL",
        assignments.join(", ")
    )
}

/// The bound values for [`match_update_sql`]: the 29 updatable columns, then `match_id`.
pub fn match_update_params(record: &MatchRow, now: i64) -> Vec<Column> {
    let mut params: Vec<Column> = match_columns(record, now)
        .into_iter()
        .filter(|(n, _)| is_update_column(n))
        .map(|(_, v)| v)
        .collect();
    params.push(record.match_id.as_str().into());
    params
}

/// What `upsertMatch` did (`RecordMatchOutcome`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordMatchOutcome {
    /// The match.
    pub match_id: String,
    /// The call created the row.
    pub inserted: bool,
    /// The call advanced a still-live row.
    pub updated: bool,
}

/// The `upsertMatch` decision, given the `ended_at` of an existing row
/// (`None` when there is no row).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UpsertPlan {
    /// No row: run [`match_insert_sql`].
    Insert,
    /// A live row: run [`match_update_sql`].
    Update,
    /// A finished row: immutable history, do nothing.
    Skip,
}

/// Decides the upsert from the `SELECT ended_at` result: `existing` is the
/// first row's `ended_at` column, `None` when no row came back.
pub fn upsert_plan(existing: Option<Option<&Column>>) -> ProfileResult<UpsertPlan> {
    match existing {
        None => Ok(UpsertPlan::Insert),
        Some(ended) => Ok(if col_int_or_null(ended)?.is_some() {
            UpsertPlan::Skip
        } else {
            UpsertPlan::Update
        }),
    }
}

/* -------------------------------------------------------------------------- */
/* hands writes                                                                */
/* -------------------------------------------------------------------------- */

/// `insertHands` column names, in TS order.
pub const HAND_COLUMN_NAMES: [&str; 15] = [
    "match_id",
    "hand_no",
    "seed",
    "dealer_seat",
    "deck_order_json",
    "moves_json",
    "trump",
    "maker_seat",
    "alone_seat",
    "tricks_us",
    "tricks_them",
    "result",
    "delta_us",
    "delta_them",
    "ended_at",
];

/// The insert for one hand; binds [`hand_columns`]. A hand is written once:
/// `ON CONFLICT(match_id, hand_no) DO NOTHING`.
pub fn hand_insert_sql() -> String {
    let placeholders = vec!["?"; HAND_COLUMN_NAMES.len()].join(", ");
    format!(
        "INSERT INTO hands ({}) VALUES ({placeholders})\n\t\t\t\t\tON CONFLICT(match_id, hand_no) DO NOTHING",
        HAND_COLUMN_NAMES.join(", ")
    )
}

/// The 15 bound values for one hand, in [`HAND_COLUMN_NAMES`] order.
pub fn hand_columns(hand: &HandRow) -> Vec<(&'static str, Column)> {
    let deck: Vec<serde_json::Value> = hand
        .deck_order
        .iter()
        .cloned()
        .map(serde_json::Value::String)
        .collect();
    let moves: Vec<serde_json::Value> = hand.moves.iter().map(|m| m.0.clone()).collect();
    let values: [Column; 15] = [
        hand.match_id.as_str().into(),
        hand.hand_no.into(),
        hand.seed.as_str().into(),
        hand.dealer_seat.into(),
        encode_json(Some(&serde_json::Value::Array(deck))).into(),
        encode_json(Some(&serde_json::Value::Array(moves))).into(),
        hand.trump.map_or(Column::Null, |s| s.as_str().into()),
        hand.maker_seat.into(),
        hand.alone_seat.into(),
        hand.tricks_won[0].into(),
        hand.tricks_won[1].into(),
        hand.result.map_or(Column::Null, |r| r.as_str().into()),
        hand.delta.map(|d| d[0]).into(),
        hand.delta.map(|d| d[1]).into(),
        hand.ended_at.into(),
    ];
    HAND_COLUMN_NAMES.into_iter().zip(values).collect()
}

/* -------------------------------------------------------------------------- */
/* episodes                                                                    */
/* -------------------------------------------------------------------------- */

/// The episode insert; binds [`episode_params`].
pub const EPISODE_INSERT_SQL: &str = "INSERT INTO episodes
\t\t\t\t\t(episode_id, role, match_id, hand_no, ts, kind, summary, salience, expires_at)
\t\t\t\t\tVALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
\t\t\t\t\tON CONFLICT(episode_id) DO NOTHING";

/// The nine bound values of [`EPISODE_INSERT_SQL`]. `salience` binds as `REAL`.
pub fn episode_params(episode: &StoredEpisode) -> Vec<Column> {
    vec![
        episode.episode_id.as_str().into(),
        episode.role.as_str().into(),
        episode.match_id.as_str().into(),
        episode.hand_no.into(),
        episode.ts.into(),
        episode.kind.as_str().into(),
        episode.summary.as_str().into(),
        episode.salience.into(),
        episode.expires_at.into(),
    ]
}

/// TS `selectEpisodesForRole` statement (needs SQLite `POWER()`); binds
/// `(role, now, now, EPISODE_HALF_LIFE_MS, limit)`. Kept for reference: the
/// port uses [`EPISODE_CANDIDATES_SQL`] plus `rank_episodes`.
pub const EPISODES_FOR_ROLE_POWER_SQL: &str = "SELECT * FROM episodes
\t\t\tWHERE role = ? AND expires_at > ?
\t\t\tORDER BY salience * POWER(0.5, (? - ts) / ?) DESC, ts DESC
\t\t\tLIMIT ?";

/// Ranking candidates for a role (05 §3.7 safe port); binds `(role, now)`.
pub const EPISODE_CANDIDATES_SQL: &str =
    "SELECT * FROM episodes WHERE role = ? AND expires_at > ? ORDER BY ts DESC LIMIT 500";

/// `listEpisodes`; binds `(now, limit)`.
pub const EPISODES_LIST_SQL: &str =
    "SELECT * FROM episodes WHERE expires_at > ? ORDER BY ts DESC LIMIT ?";

/// `deleteEpisode` probe; binds `(episodeId)`.
pub const EPISODE_EXISTS_SQL: &str = "SELECT episode_id FROM episodes WHERE episode_id = ? LIMIT 1";

/// `deleteEpisode`; binds `(episodeId)`.
pub const EPISODE_DELETE_SQL: &str = "DELETE FROM episodes WHERE episode_id = ?";

/// `pruneExpiredEpisodes`; binds `(now)`.
pub const EPISODES_PRUNE_SQL: &str = "DELETE FROM episodes WHERE expires_at <= ?";

/// `rowToEpisode(row)`.
pub fn episode_from_row(row: &Row) -> ProfileResult<StoredEpisode> {
    let episode_id = col_string(row.get("episode_id"));
    let role = col_string(row.get("role"));
    let match_id = col_string(row.get("match_id"));
    let hand_no = col_int(row.get("hand_no"), 0)?;
    let ts = col_int(row.get("ts"), 0)?;
    let kind = crate::guards::require_one_of_str(
        &col_string(row.get("kind")),
        &crate::records::EPISODE_KINDS,
        "episodes.kind",
        euchre_protocol::ProtocolErrorCode::InternalError,
    )?;
    Ok(StoredEpisode {
        episode_id,
        role,
        match_id,
        hand_no,
        ts,
        kind,
        summary: col_string(row.get("summary")),
        salience: col_float(row.get("salience"), 0.0),
        expires_at: col_int(row.get("expires_at"), 0)?,
    })
}

/* -------------------------------------------------------------------------- */
/* reads                                                                       */
/* -------------------------------------------------------------------------- */

/// `listMatches` columns.
pub const SUMMARY_COLUMNS: &str =
    "match_id, played_at, status, outcome, score_us, score_them, hands_played, opponents_json";

/// `readClientReplay` match select; binds `(matchId)`. Never reads `seed`.
pub const CLIENT_REPLAY_MATCH_SQL: &str =
    "SELECT match_id, started_at, ended_at, played_at, status, outcome, score_us, score_them,
\t\t\twinner_team, hands_played, opponents_json, euchres_for, euchres_against,
\t\t\tloners_attempted, loners_made, marches, throw_ins, tricks_us, tricks_them
\t\t\tFROM matches WHERE match_id = ? LIMIT 1";

/// `readClientReplay` hands select; binds `(matchId)`. Never reads `seed` or `deck_order_json`.
pub const CLIENT_REPLAY_HANDS_SQL: &str =
    "SELECT hand_no, dealer_seat, moves_json, trump, maker_seat, alone_seat,
\t\t\ttricks_us, tricks_them, result, delta_us, delta_them, ended_at
\t\t\tFROM hands WHERE match_id = ? ORDER BY hand_no ASC";

/// `readServerReplay` match select; binds `(matchId)`.
pub const SERVER_REPLAY_MATCH_SQL: &str = "SELECT * FROM matches WHERE match_id = ? LIMIT 1";

/// `readServerReplay` hands select; binds `(matchId)`.
pub const SERVER_REPLAY_HANDS_SQL: &str =
    "SELECT * FROM hands WHERE match_id = ? ORDER BY hand_no ASC";

/// `computeStats` aggregate; binds nothing.
pub const STATS_SQL: &str = "SELECT
\t\t\tCOUNT(*) AS matches,
\t\t\tCOALESCE(SUM(CASE WHEN outcome = 'won' THEN 1 ELSE 0 END), 0) AS won,
\t\t\tCOALESCE(SUM(CASE WHEN outcome = 'lost' THEN 1 ELSE 0 END), 0) AS lost,
\t\t\tCOALESCE(SUM(CASE WHEN outcome = 'abandoned' THEN 1 ELSE 0 END), 0) AS abandoned,
\t\t\tCOALESCE(SUM(euchres_for), 0) AS euchres_for,
\t\t\tCOALESCE(SUM(euchres_against), 0) AS euchres_against,
\t\t\tCOALESCE(SUM(loners_attempted), 0) AS loners_attempted,
\t\t\tCOALESCE(SUM(loners_made), 0) AS loners_made,
\t\t\tCOALESCE(SUM(marches), 0) AS marches,
\t\t\tCOALESCE(SUM(throw_ins), 0) AS throw_ins,
\t\t\tCOALESCE(SUM(hands_played), 0) AS hands_played,
\t\t\tCOALESCE(SUM(tricks_us), 0) AS tricks_us,
\t\t\tCOALESCE(SUM(tricks_them), 0) AS tricks_them,
\t\t\tCOALESCE(SUM(tokens_in), 0) AS tokens_in,
\t\t\tCOALESCE(SUM(tokens_out), 0) AS tokens_out,
\t\t\tCOALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens,
\t\t\tCOALESCE(SUM(llm_calls), 0) AS llm_calls
\t\t\tFROM matches";

/// Reads a JSON array column of strings (`opponents_json`, `deck_order_json`):
/// the strings of a stored array, `[]` for anything else.
pub(crate) fn col_string_array(value: Option<&Column>) -> Vec<String> {
    match decode_json(col_text(value)) {
        Some(serde_json::Value::Array(items)) => items
            .into_iter()
            .filter_map(|v| match v {
                serde_json::Value::String(s) => Some(s),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Reads a JSON array column of opaque values (`moves_json`, `personas_json`).
pub(crate) fn col_value_array(value: Option<&Column>) -> Vec<serde_json::Value> {
    match decode_json(col_text(value)) {
        Some(serde_json::Value::Array(items)) => items,
        _ => Vec::new(),
    }
}
