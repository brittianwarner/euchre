//! The felt (06 §5.4): the rail, the status row, the four seats, the trick or the bid area, and
//! the result line with Continue.
//!
//! Shared-element ids (07 §5): a card keeps one `matched` id from where it starts to where it
//! ends, so the runtime flies it.
//! - Your cards: `hcard-{hand}-{card}` in the rack, in the trick and in the winner's stack.
//! - An opponent's next card: `play-{hand}-{trick}-{seat}` on the top back of its fan, then on
//!   the trick card, then on the mini in the winner's stack.
//! - The up card: `hcard-…` when you deal (it lands in your rack), else `pickup-{hand}` (it
//!   lands on the dealer's fan). In round 2 the deck tile takes the up card's id, so the
//!   turned-down card settles into the deck.

use std::f32::consts::PI;
use std::sync::LazyLock;

use euchre_engine::{card_name, currently_winning, suit_name};
use euchre_protocol::{Card, GamePhase, GameStatus, Play, PublicGameView, Seat, Suit};
use rune::{
    Beat, Color, Cursor, El, LoopSpec, Pixels, Sequence, Shadow, Span, Spring, SpringPair,
    SurfaceFit, TextStyle, Transition, col, el, row, text,
};

use super::choreo::{Feel, PlaySlot, ShotKey};
use super::copy::{self, ResultLine};
use super::present::Presentation;
use crate::app::{AppState, TableScreen};
use crate::theme::felt::{shadow as fshadow, size, timing, ty as fty};
use crate::theme::motion;
use crate::theme::{self, FELT, PAL, card, radius, space, table as t, ty};

/// What every felt piece reads.
pub struct Ctx<'a> {
    /// The app.
    pub state: &'a AppState,
    /// The table.
    pub table: &'a TableScreen,
    /// The displayed view.
    pub view: &'a PublicGameView,
    /// Reduce motion for this commit.
    pub reduced: bool,
    /// Short window (< 640 tall).
    pub compact: bool,
    /// The window's height.
    pub height: f32,
    /// The felt surface's last laid-out size.
    pub felt: [f32; 2],
    /// The trick (or bid) area's last laid-out size.
    pub trick: [f32; 2],
}

impl Ctx<'_> {
    /// Whether the table ended and waits (its stage failed): the felt and the hand say
    /// "Paused." and the rack dims, under the banner that says why.
    pub(super) fn paused(&self) -> bool {
        matches!(self.table.stage, crate::app::TableStage::Failed(_))
    }

    /// The table's presentation (the gather, the banner, the director).
    pub(super) fn present(&self) -> &Presentation {
        &self.table.present
    }

    /// `e` as part of the table's build-in (09 §4.12, mechanism C): while the route to the
    /// table is fresh, what mounts rises 8 pt and fades in, `step` seconds after the wipe is
    /// about across. A scene or a later mount has no build-in and keeps its own entrance.
    /// Reduced: the fade only (Rune also turns the entrance into its short fade).
    pub(super) fn build_in(&self, e: El, step: f32) -> El {
        let since = self.state.clock - self.state.routed_at;
        if since >= motion::BUILD_IN_WINDOW {
            return e;
        }
        let wait = (motion::BUILD_IN_LEAD - since).max(0.0) as f32;
        let t = if self.reduced {
            Transition::opacity()
        } else {
            Transition::offset(0.0, motion::RISE).combined(Transition::opacity())
        };
        e.transition(t.springs(motion::BUILD_IN))
            .enter_delay(wait + step)
    }
}

/// The up-card pickup spring (09 §4.6, S-3: retuned to 0.55 s from 06 §6.3's 0.85 s).
pub const PICKUP: Spring = motion::PICKUP;

/// The matched id a played card carries.
pub fn play_id(view: &PublicGameView, hand_no: u32, index: u8, play: &Play) -> String {
    if play.seat == view.you {
        format!("hcard-{hand_no}-{}", play.card)
    } else {
        format!("play-{hand_no}-{index}-{}", play.seat.index())
    }
}

/// The up card's matched id.
fn up_id(view: &PublicGameView, up: Card) -> String {
    if view.dealer_seat == view.you {
        format!("hcard-{}-{up}", view.hand_no)
    } else {
        format!("pickup-{}", view.hand_no)
    }
}

/// A card face at width `w` (aspect 5:7), radius only: `clip_rounded` would hide its shadow.
pub fn face(state: &AppState, c: Card, w: f32) -> El {
    let h = w / card::ASPECT;
    let base = el().size(w, h).radius(radius::CARD).fill(PAL.card_white);
    match state.art.as_ref().and_then(|a| a.face(&c.to_string())) {
        Some(img) => base.pixels_fit(img.pixels(), SurfaceFit::Fill),
        None => base
            .center()
            .child(text(c.to_string()).font(ty::CARD_LABEL).color(PAL.ink)),
    }
}

/// A card back at `w` by `h`: the one back image everywhere (09 §5.2), its corners in step
/// with the face's.
fn back(state: &AppState, w: f32, h: f32) -> El {
    let base = el()
        .size(w, h)
        .radius((w * 0.1).clamp(2.5, radius::CARD))
        .fill(FELT.deck_dark)
        .border(1.0, FELT.deck_border);
    match &state.art {
        Some(a) => base.pixels_fit(a.back().pixels(), SurfaceFit::Fill),
        None => base,
    }
}

