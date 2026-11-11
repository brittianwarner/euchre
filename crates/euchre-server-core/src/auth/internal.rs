//! Internal tokens (05 §2.6): the deployment `EUCHRE_INTERNAL_TOKEN` and the
//! per-match token that authorizes actor-to-actor messages.
//!
//! - [`assert_internal_token`] is TS `player-profile/auth.ts` `assertInternalToken`:
//!   one code (`internal_token_mismatch`) for every failure, constant-time compare
//!   over SHA-256 digests so the comparison does not leak the length either.
//! - [`match_token_ok`] is TS `euchre-table/queues.ts` `tokenOk`: false for an empty
//!   expected token or an absent one, length short-circuit (the token is a v4 UUID,
//!   its length is public), then a constant-time compare.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::AuthError;
use crate::config::Secret;

/// Message when no internal token is configured.
pub const NOT_CONFIGURED: &str =
    "EUCHRE_INTERNAL_TOKEN is not configured; internal writes are refused";
/// Message when the caller sent no (or an empty) token.
pub const MISSING: &str = "internal token missing";
/// Message when the token does not match.
pub const REJECTED: &str = "internal token rejected";

/// Check `token` against the deployment internal token. `expected` is `None` when
/// the deployment has none configured (every internal write is then refused).
pub fn assert_internal_token(
    expected: Option<&Secret>,
    token: Option<&str>,
) -> Result<(), AuthError> {
    let Some(expected) = expected.filter(|s| !s.expose().is_empty()) else {
        return Err(AuthError::InternalTokenMismatch(NOT_CONFIGURED));
    };
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return Err(AuthError::InternalTokenMismatch(MISSING));
    };
    let a = Sha256::digest(token.as_bytes());
    let b = Sha256::digest(expected.as_bytes());
    if bool::from(a.as_slice().ct_eq(b.as_slice())) {
        Ok(())
    } else {
        Err(AuthError::InternalTokenMismatch(REJECTED))
    }
}

/// Per-match token check (TS `tokenOk(expected, got)`), compared in UTF-16 code
/// units exactly as `charCodeAt` does.
pub fn match_token_ok(expected: &str, got: Option<&str>) -> bool {
    let Some(got) = got else { return false };
    if expected.is_empty() {
        return false;
    }
    let e: Vec<u16> = expected.encode_utf16().collect();
    let g: Vec<u16> = got.encode_utf16().collect();
    if e.len() != g.len() {
        return false;
    }
    let diff = e.iter().zip(&g).fold(0u16, |acc, (x, y)| acc | (x ^ y));
    diff == 0
}

/// A fresh per-match token: a lowercase v4 UUID (TS `crypto.randomUUID()`).
pub fn new_match_token() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assert_internal_token_messages() {
        let s = Secret::new("0123456789abcdef-internal");
        assert_eq!(
            assert_internal_token(Some(&s), Some("0123456789abcdef-internal")),
            Ok(())
        );
        assert_eq!(
            assert_internal_token(Some(&s), Some("0123456789abcdef-internaX")),
            Err(AuthError::InternalTokenMismatch(REJECTED))
        );
        assert_eq!(
            assert_internal_token(Some(&s), Some("short")),
            Err(AuthError::InternalTokenMismatch(REJECTED))
        );
        assert_eq!(
            assert_internal_token(Some(&s), Some("")),
            Err(AuthError::InternalTokenMismatch(MISSING))
        );
        assert_eq!(
            assert_internal_token(Some(&s), None),
            Err(AuthError::InternalTokenMismatch(MISSING))
        );
        assert_eq!(
            assert_internal_token(None, Some("x")),
            Err(AuthError::InternalTokenMismatch(NOT_CONFIGURED))
        );
        assert_eq!(
            AuthError::InternalTokenMismatch(REJECTED).code(),
            "internal_token_mismatch"
        );
    }

    #[test]
    fn match_token_semantics() {
        let t = new_match_token();
        assert_eq!(t.len(), 36);
        assert!(match_token_ok(&t, Some(&t)));
        assert!(!match_token_ok(&t, None));
        assert!(!match_token_ok("", Some("")));
        assert!(!match_token_ok(&t, Some(&t[..35])));
        assert!(!match_token_ok(&t, Some(&new_match_token())));
        assert!(match_token_ok("é", Some("é")));
        assert!(!match_token_ok("é", Some("e")));
    }
}
