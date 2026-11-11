//! Lane E: the coach (06 §13), the sound cues (06 §15), the first-run tour (06 §14.3) and the
//! screen-reader announcer (06 §10).

use euchre_ai::{RankParams, rank_moves};
use euchre_desktop::app::AppState;
use euchre_desktop::net::TableConfig;
use euchre_desktop::net::sim::fixture_view;
use euchre_desktop::prefs::Prefs;
use euchre_desktop::scenes::{self, DRIVEN_SEED};
use euchre_desktop::screens::table::present::{Cue, Hooks};
use euchre_desktop::screens::table::{announcer, hint, tour};
use euchre_desktop::sound::{self, Filter, Sound, Voice, Wave};
use euchre_desktop::{spec, theme};
use euchre_protocol::{
    Card, DealtPacket, GamePhase, GameStatus, HandResult, PublicGameView, Seat, Step, Suit,
    SyncEvent, Team,
};
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

fn scene_runtime(name: &str) -> Runtime<AppState> {
    let (_, apply) = scenes::SCENES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("scene {name} exists"));
    let mut state = AppState::default();
    apply(&mut state);
    runtime(state)
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

fn hints_on() -> Prefs {
    Prefs {
        hints: true,
        ..Prefs::default()
    }
}

fn your_choice(s: &SyncEvent) -> bool {
    s.view.status == GameStatus::Active
        && s.view.turn_seat == Some(s.view.you)
        && s.view.legal.len() > 1
}

/// Views where the player has a real choice, one per situation the driven match reaches.
fn decisions() -> Vec<PublicGameView> {
    let wants: [fn(&SyncEvent) -> bool; 4] = [
        |s| your_choice(s) && s.view.phase == GamePhase::BidRound1,
        |s| your_choice(s) && s.view.phase == GamePhase::BidRound2,
        |s| {
            your_choice(s)
                && s.view.phase == GamePhase::TrickPlay
                && s.view.trick.led_suit.is_none()
        },
        |s| {
            your_choice(s)
                && s.view.phase == GamePhase::TrickPlay
                && s.view.trick.led_suit.is_some()
        },
    ];
    let found: Vec<PublicGameView> = wants
        .iter()
        .filter_map(|want| scenes::drive(DRIVEN_SEED, false, want).ok())
        .map(|s| s.view)
        .collect();
    assert!(found.len() >= 3, "the driven match reaches the decisions");
    found
}

fn tour_step(rt: &Runtime<AppState>) -> Option<u8> {
    rt.state
        .table
        .as_ref()
        .and_then(|t| t.present.hooks.tour_step)
}

/* ---------------------------------------------------------------------------------------- */
/* Hints (06 §13)                                                                              */
/* ---------------------------------------------------------------------------------------- */

#[test]
fn the_hint_is_the_ai_top_move() {
    for view in decisions() {
        let top = rank_moves(RankParams::new(&view))
            .into_iter()
            .next()
            .expect("a ranked move");
        let hint = hint::for_view(&view, &hints_on()).expect("a hint on your choice");
        assert_eq!(hint.move_id, top.id, "{:?}", view.phase);
        assert_eq!(hint.why, top.why);
        let legal = view
            .legal
            .iter()
            .find(|m| m.id == top.id)
            .expect("the top move is legal");
        assert_eq!(hint.label, legal.label);
        assert_eq!(hint.turn_id, view.turn_id);
        if let Some(card) = hint.card {
            assert!(view.hand.contains(&card), "a suggested card is your own");
            assert!(hint::badge(card, &hint).is_some());
        } else {
            assert!(hint::move_badge(&hint.move_id, &hint).is_some());
        }
        let (line, label) = hint::line(&hint);
        assert!(line.starts_with("Try: "), "{line}");
        assert_eq!(&line[label], legal.label);
        assert!(hint::panel(&hint).is_some());
    }
}

