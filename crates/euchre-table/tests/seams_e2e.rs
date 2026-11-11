//! The server seams, end to end (stage SERVER-SEAMS): the REAL `euchreTable`,
//! `aiSeat` and `playerProfile` on one engine, talking only by actor name and
//! action name over the shared client (D29).
//!
//! Prints `SKIP` and returns unless `EUCHRE_E2E=1`. Run:
//!
//! ```text
//! EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6841 EUCHRE_TEST_STORAGE=<dir> \
//!   cargo test -p euchre-table --test seams_e2e -- --nocapture
//! ```
//!
//! One guest plays a whole match against three real AI seats driven by a
//! scripted model (`euchre_ai::testing`, no network). Every call between the
//! actors is the production one:
//!
//! - table → aiSeat: `decide`, `resetHand`, `handEnd`, `gameEnd` on
//!   `["table", gameId, "seat", n]` with `{ internalToken }` params;
//! - aiSeat → table: `aiDecision`, `aiSay` on `["table", gameId]` with
//!   `{ internalToken }` params;
//! - table → playerProfile: `recordHands`, `recordGame`;
//! - aiSeat → playerProfile: `recordEpisodes`, `recordTokens`;
//!
//! all on `["user", userId]` with the deployment internal token in the body.
//! The test then reads the guest's profile with an owner token (`listGames`,
//! `getStats`, `getReplay`, `listEpisodes`) and checks that the match, every
//! hand, the episodes and the token usage landed.

