//! The `playerProfile` actor (05 §3, §10.3; ARCHITECTURE §2.4).
//!
//! - **State** (05 §3.3) is one JSON row (`profile_state`, key `state`) in the
//!   actor's SQLite database, next to the 05 §3.6 tables; never `ctx.kv()`.
//! - **Writes** (`updateSettings`, `forgetEpisode`, `pruneEpisodes` and the five
//!   former queues) take the [`ActorLane`], then persist, then return: the single
//!   writer of the TS run loop. The profile emits no events.
//! - **Persist, then publish** (08 §2): a state change is made on a copy, the
//!   copy is saved, and only a saved copy becomes the live state. A failed save
//!   leaves memory as it was, so no read serves an unsaved value and a retried
//!   `recordTokens` adds its increment once.
//! - **Reads** do not take the lane, as TS actions ran outside the run loop.
//! - **Authority** is re-derived from `ctx.conn().params()` in every action (D17).
//!   No connection or no token means an internal call: only the internal-token
//!   actions accept it, and they check the token before any read.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use euchre_profile_model::{
    ProfileActorState, ProfileError, ProfileSettings, QUEUE_HANDLER_FAILED_EVENT, TokenIncrement,
    apply_settings_patch, body_internal_token, clamp_limit, forged_log_event, initial_state,
    lifetime_from_stats, list_games_args, normalize_settings, parse_hands, parse_match_record,
    parse_record_episodes, parse_record_hands, parse_set_active_game, persona_views,
    require_episode_id, require_id,
};
use euchre_server_core::auth::{Authenticator, now_ms};
use euchre_server_core::rivet::{ActorLane, JsonTable, user_error};
use rivetkit::prelude::*;
use serde_json::Value;

use crate::actions::*;
use crate::auth::{ConnAuth, assert_internal_arg, authorize_params, owner_of_key};
use crate::repository::{self as repo, RepoError, profile_error};

/// The JSON table holding the actor state.
pub const STATE_TABLE: &str = "profile_state";
/// The row key of the actor state in [`STATE_TABLE`].
pub const STATE_KEY: &str = "state";
/// Longest `err` text in a queue failure log, in UTF-16 units (TS `.slice(0, 200)`).
pub const LOG_ERR_UNITS: usize = 200;

/// Message when the actor starts before [`crate::configure`] (or
/// [`crate::register`]) installed the keys.
pub const NOT_CONFIGURED: &str =
    "playerProfile is not configured: call euchre_profile::register before serving";

/// The `playerProfile` actor: one per human, key `["user", userId]`.
pub struct PlayerProfile {
    auth: Authenticator,
    owner: String,
    lane: ActorLane,
    state: parking_lot::Mutex<ProfileActorState>,
    docs: JsonTable,
}

/// How a former queue message failed.
enum QueueFailure {
    /// The body failed validation: logged and dropped, as the TS run loop
    /// completed such a message.
    Invalid(ProfileError),
    /// SQLite failed: logged and returned, so the caller retries. A failed
    /// save published nothing, so the retry applies the write once.
    Storage(anyhow::Error),
}

impl From<anyhow::Error> for QueueFailure {
    fn from(e: anyhow::Error) -> Self {
        QueueFailure::Storage(e)
    }
}

impl From<RepoError> for QueueFailure {
    /// A stored row that fails to decode fails the same way on every retry, so
    /// it is dropped like an invalid body (TS logged and completed it).
    fn from(e: RepoError) -> Self {
        match e {
            RepoError::Data(e) => QueueFailure::Invalid(e),
            RepoError::Storage(e) => QueueFailure::Storage(e),
        }
    }
}

/// Persist, then publish (08 §2): run `edit` on a copy of `cell`, `save` the
/// copy, and only then make it the live value. When `edit` or `save` fails,
/// `cell` is left as it was. Call while holding the lane, so no other write
/// can publish between the copy and the swap.
async fn commit_with<S: Clone, R>(
    cell: &parking_lot::Mutex<S>,
    edit: impl FnOnce(&mut S) -> Result<R>,
    save: impl AsyncFnOnce(&S) -> Result<()>,
) -> Result<R> {
    let mut next = cell.lock().clone();
    let out = edit(&mut next)?;
    save(&next).await?;
    *cell.lock() = next;
    Ok(out)
}

