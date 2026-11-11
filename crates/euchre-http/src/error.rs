//! The JSON error body (05 §11.1): `{"message": m, "error": {"code": c, "message": m}}`.
//!
//! `message` stays at the top level so the unchanged web client, which read
//! SvelteKit's `{"message"}`, keeps working.

use std::borrow::Cow;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// A non-2xx JSON API response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// HTTP status.
    pub status: StatusCode,
    /// Snake-case code: `PROTOCOL_ERROR_CODES` plus `bad_request`, `unauthorized`,
    /// `forbidden`, `not_found` and `unavailable`.
    pub code: Cow<'static, str>,
    /// Plain-text message, shown to people.
    pub message: Cow<'static, str>,
}

impl ApiError {
    /// An error with an explicit status and code.
    pub fn new(
        status: StatusCode,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
        }
    }

    /// `400 bad_request`.
    pub fn bad_request(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    /// `401 unauthorized`.
    pub fn unauthorized(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    /// `403 forbidden`.
    pub fn forbidden(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    /// `404 not_found`.
    pub fn not_found(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    /// `503 unavailable`.
    pub fn unavailable(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "unavailable", message)
    }

    /// `429 rate_limited`.
    pub fn rate_limited() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            RATE_LIMITED_MESSAGE,
        )
    }

    /// `413 bad_request`: the body is over 1 MiB.
    pub fn payload_too_large() -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "bad_request",
            "That request is too large.",
        )
    }
}

/// Body of every `429`.
pub const RATE_LIMITED_MESSAGE: &str = "Too many requests. Try again in a minute.";

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({
            "message": self.message,
            "error": { "code": self.code, "message": self.message },
        });
        let mut res = (self.status, Json(body)).into_response();
        res.headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        res
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}: {}",
            self.status.as_u16(),
            self.code,
            self.message
        )
    }
}

impl std::error::Error for ApiError {}

/// A plain-text response, as `server/index.ts` returned for `421 Unknown host`,
/// `404 Not found`, `401 Unauthorized` and `400 Bad path`.
pub fn text(status: StatusCode, body: &'static str) -> Response {
    let mut res = (status, body).into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain;charset=UTF-8"),
    );
    res
}

/// A small JSON `{"error": reason}` body, the gateway's own denial shape.
pub fn gateway_json(status: StatusCode, reason: &'static str) -> Response {
    let mut res = (status, Json(json!({ "error": reason }))).into_response();
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    res
}
