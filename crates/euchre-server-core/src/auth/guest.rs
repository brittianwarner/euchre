//! Guest JWTs (05 §2.2, §2.3; TS `src/lib/actors/auth/guest.ts` and `verify.ts`).
//!
//! Wire format, byte for byte as jose 6 `SignJWT` produces it:
//! - header `{"alg":"HS256"}` (no `typ`);
//! - payload keys in order `kind, sub, iss, aud, iat, exp`, with `kind: "guest"`,
//!   `iss: "euchre-guest"`, `aud` one of [`SESSION_AUDIENCE`] / [`ACTORS_AUDIENCE`];
//! - HMAC-SHA256 over `b64url(header).b64url(payload)`, base64url without padding.
//!
//! A session token is never accepted as an actor token and vice versa (`aud`).

use serde::Serialize;

use super::AuthError;
use super::jwt::{Expect, sign_bare_hs256, verify_jose_hs256};

/// `iss` of every guest token.
pub const GUEST_ISSUER: &str = "euchre-guest";
/// `aud` of the long-lived browser/device session token (`euchre_guest` cookie, bearer).
pub const SESSION_AUDIENCE: &str = "euchre-session";
/// `aud` of the short-lived table connection token (shared with profile tokens).
pub const ACTORS_AUDIENCE: &str = "euchre-actors";
/// Session token lifetime: jose `'400d'`.
pub const SESSION_TTL_SECS: i64 = 400 * 86_400;
/// Actor token lifetime: jose `'2h'`.
pub const ACTORS_TTL_SECS: i64 = 2 * 3_600;
/// Every guest user id starts with this.
pub const GUEST_ID_PREFIX: &str = "guest-";
/// The TS local-dev magic token, always refused here.
pub const DEV_PLAYER_TOKEN: &str = "dev";
/// `kind` claim of guest tokens.
pub const GUEST_KIND: &str = "guest";

/// Which guest token family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestAudience {
    /// `euchre-session`, 400 days.
    Session,
    /// `euchre-actors`, 2 hours.
    Actors,
}

impl GuestAudience {
    /// The `aud` string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => SESSION_AUDIENCE,
            Self::Actors => ACTORS_AUDIENCE,
        }
    }

    /// Lifetime in seconds.
    pub fn ttl_secs(self) -> i64 {
        match self {
            Self::Session => SESSION_TTL_SECS,
            Self::Actors => ACTORS_TTL_SECS,
        }
    }
}

/// Claims a verified table connection establishes (TS `PlayerClaims`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerClaims {
    /// The guest id (`guest-…`).
    pub user_id: String,
    /// Always empty for guest tokens (TS `email: ''`).
    pub email: String,
}

#[derive(Serialize)]
struct GuestClaims<'a> {
    kind: &'a str,
    sub: &'a str,
    iss: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

/// A fresh guest id: `guest-` + a lowercase v4 UUID (TS `crypto.randomUUID()`).
pub fn new_guest_id() -> String {
    format!("{GUEST_ID_PREFIX}{}", uuid::Uuid::new_v4())
}

/// Sign a guest token for `user_id` issued at `iat` (unix seconds). TS `signGuest`.
pub fn sign(key: &[u8], user_id: &str, aud: GuestAudience, iat: i64) -> Result<String, AuthError> {
    let claims = GuestClaims {
        kind: GUEST_KIND,
        sub: user_id,
        iss: GUEST_ISSUER,
        aud: aud.as_str(),
        iat,
        exp: iat + aud.ttl_secs(),
    };
    sign_bare_hs256(key, &claims)
}

/// Verify a guest token at `now` (unix seconds) and return `sub`. TS `verifyGuest`:
/// HS256 only, issuer `euchre-guest`, the given audience, jose claim checks with zero
/// tolerance, then `kind == "guest"` and `sub` starting with `guest-`.
///
/// Deviation: `exp` is required (jose treats it as optional; every guest token has one).
pub fn verify(key: &[u8], token: &str, aud: GuestAudience, now: i64) -> Result<String, AuthError> {
    let payload = verify_jose_hs256(
        key,
        token,
        &Expect {
            issuer: GUEST_ISSUER,
            audience: aud.as_str(),
        },
        now,
    )?;
    let kind_ok = payload.get("kind").and_then(|v| v.as_str()) == Some(GUEST_KIND);
    match payload.get("sub").and_then(|v| v.as_str()) {
        Some(sub) if kind_ok && sub.starts_with(GUEST_ID_PREFIX) => Ok(sub.to_owned()),
        _ => Err(AuthError::invalid("Invalid guest identity")),
    }
}

