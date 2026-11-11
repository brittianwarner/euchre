//! Headless shell tests: navigation, the new-game flow against the in-process SimBackend, and
//! reduce motion. No window and no network.

use std::sync::Arc;
use std::time::{Duration, Instant};

use euchre_desktop::app::{AppState, Route, TableStage};
use euchre_desktop::net::bridge::{Bridge, BridgeConfig, BridgeThread, Reply, UiSink};
use euchre_desktop::net::sim::{self, SimBackend, SimOptions, Tempo};
use euchre_desktop::prefs::Prefs;
use euchre_desktop::theme::motion;
use euchre_desktop::{scenes, spec, theme};
use euchre_profile_model::{PERSONA_PRESETS, Pace};
use rune::{App, Env, Key, KeyEvent, Mods, Runtime};

const DT: f64 = 1.0 / 60.0;

fn runtime(state: AppState) -> Runtime<AppState> {
    let mut state = state;
    state.wait_for_art = true;
    let mut rt = App::new(spec())
        .init(move || state)
        .headless_runtime(theme::window::SIZE, Env::new(false, 1.0));
    rt.frame(0.0);
    rt
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

/// A nav click once the last route change has settled (the nav ignores presses for
/// `ROUTE_SETTLE` after one).
fn nav(rt: &mut Runtime<AppState>, key: &str) {
    settle(rt, motion::ROUTE_SETTLE);
    click(rt, key);
}

fn press(rt: &mut Runtime<AppState>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    rt.frame(DT);
}

fn scene(state: &mut AppState, name: &str) {
    let (_, apply) = scenes::SCENES
        .iter()
        .find(|(n, _)| *n == name)
        .expect("scene exists");
    apply(state);
}

#[test]
fn nav_reaches_every_screen_and_escape_goes_back() {
    let mut rt = runtime(AppState::default());
    assert_eq!(rt.state.route, Route::Home);
    assert!(
        rt.find("nav").is_none(),
        "Home draws its own header, not the pill"
    );

    click(&mut rt, "home-nav-games");
    assert_eq!(rt.state.route, Route::Games);
    assert!(rt.find("nav").is_some(), "the pill shows off Home");

    nav(&mut rt, "nav-settings");
    assert_eq!(rt.state.route, Route::Settings);
    nav(&mut rt, "nav-play");
    assert_eq!(rt.state.route, Route::NewGame);

    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.route, Route::Settings);
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.route, Route::Games);

    // A press lands only where the new sheet's wipe has reached (09 §4.12), so each one
    // waits for the change to settle.
    nav(&mut rt, "nav-home");
    assert_eq!(rt.state.route, Route::Home);
    nav(&mut rt, "hero-play");
    assert_eq!(rt.state.route, Route::NewGame);
    nav(&mut rt, "new-back");
    assert_eq!(rt.state.route, Route::Games);
}

/// A double-click on Home's Play: the second click lands on the pill New game draws where
/// Home's links were, and must not take the player straight back Home (B1).
#[test]
fn a_double_click_on_a_nav_link_goes_one_screen() {
    let mut rt = runtime(AppState::default());
    settle(&mut rt, 0.5);
    let (x, y) = rt
        .find("home-nav-play")
        .expect("Home's Play link is on screen");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(DT);
    assert_eq!(rt.state.route, Route::NewGame);
    for _ in 0..12 {
        rt.frame(DT);
    }
    let under = rt.element_at(x, y).map(|p| p.path().to_owned());
    assert!(
        under.as_deref().is_some_and(|p| p.contains("nav")),
        "the pill sits under the pointer on New game: {under:?}"
    );
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(DT);
    assert_eq!(
        rt.state.route,
        Route::NewGame,
        "the second click of a double-click does not navigate again"
    );
    nav(&mut rt, "nav-home");
    assert_eq!(
        rt.state.route,
        Route::Home,
        "a later click on the nav works"
    );
}