/// TS `String(err).slice(0, 200)` over UTF-16 units.
fn log_excerpt(text: &str) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in text.chars() {
        units += c.len_utf16();
        if units > LOG_ERR_UNITS {
            break;
        }
        out.push(c);
    }
    out
}

impl PlayerProfile {
    /// The owner's user id (the key's second segment).
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Save `state` as the stored row. Call while holding the lane.
    async fn save(&self, ctx: &Ctx<Self>, state: &ProfileActorState) -> Result<()> {
        self.docs.save(ctx.sql(), STATE_KEY, state).await?;
        crate::test_support::record_state(&self.owner, state);
        Ok(())
    }

    /// [`commit_with`] on this actor's state: edit a copy, save it, publish it.
    /// Call while holding the lane.
    async fn commit<R>(
        &self,
        ctx: &Ctx<Self>,
        edit: impl FnOnce(&mut ProfileActorState) -> Result<R>,
    ) -> Result<R> {
        commit_with(&self.state, edit, async |next| self.save(ctx, next).await).await
    }

    /// D17: this call's authority, from its own connection params.
    fn conn_auth(&self, ctx: &Ctx<Self>) -> Result<ConnAuth> {
        let Some(conn) = ctx.conn() else {
            return Ok(ConnAuth::Anonymous);
        };
        let params = conn
            .params()
            .map_err(|_| user_error("bad_request", "Invalid connection params"))?;
        authorize_params(&self.auth, &self.owner, &params, now_ms()).map_err(profile_error)
    }

    /// TS `requireAuthenticated(c.conn.state)`, re-derived per call.
    fn require_owner(&self, ctx: &Ctx<Self>) -> Result<()> {
        self.conn_auth(ctx)?.require_owner().map_err(profile_error)
    }

    fn require_internal(&self, token: Option<&Value>) -> Result<()> {
        assert_internal_arg(&self.auth, token).map_err(profile_error)
    }

    /// A former queue handler: token first (`dropped forged <name>` and
    /// `internal_token_mismatch` when it fails), then the body under the lane.
    async fn queue<F, Fut>(
        self: Arc<Self>,
        name: &'static str,
        body: Option<&Value>,
        run: F,
    ) -> Result<()>
    where
        F: FnOnce(Arc<Self>, Value) -> Fut,
        Fut: std::future::Future<Output = Result<(), QueueFailure>>,
    {
        let token = body.and_then(body_internal_token);
        if let Err(e) = self.auth.assert_internal_token(token) {
            tracing::warn!("{}", forged_log_event(name));
            return Err(e.into_user_error());
        }
        let body = body.cloned().unwrap_or(Value::Null);
        let _turn = self.lane.enter().await;
        match run(self.clone(), body).await {
            Ok(()) => Ok(()),
            Err(QueueFailure::Invalid(e)) => {
                tracing::error!(
                    name,
                    err = %log_excerpt(&e.to_string()),
                    "{QUEUE_HANDLER_FAILED_EVENT}"
                );
                Ok(())
            }
            Err(QueueFailure::Storage(e)) => {
                tracing::error!(
                    name,
                    err = %log_excerpt(&format!("{e:#}")),
                    "{QUEUE_HANDLER_FAILED_EVENT}"
                );
                Err(e)
            }
        }
    }
}

#[async_trait]
impl Actor for PlayerProfile {
    type Input = Value;
    type ConnParams = Value;
    type Action = rivetkit::Raw;
    type Actions = (
        GetSettings,
        UpdateSettings,
        GetPersonas,
        GetActiveGame,
        GetStats,
        ListGames,
        GetReplay,
        ListEpisodes,
        ForgetEpisode,
        GetServerReplay,
        GetEpisodesForRole,
        PruneEpisodes,
        RecordGame,
        RecordHands,
        RecordEpisodes,
        SetActiveGame,
        RecordTokens,
    );
    type Events = ();

