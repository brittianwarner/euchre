//! Engine-backed tests of the `euchreTable` actor (lane SERVER-TABLE).
//!
//! Each test prints `SKIP` and returns unless `EUCHRE_E2E=1`, so a plain
//! `cargo test` stays engine-free. Run:
//!
//! ```text
//! EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6711 EUCHRE_TEST_STORAGE=<dir> \
//!   cargo test -p euchre-table --test table_e2e -- --nocapture
//! ```
//!
//! Every test starts its own pool on the shared engine (`testing::TestEngine`)
//! and uses fresh match and user ids, so they run in parallel. The AI seats are
//! the in-actor heuristic (`HeuristicSeat`): no `aiSeat` actor is involved.
//!
//! - `table_auth_end_to_end`: a guest Actors JWT accepted over HTTP and
//!   WebSocket (the first visitor claims the table); bad, expired, wrong-seat,
//!   wrong-protocol and foreign tokens and a bad Origin refused with the TS codes
//!   and messages; the internal token; role gates on reads, writes, former
//!   queues and schedule targets; presence on 0→1 and 1→0 only.
//! - `table_moves_end_to_end`: the initial `sync`, an illegal move, a stale
//!   turn, a duplicate `clientMoveId`, and concurrent submits serialized by the
//!   lane.
//! - `table_match_end_to_end`: a full hand (with its review), then the whole
//!   match to game over, then `game_complete`.
//! - `table_review_sleep_end_to_end`: the trick-review gate holds a finished
//!   trick; the table sleeps mid-hand at the gate and wakes with the same
//!   state; Continue then drives the AI timers again, and a second sleep at a
//!   player turn wakes the same way.

