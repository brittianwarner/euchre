//! Cross-language auth parity (lane SERVER-FOUNDATION).
//!
//! The fixtures in `tests/fixtures/auth/` are written by
//! `bun crates/euchre-server-core/scripts/gen-auth-fixtures.ts`, which runs the real TS auth code with
//! fixed secrets and a fixed clock. Every case is replayed here against the Rust port.
//!
//! `tests/fixtures/auth/rust-minted.json` goes the other way: this test asserts it
//! matches what Rust mints (regenerate with `EUCHRE_WRITE_RUST_TOKENS=1 cargo test -p
//! euchre-server --test foundation_auth`), and `bun crates/euchre-server-core/scripts/verify-rust-tokens.ts`
//! verifies those tokens with the TS verifiers.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use euchre_server_core::auth::guest::{self, GuestAudience};
use euchre_server_core::auth::identity::{self, Identity};
use euchre_server_core::auth::{AuthError, device, internal, profile};
use euchre_server_core::config::Secret;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/auth")
}

fn load(file: &str) -> Result<Value> {
    let path = fixtures_dir().join(file);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

fn s<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("`{key}` is not a string in {v}"))
}

fn i(v: &Value, key: &str) -> Result<i64> {
    v.get(key)
        .and_then(Value::as_i64)
        .with_context(|| format!("`{key}` is not an integer in {v}"))
}

fn b(v: &Value, key: &str) -> Result<bool> {
    v.get(key)
        .and_then(Value::as_bool)
        .with_context(|| format!("`{key}` is not a bool in {v}"))
}

fn arr<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    v.get(key)
        .and_then(Value::as_array)
        .with_context(|| format!("`{key}` is not an array"))
}

fn audience(name: &str) -> Result<GuestAudience> {
    match name {
        "euchre-session" => Ok(GuestAudience::Session),
        "euchre-actors" => Ok(GuestAudience::Actors),
        other => bail!("unknown audience {other}"),
    }
}

