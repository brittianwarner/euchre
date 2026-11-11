//! Engine-backed tests of the table's calls to `aiSeat` and `playerProfile`
//! (lane SERVER-TABLE, stage TABLE-AI-PROFILE; 03 §10, §11, §13, §14).
//!
//! Each test prints `SKIP` and returns unless `EUCHRE_E2E=1`. Run:
//!
//! ```text
//! EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6711 EUCHRE_TEST_STORAGE=<dir> \
//!   cargo test -p euchre-table --test table_ai_e2e -- --nocapture
//! ```
//!
//! The other actors are minimal stand-ins defined here, registered under the
//! real names (`aiSeat`, `playerProfile`) with the real action names (D29).
//! The stand-in seat acks `decide` at once and answers later through the
//! table's `aiDecision` / `aiSay` with params `{ internalToken }`, choosing the
//! `euchre-ai` heuristic top move. Per match it can answer late, twice, with a
//! wrong `turnId`, with an illegal id, never, or only once the table slept.
//! The stand-in profile records every body it receives.
//!
//! The tests run one at a time (one shared client slot per process), each on
//! its own pool of the shared engine:
//!
//! - `table_ai_full_match_end_to_end`: a whole match against three stand-in
//!   seats; `recordHands` once per hand, then `recordGame` exactly once with the
//!   TS body; each seat hears `resetHand`, `decide`, `handEnd` (slow, 300 ms)
//!   and `gameEnd` in order; chat through `aiSay` and `banter`; `thinking`
//!   edges with `extended: false`.
//! - `table_ai_misbehaving_seats_end_to_end`: late, double, wrong-turn,
//!   illegal and silent seats never stall the game or double-apply a move.
//! - `table_ai_sleep_between_dispatch_and_decision_end_to_end`: the table
//!   sleeps right after sending `decide`; the seat's answer wakes it and the
//!   turn plays exactly once.
//! - `table_ai_missing_seat_end_to_end`: no `aiSeat` registered; every AI turn
//!   plays the heuristic fallback within the hard cap.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use async_trait::async_trait;
use euchre_ai::{RankParams, top_move};
use euchre_engine::AiDecisionSource;
use euchre_protocol::{
    AIDecideRequest, AIDecision, AiSayMsg, AiSeatLifecycleMessage, MoveAck, TableCreateInput,
    default_personas,
};
use euchre_server_core::auth::{AuthKeys, Authenticator, GuestAudience, now_ms, now_secs};
use euchre_server_core::config::{OriginPolicy, Secret};
use euchre_server_core::rivet::{
    call_with_timeout, forward_events, stateless_call_with_timeout, user_error,
};
use euchre_server_core::testing::{TestEngine, TestEngineOptions, e2e_enabled, e2e_skip};
use euchre_table::{
    ACTOR_NAME, EuchreTable, Snapshot, SubmitMove, TableClient, TableConfig, actor_key,
    register_with, test_support,
};
use parking_lot::Mutex;
use rivetkit::client::{Client, ConnectionStatus, GetOrCreateOptions, SubscriptionHandle};
use rivetkit::prelude::*;
use rivetkit::{ActorConfig, Event, TypedActorConnection, TypedActorHandle, TypedClientExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

// ------------------------------------------------------------------ fixtures

const CALL: Duration = Duration::from_secs(15);
const EVENT: Duration = Duration::from_secs(20);
const ALLOWED_ORIGIN: &str = "https://app.example";
/// `EUCHRE_INTERNAL_TOKEN` of this deployment: the profile write token.
const INTERNAL: &str = "e2e-internal-token-0123456789abcdef";
/// `Tempo::FAST.ai_hard_cap_ms`.
const HARD_CAP: Duration = Duration::from_millis(1500);
const SLOW_HAND_END: Duration = Duration::from_millis(300);
/// A stand-in seat's tries at one callback into the table.
const CALLBACK_ATTEMPTS: u32 = 5;
const CALLBACK_BACKOFF: Duration = Duration::from_millis(100);
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(2);

/// The one client slot of this process (`configure` compares the slot).
static CLIENT: LazyLock<TableClient> = LazyLock::new(TableClient::deferred);
/// One test at a time: each binds its own engine's client into [`CLIENT`].
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn auth() -> Authenticator {
    Authenticator::new(AuthKeys {
        guest: Secret::new("e2e-guest-key-0123456789abcdef"),
        profile: Secret::new("e2e-profile-key-0123456789abcdef"),
        internal: Secret::new(INTERNAL),
        identity: Secret::new("e2e-identity-key-0123456789abcdef"),
        jwt_issuer: None,
        allow_unsigned_email: false,
    })
}

fn config() -> Result<TableConfig> {
    Ok(TableConfig {
        auth: auth(),
        origins: OriginPolicy::strict(vec![ALLOWED_ORIGIN.to_owned()])?,
        fast_tempo: true,
        client: CLIENT.clone(),
        profile_token: Some(Secret::new(INTERNAL)),
    })
}

fn guest() -> String {
    format!("guest-{}", uuid::Uuid::new_v4())
}

fn game_id(tag: &str) -> String {
    format!("{tag}-{}", uuid::Uuid::new_v4().simple())
}

fn player_params(user: &str) -> Result<Value> {
    let token = auth().sign_guest_at(user, GuestAudience::Actors, now_secs())?;
    Ok(json!({ "token": token, "protocolVersion": 1 }))
}

/// A table with the three default personas, as the lobby creates it.
fn table(client: &Client, game: &str, user: &str) -> Result<TypedActorHandle<EuchreTable>> {
    let input = serde_json::to_value(TableCreateInput {
        seed: Some(format!("seed-{game}")),
        personas: Some(default_personas()),
        ..TableCreateInput::default()
    })?;
    client.get_or_create_typed::<EuchreTable>(
        ACTOR_NAME,
        actor_key(game),
        GetOrCreateOptions {
            params: Some(player_params(user)?),
            create_with_input: Some(input),
            ..Default::default()
        },
    )
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .with_test_writer()
        .try_init();
}

fn quick() -> ActorConfig {
    ActorConfig {
        sleep_timeout: Duration::from_secs(1),
        ..ActorConfig::default()
    }
}

/// The stand-ins stay up for a whole test. A seat that goes to sleep just as
/// a call reaches it fails that call ("Actor queue delivery interrupted"),
/// which the table survives (logged, best effort) but which would blur the
/// order checks here.
fn awake() -> ActorConfig {
    ActorConfig {
        sleep_timeout: Duration::from_secs(600),
        ..ActorConfig::default()
    }
}

async fn run<F, Fut>(name: &str, with_seats: bool, body: F) -> Result<()>
where
    F: FnOnce(TestEngine) -> Fut,
    Fut: std::future::Future<Output = (TestEngine, Result<()>)>,
{
    if !e2e_enabled() {
        e2e_skip(name);
        return Ok(());
    }
    let _serial = SERIAL.lock().await;
    init_tracing();
    test_support::enable();
    euchre_table::configure(config()?)?;
    let cfg = config()?;
    let engine = TestEngine::start(TestEngineOptions::from_env(move |registry| {
        if let Err(e) = register_with(registry, cfg.clone(), quick()) {
            panic!("register euchreTable: {e}");
        }
        if with_seats {
            registry.register_actor_with::<StandInSeat>("aiSeat", awake());
        }
        registry.register_actor_with::<StandInProfile>("playerProfile", awake());
    })?)
    .await?;
    CLIENT.bind(engine.client());
    let (engine, result) = body(engine).await;
    CLIENT.unbind();
    engine.shutdown().await?;
    result
}

fn state_of(game: &str) -> Result<Value> {
    test_support::last_state(game).with_context(|| format!("no persisted state for {game}"))
}

async fn eventually<T>(
    what: &str,
    wait: Duration,
    mut probe: impl FnMut() -> Option<T>,
) -> Result<T> {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(v) = probe() {
            return Ok(v);
        }
        ensure!(
            Instant::now() < deadline,
            "{what}: timed out after {wait:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// ------------------------------------------------------- stand-in recordings

/// Orders every recorded call across both stand-ins.
static SEQ: AtomicU64 = AtomicU64::new(0);

fn seq() -> u64 {
    SEQ.fetch_add(1, Ordering::SeqCst)
}

/// How a stand-in seat answers `decide` for one match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Answer {
    /// At once, with the heuristic top move.
    #[default]
    Prompt,
    /// After this long.
    Late(Duration),
    /// Twice: the top move, then another legal move.
    Twice,
    /// With a `turnId` the table never issued.
    WrongTurn,
    /// With a move id outside the legal set.
    Illegal,
    /// Never.
    Never,
    /// Only once the table has slept at least once.
    AfterSleep,
}

#[derive(Clone, Copy, Debug, Default)]
struct Mode {
    answer: Answer,
    /// Seat 2 streams and finalizes a line through `aiSay`; seat 1 adds `banter`.
    talk: bool,
    /// `handEnd` takes [`SLOW_HAND_END`] (recorded when it returns).
    slow_hand_end: bool,
}

static MODES: LazyLock<Mutex<HashMap<String, Mode>>> = LazyLock::new(Default::default);
/// [`Answer::AfterSleep`] matches whose test let the seat answer.
static RELEASED: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(Default::default);

fn mode_of(game: &str) -> Mode {
    MODES.lock().get(game).copied().unwrap_or_default()
}

/// One call a stand-in seat received.
#[derive(Clone, Debug)]
struct SeatCall {
    seq: u64,
    game: String,
    seat: u8,
    action: &'static str,
    /// `decide`: `view.handNo`; lifecycle: `handNo` (absent for `gameEnd`).
    hand_no: Option<u32>,
    /// The `internalToken` connection param.
    params_token: Option<String>,
    /// `decide` only.
    turn_id: Option<String>,
    /// `decide`: what the heuristic picks (the table parks the same move).
    top: Option<String>,
    /// `decide`: the first legal id.
    legal0: Option<String>,
    /// How often the table had slept when the call arrived.
    sleeps_at: u64,
}

static SEAT_LOG: LazyLock<Mutex<Vec<SeatCall>>> = LazyLock::new(Default::default);
/// The create input each seat actor was made with: (game, seat) → input.
static SEAT_INPUTS: LazyLock<Mutex<HashMap<(String, u8), Value>>> = LazyLock::new(Default::default);
/// Failures of a stand-in's own calls back into the table.
static CALLBACK_ERRORS: LazyLock<Mutex<Vec<(String, String)>>> = LazyLock::new(Default::default);

fn callback_failed(game: &str, what: String) {
    CALLBACK_ERRORS.lock().push((game.to_owned(), what));
}

/// This match's failed callbacks (a late answer of an earlier test may land
/// after its engine is gone).
fn callback_errors(game: &str) -> Vec<String> {
    CALLBACK_ERRORS
        .lock()
        .iter()
        .filter(|(g, _)| g == game)
        .map(|(_, e)| e.clone())
        .collect()
}

fn seat_calls(game: &str) -> Vec<SeatCall> {
    SEAT_LOG
        .lock()
        .iter()
        .filter(|c| c.game == game)
        .cloned()
        .collect()
}

#[derive(Clone, Debug)]
struct ProfileCall {
    seq: u64,
    user: String,
    action: &'static str,
    body: Value,
}

static PROFILE_LOG: LazyLock<Mutex<Vec<ProfileCall>>> = LazyLock::new(Default::default);

fn profile_calls(user: &str, action: &str) -> Vec<ProfileCall> {
    PROFILE_LOG
        .lock()
        .iter()
        .filter(|c| c.user == user && c.action == action)
        .cloned()
        .collect()
}

// ------------------------------------------------------- the stand-in aiSeat

macro_rules! raw_action {
    ($ty:ident, $name:literal) => {
        #[derive(Serialize, Deserialize, Clone, Debug, Default)]
        struct $ty(Vec<Value>);
        impl Action for $ty {
            type Output = ();
            const NAME: &'static str = $name;
        }
        impl $ty {
            fn body(&self) -> Value {
                self.0.first().cloned().unwrap_or(Value::Null)
            }
        }
    };
}

raw_action!(Decide, "decide");
raw_action!(ResetHand, "resetHand");
raw_action!(HandEnd, "handEnd");
raw_action!(GameEnd, "gameEnd");
raw_action!(RecordGame, "recordGame");
raw_action!(RecordHands, "recordHands");

fn key_string(ctx_key: &rivetkit::ActorKey, i: usize) -> String {
    match ctx_key.get(i) {
        Some(rivetkit::ActorKeySegment::String(s)) => s.clone(),
        Some(rivetkit::ActorKeySegment::Number(n)) => n.to_string(),
        None => String::new(),
    }
}

/// Stand-in `aiSeat`, key `["table", gameId, "seat", "<n>"]`.
struct StandInSeat {
    game: String,
    seat: u8,
}

impl StandInSeat {
    fn params_token<A: Actor<ConnParams = Value>>(ctx: &Ctx<A>) -> Option<String> {
        let params = ctx.conn().and_then(|c| c.params().ok())?;
        params["internalToken"].as_str().map(str::to_owned)
    }

    fn lifecycle(&self, params_token: Option<String>, action: &'static str, body: &Value) {
        let msg: Option<AiSeatLifecycleMessage> = serde_json::from_value(body.clone()).ok();
        SEAT_LOG.lock().push(SeatCall {
            seq: seq(),
            game: self.game.clone(),
            seat: self.seat,
            action,
            hand_no: msg.and_then(|m| m.hand_no),
            params_token,
            turn_id: None,
            top: None,
            legal0: None,
            sleeps_at: test_support::sleeps(&self.game),
        });
    }
}

#[async_trait]
impl Actor for StandInSeat {
    type Input = Value;
    type ConnParams = Value;
    type Action = rivetkit::Raw;
    type Actions = (Decide, ResetHand, HandEnd, GameEnd);
    type Events = ();

    async fn create(ctx: &Ctx<Self>) -> Result<Self> {
        let key = ctx.key();
        let game = key_string(key, 1);
        let seat = key_string(key, 3).parse().unwrap_or(0);
        let input = ctx.input()?;
        if !input.is_null() {
            SEAT_INPUTS
                .lock()
                .entry((game.clone(), seat))
                .or_insert(input);
        }
        Ok(StandInSeat { game, seat })
    }
}

/// The table, as an aiSeat calls it: `{ internalToken }` params.
async fn call_table(game: &str, token: &str, action: &str, body: Value) {
    // TS sent these on a durable queue, which outlives a table that is going
    // to sleep. An action to a stopping table fails with `stopping`, so the
    // seat retries a few times, each on a fresh handle and with its own
    // bound (`requests/server-table-ai-seat-callback-retry.md`).
    let mut last = String::new();
    for attempt in 0..CALLBACK_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(CALLBACK_BACKOFF).await;
        }
        let Some(client) = CLIENT.get() else {
            last = format!("{action}: no client");
            break;
        };
        let handle = match client.get_or_create(
            ACTOR_NAME,
            actor_key(game).to_vec(),
            GetOrCreateOptions {
                params: Some(json!({ "internalToken": token })),
                ..Default::default()
            },
        ) {
            Ok(h) => h,
            Err(e) => {
                last = format!("{action}: {e:#}");
                continue;
            }
        };
        let call = handle.action(action, vec![body.clone()]);
        match tokio::time::timeout(CALLBACK_TIMEOUT, call).await {
            Ok(Ok(_)) => return,
            Ok(Err(e)) => last = format!("{action}: {e:#}"),
            Err(_) => last = format!("{action}: timed out"),
        }
        eprintln!("stand-in {action} for {game}, try {attempt}: {last}");
    }
    callback_failed(game, last);
}

