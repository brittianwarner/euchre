//! Client-to-server action shapes (02 section 5), actor and action names, and
//! actor keys.

use euchre_engine::{Card, EngineConfigOverrides, LegalMove, LegalMoveId, Move, Seat, Suit};
use serde::de::{self, Deserializer};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::ErrorCode;
use crate::persona::PersonaAssignment;
use crate::serde_util::{True, lenient};

/// `submitMove` argument (`SubmitMoveRequest`). There is no seat field: the
/// acting seat is the connection's.
///
/// Reading is lenient on purpose, so the table's checks run in the TS order
/// (`forbidden`, then `game_complete`, then `illegal_move` with `{ legal }`,
/// then `internal_error` for a missing `clientMoveId`) instead of a parse error
/// pre-empting them. Any payload reads, including `null` or a non-object:
/// - `moveId` that is absent or not a string reads as `""`, which is never in
///   `legal`, so it is refused with `illegal_move` (TS `request?.moveId`).
/// - `turnId` that is absent reads as `None` and skips the stale check (D9,
///   `apply()` with `turnId: undefined`). Present but not a string (`null`, a
///   number) reads as `Some("")`, which never equals a real turn id, so it is
///   refused with `stale_turn`, as TS `null !== state.turnId` is.
/// - `clientMoveId` that is absent or not a string reads as `""`; the table
///   refuses an empty one with `internal_error` ([`Self::has_client_move_id`]).
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitMoveRequest {
    /// An id from the current `view.legal`.
    pub move_id: LegalMoveId,
    /// The `view.turnId` the move was chosen against; `None` skips the stale check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// A fresh UUID per attempt, for idempotent retries.
    pub client_move_id: String,
}

impl SubmitMoveRequest {
    /// Reads a request from any JSON value, as described on the type.
    pub fn from_wire(value: &Value) -> Self {
        let field = |name: &str| value.as_object().and_then(|o| o.get(name));
        SubmitMoveRequest {
            move_id: lenient::js_string(field("moveId")),
            turn_id: field("turnId").map(|v| lenient::js_string(Some(v))),
            client_move_id: lenient::js_string(field("clientMoveId")),
        }
    }

    /// The TS `typeof clientMoveId === 'string' && clientMoveId.length > 0` check.
    pub fn has_client_move_id(&self) -> bool {
        !self.client_move_id.is_empty()
    }
}

impl<'de> Deserialize<'de> for SubmitMoveRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(SubmitMoveRequest::from_wire(&Value::deserialize(
            deserializer,
        )?))
    }
}

/// `submitMove` reply (`MoveAck`), discriminated by the boolean `ok`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MoveAck {
    /// `{ ok: true, v }`: the move landed (or was an idempotent duplicate) at `v`.
    Ok {
        /// The match `v` after the move.
        v: u64,
    },
    /// `{ ok: false, code, legal }`: refused; `legal` is the current set for the seat.
    Rejected {
        /// Why it was refused.
        code: ErrorCode,
        /// The current legal moves, so the client can snap back.
        legal: Vec<LegalMove>,
    },
}

impl MoveAck {
    /// Whether the move was accepted.
    pub fn is_ok(&self) -> bool {
        matches!(self, MoveAck::Ok { .. })
    }
}

impl Serialize for MoveAck {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            MoveAck::Ok { v } => {
                let mut m = serializer.serialize_map(Some(2))?;
                m.serialize_entry("ok", &true)?;
                m.serialize_entry("v", v)?;
                m.end()
            }
            MoveAck::Rejected { code, legal } => {
                let mut m = serializer.serialize_map(Some(3))?;
                m.serialize_entry("ok", &false)?;
                m.serialize_entry("code", code)?;
                m.serialize_entry("legal", legal)?;
                m.end()
            }
        }
    }
}

/// The flat shape both `MoveAck` arms share, for the manual `Deserialize`.
#[derive(Deserialize)]
struct RawMoveAck {
    ok: bool,
    #[serde(default)]
    v: Option<u64>,
    #[serde(default)]
    code: Option<ErrorCode>,
    #[serde(default)]
    legal: Option<Vec<LegalMove>>,
}

impl<'de> Deserialize<'de> for MoveAck {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawMoveAck::deserialize(deserializer)?;
        if raw.ok {
            let v = raw.v.ok_or_else(|| de::Error::missing_field("v"))?;
            Ok(MoveAck::Ok { v })
        } else {
            let code = raw.code.ok_or_else(|| de::Error::missing_field("code"))?;
            let legal = raw.legal.ok_or_else(|| de::Error::missing_field("legal"))?;
            Ok(MoveAck::Rejected { code, legal })
        }
    }
}

