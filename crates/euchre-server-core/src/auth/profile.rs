//! Profile JWT (05 §2.5): hand-rolled HS256, minted by the server for `playerProfile`.
//!
//! - Mint (TS `session.server.ts` `mintProfileToken`): header exactly
//!   `{"alg":"HS256","typ":"JWT"}`, payload `{"sub","aud":"euchre-actors","iat","exp"}`
//!   in that order, `exp = iat + 1800`; base64url without padding. When
//!   `EUCHRE_JWT_ISSUER` is set the Rust minter appends `"iss"` (TS never sets it,
//!   which made the TS issuer check reject every token).
//! - Verify (TS `player-profile/auth.ts` `verifyProfileToken`): the exact check order
//!   and messages of TS, including its forgiving base64 decoder (`atob`) and lossy
//!   UTF-8 decoding, so a token is accepted here iff TS accepts it.

use hmac::{Hmac, Mac};
use serde_json::{Map, Value};
use sha2::Sha256;

use super::AuthError;
use crate::config::js_len;

/// `aud` of profile tokens (shared with guest actor tokens; `iss`/`kind` tell them apart).
pub const PROFILE_AUDIENCE: &str = "euchre-actors";
/// Lifetime of minted profile tokens (TS `TOKEN_TTL_SECONDS`).
pub const PROFILE_TTL_SECS: i64 = 30 * 60;
/// Clock skew tolerated by the verifier (TS `CLOCK_SKEW_MS`).
pub const CLOCK_SKEW_MS: f64 = 60_000.0;
/// Longest accepted `exp - iat` (TS `MAX_TOKEN_LIFETIME_MS`).
pub const MAX_TOKEN_LIFETIME_MS: f64 = 3_600_000.0;
/// Longest accepted `sub`, in UTF-16 code units.
pub const MAX_SUB_LEN: usize = 128;

/// `base64url('{"alg":"HS256","typ":"JWT"}')`, the constant header segment.
pub const PROFILE_HEADER_SEGMENT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";

/// What a verified profile token establishes (TS `VerifiedClaims`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileClaims {
    /// The `sub`: compared against the actor key, never used to select one.
    pub user_id: String,
    /// `iat * 1000`, or the verification time when `iat` is absent (ms).
    pub issued_at_ms: f64,
    /// `exp * 1000` (ms).
    pub expires_at_ms: f64,
}

type HmacSha256 = Hmac<Sha256>;

/// Mint a profile token for `user_id` issued at `iat` (unix seconds).
pub fn mint(
    key: &[u8],
    user_id: &str,
    iat: i64,
    issuer: Option<&str>,
) -> Result<String, AuthError> {
    let mut payload = Map::new();
    payload.insert("sub".into(), Value::from(user_id));
    payload.insert("aud".into(), Value::from(PROFILE_AUDIENCE));
    payload.insert("iat".into(), Value::from(iat));
    payload.insert("exp".into(), Value::from(iat + PROFILE_TTL_SECS));
    if let Some(iss) = issuer {
        payload.insert("iss".into(), Value::from(iss));
    }
    let json = serde_json::to_vec(&Value::Object(payload))
        .map_err(|e| AuthError::Signing(e.to_string()))?;
    let signing_input = format!("{PROFILE_HEADER_SEGMENT}.{}", b64url(&json));
    let mut mac = HmacSha256::new_from_slice(key).map_err(|e| AuthError::Signing(e.to_string()))?;
    mac.update(signing_input.as_bytes());
    let sig = mac.finalize().into_bytes();
    Ok(format!("{signing_input}.{}", b64url(&sig)))
}

