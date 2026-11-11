//! The `euchreTable` actor (03; ARCHITECTURE §2.3).
//!
//! - **State** is one JSON row (`table_state`, key `state`) plus one row per
//!   archived hand (`hand_review`, key = hand number) in the actor's SQLite
//!   database, never `ctx.kv()`. Both are written in one batch, so a hand and
//!   its review land together. `create` reloads the row on every wake.
//! - **One writer.** Every mutating path (actions, former queues, schedule
//!   fires, the owner claim, `on_start`) runs a [`Session`] under the
//!   [`ActorLane`]. A session edits a copy; only a saved copy becomes the live
//!   state (persist, then emit, then ack).
//! - **Authority** is checked in every action (D17, amended): a live
//!   WebSocket keeps the identity it proved at connect (in memory, keyed by
//!   conn id, never persisted); a stateless HTTP action is verified in full
//!   from `ctx.conn().params()`; no connection means a schedule fire.
//! - **Timers** are `ctx.schedule()` one-shots with their due times in the
//!   state's ledger. `on_start` re-arms them (D20); a failed fire is logged and
//!   retried, never returned as an error. A turn that fails after its first
//!   save is recovered by a fresh reconcile, since a retry of its trigger is
//!   stale by then.
//! - **Other actors** are called off the lane through ordered outboxes
//!   ([`crate::remote`]); a `recordGame` outcome comes back in its own lane turn.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use euchre_engine::{HUMAN_SEAT, Seat, Trick};
use euchre_protocol::{
    AIDecision, AiSayMsg, CompletedHandReview, ErrorCode, MoveAck, MoveMsg, PresenceEvent,
    ReviewMsg, ReviewTrickAck, TableCreateInput, TickKind, TickMsg, True, hand_review_number,
    wire::view_to_value,
};
use euchre_server_core::auth::internal::new_match_token;
use euchre_server_core::auth::now_ms;
use euchre_server_core::rivet::{
    ActorLane, JsonTable, require_scheduled_fire, schedule_after, user_error,
};
use rivetkit::KeepAwakeRegion;
use rivetkit::prelude::*;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::TableConfig;
use crate::actions::*;
use crate::auth::{self as table_auth, Caller, Identity};
use crate::core::{
    Host, Session, err_excerpt, internal_view, move_persisted, queue_move_body, review_body,
    rule_error, sync_payload,
};
use crate::remote::{self, Job, Outbox, PROFILE_LANE, seat_lane};
use crate::seams::{
    AiDispatch, DecideCall, HeuristicSeat, LifecycleCall, NoProfile, ProfileSink, RecordGameCall,
    RecordHandsCall,
};
use crate::state::TableState;
use crate::tempo::Tempo;
use crate::text::random_hex;

/// The JSON table holding the actor state.
pub const STATE_TABLE: &str = "table_state";
/// The row key of the actor state in [`STATE_TABLE`].
pub const STATE_KEY: &str = "state";
/// The JSON table holding archived hands (`CompletedHandReview`), keyed by hand number.
pub const REVIEW_TABLE: &str = "hand_review";
/// Message when the actor starts before [`crate::configure`] ran.
pub const NOT_CONFIGURED: &str =
    "euchreTable is not configured: call euchre_table::register before serving";
/// How long a failed schedule fire waits before it runs again (D20).
pub const FIRE_RETRY_DELAY: Duration = Duration::from_secs(1);
/// How many times in a row one schedule target is retried before it gives up.
pub const FIRE_RETRY_LIMIT: u32 = 5;
/// Log event of a failed schedule fire.
pub const FIRE_FAILED_EVENT: &str = "euchreTable schedule target failed";
/// Log event when a turn failed after its first save and a reconcile recovers it.
pub const RECOVER_EVENT: &str = "euchreTable recovering a partial commit";
/// How long a new socket may take to be registered before it is forgotten.
pub const CONN_REGISTER_WAIT: Duration = Duration::from_secs(10);
/// How often the registration wait looks for the socket.
const CONN_REGISTER_POLL: Duration = Duration::from_millis(5);

/// One live WebSocket connection, from `on_connect` to `on_disconnect`.
#[derive(Clone, Debug)]
struct ConnEntry {
    /// The identity it proved at connect (D17, amended).
    identity: Identity,
    /// Registered with rivetkit and counted in presence (player sockets only).
    live: bool,
}

impl ConnEntry {
    fn live_player(&self) -> bool {
        self.live && matches!(self.identity, Identity::Player(_))
    }
}

/// Whether the profile seam goes through the outbox: whenever a profile token
/// is configured, bound client or not. An unbound client fails the send, so
/// the record waits for `flushProfile` instead of being marked recorded.
fn profile_via_outbox(config: &TableConfig) -> bool {
    config.profile_token.is_some()
}

