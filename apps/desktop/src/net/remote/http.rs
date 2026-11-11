//! The HTTP API client (05 §11.2; contract §N1.2): one `reqwest` client per backend, the
//! device session as `Authorization: Bearer`, and every status mapped to a [`NetError`].
//!
//! Desktop calls never send cookies, `Origin` or `X-Euchre-Identity` (bearer callers are
//! CSRF-exempt; the desktop has no email identity this wave). Profile reads and writes pass
//! `?profile=browser`: the device's guest profile, since settings and stats default to the
//! email profile the desktop never has.
//!
//! Nothing a server writes reaches the player: a status becomes a [`NetError`] through
//! [`status_error`], whose `Display` is this app's own copy. Logs name the call, the status and
//! the error code, never a token or a body.

use std::time::Duration;

use euchre_profile_model::{ClientMatchReplay, ProfileSettings, ProfileStats, SettingsPatch};
use euchre_protocol::{MatchListPage, TableSessionResponse};
use euchre_server_core::auth::DeviceEnrollment;
use reqwest::header::{ACCEPT, HeaderValue};
use reqwest::{Method, RequestBuilder};
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;

use crate::net::NetError;

/// TCP connect bound for every HTTP call.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Whole-request bound for every HTTP call (store commands also have the bridge's bounds).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `POST /api/tables` keeps retrying `503` (the first requests after a boot settle,
/// G36) before it reports [`NetError::Unavailable`].
pub const CREATE_RETRY_FOR: Duration = Duration::from_secs(20);
/// The first wait between two `POST /api/tables` attempts; it doubles up to
/// [`CREATE_RETRY_CAP`].
pub const CREATE_RETRY_FIRST: Duration = Duration::from_millis(250);
/// The longest wait between two `POST /api/tables` attempts.
pub const CREATE_RETRY_CAP: Duration = Duration::from_secs(2);
/// The query that selects the device's guest profile.
pub const PROFILE_BROWSER: &str = "profile=browser";
/// The scrapbook page size the Games screen asks for.
pub const GAMES_PAGE_LIMIT: u32 = 10;
/// The longest error body read for its `error.code` (the server's are a few hundred bytes).
const MAX_ERROR_BODY: usize = 16 * 1024;

/// The JSON bodies `euchre-http` builds with `json!` and no shared type yet
/// (`docs/port/requests/desktop-net-http-types.md`). Mirrors until that lands; each one is
/// pinned by a test against the exact JSON the handler writes.
pub mod wire {
    use serde::{Deserialize, Serialize};

    /// `POST /api/tables` request body.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
    pub struct CreateTableBody {
        /// `cfg.stickTheDealer`.
        pub stick: bool,
        /// `cfg.requireNaturalTrump`.
        pub natural: bool,
    }

    /// `POST /api/tables` `201` body.
    #[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct CreateTableResponse {
        /// The new table's id (lowercase UUID v4).
        pub game_id: String,
    }

    /// One row of `GET /api/tables/recent`.
    #[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RecentTableRow {
        /// The table.
        pub game_id: String,
        /// `"active"` / `"complete"`, or `null` when its snapshot was unavailable.
        pub status: Option<euchre_protocol::GameStatus>,
        /// The score by team, or `null`.
        pub score: Option<[u8; 2]>,
        /// The hand number, or `null`.
        pub hand_no: Option<u32>,
    }

    /// `GET /api/tables/recent` body.
    #[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
    pub struct RecentTablesResponse {
        /// One row per asked id, in the asked order.
        pub tables: Vec<RecentTableRow>,
        /// Some snapshot could not be read.
        pub unavailable: bool,
    }

    /// `GET /api/rivet/metadata` body: where clients dial the gateway.
    #[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RivetMetadata {
        /// `<public origin>/rivet`.
        pub client_endpoint: String,
        /// The engine namespace.
        pub client_namespace: String,
    }

    /// `GET /api/session` body.
    #[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SessionResponse {
        /// The verified guest id, or `null` when the bearer did not verify.
        pub guest_user_id: Option<String>,
    }

    /// The error body of every JSON `4xx`/`5xx`: `{"message", "error": {"code", "message"}}`.
    #[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
    pub struct ApiErrorBody {
        /// Plain-text message (written for browsers; never shown by the desktop).
        pub message: String,
        /// The structured error.
        pub error: Option<ApiErrorDetail>,
    }

    /// The structured part of [`ApiErrorBody`].
    #[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
    pub struct ApiErrorDetail {
        /// `bad_request`, `unauthorized`, `forbidden`, `not_found`, `unavailable`,
        /// `rate_limited`, `invalid_persona`, ...
        pub code: String,
        /// Same as [`ApiErrorBody::message`].
        pub message: String,
    }
}

