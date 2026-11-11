//! Router tests through `tower::ServiceExt::oneshot`, no network: every endpoint,
//! the auth paths, the cookies and the error bodies. Actor calls go to an engine
//! address with no listener, so they exercise the unavailable mappings.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::response::Response;
use euchre_server_core::auth::{AuthKeys, Authenticator, GuestAudience};
use euchre_server_core::config::{IpCidr, OriginPolicy, Secret};
use euchre_server_core::rivet::build_client_with_pool;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::BAD_JSON;
use super::profile::{
    BAD_ID, NEEDS_IDENTITY, NEEDS_SESSION, SAVE_NEEDS_IDENTITY, SCRAPBOOK_UNAVAILABLE,
    SETTINGS_UNAVAILABLE,
};
use super::tables::{CREATE_FAILED, INVALID_TABLE, START_A_GAME_FIRST, TABLE_UNAVAILABLE};
use crate::config::{ApiConfig, BucketConfig, CallTimeouts, GatewayOptions, RateLimitConfig};
use crate::error::RATE_LIMITED_MESSAGE;
use crate::state::{ApiState, Health};

const ORIGIN: &str = "https://euchre.sh";
const HOST: &str = "euchre.sh";
const GAME: &str = "11111111-1111-4111-8111-111111111111";

fn auth() -> Authenticator {
    Authenticator::new(AuthKeys {
        guest: Secret::new("g".repeat(40)),
        profile: Secret::new("p".repeat(40)),
        internal: Secret::new("i".repeat(40)),
        identity: Secret::new("d".repeat(40)),
        jwt_issuer: None,
        allow_unsigned_email: false,
    })
}

/// An engine URL nothing listens on.
fn dead_endpoint() -> anyhow::Result<String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(format!("http://{}", listener.local_addr()?))
}

struct Harness {
    state: ApiState,
    auth: Authenticator,
}

fn config(endpoint: &str) -> anyhow::Result<ApiConfig> {
    Ok(ApiConfig {
        origins: OriginPolicy::strict(vec![ORIGIN.to_owned(), "https://www.euchre.sh".into()])?,
        engine_endpoint: endpoint.to_owned(),
        namespace: "default".into(),
        pool_name: "default".into(),
        engine_token: None,
        secure_cookies: true,
        static_dir: None,
        downloads_dir: None,
        downloads_bucket: None,
        trust_forwarded_for: false,
        trusted_proxies: Vec::new(),
        rate_limits: RateLimitConfig::unlimited(),
        gateway: GatewayOptions::default(),
        timeouts: CallTimeouts {
            actor_call: Duration::from_millis(400),
            recent_snapshot: Duration::from_millis(400),
            persona_settings: Duration::from_millis(200),
            gateway_token: Duration::from_millis(400),
        },
    })
}

fn harness_with(edit: impl FnOnce(&mut ApiConfig)) -> anyhow::Result<Harness> {
    let endpoint = dead_endpoint()?;
    let mut cfg = config(&endpoint)?;
    edit(&mut cfg);
    let auth = auth();
    let client = build_client_with_pool(&endpoint, "default", None, Some("default"));
    let health = Health::new();
    health.set_ready();
    let state = ApiState::new(cfg, auth.clone(), Arc::clone(&client), health)?;
    Ok(Harness { state, auth })
}

fn harness() -> anyhow::Result<Harness> {
    harness_with(|_| {})
}

struct Req {
    method: Method,
    path: String,
    headers: Vec<(String, String)>,
    body: Body,
    /// The socket peer (`ConnectInfo`), as `into_make_service_with_connect_info` sets it.
    peer: Option<SocketAddr>,
}

fn req(method: Method, path: &str) -> Req {
    Req {
        method,
        path: path.to_owned(),
        headers: vec![("host".into(), HOST.into())],
        body: Body::empty(),
        peer: None,
    }
}

impl Req {
    fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_owned(), value.into()));
        self
    }

    fn origin(self) -> Self {
        self.header("origin", ORIGIN)
    }

    fn json(mut self, body: &Value) -> Self {
        self.body = Body::from(body.to_string());
        self.header("content-type", "application/json")
    }

    fn raw(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    fn host(mut self, host: &str) -> Self {
        self.headers.retain(|(n, _)| n != "host");
        self.header("host", host)
    }

    fn peer(mut self, addr: &str) -> anyhow::Result<Self> {
        self.peer = Some(addr.parse()?);
        Ok(self)
    }
}

impl Harness {
    async fn send(&self, r: Req) -> anyhow::Result<Response> {
        let mut builder = Request::builder().method(r.method).uri(r.path);
        for (name, value) in r.headers {
            builder = builder.header(name, value);
        }
        let mut request = builder.body(r.body)?;
        if let Some(peer) = r.peer {
            request.extensions_mut().insert(ConnectInfo(peer));
        }
        Ok(super::router(self.state.clone()).oneshot(request).await?)
    }

    fn session(&self, user: &str) -> anyhow::Result<String> {
        Ok(self.auth.sign_guest(user, GuestAudience::Session)?)
    }

    fn cookie(&self, user: &str) -> anyhow::Result<String> {
        Ok(format!("euchre_guest={}", self.session(user)?))
    }

    fn bearer(&self, user: &str) -> anyhow::Result<String> {
        Ok(format!("Bearer {}", self.session(user)?))
    }

    fn identity_token(&self, email: &str) -> anyhow::Result<String> {
        let identity = self.auth.identity_for_email(email)?;
        Ok(self.auth.issue_identity_token(&identity)?)
    }
}

async fn body_text(res: Response) -> anyhow::Result<String> {
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22).await?;
    Ok(String::from_utf8(bytes.to_vec())?)
}

async fn body_json(res: Response) -> anyhow::Result<Value> {
    Ok(serde_json::from_str(&body_text(res).await?)?)
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn set_cookies(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_owned))
        .collect()
}

/// Assert the `{"message","error":{"code","message"}}` body.
async fn assert_error(
    res: Response,
    status: StatusCode,
    code: &str,
    message: &str,
) -> anyhow::Result<()> {
    assert_eq!(res.status(), status);
    assert_eq!(header_str(res.headers(), "cache-control"), Some("no-store"));
    let text = body_text(res).await?;
    let expected = json!({ "message": message, "error": { "code": code, "message": message } });
    assert_eq!(text, expected.to_string(), "key order and bytes");
    Ok(())
}

// ---- Front door --------------------------------------------------------------------

#[tokio::test]
async fn health_reports_ready_and_draining_on_any_host() -> anyhow::Result<()> {
    let h = harness()?;
    let res = h
        .send(req(Method::GET, "/health").host("anything.example"))
        .await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_text(res).await?, "ok");
    h.state.health().start_draining();
    let res = h.send(req(Method::GET, "/health")).await?;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body_text(res).await?, "draining");
    Ok(())
}