    /// Every wake: check the key (TS `createState` calls `ownerOf`), migrate
    /// (TS `onMigrate` runs on every start), then load the state or build a
    /// fresh one.
    async fn create(ctx: &Ctx<Self>) -> Result<Self> {
        let auth = crate::configured().context(NOT_CONFIGURED)?;
        let owner = owner_of_key(ctx.key()).map_err(profile_error)?;
        let db = ctx.sql();
        let now = now_ms();
        repo::apply_migrations(db, now).await?;
        let docs = JsonTable::new(STATE_TABLE)?;
        docs.ensure(db).await?;
        let state = match docs.load::<Value>(db, STATE_KEY).await? {
            Some(raw) => ProfileActorState::from_stored(&raw, now),
            None => initial_state(now),
        };
        crate::test_support::record_start(&owner);
        Ok(Self {
            auth,
            owner,
            lane: ActorLane::new(),
            state: parking_lot::Mutex::new(state),
            docs,
        })
    }

    /// Every wake: repair the settings (TS `run` start; `normalizeSettings` is
    /// idempotent) and persist, before any write can run.
    async fn on_start(self: Arc<Self>, ctx: Ctx<Self>) -> Result<()> {
        let _turn = self.lane.enter().await;
        self.commit(&ctx, |state| {
            let raw = serde_json::to_value(&state.settings)?;
            state.settings = normalize_settings(&raw, now_ms());
            Ok(())
        })
        .await
    }

    /// 05 §3.2: refuse a bad protocol version, a bad token or another user's
    /// token. A token-less connection is allowed (internal calls).
    async fn on_before_connect(self: Arc<Self>, _ctx: Ctx<Self>, params: &Value) -> Result<()> {
        authorize_params(&self.auth, &self.owner, params, now_ms())
            .map(|_| ())
            .map_err(profile_error)
    }

    async fn on_sleep(self: Arc<Self>, _ctx: Ctx<Self>) -> Result<()> {
        crate::test_support::record_sleep(&self.owner);
        Ok(())
    }
}

// ------------------------------------------------------------ browser actions

impl Handler<GetSettings> for PlayerProfile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, _a: GetSettings) -> Result<ProfileSettings> {
        self.require_owner(&ctx)?;
        Ok(self.state.lock().settings.clone())
    }
}

impl Handler<UpdateSettings> for PlayerProfile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: UpdateSettings) -> Result<ProfileSettings> {
        self.require_owner(&ctx)?;
        let patch = a.0.into_iter().next().unwrap_or(Value::Null);
        let _turn = self.lane.enter().await;
        self.commit(&ctx, |state| {
            let next =
                apply_settings_patch(&state.settings, &patch, now_ms()).map_err(profile_error)?;
            state.settings = next.clone();
            Ok(next)
        })
        .await
    }
}

impl Handler<GetPersonas> for PlayerProfile {
    async fn handle(
        self: Arc<Self>,
        ctx: Ctx<Self>,
        _a: GetPersonas,
    ) -> Result<Vec<euchre_protocol::PersonaView>> {
        self.require_owner(&ctx)?;
        Ok(persona_views(&self.state.lock().settings))
    }
}

impl Handler<GetActiveGame> for PlayerProfile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, _a: GetActiveGame) -> Result<Option<String>> {
        self.require_owner(&ctx)?;
        Ok(self.state.lock().active_game_id.clone())
    }
}

impl Handler<GetStats> for PlayerProfile {
    async fn handle(
        self: Arc<Self>,
        ctx: Ctx<Self>,
        _a: GetStats,
    ) -> Result<euchre_profile_model::ProfileStats> {
        self.require_owner(&ctx)?;
        Ok(repo::compute_stats(ctx.sql(), now_ms()).await?)
    }
}

