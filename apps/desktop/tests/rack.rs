//! Lane C's runtime checks (09 §6.4): the rack's hover lift, illegal shake and deal from the
//! deck, the rolling score, the rack card's size, the narrow settings rail and the dialog scrim.

use euchre_desktop::app::AppState;
use euchre_desktop::screens::table::{copy, panels};
use euchre_desktop::{scenes, spec, theme};
use euchre_protocol::Card;
use rune::{App, Env, Key, KeyEvent, Mods, Runtime};

const DT: f64 = 1.0 / 60.0;

fn sized(state: AppState, size: (f32, f32)) -> Runtime<AppState> {
    let mut state = state;
    state.wait_for_art = true;
    let mut rt = App::new(spec())
        .init(move || state)
        .headless_runtime(size, Env::new(false, 1.0));
    rt.frame(0.0);
    // A scene's moment lands on the frame after the first.
    rt.frame(DT);
    rt
}

fn scene(state: &mut AppState, name: &str) {
    let (_, apply) = scenes::SCENES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("scene {name} exists"));
    apply(state);
}

fn scene_runtime(name: &str, reduced: bool) -> Runtime<AppState> {
    let mut state = AppState::default();
    scene(&mut state, name);
    state.prefs.reduce_motion = reduced;
    sized(state, theme::window::SIZE)
}

fn settle(rt: &mut Runtime<AppState>, seconds: f64) {
    for _ in 0..(seconds / DT).ceil() as usize {
        rt.frame(DT);
    }
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

/// `key`'s drawn rect `[x, y, w, h]` and effective opacity, exiting or not.
fn presented(rt: &Runtime<AppState>, key: &str) -> ([f32; 4], f32) {
    rt.presented(key)
        .unwrap_or_else(|| panic!("{key} is presented"))
}

/// `key`'s laid-out rect, while it is on screen and not leaving.
fn target(rt: &Runtime<AppState>, key: &str) -> Option<[f32; 4]> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .map(|e| e.target)
}

fn click(rt: &mut Runtime<AppState>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(DT);
}

fn inside(inner: [f32; 4], outer: [f32; 4]) -> bool {
    const EPS: f32 = 0.5;
    inner[0] >= outer[0] - EPS
        && inner[1] >= outer[1] - EPS
        && inner[0] + inner[2] <= outer[0] + outer[2] + EPS
        && inner[1] + inner[3] <= outer[1] + outer[3] + EPS
}

fn centre(r: [f32; 4]) -> [f32; 2] {
    [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0]
}

fn view(rt: &Runtime<AppState>) -> &euchre_protocol::PublicGameView {
    rt.state
        .table
        .as_ref()
        .and_then(|t| t.store.display_view())
        .expect("a view")
}

/// The first rack card the view lets the player play.
fn legal_card(rt: &Runtime<AppState>) -> Card {
    let view = view(rt);
    view.hand
        .iter()
        .copied()
        .find(|&c| copy::card_move(view, c).is_some())
        .expect("a playable card")
}

#[test]
fn c_hover_lift() {
    let mut rt = scene_runtime("table-hover", false);
    settle(&mut rt, 0.5);
    let c = legal_card(&rt);
    let plate = format!("plate-{c}");
    let (rest, _) = presented(&rt, &plate);
    let (x, y) = rt
        .find(&format!("hand-{c}"))
        .expect("the card is on screen");
    rt.pointer_move(x, y);
    settle(&mut rt, 0.5);
    let (up, _) = presented(&rt, &plate);
    // The plate scales from its bottom centre, so its bottom edge moves by the offset alone.
    let lift = (up[1] + up[3]) - (rest[1] + rest[3]);
    assert!(
        (lift + 10.0).abs() <= 1.0,
        "lifted {lift} (rest {rest:?}, up {up:?})"
    );
    let scale = up[2] / rest[2];
    assert!((scale - 1.04).abs() <= 0.01, "scaled {scale}");
    rt.pointer_move(4.0, 4.0);
    settle(&mut rt, 0.6);
    let (back, _) = presented(&rt, &plate);
    for (a, b) in back.iter().zip(rest) {
        assert!((a - b).abs() <= 0.5, "back at rest: {back:?} vs {rest:?}");
    }
}

/// The card the scene pressed (the shake's card, or with no shake the first card the view
/// refuses) and its art's x deviation from rest, sampled each frame for `secs`.
fn shake_trace(rt: &mut Runtime<AppState>, secs: f64) -> (Card, Vec<f32>) {
    let shaken = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.present.shake.map(|(c, _)| c));
    let card = shaken.unwrap_or_else(|| {
        let view = view(rt);
        view.hand
            .iter()
            .copied()
            .find(|&c| copy::card_move(view, c).is_none())
            .expect("the scene holds an illegal card")
    });
    let key = format!("card-{card}");
    let rest = target(rt, &key).expect("the card is laid out")[0];
    let mut dx = Vec::new();
    for _ in 0..(secs / DT).ceil() as usize {
        rt.frame(DT);
        dx.push(presented(rt, &key).0[0] - rest);
    }
    (card, dx)
}

