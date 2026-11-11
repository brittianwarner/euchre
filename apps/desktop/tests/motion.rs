//! The motion harness (09 §6.0 item 7, §6.2): helpers that read the presented picture over
//! time, run the binary for its frame-time line, and the motion checks built on them.
//!
//! - [`presented_at`]: an element's drawn rect and opacity at a time on the frame clock.
//! - [`trace_idle`]: which keyed elements change over a stretch of frames.
//! - [`stats`]: the `rune: animating frames N (cpu p50 a p95 b ms)` line of a headless run.

use std::collections::BTreeMap;
use std::process::Command;

use euchre_desktop::app::{AppState, Route, RulesMsg, TableScreen, TableStage, Then};
use euchre_desktop::net::ConnEvent;
use euchre_desktop::net::bridge::Reply;
use euchre_desktop::screens::table::choreo::{Director, Feel, PlayFlightSpec, ShotKey};
use euchre_desktop::screens::table::present::Presentation;
use euchre_desktop::screens::table::{copy, tour};
use euchre_desktop::theme::felt::{size, timing};
use euchre_desktop::theme::motion;
use euchre_desktop::{scenes, spec, theme};
use euchre_protocol::{Card, GamePhase, PublicGameView, Seat, Step, SyncEvent, TableEvent};
use rune::{App, ElementInfo, Env, Key, KeyEvent, Mods, Runtime};

const DT: f64 = 1.0 / 60.0;

/// Elements allowed to keep changing at rest (09 §6.6 idle check; §2.7 `IDLE_OK`): the turn
/// halo breathes on your turn, and nothing else moves.
const IDLE_OK: &[&str] = &["turn-halo"];

/// The motion scenes (09 §6.6) and Lane E's.
const MOTION_SCENES: &[&str] = &[
    "table-deal-anim",
    "table-upcard-flip",
    "table-play-opp",
    "table-play-you",
    "table-trick-win",
    "table-trick-win-reduced",
    "table-pickup",
    "table-trump-call",
    "table-illegal-shake",
    "table-hover",
    "table-hand-result",
    "table-euchre-stamp",
    "game-over-win",
    "game-over-loss",
    "table-rules-open",
    "route-home-new",
    "table-reconnect",
    "table-hint",
    "table-tour",
    "table-banter",
];

/// The scenes whose moment is a sync landing on the second frame.
const SYNC_SCENES: &[&str] = &[
    "table-deal-anim",
    "table-upcard-flip",
    "table-play-opp",
    "table-play-you",
    "table-trick-win",
    "table-trick-win-reduced",
    "table-pickup",
    "table-trump-call",
    "table-hand-result",
    "table-euchre-stamp",
    "game-over-win",
    "game-over-loss",
    "table-reconnect",
];

fn runtime(state: AppState) -> Runtime<AppState> {
    let mut state = state;
    state.wait_for_art = true;
    let mut rt = App::new(spec())
        .init(move || state)
        .headless_runtime(theme::window::SIZE, Env::new(false, 1.0));
    rt.frame(0.0);
    rt
}

fn scene(state: &mut AppState, name: &str) {
    let (_, apply) = scenes::SCENES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("scene {name} exists"));
    apply(state);
}

fn scene_runtime(name: &str) -> Runtime<AppState> {
    let mut state = AppState::default();
    scene(&mut state, name);
    runtime(state)
}

/// Runs frames until the poll clock reads `t` (seconds since the runtime's first frame), then
/// returns `key`'s drawn rect `[x, y, w, h]` and effective opacity, exiting or not.
fn presented_at(rt: &mut Runtime<AppState>, key: &str, t: f64) -> Option<([f32; 4], f32)> {
    for _ in 0..((t / DT).ceil() as usize + 2) {
        if rt.state.clock >= t - DT * 0.5 {
            break;
        }
        rt.frame(DT);
    }
    rt.presented(key)
}

/// One keyed element's presentation, for [`trace_idle`].
type Shown = ([f32; 4], f32, f32);

fn shown(rt: &Runtime<AppState>) -> BTreeMap<String, Shown> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| Some((e.key?, (e.rect, e.opacity, e.scale))))
        .collect()
}

/// Runs `secs` of frames and returns every key whose rect, opacity or scale moved (by more than
/// 0.01) or that came or went, sorted.
fn trace_idle(rt: &mut Runtime<AppState>, secs: f64) -> Vec<String> {
    movers_where(rt, secs, true)
}

/// Runs `secs` of frames and returns every key whose rect or scale moved (and, when
/// `opacity`, whose opacity changed) by more than 0.01, sorted. Keys that come or go count.
fn movers_where(rt: &mut Runtime<AppState>, secs: f64, opacity: bool) -> Vec<String> {
    const EPS: f32 = 0.01;
    let mut before = shown(rt);
    let mut changed = std::collections::BTreeSet::new();
    for _ in 0..(secs / DT).ceil() as usize {
        rt.frame(DT);
        let now = shown(rt);
        for (key, a) in &now {
            let moved = before.get(key).is_none_or(|b| {
                a.0.iter().zip(b.0).any(|(x, y)| (x - y).abs() > EPS)
                    || (opacity && (a.1 - b.1).abs() > EPS)
                    || (a.2 - b.2).abs() > EPS
            });
            if moved {
                changed.insert(key.clone());
            }
        }
        changed.extend(before.keys().filter(|k| !now.contains_key(*k)).cloned());
        before = now;
    }
    changed.into_iter().collect()
}

/// [`trace_idle`] without the keys allowed to move at rest.
fn idle_movers(rt: &mut Runtime<AppState>, secs: f64) -> Vec<String> {
    trace_idle(rt, secs)
        .into_iter()
        .filter(|k| !IDLE_OK.contains(&k.as_str()))
        .collect()
}

