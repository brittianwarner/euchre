//! Engine-backed tests of the `playerProfile` actor (lane SERVER-PROFILE).
//!
//! Each test prints `SKIP` and returns unless `EUCHRE_E2E=1`, so a plain
//! `cargo test` stays engine-free. Run:
//!
//! ```text
//! EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6721 EUCHRE_TEST_STORAGE=<dir> \
//!   cargo test -p euchre-profile --test profile_e2e -- --nocapture
//! ```
//!
//! The tests share one engine and each registers its own pool
//! (`testing::TestEngine`). Every test uses fresh user ids, so they run in
//! parallel. Coverage:
//!
//! - `profile_auth_end_to_end`: owner token accepted over HTTP and WebSocket; a
//!   bad token, another user's token and a bad protocol version refused at
//!   connect with the TS codes and messages; a token-less connection refused
//!   by every browser action; internal actions refused without the token.
//! - `profile_actions_end_to_end`: every action. Settings save, patch errors and
//!   personas; `setActiveGame`; `recordGame` updating stats and the lifetime
//!   cache; `recordHands`; a finished match staying immutable; invalid bodies
//!   dropped; `listGames` pagination and a bad cursor; `getReplay` redaction
//!   against `getServerReplay`; episodes recorded, listed, ranked, forgotten and
//!   pruned; `recordTokens`.
//! - `profile_durability_end_to_end`: concurrent writes serialized by the lane
//!   (counters and a racing insert), then an idle sleep and a wake that reloads
//!   state and SQLite unchanged.
//! - `profile_token_overflow_end_to_end` (W3 follow-up): two matches with
//!   near-maximum token counters, then a normal one. Every `recordGame` lands,
//!   the stored counters are clamped and the stats totals saturate at JS
//!   `Number.MAX_SAFE_INTEGER` instead of overflowing SQLite's `SUM`.

use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use euchre_profile::{
    ACTOR_NAME, ForgetEpisode, GetActiveGame, GetEpisodesForRole, GetPersonas, GetReplay,
    GetServerReplay, GetSettings, GetStats, ListEpisodes, ListGames, PlayerProfile, PruneEpisodes,
    RecordEpisodes, RecordGame, RecordHands, RecordTokens, SetActiveGame, UpdateSettings,
    actor_key, configure, register_with, test_support,
};
use euchre_server_core::auth::{AuthKeys, Authenticator};
use euchre_server_core::config::Secret;
use euchre_server_core::rivet::{
    CallError, UserError, call_with_timeout, connect_bounded, parse_rivet_error,
    stateless_call_with_timeout,
};
use euchre_server_core::testing::{TestEngine, TestEngineOptions, e2e_enabled, e2e_skip};
use rivetkit::client::GetOrCreateOptions;
use rivetkit::{ActorConfig, TypedActorHandle, TypedClientExt};
use serde_json::{Value, json};

// ------------------------------------------------------------------ fixtures

const INTERNAL: &str = "e2e-internal-token-0123456789abcdef";
const CALL: Duration = Duration::from_secs(15);

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

fn user(tag: &str) -> String {
    format!("{tag}_{}", uuid::Uuid::new_v4().simple())
}

fn owner_params(user_id: &str) -> Result<Value> {
    let token = auth().mint_profile_token(user_id)?;
    Ok(json!({ "token": token, "protocolVersion": 1 }))
}

fn handle(
    engine: &TestEngine,
    profile_of: &str,
    params: Value,
) -> Result<TypedActorHandle<PlayerProfile>> {
    engine.client().get_or_create_typed::<PlayerProfile>(
        ACTOR_NAME,
        actor_key(profile_of),
        GetOrCreateOptions {
            params: Some(params),
            ..Default::default()
        },
    )
}

fn persona(seat: u8, id: &str, name: &str) -> Value {
    json!({
        "seat": seat,
        "persona": {
            "id": id, "version": 1, "name": name, "blurb": "b", "prompt": "p",
            "housePrompt": "h", "difficulty": "casual", "aggression": 0.5, "risk": 0.5,
            "chattiness": 0.5, "temperature": 0.6, "modelId": "claude-haiku-4-5",
            "bidModelId": "claude-opus-5"
        },
        "dossier": ""
    })
}

/// A `MatchRecord` for `owner`. `ended_at: None` is a live match.
fn record(owner: &str, match_id: &str, ended_at: Option<i64>, outcome: &str) -> Value {
    let status = if ended_at.is_some() {
        "complete"
    } else {
        "active"
    };
    json!({
        "schema": 1,
        "matchId": match_id,
        "userId": owner,
        "seed": format!("seed-secret-{match_id}"),
        "firstDealer": 1,
        "startedAt": 1_761_955_200_000_i64,
        "endedAt": ended_at,
        "status": status,
        "outcome": outcome,
        "score": [10, 7],
        "winnerTeam": 0,
        "handsPlayed": 2,
        "personas": [persona(1, "marge", "Marge"), persona(2, "dutch", "Dutch"), persona(3, "verna", "Verna")],
        "stats": {
            "euchresFor": 2, "euchresAgainst": 1, "lonersAttempted": 1, "lonersMade": 1,
            "marches": 1, "throwIns": 0, "tricks": { "0": 7, "1": 3 }
        },
        "tokens": { "tokensIn": 100, "tokensOut": 20, "cacheReadTokens": 5, "calls": 3 }
    })
}