#[test]
fn c_shake() {
    let mut rt = scene_runtime("table-illegal-shake", false);
    assert!(
        rt.state
            .table
            .as_ref()
            .is_some_and(|t| t.present.shake.is_some()),
        "the scene pressed an illegal card"
    );
    let (card, dx) = shake_trace(&mut rt, 0.4);
    // The shake crosses zero at 0.06 s, so take its largest swing over its first beat.
    let early = dx[..8].iter().fold(0.0f32, |m, d| m.max(d.abs()));
    assert!(early >= 4.0, "{card} shook {early} ({dx:?})");
    let last = *dx.last().expect("frames");
    assert!(last.abs() <= 0.5, "{card} came home: {last}");

    let mut rt = scene_runtime("table-illegal-shake", true);
    let (_, dx) = shake_trace(&mut rt, 0.4);
    let most = dx.iter().fold(0.0f32, |m, d| m.max(d.abs()));
    assert!(most <= 0.01, "no shake when reduced: {dx:?}");
}

#[test]
fn c_deal_from_deck() {
    let mut rt = scene_runtime("table-deal-anim", false);
    let hand = view(&rt).hand_no;
    let first = view(&rt).hand[0];
    let key = format!("hand-{first}");
    run_to(&mut rt, 0.10);
    // The deal leaves from the deck; while the up card stands in for it, from the up card.
    let deck = [format!("deck-{hand}"), format!("up-{hand}")]
        .into_iter()
        .find_map(|k| {
            rt.elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some(k.as_str()) && !e.exiting)
                .map(|e| e.rect)
        })
        .expect("the deck or the up card is on the felt");
    let (card, _) = presented(&rt, &key);
    // A card waits on the deck, hidden, until its turn in the deal.
    let cards = view(&rt).hand.clone();
    let waiting = cards
        .iter()
        .filter(|c| presented(&rt, &format!("plate-{c}")).1 < 0.01)
        .count();
    assert!(
        waiting >= 1,
        "the cards still on the deck are hidden at 0.10"
    );
    let [cx, cy] = centre(card);
    let [dx, dy] = centre(deck);
    let off = (cx - dx).hypot(cy - dy);
    assert!(
        off <= 60.0,
        "{key} at {card:?} starts {off} from the deck {deck:?}"
    );
    run_to(&mut rt, 2.0);
    let (at, opacity) = presented(&rt, &key);
    let rest = target(&rt, &key).expect("the card is laid out");
    for (a, b) in at.iter().zip(rest) {
        assert!(
            (a - b).abs() <= 0.5,
            "{key} at rest by 2.0: {at:?} vs {rest:?}"
        );
    }
    assert!((opacity - 1.0).abs() < 1e-3, "{key} shows by 2.0");
    for c in &cards {
        let (_, shown) = presented(&rt, &format!("plate-{c}"));
        assert!(
            (shown - 1.0).abs() < 1e-3,
            "plate-{c} shows by 2.0 ({shown})"
        );
    }
}

/// Whether any glyph of the header's two rolling numbers sits off its line.
fn rolling(rt: &Runtime<AppState>) -> bool {
    rt.elements().into_iter().any(|e| {
        e.key
            .as_deref()
            .is_some_and(|k| k.starts_with("score-us-roll.") || k.starts_with("score-them-roll."))
            && (e.exiting || (e.rect[1] - e.target[1]).abs() > 1.0)
    })
}

/// The header's two numbers as the view scores them: yours, then theirs.
fn final_score(rt: &Runtime<AppState>) -> (String, String) {
    let view = view(rt);
    let team = view.you.index() % 2;
    (
        view.score[team].to_string(),
        view.score[1 - team].to_string(),
    )
}

fn label(rt: &Runtime<AppState>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .and_then(|e| e.label.or(e.text))
}

/// The header score as the presentation shows it now.
fn shown(rt: &Runtime<AppState>) -> [u8; 2] {
    rt.state
        .table
        .as_ref()
        .map(|t| t.present.score.shown)
        .expect("a table")
}

