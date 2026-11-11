//! Engine-backed tests of the `aiSeat` actor (lane SERVER-AI-SEAT).
//!
//! Each test prints `SKIP` and returns unless `EUCHRE_E2E=1`, so a plain
//! `cargo test` stays engine-free. Run:
//!
//! ```text
//! EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6771 EUCHRE_TEST_STORAGE=<dir> \
//!   cargo test -p euchre-ai-seat --test ai_seat_e2e -- --nocapture
//! ```
//!
//! The other actors are stand-ins defined here with the real registry and
//! action names (D29): `euchreTable` relays `decide` and the lifecycle
//! messages to the seat through its own `ctx.client()` (as 03 §16.7 says the
//! table does) and records every `aiDecision` / `aiSay` it receives;
//! `playerProfile` records `recordEpisodes` / `recordTokens`. The bodies it
//! records are fed to the real profile parsers (`euchre_profile_model`).
//!
//! Coverage:
//!
//! - `decide_end_to_end`: a heuristic `decide` answered through `aiDecision`
//!   before its deadline; a caller without the token refused at connect and on
//!   reads; a forged body token, another seat, another match and a redelivered
//!   turn all dropped with the TS counters; reads; idle sleep and a wake that
//!   keeps the per-hand memory and the last answered turn.
//! - `lifecycle_end_to_end`: `handEnd` flushes the hand's episode to
//!   `recordEpisodes` (accepted by the profile parser); `resetHand` moves the
//!   memory to the next hand; `gameEnd` with no model spend sends no
//!   `recordTokens`, as TS.
//! - `scripted_llm_end_to_end`: a scripted Jev decision client and talk model
//!   (no network) drive an `llm` decision, one line of table talk through
//!   `aiSay`, and the `recordTokens` flush at `gameEnd`.
//! - `out_of_order_end_to_end`: the next hand's `decide` lands before this
//!   hand's `handEnd` and `resetHand` (the table's detached sends); both are
//!   dropped and logged, and the next `handEnd` notes the right hand.
//! - `failure_paths_end_to_end`: a hung `aiDecision` is cut at the call
//!   timeout and the next decide is still answered; a failed `aiSay` logs the
//!   TS warning; a failed `recordEpisodes` keeps the buffer and the next
//!   boundary resends and clears it.
//! - `sleep_grace_end_to_end`: a seat asked to sleep while table talk holds
//!   grace open still handles a `handEnd` and a `decide` sent during grace.

