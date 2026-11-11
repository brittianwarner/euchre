//! Tables: create (TS `/play`), recent tables (TS `loadRecentTables`) and the
//! connection-credential mint (TS `POST /api/table-session/[gameId]`).

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use euchre_profile_model::{ProfileSettings, personas_from_settings};
use euchre_protocol::actor_names::{EUCHRE_TABLE, PLAYER_PROFILE};
use euchre_protocol::profile_actions::GET_SETTINGS;
use euchre_protocol::table_actions::SNAPSHOT;
use euchre_protocol::{TableSessionResponse, default_personas, is_game_id, profile_key, table_key};
use euchre_server_core::auth::guest::new_guest_id;
use euchre_server_core::auth::{GuestAudience, Identity, now_ms};
use rivetkit_client::{CreateOptions, GetOptions, GetOrCreateOptions};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{ActorCallError, ClientAddr, Origin, call, no_store, rate_limit, read_json, resolve};
use crate::caller::{Caller, Session};
use crate::cookies::{
    GUEST_COOKIE, RECENT_TABLES_COOKIE, read_cookie, recent_table_ids, remember_table, set_cookie,
};
use crate::error::ApiError;
use crate::origin::require_same_origin;
use crate::rate_limit::RouteClass;
use crate::state::ApiState;
use crate::token::GATEWAY_TOKEN_TTL_SECS;

/// `400` for a malformed table id.
pub(crate) const INVALID_TABLE: &str = "Invalid table";
/// `401` without a guest session.
pub(crate) const START_A_GAME_FIRST: &str = "Please start a game first";
/// `403` when the table's owner is another guest.
pub(crate) const ANOTHER_BROWSER: &str = "This table belongs to another browser";
/// `503` when the table cannot answer.
pub(crate) const TABLE_UNAVAILABLE: &str =
    "Your table is temporarily unavailable. Please reconnect.";
/// `503` when a table cannot be created (TS's `500` error-handler body).
pub(crate) const CREATE_FAILED: &str = "Unable to load the table. Please retry.";

fn signing_failed() -> ApiError {
    tracing::error!("token signing failed");
    ApiError::unavailable(TABLE_UNAVAILABLE)
}

/// Personas from the identity's profile settings (2 s bound), else the defaults.
async fn personas_for(state: &ApiState, identity: Option<&Identity>) -> Value {
    let defaults = || serde_json::to_value(default_personas()).unwrap_or(Value::Null);
    let Some(identity) = identity else {
        return defaults();
    };
    let inner = &state.inner;
    let loaded = async {
        let token = inner
            .auth
            .mint_profile_token(&identity.user_id)
            .map_err(|_| ActorCallError::Other)?;
        let handle = inner
            .client
            .get_or_create(
                PLAYER_PROFILE,
                profile_key(&identity.user_id).to_vec(),
                GetOrCreateOptions {
                    params: Some(json!({ "token": token })),
                    pool_name: Some(inner.config.pool_name.clone()),
                    ..GetOrCreateOptions::default()
                },
            )
            .map_err(|_| ActorCallError::Other)?;
        let settings = call(
            &handle,
            GET_SETTINGS,
            vec![],
            inner.config.timeouts.persona_settings,
        )
        .await?;
        let settings: ProfileSettings =
            serde_json::from_value(settings).map_err(|_| ActorCallError::Other)?;
        serde_json::to_value(personas_from_settings(&settings)).map_err(|_| ActorCallError::Other)
    };
    match tokio::time::timeout(inner.config.timeouts.persona_settings, loaded).await {
        Ok(Ok(personas)) => personas,
        _ => {
            tracing::warn!("Persona settings unavailable; starting with kitchen-table players");
            defaults()
        }
    }
}

#[derive(Deserialize, Default)]
pub(crate) struct CreateQuery {
    stick: Option<String>,
    natural: Option<String>,
}

#[derive(Deserialize, Default)]
struct CreateBody {
    #[serde(default)]
    stick: Option<bool>,
    #[serde(default)]
    natural: Option<bool>,
}