#[test]
fn no_hint_when_hints_are_off_or_it_is_not_your_turn() {
    let view = decisions().remove(0);
    assert!(hint::for_view(&view, &hints_on()).is_some());
    assert_eq!(
        hint::for_view(&view, &Prefs::default()),
        None,
        "off (06 §13)"
    );

    let mut theirs = view.clone();
    theirs.turn_seat = Some(Seat::wrapping(view.you.get() + 1));
    assert_eq!(hint::for_view(&theirs, &hints_on()), None, "not your turn");

    let mut forced = view.clone();
    forced.legal.truncate(1);
    assert_eq!(hint::for_view(&forced, &hints_on()), None, "one legal move");

    let mut over = view;
    over.status = GameStatus::Complete;
    assert_eq!(hint::for_view(&over, &hints_on()), None, "match over");
}

#[test]
fn a_hint_streak_retires_and_a_miss_brings_it_back() {
    let view = decisions().remove(0);
    let situation = hint::Situation::of(&view).expect("a situation");
    let top = hint::for_view(&view, &hints_on()).expect("a hint").move_id;
    let other = view
        .legal
        .iter()
        .find(|m| m.id != top)
        .expect("another move")
        .id
        .clone();
    let mut prefs = hints_on();
    for n in 1..=hint::RETIRE_AFTER {
        assert!(
            hint::for_view(&view, &prefs).is_some(),
            "shown before match {n}"
        );
        prefs.hint_streaks = hint::record_move(&view, &top, &prefs).expect("a match counts");
        assert_eq!(prefs.hint_streaks.get(situation.key()), Some(&n));
    }
    assert_eq!(hint::for_view(&view, &prefs), None, "retired after three");
    // Other situations are not retired by this one.
    assert!(hint::retired(&prefs, situation));
    prefs.hint_streaks = hint::record_move(&view, &other, &prefs).expect("a miss resets");
    assert_eq!(prefs.hint_streaks.get(situation.key()), Some(&0));
    assert!(
        hint::for_view(&view, &prefs).is_some(),
        "a miss brings it back"
    );
    assert_eq!(
        hint::record_move(&view, &other, &prefs),
        None,
        "a second miss changes nothing"
    );
    // A forced move is not a decision.
    let mut forced = view.clone();
    forced.legal.truncate(1);
    assert_eq!(
        hint::record_move(&forced, &forced.legal[0].id, &prefs),
        None
    );
}

#[test]
fn the_rack_shows_the_badge_and_a_play_keeps_the_streak() {
    let mut rt = scene_runtime("table-hint");
    settle(&mut rt, 1.0);
    let view = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.store.display_view())
        .expect("a view")
        .clone();
    let hint = hint::for_view(&view, &rt.state.prefs).expect("a hint in table-hint");
    let card = hint.card.expect("a card to play");
    assert!(
        rt.find(&format!("hint-{card}")).is_some(),
        "the badge marks hand-{card}"
    );
    let marked = rt
        .elements()
        .into_iter()
        .filter(|e| {
            !e.exiting
                && e.key
                    .as_deref()
                    .and_then(|k| k.strip_prefix("hint-"))
                    .is_some_and(|c| Card::parse(c).is_some())
        })
        .count();
    assert_eq!(marked, 1, "one card is marked");
    rt.update(|s, _| s.toggle_settings());
    settle(&mut rt, 0.5);
    assert!(rt.find("hint-panel").is_some(), "the rail carries the line");

    let key = hint.situation.key().to_owned();
    rt.update(move |s, _| s.play_card(card));
    rt.frame(DT);
    assert_eq!(rt.state.prefs.hint_streaks.get(&key), Some(&1));
}

/* ---------------------------------------------------------------------------------------- */
/* Sound (06 §15)                                                                              */
/* ---------------------------------------------------------------------------------------- */

/// Every cue but the place thud, which carries its card and is checked per card below.
const CUES: [Cue; 11] = [
    Cue::Deal,
    Cue::TrickTake,
    Cue::TrumpCalled(Suit::Spades),
    Cue::TrumpCalled(Suit::Hearts),
    Cue::TrumpCalled(Suit::Diamonds),
    Cue::TrumpCalled(Suit::Clubs),
    Cue::March,
    Cue::Euchre,
    Cue::HandWon,
    Cue::GameWon,
    Cue::Error,
];