/// The `euchreTable` actor: one per match, key `["table", gameId]`.
pub struct EuchreTable {
    config: TableConfig,
    tempo: Tempo,
    lane: ActorLane,
    live: parking_lot::Mutex<TableState>,
    /// WebSocket connections (never stateless HTTP calls): conn id → entry.
    conns: parking_lot::Mutex<HashMap<String, ConnEntry>>,
    state_table: JsonTable,
    review_table: JsonTable,
    retries: parking_lot::Mutex<HashMap<&'static str, u32>>,
    outbox: Outbox,
    /// A recovery reconcile is waiting or running.
    recovering: AtomicBool,
    /// Held while a match record waits for `flushProfile`, so an idle sleep
    /// cannot drop the retry in its grace (rivetkit suspends alarms there).
    profile_hold: parking_lot::Mutex<Option<KeepAwakeRegion>>,
    /// Set in `on_sleep`: no new hold may delay the shutdown.
    sleeping: AtomicBool,
}

/// The actor side of [`Host`]: a borrowed context plus the table, and the
/// client-backed seams when a client is bound.
struct ActorHost<'a> {
    ctx: &'a Ctx<EuchreTable>,
    table: &'a Arc<EuchreTable>,
    remote: Remote<'a>,
    /// A client is bound: AI seats are asked.
    bound: bool,
}

/// The seams over the shared client: every call is a job on an ordered outbox.
struct Remote<'a> {
    ctx: &'a Ctx<EuchreTable>,
    table: &'a Arc<EuchreTable>,
}

impl Remote<'_> {
    fn push(&self, lane: usize, job: Job) {
        if self.table.outbox.push(lane, job) {
            self.table.spawn_drain(self.ctx, lane);
        }
    }
}

impl AiDispatch for Remote<'_> {
    fn asks(&self) -> bool {
        true
    }

    fn decide(&self, call: DecideCall) {
        self.push(seat_lane(call.seat), Job::Decide(Box::new(call)));
    }

    fn notify(&self, call: LifecycleCall) {
        self.push(seat_lane(call.seat), Job::Notify(call));
    }
}

impl ProfileSink for Remote<'_> {
    fn available(&self) -> bool {
        self.table.config.profile_token.is_some()
    }

    fn record_hands(&self, call: RecordHandsCall) {
        self.push(PROFILE_LANE, Job::RecordHands(call));
    }

    fn record_game(&self, call: RecordGameCall) {
        // One in flight: its outcome settles the match (the retry is armed).
        if self.table.outbox.begin_record() {
            self.push(PROFILE_LANE, Job::RecordGame(call));
        }
    }
}

fn epoch_ms() -> u64 {
    u64::try_from(now_ms()).unwrap_or(0)
}

#[async_trait]
impl Host for ActorHost<'_> {
    fn now_ms(&self) -> u64 {
        epoch_ms()
    }

    async fn schedule(
        &self,
        delay_ms: u64,
        action: &'static str,
        args: Vec<Value>,
    ) -> Result<String> {
        schedule_after(self.ctx, Duration::from_millis(delay_ms), action, &args).await
    }

    async fn cancel(&self, id: &str) {
        if let Err(e) = self.ctx.schedule().cancel(id).await {
            tracing::warn!(err = %err_excerpt(&e), "schedule cancel failed");
        }
    }

    async fn persist(&self, state: &TableState, reviews: &[CompletedHandReview]) -> Result<()> {
        let t = self.table;
        let mut batch = vec![t.state_table.save_statement(STATE_KEY, state)?];
        for review in reviews {
            batch.push(
                t.review_table
                    .save_statement(&review.hand_no.to_string(), review)?,
            );
        }
        self.ctx.sql().execute_batch(batch).await?;
        *t.live.lock() = state.clone();
        t.hold_for_profile(self.ctx, state.ledger.profile_retry_at.is_some());
        crate::test_support::record_state(&state.match_id, state);
        Ok(())
    }

    fn broadcast(&self, name: &str, payload: Value) {
        if let Err(e) = self.ctx.broadcast(name, &payload) {
            tracing::warn!(name, err = %err_excerpt(&e), "broadcast failed");
        }
    }

    fn send_sync(&self, build: &(dyn Fn(Seat) -> Option<Value> + Sync)) {
        if !self.table.player_connected() {
            return;
        }
        let players: Vec<String> = self
            .table
            .conns
            .lock()
            .iter()
            .filter(|(_, e)| e.live_player())
            .map(|(id, _)| id.clone())
            .collect();
        for conn in self.ctx.conns_vec() {
            if !players.iter().any(|id| id == conn.id()) {
                continue;
            }
            if let Some(payload) = build(HUMAN_SEAT)
                && let Err(e) = conn.send("sync", &payload)
            {
                tracing::warn!(err = %err_excerpt(&e), "sync send failed");
            }
        }
    }

    fn player_connected(&self) -> bool {
        self.table.player_connected()
    }

    fn destroy(&self) {
        if let Err(e) = self.ctx.destroy() {
            tracing::warn!(err = %err_excerpt(&e), "destroy failed");
        }
    }

    fn ai(&self) -> &dyn AiDispatch {
        if self.bound {
            &self.remote
        } else {
            &HeuristicSeat
        }
    }

    fn profile(&self) -> &dyn ProfileSink {
        if profile_via_outbox(&self.table.config) {
            &self.remote
        } else {
            &NoProfile
        }
    }
}

