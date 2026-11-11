//! Replay: the redaction boundary of `playerProfile` (`readClientReplay`,
//! `readServerReplay`).
//!
//! [`ClientMatchReplay`] and [`ClientHandReplay`] have no field that can hold a
//! `seed` or a `deckOrder`, and every non-seat-0 `discard` is dropped. The
//! server replay returns the stored record verbatim and must sit behind the
//! internal token.

use euchre_protocol::{
    GameStatus, HandResult, MatchOutcome, MatchStats, ProtocolErrorCode, SchemaV1, Suit, TokenUsage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ProfileResult;
use crate::guards::{decode_json, require_one_of_str};
use crate::js::{JsJson, as_number, pick};
use crate::records::{
    GAME_STATUSES, HAND_RESULTS, HandRow, MATCH_OUTCOMES, MatchRow, SUITS,
    default_engine_config_value,
};
use crate::sql::{
    Row, col_int, col_int_or_null, col_pair, col_string, col_string_array, col_string_or_null,
    col_text, col_value_array,
};

/// `ClientHandReplay`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientHandReplay {
    /// Hand number.
    pub hand_no: i64,
    /// The dealer.
    pub dealer_seat: i64,
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
    /// Every client-visible action (see [`is_client_visible_move`]).
    pub moves: Vec<JsJson>,
}

/// `ClientMatchReplay`: what `getReplay` returns.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientMatchReplay {
    /// The match.
    pub match_id: String,
    /// Epoch ms.
    pub started_at: i64,
    /// Epoch ms, or `null`.
    pub ended_at: Option<i64>,
    /// `endedAt ?? startedAt`.
    pub played_at: i64,
    /// Match status.
    pub status: GameStatus,
    /// Human's outcome.
    pub outcome: MatchOutcome,
    /// `[us, them]`.
    pub score: [i64; 2],
    /// `0 | 1 | null`.
    pub winner_team: Option<i64>,
    /// Hands completed.
    pub hands_played: i64,
    /// Opponent names, seats 1..3.
    pub opponents: Vec<String>,
    /// Counters.
    pub stats: MatchStats,
    /// Hands, in order.
    pub hands: Vec<ClientHandReplay>,
}

/// `ServerMatchReplay`: the stored record and journal, secrets included.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ServerMatchReplay {
    /// The record (`MatchRecord` shape).
    #[serde(rename = "match")]
    pub match_record: MatchRow,
    /// The journal (`HandJournalEntry` shape).
    pub hands: Vec<HandRow>,
}

/// `isClientVisibleMove(action)`: everything except a `discard` by a seat
/// other than 0 (the dealer's buried card is never shown).
pub fn is_client_visible_move(action: &Value) -> bool {
    let a = Some(action);
    if pick(pick(a, "move"), "t") != Some(&Value::String("discard".to_owned())) {
        return true;
    }
    as_number(pick(a, "seat")) == Some(0.0)
}

/// The moves a client may see, in order.
pub fn redact_moves(moves: &[Value]) -> Vec<JsJson> {
    moves
        .iter()
        .filter(|m| is_client_visible_move(m))
        .cloned()
        .map(JsJson)
        .collect()
}

fn suit_or_null(row: &Row) -> ProfileResult<Option<Suit>> {
    match col_string_or_null(row.get("trump")) {
        None => Ok(None),
        Some(t) => require_one_of_str(&t, &SUITS, "hands.trump", ProtocolErrorCode::InternalError)
            .map(Some),
    }
}

fn result_or_null(row: &Row) -> ProfileResult<Option<HandResult>> {
    match col_string_or_null(row.get("result")) {
        None => Ok(None),
        Some(r) => require_one_of_str(
            &r,
            &HAND_RESULTS,
            "hands.result",
            ProtocolErrorCode::InternalError,
        )
        .map(Some),
    }
}

fn delta(row: &Row) -> ProfileResult<Option<[i64; 2]>> {
    let us = col_int_or_null(row.get("delta_us"))?;
    let them = col_int_or_null(row.get("delta_them"))?;
    Ok(match (us, them) {
        (Some(a), Some(b)) => Some([a, b]),
        _ => None,
    })
}

fn stats(row: &Row) -> ProfileResult<MatchStats> {
    let u = |k: &str| -> ProfileResult<u32> { Ok(sat_u32(col_int(row.get(k), 0)?)) };
    let tricks = col_pair(row.get("tricks_us"), row.get("tricks_them"))?;
    Ok(MatchStats {
        euchres_for: u("euchres_for")?,
        euchres_against: u("euchres_against")?,
        loners_attempted: u("loners_attempted")?,
        loners_made: u("loners_made")?,
        marches: u("marches")?,
        throw_ins: u("throw_ins")?,
        tricks: [sat_u32(tricks[0]), sat_u32(tricks[1])],
    })
}

fn sat_u32(v: i64) -> u32 {
    u32::try_from(v.max(0)).unwrap_or(u32::MAX)
}

fn sat_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn status_outcome(row: &Row) -> ProfileResult<(GameStatus, MatchOutcome)> {
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
    Ok((status, outcome))
}

