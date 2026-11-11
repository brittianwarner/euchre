//! The first-run tour (06 §14.3): six short slides over the table, once, the first time a real
//! hand is on the felt. The handlers live in app.rs:
//! - `AppState::tour_open` (the rules panel's `New to euchre? Take the quick tour again`;
//!   resets to slide 0, does not re-arm the auto-open),
//! - `AppState::tour_advance` (Next; on the last slide, `Deal me in`, it dismisses),
//! - `AppState::tour_back` (Back),
//! - `AppState::tour_dismiss` (Skip, Escape, the backdrop, finish; sets `prefs.tour_seen`).
//!
//! The step lives in `Presentation.hooks.tour_step` (`Some(0..SLIDES)` while it shows). The
//! sheet is a Rune modal: it traps focus, Escape and a press on the backdrop dismiss it, and,
//! mounted after the rules and review dialogs, it is the active modal over them.
//!
//! The auto-open is [`should_auto_open`], which app.rs `poll` asks every frame. It never opens
//! in a headless run (scenes, `--snap`): only `table-tour` shows the tour, by setting the step.

use std::sync::LazyLock;

use euchre_protocol::{GamePhase, PublicGameView};
use rune::tok::TextStyle;
use rune::{Color, Cursor, El, ScrollSpec, Spring, SpringPair, Transition, col, el, row, text};

use crate::app::AppState;
use crate::prefs::Prefs;
use crate::theme::{self, SERIF, hexes, motion, space};

/// The number of slides.
pub const SLIDES: u8 = 6;

/// The slides, title and body, verbatim from 06 §14.3.
pub const SLIDE_TEXT: [(&str, &str); SLIDES as usize] = [
    (
        "Welcome to the table",
        "Four players, two teams. You and Your partner play together. The left and right opponents are the other team. First team to 10 points wins.",
    ),
    (
        "The deck",
        "Euchre uses 24 cards: nine, ten, jack, queen, king and ace, in all four suits. That's it — nothing lower than a nine is in the deck.",
    ),
    (
        "The bowers — the one rule to know",
        "The jack of the trump suit is the best card in the game — it's called the right bower. But there is a second trump jack: the jack of the OTHER suit that shares its colour. If hearts are trump, the jack of diamonds turns into a heart for the rest of the hand — that's the left bower, and it's the second-best card on the table.",
    ),
    (
        "Bidding",
        "Each hand turns one card face up. Round one: you may \"order it up\" to make its suit trump, or pass. If everyone passes, that card is turned down and round two lets you name any OTHER suit, or pass again. Confident in your hand? Go alone — your partner sits out and a perfect hand is worth double.",
    ),
    (
        "Scoring",
        "Take 3 or 4 tricks: 1 point. Take all 5 — a march — 2 points. Go alone and take all 5: 4 points. Call trump and win fewer than 3 tricks and you’ve been euchred: the other team scores 2 instead.",
    ),
    (
        "You're ready",
        "Select a card, then press Play. Cards you cannot play explain why when you tap them. How to play opens the rules, and Settings has hints and a Wait for me option so you can review each trick at your own pace.",
    ),
];

/// The buttons (06 §14.3).
pub const SKIP: &str = "Skip";
/// Back.
pub const BACK: &str = "Back";
/// Next.
pub const NEXT: &str = "Next";
/// The last slide's Next.
pub const DEAL_ME_IN: &str = "Deal me in";
/// The dialog's accessible name.
pub const TITLE: &str = "Quick tour";

/// `Quick tour · {n} of 6` for slide `step` (0-based).
pub fn eyebrow(step: u8) -> String {
    format!("{TITLE} · {} of {SLIDES}", step.min(SLIDES - 1) + 1)
}

/// The colours (06 §14.3).
mod tokens {
    /// The sheet.
    pub const SHEET: &str = "#12362b";
    /// The sheet's and the ghost buttons' edge, `rgba(232,194,122,.35)`.
    pub const EDGE: &str = "#e8c27a59";
    /// The backdrop, `rgba(6,5,3,.72)`.
    pub const BACKDROP: &str = "#060503b8";
    /// The title.
    pub const TITLE: &str = "#f7f1e4";
    /// The body.
    pub const BODY: &str = "#e8dcc6";
    /// An inactive dot, `rgba(232,194,122,.28)`.
    pub const DOT: &str = "#e8c27a47";
    /// The primary button.
    pub const PRIMARY: &str = "#d4ee86";
    /// A ghost button's hover wash.
    pub const GHOST_HOVER: &str = "#e8c27a1f";
    /// The sheet's shadow, `0 12px 40px rgba(0,0,0,.55)`.
    pub const SHADOW: &str = "#0000008c";
    /// Every token, for the parse test.
    #[cfg(test)]
    pub const ALL: [&str; 9] = [
        SHEET,
        EDGE,
        BACKDROP,
        TITLE,
        BODY,
        DOT,
        PRIMARY,
        GHOST_HOVER,
        SHADOW,
    ];
}

