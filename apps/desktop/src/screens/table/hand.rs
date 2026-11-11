//! The hand area (06 §5.5 to §5.8): heading, instruction, the card rack with slot retention and
//! its keyboard, and the controls row (bid panel, Play or Discard, Play again).
//!
//! The controls row keeps its height in every phase, so the rack never resizes when bidding
//! starts or ends.

use euchre_engine::card_name;
use euchre_protocol::{GamePhase, GameStatus, Move};
use rune::{
    Beat, Cursor, Easing, El, Key, KeyEvent, Sequence, Spring, SpringPair, Transition, col, el,
    row, text,
};

use euchre_engine::effective_suit;

use super::choreo::{Feel, ShotKey, shake_x};
use super::copy;
use super::hint;
use super::stage::{Ctx, PICKUP, deck_key, face, ring_waits};
use crate::app::{AppState, Route};
use crate::screens::notice;
use crate::theme::felt::{size, timing, ty as fty};
use crate::theme::{self, FELT, PAL, card, motion, radius, space, table as t, ty};

/// The hand area, `height` tall, with `overlay` (the outcome banner) laid over its top.
pub fn hand_area(cx: &Ctx<'_>, height: f32, rack_size: [f32; 2], overlay: Option<El>) -> El {
    let view = cx.view;
    let store = &cx.table.store;
    let paused = cx.paused();
    let instruction = if paused {
        copy::PAUSED_LINE.to_owned()
    } else if store.submitting() {
        copy::WAITING_FOR_CONFIRM.to_owned()
    } else if let Some((line, _)) = &cx.table.present.illegal {
        line.clone()
    } else if cx.table.present.dealing(view.hand_no) {
        // 09 §4.1: whose turn it is shows once the cards have landed.
        copy::DEALING.to_owned()
    } else if ring_waits(cx) {
        // 09 §4.4: the hand area names the trick's winner on the ring's beat, as the felt's
        // result line does; until then it reads as if the trick had no winner yet.
        let mut before = view.clone();
        before.trick.winner_seat = None;
        copy::turn_instruction(&before)
    } else {
        copy::turn_instruction(view)
    };
    let mut area = col()
        .key("hand-area")
        .w_full()
        .max_w(t::HAND_MAX_W)
        .self_center()
        .h(height)
        .px(space::M)
        .pt(space::S)
        .gap(if cx.compact { space::XS } else { space::S })
        .role("region")
        .label("Your hand and controls")
        .border_sides([1.0, 0.0, 0.0, 0.0], FELT.hand_border);
    let heading = if paused {
        copy::H_PAUSED
    } else {
        copy::hand_heading(view, store.submitting())
    };
    let h1 = text(heading)
        .key("hand-h1")
        .font(if cx.compact {
            fty::HAND_H1_COMPACT
        } else {
            ty::HAND_H1
        })
        .color(PAL.felt_text)
        .role("heading")
        .level(1)
        .max_lines(1);
    let line = text(instruction)
        .key("hand-instruction")
        .font(if cx.compact {
            fty::INSTRUCTION_COMPACT
        } else {
            ty::INSTRUCTION
        })
        .color(PAL.felt_label)
        .role("status")
        .live_region(rune::LiveRegion::Polite);
    // While the match-end panel shows, it carries the heading and the one Play again (09
    // §4.10): the hand area's own fade out under the scrim and keep their place, so the rack
    // does not move.
    let panel = cx.table.present.match_end_shown(view);
    let (h1, line) = if panel {
        let gone = |e: El| {
            e.opacity(0.0)
                .opacity_motion(motion::NAME_FADE)
                .a11y_hidden()
        };
        (gone(h1), gone(line))
    } else {
        (h1, line)
    };
    // At game over Play again sits at the end of the heading row; the controls row below
    // keeps its height, so the rack does not move.
    let again = (view.status != GameStatus::Active && !panel).then(|| {
        felt_button("hand-again", copy::PLAY_AGAIN, true, cx.compact)
            .on_press_tx(|s: &mut AppState, tx| s.navigate(Route::NewGame, tx))
    });
    if cx.compact {
        // A short window keeps the heading, inline with the instruction.
        let mut heading = row()
            .key("hand-heading")
            .w_full()
            .gap(space::SM)
            .center_y()
            .child(h1)
            .child(line.grow(1.0).w(0.0).max_lines(1).truncate());
        if let Some(b) = again {
            heading = heading.child(b);
        }
        area = area.child(heading);
    } else {
        let mut heading = row()
            .key("hand-heading")
            .w_full()
            .gap(space::SM)
            .center_y()
            .child(h1.grow(1.0).w(0.0));
        if let Some(b) = again {
            heading = heading.child(b);
        }
        area = area.child(heading).child(line.max_lines(2));
    }
    area = area.child(rack(cx, rack_size)).child(controls(cx));
    if let Some(layer) = hand_result_layer(cx) {
        area = area.child(layer);
    }
    if let Some(layer) = overlay {
        area = area.child(layer);
    }
    if let Some(line) = store.error().or(store.review_error()) {
        area = area.child(
            notice("hand-error", line)
                .role("alert")
                .pin(Some(space::M), None, Some(space::M), Some(12.0))
                .raise(),
        );
    }
    area
}