fn tone(v: &Voice) -> (Wave, f32, f32, f32, f32, f32, bool) {
    match *v {
        Voice::Tone {
            wave,
            freq,
            duration,
            gain,
            attack,
            start,
            overtone,
        } => (wave, freq, duration, gain, attack, start, overtone),
        Voice::Noise { .. } => panic!("a tone, not noise: {v:?}"),
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn every_cue_follows_its_06_recipe() {
    assert_eq!(
        sound::recipe(&Cue::TrumpCalled(Suit::Hearts)),
        vec![Voice::Tone {
            wave: Wave::Triangle,
            freq: 440.0,
            duration: 0.3,
            gain: 0.24,
            attack: 0.006,
            start: 0.0,
            overtone: true,
        }]
    );
    let pitch = |s| tone(&sound::recipe(&Cue::TrumpCalled(s))[0]).1;
    assert_eq!(
        [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs].map(pitch),
        [392.0, 440.0, 493.88, 523.25]
    );
    assert_eq!(
        sound::recipe(&Cue::Deal),
        vec![Voice::Noise {
            duration: 0.18,
            gain: 0.1,
            filter: Filter::BandPass,
            freq: 2200.0,
            sweep_to: None,
            q: 0.6,
            start: 0.0,
        }]
    );
    assert_eq!(
        sound::recipe(&Cue::TrickTake),
        vec![Voice::Noise {
            duration: 0.26,
            gain: 0.13,
            filter: Filter::BandPass,
            freq: 3200.0,
            sweep_to: Some(500.0),
            q: 0.8,
            start: 0.0,
        }]
    );
    assert_eq!(
        sound::recipe(&Cue::Error),
        vec![Voice::Noise {
            duration: 0.08,
            gain: 0.16,
            filter: Filter::LowPass,
            freq: 550.0,
            sweep_to: None,
            q: 0.4,
            start: 0.0,
        }]
    );
    let place = sound::place(0.0);
    assert_eq!(
        tone(&place[0]),
        (Wave::Triangle, 150.0, 0.12, 0.22, 0.004, 0.0, false)
    );
    assert!(matches!(
        place[1],
        Voice::Noise { duration, gain, filter: Filter::LowPass, freq, .. }
            if close(duration, 0.07) && close(gain, 0.05) && close(freq, 900.0)
    ));
    let march: Vec<_> = sound::recipe(&Cue::March).iter().map(tone).collect();
    assert_eq!(
        march.iter().map(|t| t.1).collect::<Vec<_>>(),
        [349.23, 440.0, 523.25, 659.25]
    );
    assert!(march.iter().enumerate().all(|(i, t)| t.0 == Wave::Triangle
        && close(t.2, 0.16)
        && close(t.3, 0.2)
        && close(t.5, i as f32 * 0.08)
        && t.6));
    let euchre: Vec<_> = sound::recipe(&Cue::Euchre).iter().map(tone).collect();
    assert_eq!(
        (euchre[0].0, euchre[0].1, euchre[0].2),
        (Wave::Sine, 440.0, 0.22)
    );
    assert!(close(euchre[1].1, 440.0 * 2f32.powf(-3.0 / 12.0)));
    assert!(close(euchre[1].5, 0.14) && close(euchre[1].2, 0.24) && euchre[1].6);
    let won: Vec<_> = sound::recipe(&Cue::HandWon).iter().map(tone).collect();
    assert_eq!(
        won.iter().map(|t| (t.1, t.4)).collect::<Vec<_>>(),
        [(523.25, 0.02), (659.25, 0.02), (783.99, 0.02)]
    );
    assert!(won.iter().enumerate().all(|(i, t)| t.0 == Wave::Sine
        && close(t.2, 0.55)
        && close(t.3, 0.14)
        && close(t.5, i as f32 * 0.05)));
    let game: Vec<_> = sound::recipe(&Cue::GameWon).iter().map(tone).collect();
    assert_eq!(game.len(), 8);
    assert_eq!(
        game[..4].iter().map(|t| (t.1, t.5)).collect::<Vec<_>>(),
        [(392.0, 0.0), (493.88, 0.12), (587.33, 0.24), (783.99, 0.4)]
    );
    assert!(
        game[4..]
            .iter()
            .all(|t| t.0 == Wave::Sine && close(t.2, 0.7) && close(t.3, 0.12) && close(t.5, 0.55))
    );
}

#[test]
fn every_cue_renders_a_bounded_deterministic_buffer() {
    let mut sets: Vec<(String, Vec<Voice>)> = CUES
        .iter()
        .map(|cue| (format!("{cue:?}"), sound::recipe(cue)))
        .collect();
    for card in ["JS", "AH", "9C", "TD"] {
        let card = Card::parse(card).expect("a card");
        let shift = sound::place_shift(card);
        assert!((-3.0..=3.0).contains(&shift), "{card} shifts {shift}");
        let voices = sound::recipe(&Cue::Place(card));
        assert_eq!(
            sound::render(&voices),
            sound::render(&sound::place(shift)),
            "the place cue for {card} is pitched by its card"
        );
        sets.push((format!("place {card}"), voices));
    }
    sets.push(("lift".to_owned(), sound::lift()));
    for (cue, voices) in sets {
        let a = sound::render(&voices);
        assert_eq!(a, sound::render(&voices), "{cue} is deterministic");
        let end = voices
            .iter()
            .map(|v| match *v {
                Voice::Tone {
                    duration, start, ..
                }
                | Voice::Noise {
                    duration, start, ..
                } => start + duration,
            })
            .fold(0.0f32, f32::max);
        assert_eq!(
            a.len(),
            (end * sound::SAMPLE_RATE as f32).ceil() as usize,
            "{cue}"
        );
        let peak = a.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.005 && peak <= 1.0, "{cue} peak {peak}");
        let tail = &a[a.len() - a.len() / 100..];
        assert!(
            tail.iter().all(|s| s.abs() < 0.01),
            "{cue} decays to nothing"
        );
    }
    assert!(
        (sound::place_shift(Card::parse("JS").expect("JS")) - -2.028).abs() < 1e-3,
        "the web hashSemitone"
    );
    assert!((sound::place_shift(Card::parse("AH").expect("AH")) - -2.592).abs() < 1e-3);
}

#[test]
fn a_test_never_opens_a_speaker() {
    let mut s = Sound::new();
    assert!(!s.enabled(), "off by default (D14)");
    s.set_enabled(true);
    for cue in CUES {
        s.play(&cue);
    }
    assert!(!s.opened());
    assert!(!sound::live_app(), "a test binary is not the app window");
}

/* ---------------------------------------------------------------------------------------- */
/* The first-run tour (06 §14.3)                                                               */
/* ---------------------------------------------------------------------------------------- */

#[test]
fn the_tour_is_owed_once_a_real_hand_is_dealt() {
    let mut view = fixture_view("coach-tour", TableConfig::default()).expect("fixture");
    let fresh = Prefs::default();
    assert!(tour::wanted(&view, &fresh), "a dealt hand, never seen");
    let seen = Prefs {
        tour_seen: true,
        ..Prefs::default()
    };
    assert!(!tour::wanted(&view, &seen));
    view.hand.clear();
    view.phase = GamePhase::Cutting;
    assert!(!tour::wanted(&view, &fresh), "never over the cut");
    assert!(
        !tour::should_auto_open(
            &fixture_view("coach-tour", TableConfig::default()).expect("v"),
            &fresh
        ),
        "never opens by itself outside the app window"
    );
}

#[test]
fn the_tour_steps_through_and_finishing_marks_it_seen() {
    let mut rt = scene_runtime("table-tour");
    settle(&mut rt, 0.6);
    assert!(!rt.state.prefs.tour_seen);
    for n in 0..tour::SLIDES {
        assert_eq!(tour_step(&rt), Some(n));
        let (title, _) = tour::SLIDE_TEXT[usize::from(n)];
        let shown = rt.elements().into_iter().any(|e| {
            !e.exiting && e.key.as_deref() == Some("tour-title") && e.text.as_deref() == Some(title)
        });
        assert!(shown, "slide {n} shows {title}");
        if n == 1 {
            click(&mut rt, "tour-back");
            assert_eq!(tour_step(&rt), Some(0), "Back");
            click(&mut rt, "tour-next");
        }
        click(&mut rt, "tour-next");
        settle(&mut rt, 0.1);
    }
    assert_eq!(tour_step(&rt), None, "Deal me in closes it");
    assert!(rt.state.prefs.tour_seen, "and marks it seen");
    settle(&mut rt, 0.6);
    assert!(rt.find("tour-sheet").is_none(), "the sheet leaves");
}

#[test]
fn skip_escape_and_the_backdrop_each_dismiss_it_for_good() {
    for route in ["skip", "escape", "backdrop"] {
        let mut rt = scene_runtime("table-tour");
        settle(&mut rt, 0.6);
        click(&mut rt, "tour-next");
        assert_eq!(tour_step(&rt), Some(1), "{route}");
        match route {
            "skip" => click(&mut rt, "tour-skip"),
            "escape" => press(&mut rt, Key::Escape),
            _ => {
                rt.pointer_down(4.0, 4.0);
                rt.pointer_up(4.0, 4.0);
                rt.frame(DT);
            }
        }
        assert_eq!(tour_step(&rt), None, "{route} closes it");
        assert!(rt.state.prefs.tour_seen, "{route} marks it seen");
        let view = rt
            .state
            .table
            .as_ref()
            .and_then(|t| t.store.display_view())
            .expect("a view");
        assert!(!tour::wanted(view, &rt.state.prefs), "{route}: never again");
    }
}

#[test]
fn a_new_slide_replaces_the_old_one_at_once() {
    let mut rt = scene_runtime("table-tour");
    settle(&mut rt, 0.6);
    click(&mut rt, "tour-next");
    rt.frame(DT);
    let shown: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| e.opacity > 0.01)
        .filter_map(|e| e.key)
        .filter(|k| k.starts_with("tour-slide-"))
        .collect();
    assert_eq!(shown, ["tour-slide-1"], "the words never overlap");
}

