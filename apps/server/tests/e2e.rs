//! The whole server, end to end, driven only the way a real client drives it:
//! HTTP for the guest session, the lobby and the profile; rivetkit-client 3.1.0
//! (JSON) through the server's own `/rivet` gateway for the table.
//!
//! Engine tests: they skip unless `EUCHRE_E2E=1`. The engines run one at a
//! time on the port block `P` = `EUCHRE_TEST_ENGINE_PORT` (default 6860): the
//! first binds P, P+1 and P+5, the second P+2, P+3 and P+7. The tests share
//! one runtime and run one after the other, since `euchreTable` and
//! `playerProfile` keep their config process-wide.
//!
//! ```text
//! EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6860 cargo test -p euchre-server --test e2e -- --nocapture
//! ```

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use euchre_server::{BootError, DRAINING_NEW_TABLE, RunningServer, ServerConfig, ServerFailure};
use euchre_server_core::config::{
    AiConfig, LOCAL_SECRETS_FILE, LocalSecrets, write_new_private_file,
};
use euchre_server_core::rivet::parse_rivet_error;
use euchre_server_core::testing::{e2e_enabled, e2e_skip};
use rivetkit_client::connection::ActorConnection;
use rivetkit_client::{
    Client, ClientConfig, ConnectionStatus, EncodingKind, GetOptions, SubscriptionHandle,
};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

// ------------------------------------------------------------------ fixtures

/// One action or HTTP call.
const CALL: Duration = Duration::from_secs(15);
/// The next `sync` event.
const EVENT: Duration = Duration::from_secs(30);
/// One wait for a view that satisfies a condition.
const VIEW_DEADLINE: Duration = Duration::from_secs(90);
/// The profile records the match asynchronously.
const RECORD_DEADLINE: Duration = Duration::from_secs(30);
/// A whole test.
const TEST_LIMIT: Duration = Duration::from_secs(480);
/// A failing boot must fail within this.
const BOOT_FAILURE_LIMIT: Duration = Duration::from_secs(90);
/// A busy engine port is found before the engine starts.
const BUSY_PORT_LIMIT: Duration = Duration::from_secs(5);
/// A dead engine is noticed (polled every second) within this.
const ENGINE_DEATH_LIMIT: Duration = Duration::from_secs(10);

const DEFAULT_ENGINE_PORT: u16 = 6860;
const TABLE: &str = "euchreTable";

/// The first engine's guard port: `EUCHRE_TEST_ENGINE_PORT`, else 6860.
static ENGINE_PORT: LazyLock<u16> =
    LazyLock::new(|| match std::env::var("EUCHRE_TEST_ENGINE_PORT") {
        Ok(v) => v
            .parse()
            .unwrap_or_else(|_| panic!("EUCHRE_TEST_ENGINE_PORT `{v}` is not a port")),
        Err(_) => DEFAULT_ENGINE_PORT,
    });

/// The second engine's guard port (P+2: it binds P+2, P+3 and P+7).
fn second_engine_port() -> u16 {
    *ENGINE_PORT + 2
}

/// One runtime for every test, so clients and pools made by one server never
/// outlive the runtime that made them.
static RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("build the test runtime")
});

/// One server per process at a time.
static SERIAL: Mutex<()> = Mutex::new(());

/// The one secrets set every server in this process shares.
static SECRETS: LazyLock<LocalSecrets> = LazyLock::new(LocalSecrets::generate);

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_test_writer()
        .try_init();
}

/// Skip without `EUCHRE_E2E=1`; else run `body` alone on the shared runtime,
/// bounded by [`TEST_LIMIT`].
fn run<F, Fut>(name: &str, body: F) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    if !e2e_enabled() {
        e2e_skip(name);
        return Ok(());
    }
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    init_tracing();
    RT.block_on(async {
        match tokio::time::timeout(TEST_LIMIT, body()).await {
            Ok(result) => result,
            Err(_) => bail!("{name}: did not finish within {TEST_LIMIT:?}"),
        }
    })
}