/// A hand whose journal holds a buried card (seat 1 discard) the client must
/// never see, plus a visible discard by seat 0.
fn hand(match_id: &str, hand_no: i64) -> Value {
    json!({
        "matchId": match_id,
        "handNo": hand_no,
        "seed": format!("hand-seed-secret-{hand_no}"),
        "dealerSeat": 1,
        "deckOrder": ["JH", "9S", "AD"],
        "moves": [
            { "seat": 2, "move": { "t": "orderUp" } },
            { "seat": 1, "move": { "t": "discard", "card": "BURIED-CARD" } },
            { "seat": 0, "move": { "t": "discard", "card": "9S" } },
            { "seat": 0, "move": { "t": "play", "card": "JH" } }
        ],
        "trump": "H",
        "makerSeat": 2,
        "aloneSeat": null,
        "tricksWon": [3, 2],
        "result": "point",
        "delta": [1, 0],
        "endedAt": 1_761_955_300_000_i64 + hand_no
    })
}

fn episode(
    role: &str,
    match_id: &str,
    hand_no: i64,
    salience: f64,
    expires_at: Option<i64>,
) -> Value {
    let mut e = json!({
        "role": role, "matchId": match_id, "handNo": hand_no, "kind": "bid",
        "summary": format!("summary {hand_no}"), "salience": salience
    });
    if let (Some(at), Some(map)) = (expires_at, e.as_object_mut()) {
        map.insert("expiresAt".into(), json!(at));
    }
    e
}

// ------------------------------------------------------------------ helpers

