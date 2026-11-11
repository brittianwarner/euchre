//! The embedded game server on screen (contract §N2.5, §N2.10): booting, a server that stopped
//! or could not start, a table that ended, the saved tables, and quitting. Headless, with a
//! bridge whose intents the tests read; no engine and no network.

use euchre_desktop::app::{AppState, Route, TableStage};
use euchre_desktop::net::bridge::{Bridge, Intent, Reply};
use euchre_desktop::net::{
    BootProblem, ConnEvent, NetError, RecentTable, RecentTables, ServerPhase,
};
use euchre_desktop::prefs::Prefs;
use euchre_desktop::screens::{other, server};
use euchre_desktop::{app, scenes, spec, theme};
use euchre_protocol::{ChatEvent, ChatKind, Seat, TableEvent, True};
use rune::{App, Env, Runtime, Shell};
use tokio::sync::mpsc::UnboundedReceiver;

const DT: f64 = 1.0 / 60.0;

/// The game id the server scenes use.
const SCENE_GAME: &str = "g7k2m9q4x1c8v5b3n6h0";

/// A runtime on `state` after `scene` (if any), with a bridge whose intents come back.
fn runtime_with(
    scene: Option<&str>,
    edit: impl FnOnce(&mut AppState),
) -> (Runtime<AppState>, UnboundedReceiver<Intent>) {
    runtime_sized(theme::window::SIZE, scene, edit)
}

/// [`runtime_with`] in a window of `size`.
fn runtime_sized(
    size: (f32, f32),
    scene: Option<&str>,
    edit: impl FnOnce(&mut AppState),
) -> (Runtime<AppState>, UnboundedReceiver<Intent>) {
    let (bridge, intents) = Bridge::channel();
    let prefs = Prefs {
        tour_seen: true,
        ..Prefs::default()
    };
    let mut state = AppState::new(prefs, None, "embedded");
    if let Some(name) = scene {
        let (_, apply) = scenes::SCENES
            .iter()
            .find(|(n, _)| *n == name)
            .expect("scene exists");
        apply(&mut state);
    }
    // Some fixtures detach the bridge; these tests read what the screens send.
    state.bridge = Some(bridge);
    state.wait_for_art = true;
    edit(&mut state);
    let mut rt = App::new(spec())
        .init(move || state)
        .headless_runtime(size, Env::new(false, 1.0));
    rt.frame(0.0);
    settle(&mut rt, 1.0);
    (rt, intents)
}

fn settle(rt: &mut Runtime<AppState>, seconds: f64) {
    for _ in 0..(seconds / DT).ceil() as usize {
        rt.frame(DT);
    }
}

fn click(rt: &mut Runtime<AppState>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(DT);
}

/// Scrolls the page until `key` is inside the window, then clicks it.
fn reveal_click(rt: &mut Runtime<AppState>, key: &str) {
    let (_, h) = theme::window::SIZE;
    for _ in 0..60 {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is drawn"));
        if (60.0..h - 60.0).contains(&y) {
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(DT);
            return;
        }
        let dy = if y >= h - 60.0 { -240.0 } else { 240.0 };
        rt.wheel(40.0, h / 2.0, dy);
        settle(rt, 0.3);
    }
    panic!("{key} never scrolled into view");
}

fn receive(rt: &mut Runtime<AppState>, reply: Reply) {
    rt.update(move |s, tx| s.receive(reply, tx));
    rt.frame(DT);
}

fn drain(intents: &mut UnboundedReceiver<Intent>) -> Vec<Intent> {
    let mut out = Vec::new();
    while let Ok(intent) = intents.try_recv() {
        out.push(intent);
    }
    out
}

/// Every text on screen (not leaving).
fn texts(rt: &Runtime<AppState>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.text)
        .collect()
}

fn shows(rt: &Runtime<AppState>, line: &str) -> bool {
    texts(rt).iter().any(|t| t == line)
}

fn label(rt: &Runtime<AppState>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .and_then(|e| e.label)
}

#[test]
fn starting_shows_a_seat_at_the_table() {
    let (rt, _intents) = runtime_with(Some("server-starting"), |_| {});
    assert!(shows(&rt, "A seat at the table."), "{:?}", texts(&rt));
    assert!(shows(&rt, server::BOOTING_LINE));
    // Nothing to press while it boots: the table opens by itself.
    assert!(rt.find("loading-reconnect").is_none());
    assert!(rt.find("loading-error").is_none());
    assert!(rt.find("loading-games").is_some());
    assert!(
        rt.find("server-notice").is_none(),
        "no notice while booting"
    );
}

