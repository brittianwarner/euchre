//! Port of `server/rivet-gateway-proxy.test.ts`. The classifier and origin tests are
//! pure. The relay tests run a mock upstream engine (an axum WebSocket echo plus an
//! HTTP recorder) and the proxy on ephemeral loopback ports.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Request};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message as TMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::*;
use crate::config::GatewayOptions;

// ---- Pure: guard rate-limit key derivation -----------------------------------------

#[test]
fn derive_takes_the_last_forwarded_for_entry() {
    // A client-forged first entry must never win: the Guard would key its
    // rate-limit bucket on it.
    assert_eq!(
        derive_trusted_client_ip(Some("6.6.6.6, 203.0.113.9"), Some("10.0.0.1")).as_deref(),
        Some("203.0.113.9")
    );
    assert_eq!(
        derive_trusted_client_ip(Some("203.0.113.9"), Some("10.0.0.1")).as_deref(),
        Some("203.0.113.9")
    );
    assert_eq!(
        derive_trusted_client_ip(Some("2001:db8::1, 2001:db8::2"), Some("10.0.0.1")).as_deref(),
        Some("2001:db8::2")
    );
}

#[test]
fn derive_falls_back_to_the_socket_ip_when_the_header_is_absent_or_garbage() {
    for header in [
        None,
        Some("not-an-ip"),
        Some("a, b, still-not-an-ip"),
        Some(""),
    ] {
        assert_eq!(
            derive_trusted_client_ip(header, Some("10.0.0.1")).as_deref(),
            Some("10.0.0.1"),
            "{header:?}"
        );
    }
}

#[test]
fn derive_is_none_when_nothing_trustworthy_exists() {
    assert_eq!(derive_trusted_client_ip(None, None), None);
    assert_eq!(
        derive_trusted_client_ip(Some("garbage"), Some("also-garbage")),
        None
    );
}

#[test]
fn derive_tolerates_ip_port_and_bracketed_v6_port_entries() {
    let socket = Some("10.0.0.1");
    let cases = [
        ("6.6.6.6, 203.0.113.9:8080", "203.0.113.9"),
        ("6.6.6.6, [2001:db8::2]:443", "2001:db8::2"),
        ("[2001:db8::7]", "2001:db8::7"),
        // A bare IPv6 has colons but is not ip:port; it must not be mangled.
        ("2001:db8::9", "2001:db8::9"),
        // Unresolvable even after stripping: the socket fallback.
        ("[not-an-ip]:443", "10.0.0.1"),
    ];
    for (header, expected) in cases {
        assert_eq!(
            derive_trusted_client_ip(Some(header), socket).as_deref(),
            Some(expected),
            "{header}"
        );
    }
}

#[test]
fn cap_ws_close_reason_caps_by_utf8_bytes_without_splitting_a_character() {
    let multibyte = "\u{e9}".repeat(107);
    let capped = cap_ws_close_reason(&multibyte);
    assert!(capped.len() <= 123);
    assert_eq!(capped, "\u{e9}".repeat(61));
}

#[test]
fn cap_ws_close_reason_passes_ascii_within_the_limit() {
    assert_eq!(
        cap_ws_close_reason("upstream error: guard.rate_limit#abc"),
        "upstream error: guard.rate_limit#abc"
    );
    assert_eq!(cap_ws_close_reason(&"x".repeat(200)), "x".repeat(123));
}

#[test]
fn close_codes_normalize_to_sendable_values() {
    assert_eq!(normalized_close_code(1000, 1001), 1000);
    assert_eq!(normalized_close_code(1011, 1001), 1011);
    assert_eq!(normalized_close_code(4000, 1001), 4000);
    // 1005/1006/1015 can never be put in a close frame.
    for code in [1005, 1006, 1015, 999, 5000] {
        assert!(!is_sendable_close_code(code), "{code}");
        assert_eq!(normalized_close_code(code, 1001), 1001);
    }
}

#[test]
fn redact_path_hides_credentials() {
    assert_eq!(
        redact_path("/actor-1@secret/connect?rvt-token=abc&x=1"),
        "/actor-1@<redacted>/connect?rvt-token=<redacted>&x=1"
    );
}

// ---- Pure: allowlist classifier ----------------------------------------------------

fn ws(path: &str) -> ProxyRouteKind {
    classify_proxy_path(path, "GET", true, "/rivet").kind
}

fn ws_m(path: &str, method: &str) -> ProxyRouteKind {
    classify_proxy_path(path, method, true, "/rivet").kind
}

fn http(path: &str) -> ProxyRouteKind {
    classify_proxy_path(path, "GET", false, "/rivet").kind
}

fn http_m(path: &str, method: &str) -> ProxyRouteKind {
    classify_proxy_path(path, method, false, "/rivet").kind
}