/// A headless run's frame-time line.
#[derive(Debug)]
struct Stats {
    animating: u32,
    p50: f32,
    p95: f32,
}

/// Runs the `euchre` binary with `args` (plus `--size 1280x800 --scale 1`) and reads its
/// `rune: animating frames N (cpu p50 a p95 b ms)` stderr line.
fn stats(args: &[&str]) -> Stats {
    let out = Command::new(env!("CARGO_BIN_EXE_euchre"))
        .args(args)
        .args(["--size", "1280x800", "--scale", "1"])
        .output()
        .expect("the euchre binary runs");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "euchre {args:?} failed: {err}");
    let line = err
        .lines()
        .find_map(|l| l.split_once("animating frames ").map(|(_, rest)| rest))
        .unwrap_or_else(|| panic!("no stats line in: {err}"));
    // "N (cpu p50 a p95 b ms), ..."
    let words: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
        .filter(|w| !w.is_empty())
        .collect();
    let num = |after: &str| -> f32 {
        let at = words
            .iter()
            .position(|w| *w == after)
            .unwrap_or_else(|| panic!("no {after} in {line}"));
        words[at + 1].parse().expect("a number")
    };
    Stats {
        animating: words[0].parse().expect("a frame count"),
        p50: num("p50"),
        p95: num("p95"),
    }
}

fn click(rt: &mut Runtime<AppState>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(DT);
}

fn escape(rt: &mut Runtime<AppState>) {
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.frame(DT);
}

fn view_v(rt: &Runtime<AppState>) -> Option<u64> {
    rt.state
        .table
        .as_ref()
        .and_then(|t| t.store.display_view())
        .map(|v| v.v)
}

/// The first rack card the view lets the player play.
fn legal_card(rt: &Runtime<AppState>) -> Card {
    let view = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.store.display_view())
        .expect("a view");
    view.hand
        .iter()
        .copied()
        .find(|&c| copy::card_move(view, c).is_some())
        .expect("a playable card")
}

#[test]
fn every_motion_scene_reaches_its_moment() {
    for name in MOTION_SCENES {
        let mut rt = scene_runtime(name);
        if *name == "route-home-new" {
            assert_eq!(rt.state.route, Route::Home);
            continue;
        }
        let table = rt.state.table.as_ref().expect("a table");
        assert_eq!(table.stage, TableStage::Open, "{name} found its moment");
        assert!(rt.state.prefs.tour_seen || *name == "table-tour", "{name}");
        let before = view_v(&rt);
        presented_at(&mut rt, "screen-table", 0.1).expect("the table shows");
        let table = rt.state.table.as_ref().expect("a table");
        if SYNC_SCENES.contains(name) {
            assert!(view_v(&rt) > before, "{name}: the moment's sync landed");
        }
        match *name {
            "table-illegal-shake" => assert!(table.present.illegal.is_some()),
            "table-rules-open" => assert!(table.present.hooks.rules_open),
            "table-tour" => assert_eq!(table.present.hooks.tour_step, Some(0)),
            "table-trick-win-reduced" => assert!(rt.state.motion_reduced.get()),
            _ => {}
        }
        // Nothing replays: the sound and announcer queues drain every frame.
        assert!(table.present.hooks.sound.is_empty(), "{name}");
        assert!(table.present.hooks.announce.is_empty(), "{name}");
    }
}

#[test]
fn the_fx_layer_covers_the_table_and_passes_clicks() {
    let mut rt = scene_runtime("table-play");
    let (rect, _) = rt.presented("fx-layer").expect("the fx layer is mounted");
    let (w, h) = theme::window::SIZE;
    assert_eq!(rect, [0.0, 0.0, w, h], "it covers the window");
    let card = legal_card(&rt);
    click(&mut rt, &format!("hand-{card}"));
    let selected = rt.state.table.as_ref().and_then(|t| t.selected);
    assert_eq!(selected, Some(card), "the click reached the rack under it");
}

/// A-fx-layer-passes-clicks with a live burst (09 §2.10): the sparks and the confetti cover
/// the whole window, and a click still reaches what is under them.
#[test]
fn a_click_passes_through_a_live_burst() {
    let mut rt = scene_runtime("table-euchre-stamp");
    let mut live = false;
    while rt.state.clock < 5.0 && !live {
        rt.frame(DT);
        live = !keys_where(&rt, |k| k.starts_with("sparks-")).is_empty();
    }
    assert!(live, "the stamp throws its sparks");
    click(&mut rt, "table-settings-open");
    let open = rt
        .state
        .table
        .as_ref()
        .is_some_and(|t| t.present.hooks.settings_open);
    assert!(open, "the click opened the settings under the sparks");

    let mut rt = scene_runtime("game-over-win");
    let mut live = false;
    while rt.state.clock < 5.0 && !live {
        rt.frame(DT);
        live = element(&rt, "confetti-canvas").is_some();
    }
    assert!(live, "the win throws its confetti");
    click(&mut rt, "match-end-again");
    assert_eq!(
        rt.state.route,
        Route::NewGame,
        "the click reached Play again"
    );
}

/// 09 §4.8: the result line swaps its words without printing two lines over each other. The
/// new line waits for the old one to fade (`LINE_SWAP_GAP`), so through a trick's win no two
/// `result-*` lines are above 0.2 opacity in any frame, and the line does change.
#[test]
fn the_result_line_never_overprints() {
    for name in [
        "table-trick-win",
        "table-trick-win-reduced",
        "table-play-opp",
    ] {
        let mut rt = scene_runtime(name);
        let mut seen = std::collections::BTreeSet::new();
        while rt.state.clock < 3.0 {
            rt.frame(DT);
            let lines: Vec<(String, f32)> = rt
                .elements()
                .into_iter()
                .filter_map(|e| {
                    let key = e.key.filter(|k| k.starts_with("result-"))?;
                    Some((key, e.opacity))
                })
                .collect();
            seen.extend(lines.iter().map(|(k, _)| k.clone()));
            let bright: Vec<_> = lines.iter().filter(|(_, o)| *o > 0.2).collect();
            assert!(
                bright.len() <= 1,
                "{name} at {:.3} s prints two result lines: {bright:?}",
                rt.state.clock
            );
        }
        if name != "table-play-opp" {
            assert!(
                seen.len() >= 2,
                "{name}: the line changes on the win: {seen:?}"
            );
        }
    }
}