fn sheet(rt: &Runtime<AppState>) -> ([f32; 4], f32, f32) {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some("tour-sheet"))
        .map(|e| (e.rect, e.opacity, e.scale))
        .expect("the sheet is up")
}

#[test]
fn the_sheet_rises_in_and_reduce_motion_only_fades_it() {
    for reduced in [false, true] {
        let mut rt = scene_runtime("table-tour");
        settle(&mut rt, 0.6);
        click(&mut rt, "tour-skip");
        rt.update(move |s, _| s.prefs.reduce_motion = reduced);
        settle(&mut rt, 0.8);
        // The rules panel's "Take the quick tour again".
        rt.update(|s, _| s.tour_open());
        rt.frame(DT);
        rt.frame(DT);
        let (early, early_opacity, early_scale) = sheet(&rt);
        settle(&mut rt, 1.2);
        let (rest, rest_opacity, rest_scale) = sheet(&rt);
        assert!(
            early_opacity < rest_opacity,
            "reduced {reduced}: it fades in"
        );
        assert!((rest_scale - 1.0).abs() < 1e-3 && rest_opacity > 0.99);
        if reduced {
            assert_eq!(early, rest, "no travel under reduce motion");
            assert!(
                (early_scale - 1.0).abs() < 1e-3,
                "no scale under reduce motion"
            );
        } else {
            assert!(
                early[1] > rest[1] + 1.0 || early_scale < 0.99,
                "it rises in: {early:?} {early_scale} -> {rest:?}"
            );
        }
    }
}

