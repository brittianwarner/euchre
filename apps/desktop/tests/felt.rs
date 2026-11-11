//! Lane B's checks (09 §6.3): the felt holds still at rest, the trick gathers before the sweep,
//! the up card squeezes edge-on half way through its flip, the fans keep their size, the
//! confetti canvas lives only for its burst and stays cheap, and every flying or played card
//! carries the three-shadow ladder.

use std::collections::BTreeMap;
use std::process::Command;

use euchre_desktop::app::AppState;
use euchre_desktop::screens::table::choreo::{Director, ShotKey};
use euchre_desktop::screens::table::present::Presentation;
use euchre_desktop::theme::motion;
use euchre_desktop::{scenes, spec, theme};
use euchre_protocol::PublicGameView;
use rune::{App, Env, Runtime};

const DT: f64 = 1.0 / 60.0;

/// Elements allowed to keep changing at rest (09 §6.6 idle check; §2.7 `IDLE_OK`): the turn
/// halo breathes on your turn, and nothing else moves.
const IDLE_OK: &[&str] = &["turn-halo"];

fn runtime(state: AppState) -> Runtime<AppState> {
    let mut state = state;
    state.wait_for_art = true;
    let mut rt = App::new(spec())
        .init(move || state)
        .headless_runtime(theme::window::SIZE, Env::new(false, 1.0));
    rt.frame(0.0);
    rt
}

fn scene_runtime(name: &str) -> Runtime<AppState> {
    let (_, apply) = scenes::SCENES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("scene {name} exists"));
    let mut state = AppState::default();
    apply(&mut state);
    runtime(state)
}

/// Runs frames until the poll clock reads `t` (seconds since the first frame).
fn run_to(rt: &mut Runtime<AppState>, t: f64) {
    for _ in 0..((t / DT).ceil() as usize + 2) {
        if rt.state.clock >= t - DT * 0.5 {
            break;
        }
        rt.frame(DT);
    }
}

fn present(rt: &Runtime<AppState>) -> &Presentation {
    &rt.state.table.as_ref().expect("a table").present
}

fn director(rt: &Runtime<AppState>) -> &Director {
    &present(rt).director
}

fn view(rt: &Runtime<AppState>) -> PublicGameView {
    rt.state
        .table
        .as_ref()
        .and_then(|t| t.store.display_view())
        .cloned()
        .expect("a view")
}

/// One keyed element's presentation, for [`idle_movers`].
type Shown = ([f32; 4], f32, f32);

fn shown(rt: &Runtime<AppState>) -> BTreeMap<String, Shown> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| Some((e.key?, (e.rect, e.opacity, e.scale))))
        .collect()
}

/// Runs `frames` frames and returns every key whose rect, opacity or scale moved (by more
/// than 0.01) or that came or went, less [`IDLE_OK`], sorted.
fn idle_movers(rt: &mut Runtime<AppState>, frames: usize) -> Vec<String> {
    const EPS: f32 = 0.01;
    let mut before = shown(rt);
    let mut changed = std::collections::BTreeSet::new();
    for _ in 0..frames {
        rt.frame(DT);
        let now = shown(rt);
        for (key, a) in &now {
            let moved = before.get(key).is_none_or(|b| {
                a.0.iter().zip(b.0).any(|(x, y)| (x - y).abs() > EPS)
                    || (a.1 - b.1).abs() > EPS
                    || (a.2 - b.2).abs() > EPS
            });
            if moved {
                changed.insert(key.clone());
            }
        }
        changed.extend(before.keys().filter(|k| !now.contains_key(*k)).cloned());
        before = now;
    }
    changed
        .into_iter()
        .filter(|k| !IDLE_OK.contains(&k.as_str()))
        .collect()
}

/// A headless run's frame-time line: `(animating frames, p50, p95)`.
fn stats(args: &[&str]) -> (u32, f32, f32) {
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
    (
        words[0].parse().expect("a frame count"),
        num("p50"),
        num("p95"),
    )
}

/// A played card in the trick: `trick-{hand}-{index}-{seat}`.
fn is_trick_card(key: &str) -> bool {
    let parts: Vec<&str> = key.split('-').collect();
    parts.len() == 4 && parts[0] == "trick" && parts[1..].iter().all(|p| p.parse::<u32>().is_ok())
}

