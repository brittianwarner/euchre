//! Lifetime statistics (`computeStats`): one aggregate over the history table,
//! never a counter kept beside it.

use euchre_protocol::{MatchOutcome, TokenUsage};
use serde::{Deserialize, Serialize};

use crate::error::ProfileResult;
use crate::records::MatchRow;
use crate::sql::{Row, col_int, col_pair};

/// `ProfileStats`: what `getStats` returns.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileStats {
    /// Matches recorded.
    pub matches: i64,
    /// Won.
    pub won: i64,
    /// Lost.
    pub lost: i64,
    /// Abandoned.
    pub abandoned: i64,
    /// `won / (won + lost)`, `0` with no decided match.
    #[serde(with = "euchre_protocol::serde_util::js_f64")]
    pub win_rate: f64,
    /// Euchres scored.
    pub euchres_for: i64,
    /// Times euchred.
    pub euchres_against: i64,
    /// Loners attempted.
    pub loners_attempted: i64,
    /// Loners made.
    pub loners_made: i64,
    /// Marches.
    pub marches: i64,
    /// Thrown-in hands.
    pub throw_ins: i64,
    /// Hands played (from `matches`, not the hand journal).
    pub hands_played: i64,
    /// Tricks `[us, them]`.
    pub tricks: [i64; 2],
    /// Model usage.
    pub tokens: TokenUsage,
    /// When this was computed.
    pub updated_at: i64,
}

fn win_rate(won: i64, lost: i64) -> f64 {
    let decided = won.saturating_add(lost);
    if decided == 0 {
        0.0
    } else {
        won as f64 / decided as f64
    }
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

/// Builds [`ProfileStats`] from the [`crate::sql::STATS_SQL`] result row (an
/// absent row reads as all zeros, as `rows[0] ?? {}` does).
pub fn stats_from_row(row: Option<&Row>, now: i64) -> ProfileResult<ProfileStats> {
    let empty = Row::default();
    let row = row.unwrap_or(&empty);
    let g = |k: &str| col_int(row.get(k), 0);
    let won = g("won")?;
    let lost = g("lost")?;
    Ok(ProfileStats {
        matches: g("matches")?,
        won,
        lost,
        abandoned: g("abandoned")?,
        win_rate: win_rate(won, lost),
        euchres_for: g("euchres_for")?,
        euchres_against: g("euchres_against")?,
        loners_attempted: g("loners_attempted")?,
        loners_made: g("loners_made")?,
        marches: g("marches")?,
        throw_ins: g("throw_ins")?,
        hands_played: g("hands_played")?,
        tricks: col_pair(row.get("tricks_us"), row.get("tricks_them"))?,
        tokens: TokenUsage {
            tokens_in: to_u64(g("tokens_in")?),
            tokens_out: to_u64(g("tokens_out")?),
            cache_read_tokens: to_u64(g("cache_read_tokens")?),
            calls: to_u64(g("llm_calls")?),
        },
        updated_at: now,
    })
}

/// The same aggregate computed in memory over validated rows: what
/// [`crate::sql::STATS_SQL`] returns for a table holding exactly `rows`.
pub fn compute_stats(rows: &[MatchRow], now: i64) -> ProfileStats {
    let mut s = ProfileStats {
        matches: 0,
        won: 0,
        lost: 0,
        abandoned: 0,
        win_rate: 0.0,
        euchres_for: 0,
        euchres_against: 0,
        loners_attempted: 0,
        loners_made: 0,
        marches: 0,
        throw_ins: 0,
        hands_played: 0,
        tricks: [0, 0],
        tokens: TokenUsage::default(),
        updated_at: now,
    };
    for r in rows {
        s.matches += 1;
        match r.outcome {
            MatchOutcome::Won => s.won += 1,
            MatchOutcome::Lost => s.lost += 1,
            MatchOutcome::Abandoned => s.abandoned += 1,
        }
        s.euchres_for += i64::from(r.stats.euchres_for);
        s.euchres_against += i64::from(r.stats.euchres_against);
        s.loners_attempted += i64::from(r.stats.loners_attempted);
        s.loners_made += i64::from(r.stats.loners_made);
        s.marches += i64::from(r.stats.marches);
        s.throw_ins += i64::from(r.stats.throw_ins);
        s.hands_played = s.hands_played.saturating_add(r.hands_played);
        s.tricks[0] += i64::from(r.stats.tricks[0]);
        s.tricks[1] += i64::from(r.stats.tricks[1]);
        s.tokens.tokens_in = s.tokens.tokens_in.saturating_add(r.tokens.tokens_in);
        s.tokens.tokens_out = s.tokens.tokens_out.saturating_add(r.tokens.tokens_out);
        s.tokens.cache_read_tokens = s
            .tokens
            .cache_read_tokens
            .saturating_add(r.tokens.cache_read_tokens);
        s.tokens.calls = s.tokens.calls.saturating_add(r.tokens.calls);
    }
    s.win_rate = win_rate(s.won, s.lost);
    s
}
