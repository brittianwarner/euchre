//! Inbound record validation for the `recordGame`, `recordHands`,
//! `recordEpisodes`, `setActiveGame` and `recordTokens` queue bodies
//! (`repository.ts` `parseMatchRecord` / `parseHands` / `parseEpisodes`, and
//! the `index.ts` handlers), plus the validated row types they produce.
//!
//! The rows keep the opaque parts of a record (`cfg`, `personas`, `moves`)
//! exactly as they arrived, as TS does, so [`MatchRow`] and [`HandRow`]
//! serialize with the `MatchRecord` / `HandJournalEntry` key order. Use
//! [`MatchRow::to_record`] / [`HandRow::to_entry`] for the typed protocol form.

use euchre_engine::DEFAULT_ENGINE_CONFIG;
use euchre_protocol::{
    EPISODE_SUMMARY_MAX_CHARS, EpisodeKind, GameStatus, HandJournalEntry, HandResult, MatchOutcome,
    MatchRecord, MatchStats, PERSONA_NAME_MAX_CHARS, ProtocolErrorCode, SchemaV1, Suit, TokenUsage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ProfileError, ProfileResult};
use crate::guards::{
    SanitizeOptions, as_array, require_id, require_int, require_one_of, sanitize_text,
    sanitize_value, value_to_float, value_to_int,
};
use crate::js::{
    JsJson, MAX_SAFE_INTEGER, as_number, is_nullish, or_default, pick, truncate_utf16, utf16_len,
};

/* -------------------------------------------------------------------------- */
/* Constants                                                                   */
/* -------------------------------------------------------------------------- */

/// `MAX_HANDS_PER_CALL`.
pub const MAX_HANDS_PER_CALL: usize = 64;
/// `MAX_EPISODES_PER_CALL`.
pub const MAX_EPISODES_PER_CALL: usize = 32;
/// The largest episode `handNo` (`requireInt(entry.handNo, ..., 0, 10_000)`).
pub const MAX_EPISODE_HAND_NO: i64 = 10_000;
/// `MAX_DECK_CARDS`.
pub const MAX_DECK_CARDS: usize = 24;
/// `MAX_MOVES_PER_HAND`.
pub const MAX_MOVES_PER_HAND: usize = 128;
/// `EPISODE_TTL_MS`: ninety days.
pub const EPISODE_TTL_MS: i64 = 90 * 24 * 60 * 60 * 1000;
/// Max UTF-16 length of a match seed.
pub const MAX_SEED_UNITS: usize = 256;

/// `GAME_STATUSES`, in TS order.
pub const GAME_STATUSES: [(GameStatus, &str); 3] = [
    (GameStatus::Active, "active"),
    (GameStatus::Complete, "complete"),
    (GameStatus::Abandoned, "abandoned"),
];
/// `MATCH_OUTCOMES`, in TS order.
pub const MATCH_OUTCOMES: [(MatchOutcome, &str); 3] = [
    (MatchOutcome::Won, "won"),
    (MatchOutcome::Lost, "lost"),
    (MatchOutcome::Abandoned, "abandoned"),
];
/// `HAND_RESULTS`, in TS order.
pub const HAND_RESULTS: [(HandResult, &str); 6] = [
    (HandResult::Point, "point"),
    (HandResult::March, "march"),
    (HandResult::LonePoint, "lone_point"),
    (HandResult::LoneMarch, "lone_march"),
    (HandResult::Euchre, "euchre"),
    (HandResult::ThrowIn, "throw_in"),
];
/// `SUITS`, in TS order.
pub const SUITS: [(Suit, &str); 4] = [
    (Suit::Spades, "S"),
    (Suit::Hearts, "H"),
    (Suit::Diamonds, "D"),
    (Suit::Clubs, "C"),
];
/// `EPISODE_KINDS`, in TS order.
pub const EPISODE_KINDS: [(EpisodeKind, &str); 5] = [
    (EpisodeKind::Bid, "bid"),
    (EpisodeKind::Play, "play"),
    (EpisodeKind::Loner, "loner"),
    (EpisodeKind::Euchre, "euchre"),
    (EpisodeKind::Read, "read"),
];

