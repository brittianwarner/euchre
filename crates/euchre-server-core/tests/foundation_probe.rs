//! Engine-backed probe of the `euchre_server_core::rivet` helpers (lane SERVER-FOUNDATION).
//!
//! Each test prints `SKIP` and returns unless `EUCHRE_E2E=1` (a plain `cargo test`
//! stays engine-free). Run:
//!
//! ```text
//! EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6701 EUCHRE_TEST_STORAGE=<dir> \
//!   cargo test -p euchre-server-core --test foundation_probe -- --nocapture
//! ```
//!
//! The two tests run in parallel against one shared engine, each with its own registry
//! on its own pool (`testing::TestEngine`).
//!
//! `probe_actor_end_to_end` covers: a JSON action round-trip and a `user` error parsed
//! with its code (WebSocket and stateless), conn-params identity re-derived per action,
//! the initial sync `conn.send` from `on_connect` plus a `ctx.emit` broadcast, a
//! `schedule_after` fire plus the conn-None guard refusing a direct client call, a
//! SQLite save reloaded after sleep and wake, and the lane serializing two concurrent
//! mutations (with an unlaned control proving actions really interleave).
//!
//! `probe_rejections_end_to_end` covers the negative controls: a plain `bail!` reaches
//! clients masked as `core/internal_error` with its text hidden (D21), a bad token is
//! refused fast on a WebSocket by `connect_bounded` (`CallError::Rejected`) and over
//! HTTP as `user/unauthorized`, and `wait_ready` stops at once on a user error.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use async_trait::async_trait;
use euchre_server_core::rivet::{
    ActorLane, CallError, JsonTable, call_with_timeout, caller, conn_params, connect_bounded,
    decode_params, forward_events, parse_rivet_error, require_scheduled_fire, schedule_action,
    stateless_call_with_timeout, user_error, user_error_with_metadata, wait_ready,
};
use euchre_server_core::testing::{TestEngine, TestEngineOptions, e2e_enabled, e2e_skip};
use rivetkit::client::{ConnectionStatus, GetOrCreateOptions};
use rivetkit::prelude::*;
use rivetkit::{Action, ActorConfig, Event, TypedActorHandle, TypedClientExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

// ------------------------------------------------------------------ wire types

macro_rules! action {
    ($ty:ty, $name:literal, $out:ty) => {
        impl Action for $ty {
            type Output = $out;
            const NAME: &'static str = $name;
        }
    };
}

macro_rules! event {
    ($ty:ty, $name:literal) => {
        impl Event for $ty {
            const NAME: &'static str = $name;
        }
    };
}

/// Connection params: a shared-secret token and a claimed user.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
struct ProbeParams {
    token: String,
    user: String,
}

const GOOD_TOKEN: &str = "probe-token";

fn verify(params: &Value) -> Result<String> {
    let p: ProbeParams = decode_params(params)?;
    if p.token != GOOD_TOKEN {
        return Err(user_error("unauthorized", "bad probe token"));
    }
    if p.user.is_empty() {
        return Err(user_error("unauthorized", "no user"));
    }
    Ok(p.user)
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ProbeState {
    counter: i64,
    ticks: Vec<String>,
    saved: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    state: ProbeState,
    wake_no: i64,
}

/// Two fields on purpose: rivetkit decodes args as a sequence first, so a single-field
/// struct whose field accepts any JSON (`Value`, `Vec`) would swallow the whole object.
#[derive(Serialize, Deserialize, Debug)]
struct Echo {
    value: Value,
    tag: String,
}
action!(Echo, "echo", Value);

#[derive(Serialize, Deserialize, Debug)]
struct Fail {
    code: String,
    message: String,
}
action!(Fail, "fail", ());

#[derive(Serialize, Deserialize, Debug)]
struct Whoami;
action!(Whoami, "whoami", String);

/// Fails with a plain `anyhow` error, which clients must see masked (D21).
#[derive(Serialize, Deserialize, Debug)]
struct Crash;
action!(Crash, "crash", ());

/// Text the masked error must never reveal.
const CRASH_SECRET: &str = "internal detail: sqlite path /secret";

#[derive(Serialize, Deserialize, Debug)]
struct Shout {
    text: String,
}
action!(Shout, "shout", ());

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ScheduleTick {
    delay_ms: u64,
    label: String,
}
action!(ScheduleTick, "scheduleTick", String);

/// Internal: only the scheduler may run it.
#[derive(Serialize, Deserialize, Debug)]
struct Tick {
    label: String,
}
action!(Tick, "tick", ());

#[derive(Serialize, Deserialize, Debug)]
struct Save {
    text: String,
}
action!(Save, "save", ());

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Bump {
    delay_ms: u64,
}
action!(Bump, "bump", i64);

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct BumpUnlaned {
    delay_ms: u64,
}
action!(BumpUnlaned, "bumpUnlaned", i64);

#[derive(Serialize, Deserialize, Debug)]
struct GetSnapshot;
action!(GetSnapshot, "getSnapshot", Snapshot);

#[derive(Serialize, Deserialize, Debug)]
struct GoToSleep;
action!(GoToSleep, "goToSleep", ());

/// Private initial sync, sent from `on_connect` to the connecting client only.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct Sync {
    user: String,
    snapshot: Snapshot,
}
event!(Sync, "sync");

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Shouted {
    from: String,
    text: String,
}
event!(Shouted, "shouted");

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Ticked {
    label: String,
}
event!(Ticked, "ticked");

// ------------------------------------------------------------------ actor

const STATE_KEY: &str = "state";
const WAKES_KEY: &str = "wakes";

fn table() -> Result<JsonTable> {
    JsonTable::new("probe_doc")
}

struct Probe {
    lane: ActorLane,
    state: parking_lot::Mutex<ProbeState>,
    racy: parking_lot::Mutex<i64>,
    wake_no: i64,
    docs: JsonTable,
}

impl Probe {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            state: self.state.lock().clone(),
            wake_no: self.wake_no,
        }
    }

    /// Persist the current state (call while holding the lane).
    async fn persist(&self, ctx: &Ctx<Self>) -> Result<()> {
        let s = self.state.lock().clone();
        self.docs.save(ctx.sql(), STATE_KEY, &s).await
    }
}

