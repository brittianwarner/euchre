//! The `aiSeat` actor (04 §14, §15, §18; ARCHITECTURE §2.4).
//!
//! - **State** is one JSON row (`ai_seat_state`, key `state`) in the actor's
//!   SQLite database, loaded on every wake; never `ctx.kv()`.
//! - **The former queues** (`decide`, `resetHand`, `handEnd`, `gameEnd`) are
//!   actions that accept the body and return at once (03 §16.7). Each message
//!   goes into an in-memory inbox together with a keep-awake region, and one
//!   worker task (spawned in `on_start`) handles them in arrival order while
//!   holding the [`ActorLane`]: the single run loop of TS. The pure handlers
//!   live in `euchre_ai::ai_seat::handlers`.
//! - **Sleep grace**: rivetkit fires the abort signal when grace starts but
//!   still dispatches actions until it finalizes. The worker then takes the
//!   lane, closes the inbox and drains what it holds; a message that arrives
//!   after that is handled inline in its action, under the lane, so nothing
//!   that was acked is dropped.
//! - **Ordering**: the table sends detached (03 §16.7), so a `handEnd` or
//!   `resetHand` can arrive after the next hand's `decide`. A `handEnd` older
//!   than the memory's hand, or a `resetHand` for a hand the memory already
//!   holds a view of, is dropped and logged ([`LIFECYCLE_OUT_OF_ORDER`]) so it
//!   cannot note or wipe the wrong hand.
//! - **Persist, then send**: every state change is made on a copy, saved, and
//!   only then published and followed by its sends (`aiDecision`, `aiSay`,
//!   `recordEpisodes`, `recordTokens`).
//! - **Authority**: the four message actions require a connection whose params
//!   hold the match's internal token (re-checked per action, D17), and every
//!   message body is checked again by the pure handlers. The three reads stay
//!   open, as in TS (`getStatus` is the HUD chip read by the TS tooling).

use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use euchre_ai::ai_seat::handlers::{
    BanterJob, DecideHandled, EpisodeFlush, SeatEnv, begin_banter, complete_episode_flush,
    internal_token_ok, messages, on_decide, on_game_end, on_hand_end, on_reset_hand,
    run_banter_job,
};
use euchre_ai::ai_seat::state::DecisionRecord;
use euchre_ai::ai_seat::state::{AiSeatCreateInput, AiSeatState, CreateStateParams, create_state};
use euchre_ai::ai_seat::status::{AiStatus, get_persona, get_status};
use euchre_ai::js::utf16_prefix;
use euchre_protocol::{
    AIDecideRequest, AiSeatLifecycleKind, AiSeatLifecycleMessage, PersonaView, ProtocolErrorCode,
};
use euchre_server_core::rivet::{ActorLane, JsonTable, user_error};
use rivetkit::prelude::*;
use rivetkit::{ActorKey, ActorKeySegment, KeepAwakeRegion};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use crate::actions::*;
use crate::calls::{
    AI_DECISION, AI_SAY, PROFILE_ACTOR, RECORD_EPISODES, RECORD_TOKENS, TABLE_ACTOR, Target, send,
    send_to_table, table_key, table_params,
};
use crate::runtime::AiSeatRuntime;

/// The JSON table holding the seat state.
pub const STATE_TABLE: &str = "ai_seat_state";
/// The row key of the seat state in [`STATE_TABLE`].
pub const STATE_KEY: &str = "state";
/// Longest `err` text in a failure log, in UTF-16 units (TS `.slice(0, 200)`).
pub const LOG_ERR_UNITS: usize = 200;

/// Startup error when a seat wakes in a registry that did not go through
/// [`crate::register`] (so it has no runtime).
pub const NOT_CONFIGURED: &str =
    "aiSeat is not configured: register it with euchre_ai_seat::register";

/// Message of the `user/internal_token_mismatch` refusal for a caller without
/// the match's internal token (no TS string: TS refused external publishes in
/// its queue guard).
pub const CALLER_REFUSED: &str = "aiSeat accepts actor-to-actor calls with the internal token only";

/// Warning logged when a `handEnd` or `resetHand` arrives after the memory has
/// moved past its hand (no TS string: TS received them in order). Fields:
/// `name`, `handNo`, `memoryHandNo`.
pub const LIFECYCLE_OUT_OF_ORDER: &str = "aiSeat dropped an out-of-order lifecycle message";

