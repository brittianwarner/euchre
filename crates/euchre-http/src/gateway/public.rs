//! Public admission in front of [`super::GatewayProxy`] (05 §12.2, TS
//! `server/index.ts` `/rivet*` branch), in order:
//!
//! 1. Path tightening: only a concrete actor target is reachable, so no gateway
//!    `create`/`getOrCreate`, no other namespace and no actor name. Anything else is
//!    `404 Not found` (text).
//! 2. The Origin allowlist (an absent Origin passes; native clients send none).
//! 3. The caller: a browser (Origin present) needs a valid guest session (cookie or
//!    bearer). A native caller needs a valid session or an engine gateway token,
//!    which the engine verifies. Otherwise `401 Unauthorized` (text).
//! 4. The per-IP gateway bucket (`429`).

use std::sync::LazyLock;

use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use regex::Regex;

use super::{
    ProxyRouteKind, classify_proxy_path, is_canonical_path, is_websocket_upgrade, offered_protocols,
};
use crate::caller::{Caller, Session};
use crate::error::{ApiError, gateway_json, text};
use crate::rate_limit::RouteClass;
use crate::state::ApiState;

#[allow(clippy::expect_used)]
static ACTOR_PROTOCOL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^rivet_actor\.[a-z0-9_-]{20,64}$").expect("ACTOR_PROTOCOL_RE is a valid regex")
});

#[allow(clippy::expect_used)]
static TOKEN_PROTOCOL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^rivet_token\.[A-Za-z0-9_.-]+$").expect("TOKEN_PROTOCOL_RE is a valid regex")
});

#[allow(clippy::expect_used)]
static ACTOR_SEGMENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9_-]{20,64}(@[A-Za-z0-9._~%-]+)?$")
        .expect("ACTOR_SEGMENT_RE is a valid regex")
});

/// The tightened public path rule.
///
/// - `/rivet/connect` (TS browser form): an empty query and the subprotocols
///   `rivet_target.actor`, `rivet_actor.<id>` and `rivet_token.<token>`.
/// - `/rivet/gateway/<actorId>[@<token>]/...` (the Rust client's direct form): the
///   segment is a concrete actor id and the query carries no `rvt-*` key (those
///   select query-based `get`/`getOrCreate` targets). Keys are compared after
///   form decoding (`rvt%2Dmethod` is `rvt-method`); a key that does not decode
///   refuses the request.
/// - Either way the path must be [`is_canonical_path`]: no dot segment and no
///   encoded separator can rewrite it after admission.
///
/// The proxy's own allowlist still applies after this.
pub fn admits_public_path(path: &str, query: Option<&str>, protocols: &[String]) -> bool {
    if !path
        .strip_prefix("/rivet")
        .is_some_and(|rest| rest.starts_with('/') && is_canonical_path(rest))
    {
        return false;
    }
    if path == "/rivet/connect" {
        return query.is_none_or(str::is_empty)
            && protocols.iter().any(|p| p == "rivet_target.actor")
            && protocols.iter().any(|p| ACTOR_PROTOCOL_RE.is_match(p))
            && protocols.iter().any(|p| TOKEN_PROTOCOL_RE.is_match(p));
    }
    let Some(rest) = path.strip_prefix("/rivet/gateway/") else {
        return false;
    };
    let segment = rest.split('/').next().unwrap_or_default();
    if !ACTOR_SEGMENT_RE.is_match(segment) {
        return false;
    }
    query.is_none_or(|q| {
        q.split('&').all(|kv| {
            let key = kv.split_once('=').map_or(kv, |(k, _)| k);
            decode_form_component(key)
                .is_some_and(|key| !key.to_ascii_lowercase().starts_with("rvt-"))
        })
    })
}

/// One `application/x-www-form-urlencoded` component: `+` is a space, `%XX` a
/// byte. `None` for a malformed escape or a result that is not UTF-8.
fn decode_form_component(raw: &str) -> Option<String> {
    String::from_utf8(super::percent_decode_bytes(&raw.replace('+', " "))?).ok()
}

/// True when the request carries an engine gateway token: `@<token>` in the actor
/// segment, a `rivet_token.` subprotocol or an `x-rivet-token` header.
pub fn carries_gateway_token(path: &str, headers: &HeaderMap, protocols: &[String]) -> bool {
    let in_path = path
        .strip_prefix("/rivet/gateway/")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|seg| seg.split_once('@').is_some_and(|(_, t)| !t.is_empty()));
    in_path
        || protocols.iter().any(|p| TOKEN_PROTOCOL_RE.is_match(p))
        || headers
            .get("x-rivet-token")
            .is_some_and(|v| !v.as_bytes().is_empty())
}

