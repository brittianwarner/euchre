//! The two outbound seams of the table: AI dispatch (`aiSeat`) and profile
//! recording (`playerProfile`).
//!
//! The table core never talks to another actor directly. It calls these traits,
//! and every call is **detached** (03 §16.7): nothing here may hold the table's
//! lane across a network round trip. The actor's implementation is
//! `crate::remote::Remote` (one ordered outbox per AI seat and one for the
//! profile, each call bounded by [`crate::send_bounded::send_bounded`]).
//!
//! With no client bound, [`HeuristicSeat`] asks no one: `dispatchAi` has
//! already parked the `euchre-ai` heuristic top move in `game.pending` (D8), so
//! with [`AiDispatch::asks`] `false` the reveal timer plays that move at the
//! think floor. [`NoProfile`] records nothing: the match is marked recorded at
//! game over, as TS does when no deployment secret is configured.

use euchre_engine::Seat;
use euchre_protocol::{AIDecideRequest, AiSeatLifecycleMessage, HandJournalEntry, MatchRecord};
use serde_json::Value;

/// One `decide` request for an `aiSeat`.
#[derive(Clone, Debug)]
pub struct DecideCall {
    /// The seat asked.
    pub seat: Seat,
    /// The seat actor's key, `["table", gameId, "seat", "<seat>"]`.
    pub key: [String; 4],
    /// The `aiSeat` create input (TS `aiSeatCreateInput`, key order kept).
    pub create_input: Value,
    /// The request body.
    pub request: AIDecideRequest,
}

/// One lifecycle notification (`resetHand` / `handEnd` / `gameEnd`) for an `aiSeat`.
#[derive(Clone, Debug)]
pub struct LifecycleCall {
    /// The seat notified.
    pub seat: Seat,
    /// The seat actor's key.
    pub key: [String; 4],
    /// The `aiSeat` create input.
    pub create_input: Value,
    /// The message body; its `kind` is the action name.
    pub message: AiSeatLifecycleMessage,
}

/// Where AI decisions are asked for. Every method returns at once.
pub trait AiDispatch: Send + Sync {
    /// Whether a real seat is asked. `false` means the parked heuristic move
    /// plays at the think floor instead of waiting for an answer.
    fn asks(&self) -> bool;
    /// Ask a seat to decide. Detached: the answer arrives as an `aiDecision` call.
    fn decide(&self, call: DecideCall);
    /// Tell a seat about a hand or match boundary. Detached and best-effort.
    fn notify(&self, call: LifecycleCall);
}

/// The dispatch with no client bound: ask no one; the parked heuristic plays (D8).
#[derive(Clone, Copy, Debug, Default)]
pub struct HeuristicSeat;

impl AiDispatch for HeuristicSeat {
    fn asks(&self) -> bool {
        false
    }

    fn decide(&self, _call: DecideCall) {}

    fn notify(&self, _call: LifecycleCall) {}
}

/// One finished hand for the owner's `playerProfile` (TS `recordHand`).
#[derive(Clone, Debug)]
pub struct RecordHandsCall {
    /// The profile user (`profileUserId ?? ownerUserId`); the key is `["user", it]`.
    pub profile_user: String,
    /// The match id.
    pub match_id: String,
    /// The hand.
    pub entry: HandJournalEntry,
}

/// The finished match for the owner's `playerProfile` (TS `recordMatchIfNeeded`).
#[derive(Clone, Debug)]
pub struct RecordGameCall {
    /// The profile user; the key is `["user", it]`.
    pub profile_user: String,
    /// The record.
    pub record: MatchRecord,
}

/// Where the match history is recorded. Every method returns at once.
pub trait ProfileSink: Send + Sync {
    /// Whether a profile can be written at all (TS: a deployment secret is
    /// configured). When `false` the match is marked recorded locally.
    fn available(&self) -> bool;
    /// Append one finished hand. Detached; a failure is logged and dropped.
    fn record_hands(&self, call: RecordHandsCall);
    /// Record the finished match. Detached: the outcome comes back to the
    /// table as `Session::on_record_game` in a later lane turn.
    fn record_game(&self, call: RecordGameCall);
}

/// The profile sink with no client or no token: nothing to record into.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoProfile;

impl ProfileSink for NoProfile {
    fn available(&self) -> bool {
        false
    }

    fn record_hands(&self, _call: RecordHandsCall) {}

    fn record_game(&self, _call: RecordGameCall) {}
}