tokio::task_local! {
    /// The runtime of the registry whose entry is running (set by
    /// [`crate::register_with`] around rivetkit's `run_actor`).
    pub(crate) static RUNTIME: Arc<AiSeatRuntime>;
}

/// Which former queue a message came in on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageKind {
    Decide,
    ResetHand,
    HandEnd,
    GameEnd,
}

impl MessageKind {
    /// The TS queue name (the `name` field of the failure log).
    fn name(self) -> &'static str {
        match self {
            MessageKind::Decide => Decide::NAME,
            MessageKind::ResetHand => ResetHand::NAME,
            MessageKind::HandEnd => HandEnd::NAME,
            MessageKind::GameEnd => GameEnd::NAME,
        }
    }

    /// The lifecycle kind, `None` for `decide`.
    fn lifecycle(self) -> Option<AiSeatLifecycleKind> {
        match self {
            MessageKind::Decide => None,
            MessageKind::ResetHand => Some(AiSeatLifecycleKind::ResetHand),
            MessageKind::HandEnd => Some(AiSeatLifecycleKind::HandEnd),
            MessageKind::GameEnd => Some(AiSeatLifecycleKind::GameEnd),
        }
    }
}

/// One accepted message. The region keeps the actor awake until it is handled.
struct Inbound {
    kind: MessageKind,
    body: Value,
    _awake: KeepAwakeRegion,
}

/// The `aiSeat` actor: one AI chair of one match, key
/// `["table", gameId, "seat", "1"|"2"|"3"]`.
pub struct AiSeat {
    rt: Arc<AiSeatRuntime>,
    label: String,
    lane: ActorLane,
    state: parking_lot::Mutex<AiSeatState>,
    docs: JsonTable,
    inbox: mpsc::UnboundedSender<Inbound>,
    inbox_rx: parking_lot::Mutex<Option<mpsc::UnboundedReceiver<Inbound>>>,
}

/// A rivetkit key as strings (numbers print as JS would).
pub fn key_strings(key: &ActorKey) -> Vec<String> {
    key.iter()
        .map(|segment| match segment {
            ActorKeySegment::String(s) => s.clone(),
            ActorKeySegment::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
        })
        .collect()
}

/// TS `String(err).slice(0, 200)`.
fn err_excerpt(e: &anyhow::Error) -> String {
    utf16_prefix(&format!("{e:#}"), LOG_ERR_UNITS).to_owned()
}

