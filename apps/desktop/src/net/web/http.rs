//! The browser's HTTP calls: same-origin `fetch` with `credentials: "same-origin"`, so the
//! browser sends and stores the HttpOnly guest cookie itself (no bearer, no device record).
//!
//! Every status becomes a [`NetError`] through [`status_error`] (the same mapping as
//! `net/remote/http.rs`, which is native-only until W2b folds the two together). The `fetch`
//! future holds `JsValue`s, so it runs through [`on_page`] and only `Send` data crosses back.

use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AbortSignal, DomException, Headers, RequestCredentials, RequestInit, Response};

use super::local::on_page;
use crate::net::{NetError, rt};

/// Whole-request bound for every HTTP call.
pub const REQUEST_TIMEOUT_MS: u32 = 15_000;
/// How long `POST /api/tables` keeps retrying `503` (G36).
pub const CREATE_RETRY_FOR: Duration = Duration::from_secs(20);
/// The first wait between two `POST /api/tables` attempts; it doubles up to
/// [`CREATE_RETRY_CAP`].
pub const CREATE_RETRY_FIRST: Duration = Duration::from_millis(250);
/// The longest wait between two `POST /api/tables` attempts.
pub const CREATE_RETRY_CAP: Duration = Duration::from_secs(2);
/// The scrapbook page size the Games screen asks for.
pub const GAMES_PAGE_LIMIT: u32 = 10;
/// The query that selects the browser's guest profile.
pub const PROFILE_BROWSER: &str = "profile=browser";

/// Which call failed: the status mapping differs per call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    /// `GET /api/session`.
    Session,
    /// `POST /api/tables`.
    CreateTable,
    /// `GET /api/tables/recent`.
    RecentTables,
    /// `POST /api/table-session/{gameId}` (made by the bridge; mapped here).
    TableSession,
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

/// Maps a non-2xx status of `call` to the error the player reads. `status: None` is a
/// transport failure: `timed_out` is [`NetError::Timeout`], else [`NetError::Offline`].
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

/// `{"message", "error": {"code", "message"}}` of every JSON `4xx`/`5xx`.
#[derive(Deserialize)]
struct ApiErrorBody {
    error: Option<ApiErrorDetail>,
}

#[derive(Deserialize)]
struct ApiErrorDetail {
    code: Option<String>,
}

/// One failed exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// What the player reads.
    pub err: NetError,
    /// The HTTP status, `None` for a transport failure.
    pub status: Option<u16>,
}

fn other(what: &str) -> Failure {
    Failure {
        err: NetError::Other(what.to_owned()),
        status: None,
    }
}

/// `encodeURIComponent`.
pub fn enc(s: &str) -> String {
    String::from(js_sys::encode_uri_component(s))
}

/// What one fetch returned: the status and the body text.
struct Reply {
    status: u16,
    ok: bool,
    text: String,
}

async fn fetch_text(method: &str, path: &str, body: Option<String>) -> Result<Reply, (bool, ())> {
    let init = RequestInit::new();
    init.set_method(method);
    init.set_credentials(RequestCredentials::SameOrigin);
    let signal = AbortSignal::timeout_with_u32(REQUEST_TIMEOUT_MS);
    init.set_signal(Some(&signal));
    let headers = Headers::new().map_err(|_| (false, ()))?;
    let _ = headers.set("Accept", "application/json");
    if let Some(body) = &body {
        let _ = headers.set("Content-Type", "application/json");
        init.set_body(&JsValue::from_str(body));
    }
    init.set_headers(headers.as_ref());
    let global: web_sys::Window = js_sys::global().unchecked_into();
    let sent = JsFuture::from(global.fetch_with_str_and_init(path, &init)).await;
    let resp: Response = match sent {
        Ok(v) => v.unchecked_into(),
        Err(e) => {
            let timed_out = e
                .dyn_ref::<DomException>()
                .is_some_and(|d| d.name() == "TimeoutError");
            return Err((timed_out, ()));
        }
    };
    let text = match resp.text() {
        Ok(p) => JsFuture::from(p)
            .await
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default(),
        Err(_) => String::new(),
    };
    Ok(Reply {
        status: resp.status(),
        ok: resp.ok(),
        text,
    })
}

/// One same-origin JSON exchange: 2xx decodes into `T`, everything else maps per `call`.
pub async fn exchange<T: DeserializeOwned>(
    call: Call,
    method: &'static str,
    path: String,
    body: Option<String>,
) -> Result<T, Failure> {
    let reply = on_page(async move { fetch_text(method, &path, body).await })
        .await
        .ok_or_else(|| other("page task dropped"))?;
    let reply = reply.map_err(|(timed_out, ())| Failure {
        err: status_error(call, None, timed_out, None),
        status: None,
    })?;
    if !reply.ok {
        let code = serde_json::from_str::<ApiErrorBody>(&reply.text)
            .ok()
            .and_then(|b| b.error)
            .and_then(|e| e.code);
        return Err(Failure {
            err: status_error(call, Some(reply.status), false, code.as_deref()),
            status: Some(reply.status),
        });
    }
    serde_json::from_str(&reply.text).map_err(|e| {
        tracing::warn!(call = ?call, error = %e, "server reply did not decode");
        Failure {
            err: NetError::Unavailable,
            status: Some(reply.status),
        }
    })
}

/// `exchange` that keeps only the player's error.
pub async fn call<T: DeserializeOwned>(
    call: Call,
    method: &'static str,
    path: String,
    body: Option<String>,
) -> Result<T, NetError> {
    exchange(call, method, path, body).await.map_err(|f| f.err)
}

/// `POST /api/tables`, retrying `503` (the first requests after a boot, G36) with backoff
/// until [`CREATE_RETRY_FOR`]. Any other failure (a timeout included: the table may exist)
/// is final.
pub async fn create_table(body: String) -> Result<String, NetError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Created {
        game_id: String,
    }
    let started = crate::net::now_ms();
    let limit = u64::try_from(CREATE_RETRY_FOR.as_millis()).unwrap_or(u64::MAX);
    let mut wait = CREATE_RETRY_FIRST;
    loop {
        match exchange::<Created>(
            Call::CreateTable,
            "POST",
            "/api/tables".to_owned(),
            Some(body.clone()),
        )
        .await
        {
            Ok(c) => return Ok(c.game_id),
            Err(f)
                if f.status == Some(503)
                    && crate::net::now_ms().saturating_sub(started) < limit =>
            {
                rt::sleep(wait).await;
                wait = (wait * 2).min(CREATE_RETRY_CAP);
            }
            Err(f) => return Err(f.err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_per_call() {
        assert_eq!(
            status_error(Call::TableSession, Some(403), false, None),
            NetError::Forbidden
        );
        assert_eq!(
            status_error(Call::Stats, Some(403), false, None),
            NetError::Invalid("forbidden".into())
        );
        assert_eq!(
            status_error(Call::MatchDetail, Some(400), false, None),
            NetError::NotFound
        );
        assert_eq!(
            status_error(Call::Stats, None, true, None),
            NetError::Timeout
        );
        assert_eq!(
            status_error(Call::Stats, Some(429), false, None),
            NetError::RateLimited
        );
    }
}