/// The width and height of a box turned by `turn` radians whose drawn rect (the turned box's
/// bounds) is `rect`.
fn unturned([_, _, bw, bh]: [f32; 4], turn: f32) -> [f32; 2] {
    let (c, s) = (turn.cos().abs(), turn.sin().abs());
    let d = c * c - s * s;
    [(bw * c - bh * s) / d, (bh * c - bw * s) / d]
}

fn centre([x, y, w, h]: [f32; 4]) -> [f32; 2] {
    [x + w / 2.0, y + h / 2.0]
}

/// B-felt-static: once table-play settles, 30 frames change nothing on screen.
#[test]
fn b_felt_static() {
    let mut rt = scene_runtime("table-play");
    run_to(&mut rt, 3.0);
    assert_eq!(idle_movers(&mut rt, 30), Vec::<String>::new());
    for key in [
        "felt",
        "felt-lamp",
        "felt-grain",
        "felt-vignette",
        "felt-brass",
    ] {
        assert!(rt.presented(key).is_some(), "{key} is drawn");
    }
}

/// B-gather: 2.05 s after the trick's 4th card starts its Play (09 §4.4's `t`), the four
/// trick cards sit within 40 pt of the trick's centre.
#[test]
fn b_gather() {
    let mut rt = scene_runtime("table-trick-win");
    let mut t0 = None;
    for _ in 0..(1.0 / DT) as usize {
        rt.frame(DT);
        let v = view(&rt);
        if let Some(last) = v.trick.plays.get(3) {
            let key = ShotKey::play(v.hand_no, v.trick.index, last.seat);
            t0 = t0.or(director(&rt).started(&key));
        }
    }
    let t0 = t0.expect("the 4th card's Play starts");
    run_to(&mut rt, t0 + 2.05);
    let (trick, _) = rt.presented("trick").expect("the trick area");
    let [cx, cy] = centre(trick);
    let cards: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.key)
        .filter(|k| is_trick_card(k))
        .collect();
    assert_eq!(cards.len(), 4, "four cards in the trick: {cards:?}");
    for key in cards {
        let (rect, _) = rt.presented(&key).expect("a trick card");
        let [x, y] = centre(rect);
        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        assert!(d <= 40.0, "{key} is {d:.1} pt from the trick centre");
    }
}

/// B-flip-squeeze: 0.18 s into the up card's flip its art is edge-on (≤ 4 pt wide); once the
/// flip ends it is the card's full width again.
#[test]
fn b_flip_squeeze() {
    let mut rt = scene_runtime("table-upcard-flip");
    let hand = view(&rt).hand_no;
    let key = ShotKey::flip(hand);
    let mut start = None;
    for _ in 0..(6.0 / DT) as usize {
        rt.frame(DT);
        start = director(&rt).started(&key);
        if start.is_some() {
            break;
        }
    }
    let start = start.expect("the up card's flip starts");
    let at = start + f64::from(motion::FLIP) / 2.0;
    rt.frame(at - rt.state.clock);
    let up = format!("up-{hand}");
    // The up card lies tilted; its art turns with it, so undo the tilt before measuring.
    let tilt = rt.presented_pose(&up).map_or(0.0, |p| p.rotation());
    let (art, _) = rt.presented("up-art").expect("the up card's art");
    let [w, _] = unturned(art, tilt);
    assert!(
        w <= 4.0,
        "edge-on at mid flip: {w} pt wide ({art:?}, tilt {tilt})"
    );
    run_to(&mut rt, start + 1.5);
    let (card, _) = rt.presented(&up).expect("the up card");
    let (art, _) = rt.presented("up-art").expect("the up card's art");
    let ([aw, _], [cw, _]) = (unturned(art, tilt), unturned(card, tilt));
    assert!(
        (aw - cw).abs() <= 0.5,
        "full width after the flip: art {aw}, card {cw}"
    );
}