/// What the stand-in answers, after acking `decide` (TS aiSeat: ack, decide, reply).
async fn answer(req: AIDecideRequest, mode: Mode, top: String, sleeps_at: u64) {
    let decision = |move_id: String, turn_id: String| AIDecision {
        internal_token: req.internal_token.clone(),
        game_id: req.game_id.clone(),
        seat: req.seat,
        turn_id,
        move_id,
        source: AiDecisionSource::Llm,
        banter: (mode.talk && req.seat.get() == 1).then(|| "Your deal.".to_owned()),
        rationale: None,
        latency_ms: 5,
    };
    let send = |d: AIDecision| async move {
        let body = serde_json::to_value(&d).unwrap_or(Value::Null);
        call_table(&d.game_id, &d.internal_token, "aiDecision", body).await;
    };
    let turn = req.turn_id.clone();
    match mode.answer {
        Answer::Prompt => send(decision(top.clone(), turn.clone())).await,
        Answer::Late(after) => {
            tokio::time::sleep(after).await;
            send(decision(top.clone(), turn.clone())).await;
        }
        Answer::Twice => {
            send(decision(top.clone(), turn.clone())).await;
            let other = req
                .legal
                .iter()
                .map(|m| m.id.clone())
                .find(|id| *id != top)
                .unwrap_or_else(|| top.clone());
            send(decision(other, turn.clone())).await;
        }
        Answer::WrongTurn => send(decision(top.clone(), "not-this-turn".into())).await,
        Answer::Illegal => send(decision("no-such-move".into(), turn.clone())).await,
        Answer::Never => {}
        Answer::AfterSleep => {
            let slept = eventually("table sleep", Duration::from_secs(10), || {
                let slept = test_support::sleeps(&req.game_id) > sleeps_at;
                (slept && RELEASED.lock().contains(&req.game_id)).then_some(())
            })
            .await;
            if slept.is_ok() {
                send(decision(top.clone(), turn.clone())).await;
            }
        }
    }
    if mode.talk && req.seat.get() == 2 {
        let say = |delta: Option<&str>, text: Option<&str>, is_final: bool| AiSayMsg {
            internal_token: req.internal_token.clone(),
            game_id: req.game_id.clone(),
            seat: req.seat,
            turn_id: turn.clone(),
            hand_no: Some(req.view.hand_no),
            msg_id: format!("{turn}:talk:2"),
            delta: delta.map(str::to_owned),
            text: text.map(str::to_owned),
            is_final,
        };
        for msg in [
            say(Some("Good "), None, false),
            say(None, Some("Good lead."), true),
        ] {
            let body = serde_json::to_value(&msg).unwrap_or(Value::Null);
            call_table(&req.game_id, &req.internal_token, "aiSay", body).await;
        }
    }
}