/// Verify a profile token at `now_ms` (unix milliseconds). Every failure is
/// `invalid_token` with the TS message.
pub fn verify(
    key: &[u8],
    token: &str,
    now_ms: i64,
    issuer: Option<&str>,
) -> Result<ProfileClaims, AuthError> {
    let now = now_ms as f64;
    let parts: Vec<&str> = token.split('.').collect();
    let [header_seg, payload_seg, signature_seg] = parts.as_slice() else {
        return Err(AuthError::invalid("malformed token"));
    };
    let (header, payload) = match (
        decode_json_segment(header_seg),
        decode_json_segment(payload_seg),
    ) {
        (Some(h), Some(p)) => (h, p),
        _ => return Err(AuthError::invalid("token segments are not JSON")),
    };
    let Value::Object(header) = header else {
        return Err(AuthError::invalid("token header is not an object"));
    };
    let Value::Object(claims) = payload else {
        return Err(AuthError::invalid("token payload is not an object"));
    };
    if header.get("alg").and_then(Value::as_str) != Some("HS256") {
        return Err(AuthError::invalid("unsupported token algorithm"));
    }
    let Some(signature) = forgiving_base64url_decode(signature_seg) else {
        return Err(AuthError::invalid("malformed signature"));
    };
    let mut mac = HmacSha256::new_from_slice(key).map_err(|e| AuthError::Signing(e.to_string()))?;
    mac.update(header_seg.as_bytes());
    mac.update(b".");
    mac.update(payload_seg.as_bytes());
    if mac.verify_slice(&signature).is_err() {
        return Err(AuthError::invalid("signature rejected"));
    }
    let aud_ok = match claims.get("aud") {
        Some(Value::String(aud)) => aud == PROFILE_AUDIENCE,
        Some(Value::Array(items)) => items.iter().any(|v| v.as_str() == Some(PROFILE_AUDIENCE)),
        _ => false,
    };
    if !aud_ok {
        return Err(AuthError::invalid("wrong audience"));
    }
    if let Some(expected) = issuer.filter(|i| !i.is_empty())
        && claims.get("iss").and_then(Value::as_str) != Some(expected)
    {
        return Err(AuthError::invalid("wrong issuer"));
    }
    let Some(exp) = claim_number(&claims, "exp") else {
        return Err(AuthError::invalid("token has no expiry"));
    };
    let expires_at = exp * 1000.0;
    if expires_at + CLOCK_SKEW_MS < now {
        return Err(AuthError::invalid("token expired"));
    }
    if let Some(nbf) = claim_number(&claims, "nbf")
        && nbf * 1000.0 - CLOCK_SKEW_MS > now
    {
        return Err(AuthError::invalid("token not yet valid"));
    }
    let iat = claim_number(&claims, "iat");
    let issued_at = iat.map_or(now, |iat| iat * 1000.0);
    if iat.is_some() {
        if issued_at - CLOCK_SKEW_MS > now {
            return Err(AuthError::invalid("token issued in the future"));
        }
        if expires_at - issued_at > MAX_TOKEN_LIFETIME_MS {
            return Err(AuthError::invalid("token lifetime exceeds the maximum"));
        }
    }
    match claims.get("sub") {
        Some(Value::String(sub)) if (1..=MAX_SUB_LEN).contains(&js_len(sub)) => Ok(ProfileClaims {
            user_id: sub.clone(),
            issued_at_ms: issued_at,
            expires_at_ms: expires_at,
        }),
        _ => Err(AuthError::invalid("token subject is missing or unusable")),
    }
}

/// A finite JSON number claim (TS `claimNumber`).
fn claim_number(claims: &Map<String, Value>, key: &str) -> Option<f64> {
    claims
        .get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
}

/// TS `decodeJwtSegment`: forgiving base64url → lossy UTF-8 (BOM stripped) → JSON.
fn decode_json_segment(segment: &str) -> Option<Value> {
    let bytes = forgiving_base64url_decode(segment)?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    serde_json::from_str(text).ok()
}

