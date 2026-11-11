//! Headless table tests: a whole match played through the UI against the in-process
//! SimBackend, an illegal card refused, trick review, and reduce motion on the table.

use std::sync::Arc;
use std::time::{Duration, Instant};

use euchre_desktop::app::TableScreen;
use euchre_desktop::app::{AppState, Route};
use euchre_desktop::net::bridge::{Bridge, BridgeConfig, BridgeThread, Reply, UiSink};
use euchre_desktop::net::sim::{HeuristicDecider, SimBackend, SimDecider, SimOptions, Tempo};
use euchre_desktop::prefs::Prefs;
use euchre_desktop::screens::table::copy;
use euchre_desktop::theme::felt::timing;
use euchre_desktop::theme::motion;
use euchre_desktop::{scenes, spec, theme};
use euchre_protocol::store::{ACTION_TIMEOUT_MS, RequestId};
use euchre_protocol::{Card, GameStatus, Move, PublicGameView, StoreEffects, StoreEvent};
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

/// A runtime at a given window size.
fn sized(state: AppState, size: (f32, f32)) -> Runtime<AppState> {
    let mut state = state;
    state.wait_for_art = true;
    let mut rt = App::new(spec())
        .init(move || state)
        .headless_runtime(size, Env::new(false, 1.0));
    rt.frame(0.0);
    rt
}

/// The laid-out rect `[x, y, w, h]` of a keyed element still on screen.
fn rect(rt: &Runtime<AppState>, key: &str) -> Option<[f32; 4]> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .map(|e| e.rect)
}

/// The key of the focused element.
fn focused(rt: &Runtime<AppState>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused)
        .and_then(|e| e.key)
}

/// Whether `inner` lies inside `outer`, to half a pixel.
fn inside(inner: [f32; 4], outer: [f32; 4]) -> bool {
    const EPS: f32 = 0.5;
    inner[0] >= outer[0] - EPS
        && inner[1] >= outer[1] - EPS
        && inner[0] + inner[2] <= outer[0] + outer[2] + EPS
        && inner[1] + inner[3] <= outer[1] + outer[3] + EPS
}

/// Runs frames until `key` is fully shown and still (at most two seconds).
fn landed(rt: &mut Runtime<AppState>, key: &str) {
    for _ in 0..120 {
        let e = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting);
        if e.is_some_and(|e| e.opacity >= 0.99 && !e.moving) {
            return;
        }
        rt.frame(DT);
    }
}

fn collect_of(state: &AppState) -> Option<euchre_desktop::screens::table::present::Collect> {
    state.table.as_ref().and_then(|t| t.present.collect.clone())
}

fn view(state: &AppState) -> Option<&PublicGameView> {
    state.table.as_ref().and_then(|t| t.store.display_view())
}