use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use async_trait::async_trait;
use euchre_ai::ai_seat::bridge::{LlmProvider, ProviderDeps};
use euchre_ai::ai_seat::handlers::messages;
use euchre_ai::ai_seat::state::AiSeatCreateInput;
use euchre_ai::clock::SystemClock;
use euchre_ai::clock::system_now_ms;
use euchre_ai::testing::{
    RecordingLog, ScriptedBanter, ScriptedJev, ScriptedResult, ScriptedTalk, jev_choice,
    jev_factory,
};
use euchre_ai_seat::actor::key_strings;
use euchre_ai_seat::{
    ACTOR_NAME, AiSeat, AiSeatRuntime, GetDecisionLog, GetPersona, GetStatus, SeatClient,
    actor_key, register_with, test_support,
};
use euchre_engine::{
    GamePhase, GameState, GameStatus, PublicGameView, Seat, apply, legal_moves, new_game, project,
    try_advance,
};
use euchre_profile_model::{TokenIncrement, body_internal_token, parse_record_episodes};
use euchre_protocol::{AIDecideRequest, AIDecisionKind, PersonaConfig, default_personas};
use euchre_server_core::rivet::{parse_rivet_error, stateless_call_with_timeout};
use euchre_server_core::testing::{TestEngine, TestEngineOptions, e2e_enabled, e2e_skip};
use rivetkit::client::GetOrCreateOptions;
use rivetkit::prelude::*;
use rivetkit::{ActorConfig, TypedActorHandle, TypedClientExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ------------------------------------------------------------------ fixtures

const TOKEN: &str = "e2e-table-token-0123456789abcdef";
const PROFILE_TOKEN: &str = "e2e-profile-internal-0123456789ab";
const CALL: Duration = Duration::from_secs(15);
const WAIT: Duration = Duration::from_secs(20);

fn unique(tag: &str) -> String {
    format!("{tag}_{}", uuid::Uuid::new_v4().simple())
}

fn seat_of(n: u8) -> Result<Seat> {
    Seat::new(n).context("seat")
}

fn create_input(
    game_id: &str,
    seat: u8,
    user: &str,
    persona: PersonaConfig,
) -> Result<AiSeatCreateInput> {
    Ok(AiSeatCreateInput {
        game_id: game_id.to_owned(),
        seat: seat_of(seat)?,
        internal_token: TOKEN.to_owned(),
        fence_nonce: "e2e-nonce".to_owned(),
        persona,
        dossier: None,
        profile_key: Some(vec!["user".to_owned(), user.to_owned()]),
        match_id: Some(format!("m-{game_id}")),
    })
}

fn kind_for(phase: GamePhase) -> Option<AIDecisionKind> {
    match phase {
        GamePhase::Cutting => Some(AIDecisionKind::Cut),
        GamePhase::BidRound1 => Some(AIDecisionKind::Bid1),
        GamePhase::DealerDiscard => Some(AIDecisionKind::Discard),
        GamePhase::BidRound2 => Some(AIDecisionKind::Bid2),
        GamePhase::TrickPlay => Some(AIDecisionKind::Play),
        _ => None,
    }
}

/// Walk a seeded match, every seat taking its first legal move, until an AI
/// seat (1..3) is on turn and `want` accepts the position. Returns its view.
fn find_view(
    seed: &str,
    want: impl Fn(&GameState, &PublicGameView) -> bool,
) -> Result<(PublicGameView, AIDecisionKind)> {
    let mut s = new_game(seed, None);
    for _ in 0..2000 {
        if s.status != GameStatus::Active && s.hand.phase == GamePhase::GameOver {
            bail!("match ended before a wanted position");
        }
        let Some(turn) = s.hand.turn_seat else {
            s = try_advance(&s).map_err(|e| anyhow::anyhow!("{e}"))?.state;
            continue;
        };
        let legal = legal_moves(&s, turn);
        if turn.get() != 0 {
            let mut view = project(&s, turn);
            view.legal = legal.clone();
            if let Some(kind) = kind_for(s.hand.phase)
                && want(&s, &view)
            {
                return Ok((view, kind));
            }
        }
        let first = legal.first().context("no legal move")?;
        s = apply(&s, turn, first.mv, None)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?
            .state;
    }
    bail!("no wanted position within 2000 steps")
}

fn decide_request(
    game_id: &str,
    view: &PublicGameView,
    kind: AIDecisionKind,
    turn_id: &str,
) -> AIDecideRequest {
    AIDecideRequest {
        internal_token: TOKEN.to_owned(),
        game_id: game_id.to_owned(),
        seat: view.you,
        turn_id: turn_id.to_owned(),
        kind,
        view: view.clone(),
        legal: view.legal.clone(),
        ranking: None,
        deadline_at: system_now_ms() + 30_000,
    }
}

fn lifecycle(game_id: &str, kind: &str, hand_no: Option<u32>) -> Value {
    let mut body = json!({ "internalToken": TOKEN, "gameId": game_id, "kind": kind });
    if let (Some(n), Some(map)) = (hand_no, body.as_object_mut()) {
        map.insert("handNo".into(), json!(n));
    }
    body
}

fn seat_params() -> Value {
    json!({ "internalToken": TOKEN })
}

// ------------------------------------------------------------------ recorder

/// One call a stand-in received.
#[derive(Clone, Debug)]
struct Received {
    key: Vec<String>,
    action: &'static str,
    body: Value,
    params: Value,
}

static RECEIVED: LazyLock<parking_lot::Mutex<Vec<Received>>> =
    LazyLock::new(|| parking_lot::Mutex::new(Vec::new()));

fn record<A: Actor<ConnParams = Value>>(ctx: &Ctx<A>, action: &'static str, args: &[Value]) {
    let params = ctx
        .conn()
        .and_then(|c| c.params().ok())
        .unwrap_or(Value::Null);
    RECEIVED.lock().push(Received {
        key: key_strings(ctx.key()),
        action,
        body: args.first().cloned().unwrap_or(Value::Null),
        params,
    });
}

/// Every call `action` received on the actor keyed `key`.
fn received(key: &[String], action: &str) -> Vec<Received> {
    RECEIVED
        .lock()
        .iter()
        .filter(|r| r.key == key && r.action == action)
        .cloned()
        .collect()
}

/// Poll until `f` returns `Some`, or fail after `within`.
async fn wait_for<T>(what: &str, within: Duration, f: impl Fn() -> Option<T>) -> Result<T> {
    let start = Instant::now();
    loop {
        if let Some(v) = f() {
            return Ok(v);
        }
        if start.elapsed() > within {
            bail!("timed out after {within:?} waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Wait for the `n`th (1-based) call of `action` on `key`.
async fn nth(key: &[String], action: &str, n: usize) -> Result<Received> {
    wait_for(&format!("{action} #{n} on {key:?}"), WAIT, || {
        received(key, action).get(n - 1).cloned()
    })
    .await
}

// ------------------------------------------------------------------ modes

/// How a stand-in answers one action for one actor id (the game id for the
/// table, the user id for the profile).
#[derive(Clone, Copy, Debug, Default)]
struct Mode {
    /// Sleep this long before answering, this many times.
    hang: Option<(Duration, u32)>,
    /// Fail this many times (`u32::MAX`: always).
    fail: u32,
}

static MODES: LazyLock<
    parking_lot::Mutex<std::collections::HashMap<(String, &'static str), Mode>>,
> = LazyLock::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));

fn set_mode(id: &str, action: &'static str, mode: Mode) {
    MODES.lock().insert((id.to_owned(), action), mode);
}

/// Apply the mode for `action` on the actor keyed `[_, id]`: hang, then fail.
async fn apply_mode<A: Actor>(ctx: &Ctx<A>, action: &'static str) -> Result<()> {
    let id = key_strings(ctx.key()).get(1).cloned().unwrap_or_default();
    let (hang, fail) = {
        let mut modes = MODES.lock();
        let Some(mode) = modes.get_mut(&(id, action)) else {
            return Ok(());
        };
        let hang = match mode.hang {
            Some((d, n)) if n > 0 => {
                mode.hang = Some((d, n - 1));
                Some(d)
            }
            _ => None,
        };
        let fail = mode.fail > 0;
        if fail && mode.fail != u32::MAX {
            mode.fail -= 1;
        }
        (hang, fail)
    };
    if let Some(d) = hang {
        tokio::time::sleep(d).await;
    }
    if fail {
        bail!("stand-in {action} failure");
    }
    Ok(())
}

// ------------------------------------------------------------------ stand-ins

macro_rules! raw_action {
    ($ty:ident, $name:literal, $out:ty) => {
        #[derive(Serialize, Deserialize, Clone, Debug, Default)]
        struct $ty(Vec<Value>);
        impl Action for $ty {
            type Output = $out;
            const NAME: &'static str = $name;
        }
    };
}

raw_action!(Relay, "relay", Value);
raw_action!(AiDecisionIn, "aiDecision", ());
raw_action!(AiSayIn, "aiSay", ());
raw_action!(Ping, "ping", String);
raw_action!(RecordEpisodesIn, "recordEpisodes", ());
raw_action!(RecordTokensIn, "recordTokens", ());

/// Stand-in `euchreTable`, key `["table", gameId]`.
struct StandInTable {
    game_id: String,
}

#[async_trait]
impl Actor for StandInTable {
    type Input = Value;
    type ConnParams = Value;
    type Action = rivetkit::Raw;
    type Actions = (Relay, AiDecisionIn, AiSayIn, Ping);
    type Events = ();

    async fn create(ctx: &Ctx<Self>) -> Result<Self> {
        let key = key_strings(ctx.key());
        Ok(Self {
            game_id: key.get(1).cloned().unwrap_or_default(),
        })
    }
}

impl Handler<Ping> for StandInTable {
    async fn handle(self: Arc<Self>, _ctx: Ctx<Self>, _a: Ping) -> Result<String> {
        Ok("pong".into())
    }
}

/// `relay(seat, action, body, createInput, params)`: call the seat the way the
/// table does and report `{ ok, value }` or `{ ok: false, error }`.
impl Handler<Relay> for StandInTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: Relay) -> Result<Value> {
        let arg = |i: usize| a.0.get(i).cloned().unwrap_or(Value::Null);
        let seat = arg(0).as_u64().context("seat")?;
        let action = arg(1).as_str().context("action")?.to_owned();
        let seat = u8::try_from(seat)?;
        let client = ctx.client()?;
        let handle = client.get_or_create(
            ACTOR_NAME,
            actor_key(&self.game_id, seat),
            GetOrCreateOptions {
                params: Some(arg(4)),
                create_with_input: Some(arg(3)),
                ..Default::default()
            },
        )?;
        Ok(
            match tokio::time::timeout(CALL, handle.action(&action, vec![arg(2)])).await {
                Ok(Ok(value)) => json!({ "ok": true, "value": value }),
                Ok(Err(e)) => json!({ "ok": false, "error": format!("{e:#}") }),
                Err(_) => json!({ "ok": false, "error": "timeout" }),
            },
        )
    }
}

impl Handler<AiDecisionIn> for StandInTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: AiDecisionIn) -> Result<()> {
        record(&ctx, AiDecisionIn::NAME, &a.0);
        apply_mode(&ctx, AiDecisionIn::NAME).await
    }
}