/// A stateless HTTP action call: rivetkit opens a connection for it, but it
/// is not a live player and cannot receive events.
fn is_http_action(ctx: &Ctx<EuchreTable>) -> bool {
    ctx.request()
        .is_some_and(|r| r.uri().path().contains("/action/"))
}

fn origin_header(ctx: &Ctx<EuchreTable>) -> Option<String> {
    ctx.request()
        .and_then(|r| r.headers().get("origin"))
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn decode_body<T: DeserializeOwned>(name: &str, body: Value) -> Result<T> {
    serde_json::from_value(body).map_err(|_| {
        user_error(
            euchre_protocol::ProtocolErrorCode::InternalError.as_str(),
            format!("Invalid {name} request"),
        )
    })
}

impl EuchreTable {
    fn host<'a>(self: &'a Arc<Self>, ctx: &'a Ctx<Self>) -> ActorHost<'a> {
        ActorHost {
            ctx,
            table: self,
            remote: Remote { ctx, table: self },
            bound: self.config.client.get().is_some(),
        }
    }

    /// Whether any player socket is registered and live.
    fn player_connected(&self) -> bool {
        self.conns.lock().values().any(ConnEntry::live_player)
    }

    /// Hold the actor awake while a match record waits for its retry, and let
    /// it go once recorded (or once sleep has begun).
    fn hold_for_profile(&self, ctx: &Ctx<Self>, waiting: bool) {
        let mut hold = self.profile_hold.lock();
        if waiting && hold.is_none() && !self.sleeping.load(Ordering::SeqCst) {
            *hold = Some(ctx.keep_awake_region());
        } else if !waiting {
            hold.take();
        }
    }

    /// A turn failed after it persisted part of its work: the live state has
    /// moved past its trigger, so re-derive the next turn, tick or retry from
    /// the persisted state with a reconcile in a fresh lane turn (D20), with
    /// backoff. One recovery at a time; a wake reconciles anyway.
    fn spawn_recover(self: &Arc<Self>, ctx: &Ctx<Self>, name: &'static str) {
        if self.recovering.swap(true, Ordering::SeqCst) {
            return;
        }
        tracing::warn!(name, "{RECOVER_EVENT}");
        let table = Arc::clone(self);
        let task_ctx = ctx.clone();
        let awake = ctx.keep_awake_region();
        ctx.register_task(async move {
            let _awake = awake;
            let abort = task_ctx.abort_signal();
            for attempt in 1..=FIRE_RETRY_LIMIT {
                tokio::select! {
                    () = abort.cancelled() => break,
                    () = tokio::time::sleep(FIRE_RETRY_DELAY * attempt) => {}
                }
                match table.reconcile(&task_ctx).await {
                    Ok(()) => break,
                    Err(e) => tracing::error!(
                        name = "reconcile",
                        attempt,
                        err = %err_excerpt(&e),
                        "{FIRE_FAILED_EVENT}"
                    ),
                }
            }
            table.recovering.store(false, Ordering::SeqCst);
        });
    }

    /// The end of a lane turn that returned `Err`: drop its unsaved edits and,
    /// when it had already saved something, recover with a reconcile.
    fn after_failure(
        self: &Arc<Self>,
        ctx: &Ctx<Self>,
        name: &'static str,
        session: &mut Session<'_, ActorHost<'_>>,
    ) {
        session.rollback();
        if session.committed() {
            self.spawn_recover(ctx, name);
        }
    }

    /// Start the drainer of an idle outbox: one detached task, held awake
    /// (G25), that sends the lane's jobs in order until it is empty.
    fn spawn_drain(self: &Arc<Self>, ctx: &Ctx<Self>, lane: usize) {
        let table = Arc::clone(self);
        let task_ctx = ctx.clone();
        let awake = ctx.keep_awake_region();
        ctx.register_task(async move {
            let _awake = awake;
            let abort = task_ctx.abort_signal();
            let (table, task_ctx, abort) = (&table, &task_ctx, &abort);
            remote::drain(&table.outbox, lane, |job| async move {
                table.run_job(task_ctx, abort, job).await;
            })
            .await;
        });
    }

    /// Send one job and log its failure as TS did. A `recordGame` outcome
    /// goes back through the lane.
    async fn run_job(
        self: &Arc<Self>,
        ctx: &Ctx<Self>,
        abort: &tokio_util::sync::CancellationToken,
        job: Job,
    ) {
        let out = match self.config.client.get() {
            Some(client) => {
                let token = self.config.profile_token.as_ref();
                remote::deliver(&client, &job, token, abort, epoch_ms()).await
            }
            None => Err(anyhow::anyhow!("euchreTable client is not bound")),
        };
        match job {
            Job::Decide(call) => {
                if let Err(e) = out {
                    tracing::warn!(
                        seat = call.seat.get(),
                        e = %err_excerpt(&e),
                        "dispatch to aiSeat failed; heuristic stands"
                    );
                }
                if crate::test_support::take_sleep_after_dispatch(&call.request.game_id) {
                    // Tests only: sleep between the dispatch and the decision.
                    if let Err(e) = ctx.sleep() {
                        tracing::warn!(err = %err_excerpt(&e), "test sleep failed");
                    }
                }
            }
            Job::Notify(call) => {
                if let Err(e) = out {
                    tracing::warn!(
                        seat = call.seat.get(),
                        kind = call.message.kind.as_str(),
                        e = %err_excerpt(&e),
                        "notifyAiSeats failed"
                    );
                }
            }
            Job::RecordHands(call) => {
                if let Err(e) = out {
                    tracing::warn!(
                        hand_no = call.entry.hand_no,
                        err = %err_excerpt(&e),
                        "recordHand failed; hand journal entry dropped"
                    );
                }
            }
            Job::RecordGame(_) => {
                self.settle_record(ctx, out.map_err(|e| format!("{e:#}")))
                    .await;
                self.outbox.end_record();
            }
        }
    }

    /// The `recordGame` outcome, in its own lane turn (03 §16.7).
    async fn settle_record(self: &Arc<Self>, ctx: &Ctx<Self>, outcome: Result<(), String>) {
        let _turn = self.lane.enter().await;
        let host = self.host(ctx);
        let mut session = self.session(&host);
        let out = session.on_record_game(outcome).await;
        if let Err(e) = self.close_internal("recordGame", &mut session, out).await {
            tracing::error!(name = "recordGame", err = %err_excerpt(&e), "{FIRE_FAILED_EVENT}");
            self.after_failure(ctx, "recordGame", &mut session);
        }
    }

    fn session<'a>(&self, host: &'a ActorHost<'a>) -> Session<'a, ActorHost<'a>> {
        Session::new(host, self.live.lock().clone(), self.tempo)
    }

    fn match_token(&self) -> String {
        self.live.lock().game.internal_token.clone()
    }

    /// D17 (amended): this call's caller. A live WebSocket uses the identity
    /// it proved at connect, so a guest token that expires mid-match does not
    /// lock the player out of their socket (TS kept it in conn state); any
    /// other connection (a stateless HTTP action) is verified in full.
    fn caller(&self, ctx: &Ctx<Self>) -> Result<Caller> {
        let (token, owner) = {
            let st = self.live.lock();
            (st.game.internal_token.clone(), st.owner_user_id.clone())
        };
        let Some(conn) = ctx.conn() else {
            return Ok(Caller::Scheduled);
        };
        let kept = self.conns.lock().get(conn.id()).map(|e| e.identity.clone());
        if let Some(identity) = kept {
            return table_auth::authorize(identity, &owner);
        }
        let params = conn.params().unwrap_or(Value::Null);
        table_auth::caller(
            &self.config.auth,
            &token,
            &owner,
            Some(&params),
            now_ms() / 1000,
        )
    }

    /// Close an internal path: a `RuleError` is a dispatch bug, logged and
    /// absorbed (TS completed the message); the working copy is then saved.
    /// Any other error discards the unsaved working copy.
    async fn close_internal(
        &self,
        name: &'static str,
        session: &mut Session<'_, ActorHost<'_>>,
        out: Result<()>,
    ) -> Result<()> {
        match out {
            Ok(()) => session.flush().await,
            Err(e) => match rule_error(&e) {
                Some(rule) => {
                    tracing::error!(
                        name,
                        code = rule.code.as_str(),
                        "internal path raised a RuleError"
                    );
                    session.flush().await
                }
                None => Err(e),
            },
        }
    }

    /// A schedule target (03 §16.5): refuse client calls, run the fire under
    /// the lane, and never return a transient error (D20): log it and re-arm
    /// the same fire after [`FIRE_RETRY_DELAY`], up to [`FIRE_RETRY_LIMIT`] times.
    async fn fire_target(
        self: Arc<Self>,
        ctx: Ctx<Self>,
        action: &'static str,
        kind: TickKind,
        args: Vec<Value>,
    ) -> Result<()> {
        require_scheduled_fire(&ctx)?;
        let turn_id = match kind {
            TickKind::FlushProfile | TickKind::Reap => None,
            _ => match args.first().and_then(Value::as_str) {
                Some(t) => Some(t.to_owned()),
                // TS `c.state.game.turnId !== turnId`: no turn is a stale fire.
                None => return Ok(()),
            },
        };
        let seat = match args.get(1).filter(|v| !v.is_null()) {
            None => None,
            Some(v) => match serde_json::from_value::<Seat>(v.clone()) {
                Ok(s) => Some(s),
                Err(_) => return Ok(()),
            },
        };
        let (result, committed) = {
            let _turn = self.lane.enter().await;
            let host = self.host(&ctx);
            let mut session = self.session(&host);
            let out = session.fire(kind, turn_id, seat).await;
            let result = self.close_internal(action, &mut session, out).await;
            if result.is_err() {
                self.after_failure(&ctx, action, &mut session);
            }
            (result, session.committed())
        };
        match result {
            Ok(()) => {
                self.retries.lock().remove(action);
            }
            Err(e) if committed => {
                // The recovery reconcile re-derives what follows; a retry of
                // this fire would be stale.
                self.retries.lock().remove(action);
                tracing::error!(name = action, err = %err_excerpt(&e), "{FIRE_FAILED_EVENT}");
            }
            Err(e) => {
                let attempt = {
                    let mut retries = self.retries.lock();
                    let n = retries.entry(action).or_insert(0);
                    *n += 1;
                    *n
                };
                let retry = attempt <= FIRE_RETRY_LIMIT;
                tracing::error!(
                    name = action,
                    attempt,
                    retry,
                    err = %err_excerpt(&e),
                    "{FIRE_FAILED_EVENT}"
                );
                if retry && let Err(e) = schedule_after(&ctx, FIRE_RETRY_DELAY, action, &args).await
                {
                    tracing::error!(name = action, err = %err_excerpt(&e), "{FIRE_FAILED_EVENT}");
                }
            }
        }
        Ok(())
    }

    /// `on_start`: re-arm and reconcile under the lane.
    async fn reconcile(self: &Arc<Self>, ctx: &Ctx<Self>) -> Result<()> {
        let _turn = self.lane.enter().await;
        let host = self.host(ctx);
        let mut session = self.session(&host);
        let out = session.reconcile().await;
        let out = self.close_internal("reconcile", &mut session, out).await;
        if out.is_err() {
            session.rollback();
        }
        out
    }

    /// The owner claim of TS `createConnState`, under the lane.
    async fn claim(self: &Arc<Self>, ctx: &Ctx<Self>, user_id: &str) -> Result<()> {
        let _turn = self.lane.enter().await;
        let owner = self.live.lock().owner_user_id.clone();
        if table_auth::owner_claim(&owner, user_id)? {
            let host = self.host(ctx);
            let mut session = self.session(&host);
            session.st.owner_user_id = user_id.to_owned();
            session.save().await?;
        }
        Ok(())
    }

    /// The second half of TS `onConnect`, once rivetkit has registered the
    /// socket: `on_connect` runs before that, so a `sync` sent from it can be
    /// overtaken by a commit that skips the unregistered socket, and a connect
    /// aborted after it would leave a phantom in presence. Under the lane, the
    /// socket goes live, gets `sync` from the current state with `steps: []`
    /// (snap, do not animate) and, as the first live one, makes the seat present.
    async fn register_player(self: Arc<Self>, ctx: Ctx<Self>, conn: ConnCtx<Self>) {
        let id = conn.id().to_owned();
        let abort = ctx.abort_signal();
        let deadline = tokio::time::Instant::now() + CONN_REGISTER_WAIT;
        loop {
            if !self.conns.lock().contains_key(&id) {
                return; // closed before it was registered
            }
            if ctx.conns().any(|c| c.id() == id) {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                self.conns.lock().remove(&id);
                return; // never registered (the connect was aborted)
            }
            tokio::select! {
                () = abort.cancelled() => return,
                () = tokio::time::sleep(CONN_REGISTER_POLL) => {}
            }
        }
        let _turn = self.lane.enter().await;
        let first = {
            let mut conns = self.conns.lock();
            let was_present = conns.values().any(ConnEntry::live_player);
            match conns.get_mut(&id) {
                Some(entry) => entry.live = true,
                None => return, // closed while waiting for the lane
            }
            !was_present
        };
        let seat = HUMAN_SEAT;
        let payload = sync_payload(&self.live.lock(), seat, &[]);
        if let Some(payload) = payload
            && let Err(e) = conn.send("sync", &payload)
        {
            tracing::warn!(err = %err_excerpt(&e), "sync send failed");
        }
        if first && let Err(e) = ctx.broadcast("presence", &PresenceEvent { seat, online: true }) {
            tracing::warn!(err = %err_excerpt(&e), "broadcast failed");
        }
    }
}

