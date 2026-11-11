//! The `/rivet/**` gateway in front of the private engine (05 §5.3, §12.2): a port of
//! TS `server/rivet-gateway-proxy.ts`.
//!
//! [`GatewayProxy`] is the TS proxy: the allowlist classifier, the Origin gate, the
//! trusted client IP, header stripping, the HTTP passthrough and the WebSocket relay.
//! [`public`] is the stricter public admission that `server/index.ts` put in front
//! of it (actor-id-only targets, the guest session, the per-IP bucket).
//!
//! Credentials never reach a log line: a path that may carry `@<token>` or
//! `rvt-token=` goes through [`redact_path`] first.

pub mod public;
mod relay;

use std::net::IpAddr;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, FromRequestParts, Request, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use euchre_server_core::config::OriginPolicy;
use parking_lot::Mutex;
use regex::Regex;

use crate::config::{GatewayOptions, MAX_REQUEST_BODY_BYTES};
use crate::error::gateway_json;

pub use relay::{RelayTarget, relay};

// ---- Pure route classification (the allowlist) ------------------------------------

/// What [`classify_proxy_path`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyRouteKind {
    /// `GET /metadata`: answered locally with `{}`.
    Metadata,
    /// An HTTP passthrough.
    Http,
    /// A WebSocket relay.
    Ws,
    /// Refused: `404 {"error":"not_found"}`.
    Deny,
}

/// A classified gateway path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyRoute {
    /// The decision.
    pub kind: ProxyRouteKind,
    /// Path with the mount prefix stripped: what is appended to the upstream base.
    pub upstream_path: String,
}

impl ProxyRoute {
    fn deny() -> Self {
        Self {
            kind: ProxyRouteKind::Deny,
            upstream_path: String::new(),
        }
    }

    fn new(kind: ProxyRouteKind, path: &str) -> Self {
        Self {
            kind,
            upstream_path: path.to_owned(),
        }
    }
}

/// `GATEWAY_SEG`: an actor name or `actorId[@token]`.
const GATEWAY_SEG: &str = "[^/]+";

#[allow(clippy::expect_used)]
static WS_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        "^/gateway/{GATEWAY_SEG}(?:/connect|/websocket(?:/.*)?)$"
    ))
    .expect("WS_PATH_RE is a valid regex")
});

#[allow(clippy::expect_used)]
static HTTP_GATEWAY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        "^/gateway/{GATEWAY_SEG}(?:/action/[^/]+|/queue/[^/]+|/request(?:/.*)?)$"
    ))
    .expect("HTTP_GATEWAY_RE is a valid regex")
});

/// Decode `%XX` escapes byte-wise (no `+` rule: this is a path). `None` for a
/// malformed escape.
pub(crate) fn percent_decode_bytes(raw: &str) -> Option<Vec<u8>> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = bytes.get(i + 1).and_then(|b| char::from(*b).to_digit(16))?;
            let lo = bytes.get(i + 2).and_then(|b| char::from(*b).to_digit(16))?;
            out.push(u8::try_from(hi * 16 + lo).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

/// True when `path` (mount prefix already stripped, starting with `/`) reaches the
/// engine exactly as classified: nothing downstream can rewrite it into another
/// route.
///
/// Refused: a `.` or `..` segment, also percent-encoded in any case (`%2e`,
/// `.%2E`, ...), which WHATWG URL parsing (reqwest, the engine) would collapse; a
/// raw `\` (a separator for `http` URLs); an encoded `/` or `\` (`%2f`, `%5c`);
/// and a malformed `%` escape. The `@<token>` part of the gateway actor segment is
/// exempt from the encoded-separator rule only: an engine token may carry `%2F`.
///
/// The TS proxy classified the already-normalized `new URL(req.url).pathname`;
/// refusing instead of normalizing keeps the classified path and the forwarded path
/// the same string.
pub fn is_canonical_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    segments.iter().enumerate().all(|(i, seg)| {
        if seg.contains('\\') {
            return false;
        }
        let Some(decoded) = percent_decode_bytes(seg) else {
            return false;
        };
        if decoded == b"." || decoded == b".." {
            return false;
        }
        let checked = if i == 2 && segments.get(1) == Some(&"gateway") {
            seg.split_once('@').map_or(*seg, |(head, _)| head)
        } else {
            seg
        };
        percent_decode_bytes(checked).is_some_and(|d| !d.iter().any(|b| *b == b'/' || *b == b'\\'))
    })
}