fn selected(state: &AppState) -> Option<Card> {
    state.table.as_ref().and_then(|t| t.selected)
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
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

/// A table on the sim with a live bridge, past the deal and into the first decision.
fn sim_table(
    seed: &str,
) -> (
    Runtime<AppState>,
    std::sync::mpsc::Receiver<Reply>,
    BridgeThread,
) {
    let (bridge, intents) = Bridge::channel();
    let (sink, replies) = UiSink::channel();
    let backend = Arc::new(SimBackend::new(SimOptions {
        seed: seed.to_owned(),
        tempo: Tempo::INSTANT,
        ..SimOptions::default()
    }));
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
    // The tour opens by itself over a first real hand; these tests click by position.
    let prefs = Prefs {
        tour_seen: true,
        ..Prefs::default()
    };
    let mut rt = runtime(AppState::new(prefs, Some(bridge), "sim"));
    click(&mut rt, "hero-play");
    // The New game sheet takes presses once its wipe is across (09 §4.12).
    settle(&mut rt, motion::ROUTE_SETTLE);
    click(&mut rt, "new-submit");
    assert_eq!(rt.state.route, Route::Table);
    (rt, replies, thread)
}

/// Whether the player has something to do: a move, a reviewed trick, or the match is over.
fn actionable(s: &AppState) -> bool {
    let Some(table) = s.table.as_ref() else {
        return false;
    };
    if table.store.submitting() || table.store.review_busy() {
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

#[test]
fn a_whole_match_plays_to_game_over_through_the_ui() {
    let (mut rt, replies, _thread) = sim_table("ui-match");
    assert!(
        pump(&mut rt, &replies, Duration::from_secs(10), actionable),
        "the table dealt"
    );
    // The table takes presses once the route's wipe is across (09 §4.12).
    settle(&mut rt, motion::ROUTE_SETTLE);
    // Review each trick, so Continue is part of the run.
    rt.update(|s, _| s.set_review_tricks(true));
    let mut moves = 0;
    let mut continues = 0;
    let end = Instant::now() + Duration::from_secs(150);
    loop {
        assert!(Instant::now() < end, "the match finished in time");
        assert!(
            pump(&mut rt, &replies, Duration::from_secs(20), actionable),
            "the table always comes back to the player ({moves} moves)"
        );
        let v = view(&rt.state).expect("a view").clone();
        if v.status != GameStatus::Active {
            break;
        }
        if v.awaiting_trick_review == Some(true) && v.legal.is_empty() {
            click(&mut rt, "trick-continue");
            continues += 1;
            let turn = v.turn_id.clone();
            let ok = pump(&mut rt, &replies, Duration::from_secs(10), |s| {
                view(s).is_some_and(|w| w.turn_id != turn)
            });
            if !ok {
                let t = rt.state.table.as_ref().expect("table");
                let w = t.store.display_view().expect("view");
                panic!(
                    "Continue stuck: collect {:?} turn {} vs {} awaiting {:?} busy {} err {:?} rev_err {:?}",
                    t.present.collect,
                    w.turn_id,
                    turn,
                    w.awaiting_trick_review,
                    t.store.review_busy(),
                    t.store.error(),
                    t.store.review_error()
                );
            }
            settle(&mut rt, 0.1);
            assert!(
                rt.find("trick-continue").is_none()
                    || view(&rt.state).is_some_and(|w| w.awaiting_trick_review == Some(true)),
                "an answered Continue leaves"
            );
            continue;
        }
        let id = HeuristicDecider
            .decide(&v)
            .unwrap_or_else(|| v.legal[0].id.clone());
        let id = id.strip_suffix("+alone").unwrap_or(&id).to_owned();
        let entry = v
            .legal
            .iter()
            .find(|m| m.id == id)
            .expect("the decider picks a legal move")
            .clone();
        match entry.mv {
            Move::Play { card } | Move::Discard { card } => {
                // A fresh hand deals in (06 §5.6); the card takes presses once it has landed.
                landed(&mut rt, &format!("hand-{card}"));
                click(&mut rt, &format!("hand-{card}"));
                assert_eq!(selected(&rt.state), Some(card), "a press selects");
                click(&mut rt, "hand-play");
            }
            _ => click(&mut rt, &format!("bid-{id}")),
        }
        moves += 1;
        let before = v.v;
        assert!(
            pump(&mut rt, &replies, Duration::from_secs(10), |s| {
                view(s).is_some_and(|w| w.v > before)
                    && s.table.as_ref().is_some_and(|t| !t.store.submitting())
            }),
            "move {moves} ({id}) was taken"
        );
    }
    let v = view(&rt.state).expect("a view").clone();
    assert_eq!(v.status, GameStatus::Complete);
    assert!(v.score.iter().any(|&p| p >= 10), "someone reached ten");
    assert!(moves > 20, "a match is many moves: {moves}");
    assert!(continues > 0, "tricks were reviewed");
    // The last sweep and the score land, then the match-end panel offers Play again.
    settle(&mut rt, 3.0);
    assert!(
        rt.find("match-end-again").is_some(),
        "Play again is offered"
    );
    assert!(rt.find("hand-play").is_none());
    click(&mut rt, "match-end-again");
    assert_eq!(rt.state.route, Route::NewGame, "Play again goes to /new");
}

#[test]
fn an_illegal_card_is_refused_and_the_selection_stays() {
    let mut state = AppState::default();
    scene(&mut state, "table-play");
    let mut rt = runtime(state);
    settle(&mut rt, 0.5);
    let v = view(&rt.state).expect("a view").clone();
    let legal = v
        .hand
        .iter()
        .copied()
        .find(|&c| copy::card_move(&v, c).is_some())
        .expect("a legal card");
    let illegal = v
        .hand
        .iter()
        .copied()
        .find(|&c| copy::card_move(&v, c).is_none())
        .expect("the scene holds a card that cannot follow");

    click(&mut rt, &format!("hand-{legal}"));
    assert_eq!(selected(&rt.state), Some(legal));

    click(&mut rt, &format!("hand-{illegal}"));
    assert_eq!(
        selected(&rt.state),
        Some(legal),
        "the selection is unchanged"
    );
    let shown = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.present.illegal.as_ref())
        .map(|(m, _)| m.clone());
    assert_eq!(shown, Some(copy::illegal_reason(&v, illegal)));

    // A double press on it does not submit either.
    rt.update(move |s, _| s.play_card(illegal));
    rt.frame(DT);
    assert!(
        !rt.state.table.as_ref().expect("table").store.submitting(),
        "nothing was sent"
    );

    // The message lasts 3.2 s, then the instruction returns.
    settle(&mut rt, 3.6);
    assert!(
        rt.state
            .table
            .as_ref()
            .is_some_and(|t| t.present.illegal.is_none()),
        "the message cleared"
    );
}

#[test]
fn number_keys_and_arrows_reach_the_rack() {
    let mut state = AppState::default();
    scene(&mut state, "table-play");
    let mut rt = runtime(state);
    settle(&mut rt, 0.5);
    let v = view(&rt.state).expect("a view").clone();
    let slots: Vec<Card> = rt
        .state
        .table
        .as_ref()
        .map(|t| t.present.rack.present().collect())
        .unwrap_or_default();
    let (first, last) = (slots[0], slots[slots.len() - 1]);
    assert_eq!(focused(&rt), None);
    // With nothing focused, Right lands on the first card and Left on the last.
    press(&mut rt, Key::Right);
    assert_eq!(focused(&rt), Some(format!("hand-{first}")));
    assert_eq!(selected(&rt.state), None, "focus alone never selects");
    rt.update(|_, tx| tx.blur());
    rt.frame(DT);
    press(&mut rt, Key::Left);
    assert_eq!(focused(&rt), Some(format!("hand-{last}")));
    // A number picks that slot and moves focus to it.
    press(&mut rt, Key::Text("1".to_owned()));
    assert_eq!(focused(&rt), Some(format!("hand-{first}")));
    if copy::card_move(&v, first).is_some() {
        assert_eq!(selected(&rt.state), Some(first));
    } else {
        assert_eq!(selected(&rt.state), None);
        assert!(
            rt.state
                .table
                .as_ref()
                .is_some_and(|t| t.present.illegal.is_some())
        );
    }
}

#[test]
fn a_reviewed_trick_waits_for_continue() {
    let mut state = AppState::default();
    scene(&mut state, "table-trick-won");
    let mut rt = runtime(state);
    settle(&mut rt, 3.0);
    let v = view(&rt.state).expect("a view").clone();
    assert_eq!(v.awaiting_trick_review, Some(true));
    assert_eq!(v.trick.plays.len(), 4, "the cards wait on the felt");
    assert!(rt.find("trick-continue").is_some());
    let collect = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.present.collect.clone());
    assert!(
        collect.is_some_and(|c| c.moved == 0 && !c.continuing),
        "nothing gathers before Continue"
    );
    click(&mut rt, "trick-continue");
    settle(&mut rt, 2.0);
    let collect = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.present.collect.clone())
        .expect("the gather");
    assert!(collect.continuing);
    assert_eq!(collect.moved, 4, "every card went to the winner's stack");
}

