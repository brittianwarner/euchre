//! The remote backend against a real server: `euchre_server::start` on loopback with its own
//! engine, then `RemoteBackend` exactly as the app builds it for `EUCHRE_SERVER_URL`
//! (contract §N1.9). Enroll, create, open over the `/rivet` gateway, play, an illegal move,
//! a reconnect, another device refused, recent tables, the profile calls (a blank persona
//! refused), a stale turn refused, the match played to the end (its scrapbook row, its
//! detail, the stats, a move after the end refused, an unknown match), close, shutdown.
//!
//! Engine test: it skips unless `EUCHRE_E2E=1`. The engine binds `P`, `P+1` and `P+5`, where
//! `P` is `EUCHRE_TEST_ENGINE_PORT` (default 6741). The data root is
//! `$EUCHRE_TEST_TMP/n1-<pid>` (else the system temp dir), printed at the start.
//!
//! ```text
//! EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6741 EUCHRE_TEST_TMP=<dir> \
//!   cargo test -p euchre-desktop --test remote_live -- --nocapture
//! ```

use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail, ensure};
use euchre_ai::{RankParams, top_move};
use euchre_desktop::net::remote::{RemoteBackend, RemoteConfig};
use euchre_desktop::net::{Backend, ConnEvent, NetError, TableConfig, TableConn};
use euchre_profile_model::{PersonaPatch, PersonaPatchEntry, SettingsPatch};
use euchre_protocol::{
    GameStatus, MoveAck, PublicGameView, ReviewTrickRequest, SubmitMoveRequest, TableEvent,
};
use euchre_server::ServerConfig;
use euchre_server_core::config::AiConfig;
use euchre_server_core::testing::{e2e_enabled, e2e_skip};

const DEFAULT_ENGINE_PORT: u16 = 6741;
/// The whole test.
const TEST_LIMIT: Duration = Duration::from_secs(420);
/// `Status(true)` and the first `sync` after an open or a reconnect (contract: 10 s).
const LIVE: Duration = Duration::from_secs(10);
/// Waiting for the player's turn while three AI seats play (fast tempo, offline AI).
const TURN: Duration = Duration::from_secs(90);
/// One bounded call.
const CALL: Duration = Duration::from_secs(15);

fn engine_port() -> anyhow::Result<u16> {
    match std::env::var("EUCHRE_TEST_ENGINE_PORT") {
        Ok(v) => v
            .parse()
            .with_context(|| format!("EUCHRE_TEST_ENGINE_PORT `{v}` is not a port")),
        Err(_) => Ok(DEFAULT_ENGINE_PORT),
    }
}

fn data_root() -> anyhow::Result<PathBuf> {
    let base = std::env::var_os("EUCHRE_TEST_TMP")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = base.join(format!("n1-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    Ok(root)
}

fn listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_millis(200),
    )
    .is_ok()
}

/// `ports` stop accepting within 5 s.
async fn ports_closed(ports: &[u16]) -> anyhow::Result<()> {
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

/// `d` in milliseconds, to 0.1 ms.
fn ms(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1000.0)
}

fn mode(p: &Path) -> anyhow::Result<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Ok(std::fs::metadata(p)?.permissions().mode() & 0o777)
}

/// The player's table: its stream, the latest view and the last status edge.
struct Table {
    conn: TableConn,
    view: Option<PublicGameView>,
    up: bool,
}

impl Table {
    /// The next stream item within `within`, folding it into `view` / `up`.
    async fn next(&mut self, within: Duration) -> anyhow::Result<ConnEvent> {
        let item = tokio::time::timeout(within, self.conn.events.recv())
            .await
            .context("no table event in time")?
            .context("the table stream ended")?;
        match &item {
            ConnEvent::Status(up) => self.up = *up,
            ConnEvent::Event(TableEvent::Sync(sync)) => {
                if self.view.as_ref().is_none_or(|v| sync.v >= v.v) {
                    self.view = Some(sync.view.clone());
                }
            }
            ConnEvent::Event(_) => {}
            ConnEvent::Ended(err) => bail!("the table ended: {err:?}"),
        }
        Ok(item)
    }