/// Base64url without padding (TS `base64url`).
pub fn b64url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// TS `base64UrlToBytes`: map `-`/`_` to `+`/`/`, pad to a multiple of 4, then the
/// WHATWG forgiving-base64 decode that `atob` performs (ASCII whitespace removed,
/// up to two trailing `=` dropped, non-zero trailing bits ignored). `None` where
/// `atob` throws.
pub fn forgiving_base64url_decode(segment: &str) -> Option<Vec<u8>> {
    let mut s: String = segment
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            other => other,
        })
        .collect();
    let rem = js_len(&s) % 4;
    if rem != 0 {
        s.extend(std::iter::repeat_n('=', 4 - rem));
    }
    // atob: strip ASCII whitespace (TAB, LF, FF, CR, SPACE).
    let mut data: Vec<u8> = Vec::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\t' | '\n' | '\u{000C}' | '\r' | ' ') {
            continue;
        }
        if !c.is_ascii() {
            return None;
        }
        data.push(c as u8);
    }
    if data.len().is_multiple_of(4) {
        if data.ends_with(b"==") {
            data.truncate(data.len() - 2);
        } else if data.ends_with(b"=") {
            data.truncate(data.len() - 1);
        }
    }
    if data.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for &b in &data {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"0123456789abcdef-profile";
    const IAT: i64 = 1_800_000_000;
    const NOW_MS: i64 = IAT * 1000 + 5_000;

    fn forge(header: &str, payload: &str) -> String {
        let h = b64url(header.as_bytes());
        let p = b64url(payload.as_bytes());
        let mut mac = HmacSha256::new_from_slice(KEY).unwrap_or_else(|_| unreachable!());
        mac.update(format!("{h}.{p}").as_bytes());
        format!("{h}.{p}.{}", b64url(&mac.finalize().into_bytes()))
    }

    fn reason(r: Result<ProfileClaims, AuthError>) -> String {
        match r {
            Ok(c) => format!("ok:{}", c.user_id),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn header_segment_constant_is_ts_header() {
        assert_eq!(
            b64url(br#"{"alg":"HS256","typ":"JWT"}"#),
            PROFILE_HEADER_SEGMENT
        );
    }

    #[test]
    fn mint_then_verify() -> Result<(), AuthError> {
        let t = mint(KEY, "abc", IAT, None)?;
        let c = verify(KEY, &t, NOW_MS, None)?;
        assert_eq!(c.user_id, "abc");
        assert_eq!(c.issued_at_ms, (IAT * 1000) as f64);
        assert_eq!(c.expires_at_ms, ((IAT + 1800) * 1000) as f64);
        let with_iss = mint(KEY, "abc", IAT, Some("euchre"))?;
        assert!(verify(KEY, &with_iss, NOW_MS, Some("euchre")).is_ok());
        assert_eq!(
            reason(verify(KEY, &t, NOW_MS, Some("euchre"))),
            "wrong issuer"
        );
        Ok(())
    }

    #[test]
    fn golden_rejections_05_14_3() {
        let h = r#"{"alg":"HS256","typ":"JWT"}"#;
        let cases: Vec<(String, &str)> = vec![
            (
                forge(
                    r#"{"alg":"none"}"#,
                    r#"{"sub":"a","aud":"euchre-actors","exp":1800001800}"#,
                ),
                "unsupported token algorithm",
            ),
            (
                forge(
                    h,
                    r#"{"sub":"a","aud":"euchre-session","iat":1800000000,"exp":1800001800}"#,
                ),
                "wrong audience",
            ),
            (
                forge(h, r#"{"sub":"a","aud":"euchre-actors","exp":1799999939}"#),
                "token expired",
            ),
            (
                forge(
                    h,
                    r#"{"sub":"a","aud":"euchre-actors","nbf":1800000066,"exp":1800001800}"#,
                ),
                "token not yet valid",
            ),
            (
                forge(
                    h,
                    r#"{"sub":"a","aud":"euchre-actors","iat":1800000066,"exp":1800001800}"#,
                ),
                "token issued in the future",
            ),
            (
                forge(
                    h,
                    r#"{"sub":"a","aud":"euchre-actors","iat":1800000000,"exp":1800003601}"#,
                ),
                "token lifetime exceeds the maximum",
            ),
            (
                forge(
                    h,
                    &format!(
                        r#"{{"sub":"{}","aud":"euchre-actors","exp":1800001800}}"#,
                        "s".repeat(129)
                    ),
                ),
                "token subject is missing or unusable",
            ),
            (
                forge(h, r#"{"sub":"","aud":"euchre-actors","exp":1800001800}"#),
                "token subject is missing or unusable",
            ),
            (
                forge(h, r#"{"aud":"euchre-actors","exp":1800001800}"#),
                "token subject is missing or unusable",
            ),
            (
                forge(h, r#"{"sub":"a","aud":"euchre-actors"}"#),
                "token has no expiry",
            ),
            (forge(h, "[1]"), "token payload is not an object"),
            (forge("null", "{}"), "token header is not an object"),
            ("a.b".to_owned(), "malformed token"),
            ("a.b.c.d".to_owned(), "malformed token"),
            ("%%%.e30.sig".to_owned(), "token segments are not JSON"),
        ];
        for (token, want) in cases {
            assert_eq!(
                reason(verify(KEY, &token, NOW_MS, None)),
                want,
                "token {token}"
            );
        }
        let ok = forge(
            h,
            r#"{"sub":"a","aud":["x","euchre-actors"],"iat":1800000000,"exp":1800003600}"#,
        );
        assert_eq!(reason(verify(KEY, &ok, NOW_MS, None)), "ok:a");
        let skew_ok = forge(h, r#"{"sub":"a","aud":"euchre-actors","exp":1799999945}"#);
        assert_eq!(
            reason(verify(KEY, &skew_ok, NOW_MS, None)),
            "ok:a",
            "within 60 s skew"
        );
    }

    #[test]
    fn signature_checks() -> Result<(), AuthError> {
        let t = mint(KEY, "abc", IAT, None)?;
        assert_eq!(
            reason(verify(b"other-key-other-key", &t, NOW_MS, None)),
            "signature rejected"
        );
        let (head, _) = t.rsplit_once('.').unwrap_or_default();
        assert_eq!(
            reason(verify(KEY, &format!("{head}.*"), NOW_MS, None)),
            "malformed signature"
        );
        assert_eq!(
            reason(verify(KEY, &format!("{head}.AAAA"), NOW_MS, None)),
            "signature rejected"
        );
        Ok(())
    }

    #[test]
    fn forgiving_decoder_matches_atob() {
        assert_eq!(forgiving_base64url_decode("e30"), Some(b"{}".to_vec()));
        assert_eq!(forgiving_base64url_decode("e30="), Some(b"{}".to_vec()));
        assert_eq!(forgiving_base64url_decode("e3 0"), Some(b"{}".to_vec()));
        assert_eq!(
            forgiving_base64url_decode("e31"),
            Some(b"{}".to_vec()),
            "trailing bits ignored"
        );
        assert_eq!(forgiving_base64url_decode("a"), None);
        assert_eq!(forgiving_base64url_decode("ab=c"), None);
        assert_eq!(forgiving_base64url_decode(""), Some(Vec::new()));
        assert_eq!(forgiving_base64url_decode("_-8"), Some(vec![0xFF, 0xEF]));
        assert_eq!(forgiving_base64url_decode("/+8"), Some(vec![0xFF, 0xEF]));
    }
}