#[test]
fn downloading_says_the_first_start_takes_a_minute() {
    let (rt, _intents) = runtime_with(Some("server-downloading"), |_| {});
    assert!(shows(&rt, "A seat at the table."));
    assert!(shows(
        &rt,
        "Getting the table ready. The first start takes a minute."
    ));
    assert!(rt.find("loading-reconnect").is_none());
}

#[test]
fn stopped_shows_the_notice_and_restart_sends_the_intent() {
    for route in [
        Route::Home,
        Route::NewGame,
        Route::Games,
        Route::Settings,
        Route::MatchDetail("m1".into()),
    ] {
        let (rt, _intents) = runtime_with(None, |s| {
            s.route = route.clone();
            s.on_server_phase(ServerPhase::Stopped);
        });
        assert!(rt.find("server-notice").is_some(), "{route:?}");
        assert!(shows(&rt, "The game server stopped."), "{route:?}");
        assert!(shows(&rt, "Restart to keep playing. Your games are saved."));
        assert_eq!(
            label(&rt, "server-notice-action").as_deref(),
            Some("Restart")
        );
    }
    // The table has no nav and says it in its own banner instead.
    let (rt, _intents) = runtime_with(Some("server-stopped-table"), |_| {});
    assert!(rt.find("server-notice").is_none());
    assert!(rt.find("table-ended").is_some());

    let (mut rt, mut intents) = runtime_with(Some("server-stopped"), |_| {});
    drain(&mut intents);
    click(&mut rt, "server-notice-action");
    let sent = drain(&mut intents);
    assert!(
        matches!(sent.as_slice(), [Intent::RestartServer]),
        "{sent:?}"
    );
    // The supervisor answers Starting at once: the notice leaves, Home stays.
    receive(&mut rt, Reply::Server(ServerPhase::Starting));
    settle(&mut rt, 1.0);
    assert!(rt.find("server-notice").is_none());
    assert_eq!(rt.state.route, Route::Home);
}

#[test]
fn ready_after_stopped_reconnects_the_open_table_and_reloads_the_route() {
    // An open table on screen when the server stopped.
    let (mut rt, mut intents) = runtime_with(Some("server-stopped-table"), |s| {
        s.server_lost = true;
        if let Some(t) = s.table.as_mut() {
            t.game_id = Some(SCENE_GAME.into());
        }
    });
    let old_conn = rt.state.table.as_ref().map(|t| t.conn_id).expect("a table");
    drain(&mut intents);
    receive(&mut rt, Reply::Server(ServerPhase::Starting));
    assert!(drain(&mut intents).is_empty(), "nothing until it is ready");
    receive(&mut rt, Reply::Server(ServerPhase::Ready));
    let sent = drain(&mut intents);
    assert!(matches!(sent.first(), Some(Intent::Identify)), "{sent:?}");
    assert!(sent.iter().any(|i| matches!(i, Intent::RecentTables)));
    assert!(
        sent.iter()
            .any(|i| matches!(i, Intent::CloseTable { conn_id } if *conn_id == old_conn))
    );
    let reopened = sent.iter().find_map(|i| match i {
        Intent::OpenTable { conn_id, game_id } => Some((*conn_id, game_id.clone())),
        _ => None,
    });
    let (conn, game) = reopened.expect("the table reopens");
    assert_ne!(conn, old_conn, "on a new connection");
    assert_eq!(game, SCENE_GAME, "the same table");
    let table = rt.state.table.as_ref().expect("still a table");
    assert_eq!(table.stage, TableStage::Opening);
    assert_eq!(table.game_id.as_deref(), Some(SCENE_GAME));
    // The felt keeps its last view while the new connection resyncs; it never blanks to
    // the loading page (review fix PX-4).
    assert!(table.store.display_view().is_some(), "the view stays");
    settle(&mut rt, 0.5);
    assert!(rt.find("loading-games").is_none(), "{:?}", texts(&rt));
    assert!(rt.find("loading-reconnect").is_none());
    // A second Ready (no stop between) does nothing.
    receive(&mut rt, Reply::Server(ServerPhase::Ready));
    assert!(drain(&mut intents).is_empty());

    // Your games reloads its page, stats and saved tables.
    let (mut rt, mut intents) = runtime_with(Some("games"), |s| {
        s.on_server_phase(ServerPhase::Failed(BootProblem::TooSlow));
    });
    drain(&mut intents);
    receive(&mut rt, Reply::Server(ServerPhase::Starting));
    receive(&mut rt, Reply::Server(ServerPhase::Ready));
    let sent = drain(&mut intents);
    assert!(sent.iter().any(|i| matches!(i, Intent::Identify)));
    assert!(
        sent.iter()
            .any(|i| matches!(i, Intent::ListGames { cursor: None }))
    );
    assert!(sent.iter().any(|i| matches!(i, Intent::GetStats)));
    assert_eq!(
        sent.iter()
            .filter(|i| matches!(i, Intent::RecentTables))
            .count(),
        1,
        "{sent:?}"
    );
    assert!(rt.state.games.loading);

    // Settings reads its settings and the stats again.
    let (mut rt, mut intents) = runtime_with(None, |s| {
        s.route = Route::Settings;
        s.on_server_phase(ServerPhase::Stopped);
    });
    drain(&mut intents);
    receive(&mut rt, Reply::Server(ServerPhase::Starting));
    receive(&mut rt, Reply::Server(ServerPhase::Ready));
    let sent = drain(&mut intents);
    assert!(sent.iter().any(|i| matches!(i, Intent::GetSettings)));
    assert!(sent.iter().any(|i| matches!(i, Intent::GetStats)));
}