/// A stable hash of `n` into `0..=1`: the grain, the sparks and the confetti draw from it, so
/// every frame and every run is the same.
pub fn hash(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// The felt's grain: light and dark specks at a low alpha, made once and stretched over the
/// felt (09 §5.1 layer 5). Rune caches it, so the felt costs nothing per frame.
static GRAIN: LazyLock<Pixels> = LazyLock::new(|| {
    let (w, h, peak) = size::GRAIN;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for i in 0..w * h {
        let n = hash(i.wrapping_mul(2_654_435_761));
        let a = ((n - 0.5).abs() * 2.0).powf(2.2) * peak;
        let g = if n > 0.5 { 255 } else { 0 };
        rgba.extend_from_slice(&[g, g, g, a.round() as u8]);
    }
    let px = Pixels::new();
    px.set(w, h, rgba);
    px
});

/// The felt in its rail (06 §16.1, 09 §5.1): a lit wooden rail with a brass inlay, then the
/// felt: its radial nap, a lamp pool, the grain, a vignette, the club watermark, the rim, and
/// (once the match is lost) a cool wash.
pub fn felt(cx: &Ctx<'_>) -> El {
    let [fw, fh] = cx.felt;
    let rim = space::FELT_RIM_INSET;
    let fr = if cx.compact {
        radius::FELT
    } else {
        size::FELT_RADIUS
    };
    let reach = |share: [f32; 2]| [(fw * share[0]).max(1.0), (fh * share[1]).max(1.0)];
    // Each layer is its own element: one gradient per element.
    let layer = |key: &str| {
        el().key(key)
            .pin(Some(1.0), Some(1.0), Some(1.0), Some(1.0))
            .radius(fr - 1.0)
            .a11y_hidden()
    };
    let clear = Color::TRANSPARENT;
    let view = cx.view;
    let lost = view
        .winner_team
        .is_some_and(|team| team.index() != view.you.index() % 2);
    let mut surface = el()
        .key("felt")
        .w_full()
        .h_full()
        .radius(fr)
        .fill(PAL.felt_edge)
        .radial_gradient(
            [0.5, 0.46],
            reach(size::FELT_REACH),
            [
                (0.0, FELT.felt_hi),
                (0.45, PAL.felt_center),
                (1.0, PAL.felt_edge),
            ],
        )
        .border(1.0, PAL.brass.alpha(0.55))
        .role("region")
        .label("The table")
        .child(layer("felt-lamp").radial_gradient(
            [0.5, 0.42],
            reach(size::LAMP_REACH),
            [(0.0, FELT.lamp), (1.0, FELT.lamp.alpha(0.0))],
        ))
        .child(layer("felt-grain").pixels(&GRAIN))
        .child(layer("felt-vignette").radial_gradient(
            [0.5, 0.5],
            reach(size::VIGNETTE_REACH),
            [(0.0, clear), (0.62, clear), (1.0, FELT.vignette)],
        ))
        .child(
            // It dims under the bid copy, which sits over it, so the copy keeps its contrast.
            layer("felt-watermark")
                .center()
                .opacity(if copy::is_bidding(view.phase) {
                    size::WATERMARK_UNDER_COPY
                } else {
                    1.0
                })
                .opacity_motion(motion::UI)
                .child(
                    text("♣")
                        .font(TextStyle::new(size::WATERMARK, 400.0).lh(size::WATERMARK))
                        .color(PAL.felt_watermark),
                ),
        )
        // 09 §4.10 fallback: a lost match cools the felt by an overlay's opacity.
        .child(
            layer("felt-cool")
                .fill(FELT.cool)
                .opacity(if lost { 1.0 } else { 0.0 })
                .opacity_motion(motion::FELT_COOL),
        );
    // A short window drops the rim (06 §16.1 makes it optional) so the content can use
    // the room; otherwise the content sits clear of it.
    if !cx.compact {
        surface = surface.child(
            el().key("felt-rim")
                .pin(Some(rim), Some(rim), Some(rim), Some(rim))
                .radius(fr - rim / 2.0)
                .border(1.0, PAL.felt_rim)
                .a11y_hidden(),
        );
    }
    let surface = surface.child(content(cx));
    let rail_r = if cx.compact {
        radius::RAIL
    } else {
        size::RAIL_RADIUS
    };
    let inlay = size::RAIL_INLAY;
    el().key("felt-rail")
        .w_full()
        .grow(1.0)
        .h(0.0)
        .radius(rail_r)
        .fill(PAL.rail_bottom)
        .linear_gradient(
            PI,
            [
                (0.0, FELT.rail_hi),
                (0.12, PAL.rail_top),
                (1.0, PAL.rail_bottom),
            ],
        )
        .box_shadow(Shadow {
            y: 12.0,
            blur: 34.0,
            color: Color::BLACK.alpha(0.5),
            ..Default::default()
        })
        .padding(if cx.compact { space::S } else { space::RAIL })
        .child(
            el().key("felt-brass")
                .pin(Some(inlay), Some(inlay), Some(inlay), Some(inlay))
                .radius(rail_r - inlay)
                .border(space::BRASS_LINE, PAL.brass.alpha(0.7))
                .a11y_hidden(),
        )
        .child(surface)
}

fn content(cx: &Ctx<'_>) -> El {
    let view = cx.view;
    let side = (cx.felt[0] * size::SIDE_SEAT.1).min(size::SIDE_SEAT.0);
    let centre = if copy::is_bidding(view.phase) {
        bid_area(cx)
    } else {
        trick(cx)
    };
    let mut middle_col = col()
        .key("felt-centre")
        .grow(1.0)
        .w(0.0)
        .h_full()
        .child(centre);
    // Your seat block shows only while bidding; in play the trick box carries your marks
    // beside your slot, so the trick gets the height.
    if you_below(cx) {
        middle_col = middle_col.child(row().key("felt-you").w_full().justify_center().child(seat(
            cx,
            0,
            t::SEAT_W,
        )));
    }
    let middle = row()
        .key("felt-middle")
        .w_full()
        .grow(1.0)
        .h(0.0)
        .items_center()
        .child(seat(cx, 1, side))
        .child(middle_col)
        .child(seat(cx, 3, side));
    let (px, pt, pb) = size::CONTENT_PAD[usize::from(cx.compact)];
    let content = col()
        .key("felt-content")
        .w_full()
        .h_full()
        .px(px)
        .pt(pt)
        .pb(pb)
        .gap(space::XS)
        .child(status_row(cx))
        .child(middle)
        .child(result_row(cx));
    // In play your marks sit in the felt's bottom-left corner, clear of the trick.
    let content = if copy::is_bidding(view.phase) {
        content
    } else {
        content.child(you_marks(cx, [px, pb]))
    };
    match turn_marker(cx) {
        Some(marker) => content.child(marker),
        None => content,
    }
}

/// The turn marker (09 §4.5): one gold ring that travels from seat to seat on the `MARKER`
/// spring, so the eye follows the turn round the table. It rings the seat's plate (your marks
/// in play), and on your turn only a halo breathes under it (`TURN_HALO`, the one cycle), so
/// the table stays still while the others think. None while the cards deal, while a trick
/// shows its winner, or once the match is over. Reduced: the ring lands and the halo holds at
/// its still alpha.
fn turn_marker(cx: &Ctx<'_>) -> Option<El> {
    let view = cx.view;
    if view.status != GameStatus::Active
        || cx.present().dealing(view.hand_no)
        || view.trick.winner_seat.is_some()
    {
        return None;
    }
    let s = view.turn_seat?;
    let target = if s == view.you && !copy::is_bidding(view.phase) {
        "felt-you".to_owned()
    } else {
        format!("seat-{}", s.index())
    };
    let [ox, oy, _, _] = rune::memo::layout("felt-content")?;
    let [x, y, w, h] = rune::memo::layout(&target)?;
    let pad = TURN_RING_PAD;
    let mut marker = el()
        .key("turn-marker")
        .absolute(x - ox - pad, y - oy - pad)
        .size(w + 2.0 * pad, h + 2.0 * pad)
        .radius(size::PLATE_RADIUS + pad)
        .border(2.0, FELT.winner)
        .motion(motion::MARKER)
        .a11y_hidden();
    if s == view.you {
        let halo = el()
            .key("turn-halo")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radius(size::PLATE_RADIUS + pad)
            .box_shadow(Shadow {
                blur: 30.0,
                spread: 4.0,
                color: FELT.winner.alpha(0.75),
                ..Default::default()
            });
        marker = marker.child(if cx.reduced {
            halo.opacity(motion::TURN_HALO.still)
        } else {
            halo.cycle(motion::TURN_HALO)
        });
    }
    Some(marker)
}

/// How far the turn ring sits outside the seat's plate, points.
const TURN_RING_PAD: f32 = 3.0;

/// The status row (06 §5.4.1): trump, the partner's seat, tricks this hand.
/// Below this felt width the led pill sits under the trump line, and the partner's seat
/// stacks its fan under its name.
const NARROW_FELT: f32 = 900.0;

/// The eyebrow line's reserved height in a narrow felt, where the led pill rides it.
const LED_ROW_H: f32 = 24.0;

fn narrow_felt(cx: &Ctx<'_>) -> bool {
    cx.felt[0] > 0.0 && cx.felt[0] < NARROW_FELT
}

fn status_row(cx: &Ctx<'_>) -> El {
    let view = cx.view;
    let shown = trump_shown(cx);
    let (glyph, words) = copy::trump_words(shown);
    let font = if cx.compact {
        fty::TRUMP_COMPACT
    } else {
        fty::TRUMP
    };
    let line = match glyph {
        Some(g) => {
            let all = format!("{g} {words}");
            text(all)
                .font(font)
                .color(PAL.felt_text)
                .spans(vec![Span::new(0, g.len()).color(FELT.trump_glyph)])
                .label(words)
        }
        None => text(words).font(font).color(PAL.felt_text),
    };
    // 09 §4.6: the words are keyed by what they say. On the call, "Choosing trump" fades out
    // quickly, then the suit and its name grow in (`TRUMP_IN`, from the left edge) over a
    // brief gold flash.
    let mut words_el = row()
        .key(format!(
            "felt-trump-{}-{}",
            view.hand_no,
            shown.map_or("none", suit_name)
        ))
        .origin(0.0, 0.5)
        .transition(Transition::none());
    if shown.is_some() && trump_calls(cx) {
        let after = f64::from(motion::TRUMP_FADE);
        words_el = words_el
            .sequence(Sequence::new(motion::TRUMP_IN).delay(after))
            .child(
                el().key("felt-trump-flash")
                    .pin(
                        Some(-size::TRUMP_FLASH_PAD),
                        Some(-size::TRUMP_FLASH_PAD / 2.0),
                        Some(-size::TRUMP_FLASH_PAD),
                        Some(-size::TRUMP_FLASH_PAD / 2.0),
                    )
                    .radius(radius::PILL)
                    .fill(FELT.winner.alpha(0.22))
                    .box_shadow(Shadow {
                        blur: 18.0,
                        color: FELT.winner.alpha(0.55),
                        ..Default::default()
                    })
                    .sequence(Sequence::new(motion::FLASH).delay(after))
                    .a11y_hidden(),
            );
    }
    let line = row()
        .key("felt-trump-line")
        .child_exit(Transition::opacity().springs(motion::TRUMP_OUT))
        .child(words_el.child(line));
    let mut strong = row().gap(space::SM).center_y().child(line);
    // In a narrow felt the pill sits under the line, in flow: the partner's taller seat
    // sets the row's height there, so the pill shifts nothing.
    let narrow = narrow_felt(cx);
    let mut led_pill = None;
    if let Some(led) = view.trick.led_suit {
        let g = copy::glyph(led);
        let words = format!("↗ {g} Led");
        let small = words.len() - "Led".len();
        let pill = row()
            .key("felt-led")
            .px(10.0)
            .py(if narrow { 2.0 } else { space::XS })
            .radius(radius::PILL)
            .fill(FELT.led_pill)
            .label(format!("{} led", suit_name(led)))
            .child(
                text(words.clone())
                    .font(fty::LED)
                    .color(PAL.felt_text)
                    .spans(vec![Span::new(small, words.len()).size(13.0)])
                    .a11y_hidden(),
            );
        if narrow {
            led_pill = Some(pill);
        } else {
            strong = strong.child(pill);
        }
    }
    let hand_no = text(format!("Hand {}", view.hand_no + 1))
        .key("felt-hand-no")
        .font(fty::EYEBROW)
        .color(PAL.felt_label);
    // In a narrow felt the led pill rides the eyebrow line, whose height is reserved.
    let eyebrow = if narrow {
        let mut line = row()
            .key("felt-eyebrow")
            .min_h(LED_ROW_H)
            .gap(space::S)
            .center_y()
            .child(hand_no);
        if let Some(pill) = led_pill.take() {
            line = line.child(pill);
        }
        line
    } else {
        row().key("felt-eyebrow").child(hand_no)
    };
    let mut trump = col()
        .key("felt-trump")
        .grow(1.0)
        .w(0.0)
        .gap(2.0)
        .items_start()
        .child(eyebrow)
        .child(strong);
    if let Some(line) = copy::alone_line(view) {
        trump = trump.child(
            text(line)
                .key("felt-alone")
                .font(ty::SEAT_NAME)
                .color(FELT.lone),
        );
    }
    let team = view.you.index() % 2;
    // 09 §4.4: the tally counts the trick on the ring's beat, not when its last card leaves.
    let mut won = view.tricks_won;
    if ring_waits(cx)
        && let Some(w) = view.trick.winner_seat
    {
        won[w.index() % 2] = won[w.index() % 2].saturating_sub(1);
    }
    let (us, them) = (won[team].to_string(), won[1 - team].to_string());
    let tally = format!("Your team {us} · Other team {them}");
    let a = "Your team ".len();
    let b = a + us.len() + " · Other team ".len();
    let tricks = col()
        .key("felt-tricks-block")
        .grow(1.0)
        .w(0.0)
        .gap(2.0)
        .items_end()
        .child(
            text("Tricks this hand")
                .font(fty::EYEBROW)
                .color(PAL.felt_label),
        )
        .child(
            text(tally.clone())
                .key("felt-tricks")
                .font(fty::TRICKS)
                .color(PAL.felt_text)
                .spans(vec![
                    Span::new(a, a + us.len()).size(25.0).weight(600.0),
                    Span::new(b, tally.len()).size(25.0).weight(600.0),
                ]),
        );
    let wide = !cx.compact && !narrow_felt(cx);
    let partner_w = if wide {
        size::PARTNER_WIDE_W
    } else {
        t::SEAT_W
    };
    row()
        .key("felt-status")
        .w_full()
        .items_start()
        .child(trump)
        .child(seat(cx, 2, partner_w))
        .child(tricks)
}

/// One seat block (06 §5.4.2) at relative position `rel_pos`, `width` wide.
///
/// Name row: initial (hidden in a short window), the name, then the maker, lead and dealer
/// badges. The name's width is bounded so the row never leaves the block. Under it the
/// detail line, then the tokens row: the fan of backs and the taken stack, both reserved.
/// In a wide felt the partner lays out in one row, the tokens beside the name.
fn seat(cx: &Ctx<'_>, rel_pos: usize, width: f32) -> El {
    let view = cx.view;
    let s = copy::seat_at(view, rel_pos);
    let i = s.index();
    let one_row = rel_pos == 2 && !cx.compact && !narrow_felt(cx);
    let shows_fan = rel_pos != 0 && !cx.compact;
    let fan_w = size::FAN[0] + size::FAN[2] * 5.0;
    // Your block shows only while bidding, before any trick is taken, so it keeps no stack.
    let shows_stack = rel_pos != 0;
    let (pad_y, pad_x) = if cx.compact {
        size::PLATE_PAD_COMPACT
    } else if narrow_felt(cx) {
        size::PLATE_PAD_NARROW
    } else {
        size::PLATE_PAD
    };
    let tokens_w = match (shows_fan, shows_stack) {
        (true, true) => fan_w + size::SEAT_GAP + size::STACK[0],
        (true, false) => fan_w,
        (false, true) => size::STACK[0],
        (false, false) => 0.0,
    };
    let badges = usize::from(view.maker_seat == Some(s) && view.trump.is_some())
        + usize::from(copy::lead_seat(view) == Some(s))
        + usize::from(view.dealer_seat == s);
    // A narrow side seat drops the initial and gives the badges their own reserved line.
    // A tight seat gives its name the whole width (two lines if it must) and its badges a
    // line of their own, in a short window too: a name never truncates to its first letters.
    let tight = width < size::SEAT_TIGHT_W;
    let badge_line = tight;
    let initial_w = if cx.compact || tight {
        0.0
    } else {
        size::INITIAL + size::SEAT_GAP
    };
    let badge_w = if badge_line {
        0.0
    } else {
        badges as f32 * (size::CHIP + size::SEAT_GAP)
    };
    let text_w = if one_row {
        width - 2.0 * pad_x - tokens_w - space::SM
    } else {
        width - 2.0 * pad_x
    };
    let name_max = (text_w - initial_w - badge_w).clamp(size::NAME_MIN_W, size::NAME_MAX_W);
    // 09 §4.1: whose turn it is shows once the deal has landed.
    let held = cx.present().dealing(view.hand_no);
    let turn = !held && view.turn_seat == Some(s);
    let name = copy::player_name(view, s);
    let initial_color = if turn {
        FELT.initial_active
    } else if rel_pos == 2 {
        FELT.initial_partner
    } else {
        FELT.initial
    };
    let initial = name
        .chars()
        .next()
        .map_or_else(String::new, |c| c.to_string());
    // The row is bounded too: it measures the name at its natural width.
    let mut name_row = row()
        .key(format!("seat-{i}-row"))
        .max_w(text_w)
        .gap(size::SEAT_GAP)
        .center();
    if initial_w > 0.0 {
        name_row = name_row.child(
            col()
                .size(size::INITIAL, size::INITIAL)
                .center()
                .radius(radius::PILL)
                .fill(initial_color.alpha(0.18))
                .a11y_hidden()
                .child(text(initial).font(fty::INITIAL).color(initial_color)),
        );
    }
    name_row = name_row.child(
        text(name.clone())
            .key(format!("seat-{i}-name"))
            .font(ty::SEAT_NAME)
            .color(if turn {
                FELT.seat_name_active
            } else {
                PAL.felt_text
            })
            .max_w(name_max)
            .truncate()
            .max_lines(if badge_line { 2 } else { 1 })
            .description(match copy::player_blurb(view, s) {
                Some(b) => format!("{name} — {b}"),
                None => name.clone(),
            }),
    );
    let mut badge_row = row()
        .key(format!("seat-{i}-badges"))
        .gap(size::SEAT_GAP)
        .min_h(size::CHIP)
        .center();
    let mut add_badge = |b: El| {
        if badge_line {
            badge_row = std::mem::replace(&mut badge_row, row()).child(b);
        } else {
            name_row = std::mem::replace(&mut name_row, row()).child(b);
        }
    };
    if view.maker_seat == Some(s)
        && let Some(trump) = trump_shown(cx)
    {
        add_badge(maker_chip(cx, i, trump));
    }
    if copy::lead_seat(view) == Some(s) {
        add_badge(
            text("↗")
                .key(format!("seat-{i}-lead"))
                .font(ty::SEAT_NAME)
                .color(FELT.winner)
                .label("Leads this trick"),
        );
    }
    if view.dealer_seat == s {
        add_badge(dealer_chip(i));
    }
    let detail = if held {
        copy::SEAT_DEALING.to_owned()
    } else {
        copy::seat_detail(view, s, cx.table.thinking[i])
    };
    let shows_thinking = detail == "Thinking…";
    let words = if view.players.is_some() {
        format!("{} · {detail}", copy::role_word(view, s))
    } else {
        detail
    };
    let mut detail_row = row()
        .key(format!("seat-{i}-detail-row"))
        .gap(6.0)
        .center()
        .min_h(20.0)
        .child(
            text(words)
                .key(format!("seat-{i}-detail"))
                .font(ty::SEAT_DETAIL)
                .color(PAL.felt_label)
                .max_lines(1),
        );
    if shows_thinking {
        let mut dot = el()
            .key(format!("seat-{i}-thinking"))
            .size(6.0, 6.0)
            .radius(radius::PILL)
            .fill(FELT.initial_active)
            .a11y_hidden();
        if !cx.reduced {
            dot = dot.looping(LoopSpec::opacity(timing::THINKING, 0.0, 0.25, 1.0));
        }
        detail_row = detail_row.child(dot);
    }
    // The tokens: the fan a play flies from and the stack a trick flies into. Both keep
    // their space before they show, so nothing moves when they do.
    let tokens = (shows_fan || shows_stack).then(|| {
        let mut tokens = row()
            .key(format!("seat-{i}-tokens"))
            .gap(size::SEAT_GAP)
            .center_y();
        if shows_fan {
            tokens = tokens.child(fan(cx, s));
        }
        if shows_stack {
            tokens = tokens.child(stack(cx, s));
        }
        tokens
    });
    let mut words = col()
        .key(format!("seat-{i}-words"))
        .items_center()
        .gap(2.0)
        .child(name_row);
    if badge_line {
        words = words.child(badge_row);
    }
    // The plate (09 §5.2): a pinned first child, so its fill springs on its own and the
    // seat's contents keep theirs. It lights for the seat to act and for a trick's taker.
    let on = turn || winner_shown(cx) == Some(s);
    let plate = el()
        .key(format!("seat-{i}-plate"))
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .radius(size::PLATE_RADIUS)
        .fill(if on {
            FELT.seat_plate_on
        } else {
            FELT.seat_plate
        })
        .border(1.0, FELT.winner.alpha(if on { 0.5 } else { 0.0 }))
        .motion(motion::SEAT_PLATE)
        .a11y_hidden();
    let mut block = if one_row {
        row()
            .key(format!("seat-{i}"))
            .w(width)
            .center()
            .gap(space::SM)
            .child(plate)
            .child(words.child(detail_row))
            .children(tokens)
    } else if cx.compact {
        // No fan in a short window: the stack rides the detail line.
        col()
            .key(format!("seat-{i}"))
            .w(width)
            .items_center()
            .child(plate)
            .child(words.child(detail_row.children(tokens)))
    } else {
        col()
            .key(format!("seat-{i}"))
            .w(width)
            .items_center()
            .gap(2.0)
            .child(plate)
            .child(words.child(detail_row))
            .children(tokens)
    };
    block = block.px(pad_x).py(pad_y);
    if let Some(bubble) = banter_bubble(cx, s, copy::seat_banter(&cx.table.store, s)) {
        block = block.child(bubble);
    }
    // The build-in follows the wipe in from the right: the right seat, your partner, the left
    // seat, then you.
    let k = (7 - rel_pos) % 4;
    let stagger = rune::Stagger::new(motion::BUILD_IN_STAGGER);
    cx.build_in(block, stagger.delay(k, 4))
}

/// The banter bubble under `s`'s seat (06 §8 "Rune suggestion"): `line` is that seat's
/// visible banter, if any (the store applies the 6.5 s lifetime and the this-hand rule). The
/// bottom result line stays the accessible channel, so the bubble is `a11y_hidden`, and it
/// hangs from a zero-height strip at the block's foot, so it never pushes the seat. It rises
/// 4 pt as it fades in; under reduce motion it only fades.
fn banter_bubble(cx: &Ctx<'_>, s: Seat, line: Option<&str>) -> Option<El> {
    let line = line?;
    let id = cx
        .table
        .store
        .visible_table_talk()
        .map_or("talk", |t| t.msg_id.as_str());
    let [max_w, r] = size::BUBBLE;
    let bubble = col()
        .key(format!("bubble-{}-{id}", s.index()))
        .max_w(max_w)
        .px(space::SM)
        .py(6.0)
        .radius(r)
        .fill(FELT.bubble)
        .border(1.0, FELT.bubble_edge)
        .box_shadow(Shadow {
            y: 6.0,
            blur: 18.0,
            color: Color::BLACK.alpha(0.35),
            ..Default::default()
        })
        .transition(if cx.reduced {
            Transition::opacity()
        } else {
            Transition::offset(0.0, 4.0).combined(Transition::opacity())
        })
        .child(
            text(format!("\u{201c}{line}\u{201d}"))
                .font(fty::BUBBLE)
                .color(FELT.bubble_text)
                .wrap()
                .max_lines(2)
                .align(rune::Align::Center),
        );
    Some(
        col()
            .key(format!("seat-{}-bubble", s.index()))
            .pin(Some(0.0), None, Some(0.0), Some(0.0))
            .h(0.0)
            .items_center()
            .raise()
            .a11y_hidden()
            .child(el().h(space::XS))
            .child(bubble),
    )
}

fn chip(key: String) -> El {
    col()
        .key(key)
        .size(size::CHIP, size::CHIP)
        .center()
        .radius(radius::PILL)
        .fill(FELT.chip_bg)
}

/// An opponent's hand as a fan of backs (09 §5.2: 30x42, step 9, a 0.08 tilt per card about
/// the bottom centre). A back leaves only when its flyer launches: the fan keeps one more for
/// every play of this seat still queued. The top back carries the id of the seat's next card,
/// so a Brisk or Skim play flies from here, and the pickup lands here.
fn fan(cx: &Ctx<'_>, s: Seat) -> El {
    let view = cx.view;
    let director = &cx.present().director;
    // Under reduce motion no flyer draws, so no back waits for one (B3).
    let unlaunched = if cx.reduced {
        0
    } else {
        director.unlaunched_from(s)
    };
    let n = (usize::from(view.hand_counts[s.index()]) + usize::from(unlaunched)).min(FAN_MAX);
    let [bw, bh, step] = size::FAN;
    let mut fan = el()
        .key(format!("seat-{}-fan", s.index()))
        .size(bw + step * 5.0, bh)
        .a11y_hidden();
    if n == 0 {
        return fan;
    }
    let width = bw + step * (n as f32 - 1.0);
    let x0 = (bw + step * 5.0 - width) / 2.0;
    // 09 §4.1: while the hand deals, each back flies in from the deck on the deal spring, in
    // the round-robin order the rack uses, and shows only as it leaves the deck.
    let deal = super::hand::deal_pace(cx).map(|pace| {
        let from = super::hand::deal_source(cx);
        let at = rune::memo::layout(&format!("seat-{}-fan", s.index()));
        (pace, from.zip(at))
    });
    let pickup = format!("pickup-{}", view.hand_no);
    let next = if let Some(p) = director.unlaunched_play(s) {
        format!("play-{}-{}-{}", p.hand_no, p.index, s.index())
    } else if view.phase == GamePhase::DealerDiscard && view.dealer_seat == s {
        pickup.clone()
    } else {
        let played = view.trick.plays.iter().any(|p| p.seat == s);
        let index = view.trick.index + u8::from(played);
        format!("play-{}-{index}-{}", view.hand_no, s.index())
    };
    // While the pickup flies, the top back carries the up card's face, and turns on landing.
    let turn = (next == pickup)
        .then(|| pickup_turn(cx).zip(view.up_card))
        .flatten();
    // 09 §4.6: the picked-up card keeps the up card's size in flight. The top back is laid out
    // at that size, so the matched flight only moves it, and a scale about its bottom centre
    // (mechanism A) brings it down to a back's size in the flight's last stretch.
    let held = (next == pickup && !cx.reduced).then(|| pickup_size(cx));
    for k in 0..n {
        let tilt = fan_tilt(k, n);
        let top = k + 1 == n;
        let [w, h] = held.filter(|_| top).unwrap_or([bw, bh]);
        let art = match turn.filter(|_| top) {
            // Only the art changes at once: the back itself still flies.
            Some((u, up)) => el()
                .size(w, h)
                .child(pickup_art(cx.state, up, [w, h], u).instant_changes()),
            None => back(cx.state, w, h),
        };
        let mut b = art
            .absolute(x0 + step * k as f32 + (bw - w) / 2.0, bh - h)
            .rotate(tilt)
            .origin(0.5, 1.0)
            .box_shadow(Shadow {
                x: 1.0,
                y: 2.0,
                blur: 5.0,
                color: Color::BLACK.alpha(0.32),
                ..Default::default()
            });
        if let Some([w, _]) = held.filter(|_| top) {
            let grow = 1.0 + (bw / w - 1.0) * pickup_shrink(cx);
            b = b.scale(grow).scale_motion(Spring::NONE);
        }
        b = if top {
            let spring = if next == pickup {
                PICKUP
            } else {
                motion::PLAY_BRISK
            };
            b.key(format!("back-{next}"))
                .matched(next.clone())
                .motion(spring)
        } else {
            // Keyed by hand too, so the next deal mounts fresh backs that deal in (09 §4.1).
            b.key(format!("back-{}-{}-{k}", view.hand_no, s.index()))
        };
        if let Some((pace, geometry)) = deal.filter(|_| k < 5) {
            let j = copy::deal_index(view.dealer_seat, s, k);
            let offset = match geometry {
                Some(([sx, sy], [fx, fy, _, _])) => {
                    let (cx_, cy_) = (fx + x0 + step * k as f32 + bw / 2.0, fy + bh / 2.0);
                    Transition::offset(sx - cx_, sy - cy_)
                }
                None => Transition::offset(0.0, timing::DEAL_OFFSET),
            };
            let delay = motion::deal_delay(j) * pace;
            b = b
                .transition(
                    offset.springs(SpringPair::new(motion::DEAL_CARD, motion::DEAL_CARD_OUT)),
                )
                .enter_delay(delay)
                .sequence(Sequence::new(super::hand::DEAL_SHOW).delay(f64::from(delay)))
                .paint_order(20 - j as i16);
        }
        fan = fan.child(b);
    }
    fan
}

/// The most backs a fan draws.
pub(super) const FAN_MAX: usize = 6;

/// The tilt of back `k` of a fan of `n` (09 §5.2: `FAN_TILT` rad per card about the middle).
/// A played card's flyer leaves the top back at this tilt.
pub(super) fn fan_tilt(k: usize, n: usize) -> f32 {
    (k as f32 - (n as f32 - 1.0) / 2.0) * size::FAN_TILT
}

/// How far the picked-up card has turned face down on the dealer's fan, 0 to 1, while the
/// pickup shot runs; `None` once it has landed, or under reduce motion (09 §4.6). Until it
/// lands the top back shows the up card's face, so the flight keeps the card it carries.
fn pickup_turn(cx: &Ctx<'_>) -> Option<f32> {
    if cx.reduced {
        return None;
    }
    let d = &cx.present().director;
    let key = ShotKey::pickup(cx.view.hand_no);
    if d.pending(&key) {
        return Some(0.0);
    }
    let t = d.elapsed(&key, cx.state.clock)?;
    let u = ((t - motion::PICKUP_LAND) / motion::PICKUP_TURN).clamp(0.0, 1.0);
    (u < 1.0).then(|| motion::FLIP_EASE.eval(u))
}

/// The up card's size in the bid area, which the picked-up card keeps while it flies.
fn pickup_size(cx: &Ctx<'_>) -> [f32; 2] {
    let ch = (cx.trick[1] - 2.0 * BID_CARD_MARGIN).clamp(40.0, t::UP_CARD_H);
    [ch * card::ASPECT, ch]
}

/// How far the picked-up card has shrunk from the up card's size to a back's, 0 to 1: 0 until
/// the last `PICKUP_SHRINK` of the flight, 1 once it lands (or if no pickup shot ran).
fn pickup_shrink(cx: &Ctx<'_>) -> f32 {
    let d = &cx.present().director;
    let key = ShotKey::pickup(cx.view.hand_no);
    if d.pending(&key) {
        return 0.0;
    }
    let Some(t) = d.elapsed(&key, cx.state.clock) else {
        return 1.0;
    };
    let span = motion::PICKUP_LAND * motion::PICKUP_SHRINK;
    let u = ((t - (motion::PICKUP_LAND - span)) / span).clamp(0.0, 1.0);
    motion::FLIP_EASE.eval(u)
}

/// The picked-up card on the dealer's fan at turn `u`: its face, squeezed to an edge, then
/// its back widening again (the same turn as the up card's, 09 §4.2).
fn pickup_art(state: &AppState, up: Card, [w, h]: [f32; 2], u: f32) -> El {
    let wide = (w * (PI * u).cos().abs()).max(1.0);
    let art = el()
        .absolute((w - wide) / 2.0, 0.0)
        .size(wide, h)
        .radius((w * 0.1).clamp(2.5, radius::CARD));
    match (&state.art, u < 0.5) {
        (Some(a), true) => match a.face(&up.to_string()) {
            Some(img) => art.fill(PAL.card_white).pixels(img.pixels()),
            None => art.fill(PAL.card_white),
        },
        (Some(a), false) => art.fill(FELT.deck_dark).pixels(a.back().pixels()),
        (None, true) => art.fill(PAL.card_white),
        (None, false) => art.fill(FELT.deck_dark).border(1.0, FELT.deck_border),
    }
}

/// The taken-tricks stack and, during a gather, the minis the trick cards fly into.
fn stack(cx: &Ctx<'_>, s: Seat) -> El {
    let view = cx.view;
    let i = s.index();
    let shown = cx.present().has_stack(view, s);
    // 09 §4.4: when this seat's gather is all sent, the glyph re-keys and knocks once as the
    // last card lands.
    let landed = cx
        .present()
        .collect
        .as_ref()
        .filter(|c| c.winner == s && c.hand_no == view.hand_no && c.done() && !cx.reduced);
    let glyph = el()
        .key(match landed {
            Some(c) => format!("stack-glyph-{i}-{}-{}", c.hand_no, c.index),
            None => format!("stack-glyph-{i}"),
        })
        .size(size::STACK[0], size::STACK[1])
        .radius(3.0)
        .fill(FELT.stack)
        .border(1.5, FELT.stack_border)
        .linear_gradient(
            PI / 4.0,
            [
                (0.0, FELT.stack),
                (0.5, FELT.stack_stripe.alpha(0.35)),
                (1.0, FELT.stack),
            ],
        )
        .opacity(if shown { size::STACK_SHOWN } else { 0.0 })
        .transition(Transition::none());
    let glyph = if landed.is_some() {
        glyph.sequence(Sequence::new(motion::THUMP).delay(THUMP_AFTER))
    } else {
        glyph
    };
    let mut stack = el()
        .key(format!("stack-{i}"))
        .size(size::STACK[0], size::STACK[1])
        .a11y_hidden()
        .child(theme::shadows(glyph, &fshadow::STACK));
    if let Some(c) = &cx.present().collect
        && c.winner == s
        && c.hand_no == view.hand_no
    {
        for play in c.plays.iter().take(c.moved) {
            stack = stack.child(
                face(cx.state, play.card, size::MINI[0])
                    .key(format!("mini-{}", play.card))
                    .absolute(0.0, (size::STACK[1] - size::MINI[1]) / 2.0)
                    .radius(3.0)
                    .matched(play_id(view, c.hand_no, c.index, play))
                    .motion(motion::SWEEP)
                    // The taker's card leads the sweep and stays on top of the others.
                    .paint_order(i16::from(play.seat == c.winner))
                    .opacity(size::MINI_REST),
            );
        }
    }
    stack
}

/// The last sweep card lands this long after it is sent (`Collect::finish_at`), when the
/// stack knocks, on the same beat as `Cue::TrickTake`.
const THUMP_AFTER: f64 = motion::SWEEP_SECS;

/// Your marks in play, in the felt's bottom-left corner (`inset` from its left and bottom
/// edges, clear of the trick): "You", the lead arrow, the maker and dealer chips and your
/// taken stack. Your seat block shows only while bidding.
fn you_marks(cx: &Ctx<'_>, [left, bottom]: [f32; 2]) -> El {
    let view = cx.view;
    let s = view.you;
    let i = s.index();
    let mut marks = row()
        .key("felt-you")
        .gap(size::SEAT_GAP)
        .center_y()
        .pin(Some(left), None, None, Some(bottom))
        .role("group")
        .label("Your seat")
        .child(
            text(copy::seat_name(view, s))
                .key("felt-you-name")
                .font(ty::SEAT_DETAIL)
                .color(PAL.felt_label)
                .a11y_hidden(),
        );
    if copy::lead_seat(view) == Some(s) {
        marks = marks.child(
            text("↗")
                .key(format!("seat-{i}-lead"))
                .font(ty::SEAT_NAME)
                .color(FELT.winner)
                .label("Leads this trick"),
        );
    }
    if view.maker_seat == Some(s)
        && let Some(trump) = trump_shown(cx)
    {
        marks = marks.child(maker_chip(cx, i, trump));
    }
    if view.dealer_seat == s {
        marks = marks.child(dealer_chip(i));
    }
    marks.child(stack(cx, s))
}

/// The maker's chip: it knocks once (`THUMP`) as trump is called (09 §4.6). Keyed by hand, so
/// each call mounts it fresh.
fn maker_chip(cx: &Ctx<'_>, i: usize, trump: Suit) -> El {
    let red = matches!(trump, Suit::Hearts | Suit::Diamonds);
    let chip = chip(format!("seat-{i}-maker-{}", cx.view.hand_no))
        .label("Named trump")
        .child(
            text(copy::glyph(trump))
                .font(fty::CHIP)
                .color(if red { PAL.suit_red } else { FELT.maker_text })
                .a11y_hidden(),
        );
    if trump_calls(cx) {
        chip.sequence(Sequence::new(motion::THUMP))
    } else {
        chip
    }
}

/// The trump the felt shows now: the view's, once its `Trump` shot has started (09 §4.6:
/// "Choosing trump" holds until the call's beat). Reduce motion shows it at once.
fn trump_shown(cx: &Ctx<'_>) -> Option<Suit> {
    let view = cx.view;
    let waits = !cx.reduced && cx.present().director.pending(&ShotKey::trump(view.hand_no));
    view.trump.filter(|_| !waits)
}

/// Whether the trump call is playing now at full feel (the line's `TRUMP_IN`, its gold flash
/// and the maker chip's `THUMP`).
fn trump_calls(cx: &Ctx<'_>) -> bool {
    let d = &cx.present().director;
    let key = ShotKey::trump(cx.view.hand_no);
    !cx.reduced && d.started(&key).is_some() && d.feel_of(&key) == Some(Feel::Full)
}

fn dealer_chip(i: usize) -> El {
    theme::shadows(
        chip(format!("seat-{i}-dealer")).label("Dealer").child(
            text("D")
                .font(fty::CHIP)
                .color(FELT.dealer_text)
                .a11y_hidden(),
        ),
        &fshadow::DEALER,
    )
}

/// A trick card's width and height in a trick box `[bw, bh]`, in a window `height` tall.
fn played_size([bw, bh]: [f32; 2], height: f32) -> (f32, f32) {
    let cap = if height <= 850.0 {
        size::PLAYED_CAP.0
    } else {
        size::PLAYED_CAP.1
    };
    // A short window shrinks the trick first, so it never outgrows your rack's cards.
    let share = if height < size::COMPACT_H {
        size::PLAYED_H_SHARE_COMPACT
    } else {
        size::PLAYED_H_SHARE
    };
    let w = ((bh * share.0 - share.1) * card::ASPECT)
        .min(bw * t::PLAYED_W.0)
        .min(cap)
        .max(1.0);
    (w, w / card::ASPECT)
}

/// Where `seat`'s card rests in the trick (pure): its rect `[x, y, w, h]` in the space of
/// `trick_rect` (`[x, y, w, h]`; pass the `trick` box's window rect from `cx.layout("trick")`
/// for window points, or `[0, 0, w, h]` for box points), and its rotation in radians. `height`
/// is the window's height, which picks the card's size cap.
pub fn slot_rect(
    view: &PublicGameView,
    seat: Seat,
    trick_rect: [f32; 4],
    height: f32,
) -> ([f32; 4], f32) {
    let [ox, oy, bw, bh] = trick_rect;
    let (w, h) = played_size([bw, bh], height);
    let (x, y, turn) = t::SLOTS[(copy::rel(view, seat) + 3) % 4];
    (
        [
            ox + (bw * x - w / 2.0),
            oy + (slot_y(bh, y, h) - h / 2.0),
            w,
            h,
        ],
        turn,
    )
}

/// The deck tile's key in hand `hand_no` (the deal's source, 09 §2.10).
pub fn deck_key(hand_no: u32) -> String {
    format!("deck-{hand_no}")
}

/// The seat whose trick win shows now (09 §4.4, `Presentation::winner_shown`): the trick has
/// a winner and its `Win` shot is not waiting for the last card's touchdown.
pub(super) fn winner_shown(cx: &Ctx<'_>) -> Option<Seat> {
    let view = cx.view;
    view.trick
        .winner_seat
        .filter(|_| cx.present().winner_shown(view))
}

/// Whether the trick has a winner the table does not show yet: its `Win` shot waits for the
/// last card to land (09 §4.4). The result line and the tally hold their old words until then.
pub(super) fn ring_waits(cx: &Ctx<'_>) -> bool {
    cx.view.trick.winner_seat.is_some() && winner_shown(cx).is_none()
}

/// Whether the trick win's ring and pop play (a Win that ran with motion), rather than show
/// at rest.
fn win_animates(cx: &Ctx<'_>) -> bool {
    let view = cx.view;
    let key = ShotKey::win(view.hand_no, view.trick.index);
    !cx.reduced
        && cx
            .present()
            .director
            .feel_of(&key)
            .is_some_and(|f| f != Feel::Snap)
}

/// The winner ring (09 §4.4): a 3 pt gold border 6 pt outside a `w` by `h` card, a child of
/// the card so it turns and pops with it. It enters on `RING_IN` when the win animates.
pub(super) fn win_ring(key: String, w: f32, h: f32, animate: bool) -> El {
    let [width, gap] = size::RING;
    let ring = el()
        .key(key)
        .absolute(-gap, -gap)
        .size(w + 2.0 * gap, h + 2.0 * gap)
        .radius(radius::CARD + gap - 2.0)
        .border(width, FELT.winner)
        .transition(Transition::none())
        .a11y_hidden();
    if animate {
        ring.sequence(Sequence::new(motion::RING_IN))
    } else {
        ring
    }
}

/// Where the gather pulls the trick together (09 §4.4): the trick's centre, 18 pt toward the
/// winner's slot, while `Presentation::gathering` holds (from `GATHER_LEAD` before the sweep,
/// Full wins only, never under reduce motion). The holders spring there on `GATHER`.
fn gather(cx: &Ctx<'_>, w: f32, h: f32) -> Option<[f32; 2]> {
    let view = cx.view;
    if cx.reduced || !cx.present().gathering(view) {
        return None;
    }
    let winner = winner_shown(cx)?;
    let [bw, bh] = cx.trick;
    let ([x, y, _, _], _) = slot_rect(view, winner, [0.0, 0.0, bw, bh], cx.height);
    let (to_x, to_y) = (x + w / 2.0 - bw / 2.0, y + h / 2.0 - bh / 2.0);
    let len = (to_x * to_x + to_y * to_y).sqrt().max(1.0);
    let pull = size::GATHER[0];
    Some([bw / 2.0 + to_x / len * pull, bh / 2.0 + to_y / len * pull])
}

/// The trick (06 §5.4.3, 09 §4.3-§4.4): four tilted slots. Each seat keeps an empty holder
/// from the trick's first commit, so the gather's offset springs from where the card rests (a
/// fresh element would take its offset at once). A slot stays empty while a flyer owns its
/// card. The trick's taker gets the ring and the pop; the gather pulls the four together
/// before the sweep flies them to the stack.
fn trick(cx: &Ctx<'_>) -> El {
    let view = cx.view;
    let [bw, bh] = cx.trick;
    let (w, h) = played_size(cx.trick, cx.height);
    let present = cx.present();
    let director = &present.director;
    let collect = present.collect_for(view);
    let (hand, index) = (view.hand_no, view.trick.index);
    let winner = winner_shown(cx);
    let animate = win_animates(cx);
    let pull = gather(cx, w, h);
    // The plays a viewer sees now: a flyer's card has not landed yet. Under reduce motion no
    // flyer draws, so every slot shows (the frame reduce motion turns on, before the next poll
    // lands the director, B3).
    let visible: Vec<Play> = view
        .trick
        .plays
        .iter()
        .filter(|p| cx.reduced || !director.hides_slot(hand, index, p.seat))
        .copied()
        .collect();
    let leading = if view.trick.winner_seat.is_none() && visible.len() >= 2 {
        currently_winning(&visible, view.trump).map(|p| p.seat)
    } else {
        None
    };
    let n = view.trick.plays.len() as f32;
    let mut area = el()
        .key("trick")
        .w_full()
        .grow(1.0)
        .h(0.0)
        .role("group")
        .label("Cards played");
    // The taker's slot comes last, so its card sits on top as the four gather.
    let seats = Seat::ALL
        .into_iter()
        .filter(|s| winner != Some(*s))
        .chain(winner);
    for s in seats {
        if view.sitting_seat == Some(s) {
            continue;
        }
        let ([x, y, _, _], turn) = slot_rect(view, s, [0.0, 0.0, bw, bh], cx.height);
        let order = view.trick.plays.iter().position(|p| p.seat == s);
        let mut holder = el()
            .key(format!("slot-{hand}-{index}-{}", s.index()))
            .absolute(x, y)
            .size(w, h)
            .origin(0.5, 0.5)
            .transition(Transition::none());
        if let (Some([gx, gy]), Some(k)) = (pull, order) {
            let anim = motion::GATHER;
            holder = holder
                .offset(gx - (x + w / 2.0), gy - (y + h / 2.0))
                .offset_motion(anim)
                .rotate((k as f32 - (n - 1.0) / 2.0) * size::GATHER[1] - turn)
                .rotate_motion(anim);
        }
        let play = order.map(|k| view.trick.plays[k]);
        if let Some(play) = play
            && !collect.is_some_and(|c| c.has_moved(&play))
        {
            // Under reduce motion no flyer draws, so the card rests in its slot, even on the
            // one frame before the director, polled with the old setting, catches up (B3).
            let slot = match director.play_slot(hand, index, s) {
                PlaySlot::Hidden if cx.reduced => PlaySlot::Still,
                slot => slot,
            };
            if slot != PlaySlot::Hidden {
                let won = winner == Some(s);
                // 09 §4.4: the taker lifts off the felt with a gold glow under its ring.
                let (lift, glow) = if won { WON_LIFT } else { (0.0, 0.0) };
                let mut card = motion::card_shadows(face(cx.state, play.card, w), lift, glow)
                    .key(format!("trick-{hand}-{index}-{}", s.index()))
                    .rotate(turn)
                    .transition(Transition::none())
                    .role("img")
                    .label(copy::played_label(view, play.seat, play.card));
                if slot == PlaySlot::Matched {
                    // A matched card flies on its own spring when its holder moves, so it
                    // takes the gather's spring while the four pull together.
                    card =
                        card.matched(play_id(view, hand, index, &play))
                            .motion(if pull.is_some() {
                                motion::GATHER
                            } else {
                                motion::PLAY_BRISK
                            });
                }
                if leading == Some(s) {
                    card = card.outline(size::LEADING[0], 2.0, FELT.winner.alpha(size::LEADING[1]));
                }
                if won {
                    if animate {
                        card = card.sequence(Sequence::new(motion::WIN_POP));
                    }
                    card = card.child(win_ring(format!("win-ring-{hand}-{index}"), w, h, animate));
                }
                holder = holder.child(card);
            }
        }
        area = area.child(holder);
    }
    if let Some(sitting) = view.sitting_seat {
        let (x, y, _) = t::SLOTS[(copy::rel(view, sitting) + 3) % 4];
        area = area.child(
            text("Sitting out")
                .key("trick-sitting")
                .font(fty::SLOT_CAPTION)
                .color(PAL.felt_label)
                .absolute(bw * x - 40.0, bh * y - 10.0),
        );
    }
    area
}

/// The trick taker's elevation and gold glow (`motion::card_shadows`).
const WON_LIFT: (f32, f32) = (0.3, 0.7);

/// A slot's centre y in a box `bh` tall for a card `h` tall, kept clear of both edges.
fn slot_y(bh: f32, share: f32, h: f32) -> f32 {
    let edge = h / 2.0 + t::WINNER_OUTLINE[0] + t::WINNER_OUTLINE[1];
    (bh * share).min(bh - edge).max(edge.min(bh / 2.0))
}

/// The bid area (06 §5.4.4): the up card or the deck, with its context.
fn bid_area(cx: &Ctx<'_>) -> El {
    let view = cx.view;
    let [bw, bh] = cx.trick;
    let ch = (bh - 2.0 * BID_CARD_MARGIN).clamp(40.0, t::UP_CARD_H);
    let cw = ch * card::ASPECT;
    let tilt = -3.0 * PI / 180.0;
    let copy_text = copy::bid_area_copy(view);
    let art = match copy::up_card_showing(view) {
        Some(up) => up_card(cx, up, cw, ch, tilt),
        None => {
            let mut deck = col()
                .key(deck_key(view.hand_no))
                .size(cw, ch)
                .radius(radius::CARD)
                .rotate(tilt)
                .a11y_hidden()
                // The up card takes the deck's spot face down, so the swap shows nothing.
                .transition(Transition::none())
                .on_layout(|s: &mut AppState, r| s.deck_laid_out(r));
            // One back image everywhere (09 §5.2); the drawn tile only until the art loads.
            deck = match &cx.state.art {
                Some(a) => deck
                    .fill(FELT.deck_dark)
                    .outline(1.0, 0.0, FELT.deck_outline)
                    .pixels_fit(a.back().pixels(), SurfaceFit::Fill),
                None => deck
                    .border(size::DECK_BORDER, FELT.deck_border)
                    .outline(1.0, 0.0, FELT.deck_outline)
                    .linear_gradient(PI / 4.0, [(0.0, FELT.deck_light), (1.0, FELT.deck_dark)])
                    .center()
                    .child(text("♣").font(fty::DECK_CLUB).color(FELT.deck_club)),
            };
            if view.phase == GamePhase::BidRound2
                && let Some(up) = view.up_card
            {
                deck = deck.matched(up_id(view, up)).motion(PICKUP);
            }
            theme::shadows(deck, &fshadow::UP)
        }
    };
    // 09 §4.1: the deck (or the up card lying face down on it) knocks once as a Full-feel
    // deal starts. A new hand mounts it under a new key, so the knock plays once per deal.
    let art = if super::hand::deal_pace(cx) == Some(1.0) {
        art.sequence(Sequence::new(motion::THUMP))
    } else {
        art
    };
    // The copy takes the room beside the card (so the heading holds one line on a wide felt);
    // the explaining line drops when a narrow box could not hold it under a wrapped heading.
    // On a short felt your seat rides beside the copy instead of under it.
    let beside = copy::is_bidding(view.phase) && !cx.compact && !you_below(cx);
    let seat_room = if beside { t::SEAT_W + BID_GAP } else { 0.0 };
    let copy_w = (bw - cw - BID_GAP - seat_room - 2.0 * space::M).clamp(BID_COPY_W.0, BID_COPY_W.1);
    let show_line = !cx.compact && (copy_w >= BID_LINE_MIN_W || bh >= BID_LINE_MIN_H);
    let face_down = copy::up_card_showing(view).is_some() && flip_pose(cx).0 < 0.5;
    // While the deal is still on its way the up card has not turned: its eyebrow and line
    // wait for the flip to start, then fade in (the heading waits for the face).
    let flip_waits = copy::up_card_showing(view).is_some()
        && !cx.reduced
        && cx.present().director.pending(&ShotKey::flip(view.hand_no));
    let waiting = |e: El| {
        e.opacity(if flip_waits { 0.0 } else { 1.0 })
            .opacity_motion(if flip_waits {
                Spring::NONE
            } else {
                motion::NAME_FADE
            })
    };
    // A fixed width, not a cap: the card then holds its place as the copy changes (the deck
    // tile, the up card and the turned-down deck all sit on one spot).
    let mut words = col()
        .key("bid-copy")
        .gap(space::S)
        .w(copy_w)
        .shrink()
        .child(waiting(
            text(copy_text.eyebrow)
                .key("bid-eyebrow")
                .font(fty::BID_EYEBROW)
                .color(PAL.felt_label),
        ))
        .child(
            text(copy_text.heading)
                .key("bid-heading")
                .font(fty::BID_H2)
                .color(PAL.felt_text)
                .role("heading")
                .level(2)
                .wrap()
                // The card's name shows as the card turns, not before (its room is kept). It
                // hides at once, so the name never shows while the card lies face down.
                .opacity(if face_down { 0.0 } else { 1.0 })
                .opacity_motion(if face_down {
                    Spring::NONE
                } else {
                    motion::NAME_FADE
                }),
        );
    if show_line {
        words = words.child(waiting(
            text(copy_text.line)
                .key("bid-line")
                .font(fty::BID_P)
                .color(FELT.result)
                .wrap(),
        ));
    }
    let area = row()
        .key("trick")
        .w_full()
        .grow(1.0)
        .h(0.0)
        .gap(BID_GAP)
        .py(14.0)
        .center()
        .child(art)
        .child(words);
    if beside {
        area.child(seat(cx, 0, t::SEAT_W))
    } else {
        area
    }
}

/// The up card's turn now (09 §4.2): how far it has turned (`u`, 0 face down to 1 face up),
/// its glint (0 to 0.6), and whether the Full-tier flip is live (poll-driven this frame). It
/// lies face down while its flip waits for the deal; a Brisk, Skim or snapped flip, reduce
/// motion and a resync show the face at once.
fn flip_pose(cx: &Ctx<'_>) -> (f32, f32, bool) {
    if cx.reduced {
        return (1.0, 0.0, false);
    }
    let d = &cx.present().director;
    let key = ShotKey::flip(cx.view.hand_no);
    if d.pending(&key) {
        return (0.0, 0.0, false);
    }
    match (d.elapsed(&key, cx.state.clock), d.feel_of(&key)) {
        (Some(t), Some(Feel::Full)) => {
            let u = motion::FLIP_EASE.eval((t / motion::FLIP).clamp(0.0, 1.0));
            let g = (t - motion::FLIP) / motion::FLIP_GLINT;
            let glow = if (0.0..=1.0).contains(&g) {
                0.6 * (PI * g).sin()
            } else {
                0.0
            };
            (u, glow, true)
        }
        _ => (1.0, 0.0, false),
    }
}

/// The up card (09 §4.2, mechanism A): a fixed outer element that carries the pickup's
/// matched id, and its art, which squeezes to an edge and widens again as the card turns, the
/// face swapping in at half way. `.pixels` stretches, so the squeeze reads as a turn.
fn up_card(cx: &Ctx<'_>, up: Card, w: f32, h: f32, tilt: f32) -> El {
    let view = cx.view;
    let (u, glow, live) = flip_pose(cx);
    let wide = (w * (PI * u).cos().abs()).max(1.0);
    let lift = (PI * u).sin();
    let art = el()
        .key("up-art")
        .absolute((w - wide) / 2.0, 0.0)
        .size(wide, h)
        .radius(radius::CARD);
    let art = match (&cx.state.art, u >= 0.5) {
        (Some(a), true) => match a.face(&up.to_string()) {
            Some(img) => art.fill(PAL.card_white).pixels(img.pixels()),
            None => art.fill(PAL.card_white),
        },
        (Some(a), false) => art.fill(FELT.deck_dark).pixels(a.back().pixels()),
        (None, true) => art
            .fill(PAL.card_white)
            .center()
            .child(text(up.to_string()).font(ty::CARD_LABEL).color(PAL.ink)),
        (None, false) => art.fill(FELT.deck_dark).border(1.0, FELT.deck_border),
    };
    let outer = el()
        .key(format!("up-{}", view.hand_no))
        .size(w, h)
        .matched(up_id(view, up))
        .motion(PICKUP)
        .transition(Transition::none())
        .rotate(tilt)
        .scale(1.0 + 0.1 * lift)
        .role("img")
        .label(if u >= 0.5 {
            format!("Up card: {}", card_name(up))
        } else {
            "Up card, face down".to_owned()
        })
        .child(motion::card_shadows(art, lift.max(UP_CARD_LIFT), glow).instant_changes());
    if live { outer.instant_changes() } else { outer }
}

/// While bidding on a felt this tall or taller, your seat sits under the up card and its copy.
const YOU_BELOW_MIN_FELT_H: f32 = 340.0;

/// Whether your seat block sits under the bid area (bidding, not compact, a tall enough felt).
fn you_below(cx: &Ctx<'_>) -> bool {
    copy::is_bidding(cx.view.phase) && !cx.compact && cx.felt[1] >= YOU_BELOW_MIN_FELT_H
}

/// The gap between the up card and its copy.
const BID_GAP: f32 = 28.0;
/// The up card's room above and below in the bid area: it is the card the bid is about, so
/// it takes as much of the height as the tilt leaves (V14).
const BID_CARD_MARGIN: f32 = 8.0;
/// The up card's lift at rest, a step above the felt's cards (09 §5.3 ladder).
const UP_CARD_LIFT: f32 = 0.2;
/// The bid copy's width range.
const BID_COPY_W: (f32, f32) = (200.0, 440.0);
/// The copy width, or box height, that holds the explaining line under the heading.
const BID_LINE_MIN_W: f32 = 380.0;
const BID_LINE_MIN_H: f32 = 150.0;

/// The result line (06 §5.4.5), with Continue in review mode.
fn result_row(cx: &Ctx<'_>) -> El {
    // 09 §4.4: the line names the winner on the ring's beat; until then it reads as it did.
    let held;
    let view = if ring_waits(cx) {
        let mut before = cx.view.clone();
        before.trick.winner_seat = None;
        held = before;
        &held
    } else {
        cx.view
    };
    let talk = cx
        .table
        .store
        .visible_table_talk()
        .map(|t| (t.seat, t.text.as_str()));
    let result = copy::trick_result(view, talk);
    // 09 §4.8: the line is keyed by its words, so a new line enters (rises 8 pt and fades in)
    // once the old one has faded out where it was (`LINE_SWAP_GAP`), and a moment that keeps
    // the same words does not cross-fade them. Leaving lines drop out of flow, so the row keeps its
    // height.
    let moment = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        result.hash(&mut h);
        format!("result-{:016x}", h.finish())
    };
    let line = match result {
        ResultLine::Won { strong, next } => {
            let all = format!("{strong} {next}");
            text(all)
                .font(fty::RESULT)
                .color(FELT.result)
                .spans(vec![
                    Span::new(0, strong.len())
                        .size(18.0)
                        .weight(600.0)
                        .color(FELT.trick_won),
                ])
                .wrap()
                .align(rune::Align::Center)
        }
        ResultLine::Talk { speaker, quote } => {
            let all = format!("{speaker} {quote}");
            text(all)
                .font(fty::RESULT)
                .color(FELT.result)
                .spans(vec![
                    Span::new(0, speaker.len())
                        .weight(600.0)
                        .color(FELT.quote_speaker),
                ])
                .wrap()
                .max_lines(2)
                .max_w(680.0)
                .align(rune::Align::Center)
        }
        ResultLine::Plain(words) => text(words)
            .font(fty::RESULT)
            .color(FELT.result)
            .wrap()
            .align(rune::Align::Center),
    };
    let swap = if cx.reduced {
        Transition::opacity()
    } else {
        Transition::asymmetric(
            Transition::offset(0.0, motion::RISE).combined(Transition::opacity()),
            Transition::opacity(),
        )
    }
    .springs(motion::LINE_SWAP);
    let bar = row()
        .key("trick-result")
        .w_full()
        .min_h(if cx.compact {
            size::RESULT_H.1
        } else {
            size::RESULT_H.0
        })
        .gap(space::M)
        .center();
    // The match-end panel says who won; the felt's line goes quiet under it (09 §4.10). A
    // table that ended mid-hand is paused: no turn and no "take your time" on the felt; the
    // hand area and the banner say why (review fix PX-8).
    if cx.present().match_end_shown(cx.view) || cx.paused() {
        return bar.child_exit(motion::blink_out());
    }
    let mut bar = bar.child(
        row()
            .key("trick-result-line")
            .shrink()
            .center()
            .role("status")
            .live_region(rune::LiveRegion::Polite)
            .child(
                row()
                    .key(moment)
                    .shrink()
                    .center()
                    .transition(swap)
                    .enter_delay(motion::LINE_SWAP_GAP)
                    .child(line.key("trick-result-text")),
            ),
    );
    if copy::review_mode(view) && view.phase == GamePhase::TrickResolve {
        bar = bar.child(continue_button(cx));
    }
    bar
}

