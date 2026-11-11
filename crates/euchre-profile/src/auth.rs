//! Connection authority (05 §3.2, D17): the TS `createConnState` algorithm as a
//! pure function, run in `on_before_connect` and again in every action.

use euchre_profile_model::{ProfileError, ProfileResult, owner_of};
use euchre_protocol::{PROTOCOL_VERSION, ProtocolErrorCode};
use euchre_server_core::auth::{AuthError, Authenticator};
use rivetkit::{ActorKey, ActorKeySegment};
use serde_json::Value;

/// Message of a `protocol_version_mismatch` refusal.
pub const PROTOCOL_VERSION_MESSAGE: &str = "unsupported protocol version";
/// Message when a valid token names another user.
pub const WRONG_OWNER_MESSAGE: &str = "token does not belong to this profile";

/// What a connection may do (TS `ProfileConnState.authenticated`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnAuth {
    /// A verified profile token for this profile's owner: every browser action.
    Owner,
    /// No token: the shape of an actor-to-actor call. Only the internal-token
    /// actions accept it, and only with the right `internalToken`.
    Anonymous,
}

impl ConnAuth {
    /// TS `requireAuthenticated(c.conn.state)`.
    pub fn require_owner(self) -> ProfileResult<()> {
        euchre_profile_model::require_authenticated(self == ConnAuth::Owner)
    }
}

/// The actor key as the strings TS sees (`c.key`). A number segment prints as
/// JS `String(n)` would for an integer, else as Rust's shortest float form.
pub fn key_strings(key: &ActorKey) -> Vec<String> {
    key.iter()
        .map(|segment| match segment {
            ActorKeySegment::String(s) => s.clone(),
            ActorKeySegment::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
        })
        .collect()
}

/// TS `ownerOf(c.key)` over a rivetkit key.
pub fn owner_of_key(key: &ActorKey) -> ProfileResult<String> {
    owner_of(&key_strings(key))
}

/// An [`AuthError`] as the `ProfileError` TS throws (`invalid_token` with the
/// verifier's message, `internal_token_mismatch`, ...).
pub fn auth_error(error: AuthError) -> ProfileError {
    let code = match &error {
        AuthError::InvalidToken(_) => ProtocolErrorCode::InvalidToken,
        AuthError::InternalTokenMismatch(_) => ProtocolErrorCode::InternalTokenMismatch,
        AuthError::InvalidEmail | AuthError::Signing(_) => ProtocolErrorCode::InternalError,
    };
    ProfileError::new(code, error.to_string())
}

/// TS `createConnState(c, params)` at `now_ms`, for the profile owned by `owner`:
///
/// 1. a present `protocolVersion` other than the number 1 is
///    `protocol_version_mismatch` "unsupported protocol version";
/// 2. an absent, non-string or empty `token` is [`ConnAuth::Anonymous`];
/// 3. the token must verify (`invalid_token` with the verifier's message) and
///    name `owner` (`forbidden` "token does not belong to this profile").
///
/// Params that are not an object read as having neither field, as
/// `params?.token` does.
pub fn authorize_params(
    auth: &Authenticator,
    owner: &str,
    params: &Value,
    now_ms: i64,
) -> ProfileResult<ConnAuth> {
    let object = params.as_object();
    if let Some(version) = object.and_then(|m| m.get("protocolVersion"))
        && version.as_f64() != Some(f64::from(PROTOCOL_VERSION))
    {
        return Err(ProfileError::new(
            ProtocolErrorCode::ProtocolVersionMismatch,
            PROTOCOL_VERSION_MESSAGE,
        ));
    }
    let token = match object.and_then(|m| m.get("token")) {
        Some(Value::String(t)) if !t.is_empty() => t,
        _ => return Ok(ConnAuth::Anonymous),
    };
    let claims = auth
        .verify_profile_token_at(token, now_ms)
        .map_err(auth_error)?;
    if claims.user_id != owner {
        return Err(ProfileError::forbidden(WRONG_OWNER_MESSAGE));
    }
    Ok(ConnAuth::Owner)
}