#[test]
fn classify_allows_the_client_wire_contract() {
    use ProxyRouteKind::{Deny, Http, Metadata, Ws};
    assert_eq!(http("/rivet/metadata"), Metadata);
    assert_eq!(ws("/rivet/connect"), Ws);
    assert_eq!(http("/rivet/connect"), Deny);
    assert_eq!(ws("/rivet/gateway/user/connect"), Ws);
    assert_eq!(ws("/rivet/gateway/actor-abc@tok123/connect"), Ws);
    assert_eq!(ws("/rivet/gateway/user/websocket"), Ws);
    assert_eq!(ws("/rivet/gateway/user/websocket/custom/path"), Ws);
    assert_eq!(
        http_m("/rivet/gateway/user/action/getProfile", "POST"),
        Http
    );
    assert_eq!(http_m("/rivet/gateway/user/queue/jobs", "POST"), Http);
    assert_eq!(http_m("/rivet/gateway/user/request", "POST"), Http);
    assert_eq!(
        http_m("/rivet/gateway/user/request/custom/path", "GET"),
        Http
    );
    // CORS preflight on allowlisted paths rides through to the engine.
    assert_eq!(
        http_m("/rivet/gateway/user/action/getProfile", "OPTIONS"),
        Http
    );
}

#[test]
fn classify_denies_every_admin_surface_that_shares_the_guard_port() {
    let denied = [
        ("/rivet/namespaces", "GET"),
        ("/rivet/namespaces", "POST"),
        ("/rivet/runner-configs/default", "PUT"),
        ("/rivet/runner-configs", "GET"),
        ("/rivet/pools/default", "PUT"),
        ("/rivet/auth/tokens", "POST"),
        ("/rivet/datacenters", "GET"),
        ("/rivet/", "GET"),
        ("/rivet", "GET"),
        ("/rivet/ui", "GET"),
        ("/rivet/gateway/actor-1/inspector/state", "GET"),
        ("/rivet/gateway/actor-1/health", "GET"),
        ("/rivet/actors/actor-1/kv/keys/k", "GET"),
        ("/rivet/actors", "GET"),
        ("/rivet/actors", "PUT"),
        ("/rivet/actors", "POST"),
        ("/rivet/actors/actor-123", "DELETE"),
        ("/rivet/gateway/user/dynamic/reload", "PUT"),
    ];
    for (path, method) in denied {
        assert_eq!(
            http_m(path, method),
            ProxyRouteKind::Deny,
            "{method} {path}"
        );
    }
}

#[test]
fn classify_denies_malformed_and_edge_paths() {
    use ProxyRouteKind::Deny;
    assert_eq!(http("/other/actors"), Deny);
    assert_eq!(http("/rivet/actors/a/b"), Deny);
    assert_eq!(http_m("/rivet/metadata", "POST"), Deny);
    assert_eq!(http("/rivet/gateway/user/connect"), Deny);
    assert_eq!(ws_m("/rivet/gateway/user/action/x", "POST"), Deny);
    assert_eq!(http("/rivet/gateway/user/connect/extra"), Deny);
    // Prefix lookalikes are outside the mount.
    assert_eq!(ws("/rivetx/gateway/user/connect"), Deny);
}

#[test]
fn classify_denies_dot_segments_and_encoded_separators() {
    use ProxyRouteKind::Deny;
    for path in [
        "/rivet/gateway/x/request/../../actors",
        "/rivet/gateway/x/request/%2e%2e/%2E%2E/namespaces",
        "/rivet/gateway/x/request/.%2E/.%2e/../auth/tokens",
        "/rivet/gateway/x/request/%2E./inspector/ui/",
        "/rivet/gateway/x/request/../../euchreTable/action/snapshot",
        "/rivet/gateway/x/request/./health",
        "/rivet/gateway/x/request/a%2F..%2F..%2Factors",
        "/rivet/gateway/x/request/a%5c..%5cactors",
        "/rivet/gateway/x/request/a\\b",
        "/rivet/gateway/x/request/%2",
        "/rivet/gateway/x/action/..",
    ] {
        assert_eq!(http_m(path, "POST"), Deny, "{path}");
        assert_eq!(http(path), Deny, "{path}");
    }
    for path in [
        "/rivet/gateway/x/websocket/../../../actors",
        "/rivet/gateway/x/websocket/%2E%2e/%2e%2E/namespaces",
        "/rivet/gateway/x/websocket/./x",
        "/rivet/gateway/%2e%2e/connect",
    ] {
        assert_eq!(ws(path), Deny, "{path}");
    }
    // The token part of the actor segment may carry an encoded `/`; other dots
    // are ordinary characters.
    assert_eq!(
        ws("/rivet/gateway/actor@ab%2Fcd/connect"),
        ProxyRouteKind::Ws
    );
    assert_eq!(
        http("/rivet/gateway/x/request/a..b/v1.2/file.json"),
        ProxyRouteKind::Http
    );
}