/// One [`crate::sql::CLIENT_REPLAY_HANDS_SQL`] row as a [`ClientHandReplay`].
pub fn client_hand_from_row(hand: &Row) -> ProfileResult<ClientHandReplay> {
    let d = delta(hand)?;
    let moves = col_value_array(hand.get("moves_json"));
    Ok(ClientHandReplay {
        hand_no: col_int(hand.get("hand_no"), 0)?,
        dealer_seat: col_int(hand.get("dealer_seat"), 0)?,
        trump: suit_or_null(hand)?,
        maker_seat: col_int_or_null(hand.get("maker_seat"))?,
        alone_seat: col_int_or_null(hand.get("alone_seat"))?,
        tricks_won: col_pair(hand.get("tricks_us"), hand.get("tricks_them"))?,
        result: result_or_null(hand)?,
        delta: d,
        ended_at: col_int(hand.get("ended_at"), 0)?,
        moves: redact_moves(&moves),
    })
}

/// `readClientReplay`: the match row from [`crate::sql::CLIENT_REPLAY_MATCH_SQL`]
/// and its hands from [`crate::sql::CLIENT_REPLAY_HANDS_SQL`]. (The action
/// returns `null` when the match select is empty.) Hands are decoded first, as
/// in TS, so a bad hand row is the error reported.
pub fn client_replay_from_rows(row: &Row, hands: &[Row]) -> ProfileResult<ClientMatchReplay> {
    let hands = hands
        .iter()
        .map(client_hand_from_row)
        .collect::<ProfileResult<Vec<_>>>()?;
    let match_id = col_string(row.get("match_id"));
    let started_at = col_int(row.get("started_at"), 0)?;
    let ended_at = col_int_or_null(row.get("ended_at"))?;
    let played_at = col_int(row.get("played_at"), 0)?;
    let (status, outcome) = status_outcome(row)?;
    Ok(ClientMatchReplay {
        match_id,
        started_at,
        ended_at,
        played_at,
        status,
        outcome,
        score: col_pair(row.get("score_us"), row.get("score_them"))?,
        winner_team: col_int_or_null(row.get("winner_team"))?,
        hands_played: col_int(row.get("hands_played"), 0)?,
        opponents: col_string_array(row.get("opponents_json")),
        stats: stats(row)?,
        hands,
    })
}

/// `rowToHandJournal`: one `SELECT * FROM hands` row.
pub fn hand_row_from_row(hand: &Row) -> ProfileResult<HandRow> {
    let d = delta(hand)?;
    Ok(HandRow {
        match_id: col_string(hand.get("match_id")),
        hand_no: col_int(hand.get("hand_no"), 0)?,
        seed: col_string(hand.get("seed")),
        dealer_seat: col_int(hand.get("dealer_seat"), 0)?,
        deck_order: col_string_array(hand.get("deck_order_json")),
        moves: col_value_array(hand.get("moves_json"))
            .into_iter()
            .map(JsJson)
            .collect(),
        trump: suit_or_null(hand)?,
        maker_seat: col_int_or_null(hand.get("maker_seat"))?,
        alone_seat: col_int_or_null(hand.get("alone_seat"))?,
        tricks_won: col_pair(hand.get("tricks_us"), hand.get("tricks_them"))?,
        result: result_or_null(hand)?,
        delta: d,
        ended_at: col_int(hand.get("ended_at"), 0)?,
    })
}

/// One `SELECT * FROM matches` row as the stored record.
pub fn match_row_from_row(row: &Row) -> ProfileResult<MatchRow> {
    let cfg =
        decode_json(col_text(row.get("cfg_json"))).unwrap_or_else(default_engine_config_value);
    let first_dealer = col_int(row.get("first_dealer"), 0)?;
    let started_at = col_int(row.get("started_at"), 0)?;
    let ended_at = col_int_or_null(row.get("ended_at"))?;
    let (status, outcome) = status_outcome(row)?;
    let g = |k: &str| col_int(row.get(k), 0);
    Ok(MatchRow {
        schema: SchemaV1,
        match_id: col_string(row.get("match_id")),
        user_id: col_string(row.get("user_id")),
        seed: col_string(row.get("seed")),
        cfg: JsJson(cfg),
        first_dealer,
        started_at,
        ended_at,
        status,
        outcome,
        score: col_pair(row.get("score_us"), row.get("score_them"))?,
        winner_team: col_int_or_null(row.get("winner_team"))?,
        hands_played: g("hands_played")?,
        personas: col_value_array(row.get("personas_json"))
            .into_iter()
            .map(JsJson)
            .collect(),
        stats: stats(row)?,
        tokens: TokenUsage {
            tokens_in: sat_u64(g("tokens_in")?),
            tokens_out: sat_u64(g("tokens_out")?),
            cache_read_tokens: sat_u64(g("cache_read_tokens")?),
            calls: sat_u64(g("llm_calls")?),
        },
    })
}

/// `readServerReplay`: the `SELECT *` match row and its `SELECT *` hand rows.
pub fn server_replay_from_rows(row: &Row, hands: &[Row]) -> ProfileResult<ServerMatchReplay> {
    let match_record = match_row_from_row(row)?;
    let hands = hands
        .iter()
        .map(hand_row_from_row)
        .collect::<ProfileResult<Vec<_>>>()?;
    Ok(ServerMatchReplay {
        match_record,
        hands,
    })
}