#[test]
fn failed_copy_is_plain_for_every_problem() {
    let problems = [
        BootProblem::AlreadyOpen,
        BootProblem::PortInUse { port: 6420 },
        BootProblem::LowDisk {
            free_bytes: 2 << 30,
            need_bytes: 4 << 30,
        },
        BootProblem::EngineMissing,
        BootProblem::DownloadStalled,
        BootProblem::TooSlow,
        BootProblem::DataConflict,
        BootProblem::Relaunch,
        BootProblem::Other,
    ];
    for problem in problems {
        let copy = server::problem_copy(&problem);
        let (rt, _intents) = runtime_with(None, |s| {
            s.route = Route::Home;
            s.on_server_phase(ServerPhase::Failed(problem.clone()));
        });
        assert!(rt.find("server-notice").is_some(), "{problem:?}");
        assert!(shows(&rt, copy.heading), "{problem:?}: {:?}", texts(&rt));
        assert!(shows(&rt, copy.line), "{problem:?}");
        assert_eq!(
            label(&rt, "server-notice-action").as_deref(),
            Some(copy.action.label())
        );
        for t in texts(&rt) {
            for word in ["error", "Error", "BootError", "rivet", "6420"] {
                assert!(!t.contains(word), "{problem:?} shows {t:?}");
            }
        }
    }
    // The headings the contract names.
    let heading = |p: BootProblem| server::problem_copy(&p).heading;
    // Another copy of Euchre and another program on the port are told apart (review fix
    // LDS-3).
    assert_eq!(heading(BootProblem::AlreadyOpen), "Euchre is already open.");
    assert_eq!(
        heading(BootProblem::PortInUse { port: 1 }),
        "Euchre can’t start its game server."
    );
    assert_eq!(
        heading(BootProblem::TooSlow),
        "The game server is taking too long to start."
    );
    assert_eq!(heading(BootProblem::Other), "The game server didn’t start.");
}

#[test]
fn relaunch_button_relaunches() {
    let (mut rt, mut intents) = runtime_with(Some("server-relaunch"), |_| {});
    assert_eq!(
        label(&rt, "server-notice-action").as_deref(),
        Some("Reopen Euchre")
    );
    drain(&mut intents);
    click(&mut rt, "server-notice-action");
    assert_eq!(rt.shell_request, Some(Shell::Relaunch));
    assert!(drain(&mut intents).is_empty(), "no restart: a new process");

    // Try again restarts in place.
    let (mut rt, mut intents) = runtime_with(Some("server-failed-port"), |_| {});
    assert_eq!(
        label(&rt, "server-notice-action").as_deref(),
        Some("Try again")
    );
    drain(&mut intents);
    click(&mut rt, "server-notice-action");
    assert_eq!(rt.shell_request, None);
    let sent = drain(&mut intents);
    assert!(
        matches!(sent.as_slice(), [Intent::RestartServer]),
        "{sent:?}"
    );
}