/// The queue names, as the actor declares them.
pub const QUEUE_NAMES: [&str; 5] = [
    "recordGame",
    "recordHands",
    "recordEpisodes",
    "setActiveGame",
    "recordTokens",
];

/// The warn line for a queue message whose internal token failed:
/// `dropped forged <queue>`.
pub fn forged_log_event(queue: &str) -> String {
    format!("dropped forged {queue}")
}

/// The error line a failed queue handler logs (fields `name`, `err`).
pub const QUEUE_HANDLER_FAILED_EVENT: &str = "playerProfile queue handler failed";

/// `PROFILE_KEY_NAMESPACE`.
pub const PROFILE_KEY_NAMESPACE: &str = "user";

/// `ownerOf(key)`: the user id of a `["user", userId]` key, else
/// `internal_error` "playerProfile must be keyed ['user', userId]".
pub fn owner_of(key: &[String]) -> ProfileResult<String> {
    match key {
        [namespace, user_id, ..] if namespace == PROFILE_KEY_NAMESPACE && !user_id.is_empty() => {
            Ok(user_id.clone())
        }
        _ => Err(ProfileError::internal(format!(
            "playerProfile must be keyed ['{PROFILE_KEY_NAMESPACE}', userId]"
        ))),
    }
}

/// `DEFAULT_ENGINE_CONFIG` as the JSON object TS stores for a record without a `cfg`.
pub fn default_engine_config_value() -> Value {
    serde_json::to_value(DEFAULT_ENGINE_CONFIG).unwrap_or(Value::Null)
}

/* -------------------------------------------------------------------------- */
/* Rows                                                                        */
/* -------------------------------------------------------------------------- */

/// A validated `MatchRecord`, opaque parts kept verbatim (TS `MatchRecord` as
/// `parseMatchRecord` and `readServerReplay` build it).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchRow {
    /// Always `1`.
    pub schema: SchemaV1,
    /// Primary key.
    pub match_id: String,
    /// The profile owner.
    pub user_id: String,
    /// The deal seed (server-only).
    pub seed: String,
    /// The rules, as stored.
    pub cfg: JsJson,
    /// The first dealer, `0..=3` when validated.
    pub first_dealer: i64,
    /// Epoch ms.
    pub started_at: i64,
    /// Epoch ms, or `null` while live.
    pub ended_at: Option<i64>,
    /// Match status.
    pub status: GameStatus,
    /// Human's outcome.
    pub outcome: MatchOutcome,
    /// `[us, them]`, `-1000..=1000` when validated.
    pub score: [i64; 2],
    /// `0 | 1 | null`.
    pub winner_team: Option<i64>,
    /// Hands completed.
    pub hands_played: i64,
    /// The opponents snapshot, at most three, as sent.
    pub personas: Vec<JsJson>,
    /// Counters.
    pub stats: MatchStats,
    /// Model usage.
    pub tokens: TokenUsage,
}

impl MatchRow {
    /// `endedAt ?? startedAt`, the history sort key.
    pub fn played_at(&self) -> i64 {
        self.ended_at.unwrap_or(self.started_at)
    }

    /// The typed protocol record, or `internal_error` when a stored part does
    /// not fit it (a non-standard `cfg`, a malformed persona, a score outside
    /// `0..=255`).
    pub fn to_record(&self) -> ProfileResult<MatchRecord> {
        let text = serde_json::to_string(self)
            .map_err(|_| ProfileError::internal("stored match is not a MatchRecord"))?;
        serde_json::from_str(&text)
            .map_err(|_| ProfileError::internal("stored match is not a MatchRecord"))
    }
}