/// The `/rivet` and `/rivet/**` handler of the app router.
pub(crate) async fn rivet_entry(state: ApiState, mut req: Request) -> Response {
    let socket = crate::api::socket_ip(&req);
    let inner = &state.inner;
    let path = req.uri().path().to_owned();
    if path == "/rivet/metadata" {
        // rivetkit 3.1 JS clients put the namespace in every gateway URL and in the
        // WebSocket subprotocols, and learn it only from this lookup (`clientEndpoint`
        // plus `clientNamespace`); the stub `{}` left them on `default`. Native clients
        // dial the same endpoint and namespace, so they read the same answer.
        if req.method() == axum::http::Method::GET
            && let Some(origin) = req.extensions().get::<crate::origin::PublicOrigin>()
        {
            return crate::api::no_store(crate::api::rivet_discovery(&state, &origin.0));
        }
        return inner.proxy.handle(req, socket).await;
    }

    let protocols = offered_protocols(req.headers());
    if !admits_public_path(&path, req.uri().query(), &protocols) {
        return text(StatusCode::NOT_FOUND, "Not found");
    }
    // A path the proxy would refuse anyway is refused here first, before any
    // credential is looked at.
    let route = classify_proxy_path(
        &path,
        req.method().as_str(),
        is_websocket_upgrade(req.headers()),
        "/rivet",
    );
    if route.kind == ProxyRouteKind::Deny {
        return gateway_json(StatusCode::NOT_FOUND, "not_found");
    }

    let origin = match req.headers().get(header::ORIGIN) {
        None => None,
        Some(v) => match v.to_str() {
            Ok(s) => Some(s.to_owned()),
            Err(_) => return gateway_json(StatusCode::FORBIDDEN, "origin_not_allowed"),
        },
    };
    if !inner.config.origins.check(origin.as_deref()) {
        tracing::warn!(origin = ?origin, path = %super::redact_path(&path), "rivet proxy: origin rejected");
        return gateway_json(StatusCode::FORBIDDEN, "origin_not_allowed");
    }

    let caller = Caller::resolve(&inner.auth, req.headers());
    let has_session = matches!(caller.session, Session::Bearer(_) | Session::Cookie(_));
    let admitted = if origin.is_some() {
        has_session
    } else {
        has_session
            || (caller.session != Session::InvalidBearer
                && carries_gateway_token(&path, req.headers(), &protocols))
    };
    if !admitted {
        return text(StatusCode::UNAUTHORIZED, "Unauthorized");
    }

    crate::api::pin_forwarded_for(&state, req.headers_mut(), socket);
    let ip = crate::api::client_ip(&state, req.headers(), socket);
    if !inner.limiter.check(RouteClass::Gateway, ip) {
        return axum::response::IntoResponse::into_response(ApiError::rate_limited());
    }

    inner.proxy.handle(req, socket).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "d1xbeec3w8bqkydvu2fvp6hkqwcl00";

    fn protos(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn ts_connect_form_needs_all_three_subprotocols_and_no_query() {
        let full = protos(&[
            "rivet",
            "rivet_target.actor",
            &format!("rivet_actor.{ID}"),
            "rivet_token.abc.def-ghi_j",
        ]);
        assert!(admits_public_path("/rivet/connect", None, &full));
        assert!(admits_public_path("/rivet/connect", Some(""), &full));
        assert!(!admits_public_path(
            "/rivet/connect",
            Some("rvt-namespace=x"),
            &full
        ));
        for missing in 1..4 {
            let mut partial = full.clone();
            partial.remove(missing);
            assert!(
                !admits_public_path("/rivet/connect", None, &partial),
                "{partial:?}"
            );
        }
        // A short actor id or an odd token alphabet fails.
        let bad = protos(&["rivet_target.actor", "rivet_actor.short", "rivet_token.x"]);
        assert!(!admits_public_path("/rivet/connect", None, &bad));
        let bad = protos(&[
            "rivet_target.actor",
            &format!("rivet_actor.{ID}"),
            "rivet_token.a+b",
        ]);
        assert!(!admits_public_path("/rivet/connect", None, &bad));
    }

    #[test]
    fn direct_gateway_form_needs_a_concrete_actor_id() {
        let none: Vec<String> = Vec::new();
        assert!(admits_public_path(
            &format!("/rivet/gateway/{ID}/connect"),
            None,
            &none
        ));
        assert!(admits_public_path(
            &format!("/rivet/gateway/{ID}@eyJ0.e30.sig%2B/action/snapshot"),
            None,
            &none
        ));
        // Name-based and query targets (create / getOrCreate) are unreachable.
        assert!(!admits_public_path(
            "/rivet/gateway/euchreTable/action/snapshot",
            None,
            &none
        ));
        assert!(!admits_public_path(
            &format!("/rivet/gateway/{ID}/action/snapshot"),
            Some("rvt-method=getOrCreate&rvt-key=table"),
            &none
        ));
        assert!(!admits_public_path(
            &format!("/rivet/gateway/{ID}/action/snapshot"),
            Some("RVT-Method=create"),
            &none
        ));
        assert!(admits_public_path(
            &format!("/rivet/gateway/{ID}/request/x"),
            Some("page=2"),
            &none
        ));
        for denied in [
            "/rivet/actors",
            "/rivet/namespaces",
            "/rivet",
            "/rivet/",
            "/rivet/ui",
        ] {
            assert!(!admits_public_path(denied, None, &none), "{denied}");
        }
    }

    #[test]
    fn encoded_rvt_query_keys_are_refused() {
        let none: Vec<String> = Vec::new();
        let path = format!("/rivet/gateway/{ID}/action/snapshot");
        for query in [
            "rvt%2Dmethod=getOrCreate&rvt%2Dkey=evil",
            "%72vt-method=create",
            "RVT%2dnamespace=default",
            "rvt%2Drunner=pool",
            "x=1&%52%56%54%2D%6Dethod=get",
            // Malformed escapes and non-UTF-8 keys refuse rather than guess.
            "rvt%2=x",
            "%zz=1",
            "%ff=1",
        ] {
            assert!(!admits_public_path(&path, Some(query), &none), "{query}");
        }
        // Ordinary keys, including encoded ones, still pass.
        assert!(admits_public_path(&path, Some("a%20b=1&c+d=2"), &none));
        assert!(admits_public_path(&path, Some("rvtx=1"), &none));
    }

    #[test]
    fn dot_segments_and_encoded_separators_are_refused() {
        let none: Vec<String> = Vec::new();
        for path in [
            format!("/rivet/gateway/{ID}/request/../../actors"),
            format!("/rivet/gateway/{ID}/request/%2e%2e/%2E%2E/namespaces"),
            format!("/rivet/gateway/{ID}/request/.%2E/.%2e/auth/tokens"),
            format!("/rivet/gateway/{ID}/request/%2E./inspector/ui/"),
            format!("/rivet/gateway/{ID}/request/./x"),
            format!("/rivet/gateway/{ID}/request/%2e/x"),
            format!("/rivet/gateway/{ID}/websocket/../../../actors"),
            format!("/rivet/gateway/{ID}/request/a%2f..%2f..%2factors"),
            format!("/rivet/gateway/{ID}/request/a%5C..%5Cactors"),
            format!("/rivet/gateway/{ID}/request/a\\..\\actors"),
            format!("/rivet/gateway/{ID}/request/%zz"),
            format!("/rivet/gateway/{ID}/.."),
            "/rivet/connect/..".to_owned(),
            "/rivet/../rivet/connect".to_owned(),
        ] {
            assert!(!admits_public_path(&path, None, &none), "{path}");
        }
        // The token part of the actor segment may carry an encoded `/`.
        assert!(admits_public_path(
            &format!("/rivet/gateway/{ID}@ab%2Fcd/action/snapshot"),
            None,
            &none
        ));
        // Dots inside a segment are ordinary characters.
        assert!(admits_public_path(
            &format!("/rivet/gateway/{ID}/request/a..b/file.json"),
            None,
            &none
        ));
    }

    #[test]
    fn gateway_token_presence() -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();
        let none: Vec<String> = Vec::new();
        assert!(carries_gateway_token(
            &format!("/rivet/gateway/{ID}@tok/connect"),
            &headers,
            &none
        ));
        assert!(!carries_gateway_token(
            &format!("/rivet/gateway/{ID}@/connect"),
            &headers,
            &none
        ));
        assert!(!carries_gateway_token(
            &format!("/rivet/gateway/{ID}/connect"),
            &headers,
            &none
        ));
        assert!(carries_gateway_token(
            "/rivet/connect",
            &headers,
            &protos(&["rivet_token.abc"])
        ));
        headers.insert("x-rivet-token", "abc".parse()?);
        assert!(carries_gateway_token(
            &format!("/rivet/gateway/{ID}/connect"),
            &headers,
            &none
        ));
        Ok(())
    }
}