#[async_trait]
impl Actor for Probe {
    type Input = Value;
    type ConnParams = Value;
    type Action = rivetkit::Raw;
    type Actions = (
        Echo,
        Fail,
        Whoami,
        Crash,
        Shout,
        ScheduleTick,
        Tick,
        Save,
        Bump,
        BumpUnlaned,
        GetSnapshot,
        GoToSleep,
    );
    type Events = (Sync, Shouted, Ticked);

    async fn create(ctx: &Ctx<Self>) -> Result<Self> {
        let docs = table()?;
        let db = ctx.sql();
        docs.ensure(db).await?;
        let wake_no = docs.load::<i64>(db, WAKES_KEY).await?.unwrap_or(0) + 1;
        docs.save(db, WAKES_KEY, &wake_no).await?;
        let state = docs
            .load::<ProbeState>(db, STATE_KEY)
            .await?
            .unwrap_or_default();
        Ok(Self {
            lane: ActorLane::new(),
            state: parking_lot::Mutex::new(state),
            racy: parking_lot::Mutex::new(0),
            wake_no,
            docs,
        })
    }

    async fn on_before_connect(self: Arc<Self>, _ctx: Ctx<Self>, params: &Value) -> Result<()> {
        verify(params).map(|_| ())
    }

    async fn on_connect(self: Arc<Self>, _ctx: Ctx<Self>, conn: ConnCtx<Self>) -> Result<()> {
        let p: ProbeParams = conn_params(&conn)?;
        conn.send(
            Sync::NAME,
            &Sync {
                user: p.user,
                snapshot: self.snapshot(),
            },
        )?;
        Ok(())
    }
}