impl Handler<Decide> for StandInSeat {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: Decide) -> Result<()> {
        let req: AIDecideRequest = serde_json::from_value(a.body())?;
        let top = top_move(
            RankParams::new(&req.view)
                .with_hand(&req.view.hand)
                .with_legal(&req.legal),
        )
        .or_else(|| req.legal.first().map(|m| m.id.clone()))
        .unwrap_or_default();
        let mode = mode_of(&self.game);
        SEAT_LOG.lock().push(SeatCall {
            seq: seq(),
            game: self.game.clone(),
            seat: self.seat,
            action: "decide",
            hand_no: Some(req.view.hand_no),
            params_token: Self::params_token(&ctx),
            turn_id: Some(req.turn_id.clone()),
            top: Some(top.clone()),
            legal0: req.legal.first().map(|m| m.id.clone()),
            sleeps_at: test_support::sleeps(&self.game),
        });
        let sleeps_at = test_support::sleeps(&self.game);
        tokio::spawn(answer(req, mode, top, sleeps_at));
        Ok(())
    }
}

impl Handler<ResetHand> for StandInSeat {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: ResetHand) -> Result<()> {
        self.lifecycle(Self::params_token(&ctx), "resetHand", &a.body());
        Ok(())
    }
}

impl Handler<HandEnd> for StandInSeat {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: HandEnd) -> Result<()> {
        if mode_of(&self.game).slow_hand_end {
            tokio::time::sleep(SLOW_HAND_END).await;
        }
        // Recorded when it returns: a later message that overtook it shows first.
        self.lifecycle(Self::params_token(&ctx), "handEnd", &a.body());
        Ok(())
    }
}