/// A fresh data dir that holds `secrets` as its `secrets.json`.
fn data_dir_with(secrets: &LocalSecrets) -> Result<tempfile::TempDir> {
    let dir = tempfile::Builder::new()
        .prefix("euchre-server-e2e-")
        .tempdir()?;
    let body = serde_json::to_vec_pretty(secrets)?;
    write_new_private_file(&dir.path().join(LOCAL_SECRETS_FILE), &body)?;
    Ok(dir)
}

/// The embedded config the desktop uses, on this lane's engine port, offline
/// AI and the fast tempo (the same in every test: the table config is
/// process-wide).
fn local_config(dir: &tempfile::TempDir, engine_port: u16) -> Result<ServerConfig> {
    Ok(desktop_config(dir)?.with_engine_port(engine_port))
}

/// `ServerConfig::try_local(dir)` as the desktop calls it, with no engine port
/// chosen (a used data dir keeps its engine's port), offline AI and the fast
/// tempo.
fn desktop_config(dir: &tempfile::TempDir) -> Result<ServerConfig> {
    let mut cfg = ServerConfig::try_local(dir.path().to_path_buf())?;
    cfg.ai = AiConfig::default();
    cfg.fast_tempo = true;
    Ok(cfg)
}

/// Something accepts connections on `127.0.0.1:port`.
fn listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_millis(200),
    )
    .is_ok()
}

