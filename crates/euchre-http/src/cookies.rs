//! The three cookies (05 §11.3), byte-compatible with the SvelteKit server so an old
//! and a new server can share a browser session during cutover.
//!
//! SvelteKit's `cookies.set` URI-encodes values and `cookies.get` decodes them, so
//! the recent-tables list is written as `a%2Cb` and read back through
//! [`decode_cookie_value`].

use axum::http::{HeaderMap, HeaderValue, header};
use cookie::{Cookie, SameSite};
use euchre_protocol::is_game_id;
use euchre_server_core::auth::identity::decode_cookie_value;

/// Guest session JWT (`aud: euchre-session`).
pub const GUEST_COOKIE: &str = "euchre_guest";
/// Up to 12 comma-joined lowercase table UUIDs, newest first.
pub const RECENT_TABLES_COOKIE: &str = "euchre_recent_tables";
/// The signed identity token (or a legacy plain email).
pub const IDENTITY_COOKIE: &str = euchre_server_core::auth::identity::IDENTITY_COOKIE;
/// `400 * 86400` seconds.
pub const COOKIE_MAX_AGE_SECS: i64 = 34_560_000;
/// The recent-tables list keeps at most this many ids.
pub const MAX_RECENT_TABLES: usize = 12;

/// The first value of cookie `name` across every `Cookie` header, raw (undecoded).
pub fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().to_owned())
}

/// `Set-Cookie` for `name=value` with the shared attributes:
/// `Path=/; HttpOnly; SameSite=Lax; Max-Age=34560000` and `Secure` when `secure`.
/// Host-only: no `Domain`.
pub fn set_cookie(name: &'static str, value: String, secure: bool) -> Option<HeaderValue> {
    let cookie = Cookie::build((name, value))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .max_age(cookie::time::Duration::seconds(COOKIE_MAX_AGE_SECS))
        .build();
    HeaderValue::from_str(&cookie.to_string()).ok()
}

/// `Set-Cookie` that deletes `name` (SvelteKit `cookies.delete(name, {path: '/'})`).
pub fn clear_cookie(name: &'static str, secure: bool) -> Option<HeaderValue> {
    let mut cookie = Cookie::build((name, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .build();
    cookie.make_removal();
    HeaderValue::from_str(&cookie.to_string()).ok()
}

/// TS `recentTableIds`: split on `,`, keep valid UUIDs, at most 12.
pub fn recent_table_ids(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    decode_cookie_value(raw)
        .split(',')
        .filter(|id| is_game_id(id))
        .take(MAX_RECENT_TABLES)
        .map(str::to_owned)
        .collect()
}

/// TS `rememberTable`: the new cookie value with `game_id` first, or `None` when the
/// id is not a valid UUID.
pub fn remember_table(existing: &[String], game_id: &str) -> Option<String> {
    if !is_game_id(game_id) {
        return None;
    }
    let ids: Vec<&str> = std::iter::once(game_id)
        .chain(
            existing
                .iter()
                .map(String::as_str)
                .filter(|id| *id != game_id),
        )
        .take(MAX_RECENT_TABLES)
        .collect();
    // encodeURIComponent(',') === '%2C'; UUIDs need no other escaping.
    Some(ids.join("%2C"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "11111111-1111-4111-8111-111111111111";
    const B: &str = "22222222-2222-4222-8222-222222222222";

    #[test]
    fn set_cookie_carries_every_attribute() -> anyhow::Result<()> {
        let value = set_cookie(GUEST_COOKIE, "tok".into(), true)
            .ok_or_else(|| anyhow::anyhow!("header"))?;
        let text = value.to_str()?;
        assert!(text.starts_with("euchre_guest=tok"));
        for part in [
            "HttpOnly",
            "SameSite=Lax",
            "Secure",
            "Path=/",
            "Max-Age=34560000",
        ] {
            assert!(text.contains(part), "{text} lacks {part}");
        }
        assert!(!text.contains("Domain"));
        let insecure = set_cookie(GUEST_COOKIE, "tok".into(), false)
            .ok_or_else(|| anyhow::anyhow!("header"))?;
        assert!(!insecure.to_str()?.contains("Secure"));
        Ok(())
    }

    #[test]
    fn clear_cookie_expires_it() -> anyhow::Result<()> {
        let value =
            clear_cookie(IDENTITY_COOKIE, false).ok_or_else(|| anyhow::anyhow!("header"))?;
        let text = value.to_str()?;
        assert!(text.starts_with("euchre_email=;"));
        assert!(text.contains("Max-Age=0"));
        assert!(text.contains("Path=/"));
        Ok(())
    }

    #[test]
    fn read_cookie_scans_every_header() -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();
        headers.append(header::COOKIE, "a=1; euchre_guestx=no".parse()?);
        headers.append(header::COOKIE, "euchre_guest=yes; b=2".parse()?);
        assert_eq!(read_cookie(&headers, GUEST_COOKIE).as_deref(), Some("yes"));
        assert_eq!(read_cookie(&headers, "missing"), None);
        Ok(())
    }

    #[test]
    fn recent_tables_round_trip_svelte_encoding() {
        let raw = remember_table(&[], A).unwrap_or_default();
        assert_eq!(raw, A);
        let raw = remember_table(&recent_table_ids(Some(&raw)), B).unwrap_or_default();
        assert_eq!(raw, format!("{B}%2C{A}"));
        assert_eq!(recent_table_ids(Some(&raw)), vec![B, A]);
        // Re-remembering moves an id to the front without duplicating it.
        let raw = remember_table(&recent_table_ids(Some(&raw)), A).unwrap_or_default();
        assert_eq!(recent_table_ids(Some(&raw)), vec![A, B]);
        // A plain comma (an older writer) also parses; junk is dropped.
        assert_eq!(recent_table_ids(Some(&format!("{A},junk,{B}"))), vec![A, B]);
        assert_eq!(remember_table(&[], "not-a-uuid"), None);
    }

    #[test]
    fn recent_tables_cap_at_twelve() {
        let ids: Vec<String> = (0..20)
            .map(|i| format!("{i:08x}-0000-4000-8000-000000000000"))
            .collect();
        let raw = ids.join(",");
        assert_eq!(recent_table_ids(Some(&raw)).len(), MAX_RECENT_TABLES);
        let next = remember_table(&ids, A).unwrap_or_default();
        assert_eq!(recent_table_ids(Some(&next)).len(), MAX_RECENT_TABLES);
        assert_eq!(recent_table_ids(Some(&next))[0], A);
    }
}