use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use euchre_ai::ai_seat::bridge::{LlmProvider, ProviderDeps};
use euchre_ai::ai_seat::handlers::messages;
use euchre_ai::clock::SystemClock;
use euchre_ai::testing::{
    RecordingLog, ScriptedJev, ScriptedResult, ScriptedTalk, jev_choice, jev_factory,
};
use euchre_ai_seat::{AiSeatRuntime, SeatClient};
use euchre_profile::{GetReplay, GetStats, ListEpisodes, ListGames, PlayerProfile};
use euchre_protocol::{MoveAck, TableCreateInput, default_personas};
use euchre_server_core::auth::{AuthKeys, Authenticator, GuestAudience, now_secs};
use euchre_server_core::config::{OriginPolicy, Secret};
use euchre_server_core::rivet::{call_with_timeout, forward_events, stateless_call_with_timeout};
use euchre_server_core::testing::{TestEngine, TestEngineOptions, e2e_enabled, e2e_skip};
use euchre_table::{
    ACTOR_NAME, EuchreTable, Snapshot, SubmitMove, TableClient, TableConfig, actor_key,
    register_with, test_support,
};
use rivetkit::client::{Client, ConnectionStatus, GetOrCreateOptions, SubscriptionHandle};
use rivetkit::{ActorConfig, Event, TypedActorConnection, TypedActorHandle, TypedClientExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

// ------------------------------------------------------------------ fixtures

const CALL: Duration = Duration::from_secs(15);
const EVENT: Duration = Duration::from_secs(20);
const ALLOWED_ORIGIN: &str = "https://app.example";
/// `EUCHRE_INTERNAL_TOKEN` of this deployment: every profile write carries it.
const INTERNAL: &str = "e2e-internal-token-0123456789abcdef";
/// The seats' queue-handler failure line (`actor.rs`), e.g. a lost `aiDecision`.
const SEAT_HANDLER_FAILED: &str = "aiSeat queue handler failed";

/// The table's one client slot in this process (`configure` compares it).
static CLIENT: LazyLock<TableClient> = LazyLock::new(TableClient::deferred);

/// One key set for all three actors, as one deployment's `ServerConfig`.
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

fn table_config() -> Result<TableConfig> {
    Ok(TableConfig {
        auth: auth(),
        origins: OriginPolicy::strict(vec![ALLOWED_ORIGIN.to_owned()])?,
        fast_tempo: true,
        client: CLIENT.clone(),
        profile_token: Some(Secret::new(INTERNAL)),
    })
}

/// The seats' runtime: a scripted model that always picks `move_1` (10 input
/// tokens a call) and says one line, so decisions are model-sourced and token
/// usage is non-zero, with no network.
fn seat_runtime(log: Arc<RecordingLog>, client: SeatClient) -> AiSeatRuntime {
    let jev = Arc::new(ScriptedJev::new(vec![Ok(jev_choice("move_1", 0.9))]));
    let talk = Arc::new(ScriptedTalk::new(vec![Ok(ScriptedResult::text("Mine."))]));
    let factory = Arc::new(jev_factory(jev, talk));
    let provider = LlmProvider::from_factory(
        Some(factory),
        ProviderDeps {
            clock: Arc::new(SystemClock),
            log: log.clone(),
        },
    );
    AiSeatRuntime::new(client, provider, Some(INTERNAL.to_owned())).with_log(log)
}

fn guest() -> String {
    format!("guest-{}", uuid::Uuid::new_v4())
}

fn player_params(user: &str) -> Result<Value> {
    let token = auth().sign_guest_at(user, GuestAudience::Actors, now_secs())?;
    Ok(json!({ "token": token, "protocolVersion": 1 }))
}

fn owner_params(user: &str) -> Result<Value> {
    let token = auth().mint_profile_token(user)?;
    Ok(json!({ "token": token, "protocolVersion": 1 }))
}

/// A table owned by `user` with the three default personas.
fn table(client: &Client, game: &str, user: &str) -> Result<TypedActorHandle<EuchreTable>> {
    let input = serde_json::to_value(TableCreateInput {
        // As the lobby creates it (`POST /api/tables`, TS `/play`): the owner
        // is known up front, so every seat is created with its `profileKey`.
        owner_user_id: Some(user.to_owned()),
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

/// Production-like idle sleeps, short enough that the table and the seats
/// sleep and wake during the match.
fn quick() -> ActorConfig {
    ActorConfig {
        sleep_timeout: Duration::from_secs(1),
        ..ActorConfig::default()
    }
}

fn idle() -> ActorConfig {
    ActorConfig {
        sleep_timeout: Duration::from_secs(2),
        ..ActorConfig::default()
    }
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
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ------------------------------------------------------------- the player

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(transparent)]
struct SyncEv(Value);
impl Event for SyncEv {
    const NAME: &'static str = "sync";
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(transparent)]
struct ChatEv(Value);
impl Event for ChatEv {
    const NAME: &'static str = "chat";
}

fn is_player_turn(view: &Value) -> bool {
    view["status"] == "active"
        && view["turnSeat"] == 0
        && view["legal"].as_array().is_some_and(|l| !l.is_empty())
}

/// The seat-0 WebSocket.
struct Player {
    conn: TypedActorConnection<EuchreTable>,
    syncs: UnboundedReceiver<SyncEv>,
    chats: UnboundedReceiver<ChatEv>,
    _subs: Vec<SubscriptionHandle>,
    last: Option<Value>,
    moves: u64,
}

impl Player {
    async fn open(h: &TypedActorHandle<EuchreTable>) -> Result<Player> {
        let conn = h.connect();
        let (s1, syncs) = forward_events::<EuchreTable, SyncEv>(&conn).await;
        let (s2, chats) = forward_events::<EuchreTable, ChatEv>(&conn).await;
        let mut status = conn.inner().status_receiver();
        tokio::time::timeout(CALL, status.wait_for(|s| *s == ConnectionStatus::Connected))
            .await
            .context("connect timed out")??;
        Ok(Player {
            conn,
            syncs,
            chats,
            _subs: vec![s1, s2],
            last: None,
            moves: 0,
        })
    }

    async fn next_view(&mut self) -> Result<Value> {
        let ev = match tokio::time::timeout(EVENT, self.syncs.recv()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => bail!("sync channel closed"),
            Err(_) => bail!("no sync within {EVENT:?}"),
        };
        self.last = Some(ev.0["view"].clone());
        while let Ok(ev) = self.syncs.try_recv() {
            self.last = Some(ev.0["view"].clone());
        }
        self.last.clone().context("no view")
    }

    /// Play the first legal move on every seat-0 turn until game over.
    async fn play_to_end(&mut self) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(300);
        let mut view = self.next_view().await?;
        loop {
            ensure!(Instant::now() < deadline, "match timed out");
            if view["status"] != "active" {
                return Ok(view);
            }
            if is_player_turn(&view) {
                let id = view["legal"][0]["id"]
                    .as_str()
                    .context("legal id")?
                    .to_owned();
                self.moves += 1;
                let submit = SubmitMove(vec![json!({
                    "moveId": id,
                    "turnId": view["turnId"],
                    "clientMoveId": format!("c-{}", self.moves),
                })]);
                let ack = call_with_timeout(&self.conn, submit, CALL)
                    .await
                    .with_context(|| format!("submit {id}"))?;
                ensure!(
                    matches!(ack, MoveAck::Ok { .. }),
                    "move {id} refused: {ack:?}"
                );
            }
            view = self.next_view().await?;
        }
    }

    fn chats(&mut self) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(ev) = self.chats.try_recv() {
            out.push(ev.0);
        }
        out
    }
}

// ------------------------------------------------------------------ the test

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn seams_full_match_end_to_end() -> Result<()> {
    const NAME: &str = "seams_full_match_end_to_end";
    if !e2e_enabled() {
        e2e_skip(NAME);
        return Ok(());
    }
    init_tracing();
    test_support::enable();
    euchre_ai_seat::test_support::enable();
    euchre_profile::test_support::enable();
    euchre_table::configure(table_config()?)?;

    let log = Arc::new(RecordingLog::new());
    let seat_client = SeatClient::deferred();
    let runtime = seat_runtime(log.clone(), seat_client.clone());
    let cfg = table_config()?;
    let engine = TestEngine::start(TestEngineOptions::from_env(move |registry| {
        if let Err(e) = register_with(registry, cfg, quick()) {
            panic!("register euchreTable: {e}");
        }
        euchre_ai_seat::register_with(registry, runtime, idle());
        if let Err(e) = euchre_profile::register_with(registry, auth(), idle()) {
            panic!("register playerProfile: {e}");
        }
    })?)
    .await?;
    CLIENT.bind(engine.client());
    ensure!(seat_client.bind(engine.client()), "seat client bound twice");
    let result = full_match(&engine, &log).await;
    CLIENT.unbind();
    engine.shutdown().await?;
    result
}

/// Most matches to play while waiting for one that yields an episode. A seat
/// notes a hand only when its last asked-for view already shows the outcome
/// (forced moves are never asked, as in TS), so some matches have none.
const MAX_MATCHES: usize = 4;

/// What one finished match left behind.
struct Played {
    match_id: String,
    hands_played: u64,
    outcome: &'static str,
    score: Value,
    /// Per seat: `(persona id, notes, budget)` from the seat's persisted state.
    seats: Vec<(String, Vec<String>, Value)>,
}

impl Played {
    fn expected_episodes(&self) -> usize {
        self.seats.iter().map(|(_, notes, _)| notes.len()).sum()
    }
}

async fn full_match(engine: &TestEngine, log: &RecordingLog) -> Result<()> {
    let user = guest();
    let mut played: Vec<Played> = Vec::new();
    for _ in 0..MAX_MATCHES {
        let p = play_one(engine, &user).await?;
        let more = p.expected_episodes() == 0;
        played.push(p);
        if !more {
            break;
        }
    }
    let owner = engine.client().get_or_create_typed::<PlayerProfile>(
        euchre_profile::ACTOR_NAME,
        euchre_profile::actor_key(&user).to_vec(),
        GetOrCreateOptions {
            params: Some(owner_params(&user)?),
            ..Default::default()
        },
    )?;

    // listGames: every match, complete, with the table's counts.
    let page = serde_json::to_value(
        stateless_call_with_timeout(&owner, ListGames::new(None), CALL).await?,
    )?;
    let rows = page["rows"].as_array().cloned().unwrap_or_default();
    ensure!(rows.len() == played.len(), "listGames rows: {page}");
    for p in &played {
        let row = rows
            .iter()
            .find(|r| r["matchId"] == p.match_id.as_str())
            .with_context(|| format!("listGames lacks {}: {page}", p.match_id))?;
        ensure!(
            row["status"] == "complete"
                && row["outcome"] == p.outcome
                && row["handsPlayed"] == json!(p.hands_played)
                && row["score"] == p.score,
            "listGames row {row} vs table {} {} {}",
            p.outcome,
            p.hands_played,
            p.score
        );
    }

    // getStats: every match decided.
    let stats =
        serde_json::to_value(stateless_call_with_timeout(&owner, GetStats::new(), CALL).await?)?;
    let won = played.iter().filter(|p| p.outcome == "won").count();
    ensure!(
        stats["matches"] == json!(played.len())
            && stats["won"] == json!(won)
            && stats["lost"] == json!(played.len() - won),
        "getStats {stats} for {} matches, {won} won",
        played.len()
    );

    // listEpisodes: exactly the hands each seat noted, under its persona id.
    let expected: usize = played.iter().map(Played::expected_episodes).sum();
    let episodes = {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let got = serde_json::to_value(
                stateless_call_with_timeout(&owner, ListEpisodes::new(Some(json!(200))), CALL)
                    .await?,
            )?;
            let all = got.as_array().cloned().unwrap_or_default();
            if all.len() >= expected || Instant::now() >= deadline {
                break all;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    };
    ensure!(
        expected > 0,
        "no seat noted a hand in {} matches; seats: {}",
        played.len(),
        played
            .iter()
            .map(|p| format!("{:?}", p.seats))
            .collect::<Vec<_>>()
            .join(" ")
    );
    for p in &played {
        for (role, notes, _) in &p.seats {
            let mut got: Vec<String> = episodes
                .iter()
                .filter(|e| e["matchId"] == p.match_id.as_str() && e["role"] == role.as_str())
                .filter_map(|e| e["summary"].as_str().map(str::to_owned))
                .collect();
            let mut want = notes.clone();
            got.sort();
            want.sort();
            ensure!(
                got == want,
                "episodes of {role} in {}: {got:?}, seat noted {want:?}",
                p.match_id
            );
        }
    }
    ensure!(episodes.len() == expected, "listEpisodes: {episodes:?}");
    println!(
        "seams: {expected} episodes landed over {} matches",
        played.len()
    );

    // recordTokens: the profile's daily usage equals every seat's budget
    // (no read action exposes it, D12; read the persisted state).
    let budgets: Vec<&Value> = played
        .iter()
        .flat_map(|p| p.seats.iter().map(|(_, _, b)| b))
        .collect();
    let sum = |k: &str| -> u64 { budgets.iter().filter_map(|b| b[k].as_u64()).sum() };
    let (tokens_in, calls) = (sum("tokensIn"), sum("calls"));
    ensure!(tokens_in > 0 && calls > 0, "seat budgets {budgets:?}");
    let daily = eventually("profile dailyTokens", Duration::from_secs(20), || {
        euchre_profile::test_support::last_state(&user)
            .map(|s| s["dailyTokens"].clone())
            .filter(|d| d["tokensIn"].as_u64() == Some(tokens_in))
    })
    .await
    .with_context(|| {
        format!(
            "dailyTokens {:?} vs seats tokensIn {tokens_in}",
            euchre_profile::test_support::last_state(&user).map(|s| s["dailyTokens"].clone())
        )
    })?;
    ensure!(
        daily["calls"].as_u64() == Some(calls)
            && daily["tokensOut"].as_u64() == Some(sum("tokensOut"))
            && daily["cacheReadTokens"].as_u64() == Some(sum("cacheReadTokens")),
        "dailyTokens {daily} vs seat budgets {budgets:?}"
    );
    println!("seams: dailyTokens {daily}");

    // The scripted model always answers `move_1`; where that is not a
    // candidate the seat logs `jev_failure` and the table plays its fallback.
    let mut reasons: std::collections::BTreeMap<String, u64> = Default::default();
    for l in log.lines().into_iter().filter(|l| l.event == "jev_failure") {
        let r = l
            .fields
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("?");
        *reasons.entry(r.to_owned()).or_default() += 1;
    }
    println!("seams: model failures by reason {reasons:?}");

    // No callback into the table, and no profile write, was lost.
    let failed: Vec<_> = log
        .lines()
        .into_iter()
        .filter(|l| {
            l.event == SEAT_HANDLER_FAILED
                || l.event == messages::BANTER_UNDELIVERED
                || l.event == messages::FLUSH_EPISODES_FAILED
                || l.event == messages::FLUSH_TOKENS_FAILED
        })
        .collect();
    ensure!(failed.is_empty(), "seat calls failed: {failed:?}");
    Ok(())
}

/// One whole match for `user` against the three real seats, checked against
/// the table's persisted state and the profile's replay.
async fn play_one(engine: &TestEngine, user: &str) -> Result<Played> {
    let game = format!("seams-{}", uuid::Uuid::new_v4().simple());
    let h = table(&engine.client(), &game, user)?;
    engine
        .ready(|| async { h.call(Snapshot(vec![])).await })
        .await?;
    let mut player = Player::open(&h).await?;
    let started = Instant::now();
    let end = player.play_to_end().await?;
    ensure!(
        end["status"] == "complete",
        "match ended as {}",
        end["status"]
    );
    let chat_lines = player.chats().len();
    let moves = player.moves;
    drop(player);

    // The table recorded the match (recordGame accepted by the real profile).
    let st = eventually("table recordedAt", Duration::from_secs(20), || {
        test_support::last_state(&game).filter(|s| !s["recordedAt"].is_null())
    })
    .await?;
    let match_id = st["matchId"].as_str().context("matchId")?.to_owned();
    let hands_played = st["handsPlayed"].as_u64().context("handsPlayed")?;
    let throw_ins = st["stats"]["throwIns"].as_u64().unwrap_or(0);
    let outcome = if st["game"]["winnerTeam"] == 0 {
        "won"
    } else {
        "lost"
    };

    // getReplay: every finished hand (throw-ins included) through recordHands,
    // with the seats' model decisions (aiDecision) in the move journal.
    let owner = engine.client().get_or_create_typed::<PlayerProfile>(
        euchre_profile::ACTOR_NAME,
        euchre_profile::actor_key(user).to_vec(),
        GetOrCreateOptions {
            params: Some(owner_params(user)?),
            ..Default::default()
        },
    )?;
    let replay = serde_json::to_value(
        stateless_call_with_timeout(&owner, GetReplay::new(match_id.as_str()), CALL).await?,
    )?;
    ensure!(!replay.is_null(), "getReplay({match_id}) is null");
    let hands = replay["hands"].as_array().cloned().unwrap_or_default();
    ensure!(
        hands.len() as u64 == hands_played + throw_ins,
        "getReplay hands {} for {hands_played} hands + {throw_ins} throw-ins",
        hands.len()
    );
    ensure!(
        replay["handsPlayed"] == json!(hands_played) && replay["status"] == "complete",
        "getReplay header {} {}",
        replay["handsPlayed"],
        replay["status"]
    );
    let mut sources: std::collections::BTreeMap<String, u64> = Default::default();
    for m in hands.iter().filter_map(|h| h["moves"].as_array()).flatten() {
        if m["kind"] == "ai" {
            let s = m["source"].as_str().unwrap_or("?").to_owned();
            *sources.entry(s).or_default() += 1;
        }
    }
    ensure!(
        sources.get("llm").copied().unwrap_or(0) > 0,
        "no AI move came back through aiDecision: {sources:?}"
    );

    // Each seat's final persisted state: its notes (one per episode) and budget.
    let mut seats = Vec::new();
    for seat in 1u8..=3 {
        let label = euchre_ai_seat::test_support::label(&game, seat);
        let s = eventually("seat gameEnd state", Duration::from_secs(10), || {
            euchre_ai_seat::test_support::last_state(&label)
        })
        .await?;
        ensure!(
            s["profileKey"] == json!(["user", user]) && s["matchId"] == match_id.as_str(),
            "seat {seat} create input: profileKey {} matchId {}",
            s["profileKey"],
            s["matchId"]
        );
        let notes: Vec<String> = s["notes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|n| n.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        ensure!(notes.len() < 8, "seat {seat} notes hit the cap: {notes:?}");
        let role = s["persona"]["id"]
            .as_str()
            .context("persona id")?
            .to_owned();
        seats.push((role, notes, s["budget"].clone()));
    }
    let played = Played {
        match_id,
        hands_played,
        outcome,
        score: end["score"].clone(),
        seats,
    };
    println!(
        "seams: {} complete in {:?}: {hands_played} hands, {} player moves, AI sources \
         {sources:?}, {chat_lines} chat lines, {} hands noted",
        played.match_id,
        started.elapsed(),
        moves,
        played.expected_episodes()
    );
    Ok(played)
}