#[test]
fn verified_upstream_url_refuses_any_rewrite() {
    let ok = verified_upstream_url("http://127.0.0.1:6420", "/gateway/x/action/a", "?q=1");
    assert_eq!(
        ok.map(String::from).as_deref(),
        Some("http://127.0.0.1:6420/gateway/x/action/a?q=1")
    );
    assert!(verified_upstream_url("ws://127.0.0.1:6420", "/gateway/x/connect", "").is_some());
    for path in [
        "/gateway/x/request/../../actors",
        "/gateway/x/request/%2e%2E/y",
    ] {
        assert!(
            verified_upstream_url("http://127.0.0.1:6420", path, "").is_none(),
            "{path}"
        );
        assert!(
            verified_upstream_url("ws://127.0.0.1:6420", path, "").is_none(),
            "{path}"
        );
    }
}

#[test]
fn stripped_request_headers_cover_the_identity_credential() {
    assert!(
        HOP_BY_HOP_REQ.contains(
            &euchre_server_core::auth::identity::IDENTITY_HEADER
                .to_ascii_lowercase()
                .as_str()
        )
    );
    for name in HOP_BY_HOP_REQ {
        assert_eq!(name, name.to_ascii_lowercase(), "{name}");
    }
}

#[test]
fn classify_strips_the_prefix_for_the_upstream_path() {
    let route = classify_proxy_path("/rivet/gateway/user/connect", "GET", true, "/rivet");
    assert_eq!(route.upstream_path, "/gateway/user/connect");
}

#[test]
fn is_origin_allowed_cases() {
    let list = AllowedOrigins::List(vec!["https://layerr.ai".into()]);
    // An absent origin passes (non-browser clients).
    assert!(is_origin_allowed(None, &list));
    assert!(is_origin_allowed(Some("https://layerr.ai"), &list));
    assert!(!is_origin_allowed(Some("https://evil.com"), &list));
    assert!(is_origin_allowed(
        Some("https://anything.com"),
        &AllowedOrigins::Any
    ));
}

#[test]
fn to_web_socket_base_cases() {
    assert_eq!(to_web_socket_base("http://engine:6420"), "ws://engine:6420");
    assert_eq!(
        to_web_socket_base("https://engine:6420"),
        "wss://engine:6420"
    );
}

#[test]
fn websocket_admission_uses_explicit_payload_and_backpressure_ceilings() {
    let defaults = GatewayOptions::default();
    assert_eq!(defaults.max_payload_bytes, 16 * 1024 * 1024);
    assert_eq!(defaults.max_buffered_bytes, 16 * 1024 * 1024);
    assert_eq!(defaults.ping_interval, Duration::from_secs(30));
    assert_eq!(defaults.idle_timeout, Duration::from_secs(960));
    let proxy = GatewayProxy::new(
        GatewayProxyOptions {
            tunables: GatewayOptions {
                max_buffered_bytes: 2 * 1024 * 1024,
                ..GatewayOptions::default()
            },
            ..GatewayProxyOptions::new(
                "http://127.0.0.1:6420",
                AllowedOrigins::List(vec!["http://localhost:4000".into()]),
            )
        },
        reqwest::Client::new(),
    );
    assert_eq!(proxy.tunables().max_buffered_bytes, 2 * 1024 * 1024);
}

#[test]
fn offered_protocols_split_trim_and_drop_empties() -> anyhow::Result<()> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "sec-websocket-protocol",
        "rivet, rivet_encoding.bare ,,rivet_conn_params.%7B%7D".parse()?,
    );
    assert_eq!(
        offered_protocols(&headers),
        ["rivet", "rivet_encoding.bare", "rivet_conn_params.%7B%7D"]
    );
    Ok(())
}

// ---- Integration: mock upstream engine + proxy over real sockets --------------------

#[derive(Debug, Clone)]
struct RecordedUpgrade {
    protocols: Vec<String>,
    origin: Option<String>,
    path: String,
    xff: Option<String>,
    extensions: Option<String>,
}

