//! Who is calling (03 §4, D17).
//!
//! Connection params are attacker-controlled. Two things carry authority: the
//! per-match `internalToken` (held by this table's `aiSeat`s) and a guest actor
//! JWT (`GuestAudience::Actors`) whose subject owns the table. The seat is
//! assigned, never accepted: a player is always seat 0.
//!
//! The checks run at connect ([`identify`], in `on_before_connect`). A live
//! WebSocket keeps the identity it proved at connect (TS kept it in conn
//! state), so a guest token that expires during a long match does not lock the
//! player out of their own socket ([`authorize`]). A stateless HTTP action has
//! no live socket and is verified in full on every call ([`caller`]),
//! expiry included.

use anyhow::Result;
use euchre_engine::{HUMAN_SEAT, Seat};
use euchre_protocol::{ProtocolErrorCode, TableConnectParams};
use euchre_server_core::auth::Authenticator;
use euchre_server_core::auth::internal::match_token_ok;
use euchre_server_core::config::OriginPolicy;
use euchre_server_core::rivet::user_error;
use serde_json::Value;

/// TS `'Not a player connection'`.
pub const NOT_A_PLAYER: &str = "Not a player connection";

/// A verified connection identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Identity {
    /// Holds this match's internal token (an `aiSeat`, or a server-side observer).
    Internal,
    /// A verified guest; not yet checked against the owner.
    Player(String),
}

/// Who made this action call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Caller {
    /// No connection: a schedule fire.
    Scheduled,
    /// A connection holding the match's internal token.
    Internal,
    /// The table's owner, at seat 0.
    Player(String),
}

fn protocol_error(code: ProtocolErrorCode, message: &str) -> anyhow::Error {
    user_error(code.as_str(), message)
}

/// Read params leniently: a non-object reads as empty params, so it fails as an
/// invalid token (TS `params?.token ?? ''`).
pub fn read_params(params: &Value) -> TableConnectParams {
    serde_json::from_value(params.clone()).unwrap_or_default()
}

/// The identity checks of TS `createConnState`, in order: the internal token,
/// the protocol version, the seat claim, then the token. `now` is epoch seconds.
///
/// `log_claims` logs a non-human seat claim (TS logs it at connect only).
pub fn identify(
    auth: &Authenticator,
    match_token: &str,
    params: &Value,
    now_secs: i64,
    log_claims: bool,
) -> Result<Identity> {
    let params = read_params(params);
    if match_token_ok(match_token, params.internal_token.as_deref()) {
        return Ok(Identity::Internal);
    }
    match params.claim_error() {
        Some(ProtocolErrorCode::ProtocolVersionMismatch) => {
            return Err(protocol_error(
                ProtocolErrorCode::ProtocolVersionMismatch,
                "Unsupported protocol version",
            ));
        }
        Some(code) => {
            if log_claims {
                tracing::warn!(
                    claimed = %params.seat.as_ref().map(|v| v.to_string()).unwrap_or_default(),
                    code = ProtocolErrorCode::SeatMismatch.as_str(),
                    "connection claimed a non-human seat"
                );
            }
            return Err(protocol_error(
                code,
                "Seat 0 is the only seat a player may occupy",
            ));
        }
        None => {}
    }
    match auth.verify_player_at(&params.token, now_secs) {
        Ok(claims) => Ok(Identity::Player(claims.user_id)),
        Err(_) => Err(protocol_error(
            ProtocolErrorCode::InvalidToken,
            "Invalid or expired token",
        )),
    }
}

/// TS `onBeforeConnect`: the internal token passes first; anything else must
/// come from an allowed Origin (an absent Origin is a native client, D1).
pub fn origin_gate(
    origins: &OriginPolicy,
    match_token: &str,
    params: &Value,
    origin: Option<&str>,
) -> Result<()> {
    let params = read_params(params);
    if match_token_ok(match_token, params.internal_token.as_deref()) || origins.check(origin) {
        return Ok(());
    }
    Err(protocol_error(
        ProtocolErrorCode::OriginNotAllowed,
        "Origin not allowed",
    ))
}

/// The owner check: an unclaimed table (`owner` empty) is `Ok(true)` (the
/// caller claims it), the owner is `Ok(false)`, anyone else is refused.
pub fn owner_claim(owner: &str, user_id: &str) -> Result<bool> {
    if owner.is_empty() {
        Ok(true)
    } else if owner == user_id {
        Ok(false)
    } else {
        Err(protocol_error(
            ProtocolErrorCode::Forbidden,
            "Not your game",
        ))
    }
}

