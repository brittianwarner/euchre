//! The whole desktop, live (integration, contract §3): one full match to game over played by
//! clicks on the real UI, through the bridge thread, the `TableStore`, the `RemoteBackend` and
//! the embedded server on a real engine. Skipped unless `EUCHRE_E2E=1`.
//!
//! On the way it proves:
//! - the table opened while the server boots shows the booting line, then deals;
//! - a made-up move from a second client of the same device is refused by the server
//!   (`illegal_move`, with the turn's legal set) and the app's table does not move;
//! - the review gate holds a finished trick until Continue;
//! - a dropped socket mid-hand shows Reconnecting, then the same turn comes back;
//! - the engine stopping mid-match ends the table with Restart; Restart serves the same turn;
//! - the finished match lists in Your games (with its saved table) and opens to its hands;
//! - Quit stops the engine and frees every port.
//!
//! It prints wall time and per-move latency, writes the real syncs as `scenes::LiveFixture`
//! files and renders them with `--scene live-*` (the gallery's inputs).
//!
//! - `EUCHRE_TEST_ENGINE_PORT`: the engine port `P` (default 6870; the engine also binds P+1
//!   and P+5).
//! - `EUCHRE_TEST_TMP`: the parent of the run's root, `<parent>/integration-<port>` (printed:
//!   data, fixtures, snaps and `timings.json`). Removed when the run passes unless
//!   `EUCHRE_LIVE_KEEP=1` (the gallery's inputs); kept when it fails. A run that was killed
//!   leaves it behind: the next run on the same port stops the engine it left (only one whose
//!   command line names that root's storage) and starts clean (review fix LDS-5).
//!
//! It never touches the player's data dir and only ever signals the engines it started:
//! every signal is checked against the engine's command line first (review fix LDS-7).

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use euchre_ai::{RankParams, top_move};
use euchre_desktop::app::{AppState, Route, TableStage};
use euchre_desktop::net::bridge::{Bridge, BridgeConfig, BridgeThread, Intent, Reply, UiSink};
use euchre_desktop::net::embedded::{EmbeddedBackend, EmbeddedConfig};
use euchre_desktop::net::local;
use euchre_desktop::net::remote::{RemoteBackend, RemoteConfig};
use euchre_desktop::net::{Backend, ConnEvent, NetError, RecentTables, ServerPhase};
use euchre_desktop::prefs::Prefs;
use euchre_desktop::scenes::LiveFixture;
use euchre_desktop::screens::server::BOOTING_LINE;
use euchre_desktop::theme::motion;
use euchre_desktop::{spec, theme};
use euchre_profile_model::{ClientMatchReplay, ProfileStats};
use euchre_protocol::store::SubmitOutcome;
use euchre_protocol::{
    GamePhase, GameStatus, MatchListPage, Move, MoveAck, PublicGameView, Step, SubmitMoveRequest,
    SyncEvent, TableEvent,
};
use euchre_server_core::testing::{e2e_enabled, e2e_skip};
use rune::{App, Env, Runtime};

const DT: f64 = 1.0 / 60.0;
const BOOT: Duration = Duration::from_secs(120);
const TURN: Duration = Duration::from_secs(30);
const STOP_NOTICED: Duration = Duration::from_secs(15);
const PORTS_CLOSE: Duration = Duration::from_secs(5);
const QUIT: Duration = Duration::from_secs(45);
const MATCH: Duration = Duration::from_secs(420);
/// How long the review gate must hold a finished trick with nobody pressing Continue.
const REVIEW_HOLD: Duration = Duration::from_secs(2);

/// The app, its bridge and the engines this run started.
struct Run {
    rt: Runtime<AppState>,
    replies: Receiver<Reply>,
    /// A second sender, for the intents a test sends past the UI (a Games refresh).
    bridge: Bridge,
    thread: Option<BridgeThread>,
    backend: EmbeddedBackend,
    /// Every engine pid seen, for the clean-up after a failure.
    pids: Vec<u32>,
    /// The engine's storage: a pid is signalled only while its command line names it.
    storage: PathBuf,
    /// Every HTTP port the server served on.
    http_ports: Vec<u16>,
    /// Every table sync, by connection, in arrival order.
    syncs: Vec<(u64, SyncEvent)>,
    phases: Vec<ServerPhase>,
    games: Option<MatchListPage>,
    recent: Option<RecentTables>,
    replay: Option<ClientMatchReplay>,
    stats: Option<ProfileStats>,
    /// The press that sent the move waiting for its ack.
    pressed: Option<Instant>,
    /// Press to `Reply::Submit`, per move.
    acks: Vec<Duration>,
    /// Acks that were not `Ok`.
    odd_acks: Vec<String>,
}