/// The hand-result card's type (09 §4.8).
const RESULT_TITLE: rune::TextStyle = rune::TextStyle::new(28.0, 600.0).family(theme::SERIF);
const RESULT_LINE: rune::TextStyle = rune::TextStyle::new(17.0, 400.0).lh(24.0);
/// The same in a short window, so the card fits the trick box.
const RESULT_TITLE_COMPACT: rune::TextStyle =
    rune::TextStyle::new(22.0, 600.0).family(theme::SERIF);
const RESULT_LINE_COMPACT: rune::TextStyle = rune::TextStyle::new(15.0, 400.0).lh(20.0);
const RESULT_CHIP: rune::TextStyle = rune::TextStyle::new(20.0, 700.0).features("'tnum' 1");
/// A trick pip on the card.
const RESULT_PIP: f32 = 12.0;
/// The points chip waits for the card to arrive.
const CHIP_DELAY: f64 = 0.18;
/// The points chip pops in once the card is up. A sequence, since the chip is inserted with
/// the card and a child's own entrance does not play under its parent's.
const CHIP_POP: &[Beat] = &[
    Beat::new(0.0).opacity(0.0).scale(0.8),
    Beat::new(0.22).easing(Easing::EASE_OUT),
];

/// The layer over the trick area that holds the hand-result card (09 §4.8) while the hand
/// scores. It stays mounted while the trick area is laid out, so the card leaves on its own
/// exit when the next hand deals.
fn hand_result_layer(cx: &Ctx<'_>) -> Option<El> {
    let [ax, ay, _, _] = rune::memo::layout("hand-area")?;
    let [tx, ty, tw, th] = rune::memo::layout("trick")?;
    let mut layer = el()
        .key("hand-result-layer")
        .absolute(tx - ax, ty - ay)
        .size(tw, th)
        .center()
        .raise();
    if let Some(card) = hand_result(cx, th) {
        layer = layer.child(card);
    } else if cx.table.present.match_end_shown(cx.view) {
        // The match-end panel takes the card's place (09 §4.10): the card goes at once, so
        // the two never show together.
        layer = layer.child_exit(motion::blink_out());
    }
    Some(layer)
}