/// Verify a table connection token (TS `verifyPlayer`). Empty and the literal `dev`
/// are refused (`missing_or_invalid_token`), and only HS256 guest actor tokens are
/// accepted: the TS Better Auth JWKS path is dead and not ported.
pub fn verify_player(key: &[u8], token: &str, now: i64) -> Result<PlayerClaims, AuthError> {
    if token.is_empty() || token == DEV_PLAYER_TOKEN {
        return Err(AuthError::invalid("missing_or_invalid_token"));
    }
    let user_id = verify(key, token, GuestAudience::Actors, now)?;
    Ok(PlayerClaims {
        user_id,
        email: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::Value;

    use super::*;

    const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";
    const NOW: i64 = 1_800_000_000;

    fn segment(token: &str, i: usize) -> Value {
        let seg = token.split('.').nth(i).unwrap_or_default();
        let bytes = URL_SAFE_NO_PAD.decode(seg).unwrap_or_default();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    #[test]
    fn header_and_payload_shape_match_jose() -> Result<(), AuthError> {
        let t = sign(KEY, "guest-abc", GuestAudience::Session, NOW)?;
        let header_seg = t.split('.').next().unwrap_or_default();
        assert_eq!(
            URL_SAFE_NO_PAD.decode(header_seg).unwrap_or_default(),
            br#"{"alg":"HS256"}"#
        );
        let payload = segment(&t, 1);
        let keys: Vec<&str> = payload
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(keys, ["kind", "sub", "iss", "aud", "iat", "exp"]);
        assert_eq!(payload["exp"].as_i64(), Some(NOW + 34_560_000));
        let a = sign(KEY, "guest-abc", GuestAudience::Actors, NOW)?;
        assert_eq!(segment(&a, 1)["exp"].as_i64(), Some(NOW + 7200));
        Ok(())
    }

    #[test]
    fn audiences_do_not_cross() -> Result<(), AuthError> {
        let s = sign(KEY, "guest-abc", GuestAudience::Session, NOW)?;
        let a = sign(KEY, "guest-abc", GuestAudience::Actors, NOW)?;
        assert_eq!(verify(KEY, &s, GuestAudience::Session, NOW)?, "guest-abc");
        assert_eq!(verify(KEY, &a, GuestAudience::Actors, NOW)?, "guest-abc");
        assert!(verify(KEY, &s, GuestAudience::Actors, NOW).is_err());
        assert!(verify(KEY, &a, GuestAudience::Session, NOW).is_err());
        Ok(())
    }

    #[test]
    fn expiry_has_zero_tolerance_like_jose() -> Result<(), AuthError> {
        let a = sign(KEY, "guest-abc", GuestAudience::Actors, NOW)?;
        assert!(verify(KEY, &a, GuestAudience::Actors, NOW + 7199).is_ok());
        assert!(
            verify(KEY, &a, GuestAudience::Actors, NOW + 7200).is_err(),
            "exp <= now fails"
        );
        Ok(())
    }

    #[test]
    fn sub_prefix_and_dev_token_rejected() -> Result<(), AuthError> {
        let t = sign(KEY, "user-abc", GuestAudience::Actors, NOW)?;
        assert_eq!(
            verify(KEY, &t, GuestAudience::Actors, NOW),
            Err(AuthError::invalid("Invalid guest identity"))
        );
        assert!(verify_player(KEY, "", NOW).is_err());
        assert!(verify_player(KEY, DEV_PLAYER_TOKEN, NOW).is_err());
        let ok = sign(KEY, "guest-1", GuestAudience::Actors, NOW)?;
        assert_eq!(verify_player(KEY, &ok, NOW)?.user_id, "guest-1");
        assert!(
            verify(
                b"another-key-another-key-another!",
                &ok,
                GuestAudience::Actors,
                NOW
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn guest_ids_are_prefixed_v4() {
        let id = new_guest_id();
        assert!(id.starts_with("guest-"));
        assert_eq!(id.len(), "guest-".len() + 36);
        assert_eq!(id, id.to_lowercase());
        assert_eq!(id.as_bytes()["guest-".len() + 14], b'4');
    }
}