/// Re-derive the caller of one action from its connection params (D17), for
/// a call with no identity kept from connect (a stateless HTTP action).
/// `params` is `None` for a schedule fire.
pub fn caller(
    auth: &Authenticator,
    match_token: &str,
    owner: &str,
    params: Option<&Value>,
    now_secs: i64,
) -> Result<Caller> {
    let Some(params) = params else {
        return Ok(Caller::Scheduled);
    };
    authorize(identify(auth, match_token, params, now_secs, false)?, owner)
}

/// The caller of an action made with an identity already verified: at connect
/// for a live WebSocket, or just now by [`caller`]. Only the owner check runs
/// (the owner never changes once claimed).
pub fn authorize(identity: Identity, owner: &str) -> Result<Caller> {
    match identity {
        Identity::Internal => Ok(Caller::Internal),
        Identity::Player(user) => {
            // A table is claimed at connect, so an unclaimed one here is refused too.
            if owner.is_empty() || owner != user {
                return Err(protocol_error(
                    ProtocolErrorCode::Forbidden,
                    "Not your game",
                ));
            }
            Ok(Caller::Player(user))
        }
    }
}

impl Caller {
    /// The player's user id, or `forbidden` `'Not a player connection'`.
    pub fn require_player(&self) -> Result<&str> {
        match self {
            Caller::Player(user) => Ok(user),
            _ => Err(protocol_error(ProtocolErrorCode::Forbidden, NOT_A_PLAYER)),
        }
    }

    /// The seat a read is redacted for: seat 0 for the player and for an
    /// internal connection (TS assigns both `HUMAN_SEAT`); a schedule fire has
    /// no view and is refused.
    pub fn require_reader(&self) -> Result<Seat> {
        match self {
            Caller::Player(_) | Caller::Internal => Ok(HUMAN_SEAT),
            Caller::Scheduled => Err(protocol_error(ProtocolErrorCode::Forbidden, NOT_A_PLAYER)),
        }
    }