struct Colors {
    sheet: Color,
    edge: Color,
    backdrop: Color,
    soft: Color,
    gold: Color,
    title: Color,
    body: Color,
    dot: Color,
    primary: Color,
    ghost_hover: Color,
}

static COLORS: LazyLock<Colors> = LazyLock::new(|| Colors {
    sheet: theme::color(tokens::SHEET),
    edge: theme::color(tokens::EDGE),
    backdrop: theme::color(tokens::BACKDROP),
    soft: theme::color(hexes::DARK_SOFT),
    gold: theme::color(hexes::DARK_GOLD),
    title: theme::color(tokens::TITLE),
    body: theme::color(tokens::BODY),
    dot: theme::color(tokens::DOT),
    primary: theme::color(tokens::PRIMARY),
    ghost_hover: theme::color(tokens::GHOST_HOVER),
});

/// Type (06 §14.3 at a 16 pt rem, the serif throughout).
mod ty {
    use super::{SERIF, TextStyle};

    /// The eyebrow: 0.78rem, 0.06em (callers uppercase it).
    pub const EYEBROW: TextStyle = TextStyle::new(12.5, 500.0)
        .family(SERIF)
        .lh(16.0)
        .tracking(0.75);
    /// The title: 1.4rem.
    pub const TITLE: TextStyle = TextStyle::new(22.4, 600.0).family(SERIF).lh(29.0);
    /// The body: 1rem, line height 1.5.
    pub const BODY: TextStyle = TextStyle::new(16.0, 400.0).family(SERIF).lh(24.0);
    /// A button: 1rem, 600.
    pub const BUTTON: TextStyle = TextStyle::new(16.0, 600.0).family(SERIF).lh(20.0);
}

/// The sheet's widest (30rem) and its share of a narrow window (92vw).
const MAX_W: f32 = 480.0;
/// The sheet's corner (0.9rem).
const RADIUS: f32 = 14.4;
/// A dot (0.45rem).
const DOT: f32 = 7.2;
/// A button's corner (0.4rem).
const BUTTON_RADIUS: f32 = 6.4;

/// Whether a real hand is on the felt: cards in your rack, or a phase past the deal (the
/// sitting seat under a loner holds no cards).
pub fn real_hand(view: &PublicGameView) -> bool {
    !view.hand.is_empty()
        || !matches!(
            view.phase,
            GamePhase::Lobby | GamePhase::Cutting | GamePhase::Deal
        )
}

/// Whether the player is owed the tour now: it was never seen or dismissed, and a real hand
/// is on the felt (06 §14.3).
pub fn wanted(view: &PublicGameView, prefs: &Prefs) -> bool {
    !prefs.tour_seen && real_hand(view)
}

/// Whether the tour opens by itself now: [`wanted`], in the app's own window. A headless run
/// never auto-opens it (only the `table-tour` scene shows it), and neither does a test binary:
/// the same check that keeps the sound player off the speaker in tests.
pub fn should_auto_open(view: &PublicGameView, prefs: &Prefs) -> bool {
    wanted(view, prefs) && crate::sound::live_app()
}

/// A tour button. `primary` is the lime one.
fn button(key: &str, label: &'static str, primary: bool, on: fn(&mut AppState)) -> El {
    let c = &*COLORS;
    let b = row()
        .key(key.to_owned())
        .min_h(space::TAP)
        .min_w(space::TAP)
        .px(17.6)
        .center()
        .radius(BUTTON_RADIUS)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("button")
        .label(label)
        .on_press(on);
    if primary {
        b.fill(c.primary)
            .border(1.0, c.gold)
            .child(text(label).font(ty::BUTTON).color(c.sheet).a11y_hidden())
    } else {
        b.border(1.0, c.edge)
            .hover(c.ghost_hover)
            .child(text(label).font(ty::BUTTON).color(c.soft).a11y_hidden())
    }
}