#[tokio::test]
async fn health_is_draining_until_ready() -> anyhow::Result<()> {
    let endpoint = dead_endpoint()?;
    let client = build_client_with_pool(&endpoint, "default", None, None);
    let health = Health::new();
    let state = ApiState::new(config(&endpoint)?, auth(), client, health.clone())?;
    let h = Harness {
        state,
        auth: auth(),
    };
    let res = h.send(req(Method::GET, "/health")).await?;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    health.set_ready();
    assert_eq!(
        h.send(req(Method::GET, "/health")).await?.status(),
        StatusCode::OK
    );
    assert!(!health.is_draining());
    Ok(())
}

#[tokio::test]
async fn an_unknown_host_gets_421_and_forwarded_host_is_ignored() -> anyhow::Result<()> {
    let h = harness()?;
    for r in [
        req(Method::GET, "/api/session").host("attacker.example"),
        req(Method::GET, "/api/session")
            .host("attacker.example")
            .header("x-forwarded-host", HOST),
        req(Method::GET, "/").host("euchre.sh:444"),
    ] {
        let res = h.send(r).await?;
        assert_eq!(res.status(), StatusCode::MISDIRECTED_REQUEST);
        assert_eq!(body_text(res).await?, "Unknown host");
    }
    // Each configured host resolves.
    let res = h
        .send(req(Method::GET, "/api/rivet/metadata").host("www.euchre.sh"))
        .await?;
    assert_eq!(
        body_json(res).await?,
        json!({ "clientEndpoint": "https://www.euchre.sh/rivet", "clientNamespace": "default" })
    );
    Ok(())
}

#[tokio::test]
async fn api_rivet_metadata_and_not_found_shapes() -> anyhow::Result<()> {
    let h = harness()?;
    let res = h.send(req(Method::GET, "/api/rivet/metadata")).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header_str(res.headers(), "cache-control"), Some("no-store"));
    assert_eq!(
        body_text(res).await?,
        r#"{"clientEndpoint":"https://euchre.sh/rivet","clientNamespace":"default"}"#
    );
    for path in [
        "/api/rivet",
        "/api/rivet/actors",
        "/api/rivet/gateway/x/connect",
    ] {
        let res = h.send(req(Method::GET, path)).await?;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(body_text(res).await?, "Not found");
    }
    assert_error(
        h.send(req(Method::GET, "/api/nope")).await?,
        StatusCode::NOT_FOUND,
        "not_found",
        "Not found",
    )
    .await?;
    // No static dir: everything else is a plain 404.
    let res = h.send(req(Method::GET, "/play")).await?;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_text(res).await?, "Not found");
    Ok(())
}

#[tokio::test]
async fn cors_allows_listed_origins_with_credentials() -> anyhow::Result<()> {
    let h = harness()?;
    let res = h
        .send(
            req(Method::OPTIONS, "/api/session")
                .header("origin", "https://www.euchre.sh")
                .header("access-control-request-method", "POST")
                .header(
                    "access-control-request-headers",
                    "content-type,authorization",
                ),
        )
        .await?;
    let headers = res.headers();
    assert_eq!(
        header_str(headers, "access-control-allow-origin"),
        Some("https://www.euchre.sh")
    );
    assert_eq!(
        header_str(headers, "access-control-allow-credentials"),
        Some("true")
    );
    let methods = header_str(headers, "access-control-allow-methods").unwrap_or_default();
    for m in ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"] {
        assert!(methods.contains(m), "{methods}");
    }
    let allowed = header_str(headers, "access-control-allow-headers")
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(allowed.contains("content-type") && allowed.contains("authorization"));

    let res = h
        .send(
            req(Method::OPTIONS, "/api/session")
                .header("origin", "https://evil.example")
                .header("access-control-request-method", "POST"),
        )
        .await?;
    assert!(res.headers().get("access-control-allow-origin").is_none());
    Ok(())
}

// ---- Session, device, identity -----------------------------------------------------

#[tokio::test]
async fn session_reports_the_caller_without_setting_cookies() -> anyhow::Result<()> {
    let h = harness()?;
    let res = h.send(req(Method::GET, "/api/session")).await?;
    assert!(set_cookies(res.headers()).is_empty());
    assert_eq!(header_str(res.headers(), "cache-control"), Some("no-store"));
    assert_eq!(
        body_text(res).await?,
        r#"{"guestUserId":null,"identity":null}"#
    );

    let res = h
        .send(req(Method::GET, "/api/session").header("cookie", h.cookie("guest-a")?))
        .await?;
    assert_eq!(body_json(res).await?["guestUserId"], "guest-a");

    // Bearer wins over the cookie; the identity header wins over the cookie.
    let res = h
        .send(
            req(Method::GET, "/api/session")
                .header("authorization", h.bearer("guest-b")?)
                .header(
                    "cookie",
                    format!(
                        "{}; euchre_email={}",
                        h.cookie("guest-a")?,
                        h.identity_token("bob@example.com")?
                    ),
                )
                .header("x-euchre-identity", h.identity_token("ann@example.com")?),
        )
        .await?;
    let body = body_json(res).await?;
    assert_eq!(body["guestUserId"], "guest-b");
    assert_eq!(body["identity"]["email"], "ann@example.com");
    Ok(())
}

#[tokio::test]
async fn enroll_mints_a_device_session() -> anyhow::Result<()> {
    let h = harness()?;
    // Native clients send no Origin.
    let res = h.send(req(Method::POST, "/api/device/enroll")).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header_str(res.headers(), "cache-control"), Some("no-store"));
    let body = body_json(res).await?;
    let user = body["userId"].as_str().unwrap_or_default().to_owned();
    assert!(user.starts_with("guest-"), "{user}");
    let token = body["sessionToken"].as_str().unwrap_or_default();
    assert_eq!(h.auth.verify_session_bearer(token)?, user);
    assert!(body["expiresAt"].as_i64().is_some());

    let res = h
        .send(req(Method::POST, "/api/device/enroll").origin())
        .await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_error(
        h.send(req(Method::POST, "/api/device/enroll").header("origin", "https://evil.example"))
            .await?,
        StatusCode::FORBIDDEN,
        "forbidden",
        "Origin not allowed",
    )
    .await
}

#[tokio::test]
async fn enroll_is_rate_limited_per_ip() -> anyhow::Result<()> {
    let h = harness_with(|cfg| {
        cfg.rate_limits.enroll = BucketConfig::new(2, 1);
    })?;
    for _ in 0..2 {
        let res = h.send(req(Method::POST, "/api/device/enroll")).await?;
        assert_eq!(res.status(), StatusCode::OK);
    }
    assert_error(
        h.send(req(Method::POST, "/api/device/enroll")).await?,
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        RATE_LIMITED_MESSAGE,
    )
    .await
}

// ---- X-Forwarded-For trust (D11, requests/server-http-trusted-proxy.md) -----------

const DIRECT_CLIENT: &str = "203.0.113.9:50000";
const EDGE: &str = "10.0.0.1:41000";

fn enroll_from(peer: &str, xff: &[&str]) -> anyhow::Result<Req> {
    let mut r = req(Method::POST, "/api/device/enroll").peer(peer)?;
    for line in xff {
        r = r.header("x-forwarded-for", *line);
    }
    Ok(r)
}

