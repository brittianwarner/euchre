//! The router (05 §11, §12.1): the front door, the JSON API, the `/rivet/**`
//! gateway and the static web build.

mod profile;
mod session;
mod tables;

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post};
use euchre_server_core::rivet::{UserError, parse_rivet_error};
use rivetkit_client::handle::ActorHandle;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::config::MAX_REQUEST_BODY_BYTES;
use crate::error::{ApiError, text};
use crate::gateway::{derive_trusted_client_ip, public::rivet_entry};
use crate::origin::{PublicOrigin, resolve_public_origin};
use crate::rate_limit::{RouteClass, UNKNOWN_CLIENT};
use crate::state::ApiState;
use crate::static_files::StaticFiles;

/// Build the whole app router.
///
/// Serve it with `into_make_service_with_connect_info::<SocketAddr>()` so the
/// socket IP reaches the rate limiter and the gateway's `X-Forwarded-For`; without
/// it every client shares one bucket.
///
/// Routing order (TS `server/index.ts`): `/health` on any host; then a host that
/// resolves to no configured origin gets `421 Unknown host`; then
/// `/api/rivet/metadata`, the rest of `/api/rivet*` (`404 Not found`), the JSON
/// API (CORS applies here only), `/rivet/**`, the desktop downloads
/// (`GET`/`HEAD /download/**`, [`crate::downloads`]), and the static build.
///
/// Every response, `/health` and `421` included, carries `X-Content-Type-Options:
/// nosniff` and `Referrer-Policy: strict-origin-when-cross-origin` unless the
/// handler (or the proxied engine) set its own. HTML from the static build also
/// refuses framing ([`crate::static_files`]).
pub fn router(state: ApiState) -> Router {
    let api = Router::new()
        .route("/api/session", get(session::get_session))
        .route("/api/device/enroll", post(session::enroll))
        .route("/api/device/refresh", post(session::refresh))
        .route("/api/identity/identify", post(session::identify))
        .route("/api/identity", delete(session::sign_out))
        .route("/api/tables", post(tables::create_table))
        .route("/api/tables/recent", get(tables::recent_tables))
        .route("/api/table-session/{game_id}", post(tables::table_session))
        .route(
            "/api/profile/settings",
            get(profile::get_settings)
                .put(profile::save_settings)
                .patch(profile::save_settings),
        )
        .route("/api/profile/personas", get(profile::get_personas))
        .route("/api/profile/stats", get(profile::get_stats))
        .route("/api/games", get(profile::list_games))
        .route("/api/games/{match_id}", get(profile::get_replay))
        .route("/api/episodes", get(profile::list_episodes))
        .route(
            "/api/episodes/{episode_id}",
            delete(profile::forget_episode),
        )
        .route("/api/presets", get(profile::presets))
        .route("/api/rivet/metadata", get(rivet_metadata))
        .route("/api/rivet", any(api_rivet_not_found))
        .route("/api/rivet/{*rest}", any(api_rivet_not_found))
        .route("/api/{*rest}", any(api_not_found))
        .layer(cors(&state));

    let rivet = {
        let s = state.clone();
        move |req: Request| rivet_entry(s.clone(), req)
    };

    let downloads = {
        let s = state.clone();
        move |req: Request| {
            let s = s.clone();
            let client = client_ip(&s, req.headers(), socket_ip(&req));
            async move { s.inner.downloads.serve_from(req, client).await }
        }
    };

    let static_files = state.inner.config.static_dir.clone().map(StaticFiles::new);
    let fallback = move |req: Request| {
        let static_files = static_files.clone();
        async move {
            match static_files {
                Some(files) => files.serve(req).await,
                None => text(StatusCode::NOT_FOUND, "Not found"),
            }
        }
    };

    Router::new()
        .route("/health", any(health))
        .merge(api)
        .route("/rivet", any(rivet.clone()))
        .route("/rivet/", any(rivet.clone()))
        .route("/rivet/{*rest}", any(rivet))
        .route("/download/{*rest}", get(downloads))
        .fallback(fallback)
        .layer(middleware::from_fn_with_state(state.clone(), front_door))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static(REFERRER_POLICY),
        ))
        .with_state(state)
}