/// The hand-result card (09 §4.8): the result word, who scored, the makers' tricks as five
/// pips and the points chip. It stays inside the trick box (`max_h` tall): a short window
/// sets it tighter, with the pips and the chip on one line.
fn hand_result(cx: &Ctx<'_>, max_h: f32) -> Option<El> {
    let view = cx.view;
    let copy = copy::hand_result(view)?;
    let hand = view.hand_no;
    // The card lands with the score (09 §4.8): not while the last trick is still on the felt.
    // The match-end panel takes its place when the match is won (09 §4.10).
    if cx.table.present.director.pending(&ShotKey::score(hand))
        || cx.table.present.match_end_shown(view)
    {
        return None;
    }
    // While the euchre or march stamp sits on the title, the title hides under it (both say
    // the same word) and fades back as the stamp leaves.
    let stamped = super::fx::stamp_live(cx);
    let compact = cx.compact;
    let mut card = col()
        .key(format!("hand-result-{hand}"))
        .padding(if compact { space::S } else { space::L })
        .gap(if compact { space::XS } else { space::S })
        .max_h(max_h)
        .items_center()
        .radius(18.0)
        .fill(PAL.cream)
        .border(1.0, rune::Color::WHITE.alpha(0.6))
        .role("status")
        .label(format!("{} {}", copy.title, copy.line))
        .transition(super::panels::panel_transition(cx.reduced))
        .child(
            text(copy.title)
                .key(format!("hand-result-{hand}-title"))
                .font(if compact {
                    RESULT_TITLE_COMPACT
                } else {
                    RESULT_TITLE
                })
                .color(PAL.ink)
                .opacity(if stamped { 0.0 } else { 1.0 })
                .opacity_motion(if stamped {
                    Spring::NONE
                } else {
                    motion::NAME_FADE
                })
                .a11y_hidden(),
        )
        .child(
            text(copy.line)
                .key(format!("hand-result-{hand}-line"))
                .font(if compact {
                    RESULT_LINE_COMPACT
                } else {
                    RESULT_LINE
                })
                .color(PAL.text_secondary)
                .a11y_hidden(),
        );
    // A short window sets the pips and the chip side by side.
    let mut marks = row()
        .key(format!("hand-result-{hand}-marks"))
        .gap(space::S)
        .center_y();
    if let Some(won) = copy.maker_tricks {
        let mut pips = row()
            .key(format!("hand-result-{hand}-pips"))
            .gap(6.0)
            .a11y_hidden();
        for i in 0..5u8 {
            let pip = el().size(RESULT_PIP, RESULT_PIP).radius(RESULT_PIP / 2.0);
            pips = pips.child(if i < won {
                pip.fill(theme::color(motion::GOLD_DEEP))
            } else {
                pip.border(1.5, theme::color(motion::EDGE))
            });
        }
        if compact {
            marks = marks.child(pips);
        } else {
            card = card.child(pips);
        }
    }
    if copy.points > 0 {
        let chip = row()
            .key(format!("hand-result-{hand}-chip"))
            .px(space::SM)
            .py(space::XS)
            .radius(999.0)
            .fill(theme::color(motion::GOLD))
            .a11y_hidden()
            .sequence(Sequence::new(CHIP_POP).delay(CHIP_DELAY))
            .child(
                text(format!("+{}", copy.points))
                    .font(RESULT_CHIP)
                    .color(PAL.ink),
            );
        if compact {
            marks = marks.child(chip);
        } else {
            card = card.child(chip);
        }
    }
    if compact {
        card = card.child(marks);
    }
    Some(motion::e5(card))
}

/// The key of `c`'s rack cell while the card is out of the hand: a play flyer's source
/// (09 §2.10). The hole keeps the card's own size, so the flyer leaves from where the card sat.
pub fn hole_key(c: euchre_protocol::Card) -> String {
    format!("hole-{c}")
}

/// The widest a rack card grows (09 §5.4).
const RACK_CARD_MAX: f32 = 140.0;
/// The rack card's share of the window width (09 §5.4).
const RACK_CARD_SHARE: f32 = 0.092;
/// Room over the cards, so a lifted card clears the instruction line.
const RACK_TOP: f32 = 6.0;
/// The trump pip's disc and glyph (09 §5.4).
const PIP: f32 = 20.0;
const PIP_FONT: rune::TextStyle = rune::TextStyle::new(12.0, 600.0);
/// A card you may not play now (09 §4.7: legal dim).
const DIM: f32 = 0.55;
/// A dealt card stays hidden until its turn, then shows in 0.1 s as it leaves the deck. A
/// sequence, not a transition: the plate is inserted with its cell, and a child's own entrance
/// does not play under a parent's.
pub(super) const DEAL_SHOW: &[Beat] = &[
    Beat::new(0.0).opacity(0.0),
    Beat::new(0.10).easing(Easing::EASE_OUT),
];