/// 09 §4.5: one turn marker rings the seat on turn and travels to the next on its spring
/// (it passes through points between the two seats), and the halo breathes only on your
/// turn. Reduced: the halo holds still at its still alpha.
#[test]
fn the_turn_marker_travels_and_the_halo_is_yours() {
    let mut rt = scene_runtime("table-play-opp");
    let mut xs: Vec<f32> = Vec::new();
    let mut halo_off_your_turn = false;
    while rt.state.clock < 1.2 {
        rt.frame(DT);
        let yours = rt
            .state
            .table
            .as_ref()
            .and_then(|t| t.store.display_view())
            .is_some_and(|v| v.turn_seat == Some(v.you));
        if let Some(([x, ..], _)) = rt.presented("turn-marker") {
            xs.push(x);
        }
        halo_off_your_turn |= !yours && element(&rt, "turn-halo").is_some();
    }
    assert!(!halo_off_your_turn, "the halo shows on your turn only");
    let (first, last) = (xs[0], *xs.last().expect("the marker shows"));
    assert!(
        (first - last).abs() > 100.0,
        "the marker moved: {first} to {last}"
    );
    let between = xs
        .iter()
        .filter(|&&x| (x - first) * (x - last) < 0.0 && (x - last).abs() > 2.0)
        .count();
    assert!(between >= 3, "the marker travels, not jumps: {xs:?}");
    assert!(
        element(&rt, "turn-halo").is_some(),
        "your turn: the halo shows"
    );

    let mut state = AppState::default();
    scene(&mut state, "table-play");
    state.prefs.reduce_motion = true;
    let mut rt = runtime(state);
    while rt.state.clock < 1.0 {
        rt.frame(DT);
        if let Some((_, o)) = rt.presented("turn-halo") {
            assert!(
                (o - motion::TURN_HALO.still).abs() < 0.02,
                "reduced: the halo holds still: {o}"
            );
        }
    }
    assert!(
        rt.presented("turn-halo").is_some(),
        "reduced: the halo shows"
    );
}

/// 09 §4.6: the picked-up card flies at the up card's size and only shrinks to a back in the
/// last `PICKUP_SHRINK` of the flight.
#[test]
fn the_pickup_flies_at_up_card_size_and_shrinks_last() {
    let mut rt = scene_runtime("table-pickup");
    let hand = shown_view(&rt).hand_no;
    // The up card's laid-out width (its drawn rect also carries its lift).
    let up_w = element(&rt, &format!("up-{hand}")).map_or(0.0, |e| e.target[2]);
    // (seconds into the pickup shot, drawn width)
    let mut widths: Vec<(f32, f32)> = Vec::new();
    while rt.state.clock < 2.0 {
        rt.frame(DT);
        let at = director(&rt).elapsed(&ShotKey::pickup(hand), rt.state.clock);
        if let (Some(t), Some(e)) = (at, element(&rt, &format!("back-pickup-{hand}"))) {
            widths.push((t, e.rect[2]));
        }
    }
    assert!(up_w > 40.0, "the up card showed: {up_w}");
    assert!(!widths.is_empty(), "the pickup flew to a fan");
    let hold = motion::PICKUP_LAND * (1.0 - motion::PICKUP_SHRINK);
    for &(t, w) in widths.iter().filter(|(t, _)| *t < hold - 0.02) {
        assert!(
            (w - up_w).abs() < 2.0,
            "at {t:.2}s the card is still up-card size: {w} vs {up_w}"
        );
    }
    let back_w = size::FAN[0];
    for &(t, w) in widths
        .iter()
        .filter(|(t, _)| *t > motion::PICKUP_LAND + 0.02)
    {
        assert!(
            (w - back_w).abs() < 1.0,
            "at {t:.2}s the card is a back: {w} vs {back_w}"
        );
    }
    assert!(
        widths.iter().any(|&(t, w)| t > hold
            && t < motion::PICKUP_LAND
            && w < up_w - 2.0
            && w > back_w + 1.0),
        "it shrinks inside the last stretch: {widths:?}"
    );
}

#[test]
fn the_reconnect_scene_lands_in_place() {
    let mut rt = scene_runtime("table-reconnect");
    let card = legal_card(&rt);
    let key = format!("hand-{card}");
    let at_0 = presented_at(&mut rt, &key, 0.0).expect("the card shows");
    assert_eq!(idle_movers(&mut rt, 1.0), Vec::<String>::new());
    assert!(view_v(&rt).is_some());
    let at_1 = presented_at(&mut rt, &key, 1.0).expect("the card shows");
    assert_eq!(at_0, at_1);
}

// ---------------------------------------------------------------------------------------------
// Lane A (09 §6.2): the Director's runtime checks.
// ---------------------------------------------------------------------------------------------

fn present(rt: &Runtime<AppState>) -> &Presentation {
    &rt.state.table.as_ref().expect("a table").present
}

fn director(rt: &Runtime<AppState>) -> &Director {
    &present(rt).director
}

fn shown_view(rt: &Runtime<AppState>) -> &PublicGameView {
    rt.state
        .table
        .as_ref()
        .and_then(|t| t.store.display_view())
        .expect("a view")
}