#[test]
fn the_tour_eyebrow_and_slides_are_verbatim() {
    assert_eq!(tour::eyebrow(2), "Quick tour · 3 of 6");
    assert_eq!(tour::SLIDE_TEXT[0].0, "Welcome to the table");
    assert_eq!(
        tour::SLIDE_TEXT[4].1,
        "Take 3 or 4 tricks: 1 point. Take all 5 — a march — 2 points. Go alone and take all 5: 4 points. Call trump and win fewer than 3 tricks and you’ve been euchred: the other team scores 2 instead."
    );
    assert_eq!(tour::SLIDE_TEXT[5].0, "You're ready");
    assert_eq!(
        (tour::SKIP, tour::BACK, tour::NEXT, tour::DEAL_ME_IN),
        ("Skip", "Back", "Next", "Deal me in")
    );
}

/* ---------------------------------------------------------------------------------------- */
/* The announcer (06 §10)                                                                      */
/* ---------------------------------------------------------------------------------------- */

fn card(s: &str) -> Card {
    Card::parse(s).unwrap_or_else(|| panic!("{s} is a card"))
}

fn scored(result: HandResult, delta: [u8; 2], score: [u8; 2]) -> Step {
    Step::HandScored {
        result,
        delta,
        score,
        maker_seat: Some(Seat::SOUTH),
        alone_seat: None,
        tricks_won: [3, 2],
    }
}