fn fields(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

/// The create input, when the table sent a well-formed one. A missing or
/// malformed input wakes an unprovisioned seat (empty token: nothing it is
/// sent authenticates), as TS `createState` did for a missing one.
fn decode_input(raw: Value) -> Option<AiSeatCreateInput> {
    if raw.is_null() {
        return None;
    }
    match serde_json::from_value(raw) {
        Ok(input) => Some(input),
        Err(_) => {
            // The serde message can quote values (the token): never log it.
            tracing::warn!("aiSeat create input is malformed; the seat is unprovisioned");
            None
        }
    }
}

/// The body's `internalToken`, as TS read `body.internalToken`.
fn body_token(body: &Value) -> &str {
    body.get("internalToken")
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// TS `body.handNo` as a hand number: absent or `null` is `None`; anything
/// else that is not a whole `u32` is `Err`.
fn hand_no_of(body: &Value) -> std::result::Result<Option<u32>, ()> {
    match body.get("handNo") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(Some)
            .ok_or(()),
    }
}

/// Whether a lifecycle message is for a hand the memory has already moved past
/// (the table's detached sends arrived out of order): a `handEnd` for an older
/// hand, or a `resetHand` for an older hand or for the hand a `decide` has
/// already absorbed. Dropping these keeps the next hand's view from being
/// noted as this hand, and keeps a late reset from wiping it.
fn out_of_order(state: &AiSeatState, kind: MessageKind, hand_no: Option<u32>) -> bool {
    let Some(hand_no) = hand_no else {
        return false;
    };
    let memory = &state.memory;
    match kind {
        MessageKind::HandEnd => hand_no < memory.hand_no,
        MessageKind::ResetHand => {
            hand_no < memory.hand_no || (hand_no == memory.hand_no && memory.snapshot.is_some())
        }
        MessageKind::Decide | MessageKind::GameEnd => false,
    }
}

impl AiSeat {
    /// The seat's label in [`crate::test_support`] (`"<gameId>/<seat>"`).
    pub fn label(&self) -> &str {
        &self.label
    }

    fn snapshot(&self) -> AiSeatState {
        self.state.lock().clone()
    }

    /// Save `next`, then publish it. Call while holding the lane.
    async fn commit(&self, ctx: &Ctx<Self>, next: AiSeatState) -> Result<()> {
        self.docs.save(ctx.sql(), STATE_KEY, &next).await?;
        crate::test_support::record_state(&self.label, &next);
        *self.state.lock() = next;
        Ok(())
    }

    fn env(&self) -> SeatEnv<'_> {
        SeatEnv {
            clock: self.rt.clock.as_ref(),
            log: self.rt.log.as_ref(),
            provider: &self.rt.provider,
        }
    }

    /// D17: a connected caller must hold the match's internal token in its
    /// params. No connection is an internal dispatch.
    fn require_internal(&self, ctx: &Ctx<Self>) -> Result<()> {
        let Some(conn) = ctx.conn() else {
            return Ok(());
        };
        let params = conn.params().unwrap_or(Value::Null);
        self.check_params(&params)
    }

    fn check_params(&self, params: &Value) -> Result<()> {
        let got = params
            .get("internalToken")
            .and_then(Value::as_str)
            .unwrap_or("");
        let expected = self.state.lock().internal_token.clone();
        if internal_token_ok(&expected, got) {
            Ok(())
        } else {
            Err(user_error(
                ProtocolErrorCode::InternalTokenMismatch.as_str(),
                CALLER_REFUSED,
            ))
        }
    }

    /// Accept one message: queue it with a keep-awake region and return. When
    /// the worker has closed the inbox (sleep grace), handle it here, under
    /// the lane, before acking: rivetkit does not finalize the sleep while an
    /// action is in flight, so an acked message is never dropped.
    async fn accept(self: &Arc<Self>, ctx: &Ctx<Self>, kind: MessageKind, body: Value) {
        let message = Inbound {
            kind,
            body,
            _awake: ctx.keep_awake_region(),
        };
        if let Err(mpsc::error::SendError(message)) = self.inbox.send(message) {
            crate::test_support::record_inline(&self.label);
            let _turn = self.lane.enter().await;
            self.run_message(ctx, message).await;
        }
    }

    /// Handle one message (call while holding the lane); a failure is logged
    /// as TS's run loop did.
    async fn run_message(self: &Arc<Self>, ctx: &Ctx<Self>, message: Inbound) {
        if crate::test_support::take_sleep_request(&self.label) {
            // Tests only: start sleep grace while this message is in hand.
            let _ = ctx.sleep();
        }
        let kind = message.kind;
        let outcome = match kind {
            MessageKind::Decide => self.on_decide(ctx, message.body).await,
            MessageKind::ResetHand => self.on_reset_hand(ctx, message.body).await,
            MessageKind::HandEnd => self.on_hand_end(ctx, message.body).await,
            MessageKind::GameEnd => self.on_game_end(ctx, message.body).await,
        };
        if let Err(e) = outcome {
            self.rt.log.error(
                messages::QUEUE_HANDLER_FAILED,
                &fields(&[
                    ("name", json!(kind.name())),
                    ("err", json!(err_excerpt(&e))),
                ]),
            );
        }
    }

    /// The worker: one message at a time, in arrival order, under the lane.
    /// When sleep grace starts (the abort signal), it takes the lane, closes
    /// the inbox and drains what is already queued, then exits so the sleep
    /// can finalize; later messages are handled inline by [`Self::accept`].
    async fn drain(self: Arc<Self>, ctx: Ctx<Self>, mut rx: mpsc::UnboundedReceiver<Inbound>) {
        let abort = ctx.abort_signal();
        loop {
            let next = tokio::select! {
                biased;
                m = rx.recv() => m,
                _ = abort.cancelled() => None,
            };
            let Some(message) = next else {
                break;
            };
            let _turn = self.lane.enter().await;
            self.run_message(&ctx, message).await;
        }
        let _turn = self.lane.enter().await;
        rx.close();
        while let Ok(message) = rx.try_recv() {
            self.run_message(&ctx, message).await;
        }
    }

    /// Read a lifecycle body the way TS did: only `internalToken`, `gameId` and
    /// `handNo` (`kind` is the action's own; TS never read `body.kind`). A
    /// message for another match, or with a wrong token, is dropped (`None`).
    fn lifecycle_body(
        &self,
        kind: MessageKind,
        body: &Value,
    ) -> Result<Option<AiSeatLifecycleMessage>> {
        if body.is_null() {
            return Err(anyhow!("lifecycle body is null"));
        }
        let state = self.state.lock();
        let token = body_token(body);
        let game_id = body.get("gameId").and_then(Value::as_str);
        if !internal_token_ok(&state.internal_token, token)
            || game_id != Some(state.game_id.as_str())
        {
            return Ok(None);
        }
        let hand_no = match hand_no_of(body) {
            Ok(n) => n,
            // TS fed it to resetMemory as is; only resetHand ever read it.
            Err(()) if kind == MessageKind::ResetHand => {
                return Err(anyhow!("resetHand handNo is not a hand number"));
            }
            Err(()) => None,
        };
        if out_of_order(&state, kind, hand_no) {
            let memory_hand = state.memory.hand_no;
            drop(state);
            self.rt.log.warn(
                LIFECYCLE_OUT_OF_ORDER,
                &fields(&[
                    ("name", json!(kind.name())),
                    ("handNo", json!(hand_no)),
                    ("memoryHandNo", json!(memory_hand)),
                ]),
            );
            return Ok(None);
        }
        let Some(lifecycle) = kind.lifecycle() else {
            return Err(anyhow!("decide is not a lifecycle message"));
        };
        Ok(Some(AiSeatLifecycleMessage {
            internal_token: token.to_owned(),
            game_id: state.game_id.clone(),
            kind: lifecycle,
            hand_no,
        }))
    }

    /// TS `onDecide`: the pure handler, then save, then `aiDecision`, then the
    /// banter gate (save, then the line in the background).
    async fn on_decide(self: &Arc<Self>, ctx: &Ctx<Self>, body: Value) -> Result<()> {
        if body.is_null() {
            return Err(anyhow!("decide body is null"));
        }
        let req = match serde_json::from_value::<AIDecideRequest>(body.clone()) {
            Ok(req) => req,
            Err(e) => return self.malformed_decide(ctx, &body, e.into()).await,
        };
        let mut next = self.snapshot();
        let handled = on_decide(&mut next, &req, self.env()).await;
        let changed = handled.state_changed();
        let DecideHandled::Answered(answered) = handled else {
            if changed {
                self.commit(ctx, next).await?;
            }
            return Ok(());
        };
        let key = table_key(&next.game_id);
        let params = table_params(&next.internal_token);
        self.commit(ctx, next).await?;
        send_to_table(
            &self.rt.client,
            self.rt.call_timeout,
            Target {
                actor: TABLE_ACTOR,
                key: &key,
                params: Some(&params),
                action: AI_DECISION,
            },
            &answered.decision,
        )
        .await?;
        // Optional chatter starts only after the decision is delivered.
        let mut after = self.snapshot();
        if let Some(job) = begin_banter(&mut after, &req, &self.rt.provider) {
            self.commit(ctx, after).await?;
            self.spawn_banter(ctx, job);
        }
        Ok(())
    }

    /// A decide body that failed to decode, checked against the raw JSON in
    /// the TS order (token, match, seat, `turnId`): those drops count and log
    /// as in TS. A body that passes them fails (TS threw on the partial view).
    async fn malformed_decide(
        &self,
        ctx: &Ctx<Self>,
        body: &Value,
        e: anyhow::Error,
    ) -> Result<()> {
        let mut next = self.snapshot();
        if !internal_token_ok(&next.internal_token, body_token(body)) {
            next.meter.token_rejections += 1;
            self.rt.log.warn(
                messages::FORGED_DECIDE,
                &fields(&[("seat", json!(next.seat.get()))]),
            );
            return self.commit(ctx, next).await;
        }
        if body.get("gameId").and_then(Value::as_str) != Some(next.game_id.as_str()) {
            return Ok(());
        }
        let claimed = body.get("seat").cloned();
        if claimed.as_ref().and_then(Value::as_u64) != Some(u64::from(next.seat.get())) {
            next.meter.seat_rejections += 1;
            let mut log = fields(&[("mine", json!(next.seat.get()))]);
            // TS logged `claimed: undefined`, which drops the key.
            if let Some(claimed) = claimed {
                log.insert("claimed".to_owned(), claimed);
            }
            self.rt.log.warn(messages::WRONG_SEAT, &log);
            return self.commit(ctx, next).await;
        }
        // `body.turnId === state.lastTurnId` (`null === null` included).
        let last = next
            .last_turn_id
            .as_deref()
            .map_or(Value::Null, |t| json!(t));
        if body.get("turnId") == Some(&last) {
            next.meter.stale_turns += 1;
            return self.commit(ctx, next).await;
        }
        Err(e)
    }

    /// TS `c.waitUntil(callBanterWriter(...).then(send aiSay))`, held awake.
    fn spawn_banter(&self, ctx: &Ctx<Self>, job: BanterJob) {
        let awake = ctx.keep_awake_region();
        let rt = self.rt.clone();
        ctx.register_task(async move {
            let _awake = awake;
            let Some(say) = run_banter_job(&rt.provider, &job).await else {
                return;
            };
            let key = table_key(&job.game_id);
            let params = table_params(&job.internal_token);
            let sent = send_to_table(
                &rt.client,
                rt.call_timeout,
                Target {
                    actor: TABLE_ACTOR,
                    key: &key,
                    params: Some(&params),
                    action: AI_SAY,
                },
                &say,
            )
            .await;
            if sent.is_err() {
                rt.log.warn(messages::BANTER_UNDELIVERED, &Map::new());
            }
        });
    }

    /// TS `onResetHand`, then `saveState` either way.
    async fn on_reset_hand(&self, ctx: &Ctx<Self>, body: Value) -> Result<()> {
        let Some(msg) = self.lifecycle_body(MessageKind::ResetHand, &body)? else {
            return Ok(());
        };
        let mut next = self.snapshot();
        on_reset_hand(&mut next, &msg);
        self.commit(ctx, next).await
    }

    /// TS `onHandEnd`: note the hand and buffer its episode (saved), flush the
    /// episodes, save.
    async fn on_hand_end(&self, ctx: &Ctx<Self>, body: Value) -> Result<()> {
        let Some(msg) = self.lifecycle_body(MessageKind::HandEnd, &body)? else {
            return Ok(());
        };
        let mut next = self.snapshot();
        let now = self.rt.clock.now_ms();
        let Some(flush) = on_hand_end(&mut next, &msg, now, self.rt.profile_token.as_deref())
        else {
            return Ok(());
        };
        self.commit(ctx, next.clone()).await?;
        self.flush_episodes(&mut next, flush).await;
        self.commit(ctx, next).await
    }

    /// TS `onGameEnd`: flush the episodes, then the token spend, then save.
    async fn on_game_end(&self, ctx: &Ctx<Self>, body: Value) -> Result<()> {
        let Some(msg) = self.lifecycle_body(MessageKind::GameEnd, &body)? else {
            return Ok(());
        };
        let mut next = self.snapshot();
        let Some(flush) = on_game_end(&mut next, &msg, self.rt.profile_token.as_deref()) else {
            return Ok(());
        };
        self.flush_episodes(&mut next, flush.episodes).await;
        if let Some(tokens) = flush.tokens {
            let sent = send(
                &self.rt.client,
                self.rt.call_timeout,
                Target {
                    actor: PROFILE_ACTOR,
                    key: &tokens.profile_key,
                    params: None,
                    action: RECORD_TOKENS,
                },
                &tokens.body,
            )
            .await;
            if let Err(e) = sent {
                self.rt.log.warn(
                    messages::FLUSH_TOKENS_FAILED,
                    &fields(&[("err", json!(err_excerpt(&e)))]),
                );
            }
        }
        self.commit(ctx, next).await
    }

    /// TS `flushEpisodes` from the send on: send, then clear or keep the buffer.
    async fn flush_episodes(&self, state: &mut AiSeatState, flush: EpisodeFlush) {
        let EpisodeFlush::Send { profile_key, body } = flush else {
            return;
        };
        let sent = send(
            &self.rt.client,
            self.rt.call_timeout,
            Target {
                actor: PROFILE_ACTOR,
                key: &profile_key,
                params: None,
                action: RECORD_EPISODES,
            },
            &body,
        )
        .await;
        let err = sent.err().map(|e| format!("{e:#}"));
        let result = match err.as_deref() {
            None => Ok(()),
            Some(e) => Err(e),
        };
        complete_episode_flush(state, result, self.rt.log.as_ref());
    }
}