#[tokio::test]
async fn a_direct_client_cannot_pick_its_bucket_when_trust_is_off() -> anyhow::Result<()> {
    let h = harness_with(|cfg| {
        assert!(!cfg.trust_forwarded_for, "the default");
        cfg.rate_limits.enroll = BucketConfig::new(1, 1);
    })?;
    let first = h
        .send(enroll_from(DIRECT_CLIENT, &["198.51.100.1"])?)
        .await?;
    assert_eq!(first.status(), StatusCode::OK);
    // A fresh forged address per request still lands in the socket's bucket.
    for forged in ["198.51.100.2", "198.51.100.3, 198.51.100.4", "2001:db8::7"] {
        let res = h.send(enroll_from(DIRECT_CLIENT, &[forged])?).await?;
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS, "{forged}");
    }
    let res = h.send(enroll_from(DIRECT_CLIENT, &[])?).await?;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    // Another socket peer has its own bucket.
    let other = h
        .send(enroll_from("203.0.113.10:50000", &["198.51.100.1"])?)
        .await?;
    assert_eq!(other.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn trusted_forwarded_for_keys_on_the_last_entry_of_the_last_line() -> anyhow::Result<()> {
    let h = harness_with(|cfg| {
        cfg.trust_forwarded_for = true;
        cfg.rate_limits.enroll = BucketConfig::new(1, 1);
    })?;
    let ok = |r: Response| assert_eq!(r.status(), StatusCode::OK);
    let limited = |r: Response| assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);

    ok(h.send(enroll_from(EDGE, &["6.6.6.6, 198.51.100.1"])?)
        .await?);
    // A forged first entry does not buy a new bucket: the edge's entry is last.
    limited(
        h.send(enroll_from(EDGE, &["7.7.7.7, 198.51.100.1"])?)
            .await?,
    );
    // A client line followed by the edge's own line: the last line wins.
    limited(
        h.send(enroll_from(EDGE, &["198.51.100.9", "198.51.100.1"])?)
            .await?,
    );
    ok(
        h.send(enroll_from(EDGE, &["198.51.100.1", "198.51.100.2:443"])?)
            .await?,
    );
    // IPv4-mapped and plain IPv4 are one client.
    ok(h.send(enroll_from(EDGE, &["::ffff:198.51.100.3"])?).await?);
    limited(h.send(enroll_from(EDGE, &["198.51.100.3"])?).await?);
    // No usable entry: the socket peer's bucket.
    ok(h.send(enroll_from(EDGE, &["not-an-ip"])?).await?);
    limited(h.send(enroll_from(EDGE, &[])?).await?);
    Ok(())
}

#[tokio::test]
async fn trusted_proxies_honor_forwarded_for_only_from_the_proxy() -> anyhow::Result<()> {
    let edge_net: IpCidr = "10.0.0.0/8".parse()?;
    let h = harness_with(|cfg| {
        cfg.trust_forwarded_for = true;
        cfg.trusted_proxies = vec![edge_net];
        cfg.rate_limits.enroll = BucketConfig::new(1, 1);
    })?;
    let status = |r: Response| r.status();
    assert_eq!(
        status(h.send(enroll_from(EDGE, &["198.51.100.1"])?).await?),
        StatusCode::OK
    );
    assert_eq!(
        status(
            h.send(enroll_from("10.9.9.9:1", &["198.51.100.1"])?)
                .await?
        ),
        StatusCode::TOO_MANY_REQUESTS,
        "any peer inside the network names the same client"
    );
    assert_eq!(
        status(h.send(enroll_from(EDGE, &["198.51.100.2"])?).await?),
        StatusCode::OK
    );
    // A peer outside the network is a direct client: its header is ignored.
    assert_eq!(
        status(
            h.send(enroll_from(DIRECT_CLIENT, &["198.51.100.3"])?)
                .await?
        ),
        StatusCode::OK
    );
    for forged in ["198.51.100.4", "198.51.100.5"] {
        assert_eq!(
            status(h.send(enroll_from(DIRECT_CLIENT, &[forged])?).await?),
            StatusCode::TOO_MANY_REQUESTS,
            "{forged}"
        );
    }
    // No ConnectInfo at all: never trusted against a list.
    let no_peer =
        || req(Method::POST, "/api/device/enroll").header("x-forwarded-for", "198.51.100.6");
    assert_eq!(status(h.send(no_peer()).await?), StatusCode::OK);
    assert_eq!(
        status(h.send(no_peer()).await?),
        StatusCode::TOO_MANY_REQUESTS
    );
    Ok(())
}

/// An engine stand-in that answers every request with the `X-Forwarded-For` it got.
async fn xff_echo_upstream() -> anyhow::Result<String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let app = axum::Router::new().fallback(|headers: HeaderMap| async move {
        axum::Json(json!({ "xff": header_str(&headers, "x-forwarded-for") }))
    });
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(format!("http://{addr}"))
}

async fn guard_xff(h: &Harness, peer: &str, xff: &[&str]) -> anyhow::Result<Value> {
    let mut r = req(
        Method::POST,
        &format!("/rivet/gateway/{ACTOR}/action/snapshot"),
    )
    .header("authorization", h.bearer("guest-a")?)
    .peer(peer)?;
    for line in xff {
        r = r.header("x-forwarded-for", *line);
    }
    let res = h.send(r).await?;
    assert_eq!(res.status(), StatusCode::OK);
    Ok(body_json(res).await?["xff"].clone())
}

#[tokio::test]
async fn the_engine_guard_gets_the_same_client_ip_as_the_buckets() -> anyhow::Result<()> {
    let upstream = xff_echo_upstream().await?;

    // Trust off: the Guard sees the socket peer, whatever the client wrote.
    let h = harness_with(|cfg| cfg.engine_endpoint = upstream.clone())?;
    assert_eq!(
        guard_xff(&h, DIRECT_CLIENT, &["198.51.100.1"]).await?,
        "203.0.113.9"
    );
    assert_eq!(
        guard_xff(&h, "[::ffff:203.0.113.9]:1", &[]).await?,
        "203.0.113.9"
    );

    // Trust on: one entry, the last of the last line.
    let h = harness_with(|cfg| {
        cfg.engine_endpoint = upstream.clone();
        cfg.trust_forwarded_for = true;
    })?;
    assert_eq!(
        guard_xff(&h, EDGE, &["6.6.6.6, 198.51.100.1", "198.51.100.2"]).await?,
        "198.51.100.2"
    );
    assert_eq!(guard_xff(&h, EDGE, &["not-an-ip"]).await?, "10.0.0.1");

    // Trust on behind a list: a peer outside it is treated as direct.
    let edge_host = IpCidr::host("10.0.0.1".parse()?);
    let h = harness_with(|cfg| {
        cfg.engine_endpoint = upstream.clone();
        cfg.trust_forwarded_for = true;
        cfg.trusted_proxies = vec![edge_host];
    })?;
    assert_eq!(
        guard_xff(&h, EDGE, &["198.51.100.1"]).await?,
        "198.51.100.1"
    );
    assert_eq!(
        guard_xff(&h, DIRECT_CLIENT, &["198.51.100.1"]).await?,
        "203.0.113.9"
    );
    Ok(())
}