#[async_trait]
impl Actor for EuchreTable {
    type Input = Value;
    type ConnParams = Value;
    type Action = rivetkit::Raw;
    type Actions = (
        Snapshot,
        SubmitMove,
        ReviewTrick,
        GetHandReview,
        LastTrick,
        MoveQueue,
        ReviewQueue,
        AiDecisionQueue,
        AiSayQueue,
        TickQueue,
        OnTempoGate,
        ReleaseAiMove,
        OnAiTimeout,
        OnNudge,
        OnAbandon,
        FlushProfile,
        OnReap,
    );
    type Events = ();

    /// Every wake: load the state row, or mint the match from the create
    /// input (TS `createState`) and persist it before anything else runs.
    async fn create(ctx: &Ctx<Self>) -> Result<Self> {
        let config = crate::configured().context(NOT_CONFIGURED)?;
        let db = ctx.sql();
        let state_table = JsonTable::new(STATE_TABLE)?;
        let review_table = JsonTable::new(REVIEW_TABLE)?;
        state_table.ensure(db).await?;
        review_table.ensure(db).await?;
        let state = match state_table.load::<TableState>(db, STATE_KEY).await? {
            Some(state) => state,
            None => {
                let raw = ctx.input()?;
                let input: TableCreateInput = if raw.is_null() {
                    TableCreateInput::default()
                } else {
                    serde_json::from_value(raw).context("decode euchreTable create input")?
                };
                let game_id = match ctx.key().get(1) {
                    Some(rivetkit::ActorKeySegment::String(s)) => s.clone(),
                    Some(rivetkit::ActorKeySegment::Number(n)) => n.to_string(),
                    None => format!("g-{}", random_hex(8)),
                };
                // TS `input.seed ?? randomHex(16)`; an empty seed could never be
                // recorded (the profile wants 1..=256 units), so it is random too.
                let seed = input
                    .seed
                    .clone()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| random_hex(16));
                // `TableState::create` bounds the input (`create_input`): an
                // out-of-bounds id fails the create, before anything persists.
                let state =
                    TableState::create(game_id, seed, new_match_token(), &input, epoch_ms())?;
                state_table.save(db, STATE_KEY, &state).await?;
                state
            }
        };
        crate::test_support::record_start(&state.match_id);
        crate::test_support::record_state(&state.match_id, &state);
        Ok(Self {
            tempo: Tempo::select(config.fast_tempo),
            config,
            lane: ActorLane::new(),
            live: parking_lot::Mutex::new(state),
            conns: parking_lot::Mutex::new(HashMap::new()),
            state_table,
            review_table,
            retries: parking_lot::Mutex::new(HashMap::new()),
            outbox: Outbox::default(),
            recovering: AtomicBool::new(false),
            profile_hold: parking_lot::Mutex::new(None),
            sleeping: AtomicBool::new(false),
        })
    }

    /// TS `run`'s start: reconcile on every wake, before any action runs.
    /// A failure is logged, never returned, so the table still starts (D20).
    async fn on_start(self: Arc<Self>, ctx: Ctx<Self>) -> Result<()> {
        let match_id = self.live.lock().match_id.clone();
        tracing::info!(match_id = %match_id, "euchreTable run loop starting");
        if let Err(e) = self.reconcile(&ctx).await {
            tracing::error!(name = "reconcile", err = %err_excerpt(&e), "{FIRE_FAILED_EVENT}");
            self.spawn_recover(&ctx, "reconcile");
        }
        let st = self.live.lock().clone();
        tracing::info!(
            phase = st.game.hand.phase.as_str(),
            turn_seat = st.game.hand.turn_seat.map(|s| s.get()),
            status = st.game.status.as_str(),
            "euchreTable reconciled"
        );
        Ok(())
    }

    /// TS `onBeforeConnect` + `createConnState`: the internal token, the
    /// Origin, the claims, the token, then the owner (adopting the first visitor).
    async fn on_before_connect(self: Arc<Self>, ctx: Ctx<Self>, params: &Value) -> Result<()> {
        let token = self.match_token();
        let origin = origin_header(&ctx);
        table_auth::origin_gate(&self.config.origins, &token, params, origin.as_deref())?;
        match table_auth::identify(&self.config.auth, &token, params, now_ms() / 1000, true)? {
            Identity::Internal => Ok(()),
            Identity::Player(user) => self.claim(&ctx, &user).await,
        }
    }

    /// TS `onConnect`. The socket keeps the identity it proved at connect
    /// (D17, amended); a player socket goes live once rivetkit registers it
    /// ([`EuchreTable::register_player`]). Stateless HTTP calls are not sockets.
    async fn on_connect(self: Arc<Self>, ctx: Ctx<Self>, conn: ConnCtx<Self>) -> Result<()> {
        if is_http_action(&ctx) {
            return Ok(());
        }
        let params = conn.params().unwrap_or(Value::Null);
        let token = self.match_token();
        // `on_before_connect` verified these params a moment ago.
        let Ok(identity) =
            table_auth::identify(&self.config.auth, &token, &params, now_ms() / 1000, false)
        else {
            return Ok(());
        };
        let player = matches!(identity, Identity::Player(_));
        self.conns.lock().insert(
            conn.id().to_owned(),
            ConnEntry {
                identity,
                live: false,
            },
        );
        if player {
            let table = Arc::clone(&self);
            let task_ctx = ctx.clone();
            let awake = ctx.keep_awake_region();
            ctx.register_task(async move {
                let _awake = awake;
                table.register_player(task_ctx, conn).await;
            });
        }
        Ok(())
    }

    /// TS `onDisconnect`, on the last live player connection only: the seat
    /// goes offline and the open turn stops counting as observed.
    async fn on_disconnect(self: Arc<Self>, ctx: Ctx<Self>, conn: ConnCtx<Self>) {
        let last = {
            let mut conns = self.conns.lock();
            match conns.remove(conn.id()) {
                Some(entry) if entry.live_player() => !conns.values().any(ConnEntry::live_player),
                _ => false,
            }
        };
        if !last {
            return;
        }
        let seat = HUMAN_SEAT;
        if let Err(e) = ctx.broadcast(
            "presence",
            &PresenceEvent {
                seat,
                online: false,
            },
        ) {
            tracing::warn!(err = %err_excerpt(&e), "broadcast failed");
        }
        // The save waits for the lane, so it runs off the hook (the hook runs
        // inline in the actor loop).
        let table = self.clone();
        let task_ctx = ctx.clone();
        let awake = ctx.keep_awake_region();
        ctx.register_task(async move {
            let _awake = awake;
            let _turn = table.lane.enter().await;
            if table.player_connected() {
                return; // the player came back first
            }
            let host = table.host(&task_ctx);
            let mut session = table.session(&host);
            if let Some(opened) = session.st.turn_opened.as_mut() {
                opened.connected = false;
            }
            if let Err(e) = session.flush().await {
                tracing::warn!(err = %err_excerpt(&e), "presence save failed");
            }
        });
    }

    async fn on_sleep(self: Arc<Self>, _ctx: Ctx<Self>) -> Result<()> {
        // Let the shutdown finish; the next wake re-arms the retry from the ledger.
        self.sleeping.store(true, Ordering::SeqCst);
        self.profile_hold.lock().take();
        let match_id = self.live.lock().match_id.clone();
        crate::test_support::record_sleep(&match_id);
        Ok(())
    }
}