impl Handler<AiSayIn> for StandInTable {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: AiSayIn) -> Result<()> {
        record(&ctx, AiSayIn::NAME, &a.0);
        apply_mode(&ctx, AiSayIn::NAME).await
    }
}

/// Stand-in `playerProfile`, key `["user", userId]`.
struct StandInProfile;

#[async_trait]
impl Actor for StandInProfile {
    type Input = Value;
    type ConnParams = Value;
    type Action = rivetkit::Raw;
    type Actions = (RecordEpisodesIn, RecordTokensIn);
    type Events = ();

    async fn create(_ctx: &Ctx<Self>) -> Result<Self> {
        Ok(Self)
    }
}

impl Handler<RecordEpisodesIn> for StandInProfile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: RecordEpisodesIn) -> Result<()> {
        record(&ctx, RecordEpisodesIn::NAME, &a.0);
        apply_mode(&ctx, RecordEpisodesIn::NAME).await
    }
}

impl Handler<RecordTokensIn> for StandInProfile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: RecordTokensIn) -> Result<()> {
        record(&ctx, RecordTokensIn::NAME, &a.0);
        apply_mode(&ctx, RecordTokensIn::NAME).await
    }
}

// ------------------------------------------------------------------ harness

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_test_writer()
        .try_init();
}

/// One engine pool serving the seat (with `runtime`) and both stand-ins.
async fn start_engine(runtime: AiSeatRuntime) -> Result<TestEngine> {
    test_support::enable();
    let client = runtime.client().clone();
    let engine = TestEngine::start(TestEngineOptions::from_env(move |registry| {
        register_with(
            registry,
            runtime,
            ActorConfig {
                sleep_timeout: Duration::from_secs(2),
                ..ActorConfig::default()
            },
        );
        registry.register_actor::<StandInTable>("euchreTable");
        registry.register_actor::<StandInProfile>("playerProfile");
    })?)
    .await?;
    client.bind(engine.client());
    Ok(engine)
}

fn table(engine: &TestEngine, game_id: &str) -> Result<TypedActorHandle<StandInTable>> {
    engine.client().get_or_create_typed::<StandInTable>(
        "euchreTable",
        vec!["table".to_owned(), game_id.to_owned()],
        GetOrCreateOptions::default(),
    )
}

/// The seat, called directly with `params` (reads only).
fn seat_handle(
    engine: &TestEngine,
    game_id: &str,
    seat: u8,
    params: Option<Value>,
) -> Result<TypedActorHandle<AiSeat>> {
    engine.client().get_or_create_typed::<AiSeat>(
        ACTOR_NAME,
        actor_key(game_id, seat),
        GetOrCreateOptions {
            params,
            ..Default::default()
        },
    )
}

/// Relay one call through the stand-in table; `Ok(value)` or `Err(message)`.
async fn relay(
    t: &TypedActorHandle<StandInTable>,
    seat: u8,
    action: &str,
    body: Value,
    input: &AiSeatCreateInput,
    params: Value,
) -> Result<std::result::Result<Value, String>> {
    let out = stateless_call_with_timeout(
        t,
        Relay(vec![
            json!(seat),
            json!(action),
            body,
            serde_json::to_value(input)?,
            params,
        ]),
        CALL,
    )
    .await?;
    Ok(if out["ok"] == json!(true) {
        Ok(out["value"].clone())
    } else {
        Err(out["error"].as_str().unwrap_or("").to_owned())
    })
}

async fn relay_ok(
    t: &TypedActorHandle<StandInTable>,
    seat: u8,
    action: &str,
    body: Value,
    input: &AiSeatCreateInput,
) -> Result<()> {
    match relay(t, seat, action, body, input, seat_params()).await? {
        Ok(v) => {
            ensure!(v.is_null(), "{action} returned {v}");
            Ok(())
        }
        Err(e) => bail!("{action} refused: {e}"),
    }
}

async fn run<F, Fut>(name: &str, runtime: AiSeatRuntime, body: F) -> Result<()>
where
    F: FnOnce(TestEngine) -> Fut,
    Fut: std::future::Future<Output = (TestEngine, Result<()>)>,
{
    if !e2e_enabled() {
        e2e_skip(name);
        return Ok(());
    }
    init_tracing();
    let engine = start_engine(runtime).await?;
    let probe_game = unique("probe");
    let probe = table(&engine, &probe_game)?;
    engine
        .ready(|| async { probe.call(Ping::default()).await })
        .await?;
    let (engine, result) = body(engine).await;
    engine.shutdown().await?;
    result
}