impl Run {
    fn start(data: &Path, port: u16) -> Run {
        let config = EmbeddedConfig::for_tests(data.to_path_buf(), port);
        let storage = config.engine_storage();
        let backend = EmbeddedBackend::new(config);
        let (bridge, intents) = Bridge::channel();
        let (sink, replies) = UiSink::channel();
        let thread = BridgeThread::spawn(
            BridgeConfig {
                backend: Arc::new(backend.clone()),
                sink,
                prefs: None,
                quit_on_signals: false,
            },
            intents,
        )
        .expect("bridge thread");
        // The tour opens by itself over a first real hand; this run clicks by position.
        let prefs = Prefs {
            tour_seen: true,
            ..Prefs::default()
        };
        let mut state = AppState::new(prefs, Some(bridge.clone()), "embedded");
        state.wait_for_art = true;
        let mut rt = App::new(spec())
            .init(move || state)
            .headless_runtime(theme::window::SIZE, Env::new(false, 1.0));
        rt.frame(0.0);
        // What the window's on_ready does.
        rt.update(|s, _| s.on_ready());
        Run {
            rt,
            replies,
            bridge,
            thread: Some(thread),
            backend,
            pids: Vec::new(),
            storage,
            http_ports: Vec::new(),
            syncs: Vec::new(),
            phases: Vec::new(),
            games: None,
            recent: None,
            replay: None,
            stats: None,
            pressed: None,
            acks: Vec::new(),
            odd_acks: Vec::new(),
        }
    }

    /// Notes what a reply carries before the app takes it.
    fn observe(&mut self, reply: &Reply) {
        match reply {
            Reply::Conn {
                conn_id,
                event: ConnEvent::Event(TableEvent::Sync(sync)),
            } => self.syncs.push((*conn_id, (**sync).clone())),
            Reply::Submit { outcome, .. } => {
                if let Some(at) = self.pressed.take() {
                    self.acks.push(at.elapsed());
                }
                if !matches!(outcome, SubmitOutcome::Ack(MoveAck::Ok { .. })) {
                    self.odd_acks.push(format!("{outcome:?}"));
                }
            }
            Reply::Server(phase) => {
                self.phases.push(phase.clone());
                if *phase == ServerPhase::Ready {
                    if let Some(pid) = self.backend.supervisor().engine_pid()
                        && !self.pids.contains(&pid)
                    {
                        self.pids.push(pid);
                    }
                    if let Some(port) = self
                        .backend
                        .remote()
                        .and_then(|r| r.config().and_then(|c| http_port(&c.base_url)))
                        && !self.http_ports.contains(&port)
                    {
                        self.http_ports.push(port);
                    }
                }
            }
            Reply::Games {
                result: Ok(page), ..
            } => self.games = Some(page.clone()),
            Reply::RecentTables(Ok(recent)) => self.recent = Some(recent.clone()),
            Reply::MatchDetail {
                result: Ok(replay), ..
            } => self.replay = Some(replay.clone()),
            Reply::Stats(Ok(stats)) => self.stats = Some(stats.clone()),
            _ => {}
        }
    }

    /// Pumps replies into the app and runs frames until `done` or the deadline.
    fn pump(&mut self, within: Duration, done: impl Fn(&AppState) -> bool) -> bool {
        let end = Instant::now() + within;
        while Instant::now() < end {
            while let Ok(reply) = self.replies.try_recv() {
                self.observe(&reply);
                self.rt.update(move |s, tx| s.receive(reply, tx));
            }
            self.rt.frame(DT);
            if done(&self.rt.state) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    /// Like [`Run::pump`], but a miss fails the run with the table's state.
    fn must(&mut self, what: &str, within: Duration, done: impl Fn(&AppState) -> bool) {
        if !self.pump(within, done) {
            panic!("timed out: {what}\n{}", describe(&self.rt.state));
        }
    }

    fn settle(&mut self, seconds: f64) {
        for _ in 0..(seconds / DT).ceil() as usize {
            self.rt.frame(DT);
        }
    }

    fn click(&mut self, key: &str) {
        let (x, y) = self
            .rt
            .find(key)
            .unwrap_or_else(|| panic!("{key} is on screen\n{}", describe(&self.rt.state)));
        self.rt.pointer_down(x, y);
        self.rt.pointer_up(x, y);
        self.rt.frame(DT);
    }

    /// Runs frames until `key` is fully shown and still (at most two seconds).
    fn landed(&mut self, key: &str) {
        for _ in 0..120 {
            let shown = self
                .rt
                .elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some(key) && !e.exiting);
            if shown.is_some_and(|e| e.opacity >= 0.99 && !e.moving) {
                return;
            }
            self.rt.frame(DT);
        }
    }

    fn texts(&self) -> Vec<String> {
        self.rt
            .elements()
            .into_iter()
            .filter(|e| !e.exiting)
            .filter_map(|e| e.text)
            .collect()
    }

    fn view(&self) -> PublicGameView {
        view(&self.rt.state)
            .cloned()
            .unwrap_or_else(|| panic!("a view\n{}", describe(&self.rt.state)))
    }

    fn engine_pid(&self) -> u32 {
        self.backend
            .supervisor()
            .engine_pid()
            .expect("an engine pid")
    }

    fn url(&self) -> String {
        self.backend
            .remote()
            .and_then(|r| r.config().map(|c| c.base_url.clone()))
            .expect("a server url")
    }

    /// Plays the player's move with the heuristic's pick, the way a person would: a card is
    /// pressed then Play, a bid is its button. Returns press-to-landed.
    fn play_top_move(&mut self) -> Duration {
        let v = self.view();
        let chosen = top_move(RankParams::new(&v)).unwrap_or_else(|| v.legal[0].id.clone());
        let plain = chosen.strip_suffix("+alone").unwrap_or(&chosen).to_owned();
        let entry = v
            .legal
            .iter()
            .find(|m| m.id == plain)
            .or_else(|| v.legal.iter().find(|m| m.id == chosen))
            .expect("the pick is legal")
            .clone();
        let pressed = match entry.mv {
            Move::Play { card } | Move::Discard { card } => {
                let key = format!("hand-{card}");
                self.landed(&key);
                self.click(&key);
                self.landed("hand-play");
                let at = Instant::now();
                self.pressed = Some(at);
                self.click("hand-play");
                at
            }
            _ => {
                let key = format!("bid-{}", entry.id);
                let at = Instant::now();
                self.pressed = Some(at);
                if self.rt.find(&key).is_some() {
                    self.click(&key);
                } else {
                    let id = entry.id.clone();
                    self.rt.update(move |s, _| s.submit_move(&id));
                }
                at
            }
        };
        let before = v.v;
        self.must(&format!("move {} landed", entry.id), TURN, |s| {
            view(s).is_some_and(|w| w.v > before)
                && s.table.as_ref().is_some_and(|t| !t.store.submitting())
        });
        pressed.elapsed()
    }

    /// Sends `Shutdown` and waits for the bridge to end; how long it took.
    fn quit(&mut self) -> Duration {
        let started = Instant::now();
        self.bridge.send(Intent::Shutdown);
        let ended = self.thread.take().is_none_or(|t| t.join_within(QUIT));
        assert!(ended, "the bridge ended within {QUIT:?}");
        started.elapsed()
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.bridge.send(Intent::Shutdown);
            let _ = thread.join_within(QUIT);
        }
        // Only ever the engines this run started, only if they outlived the stop, and only
        // while the pid is still that engine (a pid can be reused).
        for pid in &self.pids {
            if local::is_engine_of(*pid, &self.storage) {
                eprintln!("engine {pid} outlived the run; stopping it");
                let _ = std::process::Command::new("kill")
                    .args(["-TERM", &pid.to_string()])
                    .status();
            }
        }
    }
}