#[tokio::test]
async fn refresh_rotates_a_valid_bearer_only() -> anyhow::Result<()> {
    let h = harness()?;
    for r in [
        req(Method::POST, "/api/device/refresh"),
        req(Method::POST, "/api/device/refresh").header("authorization", "Bearer nope"),
        // A cookie is not a bearer.
        req(Method::POST, "/api/device/refresh").header("cookie", h.cookie("guest-a")?),
    ] {
        assert_error(
            h.send(r).await?,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Please start a game first",
        )
        .await?;
    }
    let res = h
        .send(
            req(Method::POST, "/api/device/refresh").header("authorization", h.bearer("guest-a")?),
        )
        .await?;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await?;
    assert_eq!(body["userId"], "guest-a");
    let token = body["sessionToken"].as_str().unwrap_or_default();
    assert_eq!(h.auth.verify_session_bearer(token)?, "guest-a");
    Ok(())
}

#[tokio::test]
async fn identify_sets_the_identity_cookie_for_cookie_callers() -> anyhow::Result<()> {
    let h = harness()?;
    let body = json!({ "email": "  Ann@Example.com " });
    // CSRF: a cookie caller needs Origin == the public origin.
    assert_error(
        h.send(req(Method::POST, "/api/identity/identify").json(&body))
            .await?,
        StatusCode::FORBIDDEN,
        "forbidden",
        "Origin not allowed",
    )
    .await?;
    assert_error(
        h.send(
            req(Method::POST, "/api/identity/identify")
                .header("origin", "https://www.euchre.sh")
                .json(&body),
        )
        .await?,
        StatusCode::FORBIDDEN,
        "forbidden",
        "Origin not allowed",
    )
    .await?;

    let res = h
        .send(
            req(Method::POST, "/api/identity/identify")
                .origin()
                .json(&body),
        )
        .await?;
    assert_eq!(res.status(), StatusCode::OK);
    let cookies = set_cookies(res.headers());
    assert_eq!(cookies.len(), 1);
    let cookie = &cookies[0];
    assert!(cookie.starts_with("euchre_email="), "{cookie}");
    for attr in [
        "Path=/",
        "HttpOnly",
        "SameSite=Lax",
        "Secure",
        "Max-Age=34560000",
    ] {
        assert!(cookie.contains(attr), "{cookie} lacks {attr}");
    }
    assert!(!cookie.contains("Domain"), "{cookie}");
    let json = body_json(res).await?;
    assert_eq!(json["identity"]["email"], "ann@example.com");
    let token = json["identityToken"].as_str().unwrap_or_default();
    assert_eq!(
        h.auth.verify_identity_token(token)?.email,
        "ann@example.com"
    );
    assert!(cookie.contains(token));
    Ok(())
}

#[tokio::test]
async fn identify_for_bearer_callers_skips_csrf_and_cookies() -> anyhow::Result<()> {
    let h = harness()?;
    let res = h
        .send(
            req(Method::POST, "/api/identity/identify")
                .header("authorization", h.bearer("guest-a")?)
                .json(&json!({ "email": "ann@example.com" })),
        )
        .await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(set_cookies(res.headers()).is_empty());
    assert!(body_json(res).await?["identityToken"].is_string());

    // An invalid bearer is refused before the body is read.
    assert_error(
        h.send(
            req(Method::POST, "/api/identity/identify")
                .header("authorization", "Bearer nope")
                .origin()
                .json(&json!({ "email": "ann@example.com" })),
        )
        .await?,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "Please start a game first",
    )
    .await
}

#[tokio::test]
async fn identify_rejects_bad_emails_bad_json_and_huge_bodies() -> anyhow::Result<()> {
    let h = harness()?;
    let res = h
        .send(
            req(Method::POST, "/api/identity/identify")
                .origin()
                .json(&json!({ "email": "not-an-email" })),
        )
        .await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = body_json(res).await?;
    assert_eq!(body["error"]["code"], "bad_request");
    assert_eq!(body["message"], body["error"]["message"]);

    assert_error(
        h.send(
            req(Method::POST, "/api/identity/identify")
                .origin()
                .raw("{nope"),
        )
        .await?,
        StatusCode::BAD_REQUEST,
        "bad_request",
        BAD_JSON,
    )
    .await?;

    let huge = vec![b' '; crate::config::MAX_REQUEST_BODY_BYTES + 1];
    let res = h
        .send(
            req(Method::POST, "/api/identity/identify")
                .origin()
                .raw(huge),
        )
        .await?;
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    Ok(())
}

#[tokio::test]
async fn identify_is_rate_limited() -> anyhow::Result<()> {
    let h = harness_with(|cfg| cfg.rate_limits.identify = BucketConfig::new(1, 1))?;
    let body = json!({ "email": "ann@example.com" });
    let r = || {
        req(Method::POST, "/api/identity/identify")
            .origin()
            .json(&body)
    };
    assert_eq!(h.send(r()).await?.status(), StatusCode::OK);
    assert_eq!(h.send(r()).await?.status(), StatusCode::TOO_MANY_REQUESTS);
    Ok(())
}

#[tokio::test]
async fn sign_out_clears_the_identity_cookie() -> anyhow::Result<()> {
    let h = harness()?;
    assert_error(
        h.send(req(Method::DELETE, "/api/identity")).await?,
        StatusCode::FORBIDDEN,
        "forbidden",
        "Origin not allowed",
    )
    .await?;
    let res = h
        .send(req(Method::DELETE, "/api/identity").origin())
        .await?;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let cookies = set_cookies(res.headers());
    assert_eq!(cookies.len(), 1);
    assert!(cookies[0].starts_with("euchre_email=;"), "{}", cookies[0]);
    assert!(cookies[0].contains("Max-Age=0"), "{}", cookies[0]);
    assert!(cookies[0].contains("Path=/"), "{}", cookies[0]);

    let res = h
        .send(req(Method::DELETE, "/api/identity").header("authorization", h.bearer("guest-a")?))
        .await?;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    Ok(())
}

// ---- Tables ------------------------------------------------------------------------

#[tokio::test]
async fn create_table_guards_run_before_any_actor_call() -> anyhow::Result<()> {
    let h = harness()?;
    assert_error(
        h.send(req(Method::POST, "/api/tables")).await?,
        StatusCode::FORBIDDEN,
        "forbidden",
        "Origin not allowed",
    )
    .await?;
    assert_error(
        h.send(req(Method::POST, "/api/tables").header("authorization", "Bearer nope"))
            .await?,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        START_A_GAME_FIRST,
    )
    .await?;
    assert_error(
        h.send(req(Method::POST, "/api/tables").origin().raw("[1,"))
            .await?,
        StatusCode::BAD_REQUEST,
        "bad_request",
        BAD_JSON,
    )
    .await
}