impl Handler<ListGames> for PlayerProfile {
    async fn handle(
        self: Arc<Self>,
        ctx: Ctx<Self>,
        a: ListGames,
    ) -> Result<euchre_protocol::MatchListPage> {
        self.require_owner(&ctx)?;
        let (limit, cursor) = list_games_args(a.arg(0));
        repo::list_matches(ctx.sql(), limit, cursor).await
    }
}

impl Handler<GetReplay> for PlayerProfile {
    async fn handle(
        self: Arc<Self>,
        ctx: Ctx<Self>,
        a: GetReplay,
    ) -> Result<Option<euchre_profile_model::ClientMatchReplay>> {
        self.require_owner(&ctx)?;
        let match_id = require_id(a.0.first(), "matchId").map_err(profile_error)?;
        repo::read_client_replay(ctx.sql(), &match_id).await
    }
}

impl Handler<ListEpisodes> for PlayerProfile {
    async fn handle(
        self: Arc<Self>,
        ctx: Ctx<Self>,
        a: ListEpisodes,
    ) -> Result<Vec<euchre_profile_model::StoredEpisode>> {
        self.require_owner(&ctx)?;
        repo::list_episodes(ctx.sql(), clamp_limit(a.arg(0)), now_ms()).await
    }
}

impl Handler<ForgetEpisode> for PlayerProfile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: ForgetEpisode) -> Result<bool> {
        self.require_owner(&ctx)?;
        // D30: stored ids are `role.matchId.handNo.kind`; TS `requireId` refused them all.
        let episode_id = require_episode_id(a.0.first()).map_err(profile_error)?;
        let _turn = self.lane.enter().await;
        repo::delete_episode(ctx.sql(), &episode_id).await
    }
}

// ------------------------------------------------------------ internal reads

impl Handler<GetServerReplay> for PlayerProfile {
    async fn handle(
        self: Arc<Self>,
        ctx: Ctx<Self>,
        a: GetServerReplay,
    ) -> Result<Option<euchre_profile_model::ServerMatchReplay>> {
        self.require_internal(a.0.first())?;
        let match_id = require_id(a.0.get(1), "matchId").map_err(profile_error)?;
        repo::read_server_replay(ctx.sql(), &match_id).await
    }
}

impl Handler<GetEpisodesForRole> for PlayerProfile {
    async fn handle(
        self: Arc<Self>,
        ctx: Ctx<Self>,
        a: GetEpisodesForRole,
    ) -> Result<Vec<euchre_profile_model::StoredEpisode>> {
        self.require_internal(a.0.first())?;
        let role = require_id(a.0.get(1), "role").map_err(profile_error)?;
        let limit = clamp_limit(a.arg(2));
        repo::select_episodes_for_role(ctx.sql(), &role, limit, now_ms()).await
    }
}

impl Handler<PruneEpisodes> for PlayerProfile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: PruneEpisodes) -> Result<()> {
        self.require_internal(a.0.first())?;
        let _turn = self.lane.enter().await;
        repo::prune_expired_episodes(ctx.sql(), now_ms()).await
    }
}

// ------------------------------------------------------------ former queues

impl Handler<RecordGame> for PlayerProfile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: RecordGame) -> Result<()> {
        self.queue(RecordGame::NAME, a.body(), |me, body| async move {
            let now = now_ms();
            let b = Some(&body);
            let record =
                parse_match_record(euchre_profile_model::js::pick(b, "record"), &me.owner, now)
                    .map_err(QueueFailure::Invalid)?;
            let db = ctx.sql();
            repo::upsert_match(db, &record, now).await?;
            if let Some(raw) = body.as_object().and_then(|m| m.get("hands")) {
                let hands =
                    parse_hands(Some(raw), &record.match_id).map_err(QueueFailure::Invalid)?;
                repo::insert_hands(db, &hands).await?;
            }
            let stats = repo::compute_stats(db, now).await?;
            let lifetime = lifetime_from_stats(&stats);
            me.commit(&ctx, |state| {
                state.lifetime = lifetime;
                Ok(())
            })
            .await?;
            Ok(())
        })
        .await
    }
}