#[derive(Default)]
struct Upstream {
    /// Every path (with query) the upstream received, upgrades included.
    seen: Mutex<Vec<String>>,
    upgrades: Mutex<Vec<RecordedUpgrade>>,
    query_actions: Mutex<Vec<(String, Option<String>)>>,
    closes: Mutex<Vec<(u16, String)>>,
    close_cmd: Mutex<Option<mpsc::UnboundedSender<(u16, String)>>>,
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

async fn upstream_handler(rec: Arc<Upstream>, req: Request) -> Response {
    let path = req.uri().path().to_owned();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map_or_else(|| path.clone(), |pq| pq.as_str().to_owned());
    let headers = req.headers().clone();
    rec.seen.lock().push(path_and_query.clone());
    if path.starts_with("/gateway/") && is_websocket_upgrade(&headers) {
        rec.upgrades.lock().push(RecordedUpgrade {
            protocols: offered_protocols(&headers),
            origin: header_string(&headers, "origin"),
            path: path_and_query,
            xff: header_string(&headers, "x-forwarded-for"),
            extensions: header_string(&headers, "sec-websocket-extensions"),
        });
        let (mut parts, _) = req.into_parts();
        return match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
            Ok(upgrade) => upgrade
                .protocols(["rivet"])
                .on_upgrade(move |socket| echo(socket, rec))
                .into_response(),
            Err(_) => (StatusCode::BAD_REQUEST, "upgrade failed").into_response(),
        };
    }
    let method = req.method().clone();
    if path == "/gateway/broken/action/snapshot" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({
                "code": "internal_error",
                "group": "core",
                "message": "An internal error occurred",
                "metadata": { "error": "unsupported ID version: 146" }
            })),
        )
            .into_response();
    }
    if path == "/gateway/plain500/action/snapshot" {
        return (StatusCode::INTERNAL_SERVER_ERROR, "plain failure").into_response();
    }
    if path == "/gateway/example/action/ping" && method == "POST" {
        rec.query_actions.lock().push((
            path_and_query,
            header_string(&headers, "x-rivet-conn-params"),
        ));
        return axum::Json(json!({ "output": "pong" })).into_response();
    }
    if path == "/gateway/user/action/getProfile" && method == "POST" {
        let search = req
            .uri()
            .query()
            .map_or_else(String::new, |q| format!("?{q}"));
        let body = axum::body::to_bytes(req.into_body(), 1 << 20)
            .await
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let h = |name: &str| header_string(&headers, name);
        return axum::Json(json!({
            "method": method.as_str(),
            "search": search,
            "body": body,
            "connParams": h("x-rivet-conn-params"),
            "authorization": h("authorization"),
            "cookie": h("cookie"),
            "xff": h("x-forwarded-for"),
            "forwarded": h("forwarded"),
            "xRealIp": h("x-real-ip"),
            "trueClientIp": h("true-client-ip"),
            "identity": h("x-euchre-identity"),
            "forwardedHost": h("x-forwarded-host"),
            "forwardedProto": h("x-forwarded-proto"),
        }))
        .into_response();
    }
    (StatusCode::UNAUTHORIZED, "engine admin surface").into_response()
}

async fn echo(mut socket: WebSocket, rec: Arc<Upstream>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<(u16, String)>();
    *rec.close_cmd.lock() = Some(tx);
    loop {
        tokio::select! {
            cmd = rx.recv() => {
                if let Some((code, reason)) = cmd {
                    let _ = socket
                        .send(Message::Close(Some(CloseFrame { code, reason: reason.into() })))
                        .await;
                }
                return;
            }
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(frame))) => {
                    let (code, reason) = frame
                        .map_or((1005, String::new()), |f| (f.code, f.reason.to_string()));
                    rec.closes.lock().push((code, reason));
                    return;
                }
                Some(Ok(msg @ (Message::Text(_) | Message::Binary(_)))) => {
                    if socket.send(msg).await.is_err() {
                        return;
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => {
                    rec.closes.lock().push((1006, String::new()));
                    return;
                }
            }
        }
    }
}

async fn serve(router: Router) -> anyhow::Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    Ok(addr)
}

struct Fixture {
    upstream: Arc<Upstream>,
    upstream_addr: SocketAddr,
    proxy: SocketAddr,
}

const ALLOWED: &str = "http://localhost:4000";

impl Fixture {
    async fn start() -> anyhow::Result<Self> {
        Self::with(GatewayOptions::default()).await
    }

    async fn with(tunables: GatewayOptions) -> anyhow::Result<Self> {
        let upstream = Arc::new(Upstream::default());
        let rec = Arc::clone(&upstream);
        let upstream_addr = serve(Router::new().fallback(move |req: Request| {
            let rec = Arc::clone(&rec);
            upstream_handler(rec, req)
        }))
        .await?;
        let proxy = Self::proxy_for(upstream_addr, tunables).await?;
        Ok(Self {
            upstream,
            upstream_addr,
            proxy,
        })
    }

    async fn proxy_for(
        upstream: SocketAddr,
        tunables: GatewayOptions,
    ) -> anyhow::Result<SocketAddr> {
        let proxy = GatewayProxy::new(
            GatewayProxyOptions {
                tunables,
                ..GatewayProxyOptions::new(
                    format!("http://{upstream}"),
                    AllowedOrigins::List(vec![ALLOWED.into()]),
                )
            },
            GatewayProxy::http_client()?,
        );
        serve(
            proxy
                .router()
                .fallback(|| async { (StatusCode::NOT_FOUND, "elysia would be here") }),
        )
        .await
    }

    fn http(&self, path: &str) -> String {
        format!("http://{}{path}", self.proxy)
    }

    fn upgrades(&self) -> Vec<RecordedUpgrade> {
        self.upstream.upgrades.lock().clone()
    }

    async fn wait_upgrades(&self, n: usize) -> anyhow::Result<()> {
        wait_for(|| self.upstream.upgrades.lock().len() >= n).await
    }

