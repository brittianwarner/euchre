//! History paging (`listMatches`): the keyset cursor, the page-size clamp, the
//! statement, and the page built from its rows.

use euchre_protocol::{MatchListPage, MatchSummary, ProtocolErrorCode};
use serde_json::Value;

use crate::error::{ProfileError, ProfileResult};
use crate::guards::{require_id, require_one_of_str};
use crate::js::{as_number, pick, trunc_i64, utf16_len};
use crate::records::{GAME_STATUSES, MATCH_OUTCOMES};
use crate::sql::{Column, Row, SUMMARY_COLUMNS, col_int, col_pair, col_string, col_string_array};

/// `MAX_LIST_LIMIT`.
pub const MAX_LIST_LIMIT: i64 = 50;
/// `DEFAULT_LIST_LIMIT`.
pub const DEFAULT_LIST_LIMIT: i64 = 20;
/// Longest cursor accepted, in UTF-16 units.
pub const MAX_CURSOR_UNITS: usize = 160;

/// A decoded keyset cursor (`playedAt:matchId`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DecodedCursor {
    /// The last row's `played_at`.
    pub played_at: i64,
    /// The last row's `match_id`, the tiebreak.
    pub match_id: String,
}

/// `encodeCursor(playedAt, matchId)`: `<playedAt>:<matchId>`.
pub fn encode_cursor(played_at: i64, match_id: &str) -> String {
    format!("{played_at}:{match_id}")
}

/// `decodeCursor(raw)`: `None` for null/absent/`''`; otherwise `internal_error`
/// "cursor is not a string", "cursor is malformed", "cursor timestamp is
/// malformed", or the `requireId` message for field `cursor`.
pub fn decode_cursor(raw: Option<&Value>) -> ProfileResult<Option<DecodedCursor>> {
    let text = match raw {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) if s.is_empty() => return Ok(None),
        Some(Value::String(s)) if utf16_len(s) <= MAX_CURSOR_UNITS => s,
        Some(_) => return Err(ProfileError::internal("cursor is not a string")),
    };
    let separator = match text.find(':') {
        Some(i) if i > 0 => i,
        _ => return Err(ProfileError::internal("cursor is malformed")),
    };
    let played = &text[..separator];
    if !(1..=15).contains(&played.len()) || !played.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ProfileError::internal("cursor timestamp is malformed"));
    }
    let played_at = played
        .parse::<i64>()
        .map_err(|_| ProfileError::internal("cursor timestamp is malformed"))?;
    let rest = Value::String(text[separator + 1..].to_owned());
    let match_id = require_id(Some(&rest), "cursor")?;
    Ok(Some(DecodedCursor {
        played_at,
        match_id,
    }))
}

/// `clampLimit(raw)`: a finite number truncated into `1..=50`, else `20`.
pub fn clamp_limit(raw: Option<&Value>) -> i64 {
    match as_number(raw) {
        Some(v) if v.is_finite() => trunc_i64(v).clamp(1, MAX_LIST_LIMIT),
        _ => DEFAULT_LIST_LIMIT,
    }
}

/// `listGames(req)` arguments: `clampLimit(req?.limit)` and `req?.cursor ?? null`.
pub fn list_games_args(req: Option<&Value>) -> (i64, Option<&Value>) {
    (clamp_limit(pick(req, "limit")), pick(req, "cursor"))
}

/// The `listMatches` statement and its bound values. Fetches `limit + 1` rows
/// so [`build_page`] can tell whether a next page exists.
pub fn list_matches_query(limit: i64, cursor: Option<&DecodedCursor>) -> (String, Vec<Column>) {
    match cursor {
        Some(c) => (
            format!(
                "SELECT {SUMMARY_COLUMNS} FROM matches\n\t\t\t\t\tWHERE played_at < ? OR (played_at = ? AND match_id < ?)\n\t\t\t\t\tORDER BY played_at DESC, match_id DESC\n\t\t\t\t\tLIMIT ?"
            ),
            vec![
                Column::Integer(c.played_at),
                Column::Integer(c.played_at),
                Column::Text(c.match_id.clone()),
                Column::Integer(limit + 1),
            ],
        ),
        None => (
            format!(
                "SELECT {SUMMARY_COLUMNS} FROM matches\n\t\t\t\t\tORDER BY played_at DESC, match_id DESC\n\t\t\t\t\tLIMIT ?"
            ),
            vec![Column::Integer(limit + 1)],
        ),
    }
}

/// `rowToSummary(row)`.
///
/// Protocol `MatchSummary.score` is `[u8; 2]`; a stored score outside `0..=255`
/// saturates (TS returns it as is).
pub fn summary_from_row(row: &Row) -> ProfileResult<MatchSummary> {
    let match_id = col_string(row.get("match_id"));
    let played_at = col_int(row.get("played_at"), 0)?;
    let status = require_one_of_str(
        &col_string(row.get("status")),
        &GAME_STATUSES,
        "status",
        ProtocolErrorCode::InternalError,
    )?;
    let outcome = require_one_of_str(
        &col_string(row.get("outcome")),
        &MATCH_OUTCOMES,
        "outcome",
        ProtocolErrorCode::InternalError,
    )?;
    let score = col_pair(row.get("score_us"), row.get("score_them"))?;
    let hands_played = col_int(row.get("hands_played"), 0)?;
    Ok(MatchSummary {
        match_id,
        played_at: u64::try_from(played_at).unwrap_or(0),
        status,
        outcome,
        score: [sat_u8(score[0]), sat_u8(score[1])],
        hands_played: u32::try_from(hands_played.max(0)).unwrap_or(u32::MAX),
        opponents: col_string_array(row.get("opponents_json")),
    })
}

fn sat_u8(v: i64) -> u8 {
    u8::try_from(v.clamp(0, 255)).unwrap_or(u8::MAX)
}

/// The page `listMatches` returns from the `limit + 1` fetched rows: the first
/// `limit` summaries, and a cursor at the last of them when more rows exist.
pub fn build_page(rows: &[Row], limit: i64) -> ProfileResult<MatchListPage> {
    let take = usize::try_from(limit.max(0)).unwrap_or(0);
    let page: Vec<MatchSummary> = rows
        .iter()
        .take(take)
        .map(summary_from_row)
        .collect::<ProfileResult<_>>()?;
    let cursor = match page.last() {
        Some(last) if rows.len() > take => Some(encode_cursor(
            i64::try_from(last.played_at).unwrap_or(i64::MAX),
            &last.match_id,
        )),
        _ => None,
    };
    Ok(MatchListPage { rows: page, cursor })
}

/// Whether a row sorts strictly after `cursor` in the history order
/// (`played_at DESC, match_id DESC`): the keyset predicate, for in-memory use.
pub fn is_after_cursor(played_at: i64, match_id: &str, cursor: &DecodedCursor) -> bool {
    played_at < cursor.played_at
        || (played_at == cursor.played_at && match_id < cursor.match_id.as_str())
}