impl Handler<RecordHands> for PlayerProfile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: RecordHands) -> Result<()> {
        self.queue(RecordHands::NAME, a.body(), |_me, body| async move {
            let hands = parse_record_hands(&body).map_err(QueueFailure::Invalid)?;
            repo::insert_hands(ctx.sql(), &hands).await?;
            Ok(())
        })
        .await
    }
}

impl Handler<RecordEpisodes> for PlayerProfile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: RecordEpisodes) -> Result<()> {
        self.queue(RecordEpisodes::NAME, a.body(), |_me, body| async move {
            let episodes = parse_record_episodes(&body, now_ms()).map_err(QueueFailure::Invalid)?;
            repo::insert_episodes(ctx.sql(), &episodes).await?;
            Ok(())
        })
        .await
    }
}

impl Handler<SetActiveGame> for PlayerProfile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: SetActiveGame) -> Result<()> {
        self.queue(SetActiveGame::NAME, a.body(), |me, body| async move {
            let active = parse_set_active_game(&body);
            me.commit(&ctx, |state| {
                state.active_game_id = active;
                Ok(())
            })
            .await?;
            Ok(())
        })
        .await
    }
}

impl Handler<RecordTokens> for PlayerProfile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: RecordTokens) -> Result<()> {
        self.queue(RecordTokens::NAME, a.body(), |me, body| async move {
            let increment = TokenIncrement::from_body(&body);
            let now = now_ms();
            me.commit(&ctx, |state| {
                state.daily_tokens.record(increment, now);
                Ok(())
            })
            .await?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use euchre_profile_model::initial_state;
    use serde_json::json;

    #[tokio::test]
    async fn failed_save_leaves_state_and_retry_counts_tokens_once() {
        let now = 1_700_000_000_000;
        let cell = parking_lot::Mutex::new(initial_state(now));
        let before = cell.lock().clone();
        let inc = TokenIncrement::from_body(&json!({ "tokensIn": 10, "tokensOut": 4, "calls": 1 }));
        // The same edit the recordTokens handler commits.
        let edit = |s: &mut ProfileActorState| -> Result<()> {
            s.daily_tokens.record(inc, now);
            Ok(())
        };

        // The save fails: nothing is published.
        let failed = commit_with(&cell, edit, async |_| Err(anyhow::anyhow!("disk full"))).await;
        assert!(failed.is_err());
        assert_eq!(*cell.lock(), before);

        // The caller retries and the save succeeds: one increment, not two.
        let mut saved = None;
        commit_with(&cell, edit, async |s| {
            saved = Some(s.clone());
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())
        .unwrap_or_else(|e| panic!("{e}"));
        let after = cell.lock().clone();
        assert_eq!(after.daily_tokens.tokens_in, 10);
        assert_eq!(after.daily_tokens.tokens_out, 4);
        assert_eq!(after.daily_tokens.calls, 1);
        assert_eq!(saved.as_ref(), Some(&after));
    }

    #[tokio::test]
    async fn failed_edit_publishes_nothing() {
        let now = 1_700_000_000_000;
        let cell = parking_lot::Mutex::new(initial_state(now));
        let before = cell.lock().clone();
        let mut saves = 0;
        let r: Result<()> = commit_with(
            &cell,
            |s| {
                s.active_game_id = Some("g".into());
                Err(anyhow::anyhow!("bad patch"))
            },
            async |_| {
                saves += 1;
                Ok(())
            },
        )
        .await;
        assert!(r.is_err());
        assert_eq!(saves, 0);
        assert_eq!(*cell.lock(), before);
    }

    #[test]
    fn log_excerpt_cuts_at_200_utf16_units() {
        assert_eq!(log_excerpt("abc"), "abc");
        let long = "x".repeat(250);
        assert_eq!(log_excerpt(&long).len(), 200);
        // A surrogate pair straddling unit 200 is dropped whole.
        let s = format!("{}\u{1F0A1}", "y".repeat(199));
        assert_eq!(log_excerpt(&s), "y".repeat(199));
    }
}