#[test]
fn reduce_motion_quiets_the_table() {
    // Game over: the match-end panel shows, the confetti does not.
    for (reduce, confetti) in [(false, true), (true, false)] {
        let mut state = AppState::default();
        state.prefs.reduce_motion = reduce;
        scene(&mut state, "game-over");
        let mut rt = runtime(state);
        let mut rained = false;
        // The confetti waits for the panel to land (09 §4.10), so watch the whole moment.
        for _ in 0..(3.0 / DT).ceil() as usize {
            rt.frame(DT);
            rained |= rt.find("confetti-canvas").is_some();
        }
        let banner = rt
            .state
            .table
            .as_ref()
            .and_then(|t| t.present.banner.clone());
        let banner = banner.expect("the match ends with a banner");
        assert!(rt.find("match-end-panel").is_some(), "the panel shows");
        assert_eq!(
            rained,
            confetti && banner.confetti,
            "confetti only with motion (reduce = {reduce})"
        );
    }

    // A reviewed trick gathers at once under reduce motion.
    let mut state = AppState::default();
    state.prefs.reduce_motion = true;
    scene(&mut state, "table-trick-won");
    let mut rt = runtime(state);
    settle(&mut rt, 0.2);
    click(&mut rt, "trick-continue");
    rt.frame(DT);
    rt.frame(DT);
    let collect = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.present.collect.clone())
        .expect("the gather");
    assert!(collect.done(), "no stagger under reduce motion");
}

