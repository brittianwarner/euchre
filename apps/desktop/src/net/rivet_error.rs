//! The one mapping from a failed table action to what the player sees (contract §N1.6, web
//! plan S4), shared by the native remote backend and the browser backend.
//!
//! Both clients surface a thrown actor error as `{group, code, message, metadata}`: the
//! native rivetkit client inside an `anyhow::Error` (`euchre_server_core::rivet::
//! parse_rivet_error` recovers it), the browser bridge as JSON (`js/rivet-bridge.js`). Each
//! side converts to [`RivetError`] and calls [`move_rejection`] / [`action_error`]; no
//! structured error at all means the socket closed under the call.
//!
//! Builds on both targets with no native-only dependency.

use euchre_protocol::{ErrorCode, MoveAck, ProtocolErrorCode, UserErrorPayload};
use serde::Deserialize;
use serde_json::Value;

use crate::net::NetError;

/// The group of errors a table actor throws on purpose (`UserError`).
pub const USER_GROUP: &str = "user";

/// The group the browser bridge uses for its own "not connected" / "disposed" failures.
pub const CLIENT_GROUP: &str = "client";

/// A structured actor error: `group/code`, a developer message and optional metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct RivetError {
    /// `user`, `actor`, `core`, ... (`client` for the bridge's own refusals).
    pub group: String,
    /// The machine code within the group.
    pub code: String,
    /// Developer message; never shown to the player.
    pub message: String,
    /// Optional structured metadata (`illegal_move` carries `{ legal }`).
    pub metadata: Option<Value>,
}

/// The JSON the browser bridge rejects with (`ErrorShape` in `web/rivet-bridge.ts`).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct BridgeError {
    /// Error group, when the failure was structured.
    #[serde(default)]
    pub group: Option<String>,
    /// Error code, when the failure was structured.
    #[serde(default)]
    pub code: Option<String>,
    /// Message (developer copy).
    #[serde(default)]
    pub message: String,
    /// Metadata, `null` when none.
    #[serde(default)]
    pub metadata: Option<Value>,
    /// HTTP status of a failed `table-session` mint (`group: "session"`).
    #[serde(default)]
    pub status: Option<u16>,
}

impl BridgeError {
    /// Parses the bridge's error text; anything else (a bare string, a JS `Error` that
    /// escaped) is an unstructured error.
    pub fn parse(text: &str) -> BridgeError {
        serde_json::from_str(text).unwrap_or_else(|_| BridgeError {
            message: text.to_owned(),
            ..BridgeError::default()
        })
    }

    /// The structured error, when both a group and a code came back.
    pub fn rivet(&self) -> Option<RivetError> {
        Some(RivetError {
            group: self.group.clone()?,
            code: self.code.clone()?,
            message: self.message.clone(),
            metadata: self.metadata.clone().filter(|m| !m.is_null()),
        })
    }
}

impl RivetError {
    /// True when the actor threw it on purpose (`user/<code>`).
    pub fn is_user(&self) -> bool {
        self.group == USER_GROUP
    }

    /// True when this is `user/<code>`.
    pub fn is(&self, code: &str) -> bool {
        self.is_user() && self.code == code
    }

    /// True when the token was refused: the cached session must be minted again.
    pub fn refuses_token(&self) -> bool {
        self.is("forbidden") || self.is("invalid_token")
    }
}

/// `submitMove`'s thrown refusals as the ack the sim returns: a `user/<code>` error whose
/// code is an `ErrorCode` (`illegal_move` with `metadata.legal`, `game_complete`,
/// `internal_error`) becomes `MoveAck::Rejected { code, legal }`. `None` for anything else
/// (`forbidden` included), which [`action_error`] maps.
pub fn move_rejection(err: &RivetError) -> Option<MoveAck> {
    if !err.is_user() {
        return None;
    }
    let code: ErrorCode = err.code.parse().ok()?;
    if code == ErrorCode::Protocol(ProtocolErrorCode::Forbidden) {
        return None;
    }
    let payload = UserErrorPayload {
        code,
        message: err.message.clone(),
        metadata: err.metadata.clone(),
    };
    Some(MoveAck::Rejected {
        code,
        legal: payload.illegal_move_legal().unwrap_or_default(),
    })
}

