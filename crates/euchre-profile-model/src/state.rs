//! `ProfileActorState` and the small pure helpers of `player-profile/index.ts`:
//! `utcDay`, `emptyDailyTokens`, `emptyLifetime`, `lifetimeFromStats`, and the
//! `recordTokens` day-bucket update.

use euchre_protocol::{LifetimeStats, SchemaV1};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ProfileError, ProfileResult};
use crate::js::{as_number, pick};
use crate::records::TokenIncrement;
use crate::settings::{ProfileSettings, default_settings, normalize_settings};
use crate::stats::ProfileStats;

/// `DailyTokenUsage`: today's model usage, bucketed by UTC day.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyTokenUsage {
    /// Input tokens.
    pub tokens_in: u64,
    /// Output tokens.
    pub tokens_out: u64,
    /// Cache-read tokens.
    pub cache_read_tokens: u64,
    /// Model calls.
    pub calls: u64,
    /// `YYYY-MM-DD` (UTC).
    pub day: String,
}

/// `ProfileActorState`: the whole `c.state` of a `playerProfile`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileActorState {
    /// Always `1`.
    pub schema: SchemaV1,
    /// The live match, or `null`.
    pub active_game_id: Option<String>,
    /// Today's usage.
    pub daily_tokens: DailyTokenUsage,
    /// Lifetime totals, refreshed from the history table.
    pub lifetime: LifetimeStats,
    /// Settings.
    pub settings: ProfileSettings,
}