#[test]
fn new_game_options_toggle_and_show_the_note() {
    let mut rt = runtime(AppState::default());
    click(&mut rt, "hero-play");
    // Let the route slide finish, so the button is measured where it rests.
    settle(&mut rt, 1.0);
    click(&mut rt, "new-stick");
    assert!(rt.state.new_game.stick);
    assert!(!rt.state.new_game.natural);
    let submit_before = rt.find("new-submit");
    click(&mut rt, "new-natural");
    assert!(rt.state.new_game.natural);
    assert_eq!(
        rt.find("new-submit"),
        submit_before,
        "the note never moves the button"
    );
    press(&mut rt, Key::Tab);
    click(&mut rt, "new-natural");
    assert!(!rt.state.new_game.natural);
}

#[test]
fn table_hides_the_global_nav() {
    let mut state = AppState::default();
    scene(&mut state, "table-deal");
    let mut rt = runtime(state);
    assert_eq!(rt.state.route, Route::Table);
    assert!(rt.find("nav").is_none());
    assert!(rt.find("rack").is_some());
    let view = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.store.display_view())
        .cloned();
    let view = view.expect("the scene has a view");
    assert_eq!(view.hand.len(), 5);
    assert!(rt.find(&format!("hand-{}", view.hand[0])).is_some());
    // Escape on the table does nothing (it is not a page with a back).
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.route, Route::Table);
    click(&mut rt, "table-home");
    assert_eq!(rt.state.route, Route::Home);
    assert!(rt.state.table.is_none(), "leaving closes the table");
}

/// Pumps bridge replies into the runtime until `done` or the deadline.
fn pump(
    rt: &mut Runtime<AppState>,
    replies: &std::sync::mpsc::Receiver<Reply>,
    within: Duration,
    done: impl Fn(&AppState) -> bool,
) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        while let Ok(reply) = replies.try_recv() {
            rt.update(move |s, tx| s.receive(reply, tx));
        }
        rt.frame(DT);
        if done(&rt.state) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn new_game_flow_deals_a_hand_from_the_sim() {
    let (bridge, intents) = Bridge::channel();
    let (sink, replies) = UiSink::channel();
    let backend = Arc::new(SimBackend::new(SimOptions {
        tempo: Tempo::FAST,
        ..SimOptions::default()
    }));
    let _thread = BridgeThread::spawn(
        BridgeConfig {
            backend,
            sink,
            prefs: None,
            quit_on_signals: false,
        },
        intents,
    )
    .expect("bridge thread");
    // The first-run tour opens over the first real hand; this test is about the deal, so it
    // starts with the tour seen (requests/motion-E-to-D.md 1).
    let prefs = Prefs {
        tour_seen: true,
        ..Prefs::default()
    };
    let mut rt = runtime(AppState::new(prefs, Some(bridge), "sim"));

    click(&mut rt, "hero-play");
    // The New game sheet takes presses once its wipe is across (09 §4.12).
    nav(&mut rt, "new-stick");
    click(&mut rt, "new-submit");
    assert_eq!(rt.state.route, Route::Table);
    let table = rt.state.table.as_ref().expect("a table");
    assert!(table.cfg.stick_the_dealer);
    assert!(!table.cfg.require_natural_trump);
    assert_eq!(table.stage, TableStage::Creating);

    let my_turn = |s: &AppState| {
        s.table
            .as_ref()
            .and_then(|t| t.store.display_view())
            .is_some_and(|v| v.turn_seat == Some(v.you) && !v.legal.is_empty())
    };
    assert!(
        pump(&mut rt, &replies, Duration::from_secs(10), my_turn),
        "the sim dealt and handed the turn to the player"
    );
    let table = rt.state.table.as_ref().expect("a table");
    assert_eq!(table.stage, TableStage::Open);
    assert!(table.game_id.is_some());
    let view = table.store.display_view().expect("a view").clone();
    assert_eq!(view.hand.len(), 5);
    assert_eq!(view.rules.as_ref().map(|r| r.stick_the_dealer), Some(true));

    // Make the first legal move through the UI and wait for the table to move on.
    let first = view.legal[0].id.clone();
    let key = if let Some(card) = first.strip_prefix("play:") {
        rt.update(|s, _| {
            if let Some(c) = euchre_protocol::Card::parse(card) {
                s.select_card(c);
            }
        });
        rt.frame(DT);
        "hand-play".to_owned()
    } else {
        format!("bid-{first}")
    };
    settle(&mut rt, 0.1);
    click(&mut rt, &key);
    let moved = |s: &AppState| {
        s.table
            .as_ref()
            .and_then(|t| t.store.display_view())
            .is_some_and(|v| v.v > view.v)
            && s.table.as_ref().is_some_and(|t| !t.store.submitting())
    };
    assert!(
        pump(&mut rt, &replies, Duration::from_secs(10), moved),
        "the move was acknowledged and the view advanced"
    );
}