#[test]
fn the_table_fits_the_smallest_window() {
    for name in ["table-play", "table-bid", "game-over"] {
        let mut state = AppState::default();
        state.wait_for_art = true;
        scene(&mut state, name);
        let mut rt = App::new(spec())
            .init(move || state)
            .headless_runtime((720.0, 520.0), Env::new(false, 1.0));
        rt.frame(0.0);
        settle(&mut rt, 0.5);
        let window = [0.0, 0.0, 720.0, 520.0];
        for key in [
            "table-header",
            "table-rules-open",
            "table-settings-open",
            "felt",
            "trick",
            "rack",
            "hand-controls",
        ] {
            let r = rect(&rt, key).unwrap_or_else(|| panic!("{name}: {key} is drawn"));
            assert!(inside(r, window), "{name}: {key} {r:?} sits inside 720x520");
        }
    }
}

#[test]
fn trick_cards_read_at_the_default_window() {
    let mut state = AppState::default();
    scene(&mut state, "table-play");
    let mut rt = sized(state, (1280.0, 820.0));
    settle(&mut rt, 0.5);
    let cards: Vec<_> = rt
        .elements()
        .into_iter()
        .filter(|e| {
            !e.exiting
                && e.key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("trick-") && k.split('-').count() == 4)
        })
        .collect();
    assert!(cards.len() >= 2, "the scene has cards on the felt");
    for c in cards {
        assert!(
            c.rect[2] >= 100.0,
            "{:?} is {} wide at 1280x820",
            c.key,
            c.rect[2]
        );
    }
}

#[test]
fn seat_rows_stay_inside_the_rim() {
    for size in [(1280.0, 820.0), (720.0, 820.0)] {
        for name in ["table-play", "table-bid", "table-trick-won"] {
            let mut state = AppState::default();
            scene(&mut state, name);
            let mut rt = sized(state, size);
            settle(&mut rt, 0.5);
            let rim = rect(&rt, "felt-rim").unwrap_or_else(|| panic!("{name}: the rim"));
            let mut rows = 0;
            for i in 0..4 {
                let Some(row) = rect(&rt, &format!("seat-{i}-row")) else {
                    continue;
                };
                rows += 1;
                assert!(
                    inside(row, rim),
                    "{name} at {size:?}: seat-{i}-row {row:?} is inside the rim {rim:?}"
                );
                if let Some(block) = rect(&rt, &format!("seat-{i}")) {
                    assert!(
                        inside(row, block),
                        "{name} at {size:?}: seat-{i}-row {row:?} fits its block {block:?}"
                    );
                }
            }
            assert!(rows >= 3, "{name}: the three other seats show");
        }
    }
}

#[test]
fn an_auto_trick_gathers_after_its_pause() {
    let (sync, _) = scenes::drive_core("auto-gather", false, |s| {
        s.view.trick.winner_seat.is_some() && s.view.awaiting_trick_review != Some(true)
    })
    .expect("a won trick");
    let mut state = AppState::default();
    state.route = Route::Table;
    state.table = Some(TableScreen::showing(sync));
    let mut rt = runtime(state);
    settle(&mut rt, 0.5);
    let c = collect_of(&rt.state).expect("the trick waits to gather");
    assert!(!c.review, "no review: the gather runs itself");
    assert_eq!(c.moved, 0, "nothing moves before COLLECT_AFTER");
    assert!(rt.find("trick-continue").is_none());
    settle(&mut rt, timing::COLLECT_AFTER + 1.5);
    let c = collect_of(&rt.state).expect("the gather");
    assert!(c.done(), "every card went to the winner's stack");
}

#[test]
fn only_the_continue_failing_puts_the_trick_back() {
    let mut state = AppState::default();
    scene(&mut state, "table-trick-won");
    let mut rt = runtime(state);
    settle(&mut rt, 1.0);
    click(&mut rt, "trick-continue");
    settle(&mut rt, 2.0);
    let c = collect_of(&rt.state).expect("the gather");
    assert!(c.continuing && c.sent, "Continue went out");
    // Some other review request failing (the settings toggle) leaves the gather alone.
    rt.update(|s, _| {
        if let Some(t) = s.table.as_mut() {
            let mut fx = StoreEffects::default();
            fx.events.push(StoreEvent::ReviewSettled {
                id: RequestId(u64::MAX),
                ok: false,
            });
            t.apply(fx, None);
        }
    });
    rt.frame(DT);
    assert!(collect_of(&rt.state).is_some_and(|c| c.continuing));
    // The Continue itself going unanswered puts the cards back for another try.
    rt.update(|s, _| {
        if let Some(t) = s.table.as_mut() {
            let late = euchre_desktop::net::now_ms() + ACTION_TIMEOUT_MS + 1;
            let fx = t.store.tick(late, true);
            t.apply(fx, None);
        }
    });
    rt.frame(DT);
    let c = collect_of(&rt.state).expect("the trick");
    assert!(!c.continuing && c.moved == 0, "the trick is back: {c:?}");
}