#[test]
fn ended_keeps_the_last_view_and_says_why() {
    // The server stopped under a table in play.
    let (mut rt, mut intents) = runtime_with(Some("table-play"), |s| {
        s.on_server_phase(ServerPhase::Ready);
    });
    let conn_id = rt.state.table.as_ref().map(|t| t.conn_id).expect("a table");
    let before = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.store.display_view().cloned())
        .expect("a view");
    rt.update(|s, _| s.on_server_phase(ServerPhase::Stopped));
    receive(
        &mut rt,
        Reply::Conn {
            conn_id,
            event: ConnEvent::Ended(NetError::ServerStopped),
        },
    );
    settle(&mut rt, 0.5);
    let table = rt.state.table.as_ref().expect("the table stays");
    assert_eq!(table.store.display_view(), Some(&before), "never blanked");
    assert!(!table.store.controls_enabled(), "controls stay off");
    assert_eq!(table.stage, TableStage::Failed(NetError::ServerStopped));
    assert!(rt.find("table-ended").is_some());
    // The hand says it is paused, never whose turn it is (review fix PX-8).
    assert!(shows(&rt, "Paused."), "{:?}", texts(&rt));
    let said = texts(&rt)
        .iter()
        .filter(|t| t.as_str() == "Play picks up where it left off.")
        .count();
    assert_eq!(said, 1, "said once, under the heading: {:?}", texts(&rt));
    assert!(
        rt.find("trick-result-text").is_none(),
        "the felt's line is quiet"
    );
    assert!(
        rt.find("table-reconnecting").is_none(),
        "replaces Reconnecting…"
    );
    assert!(shows(
        &rt,
        "The game server stopped. Restart to keep playing."
    ));
    assert_eq!(label(&rt, "table-ended-action").as_deref(), Some("Restart"));
    drain(&mut intents);
    click(&mut rt, "table-ended-action");
    assert!(matches!(
        drain(&mut intents).as_slice(),
        [Intent::RestartServer]
    ));
    // While it starts again the banner waits with it.
    receive(&mut rt, Reply::Server(ServerPhase::Starting));
    settle(&mut rt, 0.5);
    assert!(shows(&rt, server::BOOTING_LINE));
    assert!(rt.find("table-ended-action").is_none());

    // Another device's table points to Your games.
    let (mut rt, _intents) = runtime_with(Some("table-play"), |_| {});
    let conn_id = rt.state.table.as_ref().map(|t| t.conn_id).expect("a table");
    receive(
        &mut rt,
        Reply::Conn {
            conn_id,
            event: ConnEvent::Ended(NetError::Forbidden),
        },
    );
    settle(&mut rt, 0.5);
    assert!(
        shows(
            &rt,
            "This table belongs to another device. Open Your games to find your table."
        ),
        "{:?}",
        texts(&rt)
    );
    assert_eq!(
        label(&rt, "table-ended-action").as_deref(),
        Some("Your saved games")
    );
    click(&mut rt, "table-ended-action");
    assert_eq!(rt.state.route, Route::Games);

    // A device that was signed out says so, and offers a new game (review fix PP-7).
    let (mut rt, _intents) = runtime_with(Some("table-play"), |_| {});
    let conn_id = rt.state.table.as_ref().map(|t| t.conn_id).expect("a table");
    receive(
        &mut rt,
        Reply::Conn {
            conn_id,
            event: ConnEvent::Ended(NetError::SignedOut),
        },
    );
    settle(&mut rt, 0.5);
    assert!(shows(
        &rt,
        "This device was signed out, so older tables can’t be opened."
    ));
    assert!(!texts(&rt).iter().any(|t| t.contains("another device")));
    assert_eq!(
        label(&rt, "table-ended-action").as_deref(),
        Some(server::NEW_GAME)
    );

    // Unreachable: the game is saved, reconnect.
    let (mut rt, mut intents) = runtime_with(Some("table-play"), |s| {
        if let Some(t) = s.table.as_mut() {
            t.game_id = Some(SCENE_GAME.into());
        }
    });
    let conn_id = rt.state.table.as_ref().map(|t| t.conn_id).expect("a table");
    receive(
        &mut rt,
        Reply::Conn {
            conn_id,
            event: ConnEvent::Ended(NetError::Unavailable),
        },
    );
    settle(&mut rt, 0.5);
    assert!(shows(&rt, server::UNREACHABLE_LINE));
    drain(&mut intents);
    click(&mut rt, "table-ended-action");
    assert!(
        drain(&mut intents)
            .iter()
            .any(|i| matches!(i, Intent::OpenTable { game_id, .. } if game_id == SCENE_GAME))
    );

    // With no view yet the same copy is the loading line, and the button follows it.
    let (rt, _intents) = runtime_with(Some("server-starting"), |s| {
        s.on_server_phase(ServerPhase::Failed(BootProblem::Other));
        if let Some(t) = s.table.as_mut() {
            t.stage = TableStage::Failed(NetError::Unauthorized);
        }
    });
    assert!(shows(&rt, "Please start a game on this device first."));
    assert_eq!(
        label(&rt, "loading-reconnect").as_deref(),
        Some(server::NEW_GAME)
    );
}

