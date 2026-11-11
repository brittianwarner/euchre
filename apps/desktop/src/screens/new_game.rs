//! New game (06 §4): two house rules and the deal button.

use rune::{Cursor, Cx, El, ScrollSpec, Spring, col, el, row, text};

use crate::app::{AppState, Route};
use crate::screens::{green_button, text_link};
use crate::theme::{self, PAL, radius, space, ty};

/// Back link.
pub const BACK: &str = "← Your saved games";
/// Heading.
pub const H1: &str = "Your table. Your rules.";
/// Lede.
pub const LEDE: &str = "Traditional 24-card euchre. You and your partner play to 10.";
/// First rule: label and detail.
pub const STICK: (&str, &str) = (
    "Stick the dealer",
    "After three passes in round two, the dealer must name trump.",
);
/// Second rule: label and detail.
pub const NATURAL: (&str, &str) = (
    "Require a natural trump to call",
    "The left bower alone does not qualify. The dealer may count the up-card when picking it up.",
);
/// The note when both rules are on.
pub const BOTH_NOTE: &str =
    "The stuck dealer may call without a natural trump, so the hand can always proceed.";
/// Submit.
pub const SUBMIT: &str = "Deal me in →";
/// Footer line (06 §4: "in this browser" becomes "on this device").
pub const FOOTER: &str =
    "Take your time. Your turn waits for you, and your table is saved on this device.";

/// How an option card's border and box change colour when it is picked (09 §5.7).
const OPTION_SPRING: Spring = Spring::new(0.25, 0.2);
/// An option card's ring (09 §5.7: 2 pt).
const OPTION_RING: f32 = 2.0;
/// The start button shrinks to this while held (09 §5.7).
const SUBMIT_PRESS: f32 = 0.96;

/// Below this window height the form tightens so "Deal me in" stays above the fold.
const SHORT_H: f32 = 700.0;

/// The New game screen.
pub fn view(state: &AppState, cx: &mut Cx<AppState>) -> El {
    let form = state.new_game;
    let height = cx
        .layout_size("root")
        .map_or(theme::window::SIZE.1, |[_, h]| h);
    let short = height < SHORT_H;
    // Short windows: tighter padding and gaps, and the note reserves one line, not two.
    let (py, gap, outer, note_lines) = if short {
        (space::L, space::M, space::M, 1.0)
    } else {
        (space::NEW_PY, space::L, space::XXL, 2.0)
    };
    let mut panel = col()
        .key("new-panel")
        .w_full()
        .max_w(space::NEW_MAX_W)
        .self_center()
        .px(space::NEW_PX)
        .py(py)
        .gap(gap)
        .radius(radius::PANEL)
        .fill(PAL.cream)
        .child(row().child(text_link(
            "new-back",
            BACK,
            ty::BODY,
            PAL.ink,
            |s: &mut AppState, tx| {
                s.navigate(Route::Games, tx);
            },
        )))
        .child(
            col()
                .gap(space::SM)
                .child(
                    text(H1)
                        .key("new-h1")
                        .font(ty::NEW_H1)
                        .color(PAL.ink)
                        .role("heading")
                        .level(1),
                )
                .child(
                    text(LEDE)
                        .key("new-lede")
                        .font(ty::NEW_LEDE)
                        .color(PAL.ink)
                        .wrap(),
                ),
        )
        .child(
            col()
                .key("new-rules")
                .role("group")
                .label("House rules")
                .gap(space::S)
                .child(checkbox("new-stick", STICK, form.stick, |s| {
                    s.new_game.stick = !s.new_game.stick;
                }))
                .child(checkbox("new-natural", NATURAL, form.natural, |s| {
                    s.new_game.natural = !s.new_game.natural;
                })),
        );
    // The note keeps its line height whether it shows or not, so the button never jumps.
    let note = text(if form.stick && form.natural {
        BOTH_NOTE
    } else {
        ""
    })
    .key("new-both")
    .font(ty::CHECK_DETAIL)
    .color(PAL.text_secondary)
    .wrap()
    .min_h(ty::CHECK_DETAIL.line_height * note_lines)
    .live_region(rune::LiveRegion::Polite);
    panel = panel
        .child(note)
        .child(
            green_button("new-submit", SUBMIT, ty::SUBMIT, |s: &mut AppState, tx| {
                s.start_game(tx)
            })
            .press_scale(SUBMIT_PRESS)
            .w_full(),
        )
        .child(
            text(FOOTER)
                .key("new-footer")
                .font(ty::SMALL)
                .color(PAL.text_muted)
                .wrap(),
        );
    col()
        .key("new")
        .w_full()
        .h_full()
        .fill(PAL.cream)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(el().h(outer))
        .child(panel)
        .child(el().h(outer))
}

fn checkbox(key: &str, (label, detail): (&str, &str), on: bool, toggle: fn(&mut AppState)) -> El {
    let mut bx = row()
        .key(format!("{key}-box"))
        .size(space::CHECKBOX, space::CHECKBOX)
        .center()
        .radius(radius::CHECKBOX)
        .border(
            2.0,
            if on {
                PAL.green_button
            } else {
                PAL.disabled_border
            },
        )
        .fill(if on { PAL.green_button } else { PAL.paper });
    if on {
        bx = bx.child(
            text("✓")
                .font(ty::TOAST)
                .color(rune::Color::WHITE)
                .a11y_hidden(),
        );
    }
    // An option card: a gold ring on cream when picked, a faint one when not. The ring draws
    // over the fill and takes no layout space, so picking never moves anything.
    let gold = theme::color(theme::motion::GOLD_DEEP);
    let ring = if on {
        gold
    } else {
        PAL.disabled_border.alpha(0.45)
    };
    row()
        .key(key.to_owned())
        .w_full()
        .min_h(space::TAP)
        .gap(space::M)
        .items_start()
        .py(space::SM)
        .px(space::SM)
        .radius(radius::BUTTON)
        .fill(PAL.cream)
        .border(OPTION_RING, ring)
        .motion(OPTION_SPRING)
        .hover(PAL.panel_hover)
        .press_scale(0.98)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("checkbox")
        .label(label.to_owned())
        .description(detail.to_owned())
        .checked(on)
        .on_press(toggle)
        .child(bx.mt(1.0))
        .child(
            col()
                .grow(1.0)
                .w(0.0)
                .gap(space::XS)
                .child(
                    text(label.to_owned())
                        .font(ty::CHECK_LABEL)
                        .color(PAL.ink)
                        .wrap(),
                )
                .child(
                    text(detail.to_owned())
                        .font(ty::CHECK_DETAIL)
                        .color(PAL.text_secondary)
                        .wrap(),
                ),
        )
}