/// The rack card width a window `window_w` wide asks for (09 §5.4): `clamp(112, 0.092 W,
/// 140)`, before the rack's own room caps it.
pub fn rack_card_w(window_w: f32) -> f32 {
    (RACK_CARD_SHARE * window_w).clamp(card::HAND_W, RACK_CARD_MAX)
}

/// One rack card's lift: plate offset y, plate scale, elevation and gold glow (09 §4.11; the
/// −10 / −18 lifts are owner item S-1). Reduce motion keeps the card in place; the glow still
/// marks the selection.
fn lift(selected: bool, hovered: bool, reduced: bool) -> (f32, f32, f32, f32) {
    let glow = if selected { 0.6 } else { 0.0 };
    let (dy, k, l) = if selected {
        (-18.0, 1.06, 0.65)
    } else if hovered {
        (-10.0, 1.04, 0.45)
    } else {
        (0.0, 1.0, 0.15)
    };
    if reduced {
        (0.0, 1.0, l, glow)
    } else {
        (dy, k, l, glow)
    }
}

/// The deal's delay scale while this hand's cards deal in from the deck (1 Full, 0.5
/// Brisk), or `None` when they mount at rest (reduce motion, Skim, Snap, no deal).
pub(super) fn deal_pace(cx: &Ctx<'_>) -> Option<f32> {
    let present = &cx.table.present;
    let view = cx.view;
    // The deal's own tier, once its shot is live; the tier in force while it waits.
    let feel = present
        .director
        .feel_of(&ShotKey::deal(view.hand_no))
        .unwrap_or_else(|| present.director.feel());
    let deal = present.deal_in == Some(view.hand_no)
        && !cx.reduced
        && matches!(feel, Feel::Full | Feel::Brisk);
    deal.then_some(if feel == Feel::Brisk { 0.5 } else { 1.0 })
}

/// Where this hand's deal leaves from, in window points: the deck tile, else the up card lying
/// on it, else where the deck last laid out.
pub(super) fn deal_source(cx: &Ctx<'_>) -> Option<[f32; 2]> {
    let hand = cx.view.hand_no;
    rune::memo::layout(&deck_key(hand))
        .or_else(|| rune::memo::layout(&format!("up-{hand}")))
        .or(cx.table.present.deck_at)
        .map(|[x, y, w, h]| [x + w / 2.0, y + h / 2.0])
}