/// Maps a failed action to the player's error: `user/forbidden` is [`NetError::Forbidden`],
/// any other `user/<code>` is [`NetError::Invalid`] with the code, the bridge's own
/// `client/*` and no structured error (the socket closed mid-call) are
/// [`NetError::Closed`], and any other structured error is [`NetError::Unavailable`].
pub fn action_error(err: Option<&RivetError>) -> NetError {
    match err {
        Some(e) if e.is("forbidden") => NetError::Forbidden,
        Some(e) if e.is_user() => NetError::Invalid(e.code.clone()),
        Some(e) if e.group == CLIENT_GROUP => NetError::Closed,
        Some(_) => NetError::Unavailable,
        None => NetError::Closed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rpc(group: &str, code: &str, metadata: Option<Value>) -> RivetError {
        RivetError {
            group: group.to_owned(),
            code: code.to_owned(),
            message: "x".to_owned(),
            metadata,
        }
    }

    #[test]
    fn illegal_move_becomes_a_rejected_ack() -> anyhow::Result<()> {
        let legal = json!([{ "id": "pass", "move": { "t": "pass" }, "label": "Pass" }]);
        let legal_moves: Vec<euchre_protocol::LegalMove> = serde_json::from_value(legal.clone())?;
        let err = rpc("user", "illegal_move", Some(json!({ "legal": legal })));
        let Some(MoveAck::Rejected { code, legal }) = move_rejection(&err) else {
            anyhow::bail!("not a rejection");
        };
        assert_eq!(code.as_str(), "illegal_move");
        assert_eq!(legal, legal_moves);
        assert_eq!(
            move_rejection(&rpc("user", "game_complete", None)),
            Some(MoveAck::Rejected {
                code: ErrorCode::Protocol(ProtocolErrorCode::GameComplete),
                legal: Vec::new(),
            })
        );
        assert_eq!(move_rejection(&rpc("user", "forbidden", None)), None);
        assert_eq!(move_rejection(&rpc("actor", "internal_error", None)), None);
        assert_eq!(move_rejection(&rpc("user", "made_up", None)), None);
        Ok(())
    }

    #[test]
    fn forbidden_and_other_errors_map() {
        assert_eq!(
            action_error(Some(&rpc("user", "forbidden", None))),
            NetError::Forbidden
        );
        assert_eq!(
            action_error(Some(&rpc("user", "invalid_token", None))),
            NetError::Invalid("invalid_token".into())
        );
        assert_eq!(
            action_error(Some(&rpc(
                "user",
                "rate_limited",
                Some(json!({"retryAfterMs": 500}))
            ))),
            NetError::Invalid("rate_limited".into())
        );
        assert_eq!(
            action_error(Some(&rpc("actor", "action_timed_out", None))),
            NetError::Unavailable
        );
        assert_eq!(
            action_error(Some(&rpc("core", "internal_error", None))),
            NetError::Unavailable
        );
        assert_eq!(
            action_error(Some(&rpc("client", "closed", None))),
            NetError::Closed
        );
        assert_eq!(action_error(None), NetError::Closed);
    }

    #[test]
    fn bridge_json_parses_structured_and_bare_errors() {
        let e = BridgeError::parse(
            r#"{"group":"user","code":"illegal_move","message":"no","metadata":{"legal":[]},"status":null}"#,
        );
        let r = e.rivet().expect("structured");
        assert!(r.is("illegal_move"));
        assert_eq!(r.metadata, Some(json!({ "legal": [] })));
        // A socket close: plain Error, no group/code.
        let e = BridgeError::parse(
            r#"{"group":null,"code":null,"message":"Socket closed","metadata":null,"status":null}"#,
        );
        assert_eq!(e.rivet(), None);
        assert_eq!(action_error(e.rivet().as_ref()), NetError::Closed);
        // A refused mint.
        let e = BridgeError::parse(
            r#"{"group":"session","code":"http_403","message":"m","metadata":null,"status":403}"#,
        );
        assert_eq!(e.status, Some(403));
        // Not JSON at all.
        let e = BridgeError::parse("boom");
        assert_eq!(e.message, "boom");
        assert_eq!(e.rivet(), None);
        assert!(rpc("user", "invalid_token", None).refuses_token());
        assert!(!rpc("actor", "forbidden", None).refuses_token());
    }
}