#[test]
fn the_saved_row_follows_the_table_on_screen() {
    // The band was read when a Restart brought the server back, mid-match. The match played on,
    // so a late or early read of that answer must not say "Resume game" at the old hand.
    let row = |status, score, hand_no| RecentTables {
        tables: vec![RecentTable {
            game_id: SCENE_GAME.into(),
            status: Some(status),
            score: Some(score),
            hand_no: Some(hand_no),
        }],
        unavailable: false,
    };
    let (mut rt, _intents) = runtime_with(Some("table-play"), |s| {
        if let Some(t) = s.table.as_mut() {
            t.game_id = Some(SCENE_GAME.into());
        }
    });
    let live = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.store.view().cloned())
        .expect("a live view");
    let stale = row(euchre_engine::GameStatus::Active, [4, 1], live.hand_no + 2);
    receive(&mut rt, Reply::RecentTables(Ok(stale)));
    let read = |rt: &Runtime<AppState>| {
        rt.state
            .recent_tables
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|r| r.tables[0].clone())
            .expect("a row")
    };
    let now = read(&rt);
    assert_eq!(now.status, Some(live.status));
    assert_eq!(now.score, Some(live.score));
    assert_eq!(now.hand_no, Some(live.hand_no));

    // Another table's row is the server's to say.
    let (mut rt, _intents) = runtime_with(Some("table-play"), |s| {
        if let Some(t) = s.table.as_mut() {
            t.game_id = Some("another-game".into());
        }
    });
    let other = row(euchre_engine::GameStatus::Active, [4, 1], 2);
    receive(&mut rt, Reply::RecentTables(Ok(other.clone())));
    assert_eq!(read(&rt), other.tables[0]);
}

#[test]
fn games_lists_saved_tables_and_opens_one() {
    // Entering Your games asks for the saved tables with the list.
    let (mut rt, mut intents) = runtime_with(None, |_| {});
    drain(&mut intents);
    rt.update(|s, tx| s.navigate(Route::Games, tx));
    let sent = drain(&mut intents);
    assert!(sent.iter().any(|i| matches!(i, Intent::ListGames { .. })));
    assert!(sent.iter().any(|i| matches!(i, Intent::RecentTables)));

    let (mut rt, mut intents) = runtime_with(Some("games-saved-tables"), |_| {});
    assert!(rt.find("saved-tables").is_some());
    assert!(shows(&rt, other::SAVED_H2));
    assert!(shows(&rt, other::SAVED_P));
    assert!(
        shows(&rt, "Resume game · Hand 8 · 6–4 →"),
        "{:?}",
        texts(&rt)
    );
    assert!(shows(&rt, "Open game · Hand 14 · 10–8 →"));
    assert!(shows(&rt, "Reconnect saved table · z5x1c7v3 →"));
    assert!(shows(&rt, other::SAVED_UNAVAILABLE));
    // Above the list.
    let band = rt.find("saved-tables").expect("band").1;
    let stats = rt.find("games-stats").expect("stats").1;
    assert!(band < stats);
    drain(&mut intents);
    click(&mut rt, "saved-table-0");
    assert_eq!(rt.state.route, Route::Table);
    let table = rt.state.table.as_ref().expect("a table");
    assert_eq!(table.game_id.as_deref(), Some(SCENE_GAME));
    assert_eq!(table.stage, TableStage::Opening);
    assert!(drain(&mut intents).iter().any(
        |i| matches!(i, Intent::OpenTable { game_id, conn_id } if game_id == SCENE_GAME && *conn_id == table.conn_id)
    ));
}