/// The engine ports of `engine_port` and the HTTP port close within 5 s.
async fn ports_closed(engine_port: u16, http_port: u16) -> Result<()> {
    let ports = [engine_port, engine_port + 1, engine_port + 5, http_port];
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let open: Vec<u16> = ports.iter().copied().filter(|p| listening(*p)).collect();
        if open.is_empty() {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "still listening after shutdown: {open:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn boot_error(err: &anyhow::Error) -> Option<&BootError> {
    err.downcast_ref::<BootError>()
}

// ------------------------------------------------------------------ HTTP

/// The HTTP API as a native client sees it (no `Origin`, bearer session).
struct Api {
    base: String,
    http: reqwest::Client,
}

impl Api {
    fn new(server: &RunningServer) -> Result<Self> {
        Ok(Self {
            base: server.url(),
            // No pooled connections, so a restart never hands us a dead one.
            http: reqwest::Client::builder()
                .no_proxy()
                .pool_max_idle_per_host(0)
                .timeout(CALL)
                .build()?,
        })
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<(u16, Value)> {
        let res = req.send().await?;
        let status = res.status().as_u16();
        let text = res.text().await?;
        Ok((
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        ))
    }

    async fn get(&self, path: &str, bearer: Option<&str>) -> Result<(u16, Value)> {
        let mut req = self.http.get(format!("{}{path}", self.base));
        if let Some(bearer) = bearer {
            req = req.bearer_auth(bearer);
        }
        self.send(req).await
    }

    async fn post(&self, path: &str, bearer: Option<&str>) -> Result<(u16, Value)> {
        let mut req = self.http.post(format!("{}{path}", self.base));
        if let Some(bearer) = bearer {
            req = req.bearer_auth(bearer);
        }
        self.send(req).await
    }

    /// `POST /api/device/enroll`: a new guest (`userId`, `sessionToken`).
    async fn enroll(&self) -> Result<(String, String)> {
        let (status, body) = self.post("/api/device/enroll", None).await?;
        ensure!(status == 200, "enroll -> {status} {body}");
        Ok((
            body["userId"].as_str().context("userId")?.to_owned(),
            body["sessionToken"]
                .as_str()
                .context("sessionToken")?
                .to_owned(),
        ))
    }

    /// `POST /api/tables` against three AI seats; retried while the first
    /// requests after boot settle (G36), bounded.
    async fn create_table(&self, bearer: &str) -> Result<String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let (status, body) = self.post("/api/tables", Some(bearer)).await?;
            if status == 201 {
                return Ok(body["gameId"].as_str().context("gameId")?.to_owned());
            }
            ensure!(
                status == 503 && Instant::now() < deadline,
                "create table -> {status} {body}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// `POST /api/table-session/{gameId}`.
    async fn table_session(&self, game_id: &str, bearer: &str) -> Result<TableSession> {
        let (status, body) = self
            .post(&format!("/api/table-session/{game_id}"), Some(bearer))
            .await?;
        ensure!(status == 200, "table-session -> {status} {body}");
        Ok(TableSession {
            actor_id: body["actorId"].as_str().context("actorId")?.to_owned(),
            token: body["token"].as_str().context("token")?.to_owned(),
            gateway_token: body["gatewayToken"]
                .as_str()
                .context("gatewayToken (Option A issuer off?)")?
                .to_owned(),
        })
    }

    /// `GET /api/rivet/metadata`: where and in which namespace a client dials.
    async fn rivet_metadata(&self) -> Result<(String, String)> {
        let (status, body) = self.get("/api/rivet/metadata", None).await?;
        ensure!(status == 200, "metadata -> {status} {body}");
        Ok((
            body["clientEndpoint"]
                .as_str()
                .context("clientEndpoint")?
                .to_owned(),
            body["clientNamespace"]
                .as_str()
                .context("clientNamespace")?
                .to_owned(),
        ))
    }
}

struct TableSession {
    actor_id: String,
    token: String,
    gateway_token: String,
}

/// A rivetkit client on the server's `/rivet` gateway, as the desktop and the
/// browser build it from `/api/rivet/metadata` and the table session.
async fn gateway_client(api: &Api, session: &TableSession) -> Result<Client> {
    let (endpoint, namespace) = api.rivet_metadata().await?;
    ensure!(
        endpoint == format!("{}/rivet", api.base),
        "metadata endpoint {endpoint}"
    );
    Ok(Client::new(
        ClientConfig::new(endpoint)
            .namespace(namespace)
            .encoding(EncodingKind::Json)
            .disable_metadata_lookup(true)
            .token(session.gateway_token.clone()),
    ))
}

// ------------------------------------------------------------------ the table

fn v_of(view: &Value) -> u64 {
    view["v"].as_u64().unwrap_or(0)
}

fn is_player_turn(view: &Value) -> bool {
    view["status"] == "active"
        && view["turnSeat"] == view["you"]
        && view["awaitingTrickReview"] != true
        && view["legal"].as_array().is_some_and(|l| !l.is_empty())
}

fn first_legal(view: &Value) -> Result<String> {
    view["legal"][0]["id"]
        .as_str()
        .map(str::to_owned)
        .context("no legal move")
}

/// The player's WebSocket to its table, with its `sync` stream.
struct Player {
    conn: ActorConnection,
    syncs: UnboundedReceiver<Value>,
    _sub: SubscriptionHandle,
    last: Option<Value>,
    moves: u64,
    syncs_seen: u64,
}

impl Player {
    /// Connect through the gateway with the table session's actor token. The
    /// `sync` listener is registered before the socket opens, so the first
    /// `sync` (the resync) is seen.
    async fn connect(client: &Client, session: &TableSession) -> Result<Self> {
        let handle = client.get_for_id(
            TABLE,
            &session.actor_id,
            GetOptions {
                params: Some(json!({ "token": session.token, "protocolVersion": 1 })),
            },
        )?;
        let conn = handle.connect();
        let (tx, syncs) = unbounded_channel();
        let sub = conn
            .on_event("sync", move |args| {
                if let Some(body) = args.first() {
                    let _ = tx.send(body.clone());
                }
            })
            .await;
        let mut status = conn.status_receiver();
        let opened =
            tokio::time::timeout(CALL, status.wait_for(|s| *s == ConnectionStatus::Connected))
                .await;
        if !matches!(opened, Ok(Ok(_))) {
            conn.disconnect().await;
            bail!("the table WebSocket did not open within {CALL:?}");
        }
        Ok(Self {
            conn,
            syncs,
            _sub: sub,
            last: None,
            moves: 0,
            syncs_seen: 0,
        })
    }

    async fn disconnect(self) {
        self.conn.disconnect().await;
    }

    async fn action(&self, name: &str, args: Vec<Value>) -> Result<Value> {
        match tokio::time::timeout(CALL, self.conn.action(name, args)).await {
            Ok(result) => result,
            Err(_) => bail!("{name} timed out after {CALL:?}"),
        }
    }

    fn absorb(&mut self, body: Value) -> Result<()> {
        let v = body["v"].as_u64().context("sync without v")?;
        ensure!(
            body["view"]["v"].as_u64() == Some(v),
            "sync v and view.v differ: {body}"
        );
        if let Some(prev) = self.last.as_ref().map(|b| v_of(&b["view"])) {
            ensure!(v >= prev, "sync went backwards: {prev} -> {v}");
        }
        self.syncs_seen += 1;
        self.last = Some(body);
        Ok(())
    }

    /// The next `sync` body (`{v, steps, view}`), bounded.
    async fn next_sync(&mut self, what: &str) -> Result<Value> {
        let body = match tokio::time::timeout(EVENT, self.syncs.recv()).await {
            Ok(Some(body)) => body,
            Ok(None) => bail!("{what}: sync stream closed"),
            Err(_) => bail!("{what}: no sync within {EVENT:?}"),
        };
        self.absorb(body.clone())?;
        Ok(body)
    }

    fn drain(&mut self) -> Result<Option<Value>> {
        while let Ok(body) = self.syncs.try_recv() {
            self.absorb(body)?;
        }
        Ok(self.last.as_ref().map(|b| b["view"].clone()))
    }

    /// The newest view at `min_v` or later that satisfies `done`.
    async fn wait_view(
        &mut self,
        min_v: u64,
        what: &str,
        done: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        let deadline = Instant::now() + VIEW_DEADLINE;
        loop {
            if let Some(view) = self.drain()?
                && v_of(&view) >= min_v
                && done(&view)
            {
                return Ok(view);
            }
            ensure!(
                Instant::now() < deadline,
                "{what}: no such view within {VIEW_DEADLINE:?}"
            );
            self.next_sync(what).await?;
        }
    }

    /// `submitMove` with the view's turn nonce.
    async fn submit(&mut self, view: &Value, move_id: &str) -> Result<Value> {
        self.moves += 1;
        let client_move_id = format!("m-{}-{}", self.moves, v_of(view));
        self.action(
            "submitMove",
            vec![json!({
                "moveId": move_id,
                "turnId": view["turnId"],
                "clientMoveId": client_move_id,
            })],
        )
        .await
    }

    /// Play the first legal move on each of the player's turns (and continue
    /// past any held trick) until `stop` or game over.
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
                    stop(v)
                        || v["status"] != "active"
                        || is_player_turn(v)
                        || v["awaitingTrickReview"] == true
                })
                .await?;
            if stop(&view) || view["status"] != "active" {
                return Ok(view);
            }
            if view["awaitingTrickReview"] == true {
                self.action(
                    "reviewTrick",
                    vec![json!({ "action": "continue", "turnId": view["turnId"] })],
                )
                .await?;
                floor = v_of(&view) + 1;
                continue;
            }
            let id = first_legal(&view)?;
            let ack = self
                .submit(&view, &id)
                .await
                .with_context(|| format!("{what}: submit {id}"))?;
            if ack["ok"] == true {
                floor = ack["v"].as_u64().unwrap_or(0).max(v_of(&view) + 1);
            } else if ack["code"] == "stale_turn" {
                // The view was already behind: wait for the newer one.
                floor = v_of(&view) + 1;
            } else {
                bail!("{what}: move {id} refused: {ack}");
            }
        }
    }
}

