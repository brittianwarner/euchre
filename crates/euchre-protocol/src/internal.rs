//! The table's internal queue bodies (`queues.ts`): `move`, `review`, `tick`.
//!
//! Every body but `move` carries the per-match envelope (`internalToken`,
//! `gameId`), checked with a constant-time compare before any state is read.

use euchre_engine::{LegalMoveId, Seat};
use serde::{Deserialize, Serialize};

use crate::serde_util::string_enum;

/// The actor-to-actor authorization envelope (`InternalEnvelope`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalEnvelope {
    /// The per-match secret.
    pub internal_token: String,
    /// The match id.
    pub game_id: String,
}

/// Constant-time token compare (`tokenOk`): `false` for an empty expected token.
pub fn token_ok(expected: &str, got: &str) -> bool {
    if expected.is_empty() || got.len() != expected.len() {
        return false;
    }
    expected
        .bytes()
        .zip(got.bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

/// A human move on the durable `move` queue (`MoveMsg`). No seat field exists.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveMsg {
    /// Queue sequence number.
    pub seq: u64,
    /// The turn the move was chosen against; `None` (a `submitMove` without
    /// `turnId`, D9) skips the stale check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// The chosen move id.
    pub move_id: LegalMoveId,
    /// The client's idempotency key.
    pub client_move_id: String,
    /// The verified sender.
    pub user_id: String,
}

string_enum! {
    /// The two review operations (`ReviewMsg.action`).
    ReviewAction {
        /// Toggle review mode.
        Set => "set",
        /// Continue past a reviewed trick.
        Continue => "continue",
    }
}

/// A queued `reviewTrick` (`ReviewMsg`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewMsg {
    /// Envelope token.
    pub internal_token: String,
    /// Envelope match id.
    pub game_id: String,
    /// Set or continue.
    pub action: ReviewAction,
    /// Present for `set` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// The request's turn for `continue`; the current turn for `set`.
    pub turn_id: String,
}

string_enum! {
    /// Why the table woke itself (`TickKind`).
    TickKind {
        /// The tempo gate before the next turn.
        Tempo => "tempo",
        /// Release a held AI move.
        ReleaseAi => "releaseAi",
        /// An AI seat missed its deadline.
        AiTimeout => "aiTimeout",
        /// Legacy human nudge.
        Nudge => "nudge",
        /// Legacy abandon.
        Abandon => "abandon",
        /// Retry the profile write.
        FlushProfile => "flushProfile",
        /// Reap an idle finished table.
        Reap => "reap",
    }
}

/// A scheduled self-message on the `tick` queue (`TickMsg`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TickMsg {
    /// Envelope token.
    pub internal_token: String,
    /// Envelope match id.
    pub game_id: String,
    /// What fired.
    pub kind: TickKind,
    /// The turn it was armed for; stale ticks are dropped.
    pub turn_id: String,
    /// The seat, for `aiTimeout`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat: Option<Seat>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_ok_is_exact_and_rejects_empty() {
        assert!(token_ok("abc", "abc"));
        assert!(!token_ok("abc", "abd"));
        assert!(!token_ok("abc", "ab"));
        assert!(!token_ok("", ""));
    }
}