// ------------------------------------------------------------- client actions

impl Handler<Snapshot> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, _a: Snapshot) -> Result<Value> {
        let caller = self.caller(&ctx)?;
        caller.require_reader()?;
        let view = match caller {
            Caller::Internal => internal_view(&self.live.lock()),
            _ => crate::core::table_view(&self.live.lock(), HUMAN_SEAT),
        };
        Ok(view_to_value(&view)?)
    }
}

impl Handler<SubmitMove> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: SubmitMove) -> Result<MoveAck> {
        let caller = self.caller(&ctx)?;
        let user = caller.require_player()?.to_owned();
        self.apply_move(&ctx, MoveInput::Submit(&user, &a.first()))
            .await
    }
}

/// Where a human move came from.
enum MoveInput<'a> {
    /// `submitMove`: the verified user and the raw request.
    Submit(&'a str, &'a Value),
    /// The `move` action: the verified user and the client's body.
    Queue(&'a str, &'a MoveMsg),
}

impl EuchreTable {
    /// TS `onMove` under the lane; a `RuleError` becomes the rejected ack.
    /// The body is built inside the lane turn from that turn's state (TS
    /// validated and committed in one synchronous turn). A failure after the
    /// move itself is persisted still acks the move; recovery arms what follows.
    async fn apply_move(
        self: &Arc<Self>,
        ctx: &Ctx<Self>,
        input: MoveInput<'_>,
    ) -> Result<MoveAck> {
        let _turn = self.lane.enter().await;
        let host = self.host(ctx);
        let mut session = self.session(&host);
        let body = match input {
            MoveInput::Submit(user, request) => {
                crate::core::submit_move_body(&session.st, user, request)?
            }
            MoveInput::Queue(user, msg) => queue_move_body(&session.st, user, msg),
        };
        let out = match session.on_move(&body).await {
            Ok(ack) => session.flush().await.map(|()| ack),
            Err(e) => match rule_error(&e) {
                Some(rule) => {
                    let ack = MoveAck::Rejected {
                        code: ErrorCode::Rule(rule.code),
                        legal: rule.legal.clone(),
                    };
                    session.flush().await.map(|()| ack)
                }
                None => Err(e),
            },
        };
        match out {
            Ok(ack) => Ok(ack),
            Err(e) => {
                self.after_failure(ctx, "move", &mut session);
                if move_persisted(session.saved(), &body) {
                    tracing::error!(name = "move", err = %err_excerpt(&e), "{FIRE_FAILED_EVENT}");
                    return Ok(MoveAck::Ok {
                        v: session.saved().game.v,
                    });
                }
                Err(e)
            }
        }
    }