/// Which call failed: the status mapping differs per call (contract §N1.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    /// `POST /api/device/enroll`.
    Enroll,
    /// `POST /api/device/refresh`.
    Refresh,
    /// `GET /api/session`.
    Session,
    /// `POST /api/tables`.
    CreateTable,
    /// `GET /api/tables/recent`.
    RecentTables,
    /// `POST /api/table-session/{gameId}`.
    TableSession,
    /// `GET /api/rivet/metadata`.
    RivetMetadata,
    /// `GET /api/games`.
    ListGames,
    /// `GET /api/games/{matchId}`.
    MatchDetail,
    /// `GET /api/profile/stats`.
    Stats,
    /// `GET /api/profile/settings`.
    GetSettings,
    /// `PATCH /api/profile/settings`.
    PatchSettings,
}

/// Maps a non-2xx status of `call` to the error the player reads (contract §N1.4). `status:
/// None` is a transport failure: `timed_out` → [`NetError::Timeout`], else
/// [`NetError::Offline`]. `code` is `error.code` of the [`wire::ApiErrorBody`] when the body
/// parsed (it rides in [`NetError::Invalid`] for logs). `401` maps to
/// [`NetError::Unauthorized`]; the caller recovers the device session and retries once
/// before it reports one.
pub fn status_error(
    call: Call,
    status: Option<u16>,
    timed_out: bool,
    code: Option<&str>,
) -> NetError {
    let Some(status) = status else {
        return if timed_out {
            NetError::Timeout
        } else {
            NetError::Offline
        };
    };
    match status {
        400 if matches!(call, Call::TableSession | Call::MatchDetail) => NetError::NotFound,
        400 => NetError::Invalid(code.unwrap_or("bad_request").to_owned()),
        401 => NetError::Unauthorized,
        403 if call == Call::TableSession => NetError::Forbidden,
        403 => NetError::Invalid("forbidden".to_owned()),
        404 => NetError::NotFound,
        413 => NetError::Invalid("too_large".to_owned()),
        429 => NetError::RateLimited,
        _ => NetError::Unavailable,
    }
}

/// One failed exchange: the player's error plus what the retry rules need.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Failure {
    err: NetError,
    status: Option<u16>,
    /// The TCP or TLS connect failed, so the request was never sent.
    unsent: bool,
}

/// The API client: the base URL plus one `reqwest::Client` (`.no_proxy()` for loopback,
/// connect [`CONNECT_TIMEOUT`], request [`REQUEST_TIMEOUT`], no redirects, no cookies).
/// Clones share the client and its connection pool.
#[derive(Clone, Debug)]
pub struct Api {
    base_url: String,
    base: Url,
    http: reqwest::Client,
    create_retry_for: Duration,
}

