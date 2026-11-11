//! Connection params, server-derived connection state and the table-session
//! response (02 sections 0.1 and 0.2).

use euchre_engine::Seat;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::ProtocolErrorCode;
use crate::serde_util::{lenient, string_enum};

/// Params a client sends on connect (`TableConnectParams`). **Untrusted.**
///
/// Reading keeps the raw claims so the table can apply the TS checks exactly
/// (`createConnState`), instead of a parse error or a silent `None` standing in
/// for them:
/// - `token` that is absent or not a string reads as `""` (TS `params?.token ?? ''`;
///   the table's own internal connections send `""` beside `internalToken`).
///   An empty or bad token is refused with `invalid_token`.
/// - `internalToken` that is not a string reads as `None` (TS `tokenOk` is false).
/// - `seat` and `protocolVersion` keep any present JSON value, `null` included,
///   so present-but-wrong differs from absent. See [`Self::claim_error`].
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableConnectParams {
    /// Server-only per-match credential. A desktop client must never send it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient::string_or_none"
    )]
    pub internal_token: Option<String>,
    /// The actor JWT.
    #[serde(default, deserialize_with = "lenient::string_or_empty")]
    pub token: String,
    /// Advisory seat claim, raw. Any present value but the number 0 (`null`,
    /// `"0"`, `7`, ...) is refused with `seat_mismatch`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient::present_value"
    )]
    pub seat: Option<Value>,
    /// The client's protocol version, raw. Any present value but the number 1
    /// (`null`, `"1"`, `1.5`, ...) is refused with `protocol_version_mismatch`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient::present_value"
    )]
    pub protocol_version: Option<Value>,
}

/// Whether a raw claim is absent or the JS number `expected` (strict `===`).
fn claim_is(claim: Option<&Value>, expected: f64) -> bool {
    match claim {
        None => true,
        Some(v) => v.as_f64() == Some(expected),
    }
}

impl TableConnectParams {
    /// Player params carrying a fresh actor token and the current protocol version.
    pub fn player(token: impl Into<String>) -> Self {
        TableConnectParams {
            internal_token: None,
            token: token.into(),
            seat: None,
            protocol_version: Some(Value::from(crate::PROTOCOL_VERSION)),
        }
    }

    /// Player params that also claim `seat` (only seat 0 is ever accepted).
    pub fn with_seat(mut self, seat: Seat) -> Self {
        self.seat = Some(Value::from(seat.get()));
        self
    }

    /// TS `params.protocolVersion === undefined || params.protocolVersion === 1`.
    pub fn protocol_version_ok(&self) -> bool {
        claim_is(
            self.protocol_version.as_ref(),
            f64::from(crate::PROTOCOL_VERSION),
        )
    }

    /// TS `params.seat === undefined || params.seat === 0` (`HUMAN_SEAT`).
    pub fn seat_claim_ok(&self) -> bool {
        claim_is(self.seat.as_ref(), 0.0)
    }

    /// The claim checks of a player connection, in the TS order (after the
    /// internal-token path, before the token is verified): a bad protocol
    /// version first, then a bad seat claim.
    pub fn claim_error(&self) -> Option<ProtocolErrorCode> {
        if !self.protocol_version_ok() {
            Some(ProtocolErrorCode::ProtocolVersionMismatch)
        } else if !self.seat_claim_ok() {
            Some(ProtocolErrorCode::SeatMismatch)
        } else {
            None
        }
    }
}

string_enum! {
    /// Who a connection is (`ConnRole`).
    ConnRole {
        /// The one human, always seat 0 in v1.
        Player => "player",
        /// Another actor holding the per-match internal token; receives no `sync`.
        Internal => "internal",
    }
}

/// Server-derived per-connection state (`TableConnState`), never client-supplied.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableConnState {
    /// Verified user id (`guest-…`, an account id, or `"internal"`).
    pub user_id: String,
    /// The seat this connection was assigned.
    pub seat: Seat,
    /// Player or internal.
    pub role: ConnRole,
    /// Epoch ms the connection was accepted.
    pub since: u64,
}

/// `POST /api/table-session/{gameId}` 200 body.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSessionResponse {
    /// The resolved rivet actor id.
    pub actor_id: String,
    /// The HS256 actor JWT for [`TableConnectParams::token`].
    pub token: String,
    /// Present only when the deployment has a gateway token configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_token: Option<String>,
    /// Epoch ms the session expires; clients refetch 10 s before.
    pub expires_at: u64,
}

/// How long before `expiresAt` a cached session is treated as stale (ms).
pub const SESSION_REFRESH_MARGIN_MS: u64 = 10_000;

impl TableSessionResponse {
    /// Whether the session should be refetched at `now_ms`.
    pub fn is_stale(&self, now_ms: u64) -> bool {
        now_ms.saturating_add(SESSION_REFRESH_MARGIN_MS) >= self.expires_at
    }
}