#[test]
fn c_score_rolls() {
    let mut rt = scene_runtime("table-hand-result", false);
    // The roll starts when the hand's Score shot does, after the last trick is gathered.
    let before = shown(&rt);
    let mut waited = 0.0;
    while shown(&rt) == before {
        assert!(waited < 6.0, "the score never stepped (still {before:?})");
        rt.frame(DT);
        waited += DT;
    }
    settle(&mut rt, 0.1);
    assert!(rolling(&rt), "a digit is mid-roll 0.1 s after the step");
    settle(&mut rt, 1.2);
    let (us, them) = final_score(&rt);
    assert_eq!(label(&rt, "score-us-roll").as_deref(), Some(us.as_str()));
    assert_eq!(
        label(&rt, "score-them-roll").as_deref(),
        Some(them.as_str())
    );
    assert!(!rolling(&rt), "the roll has landed 1.3 s after the step");

    let mut rt = scene_runtime("table-hand-result", true);
    rt.frame(DT);
    let (us, them) = final_score(&rt);
    for (key, want) in [("score-us-value", us), ("score-them-value", them)] {
        assert_eq!(
            label(&rt, key),
            Some(want),
            "reduced: {key} shows the final score at once"
        );
        let (_, opacity) = presented(&rt, key);
        assert!(opacity > 0.99, "reduced: {key} lands in place ({opacity})");
    }
}

#[test]
fn c_rack_not_smaller() {
    for size in [(1280.0, 800.0), (720.0, 520.0)] {
        rack_not_smaller_at(size);
    }
}

fn rack_not_smaller_at(size: (f32, f32)) {
    let mut state = AppState::default();
    scene(&mut state, "table-play");
    let mut rt = sized(state, size);
    settle(&mut rt, 1.0);
    let view = view(&rt);
    let trick = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter(|e| {
            e.key
                .as_deref()
                .is_some_and(|k| k.starts_with(&format!("trick-{}-", view.hand_no)))
        })
        .map(|e| e.target[2])
        .fold(0.0f32, f32::max);
    assert!(trick > 0.0, "a trick card is on the felt");
    for c in &view.hand {
        let w = target(&rt, &format!("card-{c}")).expect("the rack card")[2];
        assert!(
            w + 0.5 >= trick,
            "{size:?}: rack card {c} {w} wide, trick card {trick}"
        );
    }
}

#[test]
fn c_settings_720() {
    for size in [(720.0, 520.0), (360.0, 640.0)] {
        let mut state = AppState::default();
        scene(&mut state, "table-play");
        let mut rt = sized(state, size);
        settle(&mut rt, 0.3);
        click(&mut rt, "table-settings-open");
        settle(&mut rt, 1.0);
        let window = [0.0, 0.0, size.0, size.1];
        let rail = target(&rt, "table-settings").expect("the rail");
        assert!(inside(rail, window), "{size:?}: laid out {rail:?}");
        let (drawn, _) = presented(&rt, "table-settings");
        assert!(inside(drawn, window), "{size:?}: drawn {drawn:?}");
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        settle(&mut rt, 1.0);
        assert!(
            rt.find("table-settings").is_none(),
            "{size:?}: Escape closes it"
        );
    }
    assert_eq!(panels::rail_width(1280.0), panels::RAIL_W);
    assert_eq!(panels::rail_width(360.0), 328.0);
}

#[test]
fn c_dialog_scrim() {
    let mut rt = scene_runtime("table-rules-open", false);
    run_to(&mut rt, 0.12);
    let (_, scrim) = presented(&rt, "rules-scrim");
    assert!(
        scrim > 0.0 && scrim < 1.0,
        "the scrim is fading in: {scrim}"
    );
    let pose = rt.presented_pose("rules-dialog").expect("the rules sheet");
    assert!(
        pose.scale() < 1.0,
        "the sheet is growing in: {}",
        pose.scale()
    );
    settle(&mut rt, 1.0);
    let (_, scrim) = presented(&rt, "rules-scrim");
    let pose = rt.presented_pose("rules-dialog").expect("the rules sheet");
    assert!(scrim > 0.99 && (pose.scale() - 1.0).abs() < 1e-3, "settled");

    let mut rt = scene_runtime("table-rules-open", true);
    run_to(&mut rt, 0.12);
    let pose = rt.presented_pose("rules-dialog").expect("the rules sheet");
    assert!(
        (pose.scale() - 1.0).abs() < 1e-3,
        "reduced: the sheet fades only"
    );
}

#[test]
fn the_rules_search_filters_and_the_sections_toggle() {
    assert!(panels::section_matches("Going alone", "…", "  ALONE "));
    assert!(panels::section_matches("Scoring", "a march", "March"));
    assert!(!panels::section_matches("The deck", "24 cards", "bower"));
    assert!(panels::section_matches("The deck", "24 cards", "   "));
    assert!(!panels::section_open("", 0, 3), "closed by default");
    assert!(panels::section_open("", 0b1000, 3), "a toggle opens it");
    assert!(
        panels::section_open("trump", 0, 3),
        "a query opens the matches"
    );
    assert!(
        !panels::section_open("trump", 0b1000, 3),
        "a toggle closes one"
    );
    assert_eq!(
        panels::no_match("zz"),
        "No section matches \"zz\".",
        "06 §5.10 verbatim"
    );
}
