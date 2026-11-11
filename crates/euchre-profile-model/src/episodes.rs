//! Episode ranking for `getEpisodesForRole` (`selectEpisodesForRole`).
//!
//! TS ranks in SQL: `ORDER BY salience * POWER(0.5, (? - ts) / ?) DESC, ts DESC`.
//! Every operand of `(now - ts) / halfLife` is bound or stored as an INTEGER,
//! so SQLite divides as integers (truncating toward zero) and the decay steps
//! once per half-life. Stock SQLite may lack `POWER()`, so the port selects
//! [`crate::sql::EPISODE_CANDIDATES_SQL`] (newest 500) and ranks here.

use crate::records::StoredEpisode;

/// The decay half-life: fourteen days.
pub const EPISODE_HALF_LIFE_MS: i64 = 14 * 24 * 60 * 60 * 1000;

/// How many newest live episodes are ranked (05 §3.7).
pub const EPISODE_RANK_CANDIDATES: usize = 500;

/// `salience * POWER(0.5, (now - ts) / halfLife)` with SQLite integer division.
pub fn episode_score(salience: f64, ts: i64, now: i64) -> f64 {
    let steps = now.saturating_sub(ts) / EPISODE_HALF_LIFE_MS;
    salience * 0.5f64.powf(steps as f64)
}

/// Ranks candidates most salient first (ties: newer first) and keeps `limit`.
///
/// The sort is stable, so rows tied on both keys keep their candidate order
/// (newest first, as [`crate::sql::EPISODE_CANDIDATES_SQL`] returns them).
pub fn rank_episodes(
    mut candidates: Vec<StoredEpisode>,
    limit: usize,
    now: i64,
) -> Vec<StoredEpisode> {
    candidates.sort_by(|a, b| {
        let sa = episode_score(a.salience, a.ts, now);
        let sb = episode_score(b.salience, b.ts, now);
        sb.total_cmp(&sa).then_with(|| b.ts.cmp(&a.ts))
    });
    candidates.truncate(limit);
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_steps_per_half_life() {
        let now = 10 * EPISODE_HALF_LIFE_MS;
        assert_eq!(episode_score(0.8, now, now), 0.8);
        assert_eq!(episode_score(0.8, now - EPISODE_HALF_LIFE_MS + 1, now), 0.8);
        assert_eq!(episode_score(0.8, now - EPISODE_HALF_LIFE_MS, now), 0.4);
        assert_eq!(episode_score(0.8, now + EPISODE_HALF_LIFE_MS, now), 1.6);
    }
}
