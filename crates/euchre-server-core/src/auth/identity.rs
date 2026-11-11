//! Email nickname identity (05 §2.5, D2; TS `routes/games/_lib/session.server.ts`).
//!
//! The email is a nickname, not a login: [`normalize_email`] (JS `trim().toLowerCase()`
//! plus the TS shape check) and [`derive_user_id`] (lowercase hex SHA-256) are byte
//! for byte the TS functions, so a profile created by the TS app keeps its key.
//!
//! D2 adds a signed identity token so the `euchre_email` cookie and the
//! `X-Euchre-Identity` header can no longer be forged by typing someone else's
//! email into a cookie: HS256, header `{"alg":"HS256"}`, claims in order
//! `kind:"email-nick", sub, em, iss:"euchre-identity", aud:"euchre-identity", iat, exp`
//! with a 400-day lifetime, verified with jose `jwtVerify` semantics. The legacy
//! plain-email cookie is accepted only when `EUCHRE_ALLOW_UNSIGNED_EMAIL=1`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::AuthError;
use super::jwt::{Expect, sign_bare_hs256, verify_jose_hs256};
use crate::config::{is_js_whitespace, js_len, js_trim};

/// Cookie carrying the identity (a signed token; legacy: the normalized email).
pub const IDENTITY_COOKIE: &str = "euchre_email";
/// Header carrying the identity for non-browser clients.
pub const IDENTITY_HEADER: &str = "X-Euchre-Identity";
/// `iss` and `aud` of identity tokens.
pub const IDENTITY_ISSUER: &str = "euchre-identity";
/// `aud` of identity tokens.
pub const IDENTITY_AUDIENCE: &str = "euchre-identity";
/// `kind` claim of identity tokens.
pub const IDENTITY_KIND: &str = "email-nick";
/// Identity token lifetime (matches the 400-day cookie).
pub const IDENTITY_TTL_SECS: i64 = 400 * 86_400;
/// Longest accepted normalized email, in UTF-16 code units.
pub const MAX_EMAIL_LEN: usize = 254;

/// The identity a browser or device plays as (TS `Identity`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    /// Normalized email.
    pub email: String,
    /// `derive_user_id(email)`; the `playerProfile` key is `["user", user_id]`.
    pub user_id: String,
}

impl Identity {
    /// Normalize `raw` and derive its user id; `None` when it is not email-shaped.
    pub fn from_email(raw: &str) -> Option<Self> {
        let email = normalize_email(raw)?;
        let user_id = derive_user_id(&email);
        Some(Self { email, user_id })
    }
}

/// TS `normalizeEmail`: `raw.trim().toLowerCase()`, `None` when empty, longer than
/// 254 UTF-16 units, or not matching `^[^\s@]+@[^\s@]+\.[^\s@]+$`.
pub fn normalize_email(raw: &str) -> Option<String> {
    let normalized = js_trim(raw).to_lowercase();
    let len = js_len(&normalized);
    if len == 0 || len > MAX_EMAIL_LEN {
        return None;
    }
    email_shaped(&normalized).then_some(normalized)
}

/// The TS `EMAIL_PATTERN` without a regex engine: no JS whitespace, exactly one `@`,
/// a non-empty local part, and a domain with a `.` that is neither first nor last.
fn email_shaped(s: &str) -> bool {
    if s.chars().any(is_js_whitespace) {
        return false;
    }
    let mut parts = s.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if local.is_empty() {
        return false;
    }
    // `[^\s@]+\.[^\s@]+` on the domain: some '.' with at least one char either side.
    domain
        .char_indices()
        .any(|(i, c)| c == '.' && i > 0 && i + 1 < domain.len())
}

/// TS `deriveUserId`: lowercase hex SHA-256 of the normalized email's UTF-8 bytes.
pub fn derive_user_id(normalized_email: &str) -> String {
    hex::encode(Sha256::digest(normalized_email.as_bytes()))
}

#[derive(Serialize)]
struct IdentityClaims<'a> {
    kind: &'a str,
    sub: &'a str,
    em: &'a str,
    iss: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

