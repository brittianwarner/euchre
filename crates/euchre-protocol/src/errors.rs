//! Error codes shared by every actor and every client (protocol/index.ts section 7).
//!
//! These are **machine codes**: the client maps each to its own copy and never
//! parses a message string. Rules verdicts are the engine's [`RuleCode`]; this
//! module adds the system-level [`ProtocolErrorCode`] and the union of both,
//! [`ErrorCode`], which is what a failed `MoveAck` and a `UserError` carry.

use std::fmt;
use std::str::FromStr;

use euchre_engine::{LegalMove, RuleCode};
use serde::de::{self, Deserializer, Unexpected};
use serde::{Deserialize, Serialize, Serializer};

use crate::serde_util::{UnknownVariant, string_enum};

string_enum! {
    /// Actor- and transport-level failures (`ProtocolErrorCode`).
    ProtocolErrorCode {
        /// The `Origin` header is not in the allowlist.
        OriginNotAllowed => "origin_not_allowed",
        /// The JWT is missing, malformed, expired, or fails verification.
        InvalidToken => "invalid_token",
        /// Authenticated, but not the owner of this match or profile.
        Forbidden => "forbidden",
        /// A payload claimed a seat that disagrees with the connection's.
        SeatMismatch => "seat_mismatch",
        /// An actor-to-actor message failed the constant-time token compare.
        InternalTokenMismatch => "internal_token_mismatch",
        /// No such match, or it has been reaped.
        GameNotFound => "game_not_found",
        /// The match is over or abandoned; no move will be accepted.
        GameComplete => "game_complete",
        /// A `clientMoveId` already in the dedupe ring.
        DuplicateMove => "duplicate_move",
        /// Too many submits from one connection in the window.
        RateLimited => "rate_limited",
        /// The client's protocol version is not one this deployment speaks.
        ProtocolVersionMismatch => "protocol_version_mismatch",
        /// Persona text failed validation.
        InvalidPersona => "invalid_persona",
        /// The AI seat could not be reached or did not reply.
        AiUnavailable => "ai_unavailable",
        /// An unexpected server fault.
        InternalError => "internal_error",
    }
}

/// `PROTOCOL_ERROR_CODES`: every [`ProtocolErrorCode`], for exhaustive mapping to copy.
pub const PROTOCOL_ERROR_CODES: &[ProtocolErrorCode] = ProtocolErrorCode::ALL;

/// Any error code this system produces: a rules verdict or a protocol failure
/// (`ErrorCode = RuleCode | ProtocolErrorCode`). Serialized as the bare code string.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ErrorCode {
    /// A rules verdict from the engine.
    Rule(RuleCode),
    /// A table- or transport-level refusal.
    Protocol(ProtocolErrorCode),
}

/// Why a `submitMove` was refused (`MoveRejectionCode = RuleCode | ProtocolErrorCode`).
pub type MoveRejectionCode = ErrorCode;

impl ErrorCode {
    /// The exact wire string.
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Rule(c) => c.as_str(),
            ErrorCode::Protocol(c) => c.as_str(),
        }
    }
}

impl From<RuleCode> for ErrorCode {
    fn from(c: RuleCode) -> Self {
        ErrorCode::Rule(c)
    }
}

impl From<ProtocolErrorCode> for ErrorCode {
    fn from(c: ProtocolErrorCode) -> Self {
        ErrorCode::Protocol(c)
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ErrorCode {
    type Err = UnknownVariant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(c) = RuleCode::from_str(s) {
            return Ok(ErrorCode::Rule(c));
        }
        ProtocolErrorCode::from_str(s)
            .map(ErrorCode::Protocol)
            .map_err(|_| UnknownVariant {
                type_name: "ErrorCode",
                value: s.to_owned(),
            })
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        ErrorCode::from_str(&s)
            .map_err(|_| de::Error::invalid_value(Unexpected::Str(&s), &"an euchre error code"))
    }
}

string_enum! {
    /// Connection-guard codes raised by the client binding before a call reaches an actor.
    ConnectionErrorCode {
        /// A retryable startup race.
        ActorNotYetConnected => "ACTOR_NOT_YET_CONNECTED",
        /// A real drop that should surface a reconnect banner.
        ActorDisconnected => "ACTOR_DISCONNECTED",
    }
}

/// The serialized shape of a failure that crosses the wire (`ProtocolError`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolError {
    /// The machine code. Clients choose copy by this.
    pub code: ErrorCode,
    /// For logs and developers; never shown verbatim.
    pub message: String,
    /// `true` when the same call may succeed unchanged on a retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

/// A thrown rivetkit `UserError` as a client sees it: `{code, message, metadata?}`.
///
/// The table throws `illegal_move` with `metadata: { legal }`; see
/// [`UserErrorPayload::illegal_move_legal`].
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserErrorPayload {
    /// The machine code.
    pub code: ErrorCode,
    /// The developer message.
    pub message: String,
    /// Optional structured metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// `metadata` of the table's thrown `illegal_move` user error.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct IllegalMoveMetadata {
    /// The current legal set for the caller's seat.
    pub legal: Vec<LegalMove>,
}

impl UserErrorPayload {
    /// The legal set carried by an `illegal_move` error's metadata, if any.
    pub fn illegal_move_legal(&self) -> Option<Vec<LegalMove>> {
        let meta = self.metadata.as_ref()?;
        serde_json::from_value::<IllegalMoveMetadata>(meta.clone())
            .ok()
            .map(|m| m.legal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_round_trip_both_families() {
        for c in RuleCode::ALL {
            let e = ErrorCode::from(*c);
            assert_eq!(ErrorCode::from_str(e.as_str()), Ok(e));
        }
        for c in ProtocolErrorCode::ALL {
            let e = ErrorCode::from(*c);
            assert_eq!(ErrorCode::from_str(e.as_str()), Ok(e));
        }
        assert!(ErrorCode::from_str("nope").is_err());
        assert_eq!(PROTOCOL_ERROR_CODES.len(), 13);
    }
}