#[tokio::test]
async fn create_table_maps_an_unreachable_engine_to_503() -> anyhow::Result<()> {
    let h = harness()?;
    for r in [
        req(Method::POST, "/api/tables").origin(),
        req(Method::POST, "/api/tables?stick=true")
            .origin()
            .header("cookie", h.cookie("guest-a")?)
            .header("x-euchre-identity", h.identity_token("ann@example.com")?),
        req(Method::POST, "/api/tables")
            .header("authorization", h.bearer("guest-a")?)
            .json(&json!({ "stick": true, "natural": false })),
    ] {
        assert_error(
            h.send(r).await?,
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            CREATE_FAILED,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn create_table_is_rate_limited() -> anyhow::Result<()> {
    let h = harness_with(|cfg| cfg.rate_limits.tables = BucketConfig::new(1, 1))?;
    let _ = h.send(req(Method::POST, "/api/tables").origin()).await?;
    assert_eq!(
        h.send(req(Method::POST, "/api/tables").origin())
            .await?
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    Ok(())
}

#[tokio::test]
async fn recent_tables_without_a_session_is_empty() -> anyhow::Result<()> {
    let h = harness()?;
    let res = h
        .send(
            req(Method::GET, "/api/tables/recent")
                .header("cookie", format!("euchre_recent_tables={GAME}")),
        )
        .await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        body_text(res).await?,
        r#"{"tables":[],"unavailable":false}"#
    );
    Ok(())
}

#[tokio::test]
async fn recent_tables_marks_unreachable_tables_unavailable() -> anyhow::Result<()> {
    let h = harness()?;
    let other = "22222222-2222-4222-8222-222222222222";
    let res = h
        .send(req(Method::GET, "/api/tables/recent").header(
            "cookie",
            format!(
                "{}; euchre_recent_tables={GAME}%2Cnot-a-uuid%2C{other}",
                h.cookie("guest-a")?
            ),
        ))
        .await?;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await?;
    assert_eq!(body["unavailable"], true);
    assert_eq!(
        body["tables"],
        json!([
            { "gameId": GAME, "status": null, "score": null, "handNo": null },
            { "gameId": other, "status": null, "score": null, "handNo": null },
        ])
    );
    // `?ids=` overrides the cookie.
    let res = h
        .send(
            req(Method::GET, &format!("/api/tables/recent?ids={other}"))
                .header("authorization", h.bearer("guest-a")?),
        )
        .await?;
    let body = body_json(res).await?;
    assert_eq!(body["tables"].as_array().map(Vec::len), Some(1));
    Ok(())
}

#[tokio::test]
async fn table_session_guards() -> anyhow::Result<()> {
    let h = harness()?;
    let path = format!("/api/table-session/{GAME}");
    // Cookie callers need the same-origin check before anything else.
    assert_error(
        h.send(req(Method::POST, &path).header("cookie", h.cookie("guest-a")?))
            .await?,
        StatusCode::FORBIDDEN,
        "forbidden",
        "Origin not allowed",
    )
    .await?;
    assert_error(
        h.send(req(Method::POST, "/api/table-session/not-a-uuid").origin())
            .await?,
        StatusCode::BAD_REQUEST,
        "bad_request",
        INVALID_TABLE,
    )
    .await?;
    assert_error(
        h.send(req(Method::POST, &path).origin()).await?,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        START_A_GAME_FIRST,
    )
    .await?;
    assert_error(
        h.send(req(Method::POST, &path).header("authorization", "Bearer nope"))
            .await?,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        START_A_GAME_FIRST,
    )
    .await
}

#[tokio::test]
async fn table_session_maps_an_unreachable_table_to_503() -> anyhow::Result<()> {
    let h = harness()?;
    let path = format!("/api/table-session/{GAME}");
    for r in [
        req(Method::POST, &path)
            .origin()
            .header("cookie", h.cookie("guest-a")?),
        // A bearer caller is CSRF-exempt.
        req(Method::POST, &path).header("authorization", h.bearer("guest-a")?),
    ] {
        assert_error(
            h.send(r).await?,
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            TABLE_UNAVAILABLE,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn table_session_is_rate_limited() -> anyhow::Result<()> {
    let h = harness_with(|cfg| cfg.rate_limits.table_session = BucketConfig::new(1, 1))?;
    let path = "/api/table-session/not-a-uuid";
    assert_eq!(
        h.send(req(Method::POST, path).origin()).await?.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        h.send(req(Method::POST, path).origin()).await?.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    Ok(())
}

// ---- Profile -----------------------------------------------------------------------

#[tokio::test]
async fn profile_reads_need_an_identity() -> anyhow::Result<()> {
    let h = harness()?;
    for path in [
        "/api/profile/settings",
        "/api/profile/personas",
        "/api/profile/stats",
        "/api/episodes",
        "/api/profile/settings?profile=email",
    ] {
        assert_error(
            h.send(req(Method::GET, path).header("cookie", h.cookie("guest-a")?))
                .await?,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            NEEDS_IDENTITY,
        )
        .await?;
    }
    for path in [
        "/api/games",
        "/api/games/match-1",
        "/api/stats?profile=browser",
    ] {
        let path = path.replace("/api/stats", "/api/profile/stats");
        assert_error(
            h.send(req(Method::GET, &path)).await?,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            NEEDS_SESSION,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn profile_calls_map_an_unreachable_engine_to_503() -> anyhow::Result<()> {
    let h = harness()?;
    let identity = h.identity_token("ann@example.com")?;
    for (path, message) in [
        ("/api/profile/settings", SETTINGS_UNAVAILABLE),
        ("/api/profile/personas", SETTINGS_UNAVAILABLE),
        ("/api/profile/stats", SCRAPBOOK_UNAVAILABLE),
        ("/api/games?limit=5&cursor=abc", SCRAPBOOK_UNAVAILABLE),
        ("/api/games/match-1", SCRAPBOOK_UNAVAILABLE),
        ("/api/episodes?limit=3", SCRAPBOOK_UNAVAILABLE),
    ] {
        assert_error(
            h.send(req(Method::GET, path).header("x-euchre-identity", identity.clone()))
                .await?,
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            message,
        )
        .await?;
    }
    // The browser profile of a guest also reaches the actor.
    assert_error(
        h.send(
            req(Method::GET, "/api/games?profile=browser").header("cookie", h.cookie("guest-a")?),
        )
        .await?,
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        SCRAPBOOK_UNAVAILABLE,
    )
    .await
}

#[tokio::test]
async fn profile_writes_need_csrf_and_an_identity() -> anyhow::Result<()> {
    let h = harness()?;
    let identity = h.identity_token("ann@example.com")?;
    let patch = json!({ "displayName": "Ann" });
    for method in [Method::PUT, Method::PATCH] {
        assert_error(
            h.send(
                req(method.clone(), "/api/profile/settings")
                    .header("x-euchre-identity", identity.clone())
                    .json(&patch),
            )
            .await?,
            StatusCode::FORBIDDEN,
            "forbidden",
            "Origin not allowed",
        )
        .await?;
        assert_error(
            h.send(
                req(method.clone(), "/api/profile/settings")
                    .origin()
                    .json(&patch),
            )
            .await?,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            SAVE_NEEDS_IDENTITY,
        )
        .await?;
        assert_error(
            h.send(
                req(method, "/api/profile/settings")
                    .origin()
                    .header("x-euchre-identity", identity.clone())
                    .json(&patch),
            )
            .await?,
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            SETTINGS_UNAVAILABLE,
        )
        .await?;
    }
    assert_error(
        h.send(
            req(Method::DELETE, "/api/episodes/verna.m1.0.bid")
                .header("x-euchre-identity", identity.clone()),
        )
        .await?,
        StatusCode::FORBIDDEN,
        "forbidden",
        "Origin not allowed",
    )
    .await?;
    assert_error(
        h.send(
            req(Method::DELETE, "/api/episodes/verna.m1.0.bid")
                .header("authorization", h.bearer("guest-a")?)
                .header("x-euchre-identity", identity),
        )
        .await?,
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        SCRAPBOOK_UNAVAILABLE,
    )
    .await
}

#[tokio::test]
async fn malformed_ids_are_refused_before_any_call() -> anyhow::Result<()> {
    let h = harness()?;
    let identity = h.identity_token("ann@example.com")?;
    let long = "a".repeat(129);
    for path in ["/api/games/has.dot", &format!("/api/games/{long}")] {
        assert_error(
            h.send(req(Method::GET, path).header("x-euchre-identity", identity.clone()))
                .await?,
            StatusCode::BAD_REQUEST,
            "bad_request",
            BAD_ID,
        )
        .await?;
    }
    // D30: only `role.matchId.handNo.kind` ids reach the profile.
    for path in [
        "/api/episodes/has%20space",
        "/api/episodes/ep-1",
        "/api/episodes/verna.m1.03.bid",
        "/api/episodes/verna.m1.0.nope",
    ] {
        assert_error(
            h.send(
                req(Method::DELETE, path)
                    .origin()
                    .header("x-euchre-identity", identity.clone()),
            )
            .await?,
            StatusCode::BAD_REQUEST,
            "bad_request",
            BAD_ID,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn presets_list_the_seven_personas() -> anyhow::Result<()> {
    let h = harness()?;
    let res = h.send(req(Method::GET, "/api/presets")).await?;
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await?;
    assert_eq!(body.as_array().map(Vec::len), Some(7));
    Ok(())
}

// ---- /rivet/** admission -----------------------------------------------------------

const ACTOR: &str = "d1xbeec3w8bqkydvu2fvp6hkqwcl00";

#[tokio::test]
async fn rivet_metadata_names_the_endpoint_and_namespace() -> anyhow::Result<()> {
    let h = harness()?;
    let res = h.send(req(Method::GET, "/rivet/metadata")).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header_str(res.headers(), "cache-control"), Some("no-store"));
    assert_eq!(
        body_text(res).await?,
        r#"{"clientEndpoint":"https://euchre.sh/rivet","clientNamespace":"default"}"#
    );
    Ok(())
}

#[tokio::test]
async fn rivet_tightening_refuses_query_targets_and_admin_paths() -> anyhow::Result<()> {
    let h = harness()?;
    for path in [
        "/rivet",
        "/rivet/",
        "/rivet/namespaces",
        "/rivet/actors",
        "/rivet/gateway/euchreTable/action/snapshot?rvt-method=getOrCreate",
        &format!("/rivet/gateway/{ACTOR}/action/snapshot?rvt-namespace=other"),
    ] {
        let res = h
            .send(req(Method::GET, path).header("cookie", h.cookie("guest-a")?))
            .await?;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(body_text(res).await?, "Not found", "{path}");
    }
    // Admitted by the tightening, refused by the proxy allowlist.
    let res = h
        .send(
            req(
                Method::GET,
                &format!("/rivet/gateway/{ACTOR}/inspector/state"),
            )
            .header("cookie", h.cookie("guest-a")?),
        )
        .await?;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_text(res).await?, r#"{"error":"not_found"}"#);
    Ok(())
}

#[tokio::test]
async fn rivet_tightening_refuses_dot_segments_and_encoded_query_keys() -> anyhow::Result<()> {
    let h = harness()?;
    for (method, probe) in crate::gateway::tests::DOT_SEGMENT_PROBES {
        let path = probe.replace("{id}", ACTOR);
        let method = Method::from_bytes(method.as_bytes())?;
        for r in [
            req(method.clone(), &path).header("x-rivet-token", "anything"),
            req(method.clone(), &path).header("cookie", h.cookie("guest-a")?),
        ] {
            let res = h.send(r).await?;
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
            assert_eq!(body_text(res).await?, "Not found", "{path}");
        }
    }
    let path = format!(
        "/rivet/gateway/{ACTOR}/action/snapshot?rvt%2Dnamespace=default&rvt%2Dmethod=getOrCreate&rvt%2Dkey=evil"
    );
    let res = h
        .send(req(Method::POST, &path).header("x-rivet-token", "anything"))
        .await?;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_text(res).await?, "Not found");
    Ok(())
}

#[tokio::test]
async fn rivet_admission_checks_origin_then_caller() -> anyhow::Result<()> {
    let h = harness()?;
    let path = format!("/rivet/gateway/{ACTOR}/action/snapshot");
    let res = h
        .send(
            req(Method::POST, &path)
                .header("origin", "https://evil.example")
                .header("cookie", h.cookie("guest-a")?),
        )
        .await?;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_text(res).await?, r#"{"error":"origin_not_allowed"}"#);

    // A browser (Origin present) needs a session, even with a gateway token.
    let with_token = format!("/rivet/gateway/{ACTOR}@eyJ0.e30.sig/action/snapshot");
    for r in [
        req(Method::POST, &path).origin(),
        req(Method::POST, &with_token).origin(),
        // A native caller with an invalid bearer.
        req(Method::POST, &with_token).header("authorization", "Bearer nope"),
        // A native caller with nothing.
        req(Method::POST, &path),
    ] {
        let res = h.send(r).await?;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(body_text(res).await?, "Unauthorized");
    }

    // Admitted callers reach the (unreachable) engine: the proxy's 404.
    for r in [
        req(Method::POST, &path)
            .origin()
            .header("cookie", h.cookie("guest-a")?),
        req(Method::POST, &with_token),
        req(Method::POST, &path).header("authorization", h.bearer("guest-a")?),
    ] {
        let res = h.send(r).await?;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_text(res).await?, r#"{"error":"upstream_unreachable"}"#);
    }
    Ok(())
}

#[tokio::test]
async fn rivet_gateway_is_rate_limited() -> anyhow::Result<()> {
    let h = harness_with(|cfg| cfg.rate_limits.gateway = BucketConfig::new(1, 1))?;
    let path = format!("/rivet/gateway/{ACTOR}/action/snapshot");
    let r = || -> anyhow::Result<Req> {
        Ok(req(Method::POST, &path).header("authorization", h.bearer("guest-a")?))
    };
    assert_eq!(h.send(r()?).await?.status(), StatusCode::NOT_FOUND);
    assert_eq!(h.send(r()?).await?.status(), StatusCode::TOO_MANY_REQUESTS);
    Ok(())
}

// ---- Static build ------------------------------------------------------------------

fn static_harness() -> anyhow::Result<(Harness, tempfile::TempDir)> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    std::fs::create_dir_all(root.join("_app/immutable"))?;
    std::fs::write(root.join("index.html"), "<!doctype html>index")?;
    std::fs::write(root.join("favicon.svg"), "<svg/>")?;
    std::fs::write(root.join("_app/immutable/app.js"), "console.log(1)")?;
    std::fs::write(root.join("_app/immutable/app.js.br"), "BROTLI")?;
    std::fs::write(root.join("_app/immutable/app.js.map"), "{}")?;
    let path = root.to_path_buf();
    let h = harness_with(move |cfg| cfg.static_dir = Some(path))?;
    Ok((h, dir))
}

#[tokio::test]
async fn static_files_cache_headers_and_precompression() -> anyhow::Result<()> {
    let (h, _dir) = static_harness()?;
    let res = h.send(req(Method::GET, "/favicon.svg")).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        header_str(res.headers(), "cache-control"),
        Some("public,max-age=86400")
    );

    let res = h.send(req(Method::GET, "/_app/immutable/app.js")).await?;
    assert_eq!(
        header_str(res.headers(), "cache-control"),
        Some("public,max-age=31536000,immutable")
    );
    assert_eq!(header_str(res.headers(), "content-encoding"), None);
    assert_eq!(body_text(res).await?, "console.log(1)");

    let res = h
        .send(req(Method::GET, "/_app/immutable/app.js").header("accept-encoding", "br, gzip"))
        .await?;
    assert_eq!(header_str(res.headers(), "content-encoding"), Some("br"));
    assert_eq!(header_str(res.headers(), "vary"), Some("Accept-Encoding"));
    assert_eq!(body_text(res).await?, "BROTLI");

    let res = h.send(req(Method::HEAD, "/favicon.svg")).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_text(res).await?, "");
    Ok(())
}

#[tokio::test]
async fn static_files_fallback_and_refusals() -> anyhow::Result<()> {
    let (h, _dir) = static_harness()?;
    for path in ["/", "/play/abc", "/scrapbook"] {
        let res = h.send(req(Method::GET, path)).await?;
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        assert_eq!(body_text(res).await?, "<!doctype html>index", "{path}");
    }
    let res = h.send(req(Method::GET, "/play/abc")).await?;
    assert_eq!(header_str(res.headers(), "cache-control"), Some("no-cache"));

    for r in [
        req(Method::GET, "/missing.js"),
        req(Method::GET, "/_app/immutable/app.js.map"),
        req(Method::POST, "/play"),
    ] {
        let res = h.send(r).await?;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_text(res).await?, "Not found");
    }
    let res = h.send(req(Method::GET, "/%E0%A4%A")).await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_text(res).await?, "Bad path");
    // Traversal never leaves the directory.
    // (An extensionless miss is the SPA fallback, never the target file.)
    let res = h.send(req(Method::GET, "/..%2F..%2Fetc%2Fpasswd")).await?;
    if res.status() == StatusCode::OK {
        assert_eq!(body_text(res).await?, "<!doctype html>index");
    }
    let res = h
        .send(req(Method::GET, "/..%2F..%2Fetc%2Fhosts.txt"))
        .await?;
    assert_ne!(res.status(), StatusCode::OK);
    Ok(())
}

/// W4 deploy review: `/index.html` fetched by name is the same no-cache shell as
/// the SPA fallback, every HTML response refuses framing, and every response
/// (`/health`, `421`, the API, assets, `404`) carries `nosniff` and a referrer
/// policy.
#[tokio::test]
async fn static_html_is_no_cache_and_unframeable_and_every_response_is_nosniff()
-> anyhow::Result<()> {
    let (h, _dir) = static_harness()?;
    for path in ["/", "/index.html", "/INDEX.HTML", "/play/abc"] {
        let res = h.send(req(Method::GET, path)).await?;
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        assert_eq!(
            header_str(res.headers(), "cache-control"),
            Some("no-cache"),
            "{path}"
        );
        assert_eq!(
            header_str(res.headers(), "content-security-policy"),
            Some("frame-ancestors 'none'"),
            "{path}"
        );
        assert_eq!(
            header_str(res.headers(), "x-frame-options"),
            Some("DENY"),
            "{path}"
        );
    }
    let res = h.send(req(Method::GET, "/_app/immutable/app.js")).await?;
    assert_eq!(header_str(res.headers(), "content-security-policy"), None);
    assert_eq!(header_str(res.headers(), "x-frame-options"), None);

    for r in [
        req(Method::GET, "/health"),
        req(Method::GET, "/").host("attacker.example"),
        req(Method::GET, "/api/rivet/metadata"),
        req(Method::GET, "/api/nope"),
        req(Method::GET, "/favicon.svg"),
        req(Method::GET, "/_app/immutable/app.js"),
        req(Method::GET, "/missing.js"),
    ] {
        let res = h.send(r).await?;
        assert_eq!(
            header_str(res.headers(), "x-content-type-options"),
            Some("nosniff")
        );
        assert_eq!(
            header_str(res.headers(), "referrer-policy"),
            Some("strict-origin-when-cross-origin")
        );
    }
    Ok(())
}

/// W3 security review: the static dir never serves a dotfile or a `.map` (any
/// case, trailing slash), and a directory is an SPA route, never a redirect
/// (`//art` once answered `307 Location: //art/`, a scheme-relative URL).
#[tokio::test]
async fn static_files_hide_dotfiles_and_maps_and_never_redirect() -> anyhow::Result<()> {
    let (h, dir) = static_harness()?;
    let root = dir.path();
    std::fs::write(root.join(".env"), "DOTENV-SECRET")?;
    std::fs::create_dir_all(root.join(".git"))?;
    std::fs::write(root.join(".git/config"), "GIT-SECRET")?;
    std::fs::create_dir_all(root.join("art"))?;
    std::fs::write(root.join("art/card.png"), "PNG")?;
    for path in [
        "/.env",
        "/%2eenv",
        "/%2Eenv",
        "/.git/config",
        "/.git/",
        "/_app/immutable/app.js.map",
        "/_app/immutable/app.js.MAP",
        "/_app/immutable/app.js.Map",
        "/_app/immutable/app.js.map/",
    ] {
        let res = h.send(req(Method::GET, path)).await?;
        let status = res.status();
        let body = body_text(res).await?;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::NOT_FOUND, "Not found"),
            "{path}"
        );
    }
    for path in ["/art", "//art", "/art/", "//_app", "/"] {
        let res = h.send(req(Method::GET, path)).await?;
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        assert_eq!(header_str(res.headers(), "location"), None, "{path}");
        assert_eq!(
            header_str(res.headers(), "cache-control"),
            Some("no-cache"),
            "{path}"
        );
        assert_eq!(body_text(res).await?, "<!doctype html>index", "{path}");
    }
    let res = h.send(req(Method::GET, "/art/card.png")).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_text(res).await?, "PNG");
    Ok(())
}

// ---- Desktop downloads (crate::downloads) ------------------------------------------

/// A web build plus a downloads folder holding one mac file and its manifest.
fn downloads_harness() -> anyhow::Result<(Harness, tempfile::TempDir, tempfile::TempDir)> {
    let (_, web) = static_harness()?;
    let dl = tempfile::tempdir()?;
    let data = vec![7u8; 2048];
    std::fs::write(dl.path().join("Euchre-0.1.0-mac.dmg"), &data)?;
    let sha = {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(&data))
    };
    std::fs::write(
        dl.path().join("manifest.json"),
        json!({
            "version": "0.1.0",
            "platforms": { "mac": { "file": "Euchre-0.1.0-mac.dmg", "bytes": 2048, "sha256": sha } }
        })
        .to_string(),
    )?;
    let web_path = web.path().to_path_buf();
    let dl_path = dl.path().to_path_buf();
    let h = harness_with(move |cfg| {
        cfg.static_dir = Some(web_path);
        cfg.downloads_dir = Some(dl_path);
    })?;
    Ok((h, web, dl))
}

/// The routes sit behind the front door: the Host gate and the security headers
/// apply, `GET` and `HEAD` work, other methods are `405`, and `/download/**` is
/// never the SPA fallback.
#[tokio::test]
async fn downloads_go_through_the_front_door() -> anyhow::Result<()> {
    let (h, _web, _dl) = downloads_harness()?;

    let res = h.send(req(Method::GET, "/download/manifest.json")).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header_str(res.headers(), "cache-control"), Some("no-cache"));
    assert_eq!(
        header_str(res.headers(), "x-content-type-options"),
        Some("nosniff")
    );
    assert_eq!(
        header_str(res.headers(), "referrer-policy"),
        Some("strict-origin-when-cross-origin")
    );
    assert_eq!(
        body_json(res).await?["platforms"]["mac"]["bytes"],
        json!(2048)
    );

    for path in [
        "/download/manifest.json",
        "/download/mac",
        "/download/files/Euchre-0.1.0-mac.dmg",
    ] {
        let res = h
            .send(req(Method::GET, path).host("attacker.example"))
            .await?;
        assert_eq!(res.status(), StatusCode::MISDIRECTED_REQUEST, "{path}");
    }

    let res = h.send(req(Method::GET, "/download/mac")).await?;
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(
        header_str(res.headers(), "location"),
        Some("/download/files/Euchre-0.1.0-mac.dmg")
    );

    let res = h
        .send(req(Method::HEAD, "/download/files/Euchre-0.1.0-mac.dmg"))
        .await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header_str(res.headers(), "content-length"), Some("2048"));
    assert_eq!(
        header_str(res.headers(), "content-disposition"),
        Some("attachment; filename=\"Euchre-0.1.0-mac.dmg\"")
    );
    assert_eq!(
        header_str(res.headers(), "x-content-type-options"),
        Some("nosniff")
    );
    assert_eq!(body_text(res).await?, "");

    let res = h
        .send(req(Method::GET, "/download/files/Euchre-0.1.0-mac.dmg"))
        .await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(res.into_body(), 1 << 22).await?.len(),
        2048
    );

    for method in [Method::POST, Method::PUT, Method::DELETE] {
        let res = h.send(req(method.clone(), "/download/mac")).await?;
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED, "{method}");
    }

    // Absent platforms and unknown paths are 404, not the SPA shell.
    for path in [
        "/download/linux",
        "/download/files/x.dmg",
        "/download/nope/deeper",
    ] {
        let res = h.send(req(Method::GET, path)).await?;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(body_text(res).await?, "Not found", "{path}");
    }
    Ok(())
}