/// `reviewTrick` argument: toggle review mode, or continue past a reviewed trick.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ReviewTrickRequest {
    /// `{ action: "set", enabled }`.
    Set {
        /// Whether completed tricks wait for the player.
        enabled: bool,
    },
    /// `{ action: "continue", turnId }`.
    Continue {
        /// The `view.turnId` of the trick being continued.
        turn_id: String,
    },
}

/// `reviewTrick` reply: always `{ queued: true }` once enqueued.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewTrickAck {
    /// The literal `true`.
    pub queued: True,
}

/// The `getHandReview(handNo)` argument as the table validates it: `Some` only
/// for an integer in `0..=current_hand_no` (JS `Number.isInteger`, so `3.0` counts).
pub fn hand_review_number(arg: &Value, current_hand_no: u32) -> Option<u32> {
    let n = match arg {
        Value::Number(n) => n,
        _ => return None,
    };
    let hand = if let Some(u) = n.as_u64() {
        u
    } else {
        let f = n.as_f64()?;
        if !f.is_finite() || f.fract() != 0.0 || f < 0.0 {
            return None;
        }
        if f > f64::from(current_hand_no) {
            return None;
        }
        // Integral and within u32 range by the checks above.
        f as u64
    };
    let hand = u32::try_from(hand).ok()?;
    (hand <= current_hand_no).then_some(hand)
}

/// The table's creation input (`TableCreateInput`), supplied by `/play` or a fixture.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableCreateInput {
    /// The one human. When absent the first authenticated connection claims the table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
    /// The `playerProfile` user, when it differs from the owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_user_id: Option<String>,
    /// Overrides the random seed. Fixtures and replays only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<String>,
    /// Merged over the engine defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfg: Option<EngineConfigOverrides>,
    /// The first dealer; random when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_dealer: Option<Seat>,
    /// The three opponents, snapshotted at creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personas: Option<Vec<PersonaAssignment>>,
}

/// Actor names in the registry.
pub mod actor_names {
    /// The authoritative table actor.
    pub const EUCHRE_TABLE: &str = "euchreTable";
    /// One actor per AI opponent.
    pub const AI_SEAT: &str = "aiSeat";
    /// The durable per-user profile.
    pub const PLAYER_PROFILE: &str = "playerProfile";
}

/// `euchreTable` action names.
pub mod table_actions {
    /// `snapshot() -> PublicGameView`.
    pub const SNAPSHOT: &str = "snapshot";
    /// `submitMove(SubmitMoveRequest) -> MoveAck`.
    pub const SUBMIT_MOVE: &str = "submitMove";
    /// `reviewTrick(ReviewTrickRequest) -> { queued: true }`.
    pub const REVIEW_TRICK: &str = "reviewTrick";
    /// `getHandReview(handNo) -> CompletedHandReview | null`.
    pub const GET_HAND_REVIEW: &str = "getHandReview";
    /// `lastTrick() -> Trick | null`.
    pub const LAST_TRICK: &str = "lastTrick";
    /// Schedule target: the tempo gate fired.
    pub const ON_TEMPO_GATE: &str = "onTempoGate";
    /// Schedule target: release a held AI move.
    pub const RELEASE_AI_MOVE: &str = "releaseAiMove";
    /// Schedule target: an AI seat missed its deadline.
    pub const ON_AI_TIMEOUT: &str = "onAiTimeout";
    /// Schedule target: legacy human nudge (no-op).
    pub const ON_NUDGE: &str = "onNudge";
    /// Schedule target: legacy abandon (no-op).
    pub const ON_ABANDON: &str = "onAbandon";
    /// Schedule target: retry the profile write.
    pub const FLUSH_PROFILE: &str = "flushProfile";
    /// Schedule target: reap an idle finished table.
    pub const ON_REAP: &str = "onReap";

    /// Actions a client connection may call.
    pub const CLIENT: [&str; 5] = [
        SNAPSHOT,
        SUBMIT_MOVE,
        REVIEW_TRICK,
        GET_HAND_REVIEW,
        LAST_TRICK,
    ];
    /// Schedule-target actions that must never be client-callable.
    pub const SCHEDULE_TARGETS: [&str; 7] = [
        ON_TEMPO_GATE,
        RELEASE_AI_MOVE,
        ON_AI_TIMEOUT,
        ON_NUDGE,
        ON_ABANDON,
        FLUSH_PROFILE,
        ON_REAP,
    ];
}