/// TS `classifyProxyPath`: classify `pathname` (mount prefix included) against the
/// client wire-contract allowlist. A path outside `prefix` is denied.
///
/// - A path that is not [`is_canonical_path`] (dot segments, encoded separators)
///   is denied before any route matches.
/// - `GET /metadata` is answered locally and never proxied.
/// - `/connect` and `/gateway/<seg>/connect|websocket[/...]` relay only as upgrades.
/// - `/gateway/<seg>/action/<n>|queue/<n>|request[/...]` pass through only when not
///   an upgrade (any method, OPTIONS included).
/// - Everything else (`/actors`, `/namespaces`, `/runner-configs`, `/datacenters`,
///   `/ui`, inspector, health, dynamic reload, `POST /metadata`) is denied.
pub fn classify_proxy_path(
    pathname: &str,
    method: &str,
    is_upgrade: bool,
    prefix: &str,
) -> ProxyRoute {
    let Some(rest) = pathname.strip_prefix(prefix) else {
        return ProxyRoute::deny();
    };
    let path = if rest.is_empty() { "/" } else { rest };
    if !is_canonical_path(path) {
        return ProxyRoute::deny();
    }
    let method = method.to_ascii_uppercase();

    if path == "/metadata" && method == "GET" {
        return ProxyRoute::new(ProxyRouteKind::Metadata, path);
    }
    if path == "/connect" || WS_PATH_RE.is_match(path) {
        return if is_upgrade {
            ProxyRoute::new(ProxyRouteKind::Ws, path)
        } else {
            ProxyRoute::deny()
        };
    }
    if HTTP_GATEWAY_RE.is_match(path) {
        return if is_upgrade {
            ProxyRoute::deny()
        } else {
            ProxyRoute::new(ProxyRouteKind::Http, path)
        };
    }
    ProxyRoute::deny()
}

/// TS `isOriginAllowed`: an absent Origin passes (non-browser clients; the actor
/// token is the boundary), `Any` passes, else the origin must be listed.
pub fn is_origin_allowed(origin: Option<&str>, allowed: &AllowedOrigins) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    match allowed {
        AllowedOrigins::Any => true,
        AllowedOrigins::List(list) => list.iter().any(|o| o == origin),
        AllowedOrigins::Policy(policy) => policy.check(Some(origin)),
    }
}

/// The gateway's Origin allowlist (TS `string[] | '*'`).
#[derive(Debug, Clone)]
pub enum AllowedOrigins {
    /// `'*'`.
    Any,
    /// An exact list.
    List(Vec<String>),
    /// The server's [`OriginPolicy`] (D1).
    Policy(OriginPolicy),
}

/// TS `toWebSocketBase`: `http(s)` to `ws(s)` for the upstream dial.
pub fn to_web_socket_base(http_base: &str) -> String {
    match http_base.get(..4) {
        Some(scheme) if scheme.eq_ignore_ascii_case("http") => format!("ws{}", &http_base[4..]),
        _ => http_base.to_owned(),
    }
}

/// TS `capWsCloseReason`: cap a close reason at the protocol's 123 UTF-8 **byte**
/// limit, never splitting a character, and drop trailing U+FFFD as the TS decoder
/// round trip did.
pub fn cap_ws_close_reason(reason: &str) -> String {
    const MAX: usize = 123;
    if reason.len() <= MAX {
        return reason.to_owned();
    }
    let mut end = MAX;
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    reason[..end].trim_end_matches('\u{FFFD}').to_owned()
}

/// TS `isSendableCloseCode`: 1000-1014 except 1004, 1005 and 1006, or 3000-4999.
pub fn is_sendable_close_code(code: u16) -> bool {
    ((1000..=1014).contains(&code) && !matches!(code, 1004..=1006)) || (3000..=4999).contains(&code)
}

/// TS `normalizedCloseCode`: `code` when sendable, else `fallback` (1011 toward the
/// browser, 1001 toward the engine).
pub fn normalized_close_code(code: u16, fallback: u16) -> u16 {
    if is_sendable_close_code(code) {
        code
    } else {
        fallback
    }
}

// ---- Trusted client IP --------------------------------------------------------------

/// The unparseable-entry warn fires at most once per window: the entry is
/// attacker-supplied, so an unthrottled warn is a free log amplifier.
const XFF_WARN_INTERVAL: Duration = Duration::from_secs(60);
static LAST_XFF_WARN: Mutex<Option<Instant>> = Mutex::new(None);