fn continue_button(cx: &Ctx<'_>) -> El {
    let view = cx.view;
    let collecting = cx
        .present()
        .collect_for(view)
        .is_some_and(|c| c.collecting());
    let enabled = view.awaiting_trick_review == Some(true)
        && !collecting
        && cx.table.store.is_connected()
        && !cx.table.store.review_busy();
    let label = if collecting {
        copy::COLLECTING
    } else {
        copy::CONTINUE
    };
    let b = row()
        .key("trick-continue")
        .min_h(if cx.compact {
            size::RESULT_H.1
        } else {
            size::RESULT_H.0
        })
        .px(space::M + 2.0)
        .center()
        .radius(radius::CARD)
        .fill(PAL.felt_button)
        .border(1.0, PAL.felt_button_border)
        .role("button")
        .label(label)
        .disabled(!enabled)
        .child(text(label).font(fty::CONTINUE).color(PAL.ink));
    if enabled {
        b.hover(PAL.felt_button_hover)
            .press_scale(0.98)
            .cursor(Cursor::Pointer)
            .focusable(true)
            .on_press(|s: &mut AppState| s.continue_trick())
    } else {
        b.opacity(size::CONTINUE_OFF)
    }
}

/// A confetti piece's one fall, then it stays clear (its slide loop wraps; this fade keeps
/// the second pass hidden).
const FALL_ONCE: &[Beat] = &[
    Beat::new(0.0),
    Beat::new(timing::CONFETTI as f64 - 0.3),
    Beat::new(timing::CONFETTI as f64).opacity(0.0),
];