#[test]
fn quit_reply_quits() {
    let (mut rt, mut intents) = runtime_with(None, |_| {});
    drain(&mut intents);
    // A quit from the window (Cmd-Q, the last window closing) stops the server at once.
    rt.update(app::on_quit);
    assert!(matches!(drain(&mut intents).as_slice(), [Intent::Shutdown]));
    // A quit from the bridge (a signal) ends the loop.
    assert_eq!(rt.shell_request, None);
    receive(&mut rt, Reply::Quit);
    assert_eq!(rt.shell_request, Some(Shell::Quit));
}

#[test]
fn home_is_interactive_while_the_server_boots() {
    let (mut rt, mut intents) = runtime_with(Some("home"), |s| {
        s.on_server_phase(ServerPhase::Starting);
    });
    assert!(rt.find("server-notice").is_none(), "the boot is quiet");
    drain(&mut intents);
    click(&mut rt, "hero-play");
    assert_eq!(rt.state.route, Route::NewGame, "no waiting");
    settle(&mut rt, 1.0);
    click(&mut rt, "new-submit");
    assert_eq!(rt.state.route, Route::Table);
    assert!(
        drain(&mut intents)
            .iter()
            .any(|i| matches!(i, Intent::CreateTable { .. }))
    );
    settle(&mut rt, 0.5);
    assert!(shows(&rt, "A seat at the table."));
    assert!(shows(&rt, server::BOOTING_LINE));
}

/// Review fixes PX-6 and PP-6: a table that could not be created never says "Your game is
/// saved"; Try again creates it, and a create that timed out (it may have made a table)
/// offers a new game instead of creating a second one on its own.
#[test]
fn a_failed_create_says_so_and_never_creates_twice_by_itself() {
    let created_failed = |err: NetError| {
        runtime_with(Some("home"), move |s| {
            s.on_server_phase(ServerPhase::Ready);
            s.route = Route::Table;
            s.table = Some(app::TableScreen::creating(41, Default::default()));
            if let Some(t) = s.table.as_mut() {
                t.stage = TableStage::Failed(err);
            }
        })
    };
    let (mut rt, mut intents) = created_failed(NetError::Unavailable);
    assert!(
        shows(&rt, "We couldn’t start a game. Try again."),
        "{:?}",
        texts(&rt)
    );
    assert!(
        !texts(&rt)
            .iter()
            .any(|t| t.contains("is saved") || t.contains("are saved")),
        "{:?}",
        texts(&rt)
    );
    assert_eq!(
        label(&rt, "loading-reconnect").as_deref(),
        Some("Try again")
    );
    drain(&mut intents);
    click(&mut rt, "loading-reconnect");
    let sent = drain(&mut intents);
    assert!(
        sent.iter().any(|i| matches!(i, Intent::CreateTable { .. })),
        "{sent:?}"
    );

    let (mut rt, mut intents) = created_failed(NetError::Timeout);
    assert!(shows(&rt, "We couldn’t start a game. Try again."));
    assert_eq!(
        label(&rt, "loading-reconnect").as_deref(),
        Some(server::NEW_GAME)
    );
    // The server coming back does not create it again behind the player's back.
    drain(&mut intents);
    rt.update(|s, _| {
        s.on_server_phase(ServerPhase::Stopped);
        s.on_server_phase(ServerPhase::Starting);
        s.on_server_phase(ServerPhase::Ready);
    });
    let sent = drain(&mut intents);
    assert!(
        !sent.iter().any(|i| matches!(i, Intent::CreateTable { .. })),
        "{sent:?}"
    );
    click(&mut rt, "loading-reconnect");
    assert_eq!(rt.state.route, Route::NewGame);
    assert!(
        !drain(&mut intents)
            .iter()
            .any(|i| matches!(i, Intent::CreateTable { .. }))
    );
}

