//! The cream header (06 §5.3): the wordmark home link, the match score with its six-and-four
//! score cards, and the `How to play` and `Settings ⌄` buttons (06 §5.9, §5.10).

use euchre_protocol::PublicGameView;
use rune::{Align, Color, Cursor, El, Sequence, Spring, col, el, row, spacer, text};
use rune_kit::{Kit, RollingText};

use super::copy;
use super::present::ScoreRoll;
use crate::app::{AppState, Route};
use crate::screens::wordmark;
use crate::theme::felt::{size, ty as fty};
use crate::theme::{self, FELT, PAL, motion, space, ty};

thread_local! {
    /// `RollingText` takes its fallback font and colour from a kit; the header passes its own.
    static KIT: Kit = Kit::neutral(false);
}

/// Pip centres on the six (viewBox units, card-local).
const SIX: [(f32, f32); 6] = [
    (10.0, 13.0),
    (24.0, 13.0),
    (10.0, 24.0),
    (24.0, 24.0),
    (10.0, 35.0),
    (24.0, 35.0),
];
/// Pip centres on the four.
const FOUR: [(f32, f32); 4] = [(10.0, 13.0), (24.0, 13.0), (10.0, 35.0), (24.0, 35.0)];
/// An unlit pip's opacity.
const UNLIT: f32 = 0.08;
/// How far a rolling score digit fades at the top and bottom of its line.
const CLIP_FADE: f32 = 4.0;

/// Below this window width the wordmark gives its room to the header's two buttons.
const BRAND_MIN_W: f32 = 1000.0;
/// Below this width the six-and-four score cards step aside too; the numerals carry the score.
const SCORE_CARDS_MIN_W: f32 = 800.0;
/// At this width or narrower the header wraps: the score across the top, the two buttons under
/// it, one at each end (the web table's 700-pixel rule).
const WRAP_W: f32 = 700.0;
/// A wrapped header's side and top padding, and the gap between its two lines.
const WRAP_PAD: (f32, f32, f32) = (12.0, 6.0, 6.0);

/// The header at window `width`; `compact` below [`size::COMPACT_H`] tall. `settings_open`
/// marks the Settings button expanded. `roll` is the header's score roll and the frame clock
/// (09 §4.8): the numerals roll one point per step, and when the last step lands a gold ring
/// pulses behind the number and the number pops. With no roll the score shows the view's score.
pub fn header_with(
    view: &PublicGameView,
    width: f32,
    compact: bool,
    reduced: bool,
    settings_open: bool,
    roll: Option<(&ScoreRoll, f64)>,
) -> El {
    let team = view.you.index() % 2;
    let shown = roll.map_or(view.score, |(r, _)| r.shown);
    let (ours, theirs) = (shown[team], shown[1 - team]);
    // Seconds since the roll landed, once it has (the pulse and the pop).
    let landed = roll.and_then(|(r, now)| {
        let at = r.landed_at.filter(|_| r.shown == r.target)?;
        Some((now - at).max(0.0) as f32)
    });
    // The team that scored this hand is the one whose number pulses.
    let moved = |i: usize| landed.filter(|_| view.delta.is_some_and(|d| d[i] > 0));
    let cards = width >= SCORE_CARDS_MIN_W;
    let wrap = width <= WRAP_W;
    // A wrapped header sets the numerals at the compact size, as a short window does.
    let small = compact || wrap;
    let mut score = row()
        .key("score")
        .role("group")
        .label("Match score")
        .gap(space::SM)
        .center_y();
    if cards {
        score = score.child(score_cards("score-cards-us", ours));
    }
    score = score.child(label("Your team")).child(number(
        "score-us",
        ours,
        small,
        reduced,
        moved(team),
    ));
    // Wrapped, the score spans the header: your team at the left edge, theirs at the right.
    if wrap {
        score = score.w_full().child(spacer());
    }
    score = score.child(
        text("—")
            .font(fty::SCORE_LABEL)
            .color(FELT.score_divider)
            .a11y_hidden(),
    );
    if wrap {
        score = score.child(spacer());
    }
    score = score
        .child(number(
            "score-them",
            theirs,
            small,
            reduced,
            moved(1 - team),
        ))
        .child(label("Other team"));
    if cards {
        score = score.child(score_cards("score-cards-them", theirs));
    }
    if wrap {
        let (px, py, gap) = WRAP_PAD;
        return col()
            .key("table-header")
            .w_full()
            .px(px)
            .py(py)
            .gap(gap)
            .fill(PAL.cream)
            .child(score)
            .child(super::panels::header_tools(settings_open, compact, true));
    }
    let brand = row()
        .key("table-home")
        .min_h(space::TAP)
        .center_y()
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("link")
        .label("Euchre home")
        .on_press_tx(|s: &mut AppState, tx| s.navigate(Route::Home, tx))
        .child(wordmark(
            "table-wordmark",
            ty::BRAND,
            ty::BRAND_CLUB,
            PAL.ink,
        ));
    let mut bar = row()
        .key("table-header")
        .w_full()
        .h(if compact {
            size::HEADER_H_COMPACT
        } else {
            size::HEADER_H
        })
        .px(space::L)
        .gap(space::L)
        .center_y()
        .fill(PAL.cream);
    if width >= BRAND_MIN_W {
        bar = bar.child(brand).child(
            el().w(1.0)
                .h(space::XL)
                .fill(PAL.hairline_soft)
                .a11y_hidden(),
        );
    }
    bar = bar.child(score).child(spacer());
    if width > size::FIRST_TO_MIN_W {
        bar = bar.child(
            text(copy::FIRST_TO)
                .key("first-to")
                .font(fty::FIRST_TO)
                .color(FELT.first_to),
        );
    }
    bar.child(super::panels::header_tools(settings_open, compact, false))
}