/// `euchreTable` internal queue names.
pub mod table_queues {
    /// [`crate::MoveMsg`].
    pub const MOVE: &str = "move";
    /// [`crate::ReviewMsg`].
    pub const REVIEW: &str = "review";
    /// [`crate::AIDecision`].
    pub const AI_DECISION: &str = "aiDecision";
    /// [`crate::AiSayMsg`].
    pub const AI_SAY: &str = "aiSay";
    /// [`crate::TickMsg`].
    pub const TICK: &str = "tick";
}

/// `aiSeat` queue and action names.
pub mod ai_seat_actions {
    /// Queue: [`crate::AIDecideRequest`].
    pub const DECIDE: &str = "decide";
    /// Queue: [`crate::AiSeatLifecycleMessage`] at a new hand.
    pub const RESET_HAND: &str = "resetHand";
    /// Queue: [`crate::AiSeatLifecycleMessage`] at hand end.
    pub const HAND_END: &str = "handEnd";
    /// Queue: [`crate::AiSeatLifecycleMessage`] at game end.
    pub const GAME_END: &str = "gameEnd";
    /// Action: seat status.
    pub const GET_STATUS: &str = "getStatus";
    /// Action: [`crate::PersonaView`].
    pub const GET_PERSONA: &str = "getPersona";
    /// Action: the bounded decision log.
    pub const GET_DECISION_LOG: &str = "getDecisionLog";
}

/// `playerProfile` queue and action names (05 section 0.6).
pub mod profile_actions {
    /// Queue: a finished match.
    pub const RECORD_GAME: &str = "recordGame";
    /// Queue: hand journal entries.
    pub const RECORD_HANDS: &str = "recordHands";
    /// Queue: persona episodes.
    pub const RECORD_EPISODES: &str = "recordEpisodes";
    /// Queue: set or clear the active game.
    pub const SET_ACTIVE_GAME: &str = "setActiveGame";
    /// Queue: daily token usage.
    pub const RECORD_TOKENS: &str = "recordTokens";
    /// Action: read settings.
    pub const GET_SETTINGS: &str = "getSettings";
    /// Action: patch settings.
    pub const UPDATE_SETTINGS: &str = "updateSettings";
    /// Action: the persona views.
    pub const GET_PERSONAS: &str = "getPersonas";
    /// Action: the active game id or null.
    pub const GET_ACTIVE_GAME: &str = "getActiveGame";
    /// Action: lifetime stats.
    pub const GET_STATS: &str = "getStats";
    /// Action: a page of match summaries.
    pub const LIST_GAMES: &str = "listGames";
    /// Action: a client-safe replay.
    pub const GET_REPLAY: &str = "getReplay";
    /// Action: stored episodes.
    pub const LIST_EPISODES: &str = "listEpisodes";
    /// Action: delete one episode.
    pub const FORGET_EPISODE: &str = "forgetEpisode";
    /// Internal action: the full server replay.
    pub const GET_SERVER_REPLAY: &str = "getServerReplay";
    /// Internal action: episodes for one persona role.
    pub const GET_EPISODES_FOR_ROLE: &str = "getEpisodesForRole";
    /// Internal action: drop expired episodes.
    pub const PRUNE_EPISODES: &str = "pruneEpisodes";
}

/// Parses a [`LegalMoveId`] back into its [`Move`]; the exact inverse of
/// `Move::id()`. Ids are compared exactly and never normalized.
pub fn parse_move_id(id: &str) -> Option<Move> {
    let (base, alone) = match id.strip_suffix("+alone") {
        Some(base) => (base, true),
        None => (id, false),
    };
    let mv = match base {
        "pass" if !alone => Move::Pass,
        "orderUp" => Move::OrderUp { alone },
        "cut:yes" if !alone => Move::Cut { cut: true },
        "cut:no" if !alone => Move::Cut { cut: false },
        _ => {
            let (tag, rest) = base.split_once(':')?;
            match tag {
                "call" => {
                    let mut chars = rest.chars();
                    let suit = Suit::from_char(chars.next()?)?;
                    if chars.next().is_some() {
                        return None;
                    }
                    Move::Call { suit, alone }
                }
                "discard" if !alone => Move::Discard {
                    card: Card::parse(rest)?,
                },
                "play" if !alone => Move::Play {
                    card: Card::parse(rest)?,
                },
                _ => return None,
            }
        }
    };
    (mv.id() == id).then_some(mv)
}