/// How far the selected card's art has travelled two frames after selection, and at rest.
fn selection_travel(reduce_pref: bool, reduce_system: bool) -> (f32, f32) {
    let mut state = AppState::default();
    state.prefs.reduce_motion = reduce_pref;
    scene(&mut state, "table-deal");
    let mut rt = runtime(state);
    rt.set_system_reduced_motion(reduce_system);
    settle(&mut rt, 1.0);
    let card = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.store.display_view())
        .map(|v| v.hand[0])
        .expect("a hand");
    let key = format!("card-{card}");
    let y = |rt: &Runtime<AppState>| rt.presented(&key).expect("card art drawn").0[1];
    let before = y(&rt);
    rt.update(move |s, _| s.select_card(card));
    rt.frame(DT);
    rt.frame(DT);
    let early = before - y(&rt);
    settle(&mut rt, 1.0);
    (early, before - y(&rt))
}

#[test]
fn reduce_motion_is_respected() {
    let (early, rest) = selection_travel(false, false);
    assert!(rest > 7.0, "a selected card lifts: {rest}");
    assert!(
        early > 0.0 && early < rest,
        "with a spring, not a jump: {early} of {rest}"
    );
    assert_eq!(
        selection_travel(true, false),
        (0.0, 0.0),
        "the app preference keeps it still"
    );
    assert_eq!(
        selection_travel(false, true),
        (0.0, 0.0),
        "so does the system setting"
    );
}

fn rect(rt: &Runtime<AppState>, key: &str) -> Option<[f32; 4]> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .map(|e| e.rect)
}

#[test]
fn deal_me_in_stays_above_the_fold_in_the_smallest_window() {
    let mut state = AppState::default();
    state.wait_for_art = true;
    state.route = Route::NewGame;
    let mut rt = App::new(spec())
        .init(move || state)
        .headless_runtime((720.0, 520.0), Env::new(false, 1.0));
    rt.frame(0.0);
    settle(&mut rt, 0.3);
    let before = rect(&rt, "new-submit").expect("the button");
    assert!(
        before[1] >= 0.0 && before[1] + before[3] <= 520.0,
        "Deal me in {before:?} is inside 720x520"
    );
    click(&mut rt, "new-stick");
    click(&mut rt, "new-natural");
    settle(&mut rt, 0.3);
    assert_eq!(
        rect(&rt, "new-submit"),
        Some(before),
        "the note never moves the button"
    );
}

#[test]
fn settings_credit_the_art_and_show_its_license() {
    let mut rt = runtime(AppState::default());
    rt.update(|s, tx| s.navigate(Route::Settings, tx));
    settle(&mut rt, 0.3);
    let credits = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("settings-credits"))
        .and_then(|e| e.text)
        .expect("the credits");
    assert_eq!(credits, euchre_desktop::assets::CREDITS);
    assert!(rt.find("settings-license").is_none(), "the license waits");
    reveal_click(&mut rt, "settings-license-toggle");
    settle(&mut rt, 0.3);
    let license = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("settings-license"))
        .and_then(|e| e.text)
        .expect("the license");
    assert!(license.contains("CC0 1.0 Universal"));
}

// ---- Route transitions (09 §4.12, §6.5) ----

/// The x of `key`'s drawn rect, exiting or not, or None once it is gone.
fn drawn_x(rt: &Runtime<AppState>, key: &str) -> Option<f32> {
    rt.presented(key).map(|(r, _)| r[0])
}