/// Runs frames until the poll clock reads `t`.
fn run_to(rt: &mut Runtime<AppState>, t: f64) {
    for _ in 0..((t / DT).ceil() as usize + 2) {
        if rt.state.clock >= t - DT * 0.5 {
            break;
        }
        rt.frame(DT);
    }
}

/// The element keyed `key`, if it is mounted and not leaving.
fn element(rt: &Runtime<AppState>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

/// Every mounted, non-leaving key that `test` accepts.
fn keys_where(rt: &Runtime<AppState>, test: impl Fn(&str) -> bool) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.key)
        .filter(|k| test(k))
        .collect()
}

fn fly_keys(rt: &Runtime<AppState>) -> Vec<String> {
    keys_where(rt, |k| k.starts_with("fly-"))
}

fn centre([x, y, w, h]: [f32; 4]) -> [f32; 2] {
    [x + w / 2.0, y + h / 2.0]
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// At rest: still, and centred where its layout puts it (a turned card's drawn rect is its
/// turned bounds, so the centres are what match).
fn at_rest(e: &ElementInfo) -> bool {
    !e.moving && dist(centre(e.rect), centre(e.target)) <= 0.5
}

/// The first Full play in the air: its shot key, what it is, and when it started.
fn first_flyer(rt: &Runtime<AppState>) -> Option<(ShotKey, PlayFlightSpec, f64)> {
    let d = director(rt);
    let (key, spec, _) = d.flyers(rt.state.clock).next()?;
    Some((key.clone(), *spec, d.started(key)?))
}

/// Frames until a play is in the air (at most a quarter second).
fn until_flyer(rt: &mut Runtime<AppState>) -> (ShotKey, PlayFlightSpec, f64) {
    for _ in 0..15 {
        if let Some(f) = first_flyer(rt) {
            return f;
        }
        rt.frame(DT);
    }
    panic!("no play took off");
}

fn card_played(sync: &SyncEvent) -> Option<Seat> {
    sync.steps.iter().find_map(|st| match st {
        Step::CardPlayed { seat, .. } => Some(*seat),
        _ => None,
    })
}

/// A table showing `before` that lands `then` on the next frame.
fn moment_runtime(before: SyncEvent, then: SyncEvent, reduce: bool) -> Runtime<AppState> {
    let mut state = AppState::default();
    state.route = Route::Table;
    state.prefs.tour_seen = true;
    state.prefs.reduce_motion = reduce;
    state.table = Some(TableScreen::showing_then(
        before,
        Then::Sync(Box::new(then)),
    ));
    runtime(state)
}

/// An opponent's card that leaves you to play next, mid-hand: `(before, found)`.
fn opponent_then_you() -> (SyncEvent, SyncEvent) {
    scenes::drive_pair(scenes::DRIVEN_SEED, false, |s| {
        s.view.phase == GamePhase::TrickPlay
            && s.view.trick.index >= 1
            && s.view.turn_seat == Some(s.view.you)
            && !s.view.legal.is_empty()
            && card_played(s).is_some_and(|seat| seat.index() % 2 != s.view.you.index() % 2)
    })
    .expect("an opponent plays and you are next")
}

/// A whole trick (index 1 or later) as four syncs in a row, one card each, with you second
/// or third (so the last card is not yours), and the sync before its lead.
fn four_plays() -> (SyncEvent, Vec<SyncEvent>) {
    let seen = std::cell::RefCell::new(Vec::<SyncEvent>::new());
    scenes::drive(scenes::DRIVEN_SEED, false, |s| {
        let mut seen = seen.borrow_mut();
        seen.push(s.clone());
        let n = seen.len();
        if n < 5 {
            return false;
        }
        let four = &seen[n - 4..];
        let seats: Vec<Seat> = four.iter().filter_map(card_played).collect();
        let one_each = four.iter().all(|x| {
            x.steps
                .iter()
                .filter(|st| matches!(st, Step::CardPlayed { .. }))
                .count()
                == 1
        });
        let you = s.view.you;
        seats.len() == 4
            && one_each
            && s.view.trick.index >= 1
            && s.view.trick.winner_seat.is_some()
            && four.iter().all(|x| x.view.hand_no == s.view.hand_no)
            && four[0].view.trick.plays.len() == 1
            && (seats[1] == you || seats[2] == you)
    })
    .expect("a trick of four syncs");
    let seen = seen.into_inner();
    let n = seen.len();
    (seen[n - 5].clone(), seen[n - 4..].to_vec())
}

/// A-layout-rects (09 §6.2, §2.10): the first frame of an opponent's card in the air draws it
/// on the centre of the fan it leaves.
#[test]
fn a_layout_rects_the_flyer_leaves_from_its_fan() {
    let mut rt = scene_runtime("table-play-opp");
    let (key, spec, _) = until_flyer(&mut rt);
    let (fly, _) = rt
        .presented(&format!("fly-{key}"))
        .expect("the flyer is drawn in its first frame");
    let (fan, _) = rt
        .presented(&format!("seat-{}-fan", spec.seat.index()))
        .expect("the fan");
    let off = dist(centre(fly), centre(fan));
    assert!(off <= 2.0, "the flyer starts {off} pt from its fan");
}

/// A-play-arcs (09 §4.3): the card rides above the straight line, then hands over to its trick
/// slot, which is at rest and opaque.
#[test]
fn a_play_arcs_above_the_chord_and_lands_in_its_slot() {
    let mut rt = scene_runtime("table-play-opp");
    let (key, spec, t0) = until_flyer(&mut rt);
    let fly = format!("fly-{key}");
    let (r0, _) = rt.presented(&fly).expect("the flyer");
    let from = centre(r0);
    run_to(&mut rt, t0 + 0.275);
    let (mid, _) = rt.presented(&fly).expect("still in the air");
    let mid = centre(mid);
    run_to(&mut rt, t0 + 1.0);
    assert_eq!(
        fly_keys(&rt),
        Vec::<String>::new(),
        "the flyer has handed over"
    );
    let slot = format!(
        "trick-{}-{}-{}",
        spec.hand_no,
        spec.index,
        spec.seat.index()
    );
    let e = element(&rt, &slot).expect("the trick card");
    assert!(at_rest(&e) && e.opacity >= 0.99, "{slot} rests: {e:?}");
    let to = centre(e.rect);
    let chord = dist(from, to);
    let arc = motion::PLAY_ARC_OPP.min(motion::PLAY_ARC_SHARE * chord);
    // The chord's normal that points up the screen.
    let d = [(to[0] - from[0]) / chord, (to[1] - from[1]) / chord];
    let n = if d[0] >= 0.0 {
        [d[1], -d[0]]
    } else {
        [-d[1], d[0]]
    };
    let above = (mid[0] - from[0]) * n[0] + (mid[1] - from[1]) * n[1];
    assert!(
        above >= 0.3 * arc,
        "{above} pt above the chord at 0.275 s (arc {arc})"
    );
}

/// A-ring-waits (09 §4.4): the winner ring waits for the last card's touchdown.
#[test]
fn a_ring_waits_for_the_last_touchdown() {
    let mut rt = scene_runtime("table-trick-win");
    run_to(&mut rt, 0.3);
    let view = shown_view(&rt).clone();
    assert!(view.trick.winner_seat.is_some(), "the trick is won");
    let ring = format!("win-ring-{}-{}", view.hand_no, view.trick.index);
    assert!(
        !present(&rt).winner_shown(&view),
        "no winner before the touchdown"
    );
    assert!(element(&rt, &ring).is_none(), "no {ring} at 0.3 s");
    run_to(&mut rt, 0.7);
    assert!(present(&rt).winner_shown(&view));
    assert!(element(&rt, &ring).is_some(), "{ring} at 0.7 s");
}

/// A whole trick (index 1 or later) as four syncs, one card each, whose last card is not
/// yours and wins the trick, and the sync before its lead. Tries the fixed seeds in turn.
fn fourth_card_wins() -> (SyncEvent, Vec<SyncEvent>) {
    for seed in [
        scenes::DRIVEN_SEED,
        "s1",
        "s2",
        "s3",
        "s4",
        "s5",
        "s6",
        "s7",
    ] {
        let seen = std::cell::RefCell::new(Vec::<SyncEvent>::new());
        let found = scenes::drive(seed, false, |s| {
            let mut seen = seen.borrow_mut();
            seen.push(s.clone());
            let n = seen.len();
            if n < 5 {
                return false;
            }
            let four = &seen[n - 4..];
            let seats: Vec<Seat> = four.iter().filter_map(card_played).collect();
            let one_each = four.iter().all(|x| {
                x.steps
                    .iter()
                    .filter(|st| matches!(st, Step::CardPlayed { .. }))
                    .count()
                    == 1
            });
            seats.len() == 4
                && one_each
                && s.view.trick.index >= 1
                && four.iter().all(|x| x.view.hand_no == s.view.hand_no)
                && four[0].view.trick.plays.len() == 1
                && seats[3] != s.view.you
                && s.view.trick.winner_seat == Some(seats[3])
        });
        if found.is_ok() {
            let seen = seen.into_inner();
            let n = seen.len();
            return (seen[n - 5].clone(), seen[n - 4..].to_vec());
        }
    }
    panic!("no seed has a trick won by its last card");
}

/// A-ring-waits, 4th-card case (09 §4.4): when the last card of the trick wins, its ring and
/// pop show once the Win shot starts (touchdown + `RING_DELAY`), not at the flight's hand-off.
#[test]
fn a_ring_shows_when_the_last_card_wins() {
    let (before, syncs) = fourth_card_wins();
    let last = syncs.last().expect("four syncs");
    let (hand, index) = (last.view.hand_no, last.view.trick.index);
    let seat = card_played(last).expect("a card");
    let key = ShotKey::play(hand, index, seat);
    let mut state = AppState::default();
    state.route = Route::Table;
    state.prefs.tour_seen = true;
    state.table = Some(TableScreen::showing(before));
    let mut rt = runtime(state);
    run_to(&mut rt, 0.3);
    let n = syncs.len();
    let mut sent = 0.0;
    for (i, sync) in syncs.into_iter().enumerate() {
        rt.update(move |s, tx| {
            if let Some(t) = s.table.as_mut() {
                t.on_sync(sync, None, tx);
            }
        });
        sent = rt.state.clock;
        // Each card lands before the next one goes, so the last one flies Full.
        if i + 1 < n {
            run_to(&mut rt, sent + 1.3);
        }
    }
    rt.frame(DT);
    assert_eq!(
        director(&rt).feel_of(&key),
        Some(Feel::Full),
        "the last card flies"
    );
    let ring = format!("win-ring-{hand}-{index}");
    let card = format!("trick-{hand}-{index}-{}", seat.index());
    run_to(&mut rt, sent + 0.3);
    assert!(element(&rt, &ring).is_none(), "no {ring} before touchdown");
    run_to(&mut rt, sent + 0.70);
    assert!(
        element(&rt, &ring).is_some(),
        "{ring} at 0.70 s, before the 0.95 s hand-off"
    );
    assert!(element(&rt, &card).is_some(), "{card} carries it at 0.70 s");
    assert!(
        element(&rt, &format!("fly-{key}")).is_none(),
        "the flyer gave way to the winning card"
    );
}

/// A-sweep-inside-hold (09 §4.4): the sweep lands inside the trick's hold, and a reviewed
/// trick's Continue goes out once the last card lands.
#[test]
fn a_sweep_lands_inside_the_hold() {
    let mut rt = scene_runtime("table-trick-win");
    run_to(&mut rt, 0.1);
    let last = present(&rt)
        .collect
        .as_ref()
        .and_then(|c| c.plays.last().copied())
        .expect("the trick waits to gather");
    let mini = format!("mini-{}", last.card);
    let mut landed = None;
    while rt.state.clock < 4.0 {
        rt.frame(DT);
        // Its rest rect (the spring's own tail below half a point does not count).
        if element(&rt, &mini).is_some_and(|e| {
            e.rect
                .iter()
                .zip(e.target)
                .all(|(a, b)| (a - b).abs() <= 0.5)
        }) {
            landed = Some(rt.state.clock);
            break;
        }
    }
    let landed = landed.expect("the last card reaches the stack");
    assert!(landed <= 3.05, "the last mini rests at {landed:.3} s");

    let mut rt = scene_runtime("table-trick-won");
    run_to(&mut rt, 0.5);
    click(&mut rt, "trick-continue");
    rt.frame(DT);
    let c = present(&rt).collect.clone().expect("the gather");
    let started = c.started.expect("Continue starts the sweep");
    let finish = c.finish_at(false).expect("a finish");
    assert!(
        finish - started <= 0.74 + 1e-9,
        "Continue waits {:.3} s",
        finish - started
    );
}

/// A-skim-last-only (09 §2.3): four cards in one frame skim. Only the last one flies (matched);
/// the others are at rest in the first frame.
#[test]
fn a_backlog_skims_to_the_last_card() {
    let (before, syncs) = four_plays();
    let keys: Vec<ShotKey> = syncs
        .iter()
        .map(|s| {
            let seat = card_played(s).expect("a card");
            ShotKey::play(s.view.hand_no, s.view.trick.index, seat)
        })
        .collect();
    let slots: Vec<String> = keys
        .iter()
        .map(|k| format!("trick-{}", &k.as_str()[5..]))
        .collect();
    let mut state = AppState::default();
    state.route = Route::Table;
    state.prefs.tour_seen = true;
    state.table = Some(TableScreen::showing(before));
    let mut rt = runtime(state);
    run_to(&mut rt, 0.3);
    rt.update(move |s, tx| {
        let table = s.table.as_mut().expect("a table");
        for sync in syncs {
            table.on_sync(sync, None, tx);
        }
    });
    rt.frame(DT);
    let d = director(&rt);
    let feels: Vec<Option<Feel>> = keys.iter().map(|k| d.feel_of(k)).collect();
    assert_eq!(
        feels,
        vec![
            Some(Feel::Snap),
            Some(Feel::Snap),
            Some(Feel::Snap),
            Some(Feel::Skim)
        ],
        "only the last card moves"
    );
    assert_eq!(fly_keys(&rt), Vec::<String>::new(), "no flyer under Skim");
    rt.frame(DT);
    let moving: Vec<&String> = slots
        .iter()
        .filter(|k| element(&rt, k).is_none_or(|e| !at_rest(&e)))
        .collect();
    assert_eq!(moving, vec![&slots[3]], "one matched flight");
}

/// A-fast-forward (09 §2.3): playing while an opponent's card is in the air lands it inside
/// the catch-up window; your own card then flies.
#[test]
fn a_press_fast_forwards_the_card_in_the_air() {
    let (before, found) = opponent_then_you();
    let mut rt = moment_runtime(before, found, false);
    let (key, spec, t0) = until_flyer(&mut rt);
    assert!(!spec.mine);
    run_to(&mut rt, t0 + 0.3 * f64::from(motion::PLAY_END));
    assert!(
        element(&rt, &format!("fly-{key}")).is_some(),
        "in the air at u = 0.3"
    );
    let card = legal_card(&rt);
    let pressed = rt.state.clock;
    rt.update(move |s, _| s.play_card(card));
    run_to(&mut rt, pressed + 0.15);
    let flying: Vec<String> = fly_keys(&rt)
        .into_iter()
        .filter(|k| !k.ends_with(&format!("-{}", shown_view(&rt).you.index())))
        .collect();
    assert_eq!(flying, Vec::<String>::new(), "the opponent's card landed");
    assert!(director(&rt).landed(&key));
    let slot = format!(
        "trick-{}-{}-{}",
        spec.hand_no,
        spec.index,
        spec.seat.index()
    );
    assert!(element(&rt, &slot).is_some(), "{slot} is down");
}

/// A-reconnect-snaps (09 §2.4): a dropped link and a step-less sync land everything in place.
#[test]
fn a_reconnect_lands_everything_at_once() {
    let mut rt = scene_runtime("table-play-opp");
    let (key, _, t0) = until_flyer(&mut rt);
    run_to(&mut rt, t0 + 0.1);
    let mut again = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.store.display_view())
        .map(|v| SyncEvent {
            v: v.v + 1,
            view: v.clone(),
            steps: Vec::new(),
        })
        .expect("a view");
    again.view.v = again.v;
    let conn_id = rt.state.table.as_ref().map(|t| t.conn_id).expect("a table");
    rt.update(move |s, tx| {
        for event in [
            ConnEvent::Status(false),
            ConnEvent::Status(true),
            ConnEvent::Event(TableEvent::Sync(Box::new(again))),
        ] {
            s.receive(Reply::Conn { conn_id, event }, tx);
        }
    });
    rt.frame(DT);
    assert!(director(&rt).landed(&key), "the card in the air landed");
    assert_eq!(fly_keys(&rt), Vec::<String>::new());
    assert!(!director(&rt).busy());
    // The reconnect commits in place (`present.snap` + `tx.settle`): from the next frame no
    // element travels or scales. The fade-ins the move itself mounted one frame after t0 may
    // finish their own opacity (Rune's settle does not land a mount fade already running).
    assert_eq!(movers_where(&mut rt, 0.5, false), Vec::<String>::new());
    assert_eq!(idle_movers(&mut rt, 0.5), Vec::<String>::new());
}

