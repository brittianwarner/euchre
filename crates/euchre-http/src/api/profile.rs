//! Profile endpoints: settings, personas, stats, the scrapbook, replays, episodes
//! and the persona presets (05 §11.2). Each call mints a 30-minute profile token
//! server-side (never returned) and calls `playerProfile` `["user", userId]`.

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use euchre_profile_model::PERSONA_PRESETS;
use euchre_profile_model::guards::{is_valid_episode_id, is_valid_id};
use euchre_protocol::actor_names::PLAYER_PROFILE;
use euchre_protocol::profile_actions::{
    FORGET_EPISODE, GET_PERSONAS, GET_REPLAY, GET_SETTINGS, GET_STATS, LIST_EPISODES, LIST_GAMES,
    UPDATE_SETTINGS,
};
use euchre_protocol::profile_key;
use rivetkit_client::GetOrCreateOptions;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{ActorCallError, Origin, call, no_store, read_json};
use crate::caller::Caller;
use crate::error::ApiError;
use crate::origin::require_same_origin;
use crate::state::ApiState;

/// `401` when saving without an identity.
pub(crate) const SAVE_NEEDS_IDENTITY: &str = "Tell us who you are before saving.";
/// `401` when reading the email profile without an identity.
pub(crate) const NEEDS_IDENTITY: &str = "Tell us who you are first.";
/// `401` when neither an identity nor a guest session is present.
pub(crate) const NEEDS_SESSION: &str = "Please start a game first";
/// `400` when an update is refused.
pub(crate) const SAVE_FAILED: &str =
    "Couldn\u{2019}t save that \u{2014} one of the fields may be too long or an odd value.";
/// `503` for the settings family.
pub(crate) const SETTINGS_UNAVAILABLE: &str =
    "Couldn't open your settings just now. Try refreshing the page.";
/// `503` for the scrapbook family.
pub(crate) const SCRAPBOOK_UNAVAILABLE: &str =
    "Couldn't open your scrapbook just now. Try refreshing the page.";
/// `404` for a missing replay.
pub(crate) const NO_SUCH_GAME: &str = "No game like that in your scrapbook.";
/// `400` for a malformed match or episode id.
pub(crate) const BAD_ID: &str = "That id doesn\u{2019}t look right.";

#[derive(Deserialize, Default)]
pub(crate) struct ProfileQuery {
    profile: Option<String>,
}

/// Which profile an endpoint reads when `profile=` is absent.
#[derive(Clone, Copy)]
enum Default_ {
    /// The email identity only.
    Email,
    /// The email identity, else the browser guest.
    EmailThenBrowser,
}

/// The `playerProfile` user id for this request.
fn target(caller: &Caller, profile: Option<&str>, default: Default_) -> Result<String, ApiError> {
    let email = caller.identity.as_ref().map(|i| i.user_id.clone());
    let browser = caller.guest_user_id().map(str::to_owned);
    match (profile, default) {
        (Some("browser"), _) => browser.ok_or_else(|| ApiError::unauthorized(NEEDS_SESSION)),
        (Some("email"), _) | (_, Default_::Email) => {
            email.ok_or_else(|| ApiError::unauthorized(NEEDS_IDENTITY))
        }
        (_, Default_::EmailThenBrowser) => email
            .or(browser)
            .ok_or_else(|| ApiError::unauthorized(NEEDS_SESSION)),
    }
}

/// How a family maps actor failures.
#[derive(Clone, Copy)]
struct Family {
    /// `503` message for transport failures.
    unavailable: &'static str,
    /// `400` message for other user errors; `None` passes the actor's message.
    refused: Option<&'static str>,
}

const SETTINGS: Family = Family {
    unavailable: SETTINGS_UNAVAILABLE,
    refused: None,
};
const SCRAPBOOK: Family = Family {
    unavailable: SCRAPBOOK_UNAVAILABLE,
    refused: None,
};