    fn close_upstream(&self, code: u16, reason: &str) -> anyhow::Result<()> {
        let tx = self
            .upstream
            .close_cmd
            .lock()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no upstream socket"))?;
        tx.send((code, reason.to_owned()))?;
        Ok(())
    }
}

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn connect_ws(
    url: &str,
    protocols: &[&str],
    origin: Option<&str>,
    headers: &[(&str, &str)],
) -> anyhow::Result<Client> {
    let mut req = url.into_client_request()?;
    let h = req.headers_mut();
    if !protocols.is_empty() {
        h.insert("sec-websocket-protocol", protocols.join(", ").parse()?);
    }
    if let Some(origin) = origin {
        h.insert("origin", origin.parse()?);
    }
    for (name, value) in headers {
        h.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes())?,
            value.parse()?,
        );
    }
    let (ws, _) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(req),
    )
    .await??;
    Ok(ws)
}

/// The browser leg upgrades first; the proxy's upstream dial lands a tick later.
async fn wait_for(cond: impl Fn() -> bool) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if cond() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    anyhow::bail!("wait_for timed out")
}

async fn next_close(ws: &mut Client) -> anyhow::Result<(u16, String)> {
    let deadline = Duration::from_secs(5);
    loop {
        match tokio::time::timeout(deadline, ws.next()).await? {
            Some(Ok(TMessage::Close(frame))) => {
                return Ok(frame.map_or((1005, String::new()), |f| {
                    (u16::from(f.code), f.reason.to_string())
                }));
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => anyhow::bail!("ws error before close: {e}"),
            None => anyhow::bail!("ws ended without a close frame"),
        }
    }
}

async fn next_data(ws: &mut Client) -> anyhow::Result<TMessage> {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await? {
            Some(Ok(msg @ (TMessage::Text(_) | TMessage::Binary(_)))) => return Ok(msg),
            Some(Ok(_)) => {}
            Some(Err(e)) => anyhow::bail!("ws error: {e}"),
            None => anyhow::bail!("ws ended"),
        }
    }
}

#[tokio::test]
async fn browser_client_actions_use_the_query_gateway_without_actors() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let client = rivetkit_client::Client::new(
        rivetkit_client::ClientConfig::new(fx.http("/rivet"))
            .namespace("default")
            .disable_metadata_lookup(true)
            .encoding(rivetkit_client::EncodingKind::Json),
    );
    let handle = client.get_or_create(
        "example",
        vec!["example".into(), "key-1".into()],
        rivetkit_client::GetOrCreateOptions {
            params: Some(json!({ "authToken": "signed-actor-token" })),
            ..rivetkit_client::GetOrCreateOptions::default()
        },
    )?;
    let result =
        tokio::time::timeout(Duration::from_secs(10), handle.action("ping", vec![])).await??;
    assert_eq!(result, json!("pong"));
    let actions = fx.upstream.query_actions.lock().clone();
    assert_eq!(actions.len(), 1);
    assert!(
        actions[0].0.contains("/gateway/example/action/ping?"),
        "{}",
        actions[0].0
    );
    assert!(
        actions[0].0.contains("rvt-method=getOrCreate"),
        "{}",
        actions[0].0
    );
    assert_eq!(
        actions[0].1.as_deref(),
        Some(r#"{"authToken":"signed-actor-token"}"#)
    );
    Ok(())
}

#[tokio::test]
async fn ws_relay_echoes_text_and_binary_and_forwards_protocols_and_origin() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let mut ws = connect_ws(
        &format!(
            "ws://{}/rivet/gateway/user/connect?rvt-namespace=default&rvt-method=getOrCreate",
            fx.proxy
        ),
        &["rivet", "rivet_encoding.bare", "rivet_conn_params.%7B%7D"],
        Some(ALLOWED),
        &[],
    )
    .await?;
    fx.wait_upgrades(1).await?;
    let up = &fx.upgrades()[0];
    assert_eq!(
        up.protocols,
        ["rivet", "rivet_encoding.bare", "rivet_conn_params.%7B%7D"]
    );
    assert_eq!(up.origin.as_deref(), Some(ALLOWED));
    assert_eq!(
        up.path,
        "/gateway/user/connect?rvt-namespace=default&rvt-method=getOrCreate"
    );
    assert_eq!(up.extensions, None);

    ws.send(TMessage::text("hello")).await?;
    assert_eq!(next_data(&mut ws).await?, TMessage::text("hello"));
    ws.send(TMessage::binary(vec![1u8, 2, 3, 250])).await?;
    assert_eq!(
        next_data(&mut ws).await?,
        TMessage::binary(vec![1u8, 2, 3, 250])
    );

    // A client close reaches the upstream with the same code.
    ws.close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
        code: 1000.into(),
        reason: "done".into(),
    }))
    .await?;
    wait_for(|| !fx.upstream.closes.lock().is_empty()).await?;
    assert_eq!(fx.upstream.closes.lock()[0], (1000, "done".to_owned()));
    Ok(())
}