/// `utcDay(now)`: `new Date(now).toISOString().slice(0, 10)`.
///
/// Years outside `0..=9999` use the ISO extended form (`+010000`, `-000001`)
/// as JS does. `toISOString` throws past `±8.64e15`; this does not.
pub fn utc_day(now: i64) -> String {
    let days = now.div_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    let iso = if (0..=9999).contains(&y) {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        let sign = if y < 0 { '-' } else { '+' };
        format!("{sign}{:06}-{m:02}-{d:02}", y.unsigned_abs())
    };
    iso.chars().take(10).collect()
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 -> (y, m, d).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `emptyDailyTokens(now)`.
pub fn empty_daily_tokens(now: i64) -> DailyTokenUsage {
    DailyTokenUsage {
        tokens_in: 0,
        tokens_out: 0,
        cache_read_tokens: 0,
        calls: 0,
        day: utc_day(now),
    }
}

/// `emptyLifetime(now)`.
pub fn empty_lifetime(now: i64) -> LifetimeStats {
    LifetimeStats {
        updated_at: u64::try_from(now).unwrap_or(0),
        ..LifetimeStats::default()
    }
}

fn sat_u32(v: i64) -> u32 {
    u32::try_from(v.max(0)).unwrap_or(u32::MAX)
}

/// `lifetimeFromStats(stats)`: the cached subset of [`ProfileStats`].
pub fn lifetime_from_stats(stats: &ProfileStats) -> LifetimeStats {
    LifetimeStats {
        matches: sat_u32(stats.matches),
        won: sat_u32(stats.won),
        lost: sat_u32(stats.lost),
        abandoned: sat_u32(stats.abandoned),
        euchres_for: sat_u32(stats.euchres_for),
        euchres_against: sat_u32(stats.euchres_against),
        loners_made: sat_u32(stats.loners_made),
        hands_played: sat_u32(stats.hands_played),
        updated_at: u64::try_from(stats.updated_at).unwrap_or(0),
    }
}

/// `createState`: a brand-new profile's state.
pub fn initial_state(now: i64) -> ProfileActorState {
    ProfileActorState {
        schema: SchemaV1,
        active_game_id: None,
        daily_tokens: empty_daily_tokens(now),
        lifetime: empty_lifetime(now),
        settings: default_settings(now),
    }
}

impl DailyTokenUsage {
    /// `onRecordTokens`: reset the bucket when the UTC day changed, then add
    /// (saturating).
    pub fn record(&mut self, increment: TokenIncrement, now: i64) {
        let day = utc_day(now);
        if self.day != day {
            *self = empty_daily_tokens(now);
        }
        self.tokens_in = self.tokens_in.saturating_add(increment.tokens_in);
        self.tokens_out = self.tokens_out.saturating_add(increment.tokens_out);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(increment.cache_read_tokens);
        self.calls = self.calls.saturating_add(increment.calls);
    }
}

fn u64_field(value: Option<&Value>, fallback: u64) -> u64 {
    match as_number(value) {
        Some(v) if v.is_finite() && v >= 0.0 => v.trunc() as u64,
        _ => fallback,
    }
}

fn u32_field(value: Option<&Value>, fallback: u32) -> u32 {
    match as_number(value) {
        Some(v) if v.is_finite() && v >= 0.0 => v.trunc() as u32,
        _ => fallback,
    }
}

impl ProfileActorState {
    /// Loads state written by any earlier deployment, leniently (a Rust
    /// addition: TS trusts `c.state` and only repairs `settings`). Missing or
    /// malformed fields take their `createState` values; `settings` goes
    /// through [`normalize_settings`], exactly as the TS `run` does on wake.
    pub fn from_stored(raw: &Value, now: i64) -> ProfileActorState {
        let base = initial_state(now);
        let r = Some(raw);
        let daily = pick(r, "dailyTokens");
        let lifetime = pick(r, "lifetime");
        let day = match pick(daily, "day") {
            Some(Value::String(s)) => s.clone(),
            _ => base.daily_tokens.day.clone(),
        };
        let l = |k: &str, d: u32| u32_field(pick(lifetime, k), d);
        ProfileActorState {
            schema: SchemaV1,
            active_game_id: pick(r, "activeGameId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            daily_tokens: DailyTokenUsage {
                tokens_in: u64_field(pick(daily, "tokensIn"), 0),
                tokens_out: u64_field(pick(daily, "tokensOut"), 0),
                cache_read_tokens: u64_field(pick(daily, "cacheReadTokens"), 0),
                calls: u64_field(pick(daily, "calls"), 0),
                day,
            },
            lifetime: LifetimeStats {
                matches: l("matches", 0),
                won: l("won", 0),
                lost: l("lost", 0),
                abandoned: l("abandoned", 0),
                euchres_for: l("euchresFor", 0),
                euchres_against: l("euchresAgainst", 0),
                loners_made: l("lonersMade", 0),
                hands_played: l("handsPlayed", 0),
                updated_at: u64_field(pick(lifetime, "updatedAt"), base.lifetime.updated_at),
            },
            settings: match pick(r, "settings") {
                Some(s) => normalize_settings(s, now),
                None => base.settings,
            },
        }
    }
}

/// Whether a connection may call a browser-facing action; `forbidden` "this
/// connection is not authenticated" when not (`requireAuthenticated`).
pub fn require_authenticated(authenticated: bool) -> ProfileResult<()> {
    if authenticated {
        Ok(())
    } else {
        Err(ProfileError::forbidden(
            "this connection is not authenticated",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_day_matches_iso() {
        assert_eq!(utc_day(0), "1970-01-01");
        assert_eq!(utc_day(1_762_819_200_000), "2025-11-11");
        assert_eq!(utc_day(1_762_819_199_999), "2025-11-10");
        assert_eq!(utc_day(-1), "1969-12-31");
        assert_eq!(utc_day(951_782_400_000), "2000-02-29");
        assert_eq!(utc_day(253_402_300_800_000), "+010000-01");
    }

    #[test]
    fn tokens_reset_on_new_day() {
        let mut d = empty_daily_tokens(0);
        d.record(
            TokenIncrement {
                tokens_in: 5,
                ..TokenIncrement::default()
            },
            1000,
        );
        assert_eq!(d.tokens_in, 5);
        d.record(
            TokenIncrement {
                calls: 1,
                ..TokenIncrement::default()
            },
            86_400_000,
        );
        assert_eq!((d.tokens_in, d.calls, d.day.as_str()), (0, 1, "1970-01-02"));
    }
}