/// `Referrer-Policy` on every response (the browser default, stated outright).
pub const REFERRER_POLICY: &str = "strict-origin-when-cross-origin";

fn cors(state: &ApiState) -> CorsLayer {
    let policy = state.inner.config.origins.clone();
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin, _| {
            origin.to_str().is_ok_and(|o| policy.check(Some(o)))
        }))
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
}

/// `/health` first, then the public-origin gate (`421 Unknown host`).
async fn front_door(State(state): State<ApiState>, mut req: Request, next: Next) -> Response {
    if req.uri().path() == "/health" {
        return health(State(state)).await;
    }
    let Some(origin) = resolve_public_origin(&state.inner.config.origins, req.headers(), req.uri())
    else {
        return text(StatusCode::MISDIRECTED_REQUEST, "Unknown host");
    };
    req.extensions_mut().insert(PublicOrigin(origin));
    next.run(req).await
}

/// `200 ok`, or `503 draining` before the runner connects and while shutting down.
async fn health(State(state): State<ApiState>) -> Response {
    if state.inner.health.is_serving() {
        text(StatusCode::OK, "ok")
    } else {
        text(StatusCode::SERVICE_UNAVAILABLE, "draining")
    }
}

/// `GET /api/rivet/metadata`: gateway discovery.
async fn rivet_metadata(State(state): State<ApiState>, origin: Origin) -> Response {
    no_store(rivet_discovery(&state, &origin.0))
}

/// The gateway discovery body for the public origin `origin`: the browser client's
/// endpoint and the engine namespace.
pub(crate) fn rivet_discovery(state: &ApiState, origin: &str) -> axum::Json<Value> {
    axum::Json(json!({
        "clientEndpoint": format!("{origin}/rivet"),
        "clientNamespace": state.inner.config.namespace,
    }))
}

async fn api_rivet_not_found() -> Response {
    text(StatusCode::NOT_FOUND, "Not found")
}

async fn api_not_found() -> Response {
    ApiError::not_found("Not found").into_response()
}

// ---- Shared extractors and helpers ------------------------------------------------

/// The resolved public origin (inserted by the front door).
pub(crate) struct Origin(pub String);

impl<S: Send + Sync> FromRequestParts<S> for Origin {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<PublicOrigin>()
            .map(|o| Self(o.0.clone()))
            .ok_or_else(|| text(StatusCode::MISDIRECTED_REQUEST, "Unknown host"))
    }
}

/// The socket address, when the server was started with connect info.
pub(crate) struct ClientAddr(pub Option<IpAddr>);

impl<S: Send + Sync> FromRequestParts<S> for ClientAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ci| ci.0.ip()),
        ))
    }
}

/// The socket IP of a request, when connect info is present.
pub(crate) fn socket_ip(req: &Request) -> Option<IpAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
}

const X_FORWARDED_FOR: &str = "x-forwarded-for";

/// The client IP that `X-Forwarded-For` names, when the socket peer may name one
/// ([`crate::ApiConfig::trusts_forwarded_for_from`]).
///
/// It is the last entry of the last header line, with any port stripped: a proxy that
/// adds its own line leaves the client's earlier lines in place, and everything before
/// the last entry is client-supplied. `None` when trust is off for this peer or that
/// entry is not an IP.
pub(crate) fn forwarded_client_ip(
    state: &ApiState,
    headers: &HeaderMap,
    socket: Option<IpAddr>,
) -> Option<IpAddr> {
    let socket = socket.map(|ip| ip.to_canonical());
    if !state.inner.config.trusts_forwarded_for_from(socket) {
        return None;
    }
    let last_line = headers.get_all(X_FORWARDED_FOR).iter().next_back()?;
    derive_trusted_client_ip(Some(last_line.to_str().ok()?), None)
        .and_then(|ip| ip.parse::<IpAddr>().ok())
        .map(|ip| ip.to_canonical())
}