/// HS256-sign `header.payload` the way the fixture script's `forge` does.
fn forge(key: &[u8], header: &str, payload: &Value) -> Result<String> {
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload)?)
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(key)?;
    mac.update(input.as_bytes());
    Ok(format!(
        "{input}.{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}

/// jose messages Rust reproduces verbatim (claim checks). Signature, algorithm and
/// encoding failures come from jsonwebtoken and only need to be rejections.
fn guest_message_must_match(ts_error: &str) -> bool {
    ts_error.contains("claim")
        || ts_error.contains("Header Parameter")
        || ts_error == "Invalid guest identity"
        || ts_error == "Invalid Compact JWS"
}

#[test]
fn guest_tokens_match_ts() -> Result<()> {
    let f = load("guest.json")?;
    let key = s(&f, "actor_secret")?.as_bytes();

    for m in arr(&f, "minted")? {
        let aud = audience(s(m, "aud")?)?;
        let rust = guest::sign(key, s(m, "user_id")?, aud, i(m, "iat")?)?;
        assert_eq!(
            rust,
            s(m, "token")?,
            "Rust guest {aud:?} token is byte-identical to TS signGuest"
        );
    }

    let mut failures = Vec::new();
    for c in arr(&f, "cases")? {
        let name = s(c, "name")?;
        let got = guest::verify(key, s(c, "token")?, audience(s(c, "aud")?)?, i(c, "now")?);
        let want_ok = b(c, "rust_ok")?;
        match (&got, want_ok) {
            (Ok(sub), true) => {
                if let Some(ts_sub) = c.get("ts_sub").and_then(Value::as_str)
                    && ts_sub != sub
                {
                    failures.push(format!("{name}: sub {sub} != TS {ts_sub}"));
                }
            }
            (Err(e), false) => {
                if c.get("deviation").is_none()
                    && let Some(ts_error) = c.get("ts_error").and_then(Value::as_str)
                    && guest_message_must_match(ts_error)
                    && e.to_string() != ts_error
                {
                    failures.push(format!("{name}: message `{e}` != TS `{ts_error}`"));
                }
            }
            (got, _) => failures.push(format!("{name}: expected ok={want_ok}, got {got:?}")),
        }
    }
    ensure!(
        failures.is_empty(),
        "guest parity failures:\n{}",
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn guest_rejects_the_classic_attacks() -> Result<()> {
    let f = load("guest.json")?;
    let key = s(&f, "actor_secret")?.as_bytes();
    let names: Vec<&str> = arr(&f, "cases")?
        .iter()
        .filter_map(|c| c.get("name")?.as_str())
        .collect();
    for required in [
        "guest_tampered_payload",
        "guest_tampered_signature",
        "guest_actors_expired_at_exp",
        "guest_session_as_actors",
        "guest_wrong_issuer",
        "guest_alg_none",
    ] {
        assert!(names.contains(&required), "fixture covers {required}");
    }
    // The table seat path refuses the dev placeholder and session tokens.
    let now = i(&f, "now_secs")?;
    let session = guest::sign(key, "guest-x", GuestAudience::Session, now)?;
    assert!(guest::verify_player(key, &session, now).is_err());
    assert!(guest::verify_player(key, "dev", now).is_err());
    assert!(guest::verify_player(key, "", now).is_err());
    Ok(())
}

#[test]
fn profile_tokens_match_ts() -> Result<()> {
    let f = load("profile.json")?;
    let key = s(&f, "actor_secret")?.as_bytes();

    for m in arr(&f, "minted")? {
        let rust = profile::mint(key, s(m, "user_id")?, i(m, "iat")?, None)?;
        assert_eq!(
            rust,
            s(m, "token")?,
            "Rust profile token is byte-identical to TS mintProfileToken"
        );
    }

    let mut failures = Vec::new();
    for c in arr(&f, "cases")? {
        let name = s(c, "name")?;
        let issuer = c.get("issuer").and_then(Value::as_str);
        let got = profile::verify(key, s(c, "token")?, i(c, "now_ms")?, issuer);
        match (&got, b(c, "ok")?) {
            (Ok(claims), true) => {
                let want_user = s(c, "user_id")?;
                let want_iat = c.get("issued_at_ms").and_then(Value::as_f64);
                let want_exp = c.get("expires_at_ms").and_then(Value::as_f64);
                if claims.user_id != want_user
                    || Some(claims.issued_at_ms) != want_iat
                    || Some(claims.expires_at_ms) != want_exp
                {
                    failures.push(format!(
                        "{name}: {claims:?} != TS {want_user} {want_iat:?} {want_exp:?}"
                    ));
                }
            }
            (Err(e), false) => {
                let ts = s(c, "error")?;
                if e.to_string() != ts {
                    failures.push(format!("{name}: message `{e}` != TS `{ts}`"));
                }
                if e.code() != "invalid_token" {
                    failures.push(format!("{name}: code {} is not invalid_token", e.code()));
                }
            }
            (got, want) => failures.push(format!("{name}: expected ok={want}, got {got:?}")),
        }
    }
    ensure!(
        failures.is_empty(),
        "profile parity failures:\n{}",
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn email_normalization_matches_ts() -> Result<()> {
    let f = load("identity.json")?;
    let mut failures = Vec::new();
    for e in arr(&f, "emails")? {
        let raw = s(e, "raw")?;
        let want = e.get("normalized").and_then(Value::as_str);
        let got = identity::normalize_email(raw);
        if got.as_deref() != want {
            failures.push(format!("{raw:?}: {got:?} != TS {want:?}"));
            continue;
        }
        if let (Some(n), Some(uid)) = (got.as_deref(), e.get("user_id").and_then(Value::as_str))
            && identity::derive_user_id(n) != uid
        {
            failures.push(format!("{raw:?}: user id differs from TS deriveUserId"));
        }
    }
    ensure!(
        failures.is_empty(),
        "email parity failures:\n{}",
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn legacy_cookies_decode_like_sveltekit() -> Result<()> {
    let f = load("identity.json")?;
    let key = s(&f, "identity_secret")?.as_bytes();
    let now = i(&f, "now_secs")?;
    let mut failures = Vec::new();
    for c in arr(&f, "legacy_cookies")? {
        let raw = s(c, "raw")?;
        let decoded = identity::decode_cookie_value(raw);
        if Some(decoded.as_ref()) != c.get("decoded").and_then(Value::as_str) {
            failures.push(format!(
                "{raw:?}: decoded {decoded:?} != TS {:?}",
                c.get("decoded")
            ));
        }
        let want_email = c.get("email").and_then(Value::as_str);
        let want_uid = c.get("user_id").and_then(Value::as_str);
        let got = identity::resolve(key, raw, true, now);
        let got_pair = got.as_ref().map(|g| (g.email.as_str(), g.user_id.as_str()));
        if got_pair != want_email.zip(want_uid) {
            failures.push(format!(
                "{raw:?}: resolved {got_pair:?} != TS {want_email:?}"
            ));
        }
        if identity::resolve(key, raw, false, now).is_some() {
            failures.push(format!("{raw:?}: accepted with unsigned cookies off"));
        }
    }
    ensure!(
        failures.is_empty(),
        "legacy cookie parity failures:\n{}",
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn identity_token_matches_jose_and_rejects_variants() -> Result<()> {
    let f = load("identity.json")?;
    let key = s(&f, "identity_secret")?.as_bytes();
    let now = i(&f, "now_secs")?;
    let m = arr(&f, "minted")?
        .first()
        .context("one minted identity token")?;
    let id = Identity::from_email(s(m, "email")?).context("fixture email normalizes")?;
    assert_eq!(id.user_id, s(m, "user_id")?);

    let rust = identity::sign(key, &id, i(m, "iat")?)?;
    assert_eq!(
        rust,
        s(m, "token")?,
        "Rust identity token is byte-identical to jose SignJWT"
    );
    assert_eq!(identity::verify(key, s(m, "token")?, now + 1)?, id);
    assert_eq!(
        identity::resolve(key, s(m, "token")?, false, now + 1),
        Some(id.clone())
    );
    assert_eq!(
        identity::resolve(key, "Ann@Example.com", false, now),
        None,
        "unsigned refused"
    );
    assert_eq!(
        identity::resolve(key, "Ann@Example.com", true, now),
        Some(id.clone())
    );

    let exp = now + identity::IDENTITY_TTL_SECS;
    let base = json!({
        "kind": "email-nick", "sub": id.user_id, "em": id.email,
        "iss": "euchre-identity", "aud": "euchre-identity", "iat": now, "exp": exp,
    });
    let with = |k: &str, v: Value| -> Value {
        let mut p = base.clone();
        if let Some(o) = p.as_object_mut() {
            o.insert(k.to_owned(), v);
        }
        p
    };
    let h = r#"{"alg":"HS256"}"#;
    assert_eq!(
        forge(key, h, &base)?,
        rust,
        "forge reproduces the minted bytes"
    );
    let token = s(m, "token")?;
    let mut parts = token.split('.');
    let (head, body, sig) = (
        parts.next().context("header")?,
        parts.next().context("payload")?,
        parts.next().context("signature")?,
    );
    let tampered_payload = format!(
        "{head}.{}.{sig}",
        URL_SAFE_NO_PAD.encode(with("em", json!("eve@example.com")).to_string())
    );
    let tampered_sig = format!(
        "{head}.{body}.{}{}",
        if sig.starts_with('A') { "B" } else { "A" },
        &sig[1..]
    );
    let alg_none = format!("{}.{body}.", URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#));
    let rejected: Vec<(&str, String, i64)> = vec![
        ("tampered payload", tampered_payload, now),
        ("tampered signature", tampered_sig, now),
        ("alg none", alg_none, now),
        ("expired", token.to_owned(), exp),
        (
            "wrong aud",
            forge(key, h, &with("aud", json!("euchre-actors")))?,
            now,
        ),
        (
            "wrong iss",
            forge(key, h, &with("iss", json!("euchre-guest")))?,
            now,
        ),
        (
            "wrong kind",
            forge(key, h, &with("kind", json!("guest")))?,
            now,
        ),
        (
            "sub not derived from em",
            forge(key, h, &with("sub", json!("0".repeat(64))))?,
            now,
        ),
        (
            "em not normalized",
            forge(key, h, &with("em", json!("Ann@Example.com")))?,
            now,
        ),
        (
            "wrong key",
            forge(b"another-identity-secret-0123456789", h, &base)?,
            now,
        ),
        (
            "guest token",
            guest::sign(key, "guest-x", GuestAudience::Session, now)?,
            now,
        ),
    ];
    for (what, token, at) in rejected {
        assert!(
            matches!(
                identity::verify(key, &token, at),
                Err(AuthError::InvalidToken(_))
            ),
            "identity token with {what} is rejected"
        );
        assert_eq!(
            identity::resolve(key, &token, true, at),
            None,
            "{what} does not resolve"
        );
    }
    Ok(())
}

#[test]
fn internal_token_and_match_token_match_ts() -> Result<()> {
    let f = load("internal.json")?;
    let expected = Secret::new(s(&f, "internal_token")?);
    for c in arr(&f, "cases")? {
        let name = s(c, "name")?;
        let token = c.get("token").and_then(Value::as_str);
        let got = internal::assert_internal_token(Some(&expected), token);
        if b(c, "ok")? {
            assert!(got.is_ok(), "{name}: {got:?}");
        } else {
            let e = got
                .err()
                .with_context(|| format!("{name} must be refused"))?;
            assert_eq!(e.to_string(), s(c, "error")?, "{name}");
            assert_eq!(e.code(), "internal_token_mismatch", "{name}");
        }
    }
    let unconfigured = internal::assert_internal_token(None, Some("anything"));
    assert_eq!(
        unconfigured.err().map(|e| e.to_string()).as_deref(),
        Some(internal::NOT_CONFIGURED)
    );

    for c in arr(&f, "match_cases")? {
        let got = internal::match_token_ok(s(c, "expected")?, c.get("got").and_then(Value::as_str));
        assert_eq!(got, b(c, "ok")?, "tokenOk({c})");
    }
    Ok(())
}

/// Deterministic Rust-minted tokens for `scripts/verify-rust-tokens.ts`.
fn rust_minted() -> Result<Value> {
    let g = load("guest.json")?;
    let id_f = load("identity.json")?;
    let actor = s(&g, "actor_secret")?.as_bytes();
    let id_key = s(&id_f, "identity_secret")?.as_bytes();
    let now = i(&g, "now_secs")?;
    let uid = "guest-0b0c3f6e-7d1a-4c2b-9e8f-1a2b3c4d5e6f";

    let session = guest::sign(actor, uid, GuestAudience::Session, now)?;
    let refreshed = device::refresh(actor, &session, now + 100)?;
    let id = Identity::from_email("Ann@Example.com").context("fixture email")?;
    Ok(json!({
        "generated_by": "crates/euchre-server-core/tests/foundation_auth.rs (EUCHRE_WRITE_RUST_TOKENS=1)",
        "now_secs": now,
        "guest": [
            {"user_id": uid, "aud": "euchre-session", "iat": now, "token": session},
            {"user_id": uid, "aud": "euchre-actors", "iat": now,
             "token": guest::sign(actor, uid, GuestAudience::Actors, now)?},
            {"user_id": uid, "aud": "euchre-session", "iat": now + 100, "token": refreshed.session_token,
             "note": "device refresh of the first token"},
        ],
        "profile": [
            {"user_id": "4f2a9c1e", "iat": now, "issuer": null,
             "token": profile::mint(actor, "4f2a9c1e", now, None)?},
            {"user_id": "4f2a9c1e", "iat": now, "issuer": "euchre-test",
             "token": profile::mint(actor, "4f2a9c1e", now, Some("euchre-test"))?},
            {"user_id": "\u{1F0A1}".repeat(64), "iat": now, "issuer": null,
             "token": profile::mint(actor, &"\u{1F0A1}".repeat(64), now, None)?},
        ],
        "identity": [
            {"email": id.email, "user_id": id.user_id, "iat": now,
             "token": identity::sign(id_key, &id, now)?},
        ],
    }))
}

#[test]
fn rust_minted_fixture_is_current() -> Result<()> {
    let path = fixtures_dir().join("rust-minted.json");
    let body = format!("{}\n", serde_json::to_string_pretty(&rust_minted()?)?);
    if std::env::var("EUCHRE_WRITE_RUST_TOKENS").is_ok_and(|v| v == "1") {
        std::fs::write(&path, &body).with_context(|| format!("write {}", path.display()))?;
        return Ok(());
    }
    let on_disk = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "read {} (regenerate with EUCHRE_WRITE_RUST_TOKENS=1)",
            path.display()
        )
    })?;
    assert_eq!(
        on_disk, body,
        "rust-minted.json is stale: rerun with EUCHRE_WRITE_RUST_TOKENS=1"
    );
    Ok(())
}