/// Sign an identity token for `identity` issued at `iat` (unix seconds).
pub fn sign(key: &[u8], identity: &Identity, iat: i64) -> Result<String, AuthError> {
    sign_bare_hs256(
        key,
        &IdentityClaims {
            kind: IDENTITY_KIND,
            sub: &identity.user_id,
            em: &identity.email,
            iss: IDENTITY_ISSUER,
            aud: IDENTITY_AUDIENCE,
            iat,
            exp: iat + IDENTITY_TTL_SECS,
        },
    )
}

/// Verify an identity token at `now` (unix seconds): jose claim checks, then
/// `kind == "email-nick"`, `em` already normalized and `sub == derive_user_id(em)`.
pub fn verify(key: &[u8], token: &str, now: i64) -> Result<Identity, AuthError> {
    let claims = verify_jose_hs256(
        key,
        token,
        &Expect {
            issuer: IDENTITY_ISSUER,
            audience: IDENTITY_AUDIENCE,
        },
        now,
    )?;
    if claims.get("kind").and_then(|v| v.as_str()) != Some(IDENTITY_KIND) {
        return Err(AuthError::invalid("Invalid identity token kind"));
    }
    let (Some(em), Some(sub)) = (
        claims.get("em").and_then(|v| v.as_str()),
        claims.get("sub").and_then(|v| v.as_str()),
    ) else {
        return Err(AuthError::invalid("Invalid identity claims"));
    };
    match Identity::from_email(em) {
        Some(identity) if identity.email == em && identity.user_id == sub => Ok(identity),
        _ => Err(AuthError::invalid("Invalid identity claims")),
    }
}

/// Resolve a raw cookie or header value. A value with three `.`-separated segments is
/// a token and must verify; anything else is a legacy plain email, accepted only when
/// `allow_unsigned`. The legacy value is first percent-decoded with
/// [`decode_cookie_value`] (TS wrote it with `cookies.set`, which stores `ann%40b.co`,
/// and read it back through SvelteKit's decoding `cookies.get`), then normalized as
/// TS `readIdentity` does.
pub fn resolve(key: &[u8], value: &str, allow_unsigned: bool, now: i64) -> Option<Identity> {
    if value.split('.').count() == 3 && !value.contains('@') && !value.contains('%') {
        return verify(key, value, now).ok();
    }
    if allow_unsigned {
        Identity::from_email(&decode_cookie_value(value))
    } else {
        None
    }
}