#[async_trait]
impl Actor for AiSeat {
    type Input = Value;
    type ConnParams = Value;
    type Action = rivetkit::Raw;
    type Actions = (
        Decide,
        ResetHand,
        HandEnd,
        GameEnd,
        GetStatus,
        GetPersona,
        GetDecisionLog,
    );
    type Events = ();

    /// Every wake: load the stored state, or build it from the key and the
    /// create input (TS `createState`) and store it.
    async fn create(ctx: &Ctx<Self>) -> Result<Self> {
        let rt = RUNTIME
            .try_with(Arc::clone)
            .map_err(|_| anyhow!(NOT_CONFIGURED))?;
        let key = key_strings(ctx.key());
        let db = ctx.sql();
        let docs = JsonTable::new(STATE_TABLE)?;
        docs.ensure(db).await?;
        let state = match docs.load::<AiSeatState>(db, STATE_KEY).await? {
            Some(state) => state,
            None => {
                let input = decode_input(ctx.input()?);
                let state = create_state(CreateStateParams {
                    key: &key,
                    input: input.as_ref(),
                })?;
                docs.save(db, STATE_KEY, &state).await?;
                state
            }
        };
        let label = crate::test_support::label(&state.game_id, state.seat.get());
        crate::test_support::record_start(&label);
        crate::test_support::record_state(&label, &state);
        let (tx, rx) = mpsc::unbounded_channel();
        Ok(Self {
            rt,
            label,
            lane: ActorLane::new(),
            state: parking_lot::Mutex::new(state),
            docs,
            inbox: tx,
            inbox_rx: parking_lot::Mutex::new(Some(rx)),
        })
    }