    /// TS `onReview` under the lane.
    async fn apply_review(self: &Arc<Self>, ctx: &Ctx<Self>, body: &ReviewMsg) -> Result<()> {
        let _turn = self.lane.enter().await;
        let host = self.host(ctx);
        let mut session = self.session(&host);
        let out = session.on_review(body).await;
        let out = self.close_internal("review", &mut session, out).await;
        if out.is_err() {
            self.after_failure(ctx, "review", &mut session);
        }
        out
    }
}

impl Handler<ReviewTrick> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: ReviewTrick) -> Result<ReviewTrickAck> {
        self.caller(&ctx)?.require_player()?;
        let body = review_body(&self.live.lock(), &a.first())?;
        // Rust runs it inline under the lane (no queue to enqueue on).
        self.apply_review(&ctx, &body).await?;
        Ok(ReviewTrickAck { queued: True })
    }
}

impl Handler<GetHandReview> for EuchreTable {
    async fn handle(
        self: Arc<Self>,
        ctx: Ctx<Self>,
        a: GetHandReview,
    ) -> Result<Option<CompletedHandReview>> {
        self.caller(&ctx)?.require_player()?;
        let current = self.live.lock().game.hand.hand_no;
        let Some(hand_no) = hand_review_number(&a.first(), current) else {
            return Ok(None);
        };
        self.review_table
            .load::<CompletedHandReview>(ctx.sql(), &hand_no.to_string())
            .await
    }
}