impl Handler<Echo> for Probe {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: Echo) -> Result<Value> {
        let user = caller(&ctx, verify)?;
        Ok(json!({ "echo": a.value, "tag": a.tag, "user": user }))
    }
}

impl Handler<Fail> for Probe {
    async fn handle(self: Arc<Self>, _ctx: Ctx<Self>, a: Fail) -> Result<()> {
        Err(user_error_with_metadata(
            &a.code,
            a.message,
            &json!({ "k": "v", "n": 1 }),
        ))
    }
}

impl Handler<Whoami> for Probe {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, _a: Whoami) -> Result<String> {
        caller(&ctx, verify)
    }
}

impl Handler<Crash> for Probe {
    async fn handle(self: Arc<Self>, _ctx: Ctx<Self>, _a: Crash) -> Result<()> {
        bail!("{CRASH_SECRET}")
    }
}

impl Handler<Shout> for Probe {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: Shout) -> Result<()> {
        let from = caller(&ctx, verify)?;
        ctx.emit(Shouted { from, text: a.text })
    }
}

impl Handler<ScheduleTick> for Probe {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: ScheduleTick) -> Result<String> {
        caller(&ctx, verify)?;
        schedule_action(
            &ctx,
            Duration::from_millis(a.delay_ms),
            &Tick { label: a.label },
        )
        .await
    }
}

impl Handler<Tick> for Probe {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: Tick) -> Result<()> {
        require_scheduled_fire(&ctx)?;
        let _turn = self.lane.enter().await;
        self.state.lock().ticks.push(a.label.clone());
        self.persist(&ctx).await?;
        ctx.emit(Ticked { label: a.label })
    }
}

impl Handler<Save> for Probe {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: Save) -> Result<()> {
        caller(&ctx, verify)?;
        let _turn = self.lane.enter().await;
        self.state.lock().saved.push(a.text);
        self.persist(&ctx).await
    }
}

impl Handler<Bump> for Probe {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: Bump) -> Result<i64> {
        let me = self.clone();
        self.lane
            .run(async move {
                let read = me.state.lock().counter;
                tokio::time::sleep(Duration::from_millis(a.delay_ms)).await;
                me.state.lock().counter = read + 1;
                me.persist(&ctx).await?;
                Ok(read + 1)
            })
            .await
    }
}

impl Handler<BumpUnlaned> for Probe {
    async fn handle(self: Arc<Self>, _ctx: Ctx<Self>, a: BumpUnlaned) -> Result<i64> {
        let read = *self.racy.lock();
        tokio::time::sleep(Duration::from_millis(a.delay_ms)).await;
        *self.racy.lock() = read + 1;
        Ok(read + 1)
    }
}

impl Handler<GetSnapshot> for Probe {
    async fn handle(self: Arc<Self>, _ctx: Ctx<Self>, _a: GetSnapshot) -> Result<Snapshot> {
        Ok(self.snapshot())
    }
}

impl Handler<GoToSleep> for Probe {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, _a: GoToSleep) -> Result<()> {
        ctx.sleep()
    }
}

// ------------------------------------------------------------------ client side

const CALL: Duration = Duration::from_secs(10);

fn params(user: &str, token: &str) -> Value {
    json!({ "token": token, "user": user })
}

fn handle(
    engine: &TestEngine,
    key: &str,
    user: &str,
    token: &str,
) -> Result<TypedActorHandle<Probe>> {
    engine.client().get_or_create_typed::<Probe>(
        "probe",
        [key],
        GetOrCreateOptions {
            params: Some(params(user, token)),
            ..Default::default()
        },
    )
}

async fn recv<E>(rx: &mut UnboundedReceiver<E>, what: &str) -> Result<E> {
    match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
        Ok(Some(e)) => Ok(e),
        Ok(None) => bail!("{what}: event channel closed"),
        Err(_) => bail!("{what}: no event within 10 s"),
    }
}