/// The card rack (06 §5.6): one button per slot, sized to the space left.
fn rack(cx: &Ctx<'_>, [rw, rh]: [f32; 2]) -> El {
    let view = cx.view;
    let present = &cx.table.present;
    let enabled = cx.table.store.controls_enabled();
    let slots = &present.rack.slots;
    let n = slots.len().max(1) as f32;
    // One line under each card: why it is selected or held back, or its name while the pointer
    // is on it (09 §5.4). A short window keeps the line only while there is room for it.
    let sub_line = !cx.compact || cx.height >= size::SUB_LINE_MIN_H;
    let label_h = if !cx.compact {
        size::SUB_LINE_H + space::XS
    } else if sub_line {
        size::SUB_LINE_H + space::XS + 4.0
    } else {
        12.0
    };
    let gap = if cx.compact { 10.0 } else { card::HAND_GAP };
    let window_w = rune::memo::layout_size("root").map_or(theme::window::SIZE.0, |[w, _]| w);
    let w = rack_card_w(window_w)
        .min(rw.min(t::RACK_MAX_W) / n - gap)
        .min((rh - label_h - RACK_TOP).max(0.0) * card::ASPECT)
        .max(0.0);
    let h = w / card::ASPECT;
    // Room for the name ("Queen of diamonds") without pushing the cards apart.
    let cell_w = (w + 3.0 * gap).min(rw.min(t::RACK_MAX_W) / n);
    let has_moves = copy::has_card_moves(view);
    let pace = deal_pace(cx);
    let deal = pace.is_some();
    let pace = pace.unwrap_or(1.0);
    let from = if deal { deal_source(cx) } else { None };
    let rack_at = if deal {
        rune::memo::layout("rack")
    } else {
        None
    };
    let shake = present.shake_t(cx.state.clock).filter(|_| !cx.reduced);
    let trump = view.trump;
    let coach = hint::for_view(view, &cx.state.prefs);
    let mut cards = row()
        .key(format!("rack-{}", view.hand_no))
        .w_full()
        .h_full()
        .justify_center()
        .items_start()
        .pt(RACK_TOP);
    for (i, slot) in slots.iter().enumerate() {
        let c = slot.card;
        if !slot.present {
            cards = cards.child(el().key(hole_key(c)).size(cell_w, h));
            continue;
        }
        let legal = enabled && copy::card_move(view, c).is_some();
        let selected = cx.table.selected == Some(c);
        let hovered = present.hover == Some(c) && legal && !selected;
        let dim = has_moves && !legal && enabled;
        let (dy, k, l, glow) = lift(selected, hovered, cx.reduced);
        let mut art = face(cx.state, c, w)
            .key(format!("card-{c}"))
            .matched(format!("hcard-{}-{c}", view.hand_no))
            .motion(if i >= 5 { PICKUP } else { motion::PLAY_BRISK })
            .a11y_hidden();
        // The elevation rides on the art, so a matched pickup carries its shadow with it.
        art = motion::card_shadows(art, l, glow);
        if selected {
            art = art.outline(3.0, 3.0, PAL.focus_felt);
        }
        if dim {
            art = art.opacity(DIM);
        }
        let dealing = (deal && i < 5).then(|| {
            let j = copy::deal_index(view.dealer_seat, view.you, i);
            let offset = match (from, rack_at) {
                (Some([sx, sy]), Some([rx, ry, rwid, _])) => {
                    // From the deck centre to this cell's card centre.
                    let x = rx + rwid / 2.0 + (i as f32 + 0.5 - n / 2.0) * cell_w;
                    let y = ry + RACK_TOP + h / 2.0;
                    Transition::offset(sx - x, sy - y)
                }
                // No deck laid out yet: the cards rise in place.
                _ => Transition::offset(0.0, timing::DEAL_OFFSET),
            };
            (offset, motion::deal_delay(j) * pace, j)
        });
        let mut plate = el()
            .key(format!("plate-{c}"))
            .size(w, h)
            .radius(radius::CARD)
            .origin(0.5, 1.0)
            .motion(motion::HOVER)
            .offset(0.0, dy)
            .scale(k)
            .a11y_hidden()
            .child(art);
        if dim {
            // The art fades over a plain face, not the felt.
            plate = plate.fill(PAL.card_white);
        }
        if let Some((_, t)) = shake.filter(|(s, _)| *s == c) {
            // Poll-driven (09 §4.7): the offset lands each frame, then springs home after. The
            // plate shakes, so the plain face under a dimmed card moves with it. An illegal
            // card is never lifted, so the shake owns the plate's offset.
            let x = shake_x(t);
            plate = plate.offset(x, dy).rotate(x * 0.004).instant_changes();
        }
        if let Some((_, delay, _)) = &dealing {
            plate = plate.sequence(Sequence::new(DEAL_SHOW).delay(f64::from(*delay)));
        }
        if let Some(suit) = trump.filter(|&s| effective_suit(c, Some(s)) == s) {
            plate = plate.child(
                el().key(format!("pip-{c}"))
                    .size(PIP, PIP)
                    .radius(PIP / 2.0)
                    .center()
                    .fill(FELT.maker_text)
                    .pin(None, Some(space::XS), Some(space::XS), None)
                    .child(
                        text(copy::glyph(suit))
                            .font(PIP_FONT)
                            .color(FELT.trump_glyph),
                    ),
            );
        }
        let mut button = col()
            .key(format!("hand-{c}"))
            .w(cell_w)
            .gap(space::XS)
            .items_center()
            .cursor(if legal {
                Cursor::Pointer
            } else {
                Cursor::Default
            })
            .focusable(true)
            .role("button")
            .checked(selected)
            .label(copy::rack_label(view, c, legal, !enabled))
            .press_scale(0.96)
            .on_press(move |s: &mut AppState| s.press_card(c))
            .on_double_press(move |s: &mut AppState| s.play_card(c))
            .on_hover_change(move |s: &mut AppState, inside, _tx| s.hover_card(c, inside))
            .child(plate);
        // The web mirror's button is disabled for a card that cannot be played (S6); the
        // window keeps it pressable for the illegal-press shake.
        if crate::web_history::IS_WEB && !legal {
            button = button.disabled(true);
        }
        if let Some(badge) = coach.as_ref().and_then(|h| hint::badge(c, h)) {
            button = button.child(badge);
        }
        if sub_line {
            button = button.child(card_line(c, selected, dim, hovered));
        }
        if let Some((offset, delay, j)) = dealing {
            // The cell travels from the deck on the deal spring (09 §4.1, mechanism C); the
            // plate shows quickly as it leaves, so the waiting cards are not seen on the deck.
            button = button
                .transition(
                    offset.springs(SpringPair::new(motion::DEAL_CARD, motion::DEAL_CARD_OUT)),
                )
                .enter_delay(delay)
                .paint_order(20 - j as i16);
        }
        cards = cards.child(button);
    }
    // A card you played leaves the rack the commit its flight starts (09 §4.3): the flyer or
    // the matched flight carries it from there, so the rack's copy goes at once instead of
    // fading beside it. A discard keeps the ordinary exit.
    let played = view
        .trick
        .plays
        .iter()
        .any(|p| p.seat == view.you && slots.iter().any(|s| s.card == p.card && !s.present));
    if played {
        cards = cards.child_exit(Transition::none());
    }
    let rack = col()
        .key("rack")
        .w_full()
        .max_w(t::RACK_MAX_W)
        .self_center()
        .grow(1.0)
        .h(0.0)
        .role("group")
        .label("Your cards")
        .on_key(|s: &mut AppState, e: &KeyEvent, tx| rack_key(s, e, tx))
        .child(cards);
    // Paused: the hand stays in view, dimmed, until the table is back.
    let rack = if cx.paused() {
        rack.opacity(DIM).opacity_motion(motion::NAME_FADE)
    } else {
        rack
    };
    // The rack follows the seats in the table's build-in (09 §4.12).
    cx.build_in(rack, motion::BUILD_IN_RACK)
}