impl Handler<LastTrick> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, _a: LastTrick) -> Result<Option<Trick>> {
        self.caller(&ctx)?.require_reader()?;
        Ok(self.live.lock().game.hand.trick_log.last().cloned())
    }
}

// ------------------------------------------------------------- former queues

impl Handler<MoveQueue> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: MoveQueue) -> Result<MoveAck> {
        let caller = self.caller(&ctx)?;
        let user = caller.require_player()?.to_owned();
        let body: MoveMsg = decode_body("move", a.first())?;
        self.apply_move(&ctx, MoveInput::Queue(&user, &body)).await
    }
}

impl Handler<ReviewQueue> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: ReviewQueue) -> Result<()> {
        self.caller(&ctx)?.require_internal()?;
        let body: ReviewMsg = decode_body("review", a.first())?;
        self.apply_review(&ctx, &body).await
    }
}

impl Handler<AiDecisionQueue> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: AiDecisionQueue) -> Result<()> {
        self.caller(&ctx)?.require_internal()?;
        let body: AIDecision = decode_body("aiDecision", a.first())?;
        let _turn = self.lane.enter().await;
        let host = self.host(&ctx);
        let mut session = self.session(&host);
        let out = session.on_ai_decision(&body).await;
        let out = self.close_internal("aiDecision", &mut session, out).await;
        if out.is_err() {
            self.after_failure(&ctx, "aiDecision", &mut session);
        }
        out
    }
}