    /// A former queue: internal connections and schedule fires only (TS
    /// `externalDenied`). The body's own token is checked after this.
    pub fn require_internal(&self) -> Result<()> {
        match self {
            Caller::Internal | Caller::Scheduled => Ok(()),
            Caller::Player(_) => Err(user_error("internal_only", "This action is internal.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use euchre_server_core::auth::{AuthKeys, GuestAudience};
    use euchre_server_core::config::Secret;
    use euchre_server_core::rivet::parse_rivet_error;
    use serde_json::json;

    const MATCH: &str = "match-token-0123";
    const NOW: i64 = 1_800_000_000;

    fn auth() -> Authenticator {
        Authenticator::new(AuthKeys {
            guest: Secret::new("unit-guest-key-0123456789abcdef"),
            profile: Secret::new("unit-profile-key-0123456789abcd"),
            internal: Secret::new("unit-internal-token-0123456789ab"),
            identity: Secret::new("unit-identity-key-0123456789abcd"),
            jwt_issuer: None,
            allow_unsigned_email: false,
        })
    }

    fn token(user: &str, aud: GuestAudience, iat: i64) -> String {
        auth()
            .sign_guest_at(user, aud, iat)
            .unwrap_or_else(|e| panic!("sign: {e}"))
    }

    fn code_of(r: Result<impl std::fmt::Debug>) -> (String, String) {
        let e = r.err().unwrap_or_else(|| panic!("expected an error"));
        let u = parse_rivet_error(&e).unwrap_or_else(|| panic!("not a rivet error: {e}"));
        (u.code, u.message)
    }

    #[test]
    fn identify_checks_in_ts_order() {
        let a = auth();
        let good = token("guest-a", GuestAudience::Actors, NOW);
        // The internal token wins over everything else.
        let p = json!({ "internalToken": MATCH, "seat": 2, "protocolVersion": 9 });
        assert_eq!(
            identify(&a, MATCH, &p, NOW, false).ok(),
            Some(Identity::Internal)
        );
        // Protocol version before seat, seat before token.
        let p = json!({ "token": "junk", "seat": 2, "protocolVersion": 9 });
        assert_eq!(
            code_of(identify(&a, MATCH, &p, NOW, false)),
            (
                "protocol_version_mismatch".into(),
                "Unsupported protocol version".into()
            )
        );
        let p = json!({ "token": "junk", "seat": 2, "protocolVersion": 1 });
        assert_eq!(
            code_of(identify(&a, MATCH, &p, NOW, true)),
            (
                "seat_mismatch".into(),
                "Seat 0 is the only seat a player may occupy".into()
            )
        );
        let p = json!({ "token": "junk", "seat": 0, "protocolVersion": 1 });
        assert_eq!(
            code_of(identify(&a, MATCH, &p, NOW, false)),
            ("invalid_token".into(), "Invalid or expired token".into())
        );
        // A forged internal token is just a missing token.
        let p = json!({ "internalToken": "forged" });
        assert_eq!(
            code_of(identify(&a, MATCH, &p, NOW, false)).0,
            "invalid_token"
        );
        // Wrong audience and expired tokens are refused.
        let session = token("guest-a", GuestAudience::Session, NOW);
        let p = json!({ "token": session });
        assert_eq!(
            code_of(identify(&a, MATCH, &p, NOW, false)).0,
            "invalid_token"
        );
        let p = json!({ "token": good });
        assert_eq!(
            code_of(identify(&a, MATCH, &p, NOW + 7201, false)).0,
            "invalid_token"
        );
        assert_eq!(
            identify(&a, MATCH, &p, NOW, false).ok(),
            Some(Identity::Player("guest-a".into()))
        );
        // A non-object reads as no token.
        assert_eq!(
            code_of(identify(&a, MATCH, &json!("x"), NOW, false)).0,
            "invalid_token"
        );
    }

    #[test]
    fn origin_gate_lets_the_internal_token_and_listed_origins_through() {
        let origins = OriginPolicy::strict(vec!["https://app.example".into()])
            .unwrap_or_else(|e| panic!("{e}"));
        let internal = json!({ "internalToken": MATCH });
        let player = json!({ "token": "t" });
        assert!(origin_gate(&origins, MATCH, &internal, Some("https://evil.example")).is_ok());
        assert!(origin_gate(&origins, MATCH, &player, Some("https://app.example")).is_ok());
        assert!(origin_gate(&origins, MATCH, &player, None).is_ok());
        assert_eq!(
            code_of(origin_gate(
                &origins,
                MATCH,
                &player,
                Some("https://evil.example")
            )),
            ("origin_not_allowed".into(), "Origin not allowed".into())
        );
    }

    #[test]
    fn owner_claim_adopts_the_first_visitor_only() {
        assert_eq!(owner_claim("", "guest-a").ok(), Some(true));
        assert_eq!(owner_claim("guest-a", "guest-a").ok(), Some(false));
        assert_eq!(
            code_of(owner_claim("guest-a", "guest-b")),
            ("forbidden".into(), "Not your game".into())
        );
    }

    #[test]
    fn caller_is_rederived_and_roles_gate_actions() {
        let a = auth();
        let good = json!({ "token": token("guest-a", GuestAudience::Actors, NOW) });
        let other = json!({ "token": token("guest-b", GuestAudience::Actors, NOW) });
        let internal = json!({ "internalToken": MATCH });

        let scheduled = caller(&a, MATCH, "guest-a", None, NOW).ok();
        assert_eq!(scheduled, Some(Caller::Scheduled));
        let player =
            caller(&a, MATCH, "guest-a", Some(&good), NOW).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(player, Caller::Player("guest-a".into()));
        assert_eq!(
            code_of(caller(&a, MATCH, "guest-a", Some(&other), NOW)).0,
            "forbidden"
        );
        // The token expired after connect: a stateless call is refused...
        assert_eq!(
            code_of(caller(&a, MATCH, "guest-a", Some(&good), NOW + 7201)).0,
            "invalid_token"
        );
        // ...while a live socket keeps the identity it proved at connect, and
        // the owner check still runs on it.
        let kept = identify(&a, MATCH, &good, NOW, false).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            authorize(kept.clone(), "guest-a").ok(),
            Some(Caller::Player("guest-a".into()))
        );
        assert_eq!(code_of(authorize(kept.clone(), "guest-b")).0, "forbidden");
        assert_eq!(code_of(authorize(kept, "")).0, "forbidden");
        assert_eq!(
            authorize(Identity::Internal, "guest-a").ok(),
            Some(Caller::Internal)
        );
        let internal =
            caller(&a, MATCH, "guest-a", Some(&internal), NOW).unwrap_or_else(|e| panic!("{e}"));

        // submitMove / getHandReview / reviewTrick: the player only.
        assert_eq!(player.require_player().ok(), Some("guest-a"));
        assert_eq!(
            code_of(internal.require_player()),
            ("forbidden".into(), NOT_A_PLAYER.into())
        );
        assert_eq!(code_of(Caller::Scheduled.require_player()).0, "forbidden");
        // snapshot / lastTrick: the player or an internal connection, at seat 0.
        assert_eq!(player.require_reader().ok(), Some(HUMAN_SEAT));
        assert_eq!(internal.require_reader().ok(), Some(HUMAN_SEAT));
        assert!(Caller::Scheduled.require_reader().is_err());
        // Former queues: never a player.
        assert!(internal.require_internal().is_ok());
        assert!(Caller::Scheduled.require_internal().is_ok());
        assert_eq!(
            code_of(player.require_internal()),
            ("internal_only".into(), "This action is internal.".into())
        );
    }
}