/// True for the hosts that never go through a proxy.
fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// The one `reqwest` client of an [`Api`] (§N1.2): the timeouts, the user agent, no
/// redirects, no cookie store, and no proxy for a loopback host.
fn build_http(
    base: &Url,
    connect: Duration,
    request: Duration,
) -> Result<reqwest::Client, NetError> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(connect)
        .timeout(request)
        .user_agent(concat!("euchre-desktop/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none());
    if is_loopback_host(base) {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|err| {
        tracing::warn!(error = %err, "HTTP client could not be built");
        NetError::Other("http client".to_owned())
    })
}

impl Api {
    /// A client for `base_url` (`http(s)://host[:port]`, no trailing slash).
    pub fn new(base_url: &str) -> Result<Api, NetError> {
        let trimmed = base_url.trim().trim_end_matches('/');
        let base = Url::parse(trimmed).map_err(|_| NetError::Invalid("server_url".to_owned()))?;
        if !matches!(base.scheme(), "http" | "https") || base.host().is_none() {
            return Err(NetError::Invalid("server_url".to_owned()));
        }
        let http = build_http(&base, CONNECT_TIMEOUT, REQUEST_TIMEOUT)?;
        Ok(Api {
            base_url: trimmed.to_owned(),
            base,
            http,
            create_retry_for: CREATE_RETRY_FOR,
        })
    }

    /// Tests: the same client with other connect and request timeouts (defaults
    /// [`CONNECT_TIMEOUT`], [`REQUEST_TIMEOUT`]), so a timeout test need not wait 10 s.
    pub fn with_timeouts(mut self, connect: Duration, request: Duration) -> Result<Api, NetError> {
        self.http = build_http(&self.base, connect, request)?;
        Ok(self)
    }

    /// Tests: how long `create_table` keeps retrying a `503` (default [`CREATE_RETRY_FOR`]).
    pub fn with_create_retry(mut self, window: Duration) -> Api {
        self.create_retry_for = window;
        self
    }

    /// How long `create_table` keeps retrying a `503`.
    pub fn create_retry_for(&self) -> Duration {
        self.create_retry_for
    }

    /// The base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// `<base>/<segments...>`, each segment percent-encoded.
    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.clear();
            path.extend(segments);
        }
        url
    }

    fn request(&self, method: Method, segments: &[&str], bearer: Option<&str>) -> RequestBuilder {
        let req = self
            .http
            .request(method, self.url(segments))
            .header(ACCEPT, HeaderValue::from_static("application/json"));
        match bearer {
            Some(token) => req.bearer_auth(token),
            None => req,
        }
    }

    /// Sends `req` and decodes a 2xx body into `T`; maps everything else per §N1.4.
    async fn exchange<T: DeserializeOwned>(
        &self,
        call: Call,
        req: RequestBuilder,
    ) -> Result<T, Failure> {
        let res = match req.send().await {
            Ok(res) => res,
            Err(err) => {
                let unsent = err.is_connect();
                let failure = Failure {
                    err: status_error(call, None, err.is_timeout(), None),
                    status: None,
                    unsent,
                };
                tracing::warn!(
                    call = ?call,
                    timed_out = err.is_timeout(),
                    connect = unsent,
                    "server call failed in transport"
                );
                return Err(failure);
            }
        };
        let status = res.status().as_u16();
        if res.status().is_success() {
            let bytes = res.bytes().await.map_err(|err| {
                tracing::warn!(call = ?call, status, timed_out = err.is_timeout(), "server reply cut short");
                Failure {
                    err: status_error(call, None, err.is_timeout(), None),
                    status: None,
                    unsent: false,
                }
            })?;
            return serde_json::from_slice::<T>(&bytes).map_err(|err| {
                tracing::warn!(call = ?call, status, error = %err, "server reply did not decode");
                Failure {
                    err: NetError::Unavailable,
                    status: Some(status),
                    unsent: false,
                }
            });
        }
        let code = match res.bytes().await {
            Ok(bytes) if bytes.len() <= MAX_ERROR_BODY => {
                serde_json::from_slice::<wire::ApiErrorBody>(&bytes)
                    .ok()
                    .and_then(|body| body.error)
                    .map(|detail| detail.code)
            }
            _ => None,
        };
        let err = status_error(call, Some(status), false, code.as_deref());
        tracing::warn!(call = ?call, status, code = code.as_deref().unwrap_or("-"), "server refused a call");
        Err(Failure {
            err,
            status: Some(status),
            unsent: false,
        })
    }

    async fn call<T: DeserializeOwned>(
        &self,
        call: Call,
        req: RequestBuilder,
    ) -> Result<T, NetError> {
        self.exchange(call, req).await.map_err(|f| f.err)
    }

    /// Serializes `body` and sets `Content-Type: application/json` (once: `.json()` only adds
    /// the header when none is set).
    fn with_json<B: Serialize + ?Sized>(req: RequestBuilder, body: &B) -> RequestBuilder {
        req.json(body)
    }

    /// `POST /api/device/enroll` (no auth) → `200` [`DeviceEnrollment`].
    pub async fn enroll(&self) -> Result<DeviceEnrollment, NetError> {
        let req = self.request(Method::POST, &["api", "device", "enroll"], None);
        self.call(Call::Enroll, req).await
    }

    /// `POST /api/device/refresh` (bearer) → `200` [`DeviceEnrollment`].
    pub async fn refresh(&self, bearer: &str) -> Result<DeviceEnrollment, NetError> {
        let req = self.request(Method::POST, &["api", "device", "refresh"], Some(bearer));
        self.call(Call::Refresh, req).await
    }

    /// `GET /api/session` (bearer) → `200` [`wire::SessionResponse`].
    pub async fn session(&self, bearer: &str) -> Result<wire::SessionResponse, NetError> {
        let req = self.request(Method::GET, &["api", "session"], Some(bearer));
        self.call(Call::Session, req).await
    }

    /// `POST /api/tables` (bearer, [`wire::CreateTableBody`]) → `201` game id.
    ///
    /// A `503`, or a connect failure (nothing was sent), is retried with backoff from
    /// [`CREATE_RETRY_FIRST`] to [`CREATE_RETRY_CAP`] until [`Api::create_retry_for`] has
    /// passed: the first requests after a boot can see `503` while the runner settles (G36).
    /// A read timeout is never retried, since the table may exist: it reports
    /// [`NetError::Timeout`].
    pub async fn create_table(
        &self,
        bearer: &str,
        body: wire::CreateTableBody,
    ) -> Result<String, NetError> {
        let deadline = tokio::time::Instant::now() + self.create_retry_for;
        let mut wait = CREATE_RETRY_FIRST;
        loop {
            let req = Self::with_json(
                self.request(Method::POST, &["api", "tables"], Some(bearer)),
                &body,
            );
            match self
                .exchange::<wire::CreateTableResponse>(Call::CreateTable, req)
                .await
            {
                Ok(created) => return Ok(created.game_id),
                Err(failure) => {
                    let retryable = failure.status == Some(503) || failure.unsent;
                    let now = tokio::time::Instant::now();
                    if !retryable || now >= deadline {
                        return Err(failure.err);
                    }
                    tokio::time::sleep(wait.min(deadline - now)).await;
                    wait = (wait * 2).min(CREATE_RETRY_CAP);
                }
            }
        }
    }

    /// `GET /api/tables/recent?ids=a,b` (bearer) → `200` [`wire::RecentTablesResponse`].
    pub async fn recent_tables(
        &self,
        bearer: &str,
        ids: &[String],
    ) -> Result<wire::RecentTablesResponse, NetError> {
        let req = self
            .request(Method::GET, &["api", "tables", "recent"], Some(bearer))
            .query(&[("ids", ids.join(","))]);
        self.call(Call::RecentTables, req).await
    }

    /// `POST /api/table-session/{gameId}` (bearer) → `200` [`TableSessionResponse`].
    pub async fn table_session(
        &self,
        bearer: &str,
        game_id: &str,
    ) -> Result<TableSessionResponse, NetError> {
        let req = self.request(
            Method::POST,
            &["api", "table-session", game_id],
            Some(bearer),
        );
        self.call(Call::TableSession, req).await
    }

    /// `GET /api/rivet/metadata` (no auth) → `200` [`wire::RivetMetadata`].
    pub async fn rivet_metadata(&self) -> Result<wire::RivetMetadata, NetError> {
        let req = self.request(Method::GET, &["api", "rivet", "metadata"], None);
        self.call(Call::RivetMetadata, req).await
    }

    /// `GET /api/games?profile=browser&limit=10[&cursor=]` (bearer) → [`MatchListPage`].
    pub async fn list_games(
        &self,
        bearer: &str,
        cursor: Option<&str>,
    ) -> Result<MatchListPage, NetError> {
        let mut query = vec![
            ("profile", "browser".to_owned()),
            ("limit", GAMES_PAGE_LIMIT.to_string()),
        ];
        if let Some(cursor) = cursor.filter(|c| !c.is_empty()) {
            query.push(("cursor", cursor.to_owned()));
        }
        let req = self
            .request(Method::GET, &["api", "games"], Some(bearer))
            .query(&query);
        self.call(Call::ListGames, req).await
    }

    /// `GET /api/games/{matchId}?profile=browser` (bearer) → [`ClientMatchReplay`].
    pub async fn match_detail(
        &self,
        bearer: &str,
        match_id: &str,
    ) -> Result<ClientMatchReplay, NetError> {
        let req = self
            .request(Method::GET, &["api", "games", match_id], Some(bearer))
            .query(&[("profile", "browser")]);
        self.call(Call::MatchDetail, req).await
    }

    /// `GET /api/profile/stats?profile=browser` (bearer) → [`ProfileStats`].
    pub async fn stats(&self, bearer: &str) -> Result<ProfileStats, NetError> {
        let req = self
            .request(Method::GET, &["api", "profile", "stats"], Some(bearer))
            .query(&[("profile", "browser")]);
        self.call(Call::Stats, req).await
    }

    /// `GET /api/profile/settings?profile=browser` (bearer) → [`ProfileSettings`].
    pub async fn get_settings(&self, bearer: &str) -> Result<ProfileSettings, NetError> {
        let req = self
            .request(Method::GET, &["api", "profile", "settings"], Some(bearer))
            .query(&[("profile", "browser")]);
        self.call(Call::GetSettings, req).await
    }

    /// `PATCH /api/profile/settings?profile=browser` (bearer, [`SettingsPatch`]) →
    /// [`ProfileSettings`].
    pub async fn patch_settings(
        &self,
        bearer: &str,
        patch: &SettingsPatch,
    ) -> Result<ProfileSettings, NetError> {
        let req = Self::with_json(
            self.request(Method::PATCH, &["api", "profile", "settings"], Some(bearer))
                .query(&[("profile", "browser")]),
            patch,
        );
        self.call(Call::PatchSettings, req).await
    }
}