// ------------------------------------------------------------------ tests

#[test]
fn full_match_through_the_server() -> Result<()> {
    run("full_match_through_the_server", full_match)
}

#[test]
fn boot_failures_and_a_second_server() -> Result<()> {
    run("boot_failures_and_a_second_server", boot_failures)
}

async fn full_match() -> Result<()> {
    let dir = data_dir_with(&SECRETS)?;
    let cfg = local_config(&dir, *ENGINE_PORT)?;
    let t = Instant::now();
    let server = euchre_server::start(cfg.clone()).await?;
    let boot = t.elapsed();
    let http_addr = server.http_addr;
    eprintln!(
        "boot: {boot:?} (http {http_addr}, engine {}, pid {:?})",
        server.engine_endpoint,
        server.engine_pid()
    );
    let result = play_through(server, &dir, cfg).await;
    if result.is_err() {
        eprintln!("full match failed; the servers were dropped");
    }
    result
}

async fn play_through(
    server: RunningServer,
    dir: &tempfile::TempDir,
    cfg: ServerConfig,
) -> Result<()> {
    let started = Instant::now();
    let http_addr = server.http_addr;
    let api = Api::new(&server)?;
    let (status, body) = api.get("/health", None).await?;
    ensure!(status == 200 && body == "ok", "health -> {status} {body}");

    // A second server in the same process is refused at once.
    let second = euchre_server::start(cfg.clone()).await;
    ensure!(
        matches!(
            second.as_ref().err().and_then(boot_error),
            Some(BootError::AlreadyRunning)
        ),
        "second start: {:?}",
        second.err()
    );

    // The guest, the table, the session, the socket.
    let (user, bearer) = api.enroll().await?;
    let game_id = api.create_table(&bearer).await?;
    let session = api.table_session(&game_id, &bearer).await?;
    let client = gateway_client(&api, &session).await?;
    let mut player = Player::connect(&client, &session).await?;
    let first = player.next_sync("initial sync").await?;
    ensure!(
        first["steps"] == json!([]) && first["view"]["you"] == 0,
        "initial sync: {first}"
    );
    eprintln!(
        "table {game_id} for guest {user} open in {:?}",
        started.elapsed()
    );

    // The review gate: on, held at the first finished trick, then continue.
    let ack = player
        .action(
            "reviewTrick",
            vec![json!({ "action": "set", "enabled": true })],
        )
        .await?;
    ensure!(ack == json!({ "queued": true }), "reviewTrick ack {ack}");
    let on = player
        .wait_view(0, "review on", |v| v["reviewTricks"] == true)
        .await?;

    // An illegal move at the first player turn: refused, legal set attached.
    let turn = player
        .wait_view(v_of(&on), "first turn", is_player_turn)
        .await?;
    let err = match player.submit(&turn, "not-a-move").await {
        Ok(ack) => bail!("an illegal move was accepted: {ack}"),
        Err(e) => e,
    };
    let parsed = parse_rivet_error(&err).with_context(|| format!("illegal move: {err:#}"))?;
    ensure!(
        parsed.is_user()
            && parsed.is("illegal_move")
            && parsed.metadata.as_ref().map(|m| &m["legal"]) == Some(&turn["legal"]),
        "illegal move error: {parsed:?}"
    );
    // The game goes on: the same turn takes a legal move.
    let id = first_legal(&turn)?;
    let ack = player.submit(&turn, &id).await?;
    ensure!(ack["ok"] == true, "legal move after the illegal one: {ack}");
    let after_illegal = ack["v"].as_u64().unwrap_or(0);

    let held = player
        .play_until(after_illegal, "first held trick", |v| {
            v["awaitingTrickReview"] == true
        })
        .await?;
    ensure!(
        held["phase"] == "trick_resolve" && held["status"] == "active",
        "held: {held}"
    );
    let held_turn = held["turnId"].clone();
    tokio::time::sleep(Duration::from_millis(800)).await;
    let still = player.drain()?.context("view")?;
    ensure!(
        still["turnId"] == held_turn && still["awaitingTrickReview"] == true,
        "the gate let go by itself: {still}"
    );
    player
        .action(
            "reviewTrick",
            vec![json!({ "action": "continue", "turnId": held_turn })],
        )
        .await?;
    let moved = player
        .wait_view(v_of(&still) + 1, "after continue", |v| {
            v["turnId"] != held_turn
        })
        .await?;
    player
        .action(
            "reviewTrick",
            vec![json!({ "action": "set", "enabled": false })],
        )
        .await?;
    let off = player
        .wait_view(v_of(&moved), "review off", |v| v["reviewTricks"] == false)
        .await?;
    eprintln!("review gate ok at {:?}", started.elapsed());

    // Disconnect at a player turn mid-hand; the reconnect resyncs the same turn.
    let turn = player
        .play_until(v_of(&off), "turn before the reconnect", is_player_turn)
        .await?;
    player.disconnect().await;
    let mut player = Player::connect(&client, &session).await?;
    let resync = player.next_sync("sync after the reconnect").await?;
    ensure!(
        resync["view"]["turnId"] == turn["turnId"]
            && resync["view"]["handNo"] == turn["handNo"]
            && v_of(&resync["view"]) >= v_of(&turn),
        "resync after the reconnect: {resync} vs {turn}"
    );
    eprintln!(
        "reconnect resynced hand {} turn {} at {:?}",
        turn["handNo"],
        turn["turnId"],
        started.elapsed()
    );

    // Play on to a player turn in hand 2 or later: restart the server there.
    let turn = player
        .play_until(v_of(&resync["view"]), "turn before the restart", |v| {
            is_player_turn(v) && v["handNo"].as_u64().unwrap_or(0) >= 2
        })
        .await?;
    ensure!(
        turn["status"] == "active",
        "the match ended too soon: {turn}"
    );
    player.disconnect().await;
    drop(client);

    // Drain: health says so, new tables are refused, existing tables still open.
    server.health().start_draining();
    let (status, body) = api.get("/health", None).await?;
    ensure!(
        status == 503 && body == "draining",
        "health while draining -> {status} {body}"
    );
    let (status, body) = api.post("/api/tables", Some(&bearer)).await?;
    ensure!(
        status == 503 && body["message"] == DRAINING_NEW_TABLE,
        "new table while draining -> {status} {body}"
    );
    api.table_session(&game_id, &bearer)
        .await
        .context("table session while draining")?;

    let t = Instant::now();
    server.shutdown().await?;
    let shutdown = t.elapsed();
    ports_closed(*ENGINE_PORT, http_addr.port()).await?;
    // The desktop restarts with `ServerConfig::local(same dir)` and no port: the
    // data dir's engine comes back on its first port, so the token signing
    // ring still matches and gateway tokens are issued (W3 review, P1).
    drop(cfg);
    let restart = desktop_config(dir)?;
    ensure!(
        restart.engine.engine_port == *ENGINE_PORT,
        "the restart picked engine port {}, not {}",
        restart.engine.engine_port,
        *ENGINE_PORT
    );
    let t = Instant::now();
    let server = euchre_server::start(restart).await?;
    let reboot = t.elapsed();
    ensure!(
        server.http_addr == http_addr,
        "the restart moved the HTTP port: {http_addr} -> {}",
        server.http_addr
    );
    eprintln!(
        "restart on {}: shutdown {shutdown:?}, boot {reboot:?}",
        dir.path().display()
    );

    // Same guest, same table, same turn.
    let session = api.table_session(&game_id, &bearer).await?;
    let client = gateway_client(&api, &session).await?;
    let mut player = Player::connect(&client, &session).await?;
    let resync = player.next_sync("sync after the restart").await?;
    ensure!(
        resync["view"]["turnId"] == turn["turnId"]
            && resync["view"]["handNo"] == turn["handNo"]
            && resync["view"]["score"] == turn["score"],
        "resync after the restart: {resync} vs {turn}"
    );

    // The rest of the match.
    let end = player
        .play_until(v_of(&resync["view"]), "match", |v| v["status"] != "active")
        .await?;
    ensure!(end["status"] == "complete", "match end: {end}");
    let winner = end["winnerTeam"].as_u64().context("winnerTeam")?;
    let outcome = if winner == 0 { "won" } else { "lost" };
    let moves = player.moves;
    let syncs = player.syncs_seen;
    player.disconnect().await;
    drop(client);
    eprintln!(
        "match complete at {:?}: hand {}, score {}, {outcome}",
        started.elapsed(),
        end["handNo"],
        end["score"]
    );

    // The profile recorded it.
    let row = wait_for_record(&api, &bearer).await?;
    ensure!(
        row["outcome"] == outcome && row["score"] == end["score"],
        "listGames row {row} vs the final view {end}"
    );
    let match_id = row["matchId"].as_str().context("matchId")?.to_owned();
    let (status, stats) = api
        .get("/api/profile/stats?profile=browser", Some(&bearer))
        .await?;
    let won = u64::from(outcome == "won");
    ensure!(
        status == 200 && stats["matches"] == 1 && stats["won"] == won && stats["lost"] == 1 - won,
        "stats -> {status} {stats}"
    );
    let (status, replay) = api
        .get(&format!("/api/games/{match_id}"), Some(&bearer))
        .await?;
    let hands = replay["hands"].as_array().map_or(0, Vec::len);
    ensure!(
        status == 200 && hands >= 1 && replay["status"] == "complete",
        "replay -> {status} {replay}"
    );

    let http_port = server.http_addr.port();
    let t = Instant::now();
    server.shutdown().await?;
    let shutdown = t.elapsed();
    ports_closed(*ENGINE_PORT, http_port).await?;
    eprintln!(
        "full match ok in {:?}: {moves} player moves and \
         {syncs} syncs after the restart, {hands} replay hands, final shutdown {shutdown:?}",
        started.elapsed()
    );
    Ok(())
}