fn trick(index: u8, winner_seat: Seat) -> Step {
    Step::TrickWon {
        index,
        winner_seat,
        plays: Vec::new(),
        seals_euchre: false,
    }
}

#[test]
fn the_announcer_says_06_verbatim() {
    let view = fixture_view("coach-announcer", TableConfig::default()).expect("fixture");
    assert_eq!(view.you, Seat::SOUTH);
    let say = |step: Step| announcer::sentence(&step, &view);
    let cases: Vec<(Step, Option<&str>)> = vec![
        (
            Step::Cut {
                seat: Seat::WEST,
                cut: true,
            },
            Some("Left opponent: Bump."),
        ),
        (
            Step::Cut {
                seat: Seat::SOUTH,
                cut: false,
            },
            Some("You: Run 'em."),
        ),
        (
            Step::UpCardTurned { card: card("JD") },
            Some("Up-card is the jack of diamonds."),
        ),
        (
            Step::TurnedDown { suit: Suit::Hearts },
            Some("Hearts turned down. Hearts may not be named."),
        ),
        (
            Step::TrumpSet {
                suit: Suit::Hearts,
                maker_seat: Seat::SOUTH,
                alone_seat: None,
                sitting_seat: None,
                via_order_up: true,
            },
            Some("Hearts are trump. You called it. The jack of diamonds is now a heart."),
        ),
        (
            Step::TrumpSet {
                suit: Suit::Spades,
                maker_seat: Seat::NORTH,
                alone_seat: Some(Seat::NORTH),
                sitting_seat: Some(Seat::SOUTH),
                via_order_up: false,
            },
            Some(
                "Spades are trump, made by Your partner. Your partner is going alone. The jack of clubs is now a spade.",
            ),
        ),
        (
            Step::TrumpSet {
                suit: Suit::Clubs,
                maker_seat: Seat::SOUTH,
                alone_seat: Some(Seat::SOUTH),
                sitting_seat: Some(Seat::NORTH),
                via_order_up: false,
            },
            Some(
                "Clubs are trump. You called it. You are going alone. The jack of spades is now a club.",
            ),
        ),
        (
            Step::DealerDiscarded { seat: Seat::EAST },
            Some("Right opponent discards face down."),
        ),
        (
            Step::DealerDiscarded { seat: Seat::SOUTH },
            Some("You discard face down."),
        ),
        (
            Step::CardPlayed {
                seat: Seat::SOUTH,
                card: card("AS"),
            },
            Some("You play the ace of spades."),
        ),
        (
            Step::CardPlayed {
                seat: Seat::WEST,
                card: card("9H"),
            },
            Some("Left opponent plays the nine of hearts."),
        ),
        (
            trick(0, Seat::SOUTH),
            Some("You take the trick. 1 of five. Your lead."),
        ),
        (
            trick(1, Seat::NORTH),
            Some("Your partner takes the trick. 2 of five. Partner leads."),
        ),
        (
            trick(2, Seat::EAST),
            Some("Right opponent takes the trick. 3 of five. Right opponent leads."),
        ),
        (
            trick(4, Seat::WEST),
            Some("Left opponent takes the trick. 5 of five."),
        ),
        (
            scored(HandResult::Point, [1, 0], [4, 2]),
            Some("Point. You score 1. 4 to 2."),
        ),
        (
            scored(HandResult::Euchre, [0, 2], [3, 9]),
            Some("Euchre! They score 2. 3 to 9. They are in the barn."),
        ),
        (
            scored(HandResult::March, [2, 0], [9, 4]),
            Some("March! You score 2. 9 to 4. You are in the barn."),
        ),
        (
            scored(HandResult::LoneMarch, [4, 0], [10, 9]),
            Some("Lone march! You score 4. 10 to 9."),
        ),
        (
            scored(HandResult::LonePoint, [0, 1], [2, 3]),
            Some("Lone point. They score 1. 2 to 3."),
        ),
        (
            scored(HandResult::ThrowIn, [0, 0], [2, 3]),
            Some("Thrown in. Nobody scores. 2 to 3."),
        ),
        (
            Step::ThrowIn,
            Some("All passed twice. Thrown in — the deal moves left."),
        ),
        (
            Step::GameWon {
                team: Team::NS,
                score: [10, 6],
            },
            Some("You win, 10 to 6."),
        ),
        (
            Step::GameWon {
                team: Team::EW,
                score: [7, 10],
            },
            Some("They win, 7 to 10."),
        ),
        (
            Step::Bid {
                seat: Seat::WEST,
                id: "pass".to_owned(),
                say: "Pass.".to_owned(),
            },
            None,
        ),
    ];
    for (step, want) in cases {
        assert_eq!(say(step.clone()).as_deref(), want, "{step:?}");
    }

    // One sync's deal packets are said once, with the hand in your own words.
    let hand: Vec<String> = view
        .hand
        .iter()
        .map(|&c| euchre_engine::card_name_lower(c))
        .collect();
    let dealt = |seat| Step::Dealt {
        packets: vec![DealtPacket {
            seat,
            count: 3,
            cards: None,
        }],
    };
    let mut hooks = Hooks::default();
    for step in [
        dealt(Seat::WEST),
        dealt(Seat::NORTH),
        Step::UpCardTurned { card: card("JD") },
    ] {
        hooks.announce.push_back(step);
    }
    assert_eq!(
        announcer::take(&mut hooks, &view).as_deref(),
        Some(
            format!(
                "Cards dealt. Your hand: {}. Up-card is the jack of diamonds.",
                hand.join(", ")
            )
            .as_str()
        )
    );
    assert!(hooks.announce.is_empty());
    let mut empty = view.clone();
    empty.hand.clear();
    assert_eq!(announcer::sentence(&dealt(Seat::WEST), &empty), None);
}

fn find_live(node: &serde_json::Value, name: &str) -> bool {
    let here = node["name"].as_str() == Some(name)
        && node["states"]
            .as_array()
            .is_some_and(|s| s.iter().any(|v| v.as_str() == Some("live-polite")));
    here || node["children"]
        .as_array()
        .is_some_and(|kids| kids.iter().any(|k| find_live(k, name)))
}

#[test]
fn a_played_card_reaches_the_live_region() {
    let mut rt = scene_runtime("table-play-opp");
    settle(&mut rt, 0.3);
    let said = rt
        .state
        .table
        .as_ref()
        .and_then(|t| t.present.announced.clone())
        .map(|(_, line)| line)
        .expect("the opponent's play was announced");
    assert!(said.contains(" plays the "), "{said}");
    assert!(
        find_live(&rt.accessibility_json(), &said),
        "a polite live node carries {said:?}"
    );
    // The same words again are a fresh node, so they are spoken again.
    rt.update(|s, _| {
        if let Some(t) = s.table.as_mut() {
            t.present
                .announce_line("You play the ace of spades.".to_owned());
        }
    });
    rt.frame(DT);
    assert!(find_live(
        &rt.accessibility_json(),
        "You play the ace of spades."
    ));
}