fn map_error(err: ActorCallError, family: Family, action: &str) -> ApiError {
    match err {
        ActorCallError::Rivet(e) if e.is_user() => match e.code.as_str() {
            "invalid_persona" => ApiError::new(
                axum::http::StatusCode::BAD_REQUEST,
                "invalid_persona",
                e.message,
            ),
            "unauthorized" | "invalid_token" | "forbidden" => {
                ApiError::unauthorized(NEEDS_IDENTITY)
            }
            _ => ApiError::bad_request(family.refused.map_or(e.message, str::to_owned)),
        },
        other => {
            tracing::warn!(
                action = action,
                code = other.code(),
                "profile call unavailable"
            );
            ApiError::unavailable(family.unavailable)
        }
    }
}

async fn profile_call(
    state: &ApiState,
    user_id: &str,
    action: &str,
    args: Vec<Value>,
    family: Family,
) -> Result<Value, ApiError> {
    let inner = &state.inner;
    let token = inner
        .auth
        .mint_profile_token(user_id)
        .map_err(|_| ApiError::unavailable(family.unavailable))?;
    let handle = inner
        .client
        .get_or_create(
            PLAYER_PROFILE,
            profile_key(user_id).to_vec(),
            GetOrCreateOptions {
                params: Some(json!({ "token": token })),
                pool_name: Some(inner.config.pool_name.clone()),
                ..GetOrCreateOptions::default()
            },
        )
        .map_err(|_| ApiError::unavailable(family.unavailable))?;
    call(&handle, action, args, inner.config.timeouts.actor_call)
        .await
        .map_err(|e| map_error(e, family, action))
}

fn ok(value: Value) -> Response {
    no_store(Json(value))
}

/// `GET /api/profile/settings`.
pub(crate) async fn get_settings(
    State(state): State<ApiState>,
    Query(q): Query<ProfileQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let caller = Caller::resolve(&state.inner.auth, &headers);
    let user = target(&caller, q.profile.as_deref(), Default_::Email)?;
    profile_call(&state, &user, GET_SETTINGS, vec![], SETTINGS)
        .await
        .map(ok)
}

/// `PUT|PATCH /api/profile/settings` with a `SettingsPatch`.
pub(crate) async fn save_settings(
    State(state): State<ApiState>,
    Origin(public_origin): Origin,
    Query(q): Query<ProfileQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let caller = Caller::resolve(&state.inner.auth, &headers);
    if !caller.is_bearer() {
        require_same_origin(&headers, &public_origin)?;
    }
    let user = target(&caller, q.profile.as_deref(), Default_::Email)
        .map_err(|_| ApiError::unauthorized(SAVE_NEEDS_IDENTITY))?;
    let patch: Value = read_json(body)
        .await?
        .unwrap_or_else(|| Value::Object(Map::new()));
    let family = Family {
        unavailable: SETTINGS_UNAVAILABLE,
        refused: Some(SAVE_FAILED),
    };
    profile_call(&state, &user, UPDATE_SETTINGS, vec![patch], family)
        .await
        .map(ok)
}

/// `GET /api/profile/personas`.
pub(crate) async fn get_personas(
    State(state): State<ApiState>,
    Query(q): Query<ProfileQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let caller = Caller::resolve(&state.inner.auth, &headers);
    let user = target(&caller, q.profile.as_deref(), Default_::Email)?;
    profile_call(&state, &user, GET_PERSONAS, vec![], SETTINGS)
        .await
        .map(ok)
}

/// `GET /api/profile/stats`.
pub(crate) async fn get_stats(
    State(state): State<ApiState>,
    Query(q): Query<ProfileQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let caller = Caller::resolve(&state.inner.auth, &headers);
    let user = target(&caller, q.profile.as_deref(), Default_::Email)?;
    profile_call(&state, &user, GET_STATS, vec![], SCRAPBOOK)
        .await
        .map(ok)
}