/// What A-reduced-none forbids under reduce motion: any flyer, sparks or confetti, and any
/// travel or scale (a pop, a knock) on the trick's cards, its ring, the stacks' glyphs and
/// the hand result's chip. Each entry names the key and what it did.
fn reduce_breaks(rt: &Runtime<AppState>) -> Vec<String> {
    let mut out = Vec::new();
    for e in rt.elements().into_iter().filter(|e| !e.exiting) {
        let Some(k) = e.key.clone() else { continue };
        if k.starts_with("fly-") || k.starts_with("sparks") || k.starts_with("confetti") {
            out.push(format!("{k} drawn"));
            continue;
        }
        let parts: Vec<&str> = k.split('-').collect();
        let trick_card = parts.len() == 4
            && parts[0] == "trick"
            && parts[1..].iter().all(|p| p.parse::<u32>().is_ok());
        let watched = trick_card
            || k.starts_with("win-ring-")
            || k.starts_with("stack-glyph-")
            || (k.starts_with("hand-result-") && k.ends_with("-chip"));
        if !watched {
            continue;
        }
        if (e.scale - 1.0).abs() > 1e-3 {
            out.push(format!("{k} scale {:.4}", e.scale));
        }
        if dist(centre(e.rect), centre(e.target)) > 0.5 {
            out.push(format!("{k} travels"));
        }
    }
    out
}