    /// Reads the stream until `pred` holds on an item, within `within`.
    async fn until(
        &mut self,
        within: Duration,
        mut pred: impl FnMut(&ConnEvent, &Table) -> bool,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            ensure!(!left.is_zero(), "waited {within:?}");
            let item = self.next(left).await?;
            if pred(&item, self) {
                return Ok(());
            }
        }
    }

    /// The latest view, when it is the player's turn and at least `min_v` (a view older than
    /// the player's last ack still shows the turn the player just took).
    fn players_turn(&self, min_v: u64) -> Option<&PublicGameView> {
        self.view.as_ref().filter(|v| {
            v.v >= min_v
                && v.status == GameStatus::Active
                && v.turn_seat == Some(v.you)
                && v.awaiting_trick_review != Some(true)
                && !v.legal.is_empty()
        })
    }

    /// Waits for the player's turn at `v >= min_v`, continuing any trick held for review.
    async fn players_turn_view(&mut self, min_v: u64) -> anyhow::Result<PublicGameView> {
        self.turn_or_end(min_v)
            .await?
            .context("the match ended before the player's turn")
    }

    /// [`Table::players_turn_view`], or `None` once the match is over.
    async fn turn_or_end(&mut self, min_v: u64) -> anyhow::Result<Option<PublicGameView>> {
        let deadline = Instant::now() + TURN;
        let mut continued: Option<String> = None;
        loop {
            if let Some(view) = self.players_turn(min_v) {
                return Ok(Some(view.clone()));
            }
            if self
                .view
                .as_ref()
                .is_some_and(|v| v.v >= min_v && v.status != GameStatus::Active)
            {
                return Ok(None);
            }
            if let Some(view) = self.view.as_ref().filter(|v| {
                v.v >= min_v
                    && v.awaiting_trick_review == Some(true)
                    && continued.as_deref() != Some(v.turn_id.as_str())
            }) {
                let turn_id = view.turn_id.clone();
                continued = Some(turn_id.clone());
                let _ = self
                    .conn
                    .calls
                    .review_trick(ReviewTrickRequest::Continue { turn_id })
                    .await;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            ensure!(
                !left.is_zero(),
                "the player's turn did not come in {TURN:?}"
            );
            self.next(left).await?;
        }
    }
}

fn submit(view: &PublicGameView, move_id: String) -> SubmitMoveRequest {
    SubmitMoveRequest {
        move_id,
        turn_id: Some(view.turn_id.clone()),
        client_move_id: uuid::Uuid::new_v4().to_string(),
    }
}

/// What the report prints.
#[derive(Debug, Default)]
struct Timings {
    boot: Duration,
    enroll: Duration,
    identity_again: Duration,
    create: Duration,
    open: Duration,
    first_sync: Duration,
    move_ack: Duration,
    rejected_ack: Duration,
    stale_ack: Duration,
    match_end: Duration,
    listed: Duration,
    after_end_ack: Duration,
    reconnect: Duration,
    forbidden: Duration,
    recent: Duration,
    close: Duration,
    shutdown: Duration,
}