#[derive(Deserialize, Default)]
pub(crate) struct GamesQuery {
    profile: Option<String>,
    cursor: Option<String>,
    limit: Option<String>,
}

/// A numeric query value as a JSON number, else the raw string (the actor clamps
/// or rejects it).
fn limit_value(raw: Option<&str>, default: i64) -> Value {
    match raw.map(str::trim) {
        None | Some("") => json!(default),
        Some(text) => text
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(text.to_owned())),
    }
}

/// `GET /api/games?cursor=&limit=`: a scrapbook page (`limit` defaults to 10).
pub(crate) async fn list_games(
    State(state): State<ApiState>,
    Query(q): Query<GamesQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let caller = Caller::resolve(&state.inner.auth, &headers);
    let user = target(&caller, q.profile.as_deref(), Default_::EmailThenBrowser)?;
    let mut req = Map::new();
    req.insert("limit".into(), limit_value(q.limit.as_deref(), 10));
    if let Some(cursor) = q.cursor.filter(|c| !c.is_empty()) {
        req.insert("cursor".into(), Value::String(cursor));
    }
    profile_call(
        &state,
        &user,
        LIST_GAMES,
        vec![Value::Object(req)],
        SCRAPBOOK,
    )
    .await
    .map(ok)
}

/// `GET /api/games/{matchId}`: a client-safe replay.
pub(crate) async fn get_replay(
    State(state): State<ApiState>,
    Path(match_id): Path<String>,
    Query(q): Query<ProfileQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if !is_valid_id(&match_id) {
        return Err(ApiError::bad_request(BAD_ID));
    }
    let caller = Caller::resolve(&state.inner.auth, &headers);
    let user = target(&caller, q.profile.as_deref(), Default_::EmailThenBrowser)?;
    let replay = profile_call(&state, &user, GET_REPLAY, vec![json!(match_id)], SCRAPBOOK).await?;
    if replay.is_null() {
        return Err(ApiError::not_found(NO_SUCH_GAME));
    }
    Ok(ok(replay))
}

#[derive(Deserialize, Default)]
pub(crate) struct EpisodesQuery {
    profile: Option<String>,
    limit: Option<String>,
}

/// `GET /api/episodes?limit=`.
pub(crate) async fn list_episodes(
    State(state): State<ApiState>,
    Query(q): Query<EpisodesQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let caller = Caller::resolve(&state.inner.auth, &headers);
    let user = target(&caller, q.profile.as_deref(), Default_::Email)?;
    let args = match q.limit.as_deref() {
        None | Some("") => vec![],
        Some(raw) => vec![limit_value(Some(raw), 0)],
    };
    profile_call(&state, &user, LIST_EPISODES, args, SCRAPBOOK)
        .await
        .map(ok)
}

/// `DELETE /api/episodes/{episodeId}`: `{"forgotten": bool}`.
pub(crate) async fn forget_episode(
    State(state): State<ApiState>,
    Origin(public_origin): Origin,
    Path(episode_id): Path<String>,
    Query(q): Query<ProfileQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let caller = Caller::resolve(&state.inner.auth, &headers);
    if !caller.is_bearer() {
        require_same_origin(&headers, &public_origin)?;
    }
    // D30: stored ids are `role.matchId.handNo.kind` (TS refused every one).
    if !is_valid_episode_id(&episode_id) {
        return Err(ApiError::bad_request(BAD_ID));
    }
    let user = target(&caller, q.profile.as_deref(), Default_::Email)?;
    let forgotten = profile_call(
        &state,
        &user,
        FORGET_EPISODE,
        vec![json!(episode_id)],
        SCRAPBOOK,
    )
    .await?;
    Ok(ok(
        json!({ "forgotten": forgotten.as_bool().unwrap_or(false) }),
    ))
}

/// `GET /api/presets`.
pub(crate) async fn presets() -> Response {
    Json(PERSONA_PRESETS.to_vec()).into_response()
}