/// A validated `HandJournalEntry` (`StoredHandRow`), moves kept verbatim.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandRow {
    /// The match.
    pub match_id: String,
    /// Hand number.
    pub hand_no: i64,
    /// The hand seed (server-only).
    pub seed: String,
    /// The dealer.
    pub dealer_seat: i64,
    /// The dealt deck (server-only).
    pub deck_order: Vec<String>,
    /// Every action, as sent.
    pub moves: Vec<JsJson>,
    /// Trump, if named.
    pub trump: Option<Suit>,
    /// The maker.
    pub maker_seat: Option<i64>,
    /// The lone player.
    pub alone_seat: Option<i64>,
    /// Tricks `[us, them]`.
    pub tricks_won: [i64; 2],
    /// The scoring row.
    pub result: Option<HandResult>,
    /// Points `[us, them]`.
    pub delta: Option<[i64; 2]>,
    /// Epoch ms.
    pub ended_at: i64,
}

impl HandRow {
    /// The typed protocol entry, or `internal_error` when a stored part does
    /// not fit it.
    pub fn to_entry(&self) -> ProfileResult<HandJournalEntry> {
        let text = serde_json::to_string(self)
            .map_err(|_| ProfileError::internal("stored hand is not a HandJournalEntry"))?;
        serde_json::from_str(&text)
            .map_err(|_| ProfileError::internal("stored hand is not a HandJournalEntry"))
    }
}

/// `StoredEpisode`: one persona memory as stored.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredEpisode {
    /// `<role>.<matchId>.<handNo>.<kind>`.
    pub episode_id: String,
    /// The persona id.
    pub role: String,
    /// The match.
    pub match_id: String,
    /// Hand number.
    pub hand_no: i64,
    /// Epoch ms.
    pub ts: i64,
    /// What it was about.
    pub kind: EpisodeKind,
    /// One sanitised line.
    pub summary: String,
    /// `0..=1`.
    #[serde(with = "euchre_protocol::serde_util::js_f64")]
    pub salience: f64,
    /// Epoch ms.
    pub expires_at: i64,
}

/* -------------------------------------------------------------------------- */
/* Small parsers                                                               */
/* -------------------------------------------------------------------------- */

fn parse_seat(raw: Option<&Value>, field: &str) -> ProfileResult<i64> {
    require_int(raw, field, 0, 3)
}

fn parse_seat_or_null(raw: Option<&Value>, field: &str) -> ProfileResult<Option<i64>> {
    if is_nullish(raw) {
        Ok(None)
    } else {
        parse_seat(raw, field).map(Some)
    }
}

fn parse_team_or_null(raw: Option<&Value>, field: &str) -> ProfileResult<Option<i64>> {
    if is_nullish(raw) {
        Ok(None)
    } else {
        require_int(raw, field, 0, 1).map(Some)
    }
}

fn parse_pair(raw: Option<&Value>, field: &str) -> ProfileResult<[i64; 2]> {
    let list = as_array(raw);
    if list.len() != 2 {
        return Err(ProfileError::internal(format!("{field} must be a pair")));
    }
    Ok([
        require_int(list.first(), &format!("{field}[0]"), -1000, 1000)?,
        require_int(list.get(1), &format!("{field}[1]"), -1000, 1000)?,
    ])
}

fn parse_pair_or_null(raw: Option<&Value>, field: &str) -> ProfileResult<Option<[i64; 2]>> {
    if is_nullish(raw) {
        Ok(None)
    } else {
        parse_pair(raw, field).map(Some)
    }
}

fn parse_timestamp(raw: Option<&Value>, field: &str, fallback: i64) -> ProfileResult<i64> {
    if is_nullish(raw) {
        Ok(fallback)
    } else {
        require_int(raw, field, 0, MAX_SAFE_INTEGER)
    }
}

fn parse_count(raw: Option<&Value>) -> ProfileResult<i64> {
    Ok(value_to_int(raw, 0)?.max(0))
}