#[test]
fn header_tools_open_and_close_the_panels() {
    let mut state = AppState::default();
    scene(&mut state, "table-play");
    let mut rt = sized(state, (1280.0, 820.0));
    settle(&mut rt, 0.3);
    assert!(rt.find("table-settings").is_none());

    click(&mut rt, "table-settings-open");
    settle(&mut rt, 0.5);
    let rail = rect(&rt, "table-settings").expect("the settings rail");
    assert!(inside(rail, [0.0, 0.0, 1280.0, 820.0]), "{rail:?}");
    assert!(rt.find("settings-review").is_some());
    assert!(rt.find("settings-talk").is_some());

    click(&mut rt, "settings-review-hands");
    settle(&mut rt, 0.5);
    assert!(
        rt.find("review-dialog").is_some(),
        "Review hands opens the dialog"
    );
    press(&mut rt, Key::Escape);
    settle(&mut rt, 0.5);
    assert!(
        rt.find("review-dialog").is_none(),
        "Escape closes the top panel"
    );
    assert!(rt.find("table-settings").is_some(), "...and only that one");
    press(&mut rt, Key::Escape);
    settle(&mut rt, 0.5);
    assert!(rt.find("table-settings").is_none());

    click(&mut rt, "table-rules-open");
    settle(&mut rt, 0.5);
    assert!(rt.find("rules-dialog").is_some());
    assert!(rt.find("rules-now").is_some(), "the phase card leads");
    click(&mut rt, "rules-close");
    settle(&mut rt, 0.5);
    assert!(rt.find("rules-dialog").is_none());
    assert_eq!(
        rt.state.route,
        Route::Table,
        "Escape and close never leave the table"
    );
}

#[test]
fn the_narrow_settings_rail_lies_over_the_table() {
    let mut state = AppState::default();
    scene(&mut state, "table-play");
    let mut rt = sized(state, (720.0, 520.0));
    settle(&mut rt, 0.3);
    click(&mut rt, "table-settings-open");
    settle(&mut rt, 0.5);
    let rail = rect(&rt, "table-settings").expect("the rail");
    assert!(inside(rail, [0.0, 0.0, 720.0, 520.0]), "{rail:?}");
    press(&mut rt, Key::Escape);
    settle(&mut rt, 0.5);
    assert!(rt.find("table-settings").is_none());
}

#[test]
fn the_bid_copy_stays_in_its_box_beside_your_seat() {
    for size in [
        (1280.0, 820.0),
        (1000.0, 700.0),
        (720.0, 820.0),
        (1280.0, 700.0),
    ] {
        for name in ["table-bid", "table-deal"] {
            let mut state = AppState::default();
            scene(&mut state, name);
            let mut rt = sized(state, size);
            settle(&mut rt, 0.5);
            let area = rect(&rt, "trick").expect("the bid area");
            let words = rect(&rt, "bid-copy").expect("the bid copy");
            assert!(
                inside(words, area),
                "{name} at {size:?}: bid-copy {words:?} inside {area:?}"
            );
            if let Some(you) = rect(&rt, "seat-0") {
                let apart = words[0] + words[2] <= you[0] + 0.5
                    || you[0] + you[2] <= words[0] + 0.5
                    || words[1] + words[3] <= you[1] + 0.5
                    || you[1] + you[3] <= words[1] + 0.5;
                assert!(
                    apart,
                    "{name} at {size:?}: bid-copy {words:?} clear of your seat {you:?}"
                );
            }
        }
    }
}

/// 09 §4.10: while the match-end panel shows, it carries the one Play again; the hand area
/// drops its own, and the felt's result line goes quiet.
#[test]
fn play_again_works_under_the_match_end_panel() {
    for name in ["game-over", "game-over-win", "game-over-loss"] {
        let mut state = AppState::default();
        scene(&mut state, name);
        let mut rt = runtime(state);
        settle(&mut rt, 0.5);
        assert!(
            rt.find("match-end-scrim").is_some(),
            "{name}: the scrim shows"
        );
        assert!(
            rt.find("match-end-panel").is_some(),
            "{name}: the panel shows"
        );
        assert!(rt.find("hand-again").is_none(), "{name}: one Play again");
        assert!(
            rt.find("trick-result-text").is_none(),
            "{name}: the result line is quiet under the panel"
        );
        click(&mut rt, "match-end-again");
        assert_eq!(
            rt.state.route,
            Route::NewGame,
            "{name}: Play again opens New game"
        );
    }
}