fn expect_user_error<T: std::fmt::Debug>(
    r: Result<T>,
    code: &str,
    what: &str,
) -> Result<euchre_server_core::rivet::UserError> {
    let e = match r {
        Ok(v) => bail!("{what}: expected user/{code}, got Ok({v:?})"),
        Err(e) => e,
    };
    let parsed =
        parse_rivet_error(&e).with_context(|| format!("{what}: unparsable error `{e:#}`"))?;
    ensure!(
        parsed.is_user() && parsed.is(code),
        "{what}: expected user/{code}, got {parsed:?} from `{e:#}`"
    );
    Ok(parsed)
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .with_test_writer()
        .try_init();
}

async fn start_engine() -> Result<TestEngine> {
    TestEngine::start(TestEngineOptions::from_env(|registry| {
        registry.register_actor_with::<Probe>(
            "probe",
            ActorConfig {
                sleep_timeout: Duration::from_secs(2),
                ..ActorConfig::default()
            },
        );
    })?)
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_actor_end_to_end() -> Result<()> {
    if !e2e_enabled() {
        e2e_skip("probe_actor_end_to_end");
        return Ok(());
    }
    init_tracing();
    let engine = start_engine().await?;
    let result = run_probe(&engine).await;
    engine.shutdown().await?;
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_rejections_end_to_end() -> Result<()> {
    if !e2e_enabled() {
        e2e_skip("probe_rejections_end_to_end");
        return Ok(());
    }
    init_tracing();
    let engine = start_engine().await?;
    let result = run_rejections(&engine).await;
    engine.shutdown().await?;
    result
}

async fn run_rejections(engine: &TestEngine) -> Result<()> {
    let key = format!("reject-{}", uuid::Uuid::new_v4().simple());
    let alice = handle(engine, &key, "alice", GOOD_TOKEN)?;
    let ready = engine
        .ready(|| {
            let h = &alice;
            async move { h.call(Whoami).await }
        })
        .await?;
    ensure!(ready == "alice", "ready probe returned {ready}");
    ensure!(
        engine.token().expose() != "default",
        "the test engine must not use the well-known admin token"
    );

    // D21: a plain bail! is masked, over HTTP and over a WebSocket.
    let masked = |r: Result<()>, what: &str| -> Result<()> {
        let e = match r {
            Ok(()) => bail!("{what}: crash returned Ok"),
            Err(e) => e,
        };
        let text = format!("{e:#}");
        let parsed = parse_rivet_error(&e).with_context(|| format!("{what}: `{text}`"))?;
        ensure!(
            parsed.group == "core" && parsed.code == "internal_error",
            "{what}: expected core/internal_error, got {parsed:?}"
        );
        ensure!(
            !text.contains(CRASH_SECRET) && !parsed.message.contains("sqlite"),
            "{what}: masked error leaked its text: `{text}`"
        );
        Ok(())
    };
    masked(
        stateless_call_with_timeout(&alice, Crash, CALL).await,
        "stateless crash",
    )?;
    let conn = connect_bounded(&alice, CALL).await?;
    masked(call_with_timeout(&conn, Crash, CALL).await, "ws crash")?;
    conn.disconnect().await;

    // A bad token: refused fast on a WebSocket, classifiable over HTTP.
    let mallory = handle(engine, &key, "mallory", "wrong-token")?;
    let started = std::time::Instant::now();
    let refused = connect_bounded(&mallory, Duration::from_secs(20)).await;
    let elapsed = started.elapsed();
    let e = match refused {
        Ok(c) => {
            c.disconnect().await;
            bail!("mallory connected with a bad token")
        }
        Err(e) => e,
    };
    let call_error = e.downcast_ref::<CallError>();
    ensure!(
        matches!(call_error, Some(CallError::Rejected { .. })),
        "bad-token connect: expected CallError::Rejected, got `{e:#}`"
    );
    ensure!(
        elapsed < Duration::from_secs(10),
        "bad-token connect took {elapsed:?}; it must not wait for the timeout"
    );
    eprintln!("ws bad-token connect: `{e:#}` after {elapsed:?}");
    expect_user_error(
        stateless_call_with_timeout(&mallory, Whoami, CALL).await,
        "unauthorized",
        "bad token stateless (the documented way to classify a rejected connect)",
    )?;

    // wait_ready gives up at once on a user error instead of burning its deadline.
    let started = std::time::Instant::now();
    let r = wait_ready(Duration::from_secs(60), || {
        let h = &mallory;
        async move { h.call(Whoami).await }
    })
    .await;
    expect_user_error(r, "unauthorized", "wait_ready user error")?;
    ensure!(
        started.elapsed() < Duration::from_secs(10),
        "wait_ready retried a user error for {:?}",
        started.elapsed()
    );
    eprintln!("rejections ok: pool={} key={key}", engine.pool_name());
    Ok(())
}

async fn run_probe(engine: &TestEngine) -> Result<()> {
    let run = uuid::Uuid::new_v4().simple().to_string();
    let key = format!("probe-{run}");
    let alice = handle(engine, &key, "alice", GOOD_TOKEN)?;
    let bob = handle(engine, &key, "bob", GOOD_TOKEN)?;

    // G36: the first requests after boot can fail while the envoy registers.
    let ready = engine
        .ready(|| {
            let h = &alice;
            async move { h.call(Whoami).await }
        })
        .await?;
    ensure!(ready == "alice", "ready probe returned {ready}");

    // 1. JSON round-trip + user errors, stateless and over WebSocket.
    let payload = json!({ "s": "\u{e9}\u{1F0A1}", "n": 1.5, "big": 9_007_199_254_740_991_i64, "a": [1, null, true], "o": {"z": 1, "a": 2} });
    let echoed = stateless_call_with_timeout(
        &alice,
        Echo {
            value: payload.clone(),
            tag: "t".into(),
        },
        CALL,
    )
    .await?;
    ensure!(
        echoed == json!({ "echo": payload, "tag": "t", "user": "alice" }),
        "stateless echo: {echoed}"
    );
    let fail = Fail {
        code: "probe_failed".into(),
        message: "It did not work.".into(),
    };
    let e = expect_user_error(
        stateless_call_with_timeout(&alice, fail, CALL).await,
        "probe_failed",
        "stateless fail",
    )?;
    ensure!(
        e.message == "It did not work." && e.metadata == Some(json!({"k": "v", "n": 1})),
        "stateless error: {e:?}"
    );

    // Initial sync: listeners registered right after connect() see on_connect's conn.send.
    let conn = alice.connect();
    let (_sync_sub, mut sync_rx) = forward_events::<Probe, Sync>(&conn).await;
    let (_shout_sub, mut shout_rx) = forward_events::<Probe, Shouted>(&conn).await;
    let (_tick_sub, mut tick_rx) = forward_events::<Probe, Ticked>(&conn).await;
    let mut status = conn.inner().status_receiver();
    tokio::time::timeout(CALL, status.wait_for(|s| *s == ConnectionStatus::Connected))
        .await
        .context("alice connect timed out")??;
    let sync = recv(&mut sync_rx, "initial sync").await?;
    ensure!(
        sync.user == "alice" && sync.snapshot.wake_no >= 1,
        "sync: {sync:?}"
    );

    let echoed = call_with_timeout(
        &conn,
        Echo {
            value: payload.clone(),
            tag: "t".into(),
        },
        CALL,
    )
    .await?;
    ensure!(
        echoed == json!({ "echo": payload, "tag": "t", "user": "alice" }),
        "ws echo: {echoed}"
    );
    let fail = Fail {
        code: "probe_failed".into(),
        message: "It did not work.".into(),
    };
    let e = expect_user_error(
        call_with_timeout(&conn, fail, CALL).await,
        "probe_failed",
        "ws fail",
    )?;
    ensure!(
        e.message == "It did not work." && e.metadata == Some(json!({"k": "v", "n": 1})),
        "ws error: {e:?}"
    );

    // 2. Identity comes from each call's own conn params.
    ensure!(
        call_with_timeout(&conn, Whoami, CALL).await? == "alice",
        "ws whoami"
    );
    ensure!(
        stateless_call_with_timeout(&bob, Whoami, CALL).await? == "bob",
        "stateless whoami bob"
    );
    ensure!(
        stateless_call_with_timeout(&alice, Whoami, CALL).await? == "alice",
        "stateless whoami alice"
    );
    let mallory = handle(engine, &key, "mallory", "wrong-token")?;
    let refused = stateless_call_with_timeout(&mallory, Whoami, CALL).await;
    expect_user_error(refused, "unauthorized", "bad token stateless")?;

    // 3. ctx.emit broadcast reaches the subscribed WebSocket.
    stateless_call_with_timeout(&bob, Shout { text: "hi".into() }, CALL).await?;
    let shout = recv(&mut shout_rx, "broadcast").await?;
    ensure!(
        shout.from == "bob" && shout.text == "hi",
        "shout: {shout:?}"
    );

    // 4. schedule_after fires with no conn; a direct client call is refused.
    let started = std::time::Instant::now();
    let id = call_with_timeout(
        &conn,
        ScheduleTick {
            delay_ms: 500,
            label: "sched".into(),
        },
        CALL,
    )
    .await?;
    let tick = recv(&mut tick_rx, "scheduled tick").await?;
    ensure!(
        tick.label == "sched" && started.elapsed() >= Duration::from_millis(450),
        "tick {tick:?} after {:?}",
        started.elapsed()
    );
    ensure!(!id.is_empty(), "schedule id");
    let direct = call_with_timeout(
        &conn,
        Tick {
            label: "direct".into(),
        },
        CALL,
    )
    .await;
    let e = expect_user_error(direct, "internal_only", "direct ws tick")?;
    ensure!(e.message == "This action is internal.", "{e:?}");
    let direct = stateless_call_with_timeout(
        &alice,
        Tick {
            label: "direct-http".into(),
        },
        CALL,
    )
    .await;
    expect_user_error(direct, "internal_only", "direct stateless tick")?;

    // 5. The lane serializes concurrent mutations; unlaned actions interleave.
    let (a, b) = tokio::join!(
        call_with_timeout(&conn, Bump { delay_ms: 300 }, CALL),
        stateless_call_with_timeout(&bob, Bump { delay_ms: 300 }, CALL),
    );
    let mut laned = vec![a?, b?];
    laned.sort_unstable();
    ensure!(laned == vec![1, 2], "laned bumps returned {laned:?}");
    let (a, b) = tokio::join!(
        call_with_timeout(&conn, BumpUnlaned { delay_ms: 300 }, CALL),
        stateless_call_with_timeout(&bob, BumpUnlaned { delay_ms: 300 }, CALL),
    );
    let unlaned = vec![a?, b?];
    ensure!(
        unlaned == vec![1, 1],
        "control: unlaned bumps should lose an update, got {unlaned:?}"
    );
    conn.disconnect().await;

    // 6. SQLite save survives sleep and wake (create() reloads it).
    stateless_call_with_timeout(
        &alice,
        Save {
            text: "persist me".into(),
        },
        CALL,
    )
    .await?;
    let before = stateless_call_with_timeout(&alice, GetSnapshot, CALL).await?;
    stateless_call_with_timeout(&alice, GoToSleep, CALL).await?;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let after = engine
        .ready(|| {
            let h = &alice;
            async move { h.call(GetSnapshot).await }
        })
        .await?;
    ensure!(
        after.wake_no == before.wake_no + 1,
        "wake_no {} -> {}",
        before.wake_no,
        after.wake_no
    );
    ensure!(
        after.state == before.state,
        "state after wake {:?} != {:?}",
        after.state,
        before.state
    );
    ensure!(
        after.state.saved == vec!["persist me".to_owned()]
            && after.state.counter == 2
            && after.state.ticks == vec!["sched".to_owned()],
        "reloaded state {:?}",
        after.state
    );
    eprintln!(
        "probe ok: endpoint={} key={key} wake {} -> {} laned={laned:?} unlaned={unlaned:?}",
        engine.endpoint(),
        before.wake_no,
        after.wake_no
    );
    Ok(())
}