impl Handler<AiSayQueue> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: AiSayQueue) -> Result<()> {
        self.caller(&ctx)?.require_internal()?;
        let raw = a.first();
        let null_hand_no = raw.get("handNo").is_some_and(Value::is_null);
        let body: AiSayMsg = decode_body("aiSay", raw)?;
        let _turn = self.lane.enter().await;
        let host = self.host(&ctx);
        let session = self.session(&host);
        session.on_ai_say(&body, null_hand_no);
        Ok(())
    }
}

impl Handler<TickQueue> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: TickQueue) -> Result<()> {
        self.caller(&ctx)?.require_internal()?;
        let body: TickMsg = decode_body("tick", a.first())?;
        let _turn = self.lane.enter().await;
        let host = self.host(&ctx);
        let mut session = self.session(&host);
        let out = session.on_tick_message(&body).await;
        let out = self.close_internal("tick", &mut session, out).await;
        if out.is_err() {
            self.after_failure(&ctx, "tick", &mut session);
        }
        out
    }
}

// ----------------------------------------------------------- schedule targets

impl Handler<OnTempoGate> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: OnTempoGate) -> Result<()> {
        self.fire_target(ctx, OnTempoGate::NAME, TickKind::Tempo, a.0)
            .await
    }
}

impl Handler<ReleaseAiMove> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: ReleaseAiMove) -> Result<()> {
        self.fire_target(ctx, ReleaseAiMove::NAME, TickKind::ReleaseAi, a.0)
            .await
    }
}

impl Handler<OnAiTimeout> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: OnAiTimeout) -> Result<()> {
        self.fire_target(ctx, OnAiTimeout::NAME, TickKind::AiTimeout, a.0)
            .await
    }
}

impl Handler<OnNudge> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: OnNudge) -> Result<()> {
        self.fire_target(ctx, OnNudge::NAME, TickKind::Nudge, a.0)
            .await
    }
}

impl Handler<OnAbandon> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: OnAbandon) -> Result<()> {
        self.fire_target(ctx, OnAbandon::NAME, TickKind::Abandon, a.0)
            .await
    }
}

impl Handler<FlushProfile> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: FlushProfile) -> Result<()> {
        self.fire_target(ctx, FlushProfile::NAME, TickKind::FlushProfile, a.0)
            .await
    }
}

impl Handler<OnReap> for EuchreTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: OnReap) -> Result<()> {
        self.fire_target(ctx, OnReap::NAME, TickKind::Reap, a.0)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TableClient;
    use euchre_server_core::auth::{AuthKeys, Authenticator};
    use euchre_server_core::config::{OriginPolicy, Secret};

    fn config(profile_token: Option<Secret>, client: TableClient) -> TableConfig {
        TableConfig {
            auth: Authenticator::new(AuthKeys {
                guest: Secret::new("unit-guest-key-0123456789abcdef"),
                profile: Secret::new("unit-profile-key-0123456789abcd"),
                internal: Secret::new("unit-internal-token-0123456789ab"),
                identity: Secret::new("unit-identity-key-0123456789abcd"),
                jwt_issuer: None,
                allow_unsigned_email: false,
            }),
            origins: OriginPolicy::strict(Vec::new()).unwrap_or_else(|e| panic!("{e}")),
            fast_tempo: false,
            client,
            profile_token,
        }
    }

    #[test]
    fn an_unbound_client_is_not_a_missing_profile() {
        // A table that wakes before `TableClient::bind` must keep its record
        // for the retry, never mark it recorded unsent.
        let unbound = config(Some(Secret::new("profile-token")), TableClient::deferred());
        assert!(unbound.client.get().is_none());
        assert!(profile_via_outbox(&unbound));
        // Only a deployment with no profile token records nothing.
        assert!(!profile_via_outbox(&config(None, TableClient::deferred())));
    }

    #[test]
    fn only_registered_player_sockets_count_as_present() {
        let pending = ConnEntry {
            identity: Identity::Player("guest-a".into()),
            live: false,
        };
        assert!(!pending.live_player(), "not registered yet");
        let live = ConnEntry {
            live: true,
            ..pending.clone()
        };
        assert!(live.live_player());
        let internal = ConnEntry {
            identity: Identity::Internal,
            live: true,
        };
        assert!(!internal.live_player());
    }
}