/// The renderer-owned confetti fall (06 §6.5): 24 pieces in the five confetti colours drift
/// down over the window `[w, h]` once, with no frames from the view. Used where the canvas
/// burst has no frames to run on (a snapped match end, a lone march's banner, a march).
pub(super) fn confetti_fall(key: String, [width, height]: [f32; 2]) -> El {
    let mut layer = el()
        .key(key)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .a11y_hidden()
        .transition(Transition::none());
    let [pw, ph] = size::CONFETTI;
    let fall = (height + 2.0 * ph) / ph;
    for i in 0..size::CONFETTI_COUNT {
        let delay = f64::from((i % 6) as f32 * timing::CONFETTI_STAGGER);
        let drift = (hash(i as u32 * 31 + 7) - 0.5) * 8.0;
        let mut spec = LoopSpec::slide(f64::from(timing::CONFETTI), [0.0, 0.0], [drift, fall])
            .easing(rune::Easing::EASE_IN);
        spec.delay = delay;
        let x = ((i * 37) % 100) as f32 / 100.0;
        layer = layer.child(
            el().key(format!("piece-{i}"))
                .size(pw, ph)
                .rotate((i as f32 * 0.7) % PI)
                .absolute(x * width, -ph)
                .looping(spec)
                .transition(Transition::none())
                .child(
                    el().w_full()
                        .h_full()
                        .radius(1.5)
                        .fill(FELT.confetti[i % FELT.confetti.len()])
                        .sequence(Sequence::new(FALL_ONCE).delay(delay)),
                ),
        );
    }
    layer
}