impl Handler<GameEnd> for StandInSeat {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: GameEnd) -> Result<()> {
        self.lifecycle(Self::params_token(&ctx), "gameEnd", &a.body());
        Ok(())
    }
}

// --------------------------------------------------- the stand-in playerProfile

/// Stand-in `playerProfile`, key `["user", userId]`: records each body and
/// refuses a wrong token as the real one does.
struct StandInProfile {
    user: String,
}

impl StandInProfile {
    fn record(&self, action: &'static str, body: Value) -> Result<()> {
        if body["internalToken"] != INTERNAL {
            return Err(user_error(
                "internal_token_mismatch",
                "playerProfile accepts internal writes with the internal token only",
            ));
        }
        PROFILE_LOG.lock().push(ProfileCall {
            seq: seq(),
            user: self.user.clone(),
            action,
            body,
        });
        Ok(())
    }
}

#[async_trait]
impl Actor for StandInProfile {
    type Input = Value;
    type ConnParams = Value;
    type Action = rivetkit::Raw;
    type Actions = (RecordGame, RecordHands);
    type Events = ();

    async fn create(ctx: &Ctx<Self>) -> Result<Self> {
        Ok(StandInProfile {
            user: key_string(ctx.key(), 1),
        })
    }
}

impl Handler<RecordGame> for StandInProfile {
    async fn handle(self: Arc<Self>, _ctx: Ctx<Self>, a: RecordGame) -> Result<()> {
        self.record("recordGame", a.body())
    }
}

impl Handler<RecordHands> for StandInProfile {
    async fn handle(self: Arc<Self>, _ctx: Ctx<Self>, a: RecordHands) -> Result<()> {
        self.record("recordHands", a.body())
    }
}

// ------------------------------------------------------------- the player

macro_rules! table_event {
    ($ty:ident, $name:literal) => {
        #[derive(Serialize, Deserialize, Clone, Debug)]
        #[serde(transparent)]
        struct $ty(Value);
        impl Event for $ty {
            const NAME: &'static str = $name;
        }
    };
}

table_event!(SyncEv, "sync");
table_event!(ChatEv, "chat");
table_event!(ChatDeltaEv, "chatDelta");
table_event!(ThinkingEv, "thinking");

fn v_of(view: &Value) -> u64 {
    view["v"].as_u64().unwrap_or(0)
}

fn is_player_turn(view: &Value) -> bool {
    view["status"] == "active"
        && view["turnSeat"] == 0
        && view["legal"].as_array().is_some_and(|l| !l.is_empty())
}

fn ai_turn_seat(view: &Value) -> Option<u64> {
    let seat = view["turnSeat"].as_u64()?;
    (view["status"] == "active" && (1..=3).contains(&seat)).then_some(seat)
}