/// Plays `syncs` onto a table showing `before`, 20 frames apiece, then runs until the trick
/// has gathered, and returns every [`reduce_breaks`] entry seen on any frame, plus when the
/// gather started (seconds after the winning sync) and whether it was done in one go.
fn play_through(
    before: SyncEvent,
    syncs: Vec<SyncEvent>,
    reduce: Option<bool>,
) -> (Vec<String>, Option<(f64, bool)>, bool) {
    let mut state = AppState::default();
    state.route = Route::Table;
    state.prefs.tour_seen = true;
    // None: motion on. Some(os): reduce motion by the OS (true) or by the preference.
    state.prefs.reduce_motion = reduce == Some(false);
    state.table = Some(TableScreen::showing(before));
    let mut rt = runtime(state);
    if reduce == Some(true) {
        rt.set_system_reduced_motion(true);
    }
    run_to(&mut rt, 0.2);
    let mut breaks = std::collections::BTreeSet::new();
    let mut won_at = 0.0;
    for sync in syncs {
        let won = sync.view.trick.winner_seat.is_some();
        rt.update(move |s, tx| {
            if let Some(t) = s.table.as_mut() {
                t.on_sync(sync, None, tx);
            }
        });
        if won {
            won_at = rt.state.clock;
        }
        for _ in 0..20 {
            rt.frame(DT);
            breaks.extend(reduce_breaks(&rt));
        }
    }
    let mut gather = None;
    while rt.state.clock < won_at + timing::COLLECT_AFTER + 1.0 {
        let before = present(&rt).collect.as_ref().is_some_and(|c| c.moved > 0);
        rt.frame(DT);
        breaks.extend(reduce_breaks(&rt));
        let after = present(&rt).collect.as_ref().is_some_and(|c| c.moved > 0);
        if after && !before {
            let done = present(&rt).collect.as_ref().is_some_and(|c| c.done());
            gather = Some((rt.state.clock - won_at, done));
        }
    }
    let gathered = present(&rt).collect.as_ref().is_some_and(|c| c.done());
    (breaks.into_iter().collect(), gather, gathered)
}