fn strip_xff_entry_port(entry: &str) -> &str {
    if let Some(inner) = entry.strip_prefix('[') {
        if let Some((addr, rest)) = inner.split_once(']') {
            let port_ok = rest.is_empty()
                || rest
                    .strip_prefix(':')
                    .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
            if port_ok && !addr.is_empty() {
                return addr;
            }
        }
        return entry;
    }
    if let Some((addr, port)) = entry.rsplit_once(':') {
        let v4_shaped = addr.split('.').count() == 4
            && addr
                .split('.')
                .all(|o| (1..=3).contains(&o.len()) && o.bytes().all(|b| b.is_ascii_digit()));
        if v4_shaped && !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) {
            return addr;
        }
    }
    entry
}

fn is_ip(candidate: &str) -> bool {
    candidate.parse::<IpAddr>().is_ok()
}

/// TS `deriveTrustedClientIp`: the client IP for the engine Guard's per-IP bucket.
///
/// 1. The **last** `X-Forwarded-For` entry (appended by the nearest trusted hop; the
///    first is client-forgeable), with `ip:port` / `[v6]:port` stripped. No backward
///    walk: everything before it is client-supplied.
/// 2. The socket IP.
///
/// An unparseable last entry logs a throttled warn (once per 60 s) with the entry
/// reduced to printable ASCII and 64 chars.
pub fn derive_trusted_client_ip(
    forwarded_for: Option<&str>,
    socket_address: Option<&str>,
) -> Option<String> {
    if let Some(xff) = forwarded_for {
        let last = xff.split(',').map(str::trim).rfind(|e| !e.is_empty());
        if let Some(last) = last {
            let candidate = strip_xff_entry_port(last);
            if is_ip(candidate) {
                return Some(candidate.to_owned());
            }
            let now = Instant::now();
            let mut guard = LAST_XFF_WARN.lock();
            let due = guard.is_none_or(|at| now.duration_since(at) >= XFF_WARN_INTERVAL);
            if due {
                *guard = Some(now);
                let entry: String = last
                    .chars()
                    .map(|c| if (' '..='~').contains(&c) { c } else { '?' })
                    .take(64)
                    .collect();
                tracing::warn!(
                    entry = %entry,
                    "rivet proxy: unparseable last X-Forwarded-For entry — falling back to socket IP (shared guard bucket)"
                );
            }
        }
    }
    socket_address.filter(|addr| is_ip(addr)).map(str::to_owned)
}

/// The socket IP as TS reported it: IPv4-mapped IPv6 shown as IPv4.
pub fn socket_ip(addr: IpAddr) -> String {
    addr.to_canonical().to_string()
}

/// Replace every credential a gateway path or query can carry: `@<token>` in a
/// path segment and the value of `rvt-token=`.
pub fn redact_path(path: &str) -> String {
    let (path_part, query) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    let mut out = String::with_capacity(path.len());
    for (i, seg) in path_part.split('/').enumerate() {
        if i > 0 {
            out.push('/');
        }
        match seg.split_once('@') {
            Some((head, _)) => {
                out.push_str(head);
                out.push_str("@<redacted>");
            }
            None => out.push_str(seg),
        }
    }
    if let Some(query) = query {
        out.push('?');
        let parts: Vec<String> = query
            .split('&')
            .map(|kv| match kv.split_once('=') {
                Some((k, _)) if k.eq_ignore_ascii_case("rvt-token") => format!("{k}=<redacted>"),
                _ => kv.to_owned(),
            })
            .collect();
        out.push_str(&parts.join("&"));
    }
    out
}

// ---- Proxy -------------------------------------------------------------------------

/// Largest engine 5xx body the passthrough reads to scrub it.
const MAX_SERVER_ERROR_BODY: usize = 64 * 1024;