#[cfg(test)]
mod tests {
    use super::wire::*;
    use super::*;
    use euchre_protocol::GameStatus;
    use serde_json::json;

    #[test]
    fn status_error_maps_every_status_per_call() {
        use Call::*;
        const ALL: [Call; 12] = [
            Enroll,
            Refresh,
            Session,
            CreateTable,
            RecentTables,
            TableSession,
            RivetMetadata,
            ListGames,
            MatchDetail,
            Stats,
            GetSettings,
            PatchSettings,
        ];
        for call in ALL {
            // Transport failures.
            assert_eq!(status_error(call, None, true, None), NetError::Timeout);
            assert_eq!(status_error(call, None, false, None), NetError::Offline);
            // 400: a bad id is "not found" for the two id routes, else a refusal.
            let bad = status_error(call, Some(400), false, Some("bad_request"));
            if matches!(call, TableSession | MatchDetail) {
                assert_eq!(bad, NetError::NotFound, "{call:?}");
            } else {
                assert_eq!(bad, NetError::Invalid("bad_request".into()), "{call:?}");
                assert_eq!(
                    status_error(call, Some(400), false, None),
                    NetError::Invalid("bad_request".into())
                );
                assert_eq!(
                    status_error(call, Some(400), false, Some("invalid_persona")),
                    NetError::Invalid("invalid_persona".into())
                );
            }
            assert_eq!(
                status_error(call, Some(401), false, Some("unauthorized")),
                NetError::Unauthorized
            );
            let forbidden = status_error(call, Some(403), false, Some("forbidden"));
            if call == TableSession {
                assert_eq!(forbidden, NetError::Forbidden);
            } else {
                assert_eq!(forbidden, NetError::Invalid("forbidden".into()), "{call:?}");
            }
            assert_eq!(
                status_error(call, Some(404), false, None),
                NetError::NotFound
            );
            assert_eq!(
                status_error(call, Some(413), false, Some("bad_request")),
                NetError::Invalid("too_large".into())
            );
            assert_eq!(
                status_error(call, Some(429), false, Some("rate_limited")),
                NetError::RateLimited
            );
            for other in [421, 500, 502, 503, 504, 302, 418] {
                assert_eq!(
                    status_error(call, Some(other), false, None),
                    NetError::Unavailable,
                    "{call:?} {other}"
                );
            }
        }
    }