/// `key`'s drawn x and opacity, exiting or not, or None once it is gone.
fn drawn(rt: &Runtime<AppState>, key: &str) -> Option<(f32, f32)> {
    rt.presented(key).map(|(r, o)| (r[0], o))
}

/// Runs frames until `secs` have passed since `start` on the app clock.
fn run_to(rt: &mut Runtime<AppState>, start: f64, secs: f64) {
    while rt.state.clock < start + secs - DT * 0.5 {
        rt.frame(DT);
    }
}

/// Home → New, then (when `back`) New → Home. Returns the entering and leaving sheets' drawn
/// (x, opacity) at +0.08 s and at +1.2 s after the change (None once a sheet is gone).
fn route_change(back: bool) -> [[Option<(f32, f32)>; 2]; 2] {
    let mut rt = runtime(AppState::default());
    settle(&mut rt, 0.5);
    let (entering, leaving) = if back {
        click(&mut rt, "home-nav-play");
        settle(&mut rt, 1.5);
        ("screen-home", "screen-new")
    } else {
        ("screen-new", "screen-home")
    };
    let start = rt.state.clock;
    if back {
        press(&mut rt, Key::Escape);
        assert_eq!(rt.state.route, Route::Home);
    } else {
        click(&mut rt, "home-nav-play");
        assert_eq!(rt.state.route, Route::NewGame);
    }
    run_to(&mut rt, start, 0.08);
    let mid = [drawn(&rt, entering), drawn(&rt, leaving)];
    run_to(&mut rt, start, 1.2);
    let end = [drawn(&rt, entering), drawn(&rt, leaving)];
    [mid, end]
}

/// A screen that is gone or back at its resting x (screens fill the window from x 0).
fn at_rest(x: Option<f32>) -> bool {
    x.is_none_or(|x| x.abs() < 0.5)
}

/// The wipe (09 §4.12): mid-change the new sheet is drawn whole at x 0 (its soft edge is a
/// mask, not a slide), and the old page is still drawn under it, at x 0 and mostly opaque, so
/// the wipe never crosses an empty window. By +1.2 s the new sheet is at rest and the old one
/// has gone.
fn assert_wipe(dir: &str, [mid, end]: [[Option<(f32, f32)>; 2]; 2]) {
    let (new_x, new_o) = mid[0].unwrap_or_else(|| panic!("{dir}: the new sheet is drawn"));
    let (old_x, old_o) = mid[1].unwrap_or_else(|| panic!("{dir}: the old sheet is still drawn"));
    assert!(
        new_x.abs() < 0.5,
        "{dir}: the new sheet does not slide: x {new_x}"
    );
    assert!(
        new_o > 0.99,
        "{dir}: the new sheet is opaque, not a cross-fade: {new_o}"
    );
    assert!(
        old_x.abs() < 0.5,
        "{dir}: the old sheet does not slide: x {old_x}"
    );
    assert!(
        old_o > 0.5,
        "{dir}: the old page stays under the wipe: opacity {old_o}"
    );
    assert!(
        end[0].is_some_and(|(x, o)| x.abs() < 0.5 && o > 0.99),
        "{dir}: the new sheet is at rest by +1.2 s: {:?}",
        end[0]
    );
    assert!(
        end[1].is_none(),
        "{dir}: the old sheet is gone by +1.2 s: {:?}",
        end[1]
    );
}

#[test]
fn route_forward_wipes_the_new_sheet_over_the_old_page() {
    assert_wipe("forward", route_change(false));
}

#[test]
fn route_back_wipes_the_old_screen_back_over() {
    assert_wipe("back", route_change(true));
}