/// B-fans-size: every back in an opponent's or partner's fan is drawn 30 by 42, with one card
/// left (table-play) and with five (table-upcard-flip).
#[test]
fn b_fans_size() {
    for (name, at) in [("table-play", 3.0), ("table-upcard-flip", 3.0)] {
        let mut rt = scene_runtime(name);
        run_to(&mut rt, at);
        let backs: Vec<String> = rt
            .elements()
            .into_iter()
            .filter(|e| !e.exiting)
            .filter_map(|e| e.key)
            .filter(|k| k.starts_with("back-"))
            .collect();
        assert!(backs.len() >= 3, "{name}: the fans show backs: {backs:?}");
        for key in backs {
            let (rect, _) = rt.presented(&key).expect("a back");
            let pose = rt.presented_pose(&key).expect("a pose");
            let [w, h] = unturned(rect, pose.rotation());
            let (w, h) = (w / pose.scale(), h / pose.scale());
            assert!(
                (w - 30.0).abs() <= 0.5 && (h - 42.0).abs() <= 0.5,
                "{name}: {key} is {w}x{h}"
            );
        }
    }
}

/// B-confetti-bounded: in game-over-win the confetti canvas is drawn only while its burst runs
/// (from `CONFETTI_DELAY` after the match-end panel's banner, for `CONFETTI_SECS`), it does
/// draw, and the scene's frames stay cheap.
#[test]
fn b_confetti_bounded() {
    let mut rt = scene_runtime("game-over-win");
    let mut seen = 0;
    let delay = f64::from(motion::CONFETTI_DELAY);
    for _ in 0..(10.0 / DT) as usize {
        rt.frame(DT);
        let live = rt
            .elements()
            .into_iter()
            .any(|e| !e.exiting && e.key.as_deref() == Some("confetti-canvas"));
        if !live {
            continue;
        }
        seen += 1;
        let since = present(&rt)
            .banner
            .as_ref()
            .and_then(|b| b.since)
            .expect("a banner with a start");
        let t = rt.state.clock - since;
        assert!(
            t >= delay - DT && t < delay + motion::CONFETTI_SECS,
            "confetti canvas at {t:.3} s"
        );
    }
    assert!(seen > 0, "the confetti canvas drew");
    assert!(
        rt.elements()
            .iter()
            .all(|e| e.key.as_deref() != Some("confetti-canvas")),
        "the canvas is gone once its burst ends"
    );
    // Frame time: the best of five runs, since other work on the machine lands in a run's
    // slowest frames. `--snap` keeps file writes out of the frames being timed.
    let dir = std::env::temp_dir().join(format!("felt-confetti-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    let snap = dir.join("end.png").to_string_lossy().into_owned();
    let mut best = (0, f32::MAX, f32::MAX);
    for _ in 0..5 {
        let run = stats(&[
            "--scene",
            "game-over-win",
            "--frames",
            "240",
            "--fps",
            "60",
            "--snap",
            &snap,
        ]);
        if run.2 < best.2 {
            best = run;
        }
        if best.2 <= 4.0 {
            break;
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    let (animating, p50, p95) = best;
    assert!(animating > 0, "the burst animates");
    assert!(p95 <= 4.0, "game-over-win frames p50 {p50} p95 {p95} ms");
}

/// B-shadow-count: every flying card (`fly-*`) and played card (`trick-*`) carries exactly the
/// three shadows of `motion::card_shadows`, at every frame of a play and a trick win.
#[test]
fn b_shadow_count() {
    let mut checked = 0;
    let mut flown = false;
    for name in ["table-play-opp", "table-trick-win"] {
        let mut rt = scene_runtime(name);
        for _ in 0..(2.5 / DT) as usize {
            rt.frame(DT);
            let keys: Vec<String> = rt
                .elements()
                .into_iter()
                .filter(|e| !e.exiting)
                .filter_map(|e| e.key)
                .filter(|k| k.starts_with("fly-") || is_trick_card(k))
                .collect();
            for key in keys {
                flown |= key.starts_with("fly-");
                let pose = rt.presented_pose(&key).expect("a pose");
                assert_eq!(pose.shadows().len(), 3, "{name}: {key}");
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "cards were checked");
    assert!(flown, "a flyer was checked");
}

/// 09 §4.8: the euchre stamp sits on the hand-result card's title, which hides under it (both
/// say the same word), and the card's line stays clear below it. The title comes back once
/// the stamp has gone.
#[test]
fn b_stamp_clears_the_hand_result_line() {
    let mut rt = scene_runtime("table-euchre-stamp");
    let mut checked = false;
    for _ in 0..(8.0 / DT) as usize {
        rt.frame(DT);
        let hand = view(&rt).hand_no;
        let live = director(&rt).elapsed(&ShotKey::stamp(hand), rt.state.clock);
        if !live.is_some_and(|t| t > 0.3) {
            continue;
        }
        let shown = shown(&rt);
        let (Some(stamp), Some(line), Some(title)) = (
            shown.get(&format!("stamp-{hand}")),
            shown.get(&format!("hand-result-{hand}-line")),
            shown.get(&format!("hand-result-{hand}-title")),
        ) else {
            continue;
        };
        // The plate's own bottom edge under its centre: its height from its drawn (turned)
        // bounds.
        let [_, plate_h] = unturned(stamp.0, theme::felt::size::STAMP_TURN);
        let plate_bottom = centre(stamp.0)[1] + plate_h / 2.0;
        assert!(
            plate_bottom <= line.0[1],
            "the stamp ({plate_bottom}) clears the line ({:?})",
            line.0
        );
        assert!(
            title.1 < 0.05,
            "the title hides under the stamp ({})",
            title.1
        );
        checked = true;
        break;
    }
    assert!(checked, "the stamp and the card showed together");
    let hand = view(&rt).hand_no;
    let later = rt.state.clock + 2.0;
    run_to(&mut rt, later);
    let shown = shown(&rt);
    let title = shown
        .get(&format!("hand-result-{hand}-title"))
        .expect("the title is back");
    assert!(title.1 > 0.95, "the title shows again ({})", title.1);
}

/// 09 §4.10: the match ends in a panel centred at 38% of the window's height, over a scrim,
/// not the low banner over the hand area. It waits for the last trick's sweep and the score,
/// so no trick card shows under it, and it holds the final score and Play again.
#[test]
fn b_match_end_panel() {
    let mut rt = scene_runtime("game-over-win");
    let mut seen = false;
    while rt.state.clock < 1.2 {
        rt.frame(DT);
        let now = shown(&rt);
        if now.contains_key("match-end-panel") {
            seen = true;
            // The last trick sweeps while the panel fades up: by the time
            // the panel is nearly opaque the trick area is clear.
            let trick: Vec<&String> = now.keys().filter(|k| is_trick_card(k)).collect();
            let panel_opacity = now["match-end-panel"].1;
            assert!(
                panel_opacity < 0.9 || trick.is_empty(),
                "the trick clears before the panel is in: {trick:?}"
            );
        }
    }
    assert!(seen, "the panel shows by 1.2 s");
    let shown = shown(&rt);
    let panel = shown.get("match-end-panel").expect("the panel shows").0;
    let height = theme::window::SIZE.1;
    let mid = centre(panel)[1];
    assert!(
        (mid - height * 0.38).abs() < 3.0,
        "the panel's centre {mid} sits at 38% of {height}"
    );
    assert!(shown.contains_key("match-end-scrim"));
    assert!(shown.contains_key("match-end-score"));
    assert!(shown.contains_key("match-end-again"));
    assert!(
        !shown.contains_key("banner-layer"),
        "not over the hand area"
    );
}

/// 09 §4.4: the taker's stack knocks on the beat its last card lands, which is when
/// `Cue::TrickTake` goes out (the gather's `took`), not on the sweep spring's response.
#[test]
fn b_the_stack_knocks_as_the_last_card_lands() {
    let mut rt = scene_runtime("table-trick-win");
    let mut took = None;
    let mut knock = None;
    while rt.state.clock < 5.0 && (took.is_none() || knock.is_none()) {
        rt.frame(DT);
        let now = rt.state.clock;
        if took.is_none() && present(&rt).collect.as_ref().is_some_and(|c| c.took) {
            took = Some(now);
        }
        // The glyph re-keys `stack-glyph-{seat}-{hand}-{trick}` when its gather is all sent.
        let knocking = rt.elements().iter().any(|e| {
            e.key
                .as_deref()
                .is_some_and(|k| k.starts_with("stack-glyph-") && k.split('-').count() == 5)
                && (e.scale - 1.0).abs() > 1e-3
        });
        if knock.is_none() && knocking {
            knock = Some(now);
        }
    }
    let took = took.expect("the last card lands and the take cue goes out");
    let knock = knock.expect("the stack knocks");
    assert!(
        (knock - took).abs() <= 2.0 * DT,
        "the knock at {knock:.3} s, the take at {took:.3} s"
    );
}