#[tokio::test]
async fn ws_relay_mirrors_an_upstream_1011_close_to_the_browser() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let mut ws = connect_ws(
        &format!("ws://{}/rivet/gateway/user/connect", fx.proxy),
        &["rivet"],
        Some(ALLOWED),
        &[],
    )
    .await?;
    fx.wait_upgrades(1).await?;
    wait_for(|| fx.upstream.close_cmd.lock().is_some()).await?;
    fx.close_upstream(1011, "internal error")?;
    assert_eq!(
        next_close(&mut ws).await?,
        (1011, "internal error".to_owned())
    );
    Ok(())
}

#[tokio::test]
async fn ws_relay_passes_an_upstream_1001_and_its_reason_intact() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let mut ws = connect_ws(
        &format!("ws://{}/rivet/gateway/user/connect", fx.proxy),
        &["rivet"],
        Some(ALLOWED),
        &[],
    )
    .await?;
    fx.wait_upgrades(1).await?;
    wait_for(|| fx.upstream.close_cmd.lock().is_some()).await?;
    fx.close_upstream(1001, "engine redeploy")?;
    assert_eq!(
        next_close(&mut ws).await?,
        (1001, "engine redeploy".to_owned())
    );
    Ok(())
}

#[tokio::test]
async fn ws_relay_abrupt_browser_termination_still_closes_the_upstream_leg() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let ws = connect_ws(
        &format!("ws://{}/rivet/gateway/user/connect", fx.proxy),
        &["rivet"],
        Some(ALLOWED),
        &[],
    )
    .await?;
    fx.wait_upgrades(1).await?;
    wait_for(|| fx.upstream.close_cmd.lock().is_some()).await?;
    // Dropping the socket sends no close frame (1006 on the proxy), which must be
    // normalized before it is forwarded.
    drop(ws);
    wait_for(|| !fx.upstream.closes.lock().is_empty()).await?;
    assert_eq!(
        fx.upstream.closes.lock()[0],
        (1001, "client closed".to_owned())
    );
    Ok(())
}

#[tokio::test]
async fn ws_relay_one_frame_cannot_overshoot_the_buffer_budget() -> anyhow::Result<()> {
    let fx = Fixture::with(GatewayOptions {
        max_buffered_bytes: 512,
        ..GatewayOptions::default()
    })
    .await?;
    let mut ws = connect_ws(
        &format!("ws://{}/rivet/gateway/user/connect", fx.proxy),
        &["rivet"],
        Some(ALLOWED),
        &[],
    )
    .await?;
    fx.wait_upgrades(1).await?;
    ws.send(TMessage::binary(vec![0u8; 513])).await?;
    assert_eq!(
        next_close(&mut ws).await?,
        (1013, "backpressure limit".to_owned())
    );
    Ok(())
}

#[tokio::test]
async fn ws_relay_rejects_a_bad_origin_before_dialing() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let result = connect_ws(
        &format!("ws://{}/rivet/gateway/user/connect", fx.proxy),
        &["rivet"],
        Some("https://evil.example.com"),
        &[],
    )
    .await;
    assert!(result.is_err());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(fx.upgrades().is_empty());
    Ok(())
}

#[tokio::test]
async fn ws_relay_keeps_frames_fifo_while_the_upstream_dial_is_in_flight() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let mut ws = connect_ws(
        &format!("ws://{}/rivet/gateway/user/connect", fx.proxy),
        &["rivet"],
        Some(ALLOWED),
        &[],
    )
    .await?;
    // Sent immediately: the first frames arrive before the upstream leg is open.
    for i in 0..200 {
        ws.send(TMessage::text(format!("frame-{i}"))).await?;
    }
    for i in 0..200 {
        assert_eq!(
            next_data(&mut ws).await?,
            TMessage::text(format!("frame-{i}"))
        );
    }
    Ok(())
}

#[tokio::test]
async fn ws_relay_upstream_dial_carries_the_trusted_forwarded_for() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let _ws = connect_ws(
        &format!(
            "ws://{}/rivet/gateway/user/connect?rvt-namespace=default",
            fx.proxy
        ),
        &["rivet"],
        Some(ALLOWED),
        &[],
    )
    .await?;
    fx.wait_upgrades(1).await?;
    assert_eq!(fx.upgrades()[0].xff.as_deref(), Some("127.0.0.1"));

    let _ws2 = connect_ws(
        &format!(
            "ws://{}/rivet/gateway/user/connect?rvt-namespace=default",
            fx.proxy
        ),
        &["rivet"],
        Some(ALLOWED),
        &[("x-forwarded-for", "6.6.6.6, 203.0.113.9")],
    )
    .await?;
    fx.wait_upgrades(2).await?;
    assert_eq!(fx.upgrades()[1].xff.as_deref(), Some("203.0.113.9"));
    Ok(())
}

#[tokio::test]
async fn ws_relay_closes_1011_when_the_upstream_refuses_the_upgrade() -> anyhow::Result<()> {
    // A proxy pointed at a port with no listener: the dial fails after the
    // browser upgrade, so the browser sees a 1011 close.
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0")?;
        l.local_addr()?
    };
    let proxy = Fixture::proxy_for(dead, GatewayOptions::default()).await?;
    let mut ws = connect_ws(
        &format!("ws://{proxy}/rivet/gateway/user/connect"),
        &["rivet"],
        Some(ALLOWED),
        &[],
    )
    .await?;
    let (code, reason) = next_close(&mut ws).await?;
    assert_eq!(code, 1011);
    assert!(reason.starts_with("upstream"), "{reason}");
    assert!(!reason.contains("127.0.0.1"), "{reason}");
    Ok(())
}

fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder().no_proxy().build()?)
}

#[tokio::test]
async fn http_passthrough_forwards_method_query_body_and_strips_credentials() -> anyhow::Result<()>
{
    let fx = Fixture::start().await?;
    let res = http_client()?
        .post(fx.http("/rivet/gateway/user/action/getProfile?namespace=default"))
        .header("content-type", "application/json")
        .header("x-rivet-conn-params", r#"{"internalSecret":"x"}"#)
        .header("cookie", "better-auth.session_token=supersecret")
        .header("authorization", "Bearer supersecret")
        .body(r#"{"args":[]}"#)
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    let echoed: Value = res.json().await?;
    assert_eq!(echoed["method"], "POST");
    assert_eq!(echoed["search"], "?namespace=default");
    assert_eq!(
        serde_json::from_str::<Value>(echoed["body"].as_str().unwrap_or_default())?,
        json!({ "args": [] })
    );
    assert_eq!(echoed["connParams"], r#"{"internalSecret":"x"}"#);
    assert_eq!(echoed["authorization"], Value::Null);
    assert_eq!(echoed["cookie"], Value::Null);
    Ok(())
}

#[tokio::test]
async fn http_passthrough_overwrites_forwarded_for_with_the_trusted_derivation()
-> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let url = fx.http("/rivet/gateway/user/action/getProfile?namespace=default");
    let res = http_client()?
        .post(&url)
        .header("x-forwarded-for", "6.6.6.6, 203.0.113.9")
        .body(r#"{"args":[]}"#)
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<Value>().await?["xff"], "203.0.113.9");

    let res = http_client()?
        .post(&url)
        .header("x-forwarded-for", "not-an-ip")
        .body(r#"{"args":[]}"#)
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<Value>().await?["xff"], "127.0.0.1");
    Ok(())
}

#[tokio::test]
async fn http_passthrough_strips_other_client_identity_headers() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let res = http_client()?
        .post(fx.http("/rivet/gateway/user/action/getProfile?namespace=default"))
        .header("forwarded", "for=6.6.6.6")
        .header("x-real-ip", "6.6.6.6")
        .header("true-client-ip", "6.6.6.6")
        .body(r#"{"args":[]}"#)
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    let echoed: Value = res.json().await?;
    assert_eq!(echoed["forwarded"], Value::Null);
    assert_eq!(echoed["xRealIp"], Value::Null);
    assert_eq!(echoed["trueClientIp"], Value::Null);
    assert_eq!(echoed["xff"], "127.0.0.1");
    Ok(())
}

#[tokio::test]
async fn http_passthrough_strips_the_identity_credential_and_forwarding_headers()
-> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let res = http_client()?
        .post(fx.http("/rivet/gateway/user/action/getProfile"))
        .header("x-euchre-identity", "identity.jwt.value")
        .header("x-forwarded-host", "evil.example")
        .header("x-forwarded-proto", "https")
        .body(r#"{"args":[]}"#)
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    let echoed: Value = res.json().await?;
    assert_eq!(echoed["identity"], Value::Null);
    assert_eq!(echoed["forwardedHost"], Value::Null);
    assert_eq!(echoed["forwardedProto"], Value::Null);
    Ok(())
}

/// One raw HTTP/1.1 request, so no client library normalizes the path first.
/// Returns the status code.
pub(crate) async fn raw_status(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> anyhow::Result<u16> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = TcpStream::connect(addr).await?;
    let mut req = format!("{method} {path} HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\n");
    for (name, value) in headers {
        req.push_str(&format!("{name}: {value}\r\n"));
    }
    if method != "GET" {
        req.push_str("content-length: 2\r\n\r\n{}");
    } else {
        req.push_str("\r\n");
    }
    stream.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf)).await??;
    let text = String::from_utf8_lossy(&buf);
    let status = text
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("no status line: {text}"))?;
    Ok(status.parse()?)
}

/// The probes from the review: each must stop at the proxy, never reaching the
/// upstream under a rewritten path.
pub(crate) const DOT_SEGMENT_PROBES: [(&str, &str); 6] = [
    ("GET", "/rivet/gateway/{id}/request/../../actors"),
    (
        "GET",
        "/rivet/gateway/{id}/request/%2e%2e/%2E%2E/namespaces",
    ),
    ("POST", "/rivet/gateway/{id}/request/../../../auth/tokens"),
    ("GET", "/rivet/gateway/{id}/request/../inspector/ui/"),
    (
        "POST",
        "/rivet/gateway/{id}/request/../../euchreTable/action/snapshot",
    ),
    ("GET", "/rivet/gateway/{id}/request/.%2E/.%2e/.%2E/health"),
];