/// Decode a cookie value exactly as cookie@2 `decode` (SvelteKit `cookies.get`) does:
/// a value without `%` is returned as is; otherwise it is `decodeURIComponent`d, and a
/// malformed escape or invalid UTF-8 returns the raw value unchanged (never an error).
pub fn decode_cookie_value(raw: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    if !raw.contains('%') {
        return Cow::Borrowed(raw);
    }
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            match hex {
                Some(byte) => {
                    out.push(byte);
                    i += 3;
                }
                None => return Cow::Borrowed(raw),
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    match String::from_utf8(out) {
        Ok(decoded) => Cow::Owned(decoded),
        Err(_) => Cow::Borrowed(raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"identity-key-identity-key-identity-key";
    const NOW: i64 = 1_800_000_000;

    #[test]
    fn normalize_matches_ts() {
        assert_eq!(
            normalize_email("  Ann@Example.COM \n").as_deref(),
            Some("ann@example.com")
        );
        assert_eq!(
            normalize_email("\u{FEFF}\u{00A0}a@b.co\u{2028}").as_deref(),
            Some("a@b.co")
        );
        for bad in [
            "",
            "   ",
            "a@b",
            "@b.co",
            "a@.co",
            "a@b.",
            "a@@b.co",
            "a b@c.co",
            "a@b@c.co",
            "a@b.c\u{00A0}d",
        ] {
            assert_eq!(normalize_email(bad), None, "{bad:?}");
        }
        assert!(
            normalize_email("a@b..").is_some(),
            "regex backtracks: '.' at i=1 of 'b..'"
        );
        let long = format!("{}@b.co", "a".repeat(249));
        assert!(normalize_email(&long).is_some());
        let too_long = format!("{}@b.co", "a".repeat(250));
        assert!(normalize_email(&too_long).is_none());
    }

    #[test]
    fn cookie_values_decode_like_cookie_2() {
        assert_eq!(decode_cookie_value("ann@example.com"), "ann@example.com");
        assert_eq!(decode_cookie_value("ann%40example.com"), "ann@example.com");
        assert_eq!(decode_cookie_value("%C3%A9@b.co"), "\u{e9}@b.co");
        assert_eq!(decode_cookie_value("a%2540b.co"), "a%40b.co");
        for raw in [
            "ann%zz@example.com",
            "ann%4",
            "ann%40example.com%E0",
            "%ED%A0%80",
        ] {
            assert_eq!(decode_cookie_value(raw), raw, "{raw:?} stays raw");
        }
    }

    #[test]
    fn legacy_cookie_is_percent_decoded() {
        let got = resolve(KEY, "Ann%40Example.COM", true, NOW);
        assert_eq!(got.map(|i| i.email).as_deref(), Some("ann@example.com"));
        assert!(resolve(KEY, "ann%40example.com", false, NOW).is_none());
        // A malformed escape stays raw, as cookie@2 leaves it (and it is a valid email).
        let raw = resolve(KEY, "ann%zz@example.com", true, NOW);
        assert_eq!(raw.map(|i| i.email).as_deref(), Some("ann%zz@example.com"));
    }

    #[test]
    fn derive_is_sha256_hex() {
        assert_eq!(
            derive_user_id("a@b.co"),
            hex::encode(Sha256::digest(b"a@b.co"))
        );
        assert_eq!(derive_user_id("x").len(), 64);
    }

    #[test]
    fn token_round_trip_and_resolve() -> Result<(), AuthError> {
        let id = Identity::from_email("Ann@Example.com").ok_or(AuthError::InvalidEmail)?;
        let t = sign(KEY, &id, NOW)?;
        assert_eq!(verify(KEY, &t, NOW + 10)?, id);
        assert!(verify(KEY, &t, NOW + IDENTITY_TTL_SECS).is_err());
        assert!(verify(b"wrong-key-wrong-key-wrong-key-wrong", &t, NOW).is_err());
        assert_eq!(resolve(KEY, &t, false, NOW), Some(id.clone()));
        assert_eq!(resolve(KEY, "ann@example.com", false, NOW), None);
        assert_eq!(resolve(KEY, "ann@example.com", true, NOW), Some(id.clone()));
        assert_eq!(resolve(KEY, "a.b.c", true, NOW), None);
        Ok(())
    }

    #[test]
    fn forged_claims_rejected() -> Result<(), AuthError> {
        #[derive(Serialize)]
        struct Raw<'a> {
            kind: &'a str,
            sub: &'a str,
            em: &'a str,
            iss: &'a str,
            aud: &'a str,
            exp: i64,
        }
        let forged = |kind: &str, sub: &str, em: &str| {
            sign_bare_hs256(
                KEY,
                &Raw {
                    kind,
                    sub,
                    em,
                    iss: IDENTITY_ISSUER,
                    aud: IDENTITY_AUDIENCE,
                    exp: NOW + 100,
                },
            )
        };
        let good_sub = derive_user_id("a@b.co");
        assert!(verify(KEY, &forged(IDENTITY_KIND, &good_sub, "a@b.co")?, NOW).is_ok());
        assert!(verify(KEY, &forged("guest", &good_sub, "a@b.co")?, NOW).is_err());
        assert!(verify(KEY, &forged(IDENTITY_KIND, "other", "a@b.co")?, NOW).is_err());
        assert!(verify(KEY, &forged(IDENTITY_KIND, &good_sub, "A@b.co")?, NOW).is_err());
        Ok(())
    }
}