/// The seat-0 WebSocket with every table event it receives.
struct Player {
    conn: TypedActorConnection<EuchreTable>,
    syncs: UnboundedReceiver<SyncEv>,
    chats: UnboundedReceiver<ChatEv>,
    deltas: UnboundedReceiver<ChatDeltaEv>,
    thinking: UnboundedReceiver<ThinkingEv>,
    _subs: Vec<SubscriptionHandle>,
    last: Option<Value>,
    moves: u64,
    /// Wall time of every finished AI turn, from the `sync` that opened it.
    ai_turns: Vec<Duration>,
    open_ai_turn: Option<(String, Instant)>,
}

impl Player {
    async fn open(h: &TypedActorHandle<EuchreTable>) -> Result<Player> {
        let conn = h.connect();
        let (s1, syncs) = forward_events::<EuchreTable, SyncEv>(&conn).await;
        let (s2, chats) = forward_events::<EuchreTable, ChatEv>(&conn).await;
        let (s3, deltas) = forward_events::<EuchreTable, ChatDeltaEv>(&conn).await;
        let (s4, thinking) = forward_events::<EuchreTable, ThinkingEv>(&conn).await;
        let mut status = conn.inner().status_receiver();
        tokio::time::timeout(CALL, status.wait_for(|s| *s == ConnectionStatus::Connected))
            .await
            .context("connect timed out")??;
        Ok(Player {
            conn,
            syncs,
            chats,
            deltas,
            thinking,
            _subs: vec![s1, s2, s3, s4],
            last: None,
            moves: 0,
            ai_turns: Vec::new(),
            open_ai_turn: None,
        })
    }

    fn absorb(&mut self, ev: SyncEv) {
        let view = ev.0["view"].clone();
        let turn = view["turnId"].as_str().unwrap_or("").to_owned();
        if let Some((open, at)) = &self.open_ai_turn
            && *open != turn
        {
            self.ai_turns.push(at.elapsed());
            self.open_ai_turn = None;
        }
        if self.open_ai_turn.is_none() && ai_turn_seat(&view).is_some() {
            self.open_ai_turn = Some((turn, Instant::now()));
        }
        self.last = Some(view);
    }

    /// The newest view after at least one more `sync`.
    async fn next_view(&mut self, what: &str) -> Result<Value> {
        let ev = match tokio::time::timeout(EVENT, self.syncs.recv()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => bail!("{what}: sync channel closed"),
            Err(_) => bail!("{what}: no sync within {EVENT:?}"),
        };
        self.absorb(ev);
        while let Ok(ev) = self.syncs.try_recv() {
            self.absorb(ev);
        }
        self.last.clone().context("no view")
    }

    /// Play the first legal move on every seat-0 turn until `stop` or game over.
    async fn play_until(&mut self, what: &str, stop: impl Fn(&Value) -> bool) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(240);
        let mut view = match self.last.clone() {
            Some(v) => v,
            None => self.next_view(what).await?,
        };
        loop {
            ensure!(Instant::now() < deadline, "{what}: timed out");
            if stop(&view) || view["status"] != "active" {
                return Ok(view);
            }
            if is_player_turn(&view) {
                let id = view["legal"][0]["id"]
                    .as_str()
                    .context("legal id")?
                    .to_owned();
                self.moves += 1;
                let cmid = format!("c-{}-{}", self.moves, v_of(&view));
                let submit = SubmitMove(vec![json!({
                    "moveId": id,
                    "turnId": view["turnId"],
                    "clientMoveId": cmid,
                })]);
                let ack = call_with_timeout(&self.conn, submit, CALL)
                    .await
                    .with_context(|| format!("{what}: submit {id}"))?;
                ensure!(
                    matches!(ack, MoveAck::Ok { .. }),
                    "{what}: move {id} refused: {ack:?}"
                );
            }
            view = self.next_view(what).await?;
        }
    }

    fn drain<E>(rx: &mut UnboundedReceiver<E>) -> Vec<E> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }
}