/// Review fix PX-7: the server notice never covers the page heading, at the smallest window
/// and at a common one.
#[test]
fn the_notice_never_covers_the_heading() {
    let overlaps = |a: [f32; 4], b: [f32; 4]| {
        a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
    };
    let rect = |rt: &Runtime<AppState>, key: &str| {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
            .map(|e| e.rect)
            .unwrap_or_else(|| panic!("{key} is on screen"))
    };
    let pages = [
        (Route::Home, "hero-h1"),
        (Route::Games, "games-h1"),
        (Route::Settings, "settings-h1"),
    ];
    // The stopped notice, and the longest one (three lines at 720 wide).
    let phases = [
        ServerPhase::Stopped,
        ServerPhase::Failed(BootProblem::PortInUse { port: 6420 }),
    ];
    for size in [(720.0, 520.0), (1024.0, 700.0)] {
        for ((route, h1), phase) in pages
            .iter()
            .flat_map(|page| phases.iter().map(move |phase| (page, phase)))
        {
            let (mut rt, _intents) = runtime_sized(size, None, |s| {
                s.route = route.clone();
                s.on_server_phase(phase.clone());
            });
            settle(&mut rt, 0.5);
            let notice = rect(&rt, "server-notice");
            let heading = rect(&rt, h1);
            assert!(
                !overlaps(notice, heading),
                "{size:?} {route:?}: notice {notice:?} covers {h1} {heading:?}"
            );
            // Inside the window.
            assert!(
                notice[1] + notice[3] <= size.1 + 0.5,
                "{size:?} {route:?} {notice:?}"
            );
        }
    }
}

/// Review fix PP-5: one page at a time, and only the page on screen lands.
#[test]
fn the_games_pager_waits_for_its_page() {
    let (mut rt, mut intents) = runtime_with(Some("games"), |_| {});
    assert!(rt.find("games-older").is_some(), "{:?}", texts(&rt));
    reveal_click(&mut rt, "games-older");
    let sent = drain(&mut intents);
    let cursor = match sent.as_slice() {
        [Intent::ListGames { cursor: Some(c) }] => c.clone(),
        other => panic!("{other:?}"),
    };
    assert!(rt.state.games.loading);
    // A second press while it loads sends nothing.
    if rt.find("games-older").is_some() {
        reveal_click(&mut rt, "games-older");
    }
    if rt.find("games-newer").is_some() {
        reveal_click(&mut rt, "games-newer");
    }
    assert!(drain(&mut intents).is_empty(), "one page at a time");
    // A late answer for the first page is dropped; the asked page lands.
    let first = rt.state.games.page.clone().expect("the first page");
    receive(
        &mut rt,
        Reply::Games {
            cursor: None,
            result: Ok(first.clone()),
        },
    );
    assert!(rt.state.games.loading, "still waiting for its own page");
    let mut older = first.clone();
    older.cursor = None;
    receive(
        &mut rt,
        Reply::Games {
            cursor: Some(cursor),
            result: Ok(older.clone()),
        },
    );
    assert!(!rt.state.games.loading);
    assert_eq!(rt.state.games.page.as_ref(), Some(&older));
}

#[test]
fn a_redelivered_chat_line_shows_once() {
    // A reconnect can redeliver a line the table already has (review fix PP-4).
    let (mut rt, _intents) = runtime_with(Some("table-play"), |_| {});
    let conn_id = rt.state.table.as_ref().map(|t| t.conn_id).expect("a table");
    let line = ChatEvent {
        msg_id: "banter-1".to_owned(),
        seat: Seat::WEST,
        kind: ChatKind::Banter,
        text: "Somebody brought their good cards tonight.".to_owned(),
        is_final: True,
    };
    let chat = |line: &ChatEvent| Reply::Conn {
        conn_id,
        event: ConnEvent::Event(TableEvent::Chat(line.clone())),
    };
    receive(&mut rt, chat(&line));
    let first = {
        let store = &rt.state.table.as_ref().expect("the table").store;
        assert_eq!(store.chat().len(), 1);
        store
            .visible_table_talk()
            .map(|talk| talk.until_ms)
            .expect("the bubble is up")
    };
    // Later, the same line again.
    settle(&mut rt, 7.0);
    receive(&mut rt, chat(&line));
    let store = &rt.state.table.as_ref().expect("the table").store;
    assert_eq!(store.chat().len(), 1, "the log keeps one copy");
    let again = store.visible_table_talk().map(|talk| talk.until_ms);
    assert!(
        again.is_none_or(|until| until == first),
        "the bubble does not pop a second time: {first} then {again:?}"
    );
    // A new line still shows.
    let next = ChatEvent {
        msg_id: "banter-2".to_owned(),
        ..line
    };
    receive(&mut rt, chat(&next));
    let store = &rt.state.table.as_ref().expect("the table").store;
    assert_eq!(store.chat().len(), 2);
    assert_eq!(
        store.visible_table_talk().map(|talk| talk.msg_id.as_str()),
        Some("banter-2")
    );
}
