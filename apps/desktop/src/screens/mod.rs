//! The screens, plus the few controls they share.

pub mod home;
pub mod new_game;
pub mod other;
pub mod server;
pub mod table;

use rune::{Color, Cursor, Edge, El, Span, SpringPair, Transition, Tx, row, text};

use crate::app::{AppState, RouteDir};
use crate::theme::{PAL, motion, radius, space, ty};

/// The `euchre♣` wordmark: serif word, smaller club with a little air before it.
pub fn wordmark(
    key: &str,
    word: rune::tok::TextStyle,
    club: rune::tok::TextStyle,
    color: Color,
) -> El {
    let label = "euchre♣";
    let at = "euchre".len();
    text(label)
        .key(key.to_owned())
        .font(word)
        .color(color)
        .label("Euchre")
        .spans(vec![Span::new(at, label.len()).size(club.size)])
}

/// The web table's holding panel: the window is under the table's minimum size.
pub fn holding_panel() -> El {
    rune::col()
        .key("holding")
        .w_full()
        .h_full()
        .center()
        .gap(space::M)
        .padding(space::L)
        .child(
            text("This table needs a bigger window.")
                .key("holding-line")
                .font(ty::DARK_H2)
                .color(PAL.dark_text)
                .wrap()
                .a11y_hidden(),
        )
        .child(
            row()
                .key("use-classic")
                .min_h(space::PRIMARY_MIN)
                .px(space::L)
                .center()
                .radius(radius::BUTTON)
                .fill(PAL.dark_button)
                .cursor(Cursor::Pointer)
                .focusable(true)
                .role("button")
                .label("Use the classic table")
                .on_press(|s: &mut AppState| s.use_classic_table())
                .child(
                    text("Use the classic table")
                        .font(ty::LOADING_BUTTON)
                        .color(PAL.dark_button_text)
                        .a11y_hidden(),
                ),
        )
}

/// The strong green button (06 §4 submit, §14.1 reconnect).
pub fn green_button(
    key: &str,
    label: &str,
    style: rune::tok::TextStyle,
    on_press: impl Fn(&mut AppState, &mut Tx) + 'static,
) -> El {
    row()
        .key(key.to_owned())
        .min_h(space::PRIMARY)
        .px(space::L)
        .center()
        .radius(radius::BUTTON)
        .fill(PAL.green_button)
        .hover(PAL.green_button_hover)
        .press_scale(0.98)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("button")
        .label(label.to_owned())
        .on_press_tx(on_press)
        .child(text(label.to_owned()).font(style).color(Color::WHITE))
}

/// An underlined text link.
pub fn text_link(
    key: &str,
    label: &str,
    style: rune::tok::TextStyle,
    color: Color,
    on_press: impl Fn(&mut AppState, &mut Tx) + 'static,
) -> El {
    // A tall text box keeps its glyphs at the top, so the tap target is a
    // row that centres the underlined line vertically.
    row()
        .key(key.to_owned())
        .min_h(space::TAP)
        .center_y()
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("link")
        .label(label.to_owned())
        .on_press_tx(on_press)
        .child(
            text(label.to_owned())
                .font(style)
                .color(color)
                .spans(vec![Span::new(0, label.len()).underline(color)])
                .a11y_hidden(),
        )
}

/// The line under a failed load or action, in the notice colours (06 §1.2 `.err`).
pub fn notice(key: &str, line: &str) -> El {
    row()
        .key(key.to_owned())
        .px(space::M)
        .py(space::SM)
        .radius(radius::BUTTON)
        .fill(PAL.notice_bg)
        .border(1.0, PAL.notice_border)
        .role("alert")
        .child(text(line.to_owned()).font(ty::NOTICE).color(PAL.ink).wrap())
}

/// How a screen enters and leaves when the route changes (09 §4.12). app.rs `view` keys each
/// screen's sheet by its route and applies what this returns; `dir` says whether the change
/// went forward (a link, a button) or back (Back, Escape), and `reduced` is effective reduce
/// motion.
///
/// The new screen is one opaque sheet, its background and nav included. It wipes in over the
/// old one behind a soft edge that sweeps across the window: from the right going forward,
/// from the left going back. Nothing slides, so the window edge never shows, and nothing
/// cross-fades, so no two pages or navs print over each other. The old screen stays under the
/// sheet and dissolves on the slow `ROUTE_OUT` into its own background, which the window keeps
/// until the change is done (app.rs `view`): a theme change is a clean edge, never a grey wash.
/// Reduced: a plain fade, no wipe.
pub fn route_transition(dir: RouteDir, reduced: bool) -> Option<Transition> {
    if reduced {
        return Some(Transition::opacity());
    }
    let from = match dir {
        RouteDir::Forward => Edge::Trailing,
        RouteDir::Back => Edge::Leading,
    };
    Some(
        Transition::asymmetric(Transition::reveal(from), Transition::opacity())
            .springs(SpringPair::new(motion::ROUTE_IN, motion::ROUTE_OUT)),
    )
}

/// How the old screen leaves on a route change (app.rs `view` sets it as the root's
/// `child_exit`). It dissolves under the incoming sheet on `ROUTE_OUT`; the table goes on the
/// quick `ROUTE_OUT_TABLE`, because its raised layers (the match-end panel and its confetti,
/// a banner, the settings rail) paint above every sheet. Reduced: a plain fade.
pub fn route_exit(left_table: bool, reduced: bool) -> Transition {
    if reduced {
        return Transition::opacity();
    }
    let out = if left_table {
        motion::ROUTE_OUT_TABLE
    } else {
        motion::ROUTE_OUT
    };
    Transition::opacity().springs(SpringPair::new(motion::ROUTE_IN, out))
}