/// Non-forced AI moves of the current hand, in order: `(seat, id, source)`.
fn asked_ai_moves(game: &str) -> Vec<(u64, String, String)> {
    let Ok(st) = state_of(game) else {
        return Vec::new();
    };
    st["journal"]
        .as_array()
        .map(|j| {
            j.iter()
                .filter(|e| e["kind"] == "ai" && e["source"] != "forced")
                .map(|e| {
                    (
                        e["seat"].as_u64().unwrap_or(0),
                        e["id"].as_str().unwrap_or("").to_owned(),
                        e["source"].as_str().unwrap_or("").to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn open_match(
    engine: &TestEngine,
    game: &str,
    user: &str,
    mode: Mode,
) -> Result<(TypedActorHandle<EuchreTable>, Player)> {
    MODES.lock().insert(game.to_owned(), mode);
    let h = table(&engine.client(), game, user)?;
    engine
        .ready(|| async { h.call(Snapshot(vec![])).await })
        .await?;
    let player = Player::open(&h).await?;
    Ok((h, player))
}

// ------------------------------------------------------------------ tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn table_ai_full_match_end_to_end() -> Result<()> {
    run(
        "table_ai_full_match_end_to_end",
        true,
        |engine| async move {
            let r = full_match(&engine).await;
            (engine, r)
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn table_ai_misbehaving_seats_end_to_end() -> Result<()> {
    run(
        "table_ai_misbehaving_seats_end_to_end",
        true,
        |engine| async move {
            let r = misbehaving(&engine).await;
            (engine, r)
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn table_ai_sleep_between_dispatch_and_decision_end_to_end() -> Result<()> {
    run(
        "table_ai_sleep_between_dispatch_and_decision_end_to_end",
        true,
        |engine| async move {
            let r = sleep_between(&engine).await;
            (engine, r)
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn table_ai_missing_seat_end_to_end() -> Result<()> {
    run(
        "table_ai_missing_seat_end_to_end",
        false,
        |engine| async move {
            let r = missing_seat(&engine).await;
            (engine, r)
        },
    )
    .await
}

async fn full_match(engine: &TestEngine) -> Result<()> {
    let game = game_id("aimatch");
    let alice = guest();
    let mode = Mode {
        answer: Answer::Prompt,
        talk: true,
        slow_hand_end: true,
    };
    let (_h, mut player) = open_match(engine, &game, &alice, mode).await?;
    let started = Instant::now();
    let end = player.play_until("full match", |_| false).await?;
    ensure!(
        end["status"] == "complete",
        "match ended as {}",
        end["status"]
    );
    println!(
        "full match: {} player moves, {} AI turns, {:?}",
        player.moves,
        player.ai_turns.len(),
        started.elapsed()
    );

    // recordGame lands once; the table settles it and arms the reap.
    let st = eventually("recorded", Duration::from_secs(15), || {
        state_of(&game).ok().filter(|s| !s["recordedAt"].is_null())
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let games = profile_calls(&alice, "recordGame");
    ensure!(games.len() == 1, "recordGame calls: {}", games.len());
    let body = &games[0].body;
    let keys: Vec<&String> = body
        .as_object()
        .map(|o| o.keys().collect())
        .unwrap_or_default();
    ensure!(
        keys == ["internalToken", "record"],
        "recordGame keys {keys:?}"
    );
    let parsed = euchre_profile_model::parse_record_game(body, &alice, now_ms())
        .map_err(|e| anyhow::anyhow!("playerProfile refuses recordGame: {e:?}"))?;
    let record = &body["record"];
    ensure!(
        parsed.record.match_id == game
            && record["userId"] == alice.as_str()
            && record["status"] == "complete"
            && record["handsPlayed"] == st["handsPlayed"]
            && record["winnerTeam"] == st["game"]["winnerTeam"]
            && record["outcome"]
                == if st["game"]["winnerTeam"] == 0 {
                    "won"
                } else {
                    "lost"
                },
        "record: {record}"
    );
    ensure!(
        st["ledger"]["reapAt"].as_u64().is_some() && st["ledger"].get("profileRetryAt").is_none(),
        "ledger after the record: {}",
        st["ledger"]
    );

    // recordHands: one call per finished hand (throw-ins included), all before recordGame.
    let hands = profile_calls(&alice, "recordHands");
    let expected =
        st["handsPlayed"].as_u64().unwrap_or(0) + st["stats"]["throwIns"].as_u64().unwrap_or(0);
    ensure!(
        hands.len() as u64 == expected,
        "recordHands calls {} for {expected} hands",
        hands.len()
    );
    let mut hand_nos = Vec::new();
    let (mut llm, mut fallback) = (0, 0);
    for call in &hands {
        let keys: Vec<&String> = call
            .body
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        ensure!(
            keys == ["internalToken", "matchId", "hands"],
            "recordHands keys {keys:?}"
        );
        let rows = euchre_profile_model::parse_record_hands(&call.body)
            .map_err(|e| anyhow::anyhow!("playerProfile refuses recordHands: {e:?}"))?;
        ensure!(rows.len() == 1, "one hand per call");
        ensure!(call.seq < games[0].seq, "recordHands after recordGame");
        let entry = &call.body["hands"][0];
        hand_nos.push(entry["handNo"].as_u64().unwrap_or(0));
        for m in entry["moves"].as_array().into_iter().flatten() {
            match (m["kind"].as_str(), m["source"].as_str()) {
                (Some("ai"), Some("llm")) => llm += 1,
                (Some("ai"), Some("fallback")) => fallback += 1,
                (Some("auto"), _) => bail!("a seat was auto-played: {m}"),
                _ => {}
            }
        }
    }
    let mut sorted = hand_nos.clone();
    sorted.dedup();
    ensure!(
        sorted == hand_nos && hand_nos.windows(2).all(|w| w[0] < w[1]),
        "hand numbers {hand_nos:?}"
    );
    println!("AI moves from the stand-in seats: {llm} llm, {fallback} fallback");
    ensure!(
        llm > 0 && llm >= fallback * 4,
        "llm {llm} vs fallback {fallback}"
    );

    // Each seat: params and create input, then the ordered lifecycle.
    let token = test_support::internal_token(&game).context("match token")?;
    let calls = seat_calls(&game);
    for seat in 1..=3u8 {
        let mine: Vec<&SeatCall> = calls.iter().filter(|c| c.seat == seat).collect();
        ensure!(
            mine.iter()
                .all(|c| c.params_token.as_deref() == Some(token.as_str())),
            "seat {seat}: a call without the match token"
        );
        let input = SEAT_INPUTS
            .lock()
            .get(&(game.clone(), seat))
            .cloned()
            .with_context(|| format!("seat {seat}: no create input"))?;
        let keys: Vec<&String> = input
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        ensure!(
            keys[..7]
                == [
                    "gameId",
                    "seat",
                    "internalToken",
                    "fenceNonce",
                    "persona",
                    "dossier",
                    "matchId"
                ],
            "seat {seat} create input keys {keys:?}"
        );
        if let Some(pk) = input.get("profileKey") {
            ensure!(*pk == json!(["user", alice]), "profileKey {pk}");
        }
        check_seat_order(seat, &mine)?;
        let ends: Vec<&&SeatCall> = mine.iter().filter(|c| c.action == "gameEnd").collect();
        ensure!(ends.len() == 1, "seat {seat}: {} gameEnd", ends.len());
        ensure!(
            ends[0].seq > games[0].seq,
            "seat {seat}: gameEnd before recordGame"
        );
    }

    // Chat: aiSay deltas and final lines, and banter on a decision.
    let chats: Vec<Value> = Player::drain(&mut player.chats)
        .into_iter()
        .map(|c| c.0)
        .collect();
    let deltas: Vec<Value> = Player::drain(&mut player.deltas)
        .into_iter()
        .map(|c| c.0)
        .collect();
    let talk = chats
        .iter()
        .filter(|c| {
            c["seat"] == 2
                && c["kind"] == "banter"
                && c["msgId"].as_str().is_some_and(|m| m.ends_with(":talk:2"))
        })
        .count();
    let banter = chats
        .iter()
        .filter(|c| c["seat"] == 1 && c["kind"] == "banter" && c["text"] == "Your deal.")
        .count();
    let streamed = deltas
        .iter()
        .filter(|d| d["seat"] == 2 && d["delta"] == "Good")
        .count();
    println!("chat: {talk} aiSay lines, {streamed} deltas, {banter} banter lines");
    ensure!(
        talk > 0 && streamed > 0 && banter > 0,
        "chat {talk}/{streamed}/{banter}"
    );
    ensure!(
        chats.iter().all(|c| c["final"] == true),
        "chat events are final"
    );

    // thinking: on/off edges only, never extended (D23).
    let thinking: Vec<Value> = Player::drain(&mut player.thinking)
        .into_iter()
        .map(|t| t.0)
        .collect();
    ensure!(!thinking.is_empty(), "no thinking events");
    ensure!(
        thinking.iter().all(|t| t["extended"] == false),
        "extended thinking was emitted"
    );
    let on = thinking.iter().filter(|t| t["on"] == true).count();
    let off = thinking.iter().filter(|t| t["on"] == false).count();
    // TS: on when a seat is asked; off after every AI move (forced ones too)
    // and when the watchdog fires.
    ensure!(on > 0 && off >= on, "thinking on {on} vs off {off}");
    let failed = callback_errors(&game);
    ensure!(failed.is_empty(), "seat callbacks failed: {failed:?}");
    Ok(())
}

/// One seat's calls: every `decide` of hand N lands after `resetHand(N)` and
/// before `handEnd(N)`; `handEnd(N)` (slow) lands before `resetHand(N+1)`;
/// `gameEnd` is last.
fn check_seat_order(seat: u8, calls: &[&SeatCall]) -> Result<()> {
    let mut hand: Option<u32> = None;
    let mut ended = true;
    for (i, c) in calls.iter().enumerate() {
        match c.action {
            "resetHand" => {
                ensure!(
                    ended,
                    "seat {seat}: resetHand {:?} before handEnd {hand:?}",
                    c.hand_no
                );
                ensure!(
                    c.hand_no > hand || hand.is_none(),
                    "seat {seat}: resetHand went back"
                );
                hand = c.hand_no;
                ended = false;
            }
            "decide" => ensure!(
                !ended && c.hand_no == hand,
                "seat {seat}: decide for hand {:?} while at {hand:?} (ended {ended})",
                c.hand_no
            ),
            "handEnd" => {
                ensure!(
                    c.hand_no == hand,
                    "seat {seat}: handEnd {:?} at {hand:?}",
                    c.hand_no
                );
                ended = true;
            }
            "gameEnd" => ensure!(i + 1 == calls.len(), "seat {seat}: gameEnd is not last"),
            other => bail!("seat {seat}: unexpected {other}"),
        }
    }
    Ok(())
}

/// The first few asked AI turns of one misbehaving match.
async fn misbehaving_one(engine: &TestEngine, answer: Answer) -> Result<String> {
    const TURNS: usize = 3;
    let game = game_id("aibad");
    let user = guest();
    let (_h, mut p) = open_match(
        engine,
        &game,
        &user,
        Mode {
            answer,
            ..Mode::default()
        },
    )
    .await?;
    let what = format!("{answer:?}");
    let g = game.clone();
    p.play_until(&what, |_| asked_ai_moves(&g).len() >= TURNS)
        .await?;
    let moves = asked_ai_moves(&game);
    ensure!(
        moves.len() >= TURNS,
        "{what}: only {} asked AI moves",
        moves.len()
    );
    let decides: Vec<SeatCall> = seat_calls(&game)
        .into_iter()
        .filter(|c| c.action == "decide")
        .collect();
    ensure!(
        decides.len() >= moves.len(),
        "{what}: {} decides for {} moves",
        decides.len(),
        moves.len()
    );
    for (i, ((seat, id, source), asked)) in moves.iter().zip(&decides).enumerate() {
        ensure!(
            u64::from(asked.seat) == *seat,
            "{what} #{i}: seat {seat} vs asked {}",
            asked.seat
        );
        let (want_id, want_source) = match answer {
            Answer::Prompt | Answer::Twice => (asked.top.clone(), "llm"),
            Answer::Illegal => (asked.legal0.clone(), "fallback"),
            _ => (asked.top.clone(), "fallback"),
        };
        ensure!(
            Some(id) == want_id.as_ref() && source == want_source,
            "{what} #{i}: played {id} ({source}), wanted {want_id:?} ({want_source})"
        );
    }
    // Never stalled: a slow seat costs at most the hard cap (plus slack).
    let slowest = p.ai_turns.iter().max().copied().unwrap_or_default();
    ensure!(
        slowest < HARD_CAP + Duration::from_millis(1500),
        "{what}: an AI turn took {slowest:?}"
    );
    if matches!(answer, Answer::Never | Answer::WrongTurn | Answer::Late(_)) {
        ensure!(
            slowest + Duration::from_millis(200) >= HARD_CAP,
            "{what}: the fallback did not wait for the cap ({slowest:?})"
        );
    }
    // One move per turn: the journal has no more AI moves than turns asked.
    let unique: std::collections::HashSet<&str> = decides
        .iter()
        .filter_map(|d| d.turn_id.as_deref())
        .collect();
    ensure!(
        unique.len() == decides.len(),
        "{what}: a turn was asked twice"
    );
    Ok(format!(
        "{what}: {} asked moves, slowest AI turn {slowest:?}",
        moves.len()
    ))
}

async fn misbehaving(engine: &TestEngine) -> Result<()> {
    let late = Answer::Late(HARD_CAP + Duration::from_millis(800));
    let results = futures_join([
        misbehaving_one(engine, late),
        misbehaving_one(engine, Answer::Twice),
        misbehaving_one(engine, Answer::WrongTurn),
        misbehaving_one(engine, Answer::Illegal),
        misbehaving_one(engine, Answer::Never),
    ])
    .await;
    for r in results {
        println!("{}", r?);
    }
    Ok(())
}

/// `join_all` over a fixed array without a `futures` dependency.
async fn futures_join<F: std::future::Future>(futs: [F; 5]) -> [F::Output; 5] {
    let [a, b, c, d, e] = futs;
    let (a, b, c, d, e) = tokio::join!(a, b, c, d, e);
    [a, b, c, d, e]
}

async fn sleep_between(engine: &TestEngine) -> Result<()> {
    let game = game_id("aisleep");
    let user = guest();
    MODES.lock().insert(
        game.clone(),
        Mode {
            answer: Answer::AfterSleep,
            ..Mode::default()
        },
    );
    // Armed before the table exists: it sleeps right after its first `decide`.
    test_support::request_sleep_after_dispatch(&game);
    let h = table(&engine.client(), &game, &user)?;
    engine
        .ready(|| async { h.call(Snapshot(vec![])).await })
        .await?;
    // HTTP only (no socket keeps it up): play seat 0 until the first decide.
    // On an AI turn the test stops calling the table, so nothing but the
    // seat's answer (or the watchdog) can wake it once it sleeps.
    let decided = || seat_calls(&game).into_iter().find(|c| c.action == "decide");
    let mut n = 0;
    let first = loop {
        if let Some(d) = decided() {
            break d;
        }
        ensure!(n < 400, "no decide reached the stand-in seat");
        n += 1;
        let view = stateless_call_with_timeout(&h, Snapshot(vec![]), CALL).await?;
        if is_player_turn(&view) {
            let id = view["legal"][0]["id"].as_str().context("legal")?.to_owned();
            let submit = SubmitMove(vec![json!({
                "moveId": id, "turnId": view["turnId"], "clientMoveId": format!("s-{n}"),
            })]);
            stateless_call_with_timeout(&h, submit, CALL).await?;
        }
        // Hands off the table while the AI seats play (forced ones too).
        if let Ok(d) = eventually("decide", Duration::from_secs(3), decided).await {
            break d;
        }
    };
    eventually("the table slept", Duration::from_secs(10), || {
        (test_support::sleeps(&game) > first.sleeps_at).then_some(())
    })
    .await?;
    let asleep = test_support::starts(&game);
    RELEASED.lock().insert(game.clone());
    eventually("the answer woke the table", Duration::from_secs(15), || {
        (test_support::starts(&game) > asleep).then_some(())
    })
    .await?;
    let turn = first.turn_id.clone().context("turn")?;
    let st = eventually("the asked turn played", Duration::from_secs(15), || {
        state_of(&game)
            .ok()
            .filter(|s| s["game"]["turnId"] != turn.as_str())
    })
    .await?;
    let moved: Vec<&Value> = st["journal"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["kind"] == "ai" && e["source"] != "forced")
        .collect();
    ensure!(moved.len() == 1, "asked AI moves after the wake: {moved:?}");
    let source = moved[0]["source"].as_str().unwrap_or("");
    println!(
        "sleep between dispatch and decision: sleeps {}, starts {}, played {} ({source})",
        test_support::sleeps(&game),
        test_support::starts(&game),
        moved[0]["id"]
    );
    ensure!(
        moved[0]["id"].as_str() == first.top.as_deref(),
        "played {} instead of the asked move {:?}",
        moved[0]["id"],
        first.top
    );
    // `llm` when the answer and the wake beat the 1.5 s fast-tempo cap (most
    // runs). Under load the wake can take longer; the table then plays the
    // parked move as `fallback` at the deadline. Either way it plays once.
    ensure!(
        source == "llm" || source == "fallback",
        "unexpected source {source}"
    );
    let failed = callback_errors(&game);
    ensure!(failed.is_empty(), "seat callbacks failed: {failed:?}");
    // And the game goes on from there.
    let mut p = Player::open(&h).await?;
    let g = game.clone();
    p.play_until("after the wake", |_| asked_ai_moves(&g).len() >= 3)
        .await?;
    Ok(())
}

async fn missing_seat(engine: &TestEngine) -> Result<()> {
    const TURNS: usize = 3;
    let game = game_id("ainoseat");
    let user = guest();
    let (_h, mut p) = open_match(engine, &game, &user, Mode::default()).await?;
    let g = game.clone();
    p.play_until("missing seat", |_| asked_ai_moves(&g).len() >= TURNS)
        .await?;
    let moves = asked_ai_moves(&game);
    ensure!(
        moves.iter().all(|(_, _, s)| s == "fallback"),
        "moves without a seat: {moves:?}"
    );
    ensure!(seat_calls(&game).is_empty(), "a seat answered");
    let slowest = p.ai_turns.iter().max().copied().unwrap_or_default();
    println!(
        "missing seat: {} AI turns, slowest {slowest:?}",
        p.ai_turns.len()
    );
    ensure!(
        slowest < HARD_CAP + Duration::from_millis(1500),
        "an AI turn took {slowest:?}"
    );
    Ok(())
}
