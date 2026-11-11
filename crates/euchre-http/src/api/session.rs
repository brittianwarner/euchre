//! `GET /api/session`, device enroll/refresh (D5) and the email-nickname identity.

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use euchre_server_core::auth::AuthError;
use serde::Deserialize;
use serde_json::json;

use super::{ClientAddr, Origin, no_store, rate_limit, read_json};
use crate::caller::{Caller, Session, bearer_token};
use crate::cookies::{IDENTITY_COOKIE, clear_cookie, set_cookie};
use crate::error::ApiError;
use crate::origin::{require_allowed_origin, require_same_origin};
use crate::rate_limit::RouteClass;
use crate::state::ApiState;

/// Message for a refresh without a valid session bearer.
pub(crate) const REFRESH_UNAUTHORIZED: &str = "Please start a game first";

fn signing_failed(err: &AuthError) -> ApiError {
    tracing::error!(code = err.code(), "token signing failed");
    ApiError::unavailable("Unable to start a session just now. Please retry.")
}

/// `GET /api/session`: `{guestUserId, identity}`; never sets cookies.
pub(crate) async fn get_session(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    let caller = Caller::resolve(&state.inner.auth, &headers);
    no_store(Json(json!({
        "guestUserId": caller.guest_user_id(),
        "identity": caller.identity,
    })))
}

/// `POST /api/device/enroll`: a new device guest identity (D5).
pub(crate) async fn enroll(
    State(state): State<ApiState>,
    ClientAddr(socket): ClientAddr,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    rate_limit(&state, RouteClass::Enroll, &headers, socket)?;
    require_allowed_origin(&headers, &state.inner.config.origins)?;
    let enrollment = state
        .inner
        .auth
        .enroll_device()
        .map_err(|e| signing_failed(&e))?;
    Ok(no_store(Json(enrollment)))
}

/// `POST /api/device/refresh`: rotate a valid session bearer.
pub(crate) async fn refresh(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let Some(bearer) = bearer_token(&headers) else {
        return Err(ApiError::unauthorized(REFRESH_UNAUTHORIZED));
    };
    let enrollment = state
        .inner
        .auth
        .refresh_device(bearer)
        .map_err(|e| match e {
            AuthError::Signing(_) => signing_failed(&e),
            _ => ApiError::unauthorized(REFRESH_UNAUTHORIZED),
        })?;
    Ok(no_store(Json(enrollment)))
}

#[derive(Deserialize)]
struct IdentifyBody {
    #[serde(default)]
    email: Option<String>,
}

/// `POST /api/identity/identify {email}`: the signed identity token (11.4), also
/// set as the `euchre_email` cookie for cookie callers.
pub(crate) async fn identify(
    State(state): State<ApiState>,
    ClientAddr(socket): ClientAddr,
    Origin(public_origin): Origin,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    rate_limit(&state, RouteClass::Identify, &headers, socket)?;
    let caller = Caller::resolve(&state.inner.auth, &headers);
    if caller.session == Session::InvalidBearer {
        return Err(ApiError::unauthorized(REFRESH_UNAUTHORIZED));
    }
    if !caller.is_bearer() {
        require_same_origin(&headers, &public_origin)?;
    }
    let body: Option<IdentifyBody> = read_json(body).await?;
    let email = body.and_then(|b| b.email).unwrap_or_default();
    let auth = &state.inner.auth;
    let identity = auth
        .identity_for_email(&email)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let token = auth
        .issue_identity_token(&identity)
        .map_err(|e| signing_failed(&e))?;
    let cookie = (!caller.is_bearer())
        .then(|| {
            set_cookie(
                IDENTITY_COOKIE,
                token.clone(),
                state.inner.config.secure_cookies,
            )
        })
        .flatten();
    let mut res = no_store(Json(
        json!({ "identity": identity, "identityToken": token }),
    ));
    if let Some(cookie) = cookie {
        res.headers_mut().append(header::SET_COOKIE, cookie);
    }
    Ok(res)
}

/// `DELETE /api/identity`: forget the nickname (`204`, cookie cleared).
pub(crate) async fn sign_out(
    State(state): State<ApiState>,
    Origin(public_origin): Origin,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let caller = Caller::resolve(&state.inner.auth, &headers);
    if !caller.is_bearer() {
        require_same_origin(&headers, &public_origin)?;
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    if let Some(cookie) = clear_cookie(IDENTITY_COOKIE, state.inner.config.secure_cookies) {
        res.headers_mut().append(header::SET_COOKIE, cookie);
    }
    Ok(res)
}