/// A-reduced-none (09 §2.5, D15): a whole trick under reduce motion, by the OS or by the
/// preference, draws no flyer and no burst, pops and knocks nothing, and still gathers at
/// `COLLECT_AFTER`. With motion on, the same trick does fly and pop, so the check can fail.
#[test]
fn reduce_motion_draws_no_flight_and_no_burst() {
    let (before, syncs) = four_plays();
    let (breaks, _, _) = play_through(before, syncs, None);
    assert!(
        breaks.iter().any(|b| b.starts_with("fly-")) && breaks.iter().any(|b| b.contains("scale")),
        "with motion on the trick flies and pops: {breaks:?}"
    );
    for os in [true, false] {
        let (before, syncs) = four_plays();
        let (breaks, gather, gathered) = play_through(before, syncs, Some(os));
        assert_eq!(breaks, Vec::<String>::new(), "os = {os}");
        let (at, done) = gather.expect("the trick gathers");
        assert!(
            (at - timing::COLLECT_AFTER).abs() <= 2.0 * DT,
            "the gather starts at {at:.3} s (os = {os})"
        );
        assert!(done, "all at once under reduce motion (os = {os})");
        assert!(gathered, "the trick gathered (os = {os})");
    }
}

/// A-reduced-none for a euchre (09 §4.8): the stamp's sparks and the hand result's chip pop
/// stay off under reduce motion; with motion on the sparks do draw.
#[test]
fn reduce_motion_quiets_the_euchre_stamp() {
    for reduce in [false, true] {
        let mut state = AppState::default();
        scene(&mut state, "table-euchre-stamp");
        state.prefs.reduce_motion = reduce;
        let mut rt = runtime(state);
        let mut breaks = std::collections::BTreeSet::new();
        while rt.state.clock < 5.0 {
            rt.frame(DT);
            breaks.extend(reduce_breaks(&rt));
        }
        let sparks = breaks.iter().any(|b| b.starts_with("sparks"));
        if reduce {
            assert_eq!(breaks, std::collections::BTreeSet::new());
        } else {
            assert!(sparks, "with motion on the stamp throws sparks: {breaks:?}");
        }
    }
}

