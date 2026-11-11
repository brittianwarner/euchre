//! The browser's server backend (web plan S4, lane W2): [`WebBackend`] speaks to the server the
//! page came from.
//!
//! - HTTP is same-origin `fetch` with the browser's own cookies ([`http`]); the guest cookie is
//!   HttpOnly and never visible here, so there is no device record, no bearer, and no
//!   `recent-tables.json` (the server reads the `euchre_recent_tables` cookie).
//! - A table is the official rivetkit JS client behind `js/rivet-bridge.js` ([`table`]).
//! - [`local`] keeps `!Send` JS values behind `Send` handles; [`crate::net::rt`] owns timers.
//!
//! Compiles on wasm32 only.

pub mod http;
pub mod local;
pub mod table;

use async_trait::async_trait;
use euchre_profile_model::{ClientMatchReplay, ProfileSettings, ProfileStats, SettingsPatch};
use euchre_protocol::{MatchListPage, is_game_id};
use serde::Deserialize;
use wasm_bindgen::JsCast;

use self::http::{Call, GAMES_PAGE_LIMIT, PROFILE_BROWSER, call, enc};
use crate::net::{Backend, Identity, NetError, RecentTables, TableConfig, TableConn};

/// The server this page came from.
#[derive(Clone, Debug)]
pub struct WebBackend {
    origin: String,
}

/// `GET /api/session`: `{guestUserId, identity}`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionInfo {
    guest_user_id: Option<String>,
    identity: Option<EmailIdentity>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmailIdentity {
    email: String,
    user_id: String,
}

impl WebBackend {
    /// A backend for the page's own origin (`location.origin`).
    pub fn from_page() -> WebBackend {
        let global: web_sys::Window = js_sys::global().unchecked_into();
        WebBackend {
            origin: global.location().origin().unwrap_or_default(),
        }
    }

    /// A backend for an explicit origin (`https://host[:port]`, no trailing slash). HTTP stays
    /// relative to the page; only the rivet gateway (`<origin>/rivet`) and the table-session
    /// mint use it, so it must be the page's origin for the cookies to ride.
    pub fn with_origin(origin: &str) -> WebBackend {
        WebBackend {
            origin: origin.trim_end_matches('/').to_owned(),
        }
    }

    /// The origin the bridge dials.
    pub fn origin(&self) -> &str {
        &self.origin
    }
}

#[async_trait]
impl Backend for WebBackend {
    fn name(&self) -> &'static str {
        "web"
    }

    async fn identity(&self) -> Result<Identity, NetError> {
        let s: SessionInfo = call(Call::Session, "GET", "/api/session".to_owned(), None).await?;
        match (s.identity, s.guest_user_id) {
            (Some(id), _) => Ok(Identity {
                user_id: id.user_id,
                display_name: Some(id.email),
            }),
            (None, Some(guest)) => Ok(Identity {
                user_id: guest,
                display_name: None,
            }),
            (None, None) => Err(NetError::Unauthorized),
        }
    }

    async fn create_table(&self, cfg: TableConfig) -> Result<String, NetError> {
        let body = serde_json::json!({
            "stick": cfg.stick_the_dealer,
            "natural": cfg.require_natural_trump,
        })
        .to_string();
        http::create_table(body).await
    }

    async fn open_table(&self, game_id: &str) -> Result<TableConn, NetError> {
        if !is_game_id(game_id) {
            return Err(NetError::NotFound);
        }
        table::open(self.origin.clone(), game_id.to_owned()).await
    }

    async fn list_games(&self, cursor: Option<String>) -> Result<MatchListPage, NetError> {
        let mut path = format!("/api/games?{PROFILE_BROWSER}&limit={GAMES_PAGE_LIMIT}");
        if let Some(c) = cursor.filter(|c| !c.is_empty()) {
            path.push_str(&format!("&cursor={}", enc(&c)));
        }
        call(Call::ListGames, "GET", path, None).await
    }

    async fn match_detail(&self, match_id: &str) -> Result<ClientMatchReplay, NetError> {
        let path = format!("/api/games/{}?{PROFILE_BROWSER}", enc(match_id));
        call(Call::MatchDetail, "GET", path, None).await
    }

    async fn stats(&self) -> Result<ProfileStats, NetError> {
        let path = format!("/api/profile/stats?{PROFILE_BROWSER}");
        call(Call::Stats, "GET", path, None).await
    }

    async fn get_settings(&self) -> Result<ProfileSettings, NetError> {
        let path = format!("/api/profile/settings?{PROFILE_BROWSER}");
        call(Call::GetSettings, "GET", path, None).await
    }

    async fn patch_settings(&self, patch: SettingsPatch) -> Result<ProfileSettings, NetError> {
        let body = serde_json::to_string(&patch).map_err(|e| NetError::Other(e.to_string()))?;
        let path = format!("/api/profile/settings?{PROFILE_BROWSER}");
        call(Call::PatchSettings, "PATCH", path, Some(body)).await
    }

    async fn recent_tables(&self) -> Result<RecentTables, NetError> {
        // No `ids`: the server reads the `euchre_recent_tables` cookie, as the Svelte site does.
        call(
            Call::RecentTables,
            "GET",
            "/api/tables/recent".to_owned(),
            None,
        )
        .await
    }
}