/// The wipe comes from the right going forward and from the left going back; reduce motion
/// fades (D15). The table leaves faster than a page, so its raised panel and confetti never
/// hang over the next screen.
#[test]
fn route_wipe_edge_follows_the_direction() {
    use euchre_desktop::app::RouteDir;
    use euchre_desktop::screens::{route_exit, route_transition};
    use rune::{Edge, SpringPair, Transition};
    let wipe = |edge| {
        Some(
            Transition::asymmetric(Transition::reveal(edge), Transition::opacity())
                .springs(SpringPair::new(motion::ROUTE_IN, motion::ROUTE_OUT)),
        )
    };
    assert_eq!(
        route_transition(RouteDir::Forward, false),
        wipe(Edge::Trailing)
    );
    assert_eq!(route_transition(RouteDir::Back, false), wipe(Edge::Leading));
    for dir in [RouteDir::Forward, RouteDir::Back] {
        assert_eq!(route_transition(dir, true), Some(Transition::opacity()));
    }
    assert_ne!(route_exit(true, false), route_exit(false, false));
    assert_eq!(route_exit(true, true), Transition::opacity());
}

/// Leaving the table from the match-end panel: the felt, its raised panel and its confetti are
/// gone well before the New game page is across, so they never hang over it.
#[test]
fn leaving_the_table_clears_its_raised_layers_quickly() {
    let mut state = AppState::default();
    scene(&mut state, "game-over-win");
    let mut rt = runtime(state);
    settle(&mut rt, 2.0);
    let start = rt.state.clock;
    click(&mut rt, "match-end-again");
    assert_eq!(rt.state.route, Route::NewGame);
    run_to(&mut rt, start, 0.35);
    for key in ["screen-table", "match-end-panel"] {
        let shown = rt.presented(key).map_or(0.0, |(_, o)| o);
        assert!(shown < 0.1, "{key} has left by +0.35 s: opacity {shown}");
    }
}

#[test]
fn route_change_under_reduce_motion_only_fades() {
    for back in [false, true] {
        let mut state = AppState::default();
        state.prefs.reduce_motion = true;
        let mut rt = runtime(state);
        settle(&mut rt, 0.5);
        if back {
            click(&mut rt, "home-nav-play");
            settle(&mut rt, 1.0);
            press(&mut rt, Key::Escape);
        } else {
            click(&mut rt, "home-nav-play");
        }
        for frame in 0..40 {
            for key in ["screen-home", "screen-new"] {
                let x = drawn_x(&rt, key);
                assert!(
                    at_rest(x),
                    "{key} has no x offset under reduce motion (back {back}, frame {frame}): {x:?}"
                );
            }
            rt.frame(DT);
        }
    }
}

// ---- Frame time (09 §6.5 D-first-deal-p95) ----

/// The `p95` figures of a headless `euchre` run: the all-frames line and the animating line.
fn frame_p95(args: &[&str]) -> (f32, f32) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_euchre"))
        .args(args)
        .args(["--size", "1280x800", "--scale", "1"])
        .output()
        .expect("the euchre binary runs");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "euchre {args:?} failed: {err}");
    let p95 = |marker: &str| -> f32 {
        let line = err
            .lines()
            .find(|l| l.contains(marker))
            .unwrap_or_else(|| panic!("no '{marker}' line in: {err}"));
        let after = line
            .split_once("p95 ")
            .unwrap_or_else(|| panic!("no p95 in {line}"))
            .1;
        after
            .split(|c: char| c.is_whitespace() || c == ',' || c == ')')
            .next()
            .and_then(|w| w.parse().ok())
            .unwrap_or_else(|| panic!("a p95 number in {line}"))
    };
    (p95("frame cpu mean"), p95("animating frames"))
}