async fn steps(server_url: &str, root: &Path, t: &mut Timings) -> anyhow::Result<()> {
    let config = RemoteConfig::remote(server_url, root)?;
    println!("state dir: {}", config.state_dir.display());
    let backend = RemoteBackend::new(config.clone())?;

    // 1. Enroll on the first call; the second makes no new enroll.
    let started = Instant::now();
    let me = backend.identity().await?;
    t.enroll = started.elapsed();
    ensure!(me.user_id.starts_with("guest-"), "user id {}", me.user_id);
    let device = config.state_dir.join("device.json");
    ensure!(
        mode(&device)? == 0o600,
        "device.json mode {:o}",
        mode(&device)?
    );
    ensure!(mode(&config.state_dir)? == 0o700, "state dir mode");
    let saved = std::fs::read(&device)?;
    let started = Instant::now();
    let again = backend.identity().await?;
    t.identity_again = started.elapsed();
    ensure!(
        again.user_id == me.user_id,
        "a second identity changed the user"
    );
    ensure!(
        std::fs::read(&device)? == saved,
        "a second identity rewrote device.json"
    );

    // 2. Create, open, Status(true), then a sync.
    let started = Instant::now();
    let game_id = backend
        .create_table(TableConfig {
            stick_the_dealer: false,
            require_natural_trump: false,
        })
        .await?;
    t.create = started.elapsed();
    println!("table: {game_id}");
    let started = Instant::now();
    let conn = backend.open_table(&game_id).await?;
    t.open = started.elapsed();
    ensure!(conn.game_id == game_id);
    let mut table = Table {
        conn,
        view: None,
        up: false,
    };
    let first = table.next(LIVE).await?;
    ensure!(first == ConnEvent::Status(true), "first item {first:?}");
    table
        .until(LIVE, |item, _| {
            matches!(item, ConnEvent::Event(TableEvent::Sync(_)))
        })
        .await?;
    t.first_sync = started.elapsed();

    // 3. The player's turn: the heuristic's top move lands.
    let view = table.players_turn_view(0).await?;
    let chosen = top_move(RankParams::new(&view)).context("no move to play")?;
    let started = Instant::now();
    let ack = table.conn.calls.submit_move(submit(&view, chosen)).await?;
    t.move_ack = started.elapsed();
    let MoveAck::Ok { v: landed } = ack else {
        bail!("move ack {ack:?}");
    };
    ensure!(
        landed > view.v,
        "the move landed at v {landed} <= {}",
        view.v
    );

    // 4. A made-up move on the player's next turn: rejected with the legal set.
    let view = table.players_turn_view(landed).await?;
    let started = Instant::now();
    let ack = table
        .conn
        .calls
        .submit_move(submit(&view, "not-a-real-move".to_owned()))
        .await?;
    t.rejected_ack = started.elapsed();
    match &ack {
        MoveAck::Rejected { code, legal } => {
            ensure!(code.as_str() == "illegal_move", "code {}", code.as_str());
            ensure!(!legal.is_empty(), "no legal set with the rejection");
            ensure!(
                *legal == view.legal,
                "the rejection's legal set is not the turn's"
            );
        }
        MoveAck::Ok { .. } => bail!("a made-up move landed"),
    }

    // 5. A blip: Status(false), then Status(true); the snapshot is not behind.
    let v_before = table.view.as_ref().map_or(0, |v| v.v);
    let started = Instant::now();
    backend.interrupt_tables();
    table
        .until(LIVE, |item, _| *item == ConnEvent::Status(false))
        .await?;
    table
        .until(LIVE, |item, _| *item == ConnEvent::Status(true))
        .await?;
    t.reconnect = started.elapsed();
    let snap = table
        .conn
        .calls
        .snapshot()
        .await?
        .context("no snapshot after the reconnect")?;
    ensure!(snap.v >= v_before, "snapshot v {} < {v_before}", snap.v);

    // 6. Another device cannot open this table.
    let other = RemoteBackend::new(RemoteConfig::remote(
        server_url,
        &root.join("other-device"),
    )?)?;
    let started = Instant::now();
    match other.open_table(&game_id).await {
        Err(NetError::Forbidden) => {}
        Err(err) => bail!("another device's open: {err:?}"),
        Ok(_) => bail!("another device opened the table"),
    }
    t.forbidden = started.elapsed();

    // 7. Recent tables lists it, live.
    let started = Instant::now();
    let recent = backend.recent_tables().await?;
    t.recent = started.elapsed();
    let row = recent
        .tables
        .iter()
        .find(|r| r.game_id == game_id)
        .context("the table is not in recent tables")?;
    ensure!(
        row.status == Some(GameStatus::Active),
        "recent status {:?}",
        row.status
    );
    ensure!(row.hand_no.is_some(), "recent hand_no");

    // 8. The profile calls answer; a patch round-trips.
    let page = tokio::time::timeout(CALL, backend.list_games(None)).await??;
    println!("games listed: {}", page.rows.len());
    let stats = tokio::time::timeout(CALL, backend.stats()).await??;
    println!("matches recorded: {}", stats.matches);
    let before = tokio::time::timeout(CALL, backend.get_settings()).await??;
    let prompt = format!("Desktop check {}", std::process::id());
    let saved = tokio::time::timeout(
        CALL,
        backend.patch_settings(SettingsPatch {
            house_prompt: Some(prompt.clone()),
            ..SettingsPatch::default()
        }),
    )
    .await??;
    ensure!(saved.house_prompt == prompt, "patch not applied");
    ensure!(
        saved.version > before.version,
        "patch did not bump the version"
    );
    let read = tokio::time::timeout(CALL, backend.get_settings()).await??;
    ensure!(read.house_prompt == prompt, "patch not saved");
    // A blank persona name is refused with its code, and changes nothing.
    let blank = tokio::time::timeout(
        CALL,
        backend.patch_settings(SettingsPatch {
            personas: Some(vec![PersonaPatchEntry {
                seat: 1,
                patch: PersonaPatch {
                    name: Some("   ".to_owned()),
                    ..PersonaPatch::default()
                },
            }]),
            ..SettingsPatch::default()
        }),
    )
    .await?;
    ensure!(
        blank == Err(NetError::Invalid("invalid_persona".to_owned())),
        "a blank persona: {blank:?}"
    );
    let unchanged = tokio::time::timeout(CALL, backend.get_settings()).await??;
    ensure!(unchanged == read, "a refused patch changed the settings");

    // 9. Play the match to the end (review fix PP-2). On the way, a move for a turn that
    //    has passed is refused as stale and changes nothing.
    let started = Instant::now();
    let mut min_v = table.view.as_ref().map_or(0, |v| v.v);
    let mut previous: Option<PublicGameView> = None;
    let mut stale_done = false;
    while let Some(view) = table.turn_or_end(min_v).await? {
        let chosen = top_move(RankParams::new(&view)).context("no move to play")?;
        if !stale_done && let Some(old) = previous.as_ref().filter(|o| o.turn_id != view.turn_id) {
            let at = Instant::now();
            let ack = table
                .conn
                .calls
                .submit_move(submit(old, chosen.clone()))
                .await?;
            t.stale_ack = at.elapsed();
            match &ack {
                MoveAck::Rejected { code, .. } => {
                    ensure!(
                        code.as_str() == "stale_turn",
                        "stale code {}",
                        code.as_str()
                    );
                }
                MoveAck::Ok { .. } => bail!("a move for a passed turn landed"),
            }
            stale_done = true;
        }
        let ack = table.conn.calls.submit_move(submit(&view, chosen)).await?;
        let MoveAck::Ok { v } = ack else {
            bail!("move ack {ack:?}");
        };
        min_v = v;
        previous = Some(view);
    }
    t.match_end = started.elapsed();
    ensure!(stale_done, "no second turn to try a stale move on");
    let last = table.view.clone().context("no final view")?;
    ensure!(
        last.status == GameStatus::Complete,
        "status {:?}",
        last.status
    );
    ensure!(
        last.score.iter().any(|&p| p >= 10),
        "final score {:?}",
        last.score
    );
    // A move after the end is refused as such.
    let at = Instant::now();
    let after = table
        .conn
        .calls
        .submit_move(SubmitMoveRequest {
            move_id: "pass".to_owned(),
            turn_id: Some(last.turn_id.clone()),
            client_move_id: uuid::Uuid::new_v4().to_string(),
        })
        .await?;
    t.after_end_ack = at.elapsed();
    match &after {
        MoveAck::Rejected { code, .. } => {
            ensure!(
                code.as_str() == "game_complete",
                "after the end: {}",
                code.as_str()
            );
        }
        MoveAck::Ok { .. } => bail!("a move after the end landed"),
    }
    // One scrapbook row for the match, and its detail decodes; the stats count it.
    let started = Instant::now();
    let (page, stats) = loop {
        let page = tokio::time::timeout(CALL, backend.list_games(None)).await??;
        let stats = tokio::time::timeout(CALL, backend.stats()).await??;
        if page.rows.iter().any(|r| r.match_id == game_id) && stats.matches >= 1 {
            break (page, stats);
        }
        ensure!(
            started.elapsed() < Duration::from_secs(30),
            "the match was not recorded: {} rows, {} matches",
            page.rows.len(),
            stats.matches
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    t.listed = started.elapsed();
    ensure!(page.rows.len() == 1, "{} scrapbook rows", page.rows.len());
    ensure!(stats.matches == 1, "stats.matches {}", stats.matches);
    let detail = tokio::time::timeout(CALL, backend.match_detail(&game_id)).await??;
    ensure!(!detail.hands.is_empty(), "the detail has no hands");
    println!(
        "match over at {:?}: {} hands recorded",
        last.score,
        detail.hands.len()
    );
    let unknown = uuid::Uuid::new_v4().to_string();
    let missing = tokio::time::timeout(CALL, backend.match_detail(&unknown)).await?;
    ensure!(
        missing.as_ref().err() == Some(&NetError::NotFound),
        "an unknown match: {:?}",
        missing.map(|_| ())
    );

    // 10. Close ends the stream with no Ended.
    let started = Instant::now();
    table.conn.calls.close().await;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, table.conn.events.recv()).await {
            Ok(None) => break,
            Ok(Some(ConnEvent::Ended(err))) => bail!("close sent Ended({err:?})"),
            Ok(Some(_)) => {}
            Err(_) => bail!("the stream did not end after close"),
        }
    }
    t.close = started.elapsed();
    ensure!(
        table.conn.calls.snapshot().await == Err(NetError::Closed),
        "a closed table still answers"
    );
    Ok(())
}