/// `POST /api/tables`: create a table owned by the caller's guest identity.
pub(crate) async fn create_table(
    State(state): State<ApiState>,
    ClientAddr(socket): ClientAddr,
    Origin(public_origin): Origin,
    Query(query): Query<CreateQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    rate_limit(&state, RouteClass::Tables, &headers, socket)?;
    let inner = &state.inner;
    let caller = Caller::resolve(&inner.auth, &headers);
    if caller.session == Session::InvalidBearer {
        return Err(ApiError::unauthorized(START_A_GAME_FIRST));
    }
    let ambient = caller.is_ambient();
    if ambient {
        require_same_origin(&headers, &public_origin)?;
    }
    let body: CreateBody = read_json(body).await?.unwrap_or_default();
    let stick = body
        .stick
        .unwrap_or_else(|| query.stick.as_deref() == Some("true"));
    let natural = body
        .natural
        .unwrap_or_else(|| query.natural.as_deref() == Some("true"));

    // A cookie caller without a valid session gets a new guest identity (TS
    // `guestIdentity`: an expired or tampered session is replaced).
    let mut new_guest_cookie = None;
    let owner = match caller.guest_user_id() {
        Some(id) => id.to_owned(),
        None => {
            let id = new_guest_id();
            let token = inner
                .auth
                .sign_guest(&id, GuestAudience::Session)
                .map_err(|_| signing_failed())?;
            new_guest_cookie = set_cookie(GUEST_COOKIE, token, inner.config.secure_cookies);
            id
        }
    };

    let personas = personas_for(&state, caller.identity.as_ref()).await;
    let game_id = uuid::Uuid::new_v4().to_string();
    let mut input = Map::new();
    input.insert("ownerUserId".into(), Value::String(owner));
    input.insert("personas".into(), personas);
    if let Some(identity) = &caller.identity {
        input.insert(
            "profileUserId".into(),
            Value::String(identity.user_id.clone()),
        );
    }
    input.insert(
        "cfg".into(),
        json!({ "gameTo": 10, "stickTheDealer": stick, "requireNaturalTrump": natural }),
    );
    let create = inner.client.create(
        EUCHRE_TABLE,
        table_key(&game_id).to_vec(),
        CreateOptions {
            input: Some(Value::Object(input)),
            pool_name: Some(inner.config.pool_name.clone()),
            ..CreateOptions::default()
        },
    );
    match tokio::time::timeout(inner.config.timeouts.actor_call, create).await {
        Ok(Ok(_)) => {}
        Ok(Err(err)) => {
            let code = ActorCallError::from_anyhow(&err);
            tracing::warn!(game_id = %game_id, code = code.code(), "Application request failed: table create");
            return Err(ApiError::unavailable(CREATE_FAILED));
        }
        Err(_) => {
            tracing::warn!(game_id = %game_id, code = "timeout", "Application request failed: table create");
            return Err(ApiError::unavailable(CREATE_FAILED));
        }
    }

    let mut res = (StatusCode::CREATED, Json(json!({ "gameId": game_id }))).into_response();
    let headers_out = res.headers_mut();
    if let Some(cookie) = new_guest_cookie {
        headers_out.append(header::SET_COOKIE, cookie);
    }
    if ambient {
        let existing = recent_table_ids(read_cookie(&headers, RECENT_TABLES_COOKIE).as_deref());
        if let Some(cookie) = remember_table(&existing, &game_id)
            .and_then(|v| set_cookie(RECENT_TABLES_COOKIE, v, inner.config.secure_cookies))
        {
            headers_out.append(header::SET_COOKIE, cookie);
        }
    }
    Ok(no_store(res))
}

#[derive(Deserialize, Default)]
pub(crate) struct RecentQuery {
    ids: Option<String>,
}

/// `GET /api/tables/recent`: each listed table's snapshot summary (5 s bound each).
pub(crate) async fn recent_tables(
    State(state): State<ApiState>,
    Query(query): Query<RecentQuery>,
    headers: HeaderMap,
) -> Response {
    let inner = &state.inner;
    let caller = Caller::resolve(&inner.auth, &headers);
    let empty = || no_store(Json(json!({ "tables": [], "unavailable": false })));
    let Some(user_id) = caller.guest_user_id() else {
        return empty();
    };
    let Ok(token) = inner.auth.sign_guest(user_id, GuestAudience::Actors) else {
        return empty();
    };
    let ids = match query.ids.as_deref() {
        Some(ids) => recent_table_ids(Some(ids)),
        None => recent_table_ids(read_cookie(&headers, RECENT_TABLES_COOKIE).as_deref()),
    };
    let timeout = inner.config.timeouts.recent_snapshot;
    let lookups = ids.iter().map(|game_id| {
        let token = token.clone();
        async move {
            // A cookie is a bookmark, never authority: each table rechecks ownership.
            let handle = inner
                .client
                .get(
                    EUCHRE_TABLE,
                    table_key(game_id).to_vec(),
                    GetOptions {
                        params: Some(json!({ "token": token })),
                    },
                )
                .map_err(|_| ActorCallError::Other)?;
            call(&handle, SNAPSHOT, vec![], timeout).await
        }
    });
    let results = futures::future::join_all(lookups).await;
    let mut unavailable = false;
    let tables: Vec<Value> = ids
        .iter()
        .zip(results)
        .map(|(game_id, result)| match result {
            Ok(view) => json!({
                "gameId": game_id,
                "status": view.get("status").cloned().unwrap_or(Value::Null),
                "score": view.get("score").cloned().unwrap_or(Value::Null),
                "handNo": view.get("handNo").cloned().unwrap_or(Value::Null),
            }),
            Err(_) => {
                unavailable = true;
                json!({ "gameId": game_id, "status": null, "score": null, "handNo": null })
            }
        })
        .collect();
    no_store(Json(
        json!({ "tables": tables, "unavailable": unavailable }),
    ))
}