/// Stops the engine an earlier, killed run left on `root`'s storage (only one whose command
/// line names that storage, and whose owner is gone), then clears `root`.
fn reclaim(root: &Path) {
    let storage = EmbeddedConfig::for_tests(root.join("data"), 0).engine_storage();
    if let Some((engine, _)) = local::recorded_engine(&storage)
        && local::other_live_owner(&storage).is_none()
        && local::is_engine_of(engine, &storage)
    {
        eprintln!("stopping engine {engine}, left by an earlier run");
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &engine.to_string()])
            .status();
        let deadline = Instant::now() + Duration::from_secs(15);
        while local::is_engine_of(engine, &storage) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

fn view(state: &AppState) -> Option<&PublicGameView> {
    state.table.as_ref().and_then(|t| t.store.display_view())
}

/// One line of the table's state, for a failure message.
fn describe(s: &AppState) -> String {
    let Some(t) = s.table.as_ref() else {
        return format!("route {:?}, no table, server {:?}", s.route, s.local_server);
    };
    let v = t.store.display_view();
    format!(
        "route {:?}, server {:?}, stage {:?}, connected {}, submitting {}, review_busy {}, \
         error {:?}, review_error {:?}, v {:?}, phase {:?}, hand {:?}, turn {:?}, you {:?}, \
         legal {:?}, awaiting {:?}, collect {:?}",
        s.route,
        s.local_server,
        t.stage,
        t.store.is_connected(),
        t.store.submitting(),
        t.store.review_busy(),
        t.store.error(),
        t.store.review_error(),
        v.map(|v| v.v),
        v.map(|v| v.phase),
        v.map(|v| v.hand_no),
        v.map(|v| v.turn_seat),
        v.map(|v| v.you),
        v.map(|v| v.legal.iter().map(|m| m.id.clone()).collect::<Vec<_>>()),
        v.and_then(|v| v.awaiting_trick_review),
        t.present.collect,
    )
}

/// Whether the player has something to do: a move, a reviewed trick, or the match is over.
fn actionable(s: &AppState) -> bool {
    let Some(table) = s.table.as_ref() else {
        return false;
    };
    if matches!(table.stage, TableStage::Failed(_)) {
        return true;
    }
    if table.store.submitting() || table.store.review_busy() || !table.store.is_connected() {
        return false;
    }
    let Some(v) = table.store.display_view() else {
        return false;
    };
    v.status != GameStatus::Active
        || (v.turn_seat == Some(v.you) && !v.legal.is_empty())
        || (v.awaiting_trick_review == Some(true)
            && table.present.collect.as_ref().is_none_or(|c| !c.continuing))
}

fn controls(s: &AppState) -> bool {
    s.table
        .as_ref()
        .is_some_and(|t| t.store.controls_enabled() && t.store.is_connected())
}

fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(200),
    )
    .is_ok()
}

fn http_port(url: &str) -> Option<u16> {
    url.rsplit(':')
        .next()
        .and_then(|p| p.trim_end_matches('/').parse().ok())
}