fn heuristic_runtime(log: Arc<RecordingLog>) -> AiSeatRuntime {
    AiSeatRuntime::new(
        SeatClient::deferred(),
        LlmProvider::none(),
        Some(PROFILE_TOKEN.to_owned()),
    )
    .with_log(log)
}

fn state_of(game_id: &str, seat: u8) -> Result<Value> {
    test_support::last_state(&test_support::label(game_id, seat)).context("no recorded state")
}

fn legal_ids(view: &PublicGameView) -> Vec<String> {
    view.legal.iter().map(|m| m.id.clone()).collect()
}

// ------------------------------------------------------------------ tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn decide_end_to_end() -> Result<()> {
    let log = Arc::new(RecordingLog::new());
    run(
        "decide_end_to_end",
        heuristic_runtime(log.clone()),
        |engine| async move {
            let r = decide_checks(&engine, &log).await;
            (engine, r)
        },
    )
    .await
}

async fn decide_checks(engine: &TestEngine, log: &RecordingLog) -> Result<()> {
    let game = unique("g");
    let user = unique("u");
    let (view, kind) = find_view("ai-seat-e2e-decide", |_, v| v.legal.len() > 1)?;
    let seat = view.you.get();
    let input = create_input(&game, seat, &user, default_personas().remove(0).persona)?;
    let t = table(engine, &game)?;
    let tkey = vec!["table".to_owned(), game.clone()];
    let label = test_support::label(&game, seat);

    // decide → aiDecision on the heuristic, before the deadline.
    let req = decide_request(&game, &view, kind, "t1");
    let started = Instant::now();
    relay_ok(&t, seat, "decide", serde_json::to_value(&req)?, &input).await?;
    let got = nth(&tkey, "aiDecision", 1).await?;
    let took = started.elapsed();
    ensure!(
        system_now_ms() < req.deadline_at,
        "answered after the deadline ({took:?})"
    );
    eprintln!("decide -> aiDecision in {took:?}");
    let d = &got.body;
    let keys: Vec<_> = d.as_object().context("decision")?.keys().cloned().collect();
    ensure!(
        keys == [
            "internalToken",
            "gameId",
            "seat",
            "turnId",
            "moveId",
            "source",
            "rationale",
            "latencyMs"
        ],
        "decision keys {keys:?}"
    );
    ensure!(d["internalToken"] == json!(TOKEN));
    ensure!(d["gameId"] == json!(game));
    ensure!(d["seat"] == json!(seat));
    ensure!(d["turnId"] == json!("t1"));
    ensure!(
        legal_ids(&view).contains(&d["moveId"].as_str().unwrap_or("").to_owned()),
        "moveId {d}"
    );
    ensure!(d["source"] == json!("fallback"), "source {}", d["source"]);
    ensure!(got.params == seat_params(), "table params {}", got.params);

    // A caller without the token is refused at connect, on messages and reads.
    let refused = relay(
        &t,
        seat,
        "decide",
        serde_json::to_value(decide_request(&game, &view, kind, "t-x"))?,
        &input,
        json!({ "internalToken": "wrong-token-0123456789abcdef" }),
    )
    .await?;
    let Err(message) = refused else {
        bail!("a wrong-token caller was accepted");
    };
    ensure!(
        message.contains("internal_token_mismatch"),
        "refusal: {message}"
    );
    // The reads stay open, as in TS (`scripts/ai-status.mjs` reads
    // `getStatus` with no params); a token-less message is refused.
    let anonymous = seat_handle(engine, &game, seat, None)?;
    let status = stateless_call_with_timeout(&anonymous, GetStatus::new(), CALL).await?;
    ensure!(
        status.decisions == 1 && status.game_id == game,
        "{status:?}"
    );
    let r = stateless_call_with_timeout(
        &anonymous,
        euchre_ai_seat::Decide::new(&decide_request(&game, &view, kind, "t-anon"))?,
        CALL,
    )
    .await;
    let e = r.err().context("a token-less decide was accepted")?;
    let parsed = parse_rivet_error(&e).context("unparsable refusal")?;
    ensure!(
        parsed.is_user() && parsed.is("internal_token_mismatch"),
        "{parsed:?}"
    );
    ensure!(
        parsed.message == euchre_ai_seat::actor::CALLER_REFUSED,
        "{}",
        parsed.message
    );

    // Forged body token, another seat, another match, a redelivered turn: all
    // dropped. A fresh turn behind them is the barrier (one worker, FIFO).
    let mut forged = decide_request(&game, &view, kind, "t-forged");
    forged.internal_token = "forged-token-0123456789abcdef".into();
    relay_ok(&t, seat, "decide", serde_json::to_value(&forged)?, &input).await?;
    let mut wrong_seat = decide_request(&game, &view, kind, "t-seat");
    wrong_seat.seat = seat_of(if seat == 1 { 2 } else { 1 })?;
    relay_ok(
        &t,
        seat,
        "decide",
        serde_json::to_value(&wrong_seat)?,
        &input,
    )
    .await?;
    let mut other_game = decide_request(&game, &view, kind, "t-game");
    other_game.game_id = "another-match".into();
    relay_ok(
        &t,
        seat,
        "decide",
        serde_json::to_value(&other_game)?,
        &input,
    )
    .await?;
    let again = decide_request(&game, &view, kind, "t1");
    relay_ok(&t, seat, "decide", serde_json::to_value(&again)?, &input).await?;
    // Malformed bodies follow the TS check order on the raw JSON: no seat is
    // a wrong seat, a redelivered turnId is stale, and one that passes every
    // check fails in the handler (TS threw on the missing view).
    relay_ok(
        &t,
        seat,
        "decide",
        json!({ "internalToken": TOKEN, "gameId": game }),
        &input,
    )
    .await?;
    relay_ok(
        &t,
        seat,
        "decide",
        json!({ "internalToken": TOKEN, "gameId": game, "seat": seat, "turnId": "t1" }),
        &input,
    )
    .await?;
    relay_ok(
        &t,
        seat,
        "decide",
        json!({ "internalToken": TOKEN, "gameId": "another-match", "seat": 9 }),
        &input,
    )
    .await?;
    relay_ok(
        &t,
        seat,
        "decide",
        json!({ "internalToken": TOKEN, "gameId": game, "seat": seat, "turnId": "t-bad" }),
        &input,
    )
    .await?;
    let barrier = decide_request(&game, &view, kind, "t2");
    relay_ok(&t, seat, "decide", serde_json::to_value(&barrier)?, &input).await?;
    let second = nth(&tkey, "aiDecision", 2).await?;
    ensure!(second.body["turnId"] == json!("t2"), "{}", second.body);
    ensure!(received(&tkey, "aiDecision").len() == 2);

    let s = state_of(&game, seat)?;
    ensure!(s["meter"]["tokenRejections"] == json!(1), "{}", s["meter"]);
    ensure!(s["meter"]["seatRejections"] == json!(2), "{}", s["meter"]);
    ensure!(s["meter"]["staleTurns"] == json!(2), "{}", s["meter"]);
    ensure!(s["meter"]["decisions"] == json!(2), "{}", s["meter"]);
    ensure!(s["lastTurnId"] == json!("t2"));
    ensure!(log.find(messages::FORGED_DECIDE).is_some());
    ensure!(log.find(messages::WRONG_SEAT).is_some());
    let failed = log
        .find(messages::QUEUE_HANDLER_FAILED)
        .context("malformed decide not logged")?;
    ensure!(failed["name"] == json!("decide"), "{failed:?}");
    let failures = log
        .events()
        .iter()
        .filter(|e| e.as_str() == messages::QUEUE_HANDLER_FAILED)
        .count();
    ensure!(failures == 1, "{failures} handler failures");
    let no_seat = log
        .lines()
        .into_iter()
        .filter(|l| l.event == messages::WRONG_SEAT)
        .find(|l| !l.fields.contains_key("claimed"))
        .context("a seatless decide was not logged as another seat's")?;
    ensure!(
        no_seat.fields["mine"] == json!(seat),
        "{:?}",
        no_seat.fields
    );

    // Reads, with the token.
    let h = seat_handle(engine, &game, seat, Some(seat_params()))?;
    let status = stateless_call_with_timeout(&h, GetStatus::new(), CALL).await?;
    ensure!(status.decisions == 2 && status.provisioned, "{status:?}");
    ensure!(status.game_id == game);
    let persona = stateless_call_with_timeout(&h, GetPersona::new(), CALL).await?;
    ensure!(persona.id == input.persona.id, "{persona:?}");
    let decisions = stateless_call_with_timeout(&h, GetDecisionLog::new(), CALL).await?;
    ensure!(decisions.len() == 2, "{decisions:?}");

    // Idle sleep, then a wake: memory and the last answered turn survive.
    let before = state_of(&game, seat)?;
    let starts = test_support::starts(&label);
    wait_for("the seat to sleep", Duration::from_secs(30), || {
        (test_support::sleeps(&label) >= 1).then_some(())
    })
    .await?;
    let status = stateless_call_with_timeout(&h, GetStatus::new(), CALL).await?;
    ensure!(test_support::starts(&label) == starts + 1, "no wake");
    ensure!(status.decisions == 2, "{status:?}");
    let after = state_of(&game, seat)?;
    ensure!(
        after["memory"] == before["memory"],
        "memory changed on wake"
    );
    ensure!(after["lastTurnId"] == json!("t2"));
    // The stored lastTurnId still drops a redelivery of t2.
    relay_ok(&t, seat, "decide", serde_json::to_value(&barrier)?, &input).await?;
    let t3 = decide_request(&game, &view, kind, "t3");
    relay_ok(&t, seat, "decide", serde_json::to_value(&t3)?, &input).await?;
    let third = nth(&tkey, "aiDecision", 3).await?;
    ensure!(third.body["turnId"] == json!("t3"));
    ensure!(state_of(&game, seat)?["meter"]["staleTurns"] == json!(3));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lifecycle_end_to_end() -> Result<()> {
    let log = Arc::new(RecordingLog::new());
    run(
        "lifecycle_end_to_end",
        heuristic_runtime(log.clone()),
        |engine| async move {
            let r = lifecycle_checks(&engine, &log).await;
            (engine, r)
        },
    )
    .await
}

async fn lifecycle_checks(engine: &TestEngine, log: &RecordingLog) -> Result<()> {
    let game = unique("g");
    let user = unique("u");
    let (mut view, kind) = find_view("ai-seat-e2e-life", |s, v| {
        s.hand.phase == GamePhase::TrickPlay && v.legal.len() > 1
    })?;
    let seat = view.you.get();
    // Shape the public view so the hand reads as made by this seat: the
    // episode is then "called it and made the point".
    view.maker_seat = Some(view.you);
    view.alone_seat = None;
    let mine = euchre_engine::team_of(view.you).index();
    view.tricks_won = [0, 0];
    view.tricks_won[mine] = 3;
    view.tricks_won[1 - mine] = 1;
    let input = create_input(&game, seat, &user, default_personas().remove(1).persona)?;
    let t = table(engine, &game)?;
    let tkey = vec!["table".to_owned(), game.clone()];
    let pkey = vec!["user".to_owned(), user.clone()];
    let hand_no = view.hand_no;

    relay_ok(
        &t,
        seat,
        "decide",
        serde_json::to_value(decide_request(&game, &view, kind, "l1"))?,
        &input,
    )
    .await?;
    nth(&tkey, "aiDecision", 1).await?;

    // handEnd → recordEpisodes, accepted by the profile parser. The body has
    // no `kind`: TS never read it, so the seat does not either.
    let mut no_kind = lifecycle(&game, "handEnd", Some(hand_no));
    if let Some(map) = no_kind.as_object_mut() {
        map.remove("kind");
    }
    relay_ok(&t, seat, "handEnd", no_kind, &input).await?;
    let got = nth(&pkey, "recordEpisodes", 1).await?;
    ensure!(body_internal_token(&got.body) == Some(PROFILE_TOKEN));
    let stored = parse_record_episodes(&got.body, i64::try_from(system_now_ms())?)
        .map_err(|e| anyhow::anyhow!("profile parser refused the body: {e}"))?;
    ensure!(stored.len() == 1, "{stored:?}");
    ensure!(stored[0].role == input.persona.id);
    ensure!(stored[0].match_id == format!("m-{game}"));
    ensure!(stored[0].hand_no == i64::from(hand_no));
    let note = format!("h{hand_no}: I called it and made the point");
    ensure!(stored[0].summary == note, "{}", stored[0].summary);

    // resetHand moves the memory on; gameEnd with no spend sends no tokens and
    // (buffer flushed) no episodes. A decide behind them is the barrier.
    relay_ok(
        &t,
        seat,
        "resetHand",
        lifecycle(&game, "resetHand", Some(hand_no + 1)),
        &input,
    )
    .await?;
    relay_ok(
        &t,
        seat,
        "gameEnd",
        lifecycle(&game, "gameEnd", None),
        &input,
    )
    .await?;
    relay_ok(
        &t,
        seat,
        "decide",
        serde_json::to_value(decide_request(&game, &view, kind, "l2"))?,
        &input,
    )
    .await?;
    nth(&tkey, "aiDecision", 2).await?;
    ensure!(received(&pkey, "recordTokens").is_empty());
    ensure!(received(&pkey, "recordEpisodes").len() == 1);
    let s = state_of(&game, seat)?;
    ensure!(s["episodeBuffer"] == json!([]), "{}", s["episodeBuffer"]);
    ensure!(s["notes"] == json!([note]), "{}", s["notes"]);
    ensure!(s["notedHands"] == json!([hand_no]), "{}", s["notedHands"]);
    ensure!(
        log.find(messages::QUEUE_HANDLER_FAILED).is_none(),
        "{:?}",
        log.events()
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scripted_llm_end_to_end() -> Result<()> {
    let log = Arc::new(RecordingLog::new());
    let jev = Arc::new(ScriptedJev::new(vec![Ok(jev_choice("move_1", 0.9))]));
    let talk = Arc::new(ScriptedTalk::new(vec![Ok(ScriptedResult::text("Mine."))]));
    let factory = Arc::new(jev_factory(jev.clone(), talk.clone()));
    let provider = LlmProvider::from_factory(
        Some(factory),
        ProviderDeps {
            clock: Arc::new(SystemClock),
            log: log.clone(),
        },
    );
    let runtime = heuristic_runtime(log.clone()).with_provider(provider);
    run("scripted_llm_end_to_end", runtime, |engine| async move {
        let r = llm_checks(&engine, &jev, &talk).await;
        (engine, r)
    })
    .await
}

async fn llm_checks(engine: &TestEngine, jev: &ScriptedJev, talk: &ScriptedTalk) -> Result<()> {
    let game = unique("g");
    let user = unique("u");
    let (view, kind) = find_view("ai-seat-e2e-llm", |s, v| {
        s.hand.phase == GamePhase::TrickPlay && v.legal.len() > 1 && v.trump.is_some()
    })?;
    let seat = view.you.get();
    let mut persona = default_personas().remove(0).persona;
    persona.chattiness = 0.8;
    let input = create_input(&game, seat, &user, persona)?;
    let t = table(engine, &game)?;
    let tkey = vec!["table".to_owned(), game.clone()];
    let pkey = vec!["user".to_owned(), user.clone()];

    relay_ok(
        &t,
        seat,
        "decide",
        serde_json::to_value(decide_request(&game, &view, kind, "j1"))?,
        &input,
    )
    .await?;
    let d = nth(&tkey, "aiDecision", 1).await?.body;
    ensure!(d["source"] == json!("llm"), "decision {d}");
    ensure!(
        legal_ids(&view).contains(&d["moveId"].as_str().unwrap_or("").to_owned()),
        "moveId {d}"
    );
    ensure!(jev.calls().len() == 1, "jev calls {}", jev.calls().len());

    // One line of table talk after the decision.
    let say = nth(&tkey, "aiSay", 1).await?;
    let b = &say.body;
    ensure!(
        *b == json!({
            "internalToken": TOKEN,
            "gameId": game,
            "seat": seat,
            "turnId": "j1",
            "handNo": view.hand_no,
            "msgId": format!("j1:talk:{seat}"),
            "text": "Mine.",
            "final": true
        }),
        "aiSay {b}"
    );
    ensure!(say.params == seat_params());
    ensure!(talk.calls().len() == 1);
    let s = state_of(&game, seat)?;
    ensure!(s["banter"]["lastHandNo"] == json!(view.hand_no));

    // gameEnd → recordTokens with the scripted usage.
    relay_ok(
        &t,
        seat,
        "gameEnd",
        lifecycle(&game, "gameEnd", None),
        &input,
    )
    .await?;
    let got = nth(&pkey, "recordTokens", 1).await?;
    ensure!(body_internal_token(&got.body) == Some(PROFILE_TOKEN));
    let inc = TokenIncrement::from_body(&got.body);
    let budget = &state_of(&game, seat)?["budget"];
    ensure!(inc.tokens_in == 10, "{inc:?}");
    ensure!(json!(inc.tokens_in) == budget["tokensIn"], "{budget}");
    ensure!(
        json!(inc.calls) == budget["calls"] && inc.calls >= 1,
        "{budget}"
    );
    Ok(())
}

/// A trick-play view with trump set, shaped so the hand reads as made by the
/// seat on turn (its episode is "called it and made the point").
fn made_view(seed: &str) -> Result<(PublicGameView, AIDecisionKind)> {
    let (mut view, kind) = find_view(seed, |s, v| {
        s.hand.phase == GamePhase::TrickPlay && v.legal.len() > 1 && v.trump.is_some()
    })?;
    view.maker_seat = Some(view.you);
    view.alone_seat = None;
    let mine = euchre_engine::team_of(view.you).index();
    view.tricks_won = [0, 0];
    view.tricks_won[mine] = 3;
    view.tricks_won[1 - mine] = 1;
    Ok((view, kind))
}

fn talking_runtime(log: Arc<RecordingLog>) -> AiSeatRuntime {
    let provider = LlmProvider {
        decider: None,
        banter: Some(Arc::new(ScriptedBanter::new(Some("Mine.")))),
    };
    heuristic_runtime(log).with_provider(provider)
}

async fn logged(log: &RecordingLog, event: &str) -> Result<serde_json::Map<String, Value>> {
    wait_for(&format!("log {event}"), WAIT, || log.find(event)).await
}

fn count(log: &RecordingLog, event: &str) -> usize {
    log.events().iter().filter(|e| e.as_str() == event).count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn out_of_order_end_to_end() -> Result<()> {
    let log = Arc::new(RecordingLog::new());
    run(
        "out_of_order_end_to_end",
        heuristic_runtime(log.clone()),
        |engine| async move {
            let r = out_of_order_checks(&engine, &log).await;
            (engine, r)
        },
    )
    .await
}

async fn out_of_order_checks(engine: &TestEngine, log: &RecordingLog) -> Result<()> {
    let game = unique("g");
    let user = unique("u");
    let (view, kind) = made_view("ai-seat-e2e-order")?;
    let seat = view.you.get();
    let input = create_input(&game, seat, &user, default_personas().remove(1).persona)?;
    let t = table(engine, &game)?;
    let tkey = vec!["table".to_owned(), game.clone()];
    let pkey = vec!["user".to_owned(), user.clone()];
    let hand = view.hand_no;
    let mut next = view.clone();
    next.hand_no = hand + 1;

    let first = decide_request(&game, &view, kind, "o1");
    relay_ok(&t, seat, "decide", serde_json::to_value(&first)?, &input).await?;
    nth(&tkey, "aiDecision", 1).await?;
    // The next hand's decide overtakes this hand's handEnd and the next
    // hand's resetHand.
    let second = decide_request(&game, &next, kind, "o2");
    relay_ok(&t, seat, "decide", serde_json::to_value(&second)?, &input).await?;
    nth(&tkey, "aiDecision", 2).await?;
    let absorbed = state_of(&game, seat)?["memory"].clone();
    ensure!(absorbed["handNo"] == json!(hand + 1), "{absorbed}");
    relay_ok(
        &t,
        seat,
        "handEnd",
        lifecycle(&game, "handEnd", Some(hand)),
        &input,
    )
    .await?;
    relay_ok(
        &t,
        seat,
        "resetHand",
        lifecycle(&game, "resetHand", Some(hand + 1)),
        &input,
    )
    .await?;
    // In order again: the next hand ends and is noted as itself.
    relay_ok(
        &t,
        seat,
        "handEnd",
        lifecycle(&game, "handEnd", Some(hand + 1)),
        &input,
    )
    .await?;
    let got = nth(&pkey, "recordEpisodes", 1).await?;
    let stored = parse_record_episodes(&got.body, i64::try_from(system_now_ms())?)
        .map_err(|e| anyhow::anyhow!("profile parser refused the body: {e}"))?;
    ensure!(stored.len() == 1, "{stored:?}");
    ensure!(stored[0].hand_no == i64::from(hand + 1), "{stored:?}");
    let note = format!("h{}: I called it and made the point", hand + 1);
    ensure!(stored[0].summary == note, "{}", stored[0].summary);
    ensure!(received(&pkey, "recordEpisodes").len() == 1);

    let lines: Vec<_> = log
        .lines()
        .into_iter()
        .filter(|l| l.event == euchre_ai_seat::actor::LIFECYCLE_OUT_OF_ORDER)
        .map(|l| l.fields)
        .collect();
    ensure!(lines.len() == 2, "{lines:?}");
    ensure!(
        lines[0]["name"] == json!("handEnd")
            && lines[0]["handNo"] == json!(hand)
            && lines[0]["memoryHandNo"] == json!(hand + 1),
        "{:?}",
        lines[0]
    );
    ensure!(lines[1]["name"] == json!("resetHand"), "{:?}", lines[1]);
    let s = state_of(&game, seat)?;
    ensure!(s["notes"] == json!([note]), "{}", s["notes"]);
    ensure!(s["notedHands"] == json!([hand + 1]), "{}", s["notedHands"]);
    ensure!(
        s["memory"]["snapshot"] == absorbed["snapshot"],
        "the late resetHand wiped the memory"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failure_paths_end_to_end() -> Result<()> {
    let log = Arc::new(RecordingLog::new());
    let runtime = talking_runtime(log.clone()).with_call_timeout(Duration::from_millis(800));
    run("failure_paths_end_to_end", runtime, |engine| async move {
        let r = failure_checks(&engine, &log).await;
        (engine, r)
    })
    .await
}

async fn failure_checks(engine: &TestEngine, log: &RecordingLog) -> Result<()> {
    let game = unique("g");
    let user = unique("u");
    let (view, kind) = made_view("ai-seat-e2e-fail")?;
    let seat = view.you.get();
    let mut persona = default_personas().remove(0).persona;
    persona.chattiness = 0.8;
    let input = create_input(&game, seat, &user, persona)?;
    let t = table(engine, &game)?;
    let tkey = vec!["table".to_owned(), game.clone()];
    let pkey = vec!["user".to_owned(), user.clone()];
    set_mode(
        &game,
        "aiDecision",
        Mode {
            hang: Some((Duration::from_secs(3), 1)),
            fail: 0,
        },
    );
    set_mode(
        &game,
        "aiSay",
        Mode {
            hang: None,
            fail: u32::MAX,
        },
    );
    set_mode(
        &user,
        "recordEpisodes",
        Mode {
            hang: None,
            fail: 1,
        },
    );

    // A hung table: the send is cut at the call timeout, the seat goes on.
    let started = Instant::now();
    relay_ok(
        &t,
        seat,
        "decide",
        serde_json::to_value(decide_request(&game, &view, kind, "f1"))?,
        &input,
    )
    .await?;
    nth(&tkey, "aiDecision", 1).await?;
    let failed = logged(log, messages::QUEUE_HANDLER_FAILED).await?;
    ensure!(failed["name"] == json!("decide"), "{failed:?}");
    let err = failed["err"].as_str().unwrap_or("");
    ensure!(err.contains("aiDecision timed out"), "{err}");
    ensure!(
        started.elapsed() < Duration::from_secs(3),
        "the seat waited out the hung table ({:?})",
        started.elapsed()
    );
    // No table talk for an undelivered decision.
    ensure!(received(&tkey, "aiSay").is_empty());

    // The next decide is answered; its line of table talk fails to deliver.
    relay_ok(
        &t,
        seat,
        "decide",
        serde_json::to_value(decide_request(&game, &view, kind, "f2"))?,
        &input,
    )
    .await?;
    let second = nth(&tkey, "aiDecision", 2).await?;
    ensure!(second.body["turnId"] == json!("f2"), "{}", second.body);
    nth(&tkey, "aiSay", 1).await?;
    let undelivered = logged(log, messages::BANTER_UNDELIVERED).await?;
    ensure!(undelivered.is_empty(), "{undelivered:?}");

    // A failed recordEpisodes keeps the buffer; the next boundary resends it.
    relay_ok(
        &t,
        seat,
        "handEnd",
        lifecycle(&game, "handEnd", Some(view.hand_no)),
        &input,
    )
    .await?;
    nth(&pkey, "recordEpisodes", 1).await?;
    let flush = logged(log, messages::FLUSH_EPISODES_FAILED).await?;
    ensure!(
        flush["err"].as_str().is_some_and(|e| !e.is_empty()),
        "{flush:?}"
    );
    let kept = wait_for("the kept episode", WAIT, || {
        let s = state_of(&game, seat).ok()?;
        (s["episodeBuffer"].as_array().map(Vec::len) == Some(1)).then_some(s)
    })
    .await?;
    ensure!(kept["notedHands"] == json!([view.hand_no]));
    relay_ok(
        &t,
        seat,
        "gameEnd",
        lifecycle(&game, "gameEnd", None),
        &input,
    )
    .await?;
    let resent = nth(&pkey, "recordEpisodes", 2).await?;
    ensure!(resent.body == received(&pkey, "recordEpisodes")[0].body);
    wait_for("the cleared buffer", WAIT, || {
        let s = state_of(&game, seat).ok()?;
        (s["episodeBuffer"] == json!([])).then_some(())
    })
    .await?;
    ensure!(count(log, messages::FLUSH_EPISODES_FAILED) == 1);
    ensure!(count(log, messages::QUEUE_HANDLER_FAILED) == 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sleep_grace_end_to_end() -> Result<()> {
    let log = Arc::new(RecordingLog::new());
    run(
        "sleep_grace_end_to_end",
        talking_runtime(log.clone()),
        |engine| async move {
            let r = grace_checks(&engine, &log).await;
            (engine, r)
        },
    )
    .await
}

async fn grace_checks(engine: &TestEngine, log: &RecordingLog) -> Result<()> {
    let game = unique("g");
    let user = unique("u");
    let (view, kind) = made_view("ai-seat-e2e-grace")?;
    let seat = view.you.get();
    let mut persona = default_personas().remove(0).persona;
    persona.chattiness = 0.8;
    let input = create_input(&game, seat, &user, persona)?;
    let t = table(engine, &game)?;
    let tkey = vec!["table".to_owned(), game.clone()];
    let pkey = vec!["user".to_owned(), user.clone()];
    let label = test_support::label(&game, seat);
    // Table talk hangs 4 s: its keep-awake task holds sleep grace open.
    set_mode(
        &game,
        "aiSay",
        Mode {
            hang: Some((Duration::from_secs(4), 1)),
            fail: 0,
        },
    );

    let g1 = decide_request(&game, &view, kind, "s1");
    relay_ok(&t, seat, "decide", serde_json::to_value(&g1)?, &input).await?;
    nth(&tkey, "aiDecision", 1).await?;
    nth(&tkey, "aiSay", 1).await?;
    let starts = test_support::starts(&label);

    // The next message (a stale redelivery) makes the seat request sleep;
    // the worker drains and stops, grace stays open behind the hung talk.
    // A burst of stale redeliveries then crosses the start of grace: each is
    // queued, drained, handled inline, or (once the engine stops routing to
    // this generation) wakes the next one. None may fail.
    test_support::sleep_on_next_message(&label);
    for _ in 0..20 {
        relay_ok(&t, seat, "decide", serde_json::to_value(&g1)?, &input).await?;
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // Sent during grace: both must be handled, not refused or dropped.
    relay_ok(
        &t,
        seat,
        "handEnd",
        lifecycle(&game, "handEnd", Some(view.hand_no)),
        &input,
    )
    .await?;
    let g2 = decide_request(&game, &view, kind, "s2");
    relay_ok(&t, seat, "decide", serde_json::to_value(&g2)?, &input).await?;
    let got = nth(&pkey, "recordEpisodes", 1).await?;
    ensure!(body_internal_token(&got.body) == Some(PROFILE_TOKEN));
    let second = nth(&tkey, "aiDecision", 2).await?;
    ensure!(second.body["turnId"] == json!("s2"), "{}", second.body);
    let inline = test_support::inline_messages(&label);
    eprintln!(
        "grace: {inline} message(s) handled inline; starts {} -> {}",
        starts,
        test_support::starts(&label)
    );
    wait_for("the seat to sleep", Duration::from_secs(30), || {
        (test_support::sleeps(&label) >= 1).then_some(())
    })
    .await?;
    ensure!(
        log.find(messages::QUEUE_HANDLER_FAILED).is_none(),
        "{:?}",
        log.events()
    );
    let s = state_of(&game, seat)?;
    ensure!(s["lastTurnId"] == json!("s2"), "{}", s["lastTurnId"]);
    ensure!(s["meter"]["staleTurns"] == json!(20), "{}", s["meter"]);
    Ok(())
}