    /// Every wake: start the worker that handles the former queue messages.
    async fn on_start(self: Arc<Self>, ctx: Ctx<Self>) -> Result<()> {
        let Some(rx) = self.inbox_rx.lock().take() else {
            return Ok(());
        };
        let me = self.clone();
        let worker_ctx = ctx.clone();
        ctx.register_task(me.drain(worker_ctx, rx));
        Ok(())
    }

    async fn on_sleep(self: Arc<Self>, _ctx: Ctx<Self>) -> Result<()> {
        crate::test_support::record_sleep(&self.label);
        Ok(())
    }
}

// ------------------------------------------------------------ former queues

impl Handler<Decide> for AiSeat {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: Decide) -> Result<()> {
        self.require_internal(&ctx)?;
        self.accept(&ctx, MessageKind::Decide, a.body()).await;
        Ok(())
    }
}

impl Handler<ResetHand> for AiSeat {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: ResetHand) -> Result<()> {
        self.require_internal(&ctx)?;
        self.accept(&ctx, MessageKind::ResetHand, a.body()).await;
        Ok(())
    }
}

impl Handler<HandEnd> for AiSeat {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: HandEnd) -> Result<()> {
        self.require_internal(&ctx)?;
        self.accept(&ctx, MessageKind::HandEnd, a.body()).await;
        Ok(())
    }
}