#[tokio::test]
async fn without_a_downloads_folder_the_manifest_is_empty() -> anyhow::Result<()> {
    let h = harness()?;
    let res = h.send(req(Method::GET, "/download/manifest.json")).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        body_json(res).await?,
        json!({ "version": null, "platforms": {} })
    );
    let res = h.send(req(Method::GET, "/download/mac")).await?;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    Ok(())
}

/// A `304` carries no `Content-Length`, on `GET` or `HEAD` (RFC 9110 8.6): axum
/// adds one from an exact body size, so the handlers send an unsized body.
#[tokio::test]
async fn not_modified_has_no_content_length() -> anyhow::Result<()> {
    let (h, _web, _dl) = downloads_harness()?;
    for method in [Method::GET, Method::HEAD] {
        let res = h
            .send(
                req(method.clone(), "/download/files/Euchre-0.1.0-mac.dmg")
                    .header("if-none-match", "*"),
            )
            .await?;
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED, "{method}");
        assert_eq!(
            header_str(res.headers(), "content-length"),
            None,
            "{method}"
        );

        let res = h.send(req(method.clone(), "/favicon.svg")).await?;
        let modified = header_str(res.headers(), "last-modified")
            .unwrap_or_default()
            .to_owned();
        let res = h
            .send(req(method.clone(), "/favicon.svg").header("if-modified-since", modified))
            .await?;
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED, "{method}");
        assert_eq!(
            header_str(res.headers(), "content-length"),
            None,
            "{method}"
        );
    }
    Ok(())
}