/// An engine 5xx, buffered so its JSON error loses `metadata`: the engine puts
/// parser and internal details there (`{"group":"core","code":"internal_error",
/// "metadata":{"error":"unsupported ID version: 146"}}` for a malformed actor id).
/// Status, `group`, `code` and `message` stay, so clients still parse the error.
/// A body that is not JSON passes unchanged; one over the cap becomes a plain
/// `502 upstream_error`.
async fn server_error_response(
    mut res: axum::http::response::Builder,
    mut upstream: reqwest::Response,
) -> Response {
    let mut body = Vec::new();
    loop {
        match upstream.chunk().await {
            Ok(Some(chunk)) if body.len() + chunk.len() <= MAX_SERVER_ERROR_BODY => {
                body.extend_from_slice(&chunk);
            }
            Ok(Some(_)) | Err(_) => return gateway_json(StatusCode::BAD_GATEWAY, "upstream_error"),
            Ok(None) => break,
        }
    }
    if let Some(scrubbed) = scrub_error_metadata(&body) {
        body = scrubbed;
        if let Some(headers) = res.headers_mut() {
            headers.remove(header::CONTENT_LENGTH);
        }
    }
    res.body(Body::from(body))
        .unwrap_or_else(|_| gateway_json(StatusCode::BAD_GATEWAY, "upstream_error"))
}

/// The JSON error object `body` without its `metadata`, or `None` when `body` is
/// not a JSON object with one.
fn scrub_error_metadata(body: &[u8]) -> Option<Vec<u8>> {
    let mut value: serde_json::Value = serde_json::from_slice(body).ok()?;
    value.as_object_mut()?.remove("metadata")?;
    serde_json::to_vec(&value).ok()
}

/// Request headers never forwarded upstream: hop-by-hop, the browser's ambient
/// credentials (`cookie`, `authorization`), the server's own identity credential
/// (`x-euchre-identity`, 05 §11.4.6), every client-identity header (the proxy
/// derives the one trusted `x-forwarded-for` itself) and the client-supplied
/// `x-forwarded-host` / `x-forwarded-proto`.
pub const HOP_BY_HOP_REQ: [&str; 17] = [
    "connection",
    "keep-alive",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
    "cookie",
    "authorization",
    "forwarded",
    "x-real-ip",
    "x-client-ip",
    "true-client-ip",
    "cf-connecting-ip",
    "x-cluster-client-ip",
    "x-euchre-identity",
    "x-forwarded-host",
    "x-forwarded-proto",
];

/// Response headers never copied back.
pub const HOP_BY_HOP_RES: [&str; 3] = ["connection", "keep-alive", "transfer-encoding"];

/// Options of a [`GatewayProxy`] (TS `RivetGatewayProxyOptions`).
#[derive(Debug, Clone)]
pub struct GatewayProxyOptions {
    /// Bare engine base URL, for example `http://rivet-engine.railway.internal:6420`
    /// (no userinfo, no path). Only `http`/`ws` upstreams are dialed for WebSockets.
    pub upstream_http: String,
    /// The Origin allowlist.
    pub allowed_origins: AllowedOrigins,
    /// Mount prefix, `/rivet`.
    pub prefix: String,
    /// Relay tunables.
    pub tunables: GatewayOptions,
}

impl GatewayProxyOptions {
    /// Options with the TS defaults for `upstream_http` and `allowed_origins`.
    pub fn new(upstream_http: impl Into<String>, allowed_origins: AllowedOrigins) -> Self {
        Self {
            upstream_http: upstream_http.into(),
            allowed_origins,
            prefix: "/rivet".to_owned(),
            tunables: GatewayOptions::default(),
        }
    }
}

struct ProxyInner {
    upstream_http: String,
    upstream_ws: String,
    allowed_origins: AllowedOrigins,
    prefix: String,
    tunables: GatewayOptions,
    http: reqwest::Client,
}

/// TS `createRivetGatewayProxy`. Cheap to clone.
#[derive(Clone)]
pub struct GatewayProxy {
    inner: Arc<ProxyInner>,
}

impl std::fmt::Debug for GatewayProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayProxy")
            .field("upstream_http", &self.inner.upstream_http)
            .field("prefix", &self.inner.prefix)
            .finish_non_exhaustive()
    }
}

/// True for `Upgrade: websocket` (case-insensitive).
pub fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