/// The tour dialog at slide `step` (0-based) over a window `size`; `reduced` is effective
/// reduce motion. table/mod.rs mounts it last, over the dialogs, only while a step shows.
///
/// Motion (09: dialogs + scrim, P1): the whole layer fades on `SCRIM_IN` / `SCRIM_OUT`, the
/// sheet rises from 96 % and 10 pt low on `PANEL_IN`, and each new slide's words rise 6 pt into
/// place on `UI` while the old slide leaves at once. Under reduce motion only the fades run.
pub fn layer(step: u8, size: [f32; 2], reduced: bool) -> El {
    let c = &*COLORS;
    let step = step.min(SLIDES - 1);
    let (title, body) = SLIDE_TEXT[usize::from(step)];
    let last = step + 1 >= SLIDES;
    let mut dots = row().key("tour-dots").gap(5.6).a11y_hidden();
    for i in 0..SLIDES {
        dots = dots.child(
            el().key(format!("tour-dot-{i}"))
                .size(DOT, DOT)
                .radius(DOT / 2.0)
                .fill(if i == step { c.gold } else { c.dot }),
        );
    }
    let slide = col()
        .key(format!("tour-slide-{step}"))
        .w_full()
        .gap(space::S)
        .child(
            text(title)
                .key("tour-title")
                .font(ty::TITLE)
                .color(c.title)
                .role("heading")
                .level(2)
                .wrap(),
        )
        .child(
            text(body)
                .key("tour-body")
                .font(ty::BODY)
                .color(c.body)
                .wrap(),
        )
        // The old slide must leave the frame the new one mounts, so the words never overlap:
        // an asymmetric transition takes its springs from the insertion side, so the exit
        // spring is `Spring::NONE` (an instant curve, even under reduce motion).
        .transition(
            if reduced {
                Transition::opacity()
            } else {
                Transition::offset(0.0, 6.0).combined(Transition::opacity())
            }
            .springs(SpringPair::new(motion::UI, Spring::NONE)),
        );
    let mut actions = row()
        .key("tour-actions")
        .w_full()
        .gap(space::S)
        .center_y()
        .child(button("tour-skip", SKIP, false, |s| s.tour_dismiss()))
        .child(el().grow(1.0));
    if step > 0 {
        actions = actions.child(button("tour-back", BACK, false, |s| s.tour_back()));
    }
    actions = actions.child(
        button(
            "tour-next",
            if last { DEAL_ME_IN } else { NEXT },
            true,
            |s| s.tour_advance(),
        )
        .autofocus(),
    );
    let max_w = MAX_W.min(size[0] * 0.92);
    let sheet = col()
        .key("tour-sheet")
        .w_full()
        .max_w(max_w)
        .max_h((size[1] - 2.0 * space::M).max(200.0))
        .px(space::L)
        .pt(space::L)
        .pb(20.0)
        .gap(space::M)
        .radius(RADIUS)
        .fill(c.sheet)
        .border(1.0, c.edge)
        .role("dialog")
        .label(TITLE)
        .modal_tx(true, |s: &mut AppState, _tx| s.tour_dismiss())
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            text(eyebrow(step).to_uppercase())
                .key("tour-eyebrow")
                .font(ty::EYEBROW)
                .color(c.soft)
                .label(eyebrow(step)),
        )
        .child(slide)
        .child(dots)
        .child(actions)
        .transition(if reduced {
            Transition::none()
        } else {
            Transition::scale(0.96)
                .combined(Transition::offset(0.0, 10.0))
                .springs(SpringPair::new(motion::PANEL_IN, motion::PANEL_OUT))
        });
    let sheet = theme::shadows(sheet, &[(0.0, 12.0, 40.0, 0.0, tokens::SHADOW)]);
    el().key("tour-layer")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .raise()
        .transition(if reduced {
            Transition::opacity()
        } else {
            Transition::opacity().springs(SpringPair::new(motion::SCRIM_IN, motion::SCRIM_OUT))
        })
        .child(
            el().key("tour-scrim")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .fill(c.backdrop)
                .a11y_hidden(),
        )
        .child(
            col()
                .key("tour-centre")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .padding(space::M)
                .center()
                .child(sheet),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_token_parses() {
        for t in tokens::ALL {
            assert!(rune::color::hex(t).is_some(), "{t}");
        }
    }

    #[test]
    fn the_eyebrow_counts_from_one() {
        assert_eq!(eyebrow(0), "Quick tour · 1 of 6");
        assert_eq!(eyebrow(5), "Quick tour · 6 of 6");
        assert_eq!(eyebrow(9), "Quick tour · 6 of 6");
    }
}