/// The rate-limit key: the trusted client IP behind the edge, else the socket IP.
pub(crate) fn client_ip(state: &ApiState, headers: &HeaderMap, socket: Option<IpAddr>) -> IpAddr {
    forwarded_client_ip(state, headers, socket)
        .or_else(|| socket.map(|ip| ip.to_canonical()))
        .unwrap_or(UNKNOWN_CLIENT)
}

/// Rewrite `X-Forwarded-For` before a request goes to the `/rivet/**` proxy, which
/// stamps the engine Guard's key from it: a single entry holding the trusted client
/// ([`forwarded_client_ip`]), or no header at all, so the proxy falls back to the
/// socket IP. The D11 bucket and the Guard then key on the same address, and a
/// client that reaches the server directly cannot choose its Guard bucket.
pub(crate) fn pin_forwarded_for(state: &ApiState, headers: &mut HeaderMap, socket: Option<IpAddr>) {
    let forwarded = forwarded_client_ip(state, headers, socket);
    headers.remove(X_FORWARDED_FOR);
    if let Some(value) = forwarded.and_then(|ip| HeaderValue::from_str(&ip.to_string()).ok()) {
        headers.insert(X_FORWARDED_FOR, value);
    }
}

/// Take a token from `class`'s bucket or answer `429`.
pub(crate) fn rate_limit(
    state: &ApiState,
    class: RouteClass,
    headers: &HeaderMap,
    socket: Option<IpAddr>,
) -> Result<(), ApiError> {
    let ip = client_ip(state, headers, socket);
    if state.inner.limiter.check(class, ip) {
        Ok(())
    } else {
        Err(ApiError::rate_limited())
    }
}

/// `Cache-Control: no-store` on a response.
pub(crate) fn no_store(res: impl IntoResponse) -> Response {
    let mut res = res.into_response();
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

/// Message of a body that is not valid JSON for the endpoint.
pub(crate) const BAD_JSON: &str = "That request is not valid JSON.";

/// Read a JSON body (at most 1 MiB). An empty body is `None`.
pub(crate) async fn read_json<T: DeserializeOwned>(body: Body) -> Result<Option<T>, ApiError> {
    let bytes = axum::body::to_bytes(body, MAX_REQUEST_BODY_BYTES)
        .await
        .map_err(|_| ApiError::payload_too_large())?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| ApiError::bad_request(BAD_JSON))
}

/// Why an actor call failed.
#[derive(Debug)]
pub(crate) enum ActorCallError {
    /// No answer within the bound.
    Timeout,
    /// The actor (or engine) answered with a structured error.
    Rivet(Box<UserError>),
    /// Transport or client failure without a structured error.
    Other,
}

impl ActorCallError {
    /// The structured error code, for logs.
    pub(crate) fn code(&self) -> &str {
        match self {
            Self::Timeout => "timeout",
            Self::Rivet(e) => &e.code,
            Self::Other => "unavailable",
        }
    }

    pub(crate) fn from_anyhow(err: &anyhow::Error) -> Self {
        parse_rivet_error(err).map_or(Self::Other, |e| Self::Rivet(Box::new(e)))
    }
}

/// One stateless action within `timeout`.
pub(crate) async fn call(
    handle: &ActorHandle,
    action: &str,
    args: Vec<Value>,
    timeout: Duration,
) -> Result<Value, ActorCallError> {
    match tokio::time::timeout(timeout, handle.action(action, args)).await {
        Err(_) => Err(ActorCallError::Timeout),
        Ok(Err(err)) => Err(ActorCallError::from_anyhow(&err)),
        Ok(Ok(value)) => Ok(value),
    }
}

/// `handle.resolve()` within `timeout`.
pub(crate) async fn resolve(
    handle: &ActorHandle,
    timeout: Duration,
) -> Result<String, ActorCallError> {
    match tokio::time::timeout(timeout, handle.resolve()).await {
        Err(_) => Err(ActorCallError::Timeout),
        Ok(Err(err)) => Err(ActorCallError::from_anyhow(&err)),
        Ok(Ok(id)) => Ok(id),
    }
}

#[cfg(test)]
mod tests;