/// TS `assertInternalToken(token)` for an action argument: a non-string reads
/// as no token (`internal token missing`).
pub fn assert_internal_arg(auth: &Authenticator, token: Option<&Value>) -> ProfileResult<()> {
    auth.assert_internal_token(token.and_then(Value::as_str))
        .map_err(auth_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use euchre_server_core::auth::AuthKeys;
    use euchre_server_core::config::Secret;
    use serde_json::json;

    const NOW_MS: i64 = 1_762_819_200_000;

    fn auth() -> Authenticator {
        Authenticator::new(AuthKeys {
            guest: Secret::new("guest-key-0123456789abcdef"),
            profile: Secret::new("profile-key-0123456789abcdef"),
            internal: Secret::new("internal-key-0123456789abcdef"),
            identity: Secret::new("identity-key-0123456789abcdef"),
            jwt_issuer: None,
            allow_unsigned_email: false,
        })
    }

    fn token_for(user: &str) -> String {
        auth()
            .mint_profile_token_at(user, NOW_MS / 1000)
            .unwrap_or_default()
    }

    #[test]
    fn no_token_is_anonymous() {
        let a = auth();
        for params in [
            Value::Null,
            json!({}),
            json!({ "token": "" }),
            json!({ "token": 7 }),
            json!("x"),
            json!({ "protocolVersion": 1 }),
            json!({ "protocolVersion": 1.0, "token": null }),
        ] {
            assert_eq!(
                authorize_params(&a, "u1", &params, NOW_MS),
                Ok(ConnAuth::Anonymous),
                "{params}"
            );
        }
    }

    #[test]
    fn protocol_version_is_checked_first() {
        let a = auth();
        let good = token_for("u1");
        for version in [json!(2), json!(null), json!("1"), json!(1.5)] {
            let params = json!({ "protocolVersion": version, "token": good });
            let err = authorize_params(&a, "u1", &params, NOW_MS).err();
            assert_eq!(
                err,
                Some(ProfileError::new(
                    ProtocolErrorCode::ProtocolVersionMismatch,
                    "unsupported protocol version"
                )),
                "{params}"
            );
        }
    }

    #[test]
    fn owner_token_is_accepted_and_others_refused() {
        let a = auth();
        let params = json!({ "token": token_for("u1"), "protocolVersion": 1 });
        assert_eq!(
            authorize_params(&a, "u1", &params, NOW_MS),
            Ok(ConnAuth::Owner)
        );
        let err = authorize_params(&a, "u2", &params, NOW_MS).err();
        assert_eq!(
            err,
            Some(ProfileError::forbidden(
                "token does not belong to this profile"
            ))
        );
        let err = authorize_params(&a, "u1", &json!({ "token": "a.b.c" }), NOW_MS).err();
        assert_eq!(err.map(|e| e.code), Some(ProtocolErrorCode::InvalidToken));
        // Expired: 30 min lifetime plus 60 s skew.
        let late = NOW_MS + 31 * 60 * 1000 + 61_000;
        let err = authorize_params(&a, "u1", &params, late).err();
        assert_eq!(err.map(|e| e.code), Some(ProtocolErrorCode::InvalidToken));
    }

    #[test]
    fn owner_requirement_message() {
        assert_eq!(ConnAuth::Owner.require_owner(), Ok(()));
        assert_eq!(
            ConnAuth::Anonymous.require_owner(),
            Err(ProfileError::forbidden(
                "this connection is not authenticated"
            ))
        );
    }

    #[test]
    fn internal_token_argument() {
        let a = auth();
        let ok = json!("internal-key-0123456789abcdef");
        assert_eq!(assert_internal_arg(&a, Some(&ok)), Ok(()));
        let cases = [
            (None, "internal token missing"),
            (Some(json!(5)), "internal token missing"),
            (Some(json!("")), "internal token missing"),
            (Some(json!("nope")), "internal token rejected"),
        ];
        for (arg, message) in cases {
            assert_eq!(
                assert_internal_arg(&a, arg.as_ref()),
                Err(ProfileError::new(
                    ProtocolErrorCode::InternalTokenMismatch,
                    message
                ))
            );
        }
    }

    #[test]
    fn key_ownership() {
        let key = vec![
            ActorKeySegment::String("user".into()),
            ActorKeySegment::String("abc".into()),
        ];
        assert_eq!(owner_of_key(&key), Ok("abc".to_owned()));
        let numeric = vec![
            ActorKeySegment::String("user".into()),
            ActorKeySegment::Number(42.0),
        ];
        assert_eq!(owner_of_key(&numeric), Ok("42".to_owned()));
        let bad = vec![ActorKeySegment::String("table".into())];
        assert_eq!(
            owner_of_key(&bad).err().map(|e| e.message),
            Some("playerProfile must be keyed ['user', userId]".to_owned())
        );
    }
}