impl Handler<GameEnd> for AiSeat {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: GameEnd) -> Result<()> {
        self.require_internal(&ctx)?;
        self.accept(&ctx, MessageKind::GameEnd, a.body()).await;
        Ok(())
    }
}

// ------------------------------------------------------------ reads (open, as in TS)

impl Handler<GetStatus> for AiSeat {
    async fn handle(self: Arc<Self>, _ctx: Ctx<Self>, _a: GetStatus) -> Result<AiStatus> {
        Ok(get_status(&self.state.lock()))
    }
}

impl Handler<GetPersona> for AiSeat {
    async fn handle(self: Arc<Self>, _ctx: Ctx<Self>, _a: GetPersona) -> Result<PersonaView> {
        Ok(get_persona(&self.state.lock()))
    }
}

impl Handler<GetDecisionLog> for AiSeat {
    async fn handle(
        self: Arc<Self>,
        _ctx: Ctx<Self>,
        _a: GetDecisionLog,
    ) -> Result<Vec<DecisionRecord>> {
        Ok(self.state.lock().decision_log.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_print_numbers_like_js() {
        let key: ActorKey = vec![
            ActorKeySegment::String("table".into()),
            ActorKeySegment::String("g1".into()),
            ActorKeySegment::String("seat".into()),
            ActorKeySegment::Number(2.0),
        ];
        assert_eq!(key_strings(&key), vec!["table", "g1", "seat", "2"]);
    }

    #[test]
    fn err_excerpt_cuts_at_200_utf16_units() {
        let e = anyhow!("{}", "x".repeat(250));
        assert_eq!(err_excerpt(&e).len(), 200);
        let e = anyhow!("short");
        assert_eq!(err_excerpt(&e), "short");
    }

    #[test]
    fn a_missing_input_is_unprovisioned_and_a_bad_one_too() {
        assert!(decode_input(Value::Null).is_none());
        assert!(decode_input(json!({ "gameId": 5 })).is_none());
    }

    #[test]
    fn hand_numbers_read_like_ts() {
        assert_eq!(hand_no_of(&json!({})), Ok(None));
        assert_eq!(hand_no_of(&json!({ "handNo": null })), Ok(None));
        assert_eq!(hand_no_of(&json!({ "handNo": 4 })), Ok(Some(4)));
        assert_eq!(hand_no_of(&json!({ "handNo": "4" })), Err(()));
        assert_eq!(hand_no_of(&json!({ "handNo": -1 })), Err(()));
        assert_eq!(hand_no_of(&json!({ "handNo": 1.5 })), Err(()));
    }

    fn state_at(hand_no: u32, absorbed: bool) -> Result<AiSeatState> {
        let key: Vec<String> = ["table", "g1", "seat", "1"].map(str::to_owned).into();
        let mut state = create_state(CreateStateParams {
            key: &key,
            input: None,
        })?;
        state.memory.hand_no = hand_no;
        if absorbed {
            state.memory.snapshot = Some(serde_json::from_value(json!({
                "handNo": hand_no,
                "trump": null,
                "makerSeat": null,
                "aloneSeat": null,
                "tricksWon": [0, 0],
                "result": null
            }))?);
        }
        Ok(state)
    }

    #[test]
    fn late_lifecycle_messages_are_out_of_order() -> Result<()> {
        // The next hand's decide (hand 5) already absorbed its view.
        let state = state_at(5, true)?;
        assert!(out_of_order(&state, MessageKind::HandEnd, Some(4)));
        assert!(out_of_order(&state, MessageKind::ResetHand, Some(5)));
        assert!(out_of_order(&state, MessageKind::ResetHand, Some(4)));
        // In order: the hand that is ending, the next hand's reset.
        assert!(!out_of_order(&state, MessageKind::HandEnd, Some(5)));
        assert!(!out_of_order(&state, MessageKind::ResetHand, Some(6)));
        assert!(!out_of_order(&state, MessageKind::HandEnd, None));
        assert!(!out_of_order(&state, MessageKind::GameEnd, Some(1)));
        // A reset for the current hand before any decide is harmless.
        let fresh = state_at(5, false)?;
        assert!(!out_of_order(&fresh, MessageKind::ResetHand, Some(5)));
        Ok(())
    }

    #[test]
    fn a_closed_inbox_hands_the_message_back() {
        // The grace path relies on this: after `close`, queued messages still
        // drain, and a later send returns its message for inline handling.
        let (tx, mut rx) = mpsc::unbounded_channel::<u8>();
        assert!(tx.send(1).is_ok());
        rx.close();
        assert_eq!(rx.try_recv().ok(), Some(1));
        assert!(rx.try_recv().is_err());
        let Err(mpsc::error::SendError(back)) = tx.send(2) else {
            panic!("a send after close was accepted");
        };
        assert_eq!(back, 2);
    }

    #[test]
    fn the_message_names_are_the_ts_queue_names() {
        let names: Vec<_> = [
            MessageKind::Decide,
            MessageKind::ResetHand,
            MessageKind::HandEnd,
            MessageKind::GameEnd,
        ]
        .iter()
        .map(|k| k.name())
        .collect();
        assert_eq!(names, ["decide", "resetHand", "handEnd", "gameEnd"]);
    }
}