/// `POST /api/table-session/{gameId}`: the actor id, the guest actor token and (D3
/// Option A) an engine gateway token.
pub(crate) async fn table_session(
    State(state): State<ApiState>,
    ClientAddr(socket): ClientAddr,
    Origin(public_origin): Origin,
    Path(game_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    rate_limit(&state, RouteClass::TableSession, &headers, socket)?;
    let inner = &state.inner;
    let caller = Caller::resolve(&inner.auth, &headers);
    if caller.is_ambient() {
        require_same_origin(&headers, &public_origin)?;
    }
    if !is_game_id(&game_id) {
        return Err(ApiError::bad_request(INVALID_TABLE));
    }
    let Some(user_id) = caller.guest_user_id() else {
        return Err(ApiError::unauthorized(START_A_GAME_FIRST));
    };
    let token = inner
        .auth
        .sign_guest(user_id, GuestAudience::Actors)
        .map_err(|_| signing_failed())?;
    let timeout = inner.config.timeouts.actor_call;
    let handle = inner
        .client
        .get(
            EUCHRE_TABLE,
            table_key(&game_id).to_vec(),
            GetOptions {
                params: Some(json!({ "token": token })),
            },
        )
        .map_err(|_| ApiError::unavailable(TABLE_UNAVAILABLE))?;
    let unavailable = |code: &str| {
        tracing::warn!(game_id = %game_id, code = code, "[table-session] snapshot unavailable");
        ApiError::unavailable(TABLE_UNAVAILABLE)
    };
    // The snapshot runs the table's own origin and owner checks.
    match call(&handle, SNAPSHOT, vec![], timeout).await {
        Ok(_) => {}
        Err(ActorCallError::Rivet(e)) if e.code == "forbidden" => {
            return Err(ApiError::forbidden(ANOTHER_BROWSER));
        }
        Err(e) => return Err(unavailable(e.code())),
    }
    let actor_id = resolve(&handle, timeout)
        .await
        .map_err(|e| unavailable(e.code()))?;

    let (gateway_token, expires_at) = match &inner.issuer {
        Some(issuer) => match issuer.issue(user_id, &actor_id).await {
            Ok(issued) => (Some(issued.token), issued.expires_at_ms),
            Err(err) => {
                tracing::warn!(game_id = %game_id, error = %err, "[table-session] gateway token unavailable");
                return Err(ApiError::unavailable(TABLE_UNAVAILABLE));
            }
        },
        None => (
            None,
            u64::try_from(now_ms()).unwrap_or_default() + GATEWAY_TOKEN_TTL_SECS * 1000,
        ),
    };
    let mut res = Json(TableSessionResponse {
        actor_id,
        token,
        gateway_token,
        expires_at,
    })
    .into_response();
    // TS `/play/[gameId]` page load (`rememberTable`): opening a table moves it to
    // the front of the recent-tables cookie. Under D4 that page is the static SPA,
    // so the session that opens the table does it (the cookie is HttpOnly).
    if caller.is_ambient() {
        let existing = recent_table_ids(read_cookie(&headers, RECENT_TABLES_COOKIE).as_deref());
        if let Some(cookie) = remember_table(&existing, &game_id)
            .and_then(|v| set_cookie(RECENT_TABLES_COOKIE, v, inner.config.secure_cookies))
        {
            res.headers_mut().append(header::SET_COOKIE, cookie);
        }
    }
    Ok(no_store(res))
}