fn label(words: &'static str) -> El {
    text(words).font(fty::SCORE_LABEL).color(PAL.text_secondary)
}

/// A big score numeral (09 §4.8): its digits roll to a new value, clipped to the line; `since`
/// is the seconds since its roll landed, for the gold `PULSE` ring and the `NUM_POP`. Reduce
/// motion shows the number plainly.
pub(super) fn number(key: &str, n: u8, compact: bool, reduced: bool, since: Option<f32>) -> El {
    let font = if compact {
        fty::SCORE_COMPACT
    } else {
        fty::SCORE
    };
    let cell = el().key(key.to_owned()).min_w(space::L).center();
    if reduced {
        return cell.child(
            // One key for every value, so the new number lands in place with no crossfade.
            text(n.to_string())
                .key(format!("{key}-value"))
                .font(font)
                .color(PAL.ink),
        );
    }
    let digits = KIT.with(|k| {
        RollingText::new(format!("{key}-roll"), n.to_string())
            .value(f64::from(n))
            .font(font)
            .color(PAL.ink)
            .spring(motion::ROLL)
            .build(k)
    });
    let pop = since.map_or(1.0, |t| beat_scale(motion::NUM_POP, t));
    // The roll is clipped to one line (09 §4.8): digits rolling in or out fade at its top
    // and bottom edges instead of spilling over the header. The clip sits outside the pop,
    // and reaches past the cell's sides by the pop's growth, so the popped number is never cut.
    let line = font.line_px();
    let side = font.size * 0.2;
    let mut cell = cell.child(
        el().key(format!("{key}-clip"))
            .h(line)
            .px(side)
            .ml(-side)
            .mr(-side)
            .center()
            .clip_fade(0.0, CLIP_FADE, 0.0, CLIP_FADE)
            .child(
                el().key(format!("{key}-pop"))
                    .center()
                    .scale(pop)
                    // The pop is sampled on the frame clock, so its scale lands each frame as
                    // given. The digits keep their own roll: `RollingText` sets its spring on
                    // its row.
                    .motion(Spring::NONE)
                    .child(digits),
            ),
    );
    if since.is_some() {
        // Keyed by the value, so each landing replays the ring. It is as wide as the number
        // is tall, so at its widest it stays clear of the team labels either side.
        let d = font.size;
        cell = cell.child(
            el().key(format!("{key}-pulse-{n}"))
                .size(d, d)
                .radius(d / 2.0)
                .border(2.0, theme::color(motion::GOLD))
                .pin(None, None, None, None)
                .sequence(Sequence::new(motion::PULSE))
                .paint_order(-1)
                .a11y_hidden(),
        );
    }
    cell
}

/// The scale of `beats` at `t` seconds: the shape `Sequence` plays, sampled on the frame clock.
fn beat_scale(beats: &[rune::Beat], t: f32) -> f32 {
    let t = f64::from(t);
    let Some(last) = beats.last() else {
        return 1.0;
    };
    if t >= last.at {
        return last.scale;
    }
    beats
        .windows(2)
        .find(|p| t < p[1].at)
        .map_or(last.scale, |p| {
            let u = p[1]
                .easing
                .eval(((t - p[0].at) / (p[1].at - p[0].at)) as f32);
            p[0].scale + (p[1].scale - p[0].scale) * u
        })
}

/// `ScoreCards`: a six and a four whose lit pips count the score (06 §5.3).
pub fn score_cards(key: &str, score: u8) -> El {
    let k = size::SCORE_CARDS[0] / size::SCORE_VIEWBOX[0];
    let six_lit = usize::from(score.min(6));
    let four_lit = usize::from(score.saturating_sub(6));
    el().key(key.to_owned())
        .size(size::SCORE_CARDS[0], size::SCORE_CARDS[1])
        .role("img")
        .label(copy::score_cards_label(score))
        .child(card(k, 0.0, "6", &SIX, six_lit, "♥", PAL.score_heart))
        .child(card(k, 40.0, "4", &FOUR, four_lit, "♠", FELT.score_spade))
}

fn card(
    k: f32,
    x0: f32,
    rank: &'static str,
    pips: &[(f32, f32)],
    lit: usize,
    glyph: &'static str,
    color: Color,
) -> El {
    let mut c = el()
        .absolute(x0 * k, 1.0 * k)
        .size(35.0 * k, 48.0 * k)
        .radius(4.0 * k)
        .fill(PAL.card_white)
        .border(1.0, FELT.score_stroke)
        .a11y_hidden()
        .child(
            text(rank)
                .font(fty::SCORE_RANK)
                .color(color)
                .absolute(3.0 * k, 2.0 * k),
        );
    let pip = 14.0;
    for (i, &(x, y)) in pips.iter().enumerate() {
        c = c.child(
            col()
                .absolute(x * k - pip / 2.0, y * k - pip / 2.0)
                .size(pip, pip)
                .center()
                .opacity(if i < lit { 1.0 } else { UNLIT })
                .child(
                    text(glyph)
                        .font(fty::SCORE_PIP)
                        .color(color)
                        .align(Align::Center),
                ),
        );
    }
    c
}