use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use euchre_protocol::MoveAck;
use euchre_server_core::auth::{AuthKeys, Authenticator, GuestAudience, now_secs};
use euchre_server_core::config::{OriginPolicy, Secret};
use euchre_server_core::rivet::{
    CallError, UserError, call_with_timeout, connect_bounded, forward_events, parse_rivet_error,
    stateless_call_with_timeout,
};
use euchre_server_core::testing::{TestEngine, TestEngineOptions, e2e_enabled, e2e_skip};
use euchre_table::{
    ACTOR_NAME, EuchreTable, GetHandReview, LastTrick, MoveQueue, OnReap, OnTempoGate, ReviewQueue,
    ReviewTrick, Snapshot, SubmitMove, TableClient, TableConfig, TickQueue, actor_key,
    register_with, test_support,
};
use rivetkit::client::{
    Client, ClientConfig, ConnectionStatus, EncodingKind, GetOrCreateOptions, SubscriptionHandle,
};
use rivetkit::{ActorConfig, Event, TypedActorConnection, TypedActorHandle, TypedClientExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

// ------------------------------------------------------------------ fixtures

const CALL: Duration = Duration::from_secs(15);
const EVENT: Duration = Duration::from_secs(20);
const ALLOWED_ORIGIN: &str = "https://app.example";

/// One slot for the whole process (`configure` compares the slot, so every
/// test installs this same one).
static UNBOUND: std::sync::LazyLock<TableClient> = std::sync::LazyLock::new(TableClient::deferred);

fn auth() -> Authenticator {
    Authenticator::new(AuthKeys {
        guest: Secret::new("e2e-guest-key-0123456789abcdef"),
        profile: Secret::new("e2e-profile-key-0123456789abcdef"),
        internal: Secret::new("e2e-internal-token-0123456789abcdef"),
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
        // Never bound: the in-actor heuristic plays and nothing is recorded.
        client: UNBOUND.clone(),
        profile_token: None,
    })
}

fn guest() -> String {
    format!("guest-{}", uuid::Uuid::new_v4())
}

fn game_id(tag: &str) -> String {
    format!("{tag}-{}", uuid::Uuid::new_v4().simple())
}

fn token_at(user: &str, iat: i64) -> Result<String> {
    Ok(auth().sign_guest_at(user, GuestAudience::Actors, iat)?)
}

fn player_params(user: &str) -> Result<Value> {
    Ok(json!({ "token": token_at(user, now_secs())?, "protocolVersion": 1 }))
}

fn table(
    client: &Client,
    game: &str,
    params: Value,
    input: Option<Value>,
) -> Result<TypedActorHandle<EuchreTable>> {
    client.get_or_create_typed::<EuchreTable>(
        ACTOR_NAME,
        actor_key(game),
        GetOrCreateOptions {
            params: Some(params),
            create_with_input: input,
            ..Default::default()
        },
    )
}

fn handle(engine: &TestEngine, game: &str, params: Value) -> Result<TypedActorHandle<EuchreTable>> {
    table(
        &engine.client(),
        game,
        params,
        Some(json!({ "seed": format!("seed-{game}") })),
    )
}

/// A second client on the same pool that sends an `Origin` header.
fn origin_client(engine: &TestEngine, origin: &str) -> Client {
    Client::new(
        ClientConfig::new(engine.endpoint())
            .namespace(engine.namespace())
            .token(engine.token().expose())
            .pool_name(engine.pool_name())
            .encoding(EncodingKind::Json)
            .header("origin", origin),
    )
}

/// The `sync` event body (`{ v, view, steps }`).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(transparent)]
struct SyncEv(Value);
impl Event for SyncEv {
    const NAME: &'static str = "sync";
}

/// The `presence` event body (`{ seat, online }`).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(transparent)]
struct PresenceEv(Value);
impl Event for PresenceEv {
    const NAME: &'static str = "presence";
}

// ------------------------------------------------------------------ helpers

fn expect_user_error<T: std::fmt::Debug>(
    r: Result<T>,
    code: &str,
    message: &str,
    what: &str,
) -> Result<UserError> {
    let e = match r {
        Ok(v) => bail!("{what}: expected user/{code}, got Ok({v:?})"),
        Err(e) => e,
    };
    let parsed =
        parse_rivet_error(&e).with_context(|| format!("{what}: unparsable error `{e:#}`"))?;
    ensure!(
        parsed.is_user() && parsed.is(code) && parsed.message == message,
        "{what}: expected user/{code} `{message}`, got {parsed:?}"
    );
    Ok(parsed)
}

async fn expect_ws_rejected(h: &TypedActorHandle<EuchreTable>, what: &str) -> Result<()> {
    let started = Instant::now();
    match connect_bounded(h, Duration::from_secs(20)).await {
        Ok(c) => {
            c.disconnect().await;
            bail!("{what}: connected over WebSocket");
        }
        Err(e) => ensure!(
            matches!(
                e.downcast_ref::<CallError>(),
                Some(CallError::Rejected { .. })
            ),
            "{what}: ws connect `{e:#}`"
        ),
    }
    ensure!(
        started.elapsed() < Duration::from_secs(10),
        "{what}: rejection was slow"
    );
    Ok(())
}

async fn recv<E>(rx: &mut UnboundedReceiver<E>, what: &str) -> Result<E> {
    match tokio::time::timeout(EVENT, rx.recv()).await {
        Ok(Some(e)) => Ok(e),
        Ok(None) => bail!("{what}: event channel closed"),
        Err(_) => bail!("{what}: no event within {EVENT:?}"),
    }
}

/// Nothing arrives on `rx` within `wait`.
async fn quiet<E: std::fmt::Debug>(
    rx: &mut UnboundedReceiver<E>,
    wait: Duration,
    what: &str,
) -> Result<()> {
    match tokio::time::timeout(wait, rx.recv()).await {
        Err(_) => Ok(()),
        Ok(e) => bail!("{what}: unexpected event {e:?}"),
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .with_test_writer()
        .try_init();
}

async fn start_engine() -> Result<TestEngine> {
    // Runtime opt-in for the start/sleep/state hooks (off in every server build).
    test_support::enable();
    // One config per process: every test installs the same one, so this and the
    // `register_with` below are `Ok`; a conflict fails the test here.
    euchre_table::configure(config()?)?;
    let cfg = config()?;
    TestEngine::start(TestEngineOptions::from_env(move |registry| {
        let registered = register_with(
            registry,
            cfg.clone(),
            ActorConfig {
                sleep_timeout: Duration::from_secs(1),
                ..ActorConfig::default()
            },
        );
        if let Err(e) = registered {
            panic!("register euchreTable: {e}");
        }
    })?)
    .await
}

/// G36: wait until the envoy serves this table, through `h`.
async fn ready(engine: &TestEngine, h: &TypedActorHandle<EuchreTable>) -> Result<Value> {
    engine
        .ready(|| async move { h.call(Snapshot(vec![])).await })
        .await
}

async fn run<F, Fut>(name: &str, body: F) -> Result<()>
where
    F: FnOnce(TestEngine) -> Fut,
    Fut: std::future::Future<Output = (TestEngine, Result<()>)>,
{
    if !e2e_enabled() {
        e2e_skip(name);
        return Ok(());
    }
    init_tracing();
    let engine = start_engine().await?;
    let (engine, result) = body(engine).await;
    engine.shutdown().await?;
    result
}

fn state_of(game: &str) -> Result<Value> {
    test_support::last_state(game).with_context(|| format!("no persisted state for {game}"))
}

fn applied_seq(game: &str) -> Result<u64> {
    state_of(game)?["game"]["appliedSeq"]
        .as_u64()
        .context("appliedSeq")
}

fn v_of(view: &Value) -> u64 {
    view["v"].as_u64().unwrap_or(0)
}

fn is_player_turn(view: &Value) -> bool {
    view["status"] == "active"
        && view["turnSeat"] == 0
        && view["legal"].as_array().is_some_and(|l| !l.is_empty())
}

fn first_legal(view: &Value) -> Result<String> {
    view["legal"][0]["id"]
        .as_str()
        .map(str::to_owned)
        .context("no legal move")
}

fn submit(move_id: &str, turn_id: &Value, client_move_id: &str) -> SubmitMove {
    SubmitMove(vec![json!({
        "moveId": move_id,
        "turnId": turn_id,
        "clientMoveId": client_move_id,
    })])
}

/// A player WebSocket with its `sync` and `presence` streams. The listeners
/// are registered before the socket opens, so the initial `sync` is seen.
struct Seat {
    conn: TypedActorConnection<EuchreTable>,
    syncs: UnboundedReceiver<SyncEv>,
    presence: UnboundedReceiver<PresenceEv>,
    _subs: (SubscriptionHandle, SubscriptionHandle),
    /// The newest `sync` seen.
    last: Option<Value>,
    /// Every `sync` version seen, in order.
    seen: Vec<u64>,
    moves: u64,
}

impl Seat {
    async fn open(h: &TypedActorHandle<EuchreTable>) -> Result<Seat> {
        let conn = h.connect();
        let (sync_sub, syncs) = forward_events::<EuchreTable, SyncEv>(&conn).await;
        let (presence_sub, presence) = forward_events::<EuchreTable, PresenceEv>(&conn).await;
        let mut status = conn.inner().status_receiver();
        tokio::time::timeout(CALL, status.wait_for(|s| *s == ConnectionStatus::Connected))
            .await
            .context("connect timed out")??;
        Ok(Seat {
            conn,
            syncs,
            presence,
            _subs: (sync_sub, presence_sub),
            last: None,
            seen: Vec::new(),
            moves: 0,
        })
    }

    fn absorb(&mut self, ev: SyncEv) -> Result<()> {
        let v = ev.0["v"].as_u64().context("sync without v")?;
        ensure!(
            ev.0["view"]["v"].as_u64() == Some(v),
            "sync v and view.v differ: {}",
            ev.0
        );
        if let Some(prev) = self.seen.last() {
            ensure!(v >= *prev, "sync went backwards: {prev} -> {v}");
        }
        self.seen.push(v);
        self.last = Some(ev.0);
        Ok(())
    }

    /// The next `sync` (blocking).
    async fn next_sync(&mut self, what: &str) -> Result<Value> {
        let ev = recv(&mut self.syncs, what).await?;
        self.absorb(ev)?;
        self.last.clone().context("no sync")
    }

    /// Drain buffered `sync`s, then return the newest view.
    fn drain(&mut self) -> Result<Option<Value>> {
        while let Ok(ev) = self.syncs.try_recv() {
            self.absorb(ev)?;
        }
        Ok(self.last.as_ref().map(|s| s["view"].clone()))
    }

    /// Wait for a view (at `min_v` or later) that satisfies `done`.
    async fn wait_view(
        &mut self,
        min_v: u64,
        what: &str,
        done: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(view) = self.drain()?
                && v_of(&view) >= min_v
                && done(&view)
            {
                return Ok(view);
            }
            ensure!(Instant::now() < deadline, "{what}: timed out");
            self.next_sync(what).await?;
        }
    }

    /// Play the first legal move whenever it is seat 0's turn, until `stop`.
    async fn play_until(
        &mut self,
        min_v: u64,
        what: &str,
        stop: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        let mut floor = min_v;
        loop {
            let view = self
                .wait_view(floor, what, |v| {
                    stop(v) || v["status"] != "active" || is_player_turn(v)
                })
                .await?;
            if stop(&view) || view["status"] != "active" {
                return Ok(view);
            }
            let id = first_legal(&view)?;
            self.moves += 1;
            let cmid = format!("c-{}-{}", self.moves, v_of(&view));
            let ack = call_with_timeout(&self.conn, submit(&id, &view["turnId"], &cmid), CALL)
                .await
                .with_context(|| format!("{what}: submit {id}"))?;
            match ack {
                MoveAck::Ok { v } => floor = v.max(v_of(&view) + 1),
                other => bail!("{what}: move {id} refused: {other:?}"),
            }
        }
    }
}

// ------------------------------------------------------------------ tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn table_auth_end_to_end() -> Result<()> {
    run("table_auth_end_to_end", |engine| async move {
        let r = auth_checks(&engine).await;
        (engine, r)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn table_moves_end_to_end() -> Result<()> {
    run("table_moves_end_to_end", |engine| async move {
        let r = move_checks(&engine).await;
        (engine, r)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn table_match_end_to_end() -> Result<()> {
    run("table_match_end_to_end", |engine| async move {
        let r = match_checks(&engine).await;
        (engine, r)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn table_review_sleep_end_to_end() -> Result<()> {
    run("table_review_sleep_end_to_end", |engine| async move {
        let r = review_sleep_checks(&engine).await;
        (engine, r)
    })
    .await
}

async fn auth_checks(engine: &TestEngine) -> Result<()> {
    let game = game_id("auth");
    let alice = guest();
    let bob = guest();

    // A refused first visitor does not claim the table.
    let bad = handle(
        engine,
        &game,
        json!({ "token": "a.b.c", "protocolVersion": 1 }),
    )?;
    let invalid = "Invalid or expired token";
    // `ready` returns a user error at once: the envoy is serving.
    let _ = engine
        .ready(|| async { bad.call(Snapshot(vec![])).await })
        .await;
    expect_user_error(
        stateless_call_with_timeout(&bad, Snapshot(vec![]), CALL).await,
        "invalid_token",
        invalid,
        "bad token",
    )?;
    ensure!(
        state_of(&game)?["ownerUserId"] == "",
        "a refused visitor claimed the table"
    );

    // A valid guest Actors JWT: HTTP, then WebSocket. Alice claims the table.
    let owner = handle(engine, &game, player_params(&alice)?)?;
    let view = stateless_call_with_timeout(&owner, Snapshot(vec![]), CALL).await?;
    ensure!(
        view["you"] == 0 && view["status"] == "active",
        "snapshot: {view}"
    );
    ensure!(
        state_of(&game)?["ownerUserId"] == alice.as_str(),
        "alice did not claim the table"
    );
    let ws = connect_bounded(&owner, CALL).await?;
    let ws_view = call_with_timeout(&ws, Snapshot(vec![]), CALL).await?;
    ensure!(ws_view["you"] == 0, "ws snapshot: {ws_view}");
    ws.disconnect().await;

    // A live socket keeps the identity it proved at connect: a guest token
    // that expires mid-match does not lock the player out of it (TS kept the
    // identity in conn state). A stateless call with it is refused.
    let lifetime = 2 * 3_600;
    let short = handle(
        engine,
        &game,
        json!({ "token": token_at(&alice, now_secs() - lifetime + 3)?, "protocolVersion": 1 }),
    )?;
    let live = connect_bounded(&short, CALL).await?;
    call_with_timeout(&live, Snapshot(vec![]), CALL).await?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let late = call_with_timeout(&live, Snapshot(vec![]), CALL).await?;
    ensure!(late["you"] == 0, "snapshot after expiry: {late}");
    expect_user_error(
        stateless_call_with_timeout(&short, Snapshot(vec![]), CALL).await,
        "invalid_token",
        "Invalid or expired token",
        "expired stateless token",
    )?;
    live.disconnect().await;

    // Bad tokens, at connect and per call.
    expect_ws_rejected(&bad, "bad token").await?;
    let expired = handle(
        engine,
        &game,
        json!({ "token": token_at(&alice, now_secs() - 7_300)?, "protocolVersion": 1 }),
    )?;
    expect_user_error(
        stateless_call_with_timeout(&expired, Snapshot(vec![]), CALL).await,
        "invalid_token",
        invalid,
        "expired token",
    )?;
    expect_ws_rejected(&expired, "expired token").await?;
    let session_aud = handle(
        engine,
        &game,
        json!({ "token": auth().sign_guest_at(&alice, GuestAudience::Session, now_secs())?, "protocolVersion": 1 }),
    )?;
    expect_user_error(
        stateless_call_with_timeout(&session_aud, Snapshot(vec![]), CALL).await,
        "invalid_token",
        invalid,
        "session-audience token",
    )?;
    let none = handle(engine, &game, json!({}))?;
    expect_user_error(
        stateless_call_with_timeout(&none, Snapshot(vec![]), CALL).await,
        "invalid_token",
        invalid,
        "no token",
    )?;

    // Seat and protocol claims, checked before the token.
    for seat in [json!(1), json!(3), json!("0")] {
        let h = handle(
            engine,
            &game,
            json!({ "token": token_at(&alice, now_secs())?, "protocolVersion": 1, "seat": seat }),
        )?;
        expect_user_error(
            stateless_call_with_timeout(&h, Snapshot(vec![]), CALL).await,
            "seat_mismatch",
            "Seat 0 is the only seat a player may occupy",
            "wrong seat",
        )?;
    }
    let seat0 = handle(
        engine,
        &game,
        json!({ "token": token_at(&alice, now_secs())?, "protocolVersion": 1, "seat": 0 }),
    )?;
    stateless_call_with_timeout(&seat0, Snapshot(vec![]), CALL).await?;
    for version in [json!(2), json!("1"), json!(null)] {
        let h = handle(
            engine,
            &game,
            json!({ "token": "a.b.c", "protocolVersion": version }),
        )?;
        expect_user_error(
            stateless_call_with_timeout(&h, Snapshot(vec![]), CALL).await,
            "protocol_version_mismatch",
            "Unsupported protocol version",
            "protocol version",
        )?;
    }
    let wrong_protocol = handle(
        engine,
        &game,
        json!({ "token": token_at(&alice, now_secs())?, "protocolVersion": 2 }),
    )?;
    expect_ws_rejected(&wrong_protocol, "wrong protocol").await?;

    // Someone else's valid token.
    let foreign = handle(engine, &game, player_params(&bob)?)?;
    expect_user_error(
        stateless_call_with_timeout(&foreign, Snapshot(vec![]), CALL).await,
        "forbidden",
        "Not your game",
        "foreign token",
    )?;
    expect_ws_rejected(&foreign, "foreign token").await?;

    // Origin: an allowed one passes, any other is refused; the internal token
    // passes from anywhere.
    let evil = origin_client(engine, "https://evil.example");
    let evil_owner = table(&evil, &game, player_params(&alice)?, None)?;
    expect_user_error(
        stateless_call_with_timeout(&evil_owner, Snapshot(vec![]), CALL).await,
        "origin_not_allowed",
        "Origin not allowed",
        "bad origin",
    )?;
    expect_ws_rejected(&evil_owner, "bad origin").await?;
    let good = origin_client(engine, ALLOWED_ORIGIN);
    let good_owner = table(&good, &game, player_params(&alice)?, None)?;
    stateless_call_with_timeout(&good_owner, Snapshot(vec![]), CALL).await?;

    // The internal token: a reader, never a player.
    let internal_token = test_support::internal_token(&game).context("internal token")?;
    let internal_params = json!({ "internalToken": internal_token });
    let evil_internal = table(&evil, &game, internal_params.clone(), None)?;
    let iv = stateless_call_with_timeout(&evil_internal, Snapshot(vec![]), CALL).await?;
    ensure!(iv["you"] == 0, "internal snapshot: {iv}");
    let internal = handle(engine, &game, internal_params.clone())?;
    stateless_call_with_timeout(&internal, LastTrick(vec![]), CALL).await?;
    let not_player = "Not a player connection";
    expect_user_error(
        stateless_call_with_timeout(&internal, submit("pass", &json!(null), "c1"), CALL).await,
        "forbidden",
        not_player,
        "internal submitMove",
    )?;
    expect_user_error(
        stateless_call_with_timeout(&internal, GetHandReview(vec![json!(0)]), CALL).await,
        "forbidden",
        not_player,
        "internal getHandReview",
    )?;
    expect_user_error(
        stateless_call_with_timeout(
            &internal,
            ReviewTrick(vec![json!({ "action": "set", "enabled": true })]),
            CALL,
        )
        .await,
        "forbidden",
        not_player,
        "internal reviewTrick",
    )?;
    let wrong_internal = handle(engine, &game, json!({ "internalToken": "nope" }))?;
    expect_user_error(
        stateless_call_with_timeout(&wrong_internal, Snapshot(vec![]), CALL).await,
        "invalid_token",
        invalid,
        "wrong internal token",
    )?;

    // Former queues: internal only; schedule targets: schedule fires only.
    let internal_only = "This action is internal.";
    expect_user_error(
        stateless_call_with_timeout(&owner, TickQueue(vec![json!({})]), CALL).await,
        "internal_only",
        internal_only,
        "player tick",
    )?;
    expect_user_error(
        stateless_call_with_timeout(&owner, ReviewQueue(vec![json!({})]), CALL).await,
        "internal_only",
        internal_only,
        "player review queue",
    )?;
    for (what, r) in [
        (
            "owner onTempoGate",
            stateless_call_with_timeout(&owner, OnTempoGate(vec![json!("t")]), CALL).await,
        ),
        (
            "internal onTempoGate",
            stateless_call_with_timeout(&internal, OnTempoGate(vec![json!("t")]), CALL).await,
        ),
        (
            "internal onReap",
            stateless_call_with_timeout(&internal, OnReap(vec![]), CALL).await,
        ),
    ] {
        expect_user_error(r, "internal_only", internal_only, what)?;
    }
    // A malformed former-queue body from an internal caller.
    expect_user_error(
        stateless_call_with_timeout(&internal, MoveQueue(vec![json!({})]), CALL).await,
        "forbidden",
        not_player,
        "internal move queue",
    )?;
    expect_user_error(
        stateless_call_with_timeout(&internal, ReviewQueue(vec![json!(7)]), CALL).await,
        "internal_error",
        "Invalid review request",
        "bad review body",
    )?;
    ensure!(
        stateless_call_with_timeout(&owner, GetHandReview(vec![json!(0)]), CALL)
            .await?
            .is_none(),
        "review of an unfinished hand"
    );
    for bad_no in [json!(-1), json!(1.5), json!("0"), json!(99)] {
        ensure!(
            stateless_call_with_timeout(&owner, GetHandReview(vec![bad_no.clone()]), CALL)
                .await?
                .is_none(),
            "getHandReview({bad_no})"
        );
    }

    // Presence: an internal watcher sees online on the first player socket and
    // offline on the last; HTTP calls never count.
    let mut watcher = Seat::open(&internal).await?;
    quiet(
        &mut watcher.syncs,
        Duration::from_millis(500),
        "internal sync",
    )
    .await?;
    let first = Seat::open(&owner).await?;
    let online = recv(&mut watcher.presence, "online").await?;
    ensure!(
        online.0 == json!({ "seat": 0, "online": true }),
        "online: {:?}",
        online
    );
    let second = Seat::open(&owner).await?;
    stateless_call_with_timeout(&owner, Snapshot(vec![]), CALL).await?;
    quiet(
        &mut watcher.presence,
        Duration::from_millis(700),
        "second socket",
    )
    .await?;
    first.conn.disconnect().await;
    quiet(
        &mut watcher.presence,
        Duration::from_millis(700),
        "one socket left",
    )
    .await?;
    second.conn.disconnect().await;
    let offline = recv(&mut watcher.presence, "offline").await?;
    ensure!(
        offline.0 == json!({ "seat": 0, "online": false }),
        "offline: {:?}",
        offline
    );
    watcher.conn.disconnect().await;
    drop(evil);
    drop(good);
    eprintln!("auth ok: {game}");
    Ok(())
}

async fn move_checks(engine: &TestEngine) -> Result<()> {
    let game = game_id("moves");
    let alice = guest();
    let owner = handle(engine, &game, player_params(&alice)?)?;
    let snap = ready(engine, &owner).await?;

    // Initial sync: the current view, no steps.
    let mut seat = Seat::open(&owner).await?;
    let first = seat.next_sync("initial sync").await?;
    ensure!(
        first["steps"] == json!([]) && v_of(&first["view"]) >= v_of(&snap),
        "initial sync: {first}"
    );
    ensure!(first["view"]["you"] == 0, "sync view seat: {first}");

    // Wait for seat 0 to act.
    let view = seat.wait_view(0, "first turn", is_player_turn).await?;
    let legal_id = first_legal(&view)?;
    let turn = view["turnId"].clone();

    // An illegal move: the legal set rides on the error.
    let e = expect_user_error(
        call_with_timeout(&seat.conn, submit("not-a-move", &turn, "c-illegal"), CALL).await,
        "illegal_move",
        "That move is not legal right now",
        "illegal move",
    )?;
    ensure!(
        e.metadata.as_ref().map(|m| &m["legal"]) == Some(&view["legal"]),
        "illegal move metadata: {e:?}"
    );
    // A missing clientMoveId, checked after legality.
    expect_user_error(
        call_with_timeout(&seat.conn, submit(&legal_id, &turn, ""), CALL).await,
        "internal_error",
        "Missing clientMoveId",
        "missing clientMoveId",
    )?;

    // A stale turn nonce: a rejected ack, nothing applied.
    let before = applied_seq(&game)?;
    let ack = call_with_timeout(
        &seat.conn,
        submit(&legal_id, &json!("stale-turn"), "c-stale"),
        CALL,
    )
    .await?;
    match &ack {
        MoveAck::Rejected { code, legal } => ensure!(
            code.as_str() == "stale_turn" && !legal.is_empty(),
            "stale ack: {ack:?}"
        ),
        other => bail!("stale turn accepted: {other:?}"),
    }
    ensure!(applied_seq(&game)? == before, "a stale move applied");

    // Concurrent submits of the same move: the lane applies exactly one; the
    // rest are idempotent acks (same seq) or refused once the turn moved on.
    let race = |i: usize| submit(&legal_id, &turn, &format!("c-race-{i}"));
    let (r0, r1, r2, r3, r4) = tokio::join!(
        stateless_call_with_timeout(&owner, race(0), CALL),
        stateless_call_with_timeout(&owner, race(1), CALL),
        stateless_call_with_timeout(&owner, race(2), CALL),
        call_with_timeout(&seat.conn, race(3), CALL),
        call_with_timeout(&seat.conn, race(4), CALL),
    );
    let results = [r0, r1, r2, r3, r4];
    let mut oks = 0;
    for r in results {
        match r {
            Ok(MoveAck::Ok { .. }) => oks += 1,
            Ok(other) => ensure!(
                matches!(&other, MoveAck::Rejected { code, .. } if code.as_str() == "stale_turn"),
                "race ack: {other:?}"
            ),
            Err(e) => {
                let p = parse_rivet_error(&e).with_context(|| format!("race error `{e:#}`"))?;
                ensure!(p.is("illegal_move"), "race error {p:?}");
            }
        }
    }
    ensure!(oks >= 1, "no racing submit landed");
    ensure!(
        applied_seq(&game)? == before + 1,
        "racing submits applied {} moves",
        applied_seq(&game)? - before
    );
    let journal = state_of(&game)?["journal"].clone();
    let human_moves = journal
        .as_array()
        .map(|j| j.iter().filter(|a| a["seat"] == 0).count())
        .unwrap_or(0);
    ensure!(human_moves == 1, "journal has {human_moves} seat-0 moves");

    // A duplicate clientMoveId on the next turn: idempotent, nothing applied.
    let landed = state_of(&game)?;
    let remembered = landed["game"]["recentMoveIds"]
        .as_array()
        .and_then(|ids| ids.last())
        .and_then(Value::as_str)
        .context("no remembered clientMoveId")?
        .to_owned();
    ensure!(remembered.starts_with("c-race-"), "remembered {remembered}");
    let floor = landed["game"]["v"].as_u64().context("v")?;
    let view = seat
        .play_until(floor, "next turn", |v| {
            is_player_turn(v) && v["turnId"] != turn
        })
        .await?;
    if view["status"] == "active" {
        let seq = applied_seq(&game)?;
        let id = first_legal(&view)?;
        let ack =
            call_with_timeout(&seat.conn, submit(&id, &view["turnId"], &remembered), CALL).await?;
        ensure!(ack.is_ok(), "duplicate clientMoveId: {ack:?}");
        ensure!(
            applied_seq(&game)? == seq,
            "a duplicate clientMoveId applied"
        );
        let state = state_of(&game)?;
        ensure!(
            state["game"]["turnId"] == view["turnId"],
            "the turn moved on a duplicate"
        );
    }
    ensure!(seat.seen.len() >= 2, "too few syncs: {:?}", seat.seen);
    seat.conn.disconnect().await;
    eprintln!("moves ok: {game}");
    Ok(())
}

async fn match_checks(engine: &TestEngine) -> Result<()> {
    let game = game_id("match");
    let alice = guest();
    let owner = handle(engine, &game, player_params(&alice)?)?;
    ready(engine, &owner).await?;
    let mut seat = Seat::open(&owner).await?;
    seat.next_sync("initial sync").await?;
    let started = Instant::now();

    // One full hand.
    let view = seat
        .play_until(0, "first hand", |v| {
            v["handNo"].as_u64().unwrap_or(0) >= 2 || v["status"] != "active"
        })
        .await?;
    let state = state_of(&game)?;
    ensure!(
        state["handsPlayed"].as_u64() >= Some(1),
        "handsPlayed after a hand: {}",
        state["handsPlayed"]
    );
    let review = stateless_call_with_timeout(&owner, GetHandReview(vec![json!(1)]), CALL)
        .await?
        .context("no review of hand 1")?;
    let rj = serde_json::to_value(&review)?;
    ensure!(rj["handNo"] == 1, "review: {rj}");
    let last = stateless_call_with_timeout(&owner, LastTrick(vec![]), CALL).await?;
    eprintln!(
        "hand 1 done in {:?}: score {} last trick {}",
        started.elapsed(),
        view["score"],
        last.is_some()
    );

    // The rest of the match.
    let end = seat
        .play_until(v_of(&view), "match", |v| v["status"] != "active")
        .await?;
    ensure!(end["status"] == "complete", "match end: {end}");
    let score: Vec<u64> = end["score"]
        .as_array()
        .context("score")?
        .iter()
        .filter_map(Value::as_u64)
        .collect();
    let winner = end["winnerTeam"].as_u64().context("winnerTeam")?;
    ensure!(
        score.len() == 2 && score[usize::try_from(winner)?] >= 10,
        "final score {score:?} winner {winner}"
    );
    let state = state_of(&game)?;
    ensure!(
        state["game"]["status"] == "complete",
        "persisted status {}",
        state["game"]["status"]
    );
    let hands = state["handsPlayed"].as_u64().unwrap_or(0);
    let final_hand = end["handNo"].as_u64().context("handNo")?;
    for n in [1, final_hand] {
        ensure!(
            stateless_call_with_timeout(&owner, GetHandReview(vec![json!(n)]), CALL)
                .await?
                .is_some(),
            "no review of hand {n}"
        );
    }

    // A finished match takes no moves.
    expect_user_error(
        call_with_timeout(&seat.conn, submit("pass", &end["turnId"], "c-late"), CALL).await,
        "game_complete",
        "This match is over",
        "move after game over",
    )?;
    let snap = call_with_timeout(&seat.conn, Snapshot(vec![]), CALL).await?;
    ensure!(snap["status"] == "complete", "snapshot after end: {snap}");
    eprintln!(
        "match ok: {game} {hands} hands, {} player moves, {} syncs, score {score:?} in {:?}",
        seat.moves,
        seat.seen.len(),
        started.elapsed()
    );
    seat.conn.disconnect().await;
    Ok(())
}

/// Disconnect, wait for the idle sleep, and return the start count before it.
async fn sleep_now(game: &str, seat: Seat) -> Result<u64> {
    let starts = test_support::starts(game);
    let sleeps = test_support::sleeps(game);
    seat.conn.disconnect().await;
    let deadline = Instant::now() + Duration::from_secs(30);
    while test_support::sleeps(game) == sleeps {
        ensure!(Instant::now() < deadline, "the table never slept");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(starts)
}

async fn wake(
    engine: &TestEngine,
    owner: &TypedActorHandle<EuchreTable>,
    game: &str,
    starts: u64,
) -> Result<Seat> {
    tokio::time::sleep(Duration::from_millis(500)).await;
    ready(engine, owner).await?;
    ensure!(
        test_support::starts(game) == starts + 1,
        "wake count {starts} -> {}",
        test_support::starts(game)
    );
    Seat::open(owner).await
}

async fn review_sleep_checks(engine: &TestEngine) -> Result<()> {
    let game = game_id("review");
    let alice = guest();
    let owner = handle(engine, &game, player_params(&alice)?)?;
    ready(engine, &owner).await?;
    let mut seat = Seat::open(&owner).await?;
    seat.next_sync("initial sync").await?;

    // Turn the gate on.
    let ack = call_with_timeout(
        &seat.conn,
        ReviewTrick(vec![json!({ "action": "set", "enabled": true })]),
        CALL,
    )
    .await?;
    ensure!(
        serde_json::to_value(ack)? == json!({ "queued": true }),
        "reviewTrick ack"
    );
    let on = seat
        .wait_view(0, "review on", |v| v["reviewTricks"] == true)
        .await?;
    expect_user_error(
        call_with_timeout(
            &seat.conn,
            ReviewTrick(vec![json!({ "action": "x" })]),
            CALL,
        )
        .await,
        "internal_error",
        "Invalid review request",
        "bad review request",
    )?;

    // Play to the first finished trick; it waits for the player.
    let held = seat
        .play_until(v_of(&on), "first trick", |v| {
            v["awaitingTrickReview"] == true
        })
        .await?;
    ensure!(held["phase"] == "trick_resolve", "held: {held}");
    let held_turn = held["turnId"].clone();
    tokio::time::sleep(Duration::from_millis(800)).await;
    let still = seat.drain()?.context("view")?;
    ensure!(
        still["turnId"] == held_turn && still["awaitingTrickReview"] == true,
        "the gate let go: {still}"
    );
    // A Continue for another turn is ignored.
    call_with_timeout(
        &seat.conn,
        ReviewTrick(vec![json!({ "action": "continue", "turnId": "other" })]),
        CALL,
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let still = seat.drain()?.context("view")?;
    ensure!(still["turnId"] == held_turn, "a stale Continue advanced");
    let last = call_with_timeout(&seat.conn, LastTrick(vec![]), CALL)
        .await?
        .context("no last trick at the gate")?;
    ensure!(
        serde_json::to_value(&last)?["plays"]
            .as_array()
            .is_some_and(|p| !p.is_empty()),
        "last trick: {last:?}"
    );

    // Sleep mid-hand at the gate; wake with the same state.
    let before = state_of(&game)?;
    let starts = sleep_now(&game, seat).await?;
    let mut seat = wake(engine, &owner, &game, starts).await?;
    let woke = seat.next_sync("sync after wake").await?;
    ensure!(
        woke["steps"] == json!([])
            && woke["view"]["turnId"] == held_turn
            && woke["view"]["awaitingTrickReview"] == true
            && v_of(&woke["view"]) == v_of(&still),
        "view after wake: {woke}"
    );
    let after = state_of(&game)?;
    ensure!(
        after["game"] == before["game"] && after["journal"] == before["journal"],
        "state changed across sleep"
    );

    // Continue: the trick clears and the AI timers run again after the wake.
    call_with_timeout(
        &seat.conn,
        ReviewTrick(vec![json!({ "action": "continue", "turnId": held_turn })]),
        CALL,
    )
    .await?;
    let moved = seat
        .wait_view(v_of(&still) + 1, "after continue", |v| {
            v["turnId"] != held_turn
        })
        .await?;
    ensure!(moved["phase"] != "trick_resolve", "continue: {moved}");

    // Gate off; play to a player turn mid-hand, sleep there, wake, play on.
    call_with_timeout(
        &seat.conn,
        ReviewTrick(vec![json!({ "action": "set", "enabled": false })]),
        CALL,
    )
    .await?;
    let turn = seat
        .play_until(v_of(&moved), "player turn", |v| {
            is_player_turn(v) && v["reviewTricks"] == false
        })
        .await?;
    let hand_no = turn["handNo"].clone();
    let starts = sleep_now(&game, seat).await?;
    let mut seat = wake(engine, &owner, &game, starts).await?;
    let woke = seat.next_sync("sync after second wake").await?;
    ensure!(
        woke["view"]["turnId"] == turn["turnId"] && woke["view"]["handNo"] == hand_no,
        "view after second wake: {woke}"
    );
    let next = seat
        .play_until(v_of(&turn), "play after wake", |v| {
            v["handNo"] != hand_no || v["status"] != "active"
        })
        .await?;
    ensure!(
        next["handNo"] != hand_no || next["status"] != "active",
        "the hand did not finish after the wake: {next}"
    );
    eprintln!(
        "review + sleep ok: {game} starts {} sleeps {}",
        test_support::starts(&game),
        test_support::sleeps(&game)
    );
    seat.conn.disconnect().await;
    Ok(())
}