#[test]
fn the_first_deal_stays_inside_the_frame_budget() {
    let dir = std::env::temp_dir().join(format!("euchre-shell-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    let snap = dir.join("deal.png").to_string_lossy().into_owned();
    let (all, animating) = frame_p95(&[
        "--scene",
        "table-deal-anim",
        "--frames",
        "150",
        "--snap",
        &snap,
    ]);
    assert!(all <= 8.0, "p95 over every frame is {all} ms");
    assert!(
        animating <= 8.0,
        "p95 over animating frames is {animating} ms"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A runtime wired to a live SimBackend on its own bridge thread, with the tour seen.
fn live(
    options: SimOptions,
) -> (
    Runtime<AppState>,
    std::sync::mpsc::Receiver<Reply>,
    BridgeThread,
) {
    let (bridge, intents) = Bridge::channel();
    let (sink, replies) = UiSink::channel();
    let backend = Arc::new(SimBackend::new(options));
    let thread = BridgeThread::spawn(
        BridgeConfig {
            backend,
            sink,
            prefs: None,
            quit_on_signals: false,
        },
        intents,
    )
    .expect("bridge thread");
    let prefs = Prefs {
        tour_seen: true,
        ..Prefs::default()
    };
    (
        runtime(AppState::new(prefs, Some(bridge), "sim")),
        replies,
        thread,
    )
}

/// Scrolls the page until `key` is inside the window, then gives its centre.
fn reveal(rt: &mut Runtime<AppState>, key: &str) -> (f32, f32) {
    let (_, h) = theme::window::SIZE;
    for _ in 0..60 {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is drawn"));
        if (60.0..h - 60.0).contains(&y) {
            return (x, y);
        }
        let dy = if y >= h - 60.0 { -240.0 } else { 240.0 };
        // Wheel in the page's left margin, clear of any field with its own scroll.
        rt.wheel(40.0, h / 2.0, dy);
        settle(rt, 0.3);
    }
    panic!("{key} never scrolled into view");
}

fn reveal_click(rt: &mut Runtime<AppState>, key: &str) {
    let (x, y) = reveal(rt, key);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(DT);
}

#[test]
fn games_show_lifetime_stats_page_through_and_open_a_match() {
    let (mut rt, replies, _thread) = live(SimOptions {
        tempo: Tempo::INSTANT,
        ..SimOptions::default()
    });
    rt.update(|s, tx| s.navigate(Route::Games, tx));
    let loaded = |s: &AppState| {
        s.games.stats.is_some() && s.games.page.as_ref().is_some_and(|p| !p.rows.is_empty())
    };
    assert!(
        pump(&mut rt, &replies, Duration::from_secs(20), loaded),
        "the first page and the stats"
    );
    settle(&mut rt, 1.0);
    let page = rt.state.games.page.clone().expect("a page");
    assert_eq!(page.rows.len(), sim::GAMES_PAGE);
    let stats = rt.state.games.stats.as_ref().map(|s| s.matches);
    assert_eq!(stats, Some(sim::SIM_HISTORY as i64));
    assert!(rt.find("games-stats").is_some(), "the stats grid");
    assert!(
        rt.find(&format!("game-{}", page.rows[0].match_id))
            .is_some()
    );
    assert!(
        rt.find("games-newer").is_none(),
        "nothing newer than the first page"
    );

    reveal_click(&mut rt, "games-older");
    let second = |s: &AppState| {
        !s.games.loading
            && s.games
                .page
                .as_ref()
                .is_some_and(|p| p.rows.len() < sim::GAMES_PAGE)
    };
    assert!(
        pump(&mut rt, &replies, Duration::from_secs(5), second),
        "page two"
    );
    settle(&mut rt, 0.5);
    assert_eq!(
        rt.state.games.page.as_ref().map(|p| p.rows.len()),
        Some(sim::SIM_HISTORY - sim::GAMES_PAGE)
    );
    assert!(rt.find("games-older").is_none(), "the last page");
    assert!(rt.find("games-stats").is_some(), "the stats stay");

    reveal_click(&mut rt, "games-newer");
    let first = |s: &AppState| {
        !s.games.loading
            && s.games
                .page
                .as_ref()
                .is_some_and(|p| p.rows.len() == sim::GAMES_PAGE)
    };
    assert!(
        pump(&mut rt, &replies, Duration::from_secs(5), first),
        "back to page one"
    );
    settle(&mut rt, 0.5);
    assert!(rt.find("games-newer").is_none(), "back on the first page");

    let id = page.rows[0].match_id.clone();
    reveal_click(&mut rt, &format!("game-{id}"));
    assert_eq!(rt.state.route, Route::MatchDetail(id.clone()));
    let opened = |s: &AppState| matches!(&s.match_detail, Some((_, Ok(_))));
    assert!(
        pump(&mut rt, &replies, Duration::from_secs(5), opened),
        "the match"
    );
    settle(&mut rt, 1.0);
    let hands = page.rows[0].hands_played;
    assert!(rt.find("match-totals").is_some());
    assert!(rt.find("match-hand-0").is_some());
    assert!(rt.find(&format!("match-hand-{}", hands - 1)).is_some());
    reveal_click(&mut rt, "match-back");
    assert_eq!(rt.state.route, Route::Games);
}

#[test]
fn settings_save_round_trips_through_the_sim() {
    let (mut rt, replies, _thread) = live(SimOptions {
        history: 0,
        ..SimOptions::default()
    });
    rt.update(|s, tx| s.navigate(Route::Settings, tx));
    let loaded = |s: &AppState| s.settings.value.is_some();
    assert!(
        pump(&mut rt, &replies, Duration::from_secs(5), loaded),
        "settings load"
    );
    settle(&mut rt, 1.0);
    let draft = |rt: &Runtime<AppState>| {
        rt.state
            .settings
            .value
            .as_ref()
            .expect("data")
            .draft
            .clone()
    };
    let name = draft(&rt).personas[0].name.text.clone();

    reveal_click(&mut rt, "set-pace-relaxed");
    assert_eq!(draft(&rt).pace, Pace::Relaxed);

    let preset = &PERSONA_PRESETS[1];
    reveal_click(&mut rt, &format!("set-p0-preset-{}", preset.id));
    assert_eq!(draft(&rt).personas[0].blurb.text, preset.blurb);
    assert_eq!(
        draft(&rt).personas[0].name.text,
        name,
        "a preset keeps the name"
    );

    // A dial: the far end of the track is 100%, and the left arrow steps back 5.
    let (_, vy) = reveal(&mut rt, "set-p0-risk");
    let ([x, _, w, _], _) = rt.presented("set-p0-risk").expect("the risk dial");
    rt.pointer_down(x + w - 1.0, vy);
    rt.pointer_up(x + w - 1.0, vy);
    rt.frame(DT);
    assert_eq!(draft(&rt).personas[0].risk, 1.0);
    press(&mut rt, Key::Left);
    assert!((draft(&rt).personas[0].risk - 0.95).abs() < 1e-9);

    // A name never runs past its limit.
    reveal_click(&mut rt, "set-p0-name");
    rt.paste("x".repeat(40));
    rt.frame(DT);
    let long = draft(&rt).personas[0].name.text.clone();
    assert_eq!(
        long.chars().count(),
        euchre_protocol::PERSONA_NAME_MAX_CHARS
    );

    reveal_click(&mut rt, "set-save");
    let saved = |s: &AppState| s.settings.value.as_ref().is_some_and(|d| d.just_saved);
    assert!(
        pump(&mut rt, &replies, Duration::from_secs(5), saved),
        "the save"
    );
    rt.frame(DT);
    let data = rt.state.settings.value.clone().expect("data");
    assert_eq!(data.saved.table.pace, Pace::Relaxed);
    assert_eq!(data.saved.personas[0].blurb, preset.blurb);
    assert_eq!(data.saved.personas[0].name, long);
    assert!((data.saved.personas[0].risk - 0.95).abs() < 1e-9);
    assert!(rt.find("set-saved").is_some(), "Saved. shows");
    assert!(rt.state.settings.error.is_none());

    reveal_click(&mut rt, "set-pace-brisk");
    assert!(rt.find("set-saved").is_none(), "a change clears Saved.");
}

/// Lane N2's scenes (contract §N2.9) render headless, each showing what it is for.
#[test]
fn server_scenes_render() {
    for &name in scenes::SERVER_SCENES {
        let mut state = AppState::default();
        scene(&mut state, name);
        let mut rt = runtime(state);
        settle(&mut rt, 1.0);
        let expect = match name {
            "server-starting" | "server-downloading" => "loading-h1",
            "server-stopped-table" | "live-stopped" => "table-ended",
            "games-saved-tables" => "saved-table-2",
            "live-table" => "table-layout",
            "live-games" => "games-h1",
            "live-match" => "match-totals",
            _ => "server-notice-action",
        };
        assert!(rt.find(expect).is_some(), "{name} shows {expect}");
    }
}