/// Static files ignore a list of ranges (whole file, 200) and keep one range.
#[tokio::test]
async fn static_files_serve_one_range_and_ignore_lists() -> anyhow::Result<()> {
    let (h, _dir) = static_harness()?;
    let res = h
        .send(req(Method::GET, "/favicon.svg").header("range", "bytes=0-1"))
        .await?;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(body_text(res).await?, "<s");
    let huge = (0..3000)
        .map(|i| format!("{}-{}", i * 2, i * 2))
        .collect::<Vec<_>>()
        .join(",");
    for range in [
        "bytes=0-1,3-4",
        "lines=0-1",
        format!("bytes={huge}").as_str(),
    ] {
        let res = h
            .send(req(Method::GET, "/favicon.svg").header("range", range))
            .await?;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_text(res).await?, "<svg/>");
    }
    Ok(())
}

/// The router hands the client IP to the download limits: each peer has its
/// own byte budget.
#[tokio::test]
async fn the_download_budget_is_per_client() -> anyhow::Result<()> {
    let (_, web) = static_harness()?;
    let (_, _web2, dl) = downloads_harness()?;
    let web_path = web.path().to_path_buf();
    let dl_path = dl.path().to_path_buf();
    let h = harness_with(move |cfg| {
        cfg.static_dir = Some(web_path);
        cfg.downloads_dir = Some(dl_path);
        cfg.rate_limits.download = BucketConfig::new(1, 1);
    })?;
    let file = "/download/files/Euchre-0.1.0-mac.dmg";
    let res = h
        .send(req(Method::GET, file).peer("203.0.113.1:5000")?)
        .await?;
    assert_eq!(res.status(), StatusCode::OK);
    let res = h
        .send(req(Method::GET, file).peer("203.0.113.1:5001")?)
        .await?;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(header_str(res.headers(), "retry-after").is_some());
    assert_eq!(
        header_str(res.headers(), "x-content-type-options"),
        Some("nosniff")
    );
    let res = h
        .send(req(Method::GET, file).peer("203.0.113.2:5000")?)
        .await?;
    assert_eq!(res.status(), StatusCode::OK);
    Ok(())
}