/// The one line under a rack card (09 §5.4): why it is selected or held back, else its name
/// while the pointer is on it. The line keeps its height when empty, so nothing moves.
fn card_line(c: euchre_protocol::Card, selected: bool, dim: bool, hovered: bool) -> El {
    let (line, font, color) = if selected {
        ("Selected ✓".to_owned(), fty::CARD_SUB_ON, FELT.card_sub)
    } else if dim {
        ("Must follow suit".to_owned(), fty::CARD_SUB, FELT.card_sub)
    } else if hovered {
        (card_name(c), ty::CARD_LABEL, FELT.card_label)
    } else {
        (String::new(), fty::CARD_SUB, FELT.card_sub)
    };
    text(line)
        .key(format!("hand-{c}-sub"))
        .font(font)
        .color(color)
        .max_lines(1)
        .min_h(size::SUB_LINE_H)
}

/// Left/Right (wrapping), Home and End move focus along the rack; Return on a selected card
/// plays it (a first Return selects, as the button's own press).
fn rack_key(s: &mut AppState, e: &KeyEvent, tx: &mut rune::Tx) -> bool {
    let Some(table) = s.table.as_ref() else {
        return false;
    };
    let cards: Vec<_> = table.present.rack.present().collect();
    if cards.is_empty() {
        return false;
    }
    let focused = tx
        .focused()
        .and_then(|k| k.strip_prefix("hand-"))
        .and_then(euchre_protocol::Card::parse);
    let at = focused.and_then(|c| cards.iter().position(|&x| x == c));
    let to = match (&e.key, at) {
        (Key::Left, Some(i)) => Some((i + cards.len() - 1) % cards.len()),
        (Key::Right, Some(i)) => Some((i + 1) % cards.len()),
        (Key::Home, _) => Some(0),
        (Key::End, _) => Some(cards.len() - 1),
        (Key::Enter, Some(i)) if table.selected == Some(cards[i]) => {
            s.play_card(cards[i]);
            return true;
        }
        _ => None,
    };
    match to {
        Some(i) => {
            tx.focus(format!("hand-{}", cards[i]), true);
            true
        }
        None => false,
    }
}