    #[test]
    fn wire_bodies_match_the_handlers() -> anyhow::Result<()> {
        // `POST /api/tables` reads `{stick, natural}` booleans.
        assert_eq!(
            serde_json::to_value(CreateTableBody {
                stick: true,
                natural: false
            })?,
            json!({ "stick": true, "natural": false })
        );
        // `(StatusCode::CREATED, Json(json!({ "gameId": game_id })))`.
        let created: CreateTableResponse =
            serde_json::from_value(json!({ "gameId": "4f1c2b0e-8d7a-4f61-9a3e-2b1c0d9e8f7a" }))?;
        assert_eq!(created.game_id, "4f1c2b0e-8d7a-4f61-9a3e-2b1c0d9e8f7a");
        // `recent_tables`: a read row and an unavailable one, in the asked order.
        let recent: RecentTablesResponse = serde_json::from_value(json!({
            "tables": [
                { "gameId": "a", "status": "active", "score": [3, 7], "handNo": 4 },
                { "gameId": "b", "status": "complete", "score": [10, 6], "handNo": 9 },
                { "gameId": "c", "status": null, "score": null, "handNo": null },
            ],
            "unavailable": true,
        }))?;
        assert!(recent.unavailable);
        assert_eq!(
            recent.tables,
            vec![
                RecentTableRow {
                    game_id: "a".into(),
                    status: Some(GameStatus::Active),
                    score: Some([3, 7]),
                    hand_no: Some(4),
                },
                RecentTableRow {
                    game_id: "b".into(),
                    status: Some(GameStatus::Complete),
                    score: Some([10, 6]),
                    hand_no: Some(9),
                },
                RecentTableRow {
                    game_id: "c".into(),
                    status: None,
                    score: None,
                    hand_no: None,
                },
            ]
        );
        let empty: RecentTablesResponse =
            serde_json::from_value(json!({ "tables": [], "unavailable": false }))?;
        assert!(empty.tables.is_empty() && !empty.unavailable);
        // `rivet_metadata`.
        let meta: RivetMetadata = serde_json::from_value(json!({
            "clientEndpoint": "http://127.0.0.1:41234/rivet",
            "clientNamespace": "default",
        }))?;
        assert_eq!(meta.client_endpoint, "http://127.0.0.1:41234/rivet");
        assert_eq!(meta.client_namespace, "default");
        // `get_session`: `{guestUserId, identity}`.
        let verified: SessionResponse =
            serde_json::from_value(json!({ "guestUserId": "guest-1", "identity": null }))?;
        assert_eq!(verified.guest_user_id.as_deref(), Some("guest-1"));
        let unverified: SessionResponse =
            serde_json::from_value(json!({ "guestUserId": null, "identity": null }))?;
        assert_eq!(unverified.guest_user_id, None);
        // `ApiError::into_response`.
        let err: ApiErrorBody = serde_json::from_value(json!({
            "message": "Please start a game first",
            "error": { "code": "unauthorized", "message": "Please start a game first" },
        }))?;
        assert_eq!(err.error.map(|e| e.code).as_deref(), Some("unauthorized"));
        // The shared types the handlers serialize.
        let enrolled: DeviceEnrollment = serde_json::from_value(
            json!({ "userId": "guest-1", "sessionToken": "t", "expiresAt": 1_900_000_000_000_i64 }),
        )?;
        assert_eq!(enrolled.user_id, "guest-1");
        let session: TableSessionResponse = serde_json::from_value(json!({
            "actorId": "actor-1", "token": "a", "gatewayToken": "g", "expiresAt": 5_u64,
        }))?;
        assert_eq!(session.gateway_token.as_deref(), Some("g"));
        let no_gateway: TableSessionResponse = serde_json::from_value(
            json!({ "actorId": "actor-1", "token": "a", "expiresAt": 5_u64 }),
        )?;
        assert_eq!(no_gateway.gateway_token, None);
        Ok(())
    }

    #[test]
    fn urls_encode_segments_and_keep_the_origin() -> anyhow::Result<()> {
        let api = Api::new("http://127.0.0.1:8080/")?;
        assert_eq!(api.base_url(), "http://127.0.0.1:8080");
        assert_eq!(
            api.url(&["api", "games", "a b/c"]).as_str(),
            "http://127.0.0.1:8080/api/games/a%20b%2Fc"
        );
        assert!(is_loopback_host(&Url::parse("http://localhost:1")?));
        assert!(is_loopback_host(&Url::parse("http://[::1]:1")?));
        assert!(!is_loopback_host(&Url::parse("https://euchre.example")?));
        assert!(Api::new("ftp://x").is_err());
        assert!(Api::new("not a url").is_err());
        Ok(())
    }
}
