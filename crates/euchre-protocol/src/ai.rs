//! Actor-to-actor messages between `euchreTable` and `aiSeat` (02 section 8.1, 04 section 2).

use euchre_engine::{AiDecisionSource, LegalMove, LegalMoveId, PublicGameView, Seat};
use serde::{Deserialize, Serialize};

use crate::serde_util::{js_f64, string_enum};
use crate::wire;

string_enum! {
    /// Which decision an AI seat is asked for (`AIDecisionKind`).
    AIDecisionKind {
        /// Bump or run the deck.
        Cut => "cut",
        /// First bidding round.
        Bid1 => "bid1",
        /// Dealer discard.
        Discard => "discard",
        /// Second bidding round.
        Bid2 => "bid2",
        /// Card play.
        Play => "play",
    }
}

/// Rust-style alias of [`AIDecisionKind`].
pub type AiDecisionKind = AIDecisionKind;

/// One heuristic-ranked option offered to the model (`RankedMove`).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedMove {
    /// A legal move id.
    pub id: LegalMoveId,
    /// Heuristic score; higher is better.
    #[serde(with = "js_f64")]
    pub score: f64,
    /// A short reason.
    pub why: String,
}

/// The table asking an AI seat to decide (`AIDecideRequest`, queue `decide`).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AIDecideRequest {
    /// Envelope token.
    pub internal_token: String,
    /// Envelope match id.
    pub game_id: String,
    /// The deciding seat.
    pub seat: Seat,
    /// The turn being decided.
    pub turn_id: String,
    /// Which decision.
    pub kind: AIDecisionKind,
    /// The view redacted for this seat, including actor metadata.
    #[serde(serialize_with = "wire::serialize_view")]
    pub view: PublicGameView,
    /// The legal set for this seat.
    pub legal: Vec<LegalMove>,
    /// Optional heuristic ranking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking: Option<Vec<RankedMove>>,
    /// Epoch ms by which a reply is expected.
    pub deadline_at: u64,
}

/// An AI seat's answer (`AIDecision`, queue `aiDecision`). The table re-derives
/// seat and turn and never trusts them.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AIDecision {
    /// Envelope token.
    pub internal_token: String,
    /// Envelope match id.
    pub game_id: String,
    /// The answering seat.
    pub seat: Seat,
    /// The turn answered.
    pub turn_id: String,
    /// The chosen move id.
    pub move_id: LegalMoveId,
    /// Model, heuristic fallback, or forced.
    pub source: AiDecisionSource,
    /// Optional table talk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banter: Option<String>,
    /// Optional short rationale for the log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// Decision latency.
    pub latency_ms: u64,
}

string_enum! {
    /// Lifecycle queue kinds sent to every AI seat (`AiSeatLifecycleKind`).
    AiSeatLifecycleKind {
        /// A new hand starts.
        ResetHand => "resetHand",
        /// A hand finished.
        HandEnd => "handEnd",
        /// The match finished.
        GameEnd => "gameEnd",
    }
}

/// A lifecycle message (`AiSeatLifecycleMessage`); `handNo` is absent for `gameEnd`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSeatLifecycleMessage {
    /// Envelope token.
    pub internal_token: String,
    /// Envelope match id.
    pub game_id: String,
    /// Which lifecycle point.
    pub kind: AiSeatLifecycleKind,
    /// The hand, for `resetHand` and `handEnd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand_no: Option<u32>,
}

/// Streamed or final table talk from an AI seat (`AiSayMsg`, queue `aiSay`).
///
/// Fields are declared in the key order of the real producer, aiSeat's
/// `table.send('aiSay', { internalToken, gameId, seat, turnId, handNo, msgId,
/// text, final })` (`ai-seat/index.ts`), not the interface order. `delta` has no
/// TS producer; it sits after `msgId`, as in the interface.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSayMsg {
    /// Envelope token.
    pub internal_token: String,
    /// Envelope match id.
    pub game_id: String,
    /// The speaking seat.
    pub seat: Seat,
    /// The turn the line belongs to; stale lines are dropped.
    pub turn_id: String,
    /// Post-move chatter may land during the next turn of the same hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand_no: Option<u32>,
    /// Stable across deltas and the final line.
    pub msg_id: String,
    /// A streamed fragment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    /// The full line, with `final: true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Whether this is the finished line.
    #[serde(rename = "final")]
    pub is_final: bool,
}