/// The finished match in `GET /api/games` (default profile: the browser guest).
async fn wait_for_record(api: &Api, bearer: &str) -> Result<Value> {
    let deadline = Instant::now() + RECORD_DEADLINE;
    loop {
        let (status, page) = api.get("/api/games", Some(bearer)).await?;
        ensure!(status == 200, "listGames -> {status} {page}");
        let rows = page["rows"].as_array().cloned().unwrap_or_default();
        if let [row] = rows.as_slice()
            && row["status"] == "complete"
        {
            return Ok(row.clone());
        }
        ensure!(rows.len() <= 1, "listGames has more than one match: {page}");
        ensure!(
            Instant::now() < deadline,
            "the match was not recorded within {RECORD_DEADLINE:?}: {page}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn boot_failures() -> Result<()> {
    // A second server in this process, with the shared secrets, on its own
    // data dir and engine ports.
    let second_port = second_engine_port();
    let dir = data_dir_with(&SECRETS)?;
    let t = Instant::now();
    let server = euchre_server::start(local_config(&dir, second_port)?).await?;
    let boot = t.elapsed();
    let api = Api::new(&server)?;
    let (status, body) = api.get("/health", None).await?;
    ensure!(status == 200 && body == "ok", "health -> {status} {body}");
    let (_user, bearer) = api.enroll().await?;
    let game_id = api.create_table(&bearer).await?;
    api.table_session(&game_id, &bearer).await?;
    let http_port = server.http_addr.port();
    server.shutdown().await?;
    ports_closed(second_port, http_port).await?;
    eprintln!("second server: boot {boot:?}, table {game_id} created");

    // The same data dir on another engine port: refused before any engine
    // starts (engine 3 would refuse every gateway token there).
    let t = Instant::now();
    let refused = tokio::time::timeout(
        BOOT_FAILURE_LIMIT,
        euchre_server::start(local_config(&dir, *ENGINE_PORT)?),
    )
    .await
    .context("the moved-port boot hung")?;
    let failed_in = t.elapsed();
    let err = refused.err().context("booted on a moved engine port")?;
    ensure!(
        matches!(
            boot_error(&err),
            Some(BootError::EnginePortChanged { recorded, requested })
                if *recorded == second_port && *requested == *ENGINE_PORT
        ),
        "moved port: {err:#}"
    );
    ensure!(
        failed_in < BUSY_PORT_LIMIT,
        "the moved-port boot took {failed_in:?}"
    );
    ensure!(
        !listening(*ENGINE_PORT),
        "an engine runs after the moved-port refusal"
    );
    eprintln!("moved engine port in {failed_in:?}: {err}");

    // Other secrets in the same process: refused before any engine starts.
    let other = tempfile::Builder::new()
        .prefix("euchre-server-e2e-")
        .tempdir()?;
    let t = Instant::now();
    let refused = tokio::time::timeout(
        BOOT_FAILURE_LIMIT,
        euchre_server::start(local_config(&other, second_port)?),
    )
    .await
    .context("the keys-conflict boot hung")?;
    let err = refused.err().context("other secrets booted")?;
    ensure!(
        matches!(boot_error(&err), Some(BootError::KeysConflict)),
        "other secrets: {err:#}"
    );
    ensure!(
        !listening(second_port),
        "an engine runs after the keys conflict"
    );
    eprintln!("keys conflict in {:?}: {err}", t.elapsed());

    // A busy engine port: a clear error naming it, not a hang.
    let squatter = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, second_port)))
        .context("hold the engine port")?;
    let t = Instant::now();
    let refused = tokio::time::timeout(
        BOOT_FAILURE_LIMIT,
        euchre_server::start(local_config(&dir, second_port)?),
    )
    .await
    .context("the busy-port boot hung")?;
    let failed_in = t.elapsed();
    drop(squatter);
    let err = refused.err().context("booted on a busy engine port")?;
    ensure!(
        matches!(
            boot_error(&err),
            Some(BootError::EnginePortInUse { port, .. }) if *port == second_port
        ),
        "busy port: {err:#}"
    );
    ensure!(
        failed_in < BUSY_PORT_LIMIT,
        "the busy-port boot took {failed_in:?}"
    );
    eprintln!("busy engine port in {failed_in:?}: {err}");
    ports_closed(second_port, http_port).await?;

    // The lease was released: the shared secrets boot again afterwards, with
    // no port given (the data dir's own).
    let server = euchre_server::start(desktop_config(&dir)?).await?;
    let http_port = server.http_addr.port();
    let api = Api::new(&server)?;
    api.table_session(&game_id, &bearer)
        .await
        .context("table session after the reboot")?;

    // The engine dies under a running server: it stops serving, says why, and
    // still shuts down cleanly.
    let pid = server.engine_pid().context("no engine pid")?;
    let killed = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status()
        .context("kill the engine")?;
    ensure!(killed.success(), "kill -KILL {pid}: {killed}");
    let t = Instant::now();
    let failure = tokio::time::timeout(ENGINE_DEATH_LIMIT, server.closed())
        .await
        .context("the dead engine went unnoticed")?;
    let noticed = t.elapsed();
    ensure!(
        matches!(failure, ServerFailure::EngineExited { .. }),
        "failure: {failure}"
    );
    ensure!(server.failure() == Some(failure.clone()));
    let (status, body) = api.get("/health", None).await?;
    ensure!(
        status == 503,
        "health after the engine died -> {status} {body}"
    );
    let t = Instant::now();
    server.shutdown().await?;
    let stopped = t.elapsed();
    ports_closed(second_port, http_port).await?;
    eprintln!("dead engine noticed in {noticed:?} ({failure}); shutdown {stopped:?}");
    Ok(())
}