/// Whether `s` is a lowercase UUID, the only accepted `gameId` shape.
pub fn is_game_id(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    b.iter().enumerate().all(|(i, c)| match i {
        8 | 13 | 18 | 23 => *c == b'-',
        _ => c.is_ascii_digit() || (b'a'..=b'f').contains(c),
    })
}

/// The `euchreTable` key `["table", gameId]`.
pub fn table_key(game_id: &str) -> [String; 2] {
    ["table".to_owned(), game_id.to_owned()]
}

/// The `aiSeat` key `["table", gameId, "seat", "<1|2|3>"]`.
pub fn ai_seat_key(game_id: &str, seat: Seat) -> [String; 4] {
    [
        "table".to_owned(),
        game_id.to_owned(),
        "seat".to_owned(),
        seat.get().to_string(),
    ]
}

/// The `playerProfile` key `["user", userId]`.
pub fn profile_key(user_id: &str) -> [String; 2] {
    ["user".to_owned(), user_id.to_owned()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hand_review_number_matches_the_table_guard() {
        assert_eq!(hand_review_number(&json!(0), 3), Some(0));
        assert_eq!(hand_review_number(&json!(3), 3), Some(3));
        assert_eq!(hand_review_number(&json!(3.0), 3), Some(3));
        assert_eq!(hand_review_number(&json!(4), 3), None);
        assert_eq!(hand_review_number(&json!(-1), 3), None);
        assert_eq!(hand_review_number(&json!(1.5), 3), None);
        assert_eq!(hand_review_number(&json!("1"), 3), None);
        assert_eq!(hand_review_number(&Value::Null, 3), None);
    }

    #[test]
    fn move_ids_parse_back_to_their_moves() {
        let ids = [
            "pass",
            "orderUp",
            "orderUp+alone",
            "call:H",
            "call:S+alone",
            "cut:yes",
            "cut:no",
            "discard:9C",
            "play:JS",
        ];
        for id in ids {
            let mv = parse_move_id(id).unwrap_or_else(|| panic!("{id} parses"));
            assert_eq!(mv.id(), id);
        }
        for bad in [
            "pass+alone",
            "call:X",
            "call:HH",
            "play:1S",
            "play:JS+alone",
            "cut:maybe",
            "Pass",
            "",
        ] {
            assert_eq!(parse_move_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn game_ids_are_lowercase_uuids() {
        assert!(is_game_id("6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b"));
        assert!(!is_game_id("6F1D2C3B-4A5E-4F60-8A7B-9C0D1E2F3A4B"));
        assert!(!is_game_id("6f1d2c3b4a5e4f608a7b9c0d1e2f3a4b"));
        assert!(!is_game_id("6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4g"));
    }

    #[test]
    fn actor_keys_have_the_ts_shape() {
        let seat = Seat::new(2).expect("seat 2");
        assert_eq!(table_key("g"), ["table", "g"]);
        assert_eq!(ai_seat_key("g", seat), ["table", "g", "seat", "2"]);
        assert_eq!(profile_key("guest-1"), ["user", "guest-1"]);
    }

    #[test]
    fn move_ack_requires_the_fields_of_its_arm() {
        assert!(serde_json::from_str::<MoveAck>(r#"{"ok":true}"#).is_err());
        assert!(serde_json::from_str::<MoveAck>(r#"{"ok":false,"code":"stale_turn"}"#).is_err());
        assert!(serde_json::from_str::<MoveAck>(r#"{"ok":false,"legal":[]}"#).is_err());
        assert!(
            serde_json::from_str::<MoveAck>(r#"{"ok":false,"code":"nope","legal":[]}"#).is_err()
        );
        let ok: MoveAck = serde_json::from_str(r#"{"v":3,"ok":true}"#).expect("ok ack");
        assert_eq!(ok, MoveAck::Ok { v: 3 });
    }

    #[test]
    fn review_request_rejects_bad_shapes() {
        assert!(serde_json::from_str::<ReviewTrickRequest>(r#"{"action":"set"}"#).is_err());
        assert!(
            serde_json::from_str::<ReviewTrickRequest>(r#"{"action":"set","enabled":"yes"}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<ReviewTrickRequest>(r#"{"action":"continue"}"#).is_err());
        assert!(serde_json::from_str::<ReviewTrickRequest>(r#"{"action":"skip"}"#).is_err());
        assert!(serde_json::from_str::<ReviewTrickAck>(r#"{"queued":false}"#).is_err());
    }
}