/// The offered `Sec-WebSocket-Protocol` list: split on `,`, trimmed, empties dropped.
pub fn offered_protocols(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// TS metadata stub: `200 {}`, `Cache-Control: no-store`. The empty object means
/// "keep the configured endpoint", so the engine can never hand a client a
/// different endpoint or token.
pub fn metadata_stub() -> Response {
    let mut res = Response::new(Body::from("{}"));
    let headers = res.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

impl GatewayProxy {
    /// Build a proxy that forwards with `http` (built with redirects disabled).
    pub fn new(opts: GatewayProxyOptions, http: reqwest::Client) -> Self {
        let upstream_http = opts.upstream_http.trim_end_matches('/').to_owned();
        let upstream_ws = to_web_socket_base(&upstream_http);
        Self {
            inner: Arc::new(ProxyInner {
                upstream_http,
                upstream_ws,
                allowed_origins: opts.allowed_origins,
                prefix: opts.prefix,
                tunables: opts.tunables,
                http,
            }),
        }
    }

    /// A `reqwest` client fit for the passthrough: no redirects followed, and no
    /// `HTTP_PROXY` / `HTTPS_PROXY` from the environment (the engine is always on a
    /// private network or loopback, which a proxy cannot reach).
    pub fn http_client() -> reqwest::Result<reqwest::Client> {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
    }

    /// The relay tunables.
    pub fn tunables(&self) -> &GatewayOptions {
        &self.inner.tunables
    }

    /// A router serving only this proxy under its prefix (TS
    /// `routes: {'/rivet/*': proxy.handleRequest}`). Serve it with
    /// `into_make_service_with_connect_info::<SocketAddr>()` so the socket IP is known.
    pub fn router(&self) -> Router {
        let proxy = self.clone();
        let handler = move |req: Request| {
            let proxy = proxy.clone();
            async move {
                let ip = req
                    .extensions()
                    .get::<ConnectInfo<std::net::SocketAddr>>()
                    .map(|ci| ci.0.ip());
                proxy.handle(req, ip).await
            }
        };
        let prefix = self.inner.prefix.clone();
        Router::new()
            .route(&prefix, any(handler.clone()))
            .route(&format!("{prefix}/"), any(handler.clone()))
            .route(&format!("{prefix}/{{*rest}}"), any(handler))
    }

    /// TS `handleRequest`: classify, answer the metadata stub, apply the Origin gate,
    /// derive the client IP, then pass through or relay.
    pub async fn handle(&self, req: Request, socket: Option<IpAddr>) -> Response {
        let is_upgrade = is_websocket_upgrade(req.headers());
        let route = classify_proxy_path(
            req.uri().path(),
            req.method().as_str(),
            is_upgrade,
            &self.inner.prefix,
        );
        match route.kind {
            ProxyRouteKind::Deny => return gateway_json(StatusCode::NOT_FOUND, "not_found"),
            ProxyRouteKind::Metadata => return metadata_stub(),
            ProxyRouteKind::Http | ProxyRouteKind::Ws => {}
        }

        let origin = match req.headers().get(header::ORIGIN) {
            None => None,
            Some(v) => match v.to_str() {
                Ok(s) => Some(s.to_owned()),
                Err(_) => return gateway_json(StatusCode::FORBIDDEN, "origin_not_allowed"),
            },
        };
        if !is_origin_allowed(origin.as_deref(), &self.inner.allowed_origins) {
            tracing::warn!(
                origin = ?origin,
                path = %redact_path(req.uri().path()),
                "rivet proxy: origin rejected"
            );
            return gateway_json(StatusCode::FORBIDDEN, "origin_not_allowed");
        }

        let socket = socket.map(socket_ip);
        let client_ip = derive_trusted_client_ip(
            req.headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok()),
            socket.as_deref(),
        );

        if route.kind == ProxyRouteKind::Http {
            return self
                .passthrough_http(req, &route.upstream_path, client_ip)
                .await;
        }
        self.upgrade(req, &route.upstream_path, origin, client_ip)
            .await
    }

    async fn passthrough_http(
        &self,
        req: Request,
        upstream_path: &str,
        client_ip: Option<String>,
    ) -> Response {
        let search = req
            .uri()
            .query()
            .filter(|q| !q.is_empty())
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        let Some(upstream_url) =
            verified_upstream_url(&self.inner.upstream_http, upstream_path, &search)
        else {
            return gateway_json(StatusCode::NOT_FOUND, "not_found");
        };
        let mut headers = HeaderMap::new();
        for (key, value) in req.headers() {
            if !HOP_BY_HOP_REQ.contains(&key.as_str()) {
                headers.append(key.clone(), value.clone());
            }
        }
        // Guard rate-limit key: overwrite, never pass through.
        headers.remove("x-forwarded-for");
        if let Some(ip) = client_ip
            .as_deref()
            .and_then(|ip| HeaderValue::from_str(ip).ok())
        {
            headers.insert(HeaderName::from_static("x-forwarded-for"), ip);
        }
        let method = req.method().clone();
        let has_body = method != Method::GET && method != Method::HEAD;
        let body = if has_body {
            match axum::body::to_bytes(req.into_body(), MAX_REQUEST_BODY_BYTES).await {
                Ok(bytes) => Some(bytes),
                Err(_) => return gateway_json(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"),
            }
        } else {
            None
        };
        let mut builder = self
            .inner
            .http
            .request(method, upstream_url)
            .headers(headers)
            .timeout(self.inner.tunables.http_timeout);
        if let Some(body) = body {
            builder = builder.body(body);
        }
        match builder.send().await {
            Ok(upstream) => {
                if upstream.status() == StatusCode::TOO_MANY_REQUESTS {
                    tracing::warn!(
                        path = %redact_path(upstream_path),
                        client_ip = ?client_ip,
                        "[guard-503] rivet proxy: upstream 429 (guard rate limit)"
                    );
                }
                let mut res = Response::builder().status(upstream.status());
                if let Some(out) = res.headers_mut() {
                    for (key, value) in upstream.headers() {
                        if !HOP_BY_HOP_RES.contains(&key.as_str()) {
                            out.append(key.clone(), value.clone());
                        }
                    }
                }
                if upstream.status().is_server_error() {
                    return server_error_response(res, upstream).await;
                }
                res.body(Body::from_stream(upstream.bytes_stream()))
                    .unwrap_or_else(|_| gateway_json(StatusCode::NOT_FOUND, "upstream_unreachable"))
            }
            Err(err) => {
                tracing::warn!(
                    path = %redact_path(upstream_path),
                    error = %redact_error(&err),
                    "rivet proxy: upstream HTTP error"
                );
                gateway_json(StatusCode::NOT_FOUND, "upstream_unreachable")
            }
        }
    }

    async fn upgrade(
        &self,
        req: Request,
        upstream_path: &str,
        origin: Option<String>,
        client_ip: Option<String>,
    ) -> Response {
        let search = req
            .uri()
            .query()
            .filter(|q| !q.is_empty())
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        let protocols = offered_protocols(req.headers());
        let (mut parts, _body) = req.into_parts();
        let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
            Ok(ws) => ws,
            Err(_) => return gateway_json(StatusCode::NOT_FOUND, "upgrade_failed"),
        };
        let tunables = self.inner.tunables;
        let Some(url) = verified_upstream_url(&self.inner.upstream_ws, upstream_path, &search)
        else {
            return gateway_json(StatusCode::NOT_FOUND, "not_found");
        };
        let target = RelayTarget {
            url: url.into(),
            protocols,
            origin,
            client_ip,
        };
        ws.protocols(["rivet"])
            .max_message_size(tunables.max_payload_bytes)
            .max_frame_size(tunables.max_payload_bytes)
            .on_failed_upgrade(|err| {
                tracing::warn!(error = %err, "rivet proxy: browser upgrade failed");
            })
            .on_upgrade(move |socket| relay(socket, target, tunables))
            .into_response()
    }
}

/// `base + upstream_path + search` as a parsed URL, or `None` when parsing would
/// change the path (any normalization or re-encoding): what is dialed is then
/// exactly what [`classify_proxy_path`] admitted.
pub(crate) fn verified_upstream_url(
    base: &str,
    upstream_path: &str,
    search: &str,
) -> Option<url::Url> {
    let base_path = url::Url::parse(base)
        .ok()?
        .path()
        .trim_end_matches('/')
        .to_owned();
    let url = url::Url::parse(&format!("{base}{upstream_path}{search}")).ok()?;
    (url.path() == format!("{base_path}{upstream_path}")).then_some(url)
}

/// A `reqwest` error as text without its URL (which may carry `@<token>`).
pub(crate) fn redact_error(err: &reqwest::Error) -> String {
    let kind = if err.is_timeout() {
        "timeout"
    } else if err.is_connect() {
        "connect"
    } else if err.is_body() {
        "body"
    } else if err.is_request() {
        "request"
    } else {
        "error"
    };
    let mut source = std::error::Error::source(err);
    let mut detail = None;
    while let Some(inner) = source {
        detail = Some(inner.to_string());
        source = inner.source();
    }
    match detail {
        Some(detail) => format!("{kind}: {detail}"),
        None => kind.to_owned(),
    }
}

#[cfg(test)]
pub(crate) mod tests;