#[tokio::test]
async fn http_passthrough_never_forwards_a_dot_segment_path() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    for (method, probe) in DOT_SEGMENT_PROBES {
        let path = probe.replace("{id}", "actor-1");
        let status = raw_status(fx.proxy, method, &path, &[("x-rivet-token", "anything")]).await?;
        assert_eq!(status, 404, "{method} {path}");
    }
    assert!(
        fx.upstream.seen.lock().is_empty(),
        "{:?}",
        fx.upstream.seen.lock()
    );
    // Control: the same raw request without dot segments does reach it.
    let status = raw_status(fx.proxy, "POST", "/rivet/gateway/user/action/other", &[]).await?;
    assert_eq!(status, 401);
    assert_eq!(
        fx.upstream.seen.lock().clone(),
        vec!["/gateway/user/action/other".to_owned()]
    );
    Ok(())
}

#[tokio::test]
async fn ws_relay_never_dials_a_dot_segment_path() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let base = format!("ws://{}", fx.proxy);
    for path in [
        "/rivet/gateway/actor-1/websocket/../../../actors",
        "/rivet/gateway/actor-1/websocket/%2e%2E/%2E%2e/%2e%2e/namespaces",
    ] {
        let res = connect_ws(&format!("{base}{path}"), &["rivet"], None, &[]).await;
        assert!(res.is_err(), "{path} upgraded");
    }
    assert!(
        fx.upstream.seen.lock().is_empty(),
        "{:?}",
        fx.upstream.seen.lock()
    );
    Ok(())
}

#[tokio::test]
async fn metadata_is_a_static_stub_never_proxied() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let res = http_client()?
        .get(fx.http("/rivet/metadata"))
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(res.json::<Value>().await?, json!({}));
    Ok(())
}

#[tokio::test]
async fn admin_surfaces_are_denied_by_the_proxy_not_the_engine() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    for path in [
        "/rivet/namespaces",
        "/rivet/runner-configs/default",
        "/rivet/actors",
        "/rivet/actors/actor-1",
        "/rivet/gateway/user/dynamic/reload",
        "/rivet/",
    ] {
        let res = http_client()?.get(fx.http(path)).send().await?;
        assert_eq!(res.status(), 404, "{path}");
        assert_eq!(res.json::<Value>().await?["error"], "not_found", "{path}");
    }
    Ok(())
}

#[tokio::test]
async fn http_origin_gate_applies_to_passthrough_too() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let res = http_client()?
        .post(fx.http("/rivet/gateway/user/action/getProfile?namespace=default"))
        .header("origin", "https://evil.example.com")
        .body(r#"{"args":[]}"#)
        .send()
        .await?;
    assert_eq!(res.status(), 403);
    assert_eq!(res.json::<Value>().await?["error"], "origin_not_allowed");
    Ok(())
}

#[tokio::test]
async fn http_passthrough_reports_an_unreachable_upstream_as_404() -> anyhow::Result<()> {
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0")?;
        l.local_addr()?
    };
    let proxy = Fixture::proxy_for(dead, GatewayOptions::default()).await?;
    let res = http_client()?
        .post(format!(
            "http://{proxy}/rivet/gateway/user/action/getProfile"
        ))
        .body("{}")
        .send()
        .await?;
    assert_eq!(res.status(), 404);
    assert_eq!(res.json::<Value>().await?["error"], "upstream_unreachable");
    Ok(())
}

/// W3 security review: an engine 5xx keeps its status, group, code and message,
/// but never its `metadata` (engine parser details for a malformed actor id).
#[tokio::test]
async fn http_passthrough_scrubs_engine_error_metadata() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let res = http_client()?
        .post(fx.http("/rivet/gateway/broken/action/snapshot"))
        .body("{}")
        .send()
        .await?;
    assert_eq!(res.status(), 500);
    let body: Value = res.json().await?;
    assert_eq!(
        body,
        json!({ "code": "internal_error", "group": "core", "message": "An internal error occurred" })
    );
    // Not JSON: passed through unchanged.
    let res = http_client()?
        .post(fx.http("/rivet/gateway/plain500/action/snapshot"))
        .body("{}")
        .send()
        .await?;
    assert_eq!(res.status(), 500);
    assert_eq!(res.text().await?, "plain failure");
    assert_eq!(
        scrub_error_metadata(br#"{"group":"user","code":"x"}"#),
        None
    );
    assert_eq!(scrub_error_metadata(b"[1]"), None);
    Ok(())
}

#[tokio::test]
async fn http_passthrough_returns_the_upstream_status_and_body() -> anyhow::Result<()> {
    let fx = Fixture::start().await?;
    let res = http_client()?
        .post(fx.http("/rivet/gateway/user/action/other"))
        .body("{}")
        .send()
        .await?;
    assert_eq!(res.status(), 401);
    assert_eq!(res.text().await?, "engine admin surface");
    let _ = fx.upstream_addr;
    Ok(())
}