/// A-idle (09 §6.2): once the last shot is done the presentation asks for no more wakes.
#[test]
fn the_table_sleeps_after_the_last_shot() {
    let mut rt = scene_runtime("table-play-opp");
    let (key, _, t0) = until_flyer(&mut rt);
    let end = t0 + f64::from(motion::PLAY_END);
    run_to(&mut rt, end + 1.5);
    assert!(director(&rt).landed(&key) && !director(&rt).busy());
    let clock = rt.state.clock;
    let table = rt.state.table.as_mut().expect("a table");
    assert!(table.present.banner.is_none() && table.present.illegal.is_none());
    let out = table.present.poll(clock, false);
    assert_eq!(out.wake, None, "nothing left to wake for");
    assert!(!out.frame);
}

/// A-fx-layer-passes-clicks (09 §2.10): a click reaches the rack under a card in the air.
#[test]
fn a_click_reaches_the_rack_under_a_flyer() {
    let (before, found) = opponent_then_you();
    let mut rt = moment_runtime(before, found, false);
    let (key, _, t0) = until_flyer(&mut rt);
    run_to(&mut rt, t0 + 0.2);
    assert!(
        element(&rt, &format!("fly-{key}")).is_some(),
        "a flyer is live"
    );
    let card = legal_card(&rt);
    click(&mut rt, &format!("hand-{card}"));
    let selected = rt.state.table.as_ref().and_then(|t| t.selected);
    assert_eq!(selected, Some(card), "the click reached the rack");
}

#[test]
fn a_played_card_comes_to_rest() {
    let mut rt = scene_runtime("table-play-opp");
    // The flight's spring has a long tail: it settles by about 1.6 s.
    presented_at(&mut rt, "trick", 2.0).expect("the trick shows");
    assert_eq!(idle_movers(&mut rt, 0.5), Vec::<String>::new());
}

#[test]
fn the_stats_line_reads_a_scene() {
    let s = stats(&[
        "--scene",
        "table-play-opp",
        "--at",
        "1.0",
        "--snap",
        &snap_path(),
    ]);
    assert!(s.animating > 0, "the play animates: {s:?}");
    assert!(s.p50 >= 0.0 && s.p95 >= s.p50, "{s:?}");
}

fn snap_path() -> String {
    let dir = std::env::temp_dir().join(format!("euchre-motion-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    dir.join("stats.png").to_string_lossy().into_owned()
}

#[test]
fn the_tour_steps_and_escape_dismisses_it() {
    let mut rt = scene_runtime("table-tour");
    for step in 1..tour::SLIDES {
        rt.update(|s, _| s.tour_advance());
        rt.frame(DT);
        let at = rt
            .state
            .table
            .as_ref()
            .and_then(|t| t.present.hooks.tour_step);
        assert_eq!(at, Some(step));
    }
    rt.update(|s, _| s.tour_back());
    rt.frame(DT);
    let at = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.present.hooks.tour_step);
    assert_eq!(at, Some(tour::SLIDES - 2));
    rt.update(|s, _| s.toggle_settings());
    escape(&mut rt);
    let hooks = &rt.state.table.as_ref().expect("a table").present.hooks;
    assert_eq!(hooks.tour_step, None, "Escape closes the tour first");
    assert!(
        hooks.settings_open,
        "and leaves the rail for the next Escape"
    );
    assert!(rt.state.prefs.tour_seen, "a dismissed tour stays seen");
    rt.update(|s, _| s.tour_open());
    let at = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.present.hooks.tour_step);
    assert_eq!(at, Some(0), "the rules link opens it again");
}

#[test]
fn the_rules_messages_reach_the_hooks() {
    let mut rt = scene_runtime("table-rules-open");
    rt.update(|s, _| s.rules(RulesMsg::Query("bower".to_owned())));
    rt.update(|s, _| s.rules(RulesMsg::Toggle(2)));
    rt.update(|s, _| s.rules(RulesMsg::Toggle(9)));
    rt.frame(DT);
    let hooks = &rt.state.table.as_ref().expect("a table").present.hooks;
    assert_eq!(hooks.rules_query, "bower");
    assert_eq!(hooks.rules_flipped, 0b100);
}

#[test]
fn the_sound_toggle_turns_the_player_on_and_off() {
    let mut rt = scene_runtime("table-play");
    assert!(
        !rt.state.prefs.sound && !rt.state.sound.enabled(),
        "off (D14)"
    );
    rt.update(|s, _| s.toggle_sound());
    assert!(rt.state.prefs.sound && rt.state.sound.enabled());
    rt.update(|s, _| s.toggle_sound());
    assert!(!rt.state.prefs.sound && !rt.state.sound.enabled());
}

/// D15 (09 §2.5): reduce motion is the app preference OR the system setting. The system setting
/// alone presents every frame exactly as the preference alone does.
#[test]
fn the_system_setting_matches_the_preference() {
    for name in [
        "table-trick-win",
        "table-euchre-stamp",
        "game-over-win",
        "table-deal-anim",
    ] {
        let mut by_pref = AppState::default();
        scene(&mut by_pref, name);
        by_pref.prefs.reduce_motion = true;
        let mut by_pref = runtime(by_pref);
        let mut by_os = AppState::default();
        scene(&mut by_os, name);
        by_os.prefs.reduce_motion = false;
        by_os.wait_for_art = true;
        let mut by_os = App::new(spec())
            .init(move || by_os)
            .headless_runtime(theme::window::SIZE, Env::new(false, 1.0));
        by_os.set_system_reduced_motion(true);
        by_os.frame(0.0);
        for frame in 0..(4.0 / DT) as usize {
            by_pref.frame(DT);
            by_os.frame(DT);
            assert_eq!(
                shown(&by_pref),
                shown(&by_os),
                "{name}: frame {frame} differs between the preference and the system setting"
            );
            assert_eq!(
                reduce_breaks(&by_os),
                Vec::<String>::new(),
                "{name}: the system setting alone draws no flight, burst or pop"
            );
        }
    }
}