/// Waits until none of `ports` answers and none of `pids` lives.
fn all_closed(pids: &[u32], ports: &[u16]) -> Duration {
    let started = Instant::now();
    while pids.iter().any(|&p| alive(p)) || ports.iter().any(|&p| listening(p)) {
        assert!(
            started.elapsed() < PORTS_CLOSE,
            "alive: {:?}; listening: {:?}",
            pids.iter().filter(|&&p| alive(p)).collect::<Vec<_>>(),
            ports.iter().filter(|&&p| listening(p)).collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    started.elapsed()
}

fn ms(d: Duration) -> u128 {
    d.as_millis()
}

/// The `q` quantile (nearest rank) of `samples`, in ms.
fn quantile(samples: &[Duration], q: f64) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    let mut sorted: Vec<u128> = samples.iter().map(|d| d.as_millis()).collect();
    sorted.sort_unstable();
    let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

/// A made-up move from a second client of the same device (its own HTTP pool, its own
/// runtime): the server refuses it with `illegal_move` and the turn's legal set.
fn refused_by_the_server(url: &str, data: &Path, game_id: &str, at: &PublicGameView) -> Duration {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a runtime");
    let backend =
        RemoteBackend::new(RemoteConfig::embedded(url.to_owned(), data)).expect("a second client");
    let took = runtime.block_on(async {
        let mut conn = tokio::time::timeout(TURN, backend.open_table(game_id))
            .await
            .expect("opened in time")
            .expect("the table opens on a second client");
        // Its first sync, at the app's turn.
        let seen = tokio::time::timeout(TURN, async {
            while let Some(event) = conn.events.recv().await {
                if let ConnEvent::Event(TableEvent::Sync(sync)) = event
                    && sync.v >= at.v
                {
                    return Some(sync.view);
                }
            }
            None
        })
        .await
        .expect("a sync in time")
        .expect("the second client's sync");
        assert_eq!(
            seen.turn_id, at.turn_id,
            "the second client sees the same turn"
        );
        let started = Instant::now();
        let ack = tokio::time::timeout(
            TURN,
            conn.calls.submit_move(SubmitMoveRequest {
                move_id: "not-a-real-move".to_owned(),
                turn_id: Some(at.turn_id.clone()),
                client_move_id: uuid::Uuid::new_v4().to_string(),
            }),
        )
        .await
        .expect("an ack in time")
        .expect("the call answered");
        let took = started.elapsed();
        match ack {
            MoveAck::Rejected { code, legal } => {
                assert_eq!(code.as_str(), "illegal_move");
                assert!(!legal.is_empty(), "the rejection carries the legal set");
                assert_eq!(legal, at.legal, "the rejection's legal set is the turn's");
            }
            MoveAck::Ok { .. } => panic!("a made-up move landed"),
        }
        conn.calls.close().await;
        took
    });
    runtime.shutdown_timeout(Duration::from_secs(2));
    took
}

/// The run's table moments, as `(conn_id, sync)` pairs in arrival order on one connection:
/// `(before, moment)`.
fn moments(syncs: &[(u64, SyncEvent)]) -> Vec<(&SyncEvent, &SyncEvent)> {
    syncs
        .windows(2)
        .filter(|w| w[0].0 == w[1].0)
        .map(|w| (&w[0].1, &w[1].1))
        .collect()
}

fn my_turn(v: &PublicGameView) -> bool {
    v.status == GameStatus::Active && v.turn_seat == Some(v.you) && !v.legal.is_empty()
}

/// The fixtures for the gallery's live scenes: name, fixture, scene, `--at` seconds.
fn fixtures(
    run: &Run,
    stopped: Option<&SyncEvent>,
) -> Vec<(&'static str, LiveFixture, &'static str, f64)> {
    let all = moments(&run.syncs);
    let table = |before: &SyncEvent, moment: &SyncEvent| LiveFixture {
        syncs: vec![before.clone(), moment.clone()],
        ..LiveFixture::default()
    };
    let has = |s: &SyncEvent, f: fn(&Step) -> bool| s.steps.iter().any(f);
    let mut out = Vec::new();
    let pick =
        |f: &dyn Fn(&SyncEvent, &SyncEvent) -> bool| all.iter().copied().find(|(b, m)| f(b, m));
    if let Some((b, m)) = pick(&|_, m| {
        matches!(m.view.phase, GamePhase::BidRound1 | GamePhase::BidRound2)
            && my_turn(&m.view)
            && !m.steps.is_empty()
    }) {
        out.push(("bid", table(b, m), "live-table", 2.5));
    }
    if let Some((b, m)) = pick(&|_, m| {
        m.view.phase == GamePhase::TrickPlay && my_turn(&m.view) && m.view.trick.plays.len() >= 2
    }) {
        out.push(("play", table(b, m), "live-table", 2.5));
    }
    if let Some((b, m)) = pick(&|_, m| {
        has(m, |s| matches!(s, Step::TrickWon { .. }))
            && !has(m, |s| matches!(s, Step::HandScored { .. }))
            && m.view.review_tricks != Some(true)
            && m.view.awaiting_trick_review != Some(true)
    }) {
        out.push(("trick-won", table(b, m), "live-table", 1.2));
    }
    if let Some((b, m)) = pick(&|_, m| m.view.awaiting_trick_review == Some(true)) {
        out.push(("trick-review", table(b, m), "live-table", 3.0));
    }
    if let Some((b, m)) = pick(&|_, m| {
        has(m, |s| matches!(s, Step::HandScored { .. })) && m.view.status == GameStatus::Active
    }) {
        out.push(("hand-result", table(b, m), "live-table", 5.0));
    }
    if let Some((b, m)) =
        pick(&|b, m| b.view.status == GameStatus::Active && m.view.status != GameStatus::Active)
    {
        out.push(("game-over", table(b, m), "live-table", 4.0));
    }
    if run.games.is_some() {
        out.push((
            "games",
            LiveFixture {
                games: run.games.clone(),
                recent: run.recent.clone(),
                stats: run.stats.clone(),
                ..LiveFixture::default()
            },
            "live-games",
            1.0,
        ));
    }
    if run.replay.is_some() {
        out.push((
            "match",
            LiveFixture {
                replay: run.replay.clone(),
                ..LiveFixture::default()
            },
            "live-match",
            1.0,
        ));
    }
    if let Some(sync) = stopped {
        out.push((
            "stopped",
            LiveFixture {
                syncs: vec![sync.clone()],
                ..LiveFixture::default()
            },
            "live-stopped",
            1.0,
        ));
    }
    out
}

/// Writes each fixture and renders its scene to `<root>/snaps/live-<name>.png` at 1280x800.
fn render(
    root: &Path,
    fixtures: &[(&'static str, LiveFixture, &'static str, f64)],
) -> Vec<PathBuf> {
    let (dir, snaps, scene_home) = (
        root.join("fixtures"),
        root.join("snaps"),
        root.join("scene"),
    );
    for d in [&dir, &snaps, &scene_home] {
        std::fs::create_dir_all(d).expect("a fixture dir");
    }
    let mut out = Vec::new();
    for (name, fixture, scene, at) in fixtures {
        let path = dir.join(format!("{name}.json"));
        std::fs::write(&path, serde_json::to_vec_pretty(fixture).expect("json")).expect("fixture");
        let png = snaps.join(format!("live-{name}.png"));
        let status = std::process::Command::new(env!("CARGO_BIN_EXE_euchre"))
            .args(["--scene", scene, "--snap"])
            .arg(&png)
            .args([
                "--size",
                "1280x800",
                "--scale",
                "1",
                "--at",
                &at.to_string(),
            ])
            .env("EUCHRE_LIVE_FIXTURE", &path)
            .env("EUCHRE_DESKTOP_BACKEND", "sim")
            .env("EUCHRE_DATA_DIR", &scene_home)
            .env("EUCHRE_PREFS_DIR", &scene_home)
            .env("NO_PROXY", "127.0.0.1,localhost,::1")
            .env("no_proxy", "127.0.0.1,localhost,::1")
            .stdout(std::process::Stdio::null())
            .status()
            .expect("the scene runs");
        assert!(status.success(), "{scene} ({name}) rendered");
        let size = std::fs::metadata(&png).map(|m| m.len()).unwrap_or(0);
        assert!(size > 10_000, "{name}: a real picture ({size} bytes)");
        out.push(png);
    }
    out
}

#[test]
fn live_match_through_the_app() {
    if !e2e_enabled() {
        e2e_skip("live_match_through_the_app");
        return;
    }
    let wall = Instant::now();
    // The app's milestones (boot_ms, stop_ms) and warnings, on stderr.
    euchre_desktop::startup::init_logging(None);
    let port: u16 = std::env::var("EUCHRE_TEST_ENGINE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(6870);
    let base = std::env::var_os("EUCHRE_TEST_TMP")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = base.join(format!("integration-{port}"));
    reclaim(&root);
    let data = root.join("data");
    std::fs::create_dir_all(&data).expect("data dir");
    let keep = std::env::var("EUCHRE_LIVE_KEEP").as_deref() == Ok("1");
    println!("live root: {} (kept: {keep})", root.display());
    let engine_ports = [port, port + 1, port + 5];
    for p in engine_ports {
        assert!(!listening(p), "port {p} is free before the run");
    }

    // 1. The app starts; the server boots behind Home. Play a game at once: the table says it
    //    is getting ready, then deals.
    let mut run = Run::start(&data, port);
    let booted = Instant::now();
    run.click("hero-play");
    run.settle(motion::ROUTE_SETTLE);
    let created = Instant::now();
    run.click("new-submit");
    assert_eq!(run.rt.state.route, Route::Table);
    run.pump(Duration::from_millis(50), |_| false);
    let booting_seen = matches!(
        run.rt.state.local_server,
        Some(ServerPhase::Starting | ServerPhase::Downloading)
    );
    if booting_seen {
        assert!(
            run.texts().iter().any(|t| t == BOOTING_LINE),
            "the booting line shows while the server starts: {:?}",
            run.texts()
        );
    }
    run.must("the server is ready", BOOT, |s| {
        s.local_server == Some(ServerPhase::Ready)
    });
    let boot_cold = booted.elapsed();
    run.must("the player is known", BOOT, |s| s.identity.is_some());
    run.must("the table dealt to the player", BOOT, actionable);
    let first_turn = created.elapsed();
    let game_id = run
        .rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.game_id.clone())
        .expect("a game id");
    let url = run.url();
    println!(
        "ready at {url} (engine {}) in {} ms; booting line shown: {booting_seen}; \
         Create to the first turn {} ms; game {game_id}",
        run.engine_pid(),
        ms(boot_cold),
        ms(first_turn)
    );
    run.settle(motion::ROUTE_SETTLE);

    // 2. Play the match. The player's first trick-play turn (hand 0 unless the player sat it out
    //    or it was thrown in): a made-up move is refused, then the next trick is reviewed.
    //    Hand 1: the socket drops mid-hand. Hand 2: the engine stops and Restart serves.
    let mut moves = 0usize;
    let mut landed = Vec::new();
    let mut refused: Option<Duration> = None;
    let mut review: Option<(bool, Duration)> = None; // (asked, hold)
    let mut reconnect: Option<Duration> = None;
    let mut stop: Option<(Duration, Duration, Duration)> = None; // noticed, ready, resumed
    let mut stopped_sync: Option<SyncEvent> = None;
    let started = Instant::now();
    loop {
        assert!(
            started.elapsed() < MATCH,
            "the match finished in time ({moves} moves)"
        );
        run.must(
            &format!("the table comes back to the player ({moves} moves)"),
            TURN,
            actionable,
        );
        if let Some(t) = run.rt.state.table.as_ref()
            && let TableStage::Failed(err) = &t.stage
        {
            panic!("the table ended: {err:?}\n{}", describe(&run.rt.state));
        }
        let v = run.view();
        if v.status != GameStatus::Active {
            break;
        }

        // The review gate holds a finished trick until Continue.
        if v.awaiting_trick_review == Some(true) && v.legal.is_empty() {
            let (turn, at_v) = (v.turn_id.clone(), v.v);
            let hold = Instant::now();
            run.pump(REVIEW_HOLD, |_| false);
            let still = run.view();
            assert_eq!(
                (still.v, &still.turn_id),
                (at_v, &turn),
                "the gate holds the trick"
            );
            assert!(
                run.rt.find("trick-continue").is_some(),
                "Continue is offered"
            );
            let held = hold.elapsed();
            run.click("trick-continue");
            run.must("Continue moves the table on", TURN, |s| {
                view(s).is_some_and(|w| w.turn_id != turn)
            });
            // Once is the proof; the rest of the match plays at speed. The store refuses a
            // review change while the Continue is still in flight, so wait for its reply.
            run.must("the Continue settles", TURN, |s| {
                s.table.as_ref().is_some_and(|t| !t.store.review_busy())
            });
            run.rt.update(|s, _| s.set_review_tricks(false));
            run.must("review turns off at the server", TURN, |s| {
                view(s).is_some_and(|w| w.review_tricks == Some(false))
            });
            review = Some((true, held));
            continue;
        }

        if refused.is_none() && v.phase == GamePhase::TrickPlay && !v.legal.is_empty() {
            refused = Some(refused_by_the_server(&url, &data, &game_id, &v));
            run.pump(Duration::from_millis(300), |_| false);
            let after = run.view();
            assert_eq!(
                (after.v, &after.turn_id),
                (v.v, &v.turn_id),
                "the refused move changed nothing at the app's table"
            );
            assert!(controls(&run.rt.state), "the player's controls stay");
            // Review the next finished trick. Wait for the server to take it, so a stop or a
            // drop right after cannot lose the change.
            run.rt.update(|s, _| s.set_review_tricks(true));
            run.must("review turns on at the server", TURN, |s| {
                view(s).is_some_and(|w| w.review_tricks == Some(true))
            });
            review = Some((true, Duration::ZERO));
            continue;
        }

        if v.hand_no == 1 && reconnect.is_none() {
            let dropped = Instant::now();
            run.backend
                .remote()
                .expect("a remote backend while ready")
                .interrupt_tables();
            run.must("Reconnecting shows", Duration::from_secs(5), |s| {
                s.table
                    .as_ref()
                    .is_some_and(|t| !t.store.is_connected() && !t.store.controls_enabled())
            });
            run.settle(0.1);
            assert!(
                run.rt.find("table-reconnecting").is_some(),
                "the Reconnecting banner shows"
            );
            assert!(view(&run.rt.state).is_some(), "the table stays on screen");
            let (turn, hand) = (v.turn_id.clone(), v.hand_no);
            run.must("the table reconnects to the same turn", TURN, |s| {
                controls(s) && view(s).is_some_and(|w| w.turn_id == turn && w.hand_no == hand)
            });
            run.settle(0.4);
            assert!(
                run.rt.find("table-reconnecting").is_none(),
                "the banner leaves"
            );
            reconnect = Some(dropped.elapsed());
            continue;
        }

        if v.hand_no == 2 && stop.is_none() {
            let pid = run.engine_pid();
            assert!(
                local::is_engine_of(pid, &run.storage),
                "{pid} is our engine"
            );
            let stopped_at = Instant::now();
            let killed = std::process::Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status()
                .is_ok_and(|s| s.success());
            assert!(killed, "stopped engine {pid}");
            run.must("the table ends with Restart", STOP_NOTICED, |s| {
                s.table
                    .as_ref()
                    .is_some_and(|t| matches!(t.stage, TableStage::Failed(NetError::ServerStopped)))
            });
            let noticed = stopped_at.elapsed();
            run.settle(0.6);
            assert!(
                run.rt.find("table-ended").is_some(),
                "the ended banner shows"
            );
            assert!(view(&run.rt.state).is_some(), "the last view stays");
            assert!(!controls(&run.rt.state), "the controls are off");
            let last = run.view();
            stopped_sync = Some(SyncEvent {
                v: last.v,
                view: last,
                steps: Vec::new(),
            });
            let restart = Instant::now();
            run.click("table-ended-action");
            run.must("the server is back", BOOT, |s| {
                s.local_server == Some(ServerPhase::Ready)
            });
            let ready = restart.elapsed();
            let (turn, hand) = (v.turn_id.clone(), v.hand_no);
            run.must("the table resumes at the same turn", TURN, |s| {
                controls(s) && view(s).is_some_and(|w| w.turn_id == turn && w.hand_no == hand)
            });
            let resumed = restart.elapsed();
            assert_ne!(run.engine_pid(), pid, "a new engine serves");
            println!(
                "engine stop noticed in {} ms; Restart ready in {} ms; table back in {} ms",
                ms(noticed),
                ms(ready),
                ms(resumed)
            );
            stop = Some((noticed, ready, resumed));
            run.settle(motion::ROUTE_SETTLE);
            continue;
        }

        landed.push(run.play_top_move());
        moves += 1;
    }
    let match_time = started.elapsed();
    let v = run.view();
    assert_eq!(v.status, GameStatus::Complete);
    assert!(
        v.score.iter().any(|&p| p >= 10),
        "someone reached ten: {:?}",
        v.score
    );
    let we_won = v.score[0] > v.score[1];
    assert_eq!(
        v.winner_team.map(|t| t == euchre_engine::Team::NS),
        Some(we_won),
        "the winner is the team with the score"
    );
    assert!(moves > 15, "a match is many moves: {moves}");
    assert!(refused.is_some(), "the made-up move was tried");
    assert!(
        review.is_some_and(|(asked, held)| asked && held >= REVIEW_HOLD),
        "a trick was reviewed"
    );
    assert!(reconnect.is_some(), "the socket dropped mid-hand");
    assert!(stop.is_some(), "the engine stopped mid-match");
    assert!(
        run.odd_acks.is_empty(),
        "every move acked Ok: {:?}",
        run.odd_acks
    );
    // The match-end panel comes up on its own beat: wait for it (bounded) rather than a fixed
    // pause, and say how long it took (review fix PX-3).
    let ending = Instant::now();
    let mut panel = None;
    while ending.elapsed() < Duration::from_secs(10) {
        run.settle(0.1);
        if run.rt.find("match-end-panel").is_some() {
            panel = Some(ending.elapsed());
            break;
        }
    }
    let panel_time = panel.unwrap_or_else(|| {
        panic!(
            "the match-end panel shows within 10 s\n{}",
            describe(&run.rt.state)
        )
    });
    println!(
        "match-end panel up {} ms after the last move",
        ms(panel_time)
    );
    assert!(run.rt.find("hand-play").is_none());

    // 3. Your games: the finished match is listed and counted, with its saved table; it opens to
    //    its hands.
    let listed = Instant::now();
    run.rt.update(|s, tx| s.nav_to(Route::Games, tx));
    let id = game_id.clone();
    let mut asked = Instant::now();
    loop {
        let id = id.clone();
        if run.pump(Duration::from_millis(250), move |s| {
            s.games
                .page
                .as_ref()
                .is_some_and(|p| p.rows.iter().any(|m| m.match_id == id))
                && s.games.stats.as_ref().is_some_and(|st| st.matches >= 1)
        }) {
            break;
        }
        assert!(
            listed.elapsed() < Duration::from_secs(30),
            "the match is listed and counted: {:?} rows, stats {:?}",
            run.rt.state.games.page.as_ref().map(|p| p.rows.len()),
            run.rt.state.games.stats
        );
        if asked.elapsed() >= Duration::from_millis(500) {
            run.bridge.send(Intent::ListGames { cursor: None });
            run.bridge.send(Intent::RecentTables);
            run.bridge.send(Intent::GetStats);
            asked = Instant::now();
        }
    }
    let gid = game_id.clone();
    run.must("the saved table is listed", TURN, move |s| {
        s.recent_tables
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .is_some_and(|r| r.tables.iter().any(|t| t.game_id == gid))
    });
    let list_time = listed.elapsed();
    run.settle(1.0);
    let row = format!("game-{game_id}");
    assert!(run.rt.find(&row).is_some(), "{row} shows");
    assert!(run.rt.find("games-stats").is_some(), "the stats show");
    let saved = run
        .rt
        .state
        .recent_tables
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .and_then(|r| r.tables.iter().position(|t| t.game_id == game_id))
        .expect("the saved table");
    let saved_key = format!("saved-table-{saved}");
    let saved_label = run
        .rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(saved_key.as_str()))
        .and_then(|e| e.label.or(e.text))
        .unwrap_or_default();
    // The row is the table's live snapshot: after the Restart it must be the match the
    // player finished, not a copy of the table from before the engine stopped.
    let final_score = format!("{}–{}", v.score[0], v.score[1]);
    if !(saved_label.starts_with("Open game") && saved_label.contains(&final_score)) {
        // Say whether a fresh read from the server agrees before failing: a stale row with a
        // fresh read that is right is the app's own cache (`AppState::note_open_table`).
        let gid = game_id.clone();
        let asked = Instant::now();
        let mut healed = None;
        while asked.elapsed() < Duration::from_secs(30) {
            run.bridge.send(Intent::RecentTables);
            let gid = gid.clone();
            if run.pump(Duration::from_secs(1), move |s| {
                s.recent_tables
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .and_then(|r| r.tables.iter().find(|t| t.game_id == gid))
                    .is_some_and(|t| t.status == Some(GameStatus::Complete))
            }) {
                healed = Some(asked.elapsed());
                break;
            }
        }
        panic!(
            "the finished table reads Open game at {final_score}: {saved_label:?} (a fresh read \
             from the server {})",
            healed.map_or_else(
                || "stayed stale for 30 s".to_owned(),
                |t| format!("read complete after {} ms", ms(t))
            )
        );
    }
    run.click(&row);
    assert_eq!(run.rt.state.route, Route::MatchDetail(game_id.clone()));
    let opened = Instant::now();
    run.must("the match detail loads", TURN, |s| {
        s.match_detail.as_ref().is_some_and(|(_, r)| r.is_ok())
    });
    let detail_time = opened.elapsed();
    run.settle(1.0);
    assert!(run.rt.find("match-totals").is_some(), "the totals show");
    assert!(run.rt.find("match-hand-1").is_some(), "the hands list");
    let hands = run
        .replay
        .as_ref()
        .map(|r| r.hands.len())
        .expect("the replay");
    assert!(hands >= 3, "a match to ten is three hands or more: {hands}");
    // Every deal should have its row. A Restart mid-hand can drop the hand it cut today: the
    // table's `recordHand` to the profile is not retried once the engine is back
    // (crates/euchre-table; docs/port/requests/desktop-net-stale-registry.md). So the only
    // hand allowed missing is the one the engine stopped in (review fix PX-2); any other gap
    // fails.
    let deals = i64::from(v.hand_no) + 1;
    let missing_hands: Vec<i64> = (0..deals)
        .filter(|n| {
            run.replay
                .as_ref()
                .is_some_and(|r| !r.hands.iter().any(|h| h.hand_no == *n))
        })
        .collect();
    let stopped_hand = stopped_sync.as_ref().map(|s| i64::from(s.view.hand_no));
    if !missing_hands.is_empty() {
        println!(
            "NOTE: the match history is missing hands {missing_hands:?} of {deals} (the engine \
             stopped in hand {stopped_hand:?})"
        );
    }
    assert!(
        missing_hands.iter().all(|n| Some(*n) == stopped_hand),
        "only the hand the Restart cut may be missing: {missing_hands:?} (stopped in \
         {stopped_hand:?})"
    );

    // 4. Quit: the bridge ends, every engine is gone and every port closes.
    let quit = run.quit();
    let mut ports = engine_ports.to_vec();
    ports.extend(run.http_ports.iter().copied());
    let closed = all_closed(&run.pids, &ports);
    println!(
        "quit in {} ms (+{} ms until every port closed: {ports:?})",
        ms(quit),
        ms(closed)
    );

    // 5. The gallery's inputs: the real syncs as fixtures, rendered by the live scenes.
    let fixtures = fixtures(&run, stopped_sync.as_ref());
    let names: Vec<&str> = fixtures.iter().map(|f| f.0).collect();
    for want in [
        "bid",
        "play",
        "trick-won",
        "hand-result",
        "game-over",
        "games",
        "match",
        "stopped",
    ] {
        assert!(
            names.contains(&want),
            "a {want} moment was captured: {names:?}"
        );
    }
    let rendered = Instant::now();
    let pngs = render(&root, &fixtures);
    let render_time = rendered.elapsed();

    let (noticed, ready, resumed) = stop.unwrap_or_default();
    let timings = serde_json::json!({
        "wall_ms": ms(wall.elapsed()),
        "boot_cold_ms": ms(boot_cold),
        "booting_line_shown": booting_seen,
        "create_to_first_turn_ms": ms(first_turn),
        "match_ms": ms(match_time),
        "moves": moves,
        "move_ack_p50_ms": quantile(&run.acks, 0.5),
        "move_ack_p95_ms": quantile(&run.acks, 0.95),
        "move_ack_max_ms": quantile(&run.acks, 1.0),
        "move_landed_p50_ms": quantile(&landed, 0.5),
        "move_landed_p95_ms": quantile(&landed, 0.95),
        "refused_ack_ms": refused.map(ms),
        "review_hold_ms": review.map(|(_, h)| ms(h)),
        "reconnect_ms": reconnect.map(ms),
        "engine_stop_noticed_ms": ms(noticed),
        "restart_ready_ms": ms(ready),
        "restart_table_back_ms": ms(resumed),
        "match_end_panel_ms": ms(panel_time),
        "games_listed_ms": ms(list_time),
        "match_detail_ms": ms(detail_time),
        "quit_ms": ms(quit),
        "ports_closed_ms": ms(closed),
        "render_ms": ms(render_time),
        "hands": hands,
        "deals": deals,
        "missing_hands": missing_hands,
        "stats_matches": run.stats.as_ref().map(|s| s.matches),
        "score": v.score,
        "we_won": we_won,
        "syncs": run.syncs.len(),
        "pngs": pngs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    });
    std::fs::write(
        root.join("timings.json"),
        serde_json::to_vec_pretty(&timings).expect("json"),
    )
    .expect("timings");
    println!("LIVE {timings}");
    if !keep {
        // A pass leaves nothing behind (review fix LDS-5); EUCHRE_LIVE_KEEP=1 keeps the
        // fixtures and snaps for the gallery.
        drop(run);
        let _ = std::fs::remove_dir_all(&root);
    }
}