/// The controls row: the bid panel, the confirm button, or Play again.
fn controls(cx: &Ctx<'_>) -> El {
    let view = cx.view;
    let mut bar = row()
        .key("hand-controls")
        .w_full()
        .h(if cx.compact {
            size::CONTROLS_H.1
        } else {
            size::CONTROLS_H.0
        })
        .mb(space::S)
        .justify_center()
        .center_y();
    if view.status != GameStatus::Active {
        // Play again lives in the heading row; this row only keeps its height.
        return bar;
    }
    if let Some(panel) = bid_panel(cx) {
        return bar.child(panel);
    }
    if matches!(
        view.phase,
        GamePhase::TrickPlay | GamePhase::TrickResolve | GamePhase::DealerDiscard
    ) {
        let store = &cx.table.store;
        // Play or Discard waits until a card is selected (09 §5.4); the row keeps its height.
        let Some(selected) = cx.table.selected.filter(|c| view.hand.contains(c)) else {
            return bar;
        };
        let ready = store.controls_enabled() && copy::card_move(view, selected).is_some();
        let label = copy::play_button_label(view, Some(selected), store.submitting());
        let mut button = felt_button("hand-play", &label, ready, cx.compact)
            .min_w(t::BUTTON_MIN_W)
            .transition(if cx.reduced {
                Transition::opacity().springs(motion::BUTTON_IN)
            } else {
                Transition::offset(0.0, motion::RISE)
                    .combined(Transition::opacity())
                    .springs(motion::BUTTON_IN)
            });
        if ready {
            button = button.on_press(|s: &mut AppState| s.play_selected());
        }
        bar = bar.child(button);
    }
    bar
}

/// A cream button on the felt (06 §5.5).
fn felt_button(key: &str, label: &str, enabled: bool, compact: bool) -> El {
    let b = row()
        .key(key.to_owned())
        .min_h(if compact {
            space::PRIMARY_MIN
        } else {
            space::PRIMARY_MIN + 4.0
        })
        .px(18.0)
        .center()
        .radius(radius::BUTTON)
        .role("button")
        .label(label.to_owned())
        .disabled(!enabled)
        .child(
            text(label.to_owned())
                .font(ty::FELT_BUTTON)
                .color(if enabled {
                    PAL.ink
                } else {
                    FELT.button_off_text
                }),
        );
    if enabled {
        b.fill(PAL.felt_button)
            .border(1.0, PAL.felt_button_border)
            .hover(PAL.felt_button_hover)
            .motion(motion::KEY_PRESS)
            .press_scale(0.96)
            .cursor(Cursor::Pointer)
            .focusable(true)
    } else {
        b.fill(FELT.button_off_bg)
            .border(1.0, FELT.button_off_border)
    }
}