fn expect_user_error<T: std::fmt::Debug>(
    r: Result<T>,
    code: &str,
    what: &str,
) -> Result<UserError> {
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

fn expect_error_message<T: std::fmt::Debug>(
    r: Result<T>,
    code: &str,
    message: &str,
    what: &str,
) -> Result<()> {
    let e = expect_user_error(r, code, what)?;
    ensure!(
        e.message == message,
        "{what}: message `{}` != `{message}`",
        e.message
    );
    Ok(())
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
    // One key set per process: every test installs the same keys, so this and
    // the `register_with` below are `Ok`; a conflict fails the test here.
    configure(auth())?;
    TestEngine::start(TestEngineOptions::from_env(|registry| {
        let registered = register_with(
            registry,
            auth(),
            ActorConfig {
                sleep_timeout: Duration::from_secs(1),
                ..ActorConfig::default()
            },
        );
        if let Err(e) = registered {
            panic!("register playerProfile: {e}");
        }
    })?)
    .await
}

/// G36: wait until the envoy serves this profile, through `h`.
async fn ready(engine: &TestEngine, h: &TypedActorHandle<PlayerProfile>) -> Result<()> {
    engine
        .ready(|| async move { h.call(GetSettings::new()).await })
        .await
        .map(|_| ())
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

// ------------------------------------------------------------------ tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn profile_auth_end_to_end() -> Result<()> {
    run("profile_auth_end_to_end", |engine| async move {
        let r = auth_checks(&engine).await;
        (engine, r)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn profile_actions_end_to_end() -> Result<()> {
    run("profile_actions_end_to_end", |engine| async move {
        let r = action_checks(&engine).await;
        (engine, r)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn profile_durability_end_to_end() -> Result<()> {
    run("profile_durability_end_to_end", |engine| async move {
        let r = durability_checks(&engine).await;
        (engine, r)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn profile_token_overflow_end_to_end() -> Result<()> {
    run("profile_token_overflow_end_to_end", |engine| async move {
        let r = token_overflow_checks(&engine).await;
        (engine, r)
    })
    .await
}

async fn auth_checks(engine: &TestEngine) -> Result<()> {
    let alice = user("alice");
    let bob = user("bob");
    let owner = handle(engine, &alice, owner_params(&alice)?)?;
    ready(engine, &owner).await?;

    // Owner token: HTTP and WebSocket.
    let settings = stateless_call_with_timeout(&owner, GetSettings::new(), CALL).await?;
    ensure!(
        settings.personas.len() == 3,
        "default personas: {settings:?}"
    );
    let conn = connect_bounded(&owner, CALL).await?;
    let ws_settings = call_with_timeout(&conn, GetSettings::new(), CALL).await?;
    ensure!(ws_settings == settings, "ws settings differ");
    conn.disconnect().await;

    // A token with no protocolVersion is fine; protocolVersion 1.0 too.
    let token = auth().mint_profile_token(&alice)?;
    let bare = handle(engine, &alice, json!({ "token": token }))?;
    stateless_call_with_timeout(&bare, GetActiveGame::new(), CALL).await?;

    // A bad token.
    let bad = handle(
        engine,
        &alice,
        json!({ "token": "a.b.c", "protocolVersion": 1 }),
    )?;
    expect_user_error(
        stateless_call_with_timeout(&bad, GetSettings::new(), CALL).await,
        "invalid_token",
        "bad token",
    )?;
    let started = std::time::Instant::now();
    match connect_bounded(&bad, Duration::from_secs(20)).await {
        Ok(c) => {
            c.disconnect().await;
            bail!("a bad token connected over WebSocket");
        }
        Err(e) => ensure!(
            matches!(
                e.downcast_ref::<CallError>(),
                Some(CallError::Rejected { .. })
            ),
            "bad-token ws connect: `{e:#}`"
        ),
    }
    ensure!(
        started.elapsed() < Duration::from_secs(10),
        "rejection was slow"
    );

    // Another user's (valid) token.
    let foreign = handle(engine, &alice, owner_params(&bob)?)?;
    expect_error_message(
        stateless_call_with_timeout(&foreign, GetSettings::new(), CALL).await,
        "forbidden",
        "token does not belong to this profile",
        "wrong owner",
    )?;

    // A bad protocol version, checked before the token.
    for version in [json!(2), json!("1"), json!(null)] {
        let token = auth().mint_profile_token(&alice)?;
        let h = handle(
            engine,
            &alice,
            json!({ "token": token, "protocolVersion": version }),
        )?;
        expect_error_message(
            stateless_call_with_timeout(&h, GetSettings::new(), CALL).await,
            "protocol_version_mismatch",
            "unsupported protocol version",
            "protocol version",
        )?;
    }

    // No token: connects (internal shape) but no browser action runs.
    let anon = handle(engine, &alice, json!({}))?;
    let forbidden = "this connection is not authenticated";
    expect_error_message(
        stateless_call_with_timeout(&anon, GetSettings::new(), CALL).await,
        "forbidden",
        forbidden,
        "anon getSettings",
    )?;
    expect_error_message(
        stateless_call_with_timeout(&anon, UpdateSettings::new(json!({})), CALL).await,
        "forbidden",
        forbidden,
        "anon updateSettings",
    )?;
    expect_error_message(
        stateless_call_with_timeout(&anon, ListGames::new(None), CALL).await,
        "forbidden",
        forbidden,
        "anon listGames",
    )?;
    expect_error_message(
        stateless_call_with_timeout(&anon, ForgetEpisode::new("e1"), CALL).await,
        "forbidden",
        forbidden,
        "anon forgetEpisode",
    )?;
    let ws_anon = connect_bounded(&anon, CALL).await?;
    expect_error_message(
        call_with_timeout(&ws_anon, GetStats::new(), CALL).await,
        "forbidden",
        forbidden,
        "anon ws getStats",
    )?;
    ws_anon.disconnect().await;

    // Internal actions need the internal token, from any connection.
    expect_error_message(
        stateless_call_with_timeout(&owner, GetServerReplay::new(Value::Null, "m1"), CALL).await,
        "internal_token_mismatch",
        "internal token missing",
        "owner getServerReplay without token",
    )?;
    expect_error_message(
        stateless_call_with_timeout(&anon, PruneEpisodes::new("wrong-token"), CALL).await,
        "internal_token_mismatch",
        "internal token rejected",
        "pruneEpisodes wrong token",
    )?;
    let r = stateless_call_with_timeout(&anon, GetServerReplay::new(INTERNAL, "m1"), CALL).await?;
    ensure!(r.is_none(), "unknown match replay: {r:?}");
    eprintln!("auth ok: {alice}");
    Ok(())
}

async fn action_checks(engine: &TestEngine) -> Result<()> {
    let me = user("carol");
    let owner = handle(engine, &me, owner_params(&me)?)?;
    let anon = handle(engine, &me, json!({}))?;
    ready(engine, &owner).await?;
    let call = |what: &'static str| move |e: anyhow::Error| e.context(what);

    // ---- settings
    let before = stateless_call_with_timeout(&owner, GetSettings::new(), CALL).await?;
    let saved = stateless_call_with_timeout(
        &owner,
        UpdateSettings::new(json!({ "table": { "pace": "brisk" }, "personas": [{ "seat": 2, "patch": { "name": "Dutchie" } }] })),
        CALL,
    )
    .await?;
    let saved_json = serde_json::to_value(&saved)?;
    ensure!(saved_json["table"]["pace"] == "brisk", "pace: {saved_json}");
    ensure!(
        saved_json["personas"][1]["name"] == "Dutchie",
        "name: {saved_json}"
    );
    ensure!(
        saved.version > before.version,
        "version bump {} -> {}",
        before.version,
        saved.version
    );
    let read = stateless_call_with_timeout(&owner, GetSettings::new(), CALL).await?;
    ensure!(read == saved, "getSettings after save");
    let personas = stateless_call_with_timeout(&owner, GetPersonas::new(), CALL).await?;
    let personas_json = serde_json::to_value(&personas)?;
    ensure!(personas.len() == 3, "personas: {personas_json}");
    ensure!(
        personas_json[1]["name"] == "Dutchie",
        "persona view: {personas_json}"
    );
    for (patch, message) in [
        (json!(null), "settings patch must be an object"),
        (
            json!({ "table": { "pace": "fast" } }),
            "table.pace must be one of: brisk, normal, relaxed",
        ),
        (
            json!({ "personas": [{ "seat": 0, "patch": {} }] }),
            "seat must be one of: 1, 2, 3",
        ),
        (json!({ "housePrompt": 7 }), "housePrompt must be a string"),
    ] {
        expect_error_message(
            stateless_call_with_timeout(&owner, UpdateSettings::new(patch.clone()), CALL).await,
            "invalid_persona",
            message,
            "settings patch",
        )?;
    }
    expect_error_message(
        stateless_call_with_timeout(&owner, UpdateSettings(Vec::new()), CALL).await,
        "invalid_persona",
        "settings patch must be an object",
        "missing patch",
    )?;
    ensure!(
        stateless_call_with_timeout(&owner, GetSettings::new(), CALL).await? == saved,
        "a refused patch changed settings"
    );

    // ---- active game
    ensure!(
        stateless_call_with_timeout(&owner, GetActiveGame::new(), CALL)
            .await?
            .is_none(),
        "fresh active game"
    );
    stateless_call_with_timeout(
        &anon,
        SetActiveGame::new(json!({ "internalToken": INTERNAL, "gameId": "g_live" })),
        CALL,
    )
    .await
    .map_err(call("setActiveGame"))?;
    ensure!(
        stateless_call_with_timeout(&owner, GetActiveGame::new(), CALL)
            .await?
            .as_deref()
            == Some("g_live"),
        "active game set"
    );
    expect_error_message(
        stateless_call_with_timeout(&anon, SetActiveGame::new(json!({ "gameId": "g_x" })), CALL)
            .await,
        "internal_token_mismatch",
        "internal token missing",
        "forged setActiveGame",
    )?;

    // ---- recordGame + recordHands → stats, lifetime
    let m1 = "m_first";
    stateless_call_with_timeout(
        &anon,
        RecordGame::new(json!({
            "internalToken": INTERNAL,
            "record": record(&me, m1, None, "abandoned"),
            "hands": [hand(m1, 0)]
        })),
        CALL,
    )
    .await
    .map_err(call("recordGame live"))?;
    stateless_call_with_timeout(
        &anon,
        RecordHands::new(json!({ "internalToken": INTERNAL, "matchId": m1, "hands": [hand(m1, 1), hand(m1, 0)] })),
        CALL,
    )
    .await
    .map_err(call("recordHands"))?;
    // The live row advances to its final state.
    stateless_call_with_timeout(
        &anon,
        RecordGame::new(json!({ "internalToken": INTERNAL, "record": record(&me, m1, Some(1_761_957_000_000), "won") })),
        CALL,
    )
    .await
    .map_err(call("recordGame final"))?;
    // A finished row is immutable history.
    stateless_call_with_timeout(
        &anon,
        RecordGame::new(json!({ "internalToken": INTERNAL, "record": record(&me, m1, Some(1_761_957_999_000), "lost") })),
        CALL,
    )
    .await
    .map_err(call("recordGame after end"))?;
    let stats = stateless_call_with_timeout(&owner, GetStats::new(), CALL).await?;
    ensure!(
        stats.matches == 1 && stats.won == 1 && stats.lost == 0 && stats.euchres_for == 2,
        "stats after one match: {stats:?}"
    );
    let state = test_support::last_state(&me).context("no persisted state")?;
    ensure!(
        state["lifetime"]["matches"] == 1
            && state["lifetime"]["won"] == 1
            && state["lifetime"]["handsPlayed"] == 2,
        "lifetime cache: {}",
        state["lifetime"]
    );

    // Forged and invalid queue bodies.
    expect_error_message(
        stateless_call_with_timeout(
            &anon,
            RecordGame::new(json!({ "internalToken": "forged", "record": record(&me, "m_forged", None, "won") })),
            CALL,
        )
        .await,
        "internal_token_mismatch",
        "internal token rejected",
        "forged recordGame",
    )?;
    // Valid token, invalid body (another user's record): logged and dropped, as TS.
    stateless_call_with_timeout(
        &anon,
        RecordGame::new(json!({ "internalToken": INTERNAL, "record": record("someone_else", "m_other", None, "won") })),
        CALL,
    )
    .await
    .map_err(call("invalid recordGame"))?;
    stateless_call_with_timeout(
        &anon,
        RecordHands::new(json!({ "internalToken": INTERNAL, "matchId": "bad id!", "hands": [] })),
        CALL,
    )
    .await
    .map_err(call("invalid recordHands"))?;

    // ---- listGames pagination (m1 plus five more, newest first)
    for i in 0..5_i64 {
        let id = format!("m_page_{i}");
        stateless_call_with_timeout(
            &anon,
            RecordGame::new(json!({ "internalToken": INTERNAL, "record": record(&me, &id, Some(1_762_000_000_000 + i * 1000), "lost") })),
            CALL,
        )
        .await
        .map_err(call("recordGame page"))?;
    }
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let req = match &cursor {
            Some(c) => json!({ "limit": 2, "cursor": c }),
            None => json!({ "limit": 2 }),
        };
        let page = stateless_call_with_timeout(&owner, ListGames::new(Some(req)), CALL).await?;
        pages += 1;
        ensure!(page.rows.len() <= 2, "page size {}", page.rows.len());
        seen.extend(page.rows.iter().map(|r| r.match_id.clone()));
        match page.cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
        ensure!(pages < 10, "pagination did not end");
    }
    let expected = [
        "m_page_4", "m_page_3", "m_page_2", "m_page_1", "m_page_0", m1,
    ];
    ensure!(seen == expected, "history order {seen:?}");
    ensure!(pages == 3, "pages {pages}");
    let all = stateless_call_with_timeout(&owner, ListGames::new(None), CALL).await?;
    ensure!(
        all.rows.len() == 6 && all.cursor.is_none(),
        "default page: {all:?}"
    );
    let undefined = stateless_call_with_timeout(&owner, ListGames(vec![Value::Null]), CALL).await?;
    ensure!(undefined == all, "listGames(undefined)");
    expect_error_message(
        stateless_call_with_timeout(
            &owner,
            ListGames::new(Some(json!({ "cursor": "nope" }))),
            CALL,
        )
        .await,
        "internal_error",
        "cursor is malformed",
        "bad cursor",
    )?;
    ensure!(
        !seen.iter().any(|m| m == "m_other" || m == "m_forged"),
        "a refused record was stored"
    );

    // ---- replays: the client view hides seeds, deck order and buried cards
    let client = stateless_call_with_timeout(&owner, GetReplay::new(m1), CALL)
        .await?
        .context("client replay missing")?;
    let client_json = serde_json::to_string(&client)?;
    ensure!(
        client.hands.len() == 2,
        "client hands {}",
        client.hands.len()
    );
    for secret in [
        "seed-secret",
        "hand-seed-secret",
        "BURIED-CARD",
        "deckOrder",
        "JH\",\"9S\",\"AD",
    ] {
        ensure!(
            !client_json.contains(secret),
            "client replay leaks `{secret}`: {client_json}"
        );
    }
    ensure!(
        client_json.contains("\"9S\""),
        "seat 0 discard must stay visible"
    );
    ensure!(
        client_json.contains("\"tricks\":[7,3]"),
        "match tricks: {client_json}"
    );
    let server = stateless_call_with_timeout(&anon, GetServerReplay::new(INTERNAL, m1), CALL)
        .await?
        .context("server replay missing")?;
    let server_json = serde_json::to_string(&server)?;
    for secret in [
        "seed-secret-m_first",
        "hand-seed-secret-0",
        "BURIED-CARD",
        "deckOrder",
    ] {
        ensure!(
            server_json.contains(secret),
            "server replay lacks `{secret}`"
        );
    }
    ensure!(
        server_json.starts_with("{\"match\":"),
        "server replay shape: {}",
        &server_json[..40.min(server_json.len())]
    );
    ensure!(
        stateless_call_with_timeout(&owner, GetReplay::new("m_unknown"), CALL)
            .await?
            .is_none(),
        "unknown replay"
    );
    expect_error_message(
        stateless_call_with_timeout(&owner, GetReplay::new("bad id!"), CALL).await,
        "internal_error",
        "matchId must match /^[A-Za-z0-9_-]{1,128}$/",
        "bad replay id",
    )?;
    expect_error_message(
        stateless_call_with_timeout(&owner, GetReplay(Vec::new()), CALL).await,
        "internal_error",
        "matchId must match /^[A-Za-z0-9_-]{1,128}$/",
        "missing replay id",
    )?;

    // ---- episodes
    expect_error_message(
        stateless_call_with_timeout(
            &anon,
            RecordEpisodes::new(json!({ "episodes": [episode("marge", m1, 0, 0.5, None)] })),
            CALL,
        )
        .await,
        "internal_token_mismatch",
        "internal token missing",
        "recordEpisodes without token",
    )?;
    stateless_call_with_timeout(
        &anon,
        RecordEpisodes::new(json!({
            "internalToken": INTERNAL,
            "episodes": [
                episode("marge", m1, 0, 0.2, None),
                episode("marge", m1, 1, 0.9, None),
                episode("verna", m1, 0, 0.5, None),
                episode("marge", m1, 2, 0.7, Some(1)),
                episode("marge", m1, 3, 0.7, Some(2))
            ]
        })),
        CALL,
    )
    .await
    .map_err(call("recordEpisodes"))?;
    let listed = stateless_call_with_timeout(&owner, ListEpisodes::new(None), CALL).await?;
    ensure!(listed.len() == 3, "live episodes: {listed:?}");
    let one = stateless_call_with_timeout(&owner, ListEpisodes::new(Some(json!(1))), CALL).await?;
    ensure!(one.len() == 1, "listEpisodes(1): {one:?}");
    let ranked = stateless_call_with_timeout(
        &anon,
        GetEpisodesForRole::new(INTERNAL, "marge", None),
        CALL,
    )
    .await?;
    let ids: Vec<&str> = ranked.iter().map(|e| e.episode_id.as_str()).collect();
    ensure!(
        ids == [format!("marge.{m1}.1.bid"), format!("marge.{m1}.0.bid")],
        "ranked episodes {ids:?}"
    );
    expect_error_message(
        stateless_call_with_timeout(&anon, GetEpisodesForRole::new("nope", "marge", None), CALL)
            .await,
        "internal_token_mismatch",
        "internal token rejected",
        "getEpisodesForRole wrong token",
    )?;
    // forgetEpisode (D30): forgets by the stored `role.matchId.handNo.kind` id,
    // which TS refused. Malformed ids are still refused.
    let forget = format!("verna.{m1}.0.bid");
    ensure!(
        stateless_call_with_timeout(&owner, ForgetEpisode::new(forget.clone()), CALL).await?,
        "forgetEpisode stored id"
    );
    let listed = stateless_call_with_timeout(&owner, ListEpisodes::new(None), CALL).await?;
    ensure!(
        listed.len() == 2 && listed.iter().all(|e| e.episode_id != forget),
        "forgotten episode still listed: {listed:?}"
    );
    ensure!(
        !stateless_call_with_timeout(&owner, ForgetEpisode::new(forget), CALL).await?,
        "forgetEpisode twice"
    );
    ensure!(
        !stateless_call_with_timeout(
            &owner,
            ForgetEpisode::new(format!("marge.{m1}.9.bid")),
            CALL
        )
        .await?,
        "forgetEpisode unknown well-formed id"
    );
    for bad in [
        "no_such_episode",
        "verna.m.03.bid",
        "../m.0.bid",
        "verna.m.0.bid.x",
    ] {
        expect_error_message(
            stateless_call_with_timeout(&owner, ForgetEpisode::new(bad), CALL).await,
            "internal_error",
            "episodeId must match <role>.<matchId>.<handNo>.<kind>",
            "forgetEpisode malformed id",
        )?;
    }
    // Put it back so the prune checks below see the same three live rows.
    stateless_call_with_timeout(
        &anon,
        RecordEpisodes::new(json!({
            "internalToken": INTERNAL,
            "episodes": [episode("verna", m1, 0, 0.5, None)]
        })),
        CALL,
    )
    .await
    .map_err(call("re-record forgotten episode"))?;
    // pruneEpisodes. Expired rows are hidden but still occupy their id, so a
    // re-record of an expired id is ignored (ON CONFLICT DO NOTHING) until the
    // prune deletes the row; after it the same re-record lands.
    let rerecord = || {
        stateless_call_with_timeout(
            &anon,
            RecordEpisodes::new(json!({
                "internalToken": INTERNAL,
                "episodes": [episode("marge", m1, 2, 0.7, None), episode("marge", m1, 3, 0.7, None)]
            })),
            CALL,
        )
    };
    rerecord().await.map_err(call("re-record before prune"))?;
    ensure!(
        stateless_call_with_timeout(&owner, ListEpisodes::new(None), CALL)
            .await?
            .len()
            == 3,
        "an expired row was overwritten before prune"
    );
    stateless_call_with_timeout(&anon, PruneEpisodes::new(INTERNAL), CALL).await?;
    ensure!(
        stateless_call_with_timeout(&owner, ListEpisodes::new(None), CALL)
            .await?
            .len()
            == 3,
        "prune removed a live episode"
    );
    rerecord().await.map_err(call("re-record after prune"))?;
    ensure!(
        stateless_call_with_timeout(&owner, ListEpisodes::new(None), CALL)
            .await?
            .len()
            == 5,
        "pruneEpisodes left the expired rows"
    );

    // ---- recordTokens
    stateless_call_with_timeout(
        &anon,
        RecordTokens::new(json!({ "internalToken": INTERNAL, "tokensIn": 10, "tokensOut": 4.9, "cacheReadTokens": -3, "calls": 1 })),
        CALL,
    )
    .await?;
    let state = test_support::last_state(&me).context("no persisted state")?;
    let daily = &state["dailyTokens"];
    ensure!(
        daily["tokensIn"] == 10
            && daily["tokensOut"] == 4
            && daily["cacheReadTokens"] == 0
            && daily["calls"] == 1,
        "daily tokens: {daily}"
    );
    expect_error_message(
        stateless_call_with_timeout(
            &anon,
            RecordTokens::new(json!({ "internalToken": "x", "calls": 5 })),
            CALL,
        )
        .await,
        "internal_token_mismatch",
        "internal token rejected",
        "forged recordTokens",
    )?;
    eprintln!("actions ok: {me}");
    Ok(())
}

async fn durability_checks(engine: &TestEngine) -> Result<()> {
    let me = user("dave");
    let owner = handle(engine, &me, owner_params(&me)?)?;
    let anon = handle(engine, &me, json!({}))?;
    ready(engine, &owner).await?;

    // Concurrent writes: 24 token increments and 6 racing first inserts.
    let tokens = (0..24).map(|_| {
        stateless_call_with_timeout(
            &anon,
            RecordTokens::new(json!({ "internalToken": INTERNAL, "tokensIn": 5, "calls": 1 })),
            CALL,
        )
    });
    let games = (0..6).map(|_| {
        stateless_call_with_timeout(
            &anon,
            RecordGame::new(json!({ "internalToken": INTERNAL, "record": record(&me, "m_race", Some(1_762_100_000_000), "won"), "hands": [hand("m_race", 0)] })),
            CALL,
        )
    });
    let settings = [
        json!({ "table": { "pace": "relaxed" } }),
        json!({ "table": { "sound": false } }),
        json!({ "housePrompt": "Quiet table." }),
    ]
    .map(|p| stateless_call_with_timeout(&owner, UpdateSettings::new(p), CALL));
    let (t, g, s) = tokio::join!(
        futures_all(tokens),
        futures_all(games),
        futures_all(settings)
    );
    t?;
    g?;
    s?;
    let state = test_support::last_state(&me).context("no persisted state")?;
    ensure!(
        state["dailyTokens"]["calls"] == 24 && state["dailyTokens"]["tokensIn"] == 120,
        "serialized counters: {}",
        state["dailyTokens"]
    );
    ensure!(
        state["lifetime"]["matches"] == 1,
        "racing inserts: {}",
        state["lifetime"]
    );
    let settings = stateless_call_with_timeout(&owner, GetSettings::new(), CALL).await?;
    let sj = serde_json::to_value(&settings)?;
    ensure!(
        sj["table"]["pace"] == "relaxed"
            && sj["table"]["sound"] == false
            && sj["housePrompt"] == "Quiet table.",
        "all three concurrent patches must land: {sj}"
    );
    stateless_call_with_timeout(
        &anon,
        SetActiveGame::new(json!({ "internalToken": INTERNAL, "gameId": "g_resume" })),
        CALL,
    )
    .await?;
    stateless_call_with_timeout(
        &anon,
        RecordEpisodes::new(json!({ "internalToken": INTERNAL, "episodes": [episode("marge", "m_race", 0, 0.4, None)] })),
        CALL,
    )
    .await?;
    let stats_before = stateless_call_with_timeout(&owner, GetStats::new(), CALL).await?;
    let page_before = stateless_call_with_timeout(&owner, ListGames::new(None), CALL).await?;
    let replay_before = stateless_call_with_timeout(&owner, GetReplay::new("m_race"), CALL).await?;
    let state_before = test_support::last_state(&me).context("no persisted state")?;
    ensure!(
        page_before.rows.len() == 1,
        "racing inserts made {} rows",
        page_before.rows.len()
    );

    // Idle sleep (sleep_timeout 1 s), then wake.
    let starts = test_support::starts(&me);
    let sleeps = test_support::sleeps(&me);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while test_support::sleeps(&me) == sleeps {
        ensure!(
            std::time::Instant::now() < deadline,
            "the actor never slept"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    ready(engine, &owner).await?;
    ensure!(
        test_support::starts(&me) == starts + 1,
        "wake count {} -> {}",
        starts,
        test_support::starts(&me)
    );
    ensure!(
        stateless_call_with_timeout(&owner, GetSettings::new(), CALL).await? == settings,
        "settings after wake"
    );
    ensure!(
        stateless_call_with_timeout(&owner, GetActiveGame::new(), CALL)
            .await?
            .as_deref()
            == Some("g_resume"),
        "active game after wake"
    );
    let stats_after = stateless_call_with_timeout(&owner, GetStats::new(), CALL).await?;
    ensure!(
        stats_after.matches == stats_before.matches && stats_after.won == stats_before.won,
        "stats after wake {stats_after:?}"
    );
    ensure!(
        stateless_call_with_timeout(&owner, ListGames::new(None), CALL).await? == page_before,
        "history after wake"
    );
    ensure!(
        stateless_call_with_timeout(&owner, GetReplay::new("m_race"), CALL).await? == replay_before,
        "replay after wake"
    );
    ensure!(
        stateless_call_with_timeout(&owner, ListEpisodes::new(None), CALL)
            .await?
            .len()
            == 1,
        "episodes after wake"
    );
    let state_after = test_support::last_state(&me).context("no state after wake")?;
    let strip = |mut v: Value| {
        if let Some(m) = v.as_object_mut() {
            m.remove("settings");
        }
        v
    };
    ensure!(
        strip(state_after.clone()) == strip(state_before),
        "state after wake {state_after}"
    );
    eprintln!(
        "durability ok: {me} starts {starts} -> {} sleeps {sleeps} -> {}",
        test_support::starts(&me),
        test_support::sleeps(&me)
    );
    Ok(())
}

/// Run every future concurrently; the first error wins.
/// W3 follow-up: near-maximum token counters on two matches used to overflow
/// `STATS_SQL`'s integer `SUM(tokens_in)`, so every later `recordGame` for the
/// profile failed (`compute_stats` runs on each) and its table retried forever.
async fn token_overflow_checks(engine: &TestEngine) -> Result<()> {
    const MAX_SAFE: u64 = 9_007_199_254_740_991;
    let me = user("erin");
    let owner = handle(engine, &me, owner_params(&me)?)?;
    let anon = handle(engine, &me, json!({}))?;
    ready(engine, &owner).await?;

    let huge = json!({
        "tokensIn": u64::MAX,
        "tokensOut": MAX_SAFE + 1,
        "cacheReadTokens": 1e300,
        "calls": i64::MAX,
    });
    for (i, match_id) in ["m_huge_1", "m_huge_2", "m_normal"].into_iter().enumerate() {
        let mut body = record(&me, match_id, Some(1_762_200_000_000 + i as i64), "won");
        if match_id != "m_normal" {
            body["tokens"] = huge.clone();
        }
        stateless_call_with_timeout(
            &anon,
            RecordGame::new(json!({ "internalToken": INTERNAL, "record": body })),
            CALL,
        )
        .await
        .with_context(|| format!("recordGame {match_id}"))?;
    }

    let stats = stateless_call_with_timeout(&owner, GetStats::new(), CALL).await?;
    ensure!(
        stats.matches == 3 && stats.won == 3 && stats.euchres_for == 6 && stats.hands_played == 6,
        "every match counted, exact counters: {stats:?}"
    );
    let t = &stats.tokens;
    ensure!(
        [t.tokens_in, t.tokens_out, t.cache_read_tokens, t.calls] == [MAX_SAFE; 4],
        "token totals saturate at MAX_SAFE_INTEGER: {t:?}"
    );
    let state = test_support::last_state(&me).context("no persisted state")?;
    ensure!(
        state["lifetime"]["matches"] == 3 && state["lifetime"]["handsPlayed"] == 6,
        "lifetime cache: {}",
        state["lifetime"]
    );

    let replay =
        stateless_call_with_timeout(&anon, GetServerReplay::new(INTERNAL, "m_huge_2"), CALL)
            .await?
            .context("server replay missing")?;
    let stored = serde_json::to_value(&replay)?;
    ensure!(
        stored["match"]["tokens"]
            == json!({ "tokensIn": MAX_SAFE, "tokensOut": MAX_SAFE, "cacheReadTokens": MAX_SAFE, "calls": MAX_SAFE }),
        "stored counters clamped: {}",
        stored["match"]["tokens"]
    );
    let normal =
        stateless_call_with_timeout(&anon, GetServerReplay::new(INTERNAL, "m_normal"), CALL)
            .await?
            .context("server replay missing")?;
    ensure!(
        serde_json::to_value(&normal)?["match"]["tokens"]
            == json!({ "tokensIn": 100, "tokensOut": 20, "cacheReadTokens": 5, "calls": 3 }),
        "a normal match keeps its counters"
    );

    // A fourth record still lands on the saturated profile.
    stateless_call_with_timeout(
        &anon,
        RecordGame::new(json!({ "internalToken": INTERNAL, "record": record(&me, "m_after", Some(1_762_300_000_000), "lost") })),
        CALL,
    )
    .await
    .context("recordGame after saturation")?;
    let stats = stateless_call_with_timeout(&owner, GetStats::new(), CALL).await?;
    ensure!(
        stats.matches == 4 && stats.lost == 1 && stats.tokens.tokens_in == MAX_SAFE,
        "stats after a fourth match: {stats:?}"
    );
    eprintln!("token overflow ok: {me}");
    Ok(())
}

async fn futures_all<T, F>(futures: impl IntoIterator<Item = F>) -> Result<Vec<T>>
where
    F: std::future::Future<Output = Result<T>>,
{
    let handles: Vec<F> = futures.into_iter().collect();
    let mut out = Vec::with_capacity(handles.len());
    let results = join_all(handles).await;
    for r in results {
        out.push(r?);
    }
    Ok(out)
}

/// A tiny `join_all` so the test needs no `futures` dependency.
async fn join_all<F: std::future::Future>(futures: Vec<F>) -> Vec<F::Output> {
    let mut set = Vec::with_capacity(futures.len());
    let mut pinned: Vec<std::pin::Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    let mut done: Vec<Option<F::Output>> = (0..pinned.len()).map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (i, f) in pinned.iter_mut().enumerate() {
            if done[i].is_none() {
                match f.as_mut().poll(cx) {
                    std::task::Poll::Ready(v) => done[i] = Some(v),
                    std::task::Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    set.extend(done.into_iter().flatten());
    set
}