#[test]
fn remote_backend_plays_against_a_real_server() -> anyhow::Result<()> {
    if !e2e_enabled() {
        e2e_skip("remote_backend_plays_against_a_real_server");
        return Ok(());
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_test_writer()
        .try_init();
    let port = engine_port()?;
    let root = data_root()?;
    println!("data root: {}", root.display());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    rt.block_on(async {
        let mut cfg = ServerConfig::try_local(root.join("server"))?.with_engine_port(port);
        cfg.ai = AiConfig::default();
        cfg.fast_tempo = true;
        let mut t = Timings::default();
        let started = Instant::now();
        let server = euchre_server::start(cfg).await?;
        t.boot = started.elapsed();
        let url = server.url();
        let http_port = server.http_addr.port();
        println!("server: {url} (engine port {port})");
        let result = match tokio::time::timeout(TEST_LIMIT, steps(&url, &root, &mut t)).await {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("did not finish within {TEST_LIMIT:?}")),
        };
        let started = Instant::now();
        let stopped = server.shutdown().await;
        t.shutdown = started.elapsed();
        let closed = ports_closed(&[port, port + 1, port + 5, http_port]).await;
        println!(
            "timings ms: boot {} | enroll {} | identity again {} | create table {} | open {} | \
             first sync {} | move ack {} | rejected ack {} | stale ack {} | reconnect {} | \
             forbidden open {} | recent {} | match to the end {} | after-end ack {} | \
             recorded {} | close {} | shutdown {}",
            ms(t.boot),
            ms(t.enroll),
            ms(t.identity_again),
            ms(t.create),
            ms(t.open),
            ms(t.first_sync),
            ms(t.move_ack),
            ms(t.rejected_ack),
            ms(t.stale_ack),
            ms(t.reconnect),
            ms(t.forbidden),
            ms(t.recent),
            ms(t.match_end),
            ms(t.after_end_ack),
            ms(t.listed),
            ms(t.close),
            ms(t.shutdown),
        );
        result?;
        stopped?;
        closed
    })
}