fn to_u32(v: i64) -> u32 {
    u32::try_from(v.max(0)).unwrap_or(u32::MAX)
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

/// `parseStats`. Mirrors TS exactly, including that `tricks` is read with
/// `pick`, which only sees a plain object: an array `[a, b]` reads as `[0, 0]`.
fn parse_stats(raw: Option<&Value>) -> ProfileResult<MatchStats> {
    let tricks = pick(raw, "tricks");
    Ok(MatchStats {
        euchres_for: to_u32(parse_count(pick(raw, "euchresFor"))?),
        euchres_against: to_u32(parse_count(pick(raw, "euchresAgainst"))?),
        loners_attempted: to_u32(parse_count(pick(raw, "lonersAttempted"))?),
        loners_made: to_u32(parse_count(pick(raw, "lonersMade"))?),
        marches: to_u32(parse_count(pick(raw, "marches"))?),
        throw_ins: to_u32(parse_count(pick(raw, "throwIns"))?),
        tricks: [
            to_u32(parse_count(pick(tricks, "0"))?),
            to_u32(parse_count(pick(tricks, "1"))?),
        ],
    })
}

fn parse_tokens(raw: Option<&Value>) -> ProfileResult<TokenUsage> {
    Ok(TokenUsage {
        tokens_in: to_u64(parse_count(pick(raw, "tokensIn"))?),
        tokens_out: to_u64(parse_count(pick(raw, "tokensOut"))?),
        cache_read_tokens: to_u64(parse_count(pick(raw, "cacheReadTokens"))?),
        calls: to_u64(parse_count(pick(raw, "calls"))?),
    })
}

/// `opponentNames(personas)`: sanitised display names for seats 1, 2, 3, `''`
/// for a seat with no assignment. Entries whose `seat` is not a number or
/// whose `persona.name` is not a string are skipped; a later entry for the
/// same seat wins.
pub fn opponent_names(personas: &[JsJson]) -> Vec<String> {
    let mut by_seat: [Option<String>; 3] = [None, None, None];
    for assignment in personas {
        let a = Some(assignment.value());
        let Some(seat) = as_number(pick(a, "seat")) else {
            continue;
        };
        let Some(Value::String(name)) = pick(pick(a, "persona"), "name") else {
            continue;
        };
        let clean = sanitize_text(
            name,
            SanitizeOptions::new(false, PERSONA_NAME_MAX_CHARS, "persona.name")
                .with_code(ProtocolErrorCode::InternalError),
        );
        for (slot, n) in by_seat.iter_mut().zip([1.0, 2.0, 3.0]) {
            if seat == n {
                *slot = Some(clean.clone());
            }
        }
    }
    by_seat.into_iter().map(Option::unwrap_or_default).collect()
}

/* -------------------------------------------------------------------------- */
/* Inbound record validation                                                   */
/* -------------------------------------------------------------------------- */

/// `parseMatchRecord(raw, owner, now)`.
///
/// `owner` is the actor's own key ([`owner_of`]), never anything the message
/// said; a record whose `userId` disagrees is `forbidden`.
pub fn parse_match_record(raw: Option<&Value>, owner: &str, now: i64) -> ProfileResult<MatchRow> {
    let Some(Value::Object(map)) = raw else {
        return Err(ProfileError::internal("record must be an object"));
    };
    let get = |k: &str| map.get(k);

    match get("userId") {
        Some(Value::String(u)) if u == owner => {}
        _ => {
            return Err(ProfileError::forbidden(
                "record does not belong to this profile",
            ));
        }
    }

    let match_id = require_id(get("matchId"), "record.matchId")?;
    let seed = match get("seed") {
        Some(Value::String(s)) if (1..=MAX_SEED_UNITS).contains(&utf16_len(s)) => s.clone(),
        _ => {
            return Err(ProfileError::internal(
                "record.seed must be a bounded non-empty string",
            ));
        }
    };

    let started_at = parse_timestamp(get("startedAt"), "record.startedAt", now)?;
    let ended_at = if is_nullish(get("endedAt")) {
        None
    } else {
        Some(parse_timestamp(get("endedAt"), "record.endedAt", now)?)
    };

    let personas: Vec<JsJson> = as_array(get("personas"))
        .iter()
        .take(3)
        .cloned()
        .map(JsJson)
        .collect();

    let cfg = match get("cfg") {
        Some(v @ Value::Object(_)) => v.clone(),
        _ => default_engine_config_value(),
    };

    let zero = Value::from(0);
    let first_dealer = parse_seat(
        Some(or_default(get("firstDealer"), &zero)),
        "record.firstDealer",
    )?;
    let status = require_one_of(
        get("status"),
        &GAME_STATUSES,
        "record.status",
        ProtocolErrorCode::InternalError,
    )?;
    let outcome = require_one_of(
        get("outcome"),
        &MATCH_OUTCOMES,
        "record.outcome",
        ProtocolErrorCode::InternalError,
    )?;
    let score = parse_pair(get("score"), "record.score")?;
    let winner_team = parse_team_or_null(get("winnerTeam"), "record.winnerTeam")?;
    let hands_played = require_int(
        Some(or_default(get("handsPlayed"), &zero)),
        "record.handsPlayed",
        0,
        10_000,
    )?;
    let stats = parse_stats(get("stats"))?;
    let tokens = parse_tokens(get("tokens"))?;

    Ok(MatchRow {
        schema: SchemaV1,
        match_id,
        user_id: owner.to_owned(),
        seed,
        cfg: JsJson(cfg),
        first_dealer,
        started_at,
        ended_at,
        status,
        outcome,
        score,
        winner_team,
        hands_played,
        personas,
        stats,
        tokens,
    })
}

/// `parseHands(raw, matchId)`: a non-array is an empty batch; each entry is
/// checked in order and the first fault wins. A hand `seed` is cut to 256
/// UTF-16 units without splitting a surrogate pair.
pub fn parse_hands(raw: Option<&Value>, match_id: &str) -> ProfileResult<Vec<HandRow>> {
    let list = as_array(raw);
    if list.len() > MAX_HANDS_PER_CALL {
        return Err(ProfileError::internal(format!(
            "at most {MAX_HANDS_PER_CALL} hands per call"
        )));
    }
    let fallback_match = Value::String(match_id.to_owned());
    let zero = Value::from(0);
    let no_tricks = Value::Array(vec![Value::from(0), Value::from(0)]);

    let mut rows = Vec::with_capacity(list.len());
    for (index, entry) in list.iter().enumerate() {
        let Value::Object(map) = entry else {
            return Err(ProfileError::internal(format!(
                "hands[{index}] must be an object"
            )));
        };
        let get = |k: &str| map.get(k);
        let f = |name: &str| format!("hands[{index}].{name}");

        let entry_match = require_id(
            Some(or_default(get("matchId"), &fallback_match)),
            &f("matchId"),
        )?;
        if entry_match != match_id {
            return Err(ProfileError::internal(format!(
                "hands[{index}] belongs to a different match"
            )));
        }
        let deck = as_array(get("deckOrder"));
        if deck.len() > MAX_DECK_CARDS {
            return Err(ProfileError::internal(format!(
                "hands[{index}].deckOrder is too long"
            )));
        }
        let moves = as_array(get("moves"));
        if moves.len() > MAX_MOVES_PER_HAND {
            return Err(ProfileError::internal(format!(
                "hands[{index}].moves is too long"
            )));
        }

        let hand_no = require_int(get("handNo"), &f("handNo"), 0, 10_000)?;
        let seed = match get("seed") {
            Some(Value::String(s)) => truncate_utf16(s, MAX_SEED_UNITS).to_owned(),
            _ => String::new(),
        };
        let dealer_seat = parse_seat(Some(or_default(get("dealerSeat"), &zero)), &f("dealerSeat"))?;
        let deck_order: Vec<String> = deck
            .iter()
            .filter_map(|c| c.as_str().map(str::to_owned))
            .collect();
        let trump = if is_nullish(get("trump")) {
            None
        } else {
            Some(require_one_of(
                get("trump"),
                &SUITS,
                &f("trump"),
                ProtocolErrorCode::InternalError,
            )?)
        };
        let maker_seat = parse_seat_or_null(get("makerSeat"), &f("makerSeat"))?;
        let alone_seat = parse_seat_or_null(get("aloneSeat"), &f("aloneSeat"))?;
        let tricks_won = parse_pair(
            Some(or_default(get("tricksWon"), &no_tricks)),
            &f("tricksWon"),
        )?;
        let result = if is_nullish(get("result")) {
            None
        } else {
            Some(require_one_of(
                get("result"),
                &HAND_RESULTS,
                &f("result"),
                ProtocolErrorCode::InternalError,
            )?)
        };
        let delta = parse_pair_or_null(get("delta"), &f("delta"))?;
        let ended_at = parse_timestamp(get("endedAt"), &f("endedAt"), 0)?;

        rows.push(HandRow {
            match_id: entry_match,
            hand_no,
            seed,
            dealer_seat,
            deck_order,
            moves: moves.iter().cloned().map(JsJson).collect(),
            trump,
            maker_seat,
            alone_seat,
            tricks_won,
            result,
            delta,
            ended_at,
        });
    }
    Ok(rows)
}

/// `episodeId(role, matchId, handNo, kind)`: `<role>.<matchId>.<handNo>.<kind>`.
///
/// `forgetEpisode` takes this id back; [`crate::guards::require_episode_id`] is its
/// guard (D30).
pub fn episode_id(role: &str, match_id: &str, hand_no: i64, kind: EpisodeKind) -> String {
    format!("{role}.{match_id}.{hand_no}.{}", kind.as_str())
}

/// `parseEpisodes(raw, now)`: a non-array is an empty batch; the summary is
/// sanitised single-line to 140 code points; salience clamps to `0..=1`;
/// `expiresAt` defaults to `ts + 90 days`.
pub fn parse_episodes(raw: Option<&Value>, now: i64) -> ProfileResult<Vec<StoredEpisode>> {
    let list = as_array(raw);
    if list.len() > MAX_EPISODES_PER_CALL {
        return Err(ProfileError::internal(format!(
            "at most {MAX_EPISODES_PER_CALL} episodes per call"
        )));
    }
    let mut out = Vec::with_capacity(list.len());
    for (index, entry) in list.iter().enumerate() {
        let Value::Object(map) = entry else {
            return Err(ProfileError::internal(format!(
                "episodes[{index}] must be an object"
            )));
        };
        let get = |k: &str| map.get(k);
        let f = |name: &str| format!("episodes[{index}].{name}");

        let role = require_id(get("role"), &f("role"))?;
        let match_id = require_id(get("matchId"), &f("matchId"))?;
        let hand_no = require_int(get("handNo"), &f("handNo"), 0, MAX_EPISODE_HAND_NO)?;
        let kind = require_one_of(
            get("kind"),
            &EPISODE_KINDS,
            &f("kind"),
            ProtocolErrorCode::InternalError,
        )?;
        let ts = parse_timestamp(get("ts"), &f("ts"), now)?;
        let summary_field = f("summary");
        let summary = sanitize_value(
            get("summary"),
            SanitizeOptions::new(false, EPISODE_SUMMARY_MAX_CHARS, &summary_field)
                .with_code(ProtocolErrorCode::InternalError),
        )?;
        let salience = crate::guards::js_clamp(value_to_float(get("salience"), 0.0), 0.0, 1.0);
        let default_expiry = ts.saturating_add(EPISODE_TTL_MS);
        let expires_at = parse_timestamp(get("expiresAt"), &f("expiresAt"), default_expiry)?;

        out.push(StoredEpisode {
            episode_id: episode_id(&role, &match_id, hand_no, kind),
            role,
            match_id,
            hand_no,
            ts,
            kind,
            summary,
            salience,
            expires_at,
        });
    }
    Ok(out)
}

/* -------------------------------------------------------------------------- */
/* Queue bodies                                                                */
/* -------------------------------------------------------------------------- */

/// `body.internalToken` when it is a string (every queue body carries one).
pub fn body_internal_token(body: &Value) -> Option<&str> {
    pick(Some(body), "internalToken").and_then(Value::as_str)
}

/// A validated `recordGame` body: the record, and the hands when the body had
/// a `hands` key (`body.hands !== undefined`; `null` is an empty batch).
#[derive(Clone, PartialEq, Debug)]
pub struct RecordGame {
    /// The match row to upsert.
    pub record: MatchRow,
    /// The hands to journal, when sent.
    pub hands: Option<Vec<HandRow>>,
}

/// `onRecordGame` validation: `parseMatchRecord(body.record, owner, now)` then,
/// when present, `parseHands(body.hands, record.matchId)`.
pub fn parse_record_game(body: &Value, owner: &str, now: i64) -> ProfileResult<RecordGame> {
    let b = Some(body);
    let record = parse_match_record(pick(b, "record"), owner, now)?;
    let hands = match body.as_object().and_then(|m| m.get("hands")) {
        None => None,
        Some(h) => Some(parse_hands(Some(h), &record.match_id)?),
    };
    Ok(RecordGame { record, hands })
}

/// `onRecordHands` validation: `requireId(body.matchId, 'matchId')` then
/// `parseHands(body.hands, matchId)`.
pub fn parse_record_hands(body: &Value) -> ProfileResult<Vec<HandRow>> {
    let b = Some(body);
    let match_id = require_id(pick(b, "matchId"), "matchId")?;
    parse_hands(pick(b, "hands"), &match_id)
}

/// `onRecordEpisodes` validation: `parseEpisodes(body.episodes, now)`.
pub fn parse_record_episodes(body: &Value, now: i64) -> ProfileResult<Vec<StoredEpisode>> {
    parse_episodes(pick(Some(body), "episodes"), now)
}

/// `onSetActiveGame`: `body.gameId`. TS stores the value unvalidated; here a
/// non-string reads as `null`.
pub fn parse_set_active_game(body: &Value) -> Option<String> {
    pick(Some(body), "gameId")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// `nonNegativeInt(raw)`: a finite positive number truncates, else `0`.
pub fn non_negative_int(raw: Option<&Value>) -> u64 {
    match as_number(raw) {
        // `as` saturates at u64::MAX.
        Some(v) if v.is_finite() && v > 0.0 => v.trunc() as u64,
        _ => 0,
    }
}

/// A validated `recordTokens` body (each field via [`non_negative_int`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TokenIncrement {
    /// Input tokens.
    pub tokens_in: u64,
    /// Output tokens.
    pub tokens_out: u64,
    /// Cache-read tokens.
    pub cache_read_tokens: u64,
    /// Model calls.
    pub calls: u64,
}

impl TokenIncrement {
    /// Reads `tokensIn`, `tokensOut`, `cacheReadTokens` and `calls` from a body.
    pub fn from_body(body: &Value) -> TokenIncrement {
        let b = Some(body);
        TokenIncrement {
            tokens_in: non_negative_int(pick(b, "tokensIn")),
            tokens_out: non_negative_int(pick(b, "tokensOut")),
            cache_read_tokens: non_negative_int(pick(b, "cacheReadTokens")),
            calls: non_negative_int(pick(b, "calls")),
        }
    }
}