/// The bid panel (06 §5.8), when it is your decision in a bidding phase.
fn bid_panel(cx: &Ctx<'_>) -> Option<El> {
    let view = cx.view;
    if !copy::is_bid_panel_phase(view.phase) || view.turn_seat != Some(view.you) {
        return None;
    }
    let moves: Vec<_> = view
        .legal
        .iter()
        .filter(|m| !matches!(m.mv, Move::Play { .. } | Move::Discard { .. }))
        .collect();
    if moves.is_empty() {
        return None;
    }
    let enabled = cx.table.store.controls_enabled();
    let alone_on = cx.table.present.go_alone_on(view);
    let any_alone = moves.iter().any(|m| m.id.contains("+alone"));
    // The coach marks the suggested bid (06 §13); a card suggestion marks the rack instead.
    let coach = hint::for_view(view, &cx.state.prefs);
    let mut buttons = row()
        .key("bid-buttons")
        .grow(1.0)
        .w(0.0)
        .gap(10.0)
        .center_y();
    for m in moves.iter().filter(|m| !m.id.contains("+alone")) {
        let alone_id = format!("{}+alone", m.id);
        let id = if alone_on && view.legal.iter().any(|l| l.id == alone_id) {
            alone_id
        } else {
            m.id.clone()
        };
        let secondary = copy::bid_is_secondary(m);
        let label = copy::bid_label(view, m);
        let (bg, fg, border, hover) = if secondary {
            (
                FELT.bid_secondary,
                FELT.bid_secondary_text,
                FELT.bid_secondary_border,
                FELT.bid_secondary_hover,
            )
        } else {
            (
                PAL.felt_button,
                PAL.ink,
                PAL.felt_button_border,
                PAL.felt_button_hover,
            )
        };
        let mut b = row()
            .key(format!("bid-{}", m.id))
            .grow(1.0)
            .w(0.0)
            .h(if cx.compact { 46.0 } else { 56.0 })
            .px(space::M)
            .center()
            .radius(radius::BUTTON)
            .fill(bg)
            .border(1.0, border)
            .role("button")
            .label(label.clone())
            .disabled(!enabled)
            .child(
                text(label)
                    .font(if cx.compact {
                        fty::BID_COMPACT
                    } else {
                        fty::BID
                    })
                    .color(fg)
                    .max_lines(1),
            );
        b = if enabled {
            b.hover(hover)
                .motion(motion::KEY_PRESS)
                .press_scale(0.96)
                .cursor(Cursor::Pointer)
                .focusable(true)
                .on_press(move |s: &mut AppState| s.submit_move(&id))
        } else {
            b.opacity(size::BID_OFF)
        };
        if let Some(mark) = coach.as_ref().and_then(|h| hint::move_badge(&m.id, h)) {
            b = b.child(mark);
        }
        buttons = buttons.child(b);
    }
    let mut panel = row()
        .key("bid-panel")
        .w_full()
        .max_w(t::RACK_MAX_W)
        .gap(space::M)
        .center_y()
        .role("group")
        .label("Bidding")
        .child(buttons);
    if any_alone {
        panel = panel.child(go_alone(alone_on, enabled));
    }
    Some(panel)
}

/// The `Go alone` checkbox (06 §5.8).
fn go_alone(on: bool, enabled: bool) -> El {
    let mut boxed = col()
        .key("go-alone-box")
        .size(22.0, 22.0)
        .center()
        .radius(radius::CHECKBOX)
        .border(1.5, PAL.felt_button_border)
        .fill(if on { PAL.felt_button } else { FELT.led_pill })
        .a11y_hidden();
    if on {
        boxed = boxed.child(text("✓").font(fty::CHIP).color(PAL.ink));
    }
    let mut b = row()
        .key("go-alone")
        .min_h(48.0)
        .gap(space::S)
        .px(space::S)
        .center_y()
        .radius(radius::BUTTON)
        .role("checkbox")
        .checked(on)
        .label(copy::GO_ALONE)
        .disabled(!enabled)
        .child(boxed)
        .child(text(copy::GO_ALONE).font(fty::ALONE).color(PAL.felt_text));
    if enabled {
        b = b
            .cursor(Cursor::Pointer)
            .focusable(true)
            .hover(FELT.seat_tint)
            .on_press(|s: &mut AppState| s.toggle_go_alone());
    } else {
        b = b.opacity(size::BID_OFF);
    }
    b
}
