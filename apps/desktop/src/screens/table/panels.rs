//! The table's side panels (06 §5.9, §5.10, §11): the header's `How to play` and `Settings ⌄`
//! buttons, the settings rail, the rules dialog and the hand review.
//!
//! Every value they show comes from the store or the view; the rail's Wait-for-me box sends
//! `reviewTrick({action:'set'})` through the store, and the hand review reads the store's
//! post-hand archive contract only.

use std::sync::LazyLock;

use euchre_engine::cards::{color_mate, suit_of};
use euchre_engine::{card_name, card_name_lower, suit_name};
use euchre_protocol::store::{MSG_OPENING_HAND, TableStore};
use euchre_protocol::{Card, GamePhase, GameStatus, HandResult, PublicGameView, Suit};
use rune::{
    Color, Cursor, El, LineCap, ScrollSpec, Shape, Span, Spring, SpringPair, TextEdit, Transition,
    canvas, col, el, row, text, text_input,
};

use super::copy;
use super::hint;
use super::stage::face;
use crate::app::{AppState, Route, RulesMsg, TableScreen};
use crate::theme::{self, PAL, color, motion, radius, space, ty};

/// Header button label: the rules.
pub const HOW_TO_PLAY: &str = "How to play";
/// Header button label: the settings rail.
pub const SETTINGS_BUTTON: &str = "Settings ⌄";
/// Rail eyebrow.
pub const RAIL_EYEBROW: &str = "Your table";
/// Rail heading.
pub const RAIL_H2: &str = "Table settings";
/// Rail close button's accessible name.
pub const CLOSE_SETTINGS: &str = "Close settings";
/// The sound row.
pub const TABLE_SOUNDS: &str = "Table sounds";
/// The review checkbox.
pub const WAIT_FOR_ME: &str = "Wait for me after each trick";
/// The review checkbox's small line.
pub const WAIT_FOR_ME_SMALL: &str = "Review the cards, then press Continue.";
/// The hand review launcher.
pub const REVIEW_HANDS: &str = "↶ Review hands";
/// The hints toggle.
pub const HINTS: &str = "Hints";
/// The players section heading.
pub const AROUND_THE_TABLE: &str = "Around the table";
/// The personalities link.
pub const CHOOSE_PERSONALITIES: &str = "Choose player personalities →";
/// Under the personalities link.
pub const NEXT_GAME_NOTE: &str = "Changes apply to your next game.";
/// Rail link.
pub const START_NEW_GAME: &str = "Start a new game";
/// Rail link.
pub const BACK_TO_HOME: &str = "Back to home";
/// Rail link.
pub const SAVED_GAMES: &str = "Your saved games";
/// The chat disclosure.
pub const TABLE_TALK: &str = "Table talk";
/// The chat disclosure's empty state.
pub const TALK_EMPTY: &str = "The conversation starts with the first call.";
/// Rules close button's accessible name.
pub const CLOSE_RULES: &str = "Close rules";
/// Hand review heading.
pub const REVIEW_H2: &str = "Review a hand";
/// Hand review close button's accessible name.
pub const CLOSE_REVIEW: &str = "Close trick review";
/// No tricks yet.
pub const REVIEW_NO_TRICKS: &str =
    "No completed tricks in this hand yet. Choose an earlier hand above to review it.";
/// Buried cards heading.
pub const BURIED: &str = "Buried cards";
/// Unplayed cards heading.
pub const STAYED: &str = "Cards that stayed in hand";
/// No archive yet.
pub const BURIED_LATER: &str = "The buried cards are revealed only after this hand is finished.";

/// The rules sections (06 §5.10), title and body; the second body is built per view.
const RULES: [(&str, &str); 8] = [
    (
        "The deck",
        "24 cards: nine, ten, jack, queen, king and ace, in all four suits. Nothing lower than a nine is used.",
    ),
    (
        "Trump and the bowers",
        "The jack of the trump suit — the right bower — is the single best card in the game. The jack of the OTHER suit that's the same color becomes trump too, and outranks every other trump except the right bower — that's the left bower. ",
    ),
    (
        "Following suit",
        "If you can play a card of the suit that was led, you must — including the left bower, which counts as trump, not its printed suit, once trump is set. If you're void in the led suit, play anything.",
    ),
    (
        "Bidding, round one",
        "One card is turned face up. Starting left of the dealer, each player may \"order it up\" — making its suit trump for the hand — or pass. If the dealer's side orders it up, the dealer picks the card into hand and discards one card face down.",
    ),
    (
        "Bidding, round two",
        "If everyone passes, the up-card turns face down and can't be named. Starting left of the dealer again, each player may name any of the other three suits as trump, or pass. If it reaches the dealer with all three others having passed, the dealer must name a suit if stick the dealer is enabled; otherwise, four passes mean a fresh deal.",
    ),
    (
        "Going alone",
        "Whoever calls trump may go alone: their partner sits out entirely and they play all five tricks solo against both opponents. Win all 5 alone and it's worth 4 points instead of 2.",
    ),
    (
        "Scoring",
        "3 or 4 tricks: 1 point. All 5 (a march): 2 points. Alone and all 5: 4 points. The calling team wins fewer than 3 tricks — euchred — and the other team scores 2 instead. First to 10 wins the game.",
    ),
    (
        "What you'll hear at the table",
        "\"Order it up.\" / \"I assist.\" (dealer's partner) / \"I take it.\" or \"Turn it down.\" (dealer) / \"Next.\" or \"crossing the creek\" (round two) / \"You're stuck\" (dealer forced to call) / \"March!\" / \"Euchre!\"",
    ),
];

/// The panel colours (06 §5.9, §5.10, §11).
struct PanelColors {
    rail: Color,
    rail_border: Color,
    text: Color,
    eyebrow: Color,
    rule: Color,
    accent: Color,
    sub: Color,
    avatar_text: Color,
    help_bg: Color,
    help_border: Color,
    settings_border: Color,
    launcher_border: Color,
    context: Color,
}

static PANEL: LazyLock<PanelColors> = LazyLock::new(|| PanelColors {
    rail: color("#fffdf6"),
    rail_border: color("#d4dccb"),
    text: color("#233e32"),
    eyebrow: color("#69765f"),
    rule: color("#dce1d4"),
    accent: color("#285641"),
    sub: color("#58695b"),
    avatar_text: color("#fff5d9"),
    help_bg: color("#fffdf5"),
    help_border: color("#e8c27a8c"),
    settings_border: color("#cad3c1"),
    launcher_border: color("#bbc9ae"),
    context: color("#ebe8d8"),
});

/// Panel type: 17px body, 13px eyebrow and small lines, 14px link rows.
mod pty {
    use rune::tok::TextStyle;

    use crate::theme::SERIF;

    pub const BODY: TextStyle = TextStyle::new(17.0, 400.0).lh(25.5);
    pub const BODY_BOLD: TextStyle = TextStyle::new(17.0, 700.0).lh(25.5);
    pub const SMALL: TextStyle = TextStyle::new(13.0, 400.0).lh(18.0);
    pub const EYEBROW: TextStyle = TextStyle::new(13.0, 400.0);
    pub const H2: TextStyle = TextStyle::new(28.0, 500.0).family(SERIF);
    pub const H3: TextStyle = TextStyle::new(20.0, 500.0).family(SERIF);
    pub const BUTTON: TextStyle = TextStyle::new(17.0, 700.0);
    pub const LAUNCHER: TextStyle = TextStyle::new(16.0, 600.0);
    pub const AVATAR: TextStyle = TextStyle::new(20.0, 500.0).family(SERIF);
    pub const CLOSE: TextStyle = TextStyle::new(26.0, 400.0);
}

/// The dialog scrim's colour, alpha and blur (09 §3.8, §4.12).
const SCRIM: &str = "#0b1d16";
const SCRIM_ALPHA: f32 = 0.45;
const SCRIM_BLUR: f32 = 10.0;
/// The match-end scrim's darkening: lighter than a dialog's, since the panel over it holds the
/// next step and the table stays readable around it.
const PLAIN_SCRIM_ALPHA: f32 = 0.25;
/// A dialog sheet's corner radius (09 §5.6).
const SHEET_RADIUS: f32 = 18.0;
/// The rail's width (06 §5.9).
pub const RAIL_W: f32 = 340.0;
/// At this window width or wider the rail sits inline; narrower, it overlays.
pub const RAIL_INLINE_MIN_W: f32 = 1200.0;
/// The header buttons' height, normal and compact.
const TOOL_H: (f32, f32) = (52.0, 44.0);
/// The header buttons' type size in a wrapped header.
const NARROW_TOOL_TYPE: f32 = 15.0;
/// The rules sheet's width cap (34rem).
const RULES_MAX_W: f32 = 544.0;
/// The rules sheet's height cap (44rem).
const RULES_MAX_H: f32 = 704.0;
/// The hand review sheet's width cap.
const REVIEW_MAX_W: f32 = 720.0;
/// Chat lines the Table talk disclosure keeps.
const TALK_LINES: usize = 6;
/// A reviewed card's width.
const REVIEW_CARD_W: f32 = 64.0;

/// The header's two buttons: `How to play` and `Settings ⌄`. `narrow` (a wrapped header, 700
/// points wide or less): 44-point buttons with 15-point type, one at each end of a full-width row.
pub fn header_tools(settings_open: bool, compact: bool, narrow: bool) -> El {
    let h = if compact || narrow {
        TOOL_H.1
    } else {
        TOOL_H.0
    };
    let (pad, mut help_font, mut settings_font) = (space::M, pty::BUTTON, ty::FELT_BUTTON);
    let pad = if narrow {
        help_font.size = NARROW_TOOL_TYPE;
        settings_font.size = NARROW_TOOL_TYPE;
        12.0
    } else {
        pad
    };
    let help = row()
        .key("table-rules-open")
        .min_w(if narrow { 0.0 } else { 120.0 })
        .min_h(h)
        .px(pad)
        .center()
        .radius(radius::BUTTON)
        .fill(PANEL.help_bg)
        .border(1.0, PANEL.help_border)
        .hover(PAL.panel_hover)
        .press_scale(0.98)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("button")
        .label(HOW_TO_PLAY)
        .on_press(|s: &mut AppState| s.set_rules_open(true))
        .child(text(HOW_TO_PLAY).font(help_font).color(PANEL.text));
    let settings = row()
        .key("table-settings-open")
        .min_h(h)
        .px(pad)
        .center()
        .radius(radius::BUTTON)
        .fill(if settings_open {
            PAL.panel_hover
        } else {
            PANEL.rail
        })
        .border(1.0, PANEL.settings_border)
        .hover(PAL.panel_hover)
        .press_scale(0.98)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("button")
        .label("Settings")
        .expanded(settings_open)
        .on_press(|s: &mut AppState| s.toggle_settings())
        .child(text(SETTINGS_BUTTON).font(settings_font).color(PANEL.text));
    let tools = row().key("table-tools").gap(space::S).center_y();
    if narrow {
        tools
            .w_full()
            .child(help)
            .child(rune::spacer())
            .child(settings)
    } else {
        tools.child(help).child(settings)
    }
}

/// The sound toggle's 20 pt speaker (06 §15.1): sound waves when on, an X when off.
fn speaker(on: bool, colour: Color) -> El {
    const ICON: f32 = 20.0;
    const LINE: f32 = 1.8;
    // Drawn on a 24 x 24 grid that stretches to the icon.
    let body = vec![
        [2.0, 9.0],
        [6.0, 9.0],
        [11.0, 5.0],
        [11.0, 19.0],
        [6.0, 15.0],
        [2.0, 15.0],
    ];
    let mut shape = Shape::new().view_box(24.0, 24.0).fill(body, colour);
    if on {
        for r in [5.0_f32, 9.5] {
            let arc: Vec<[f32; 2]> = (0..=10)
                .map(|i| {
                    let a = (-45.0 + 9.0 * i as f32).to_radians();
                    [12.0 + r * a.cos(), 12.0 + r * a.sin()]
                })
                .collect();
            shape = shape.stroke(arc, LINE, colour);
        }
    } else {
        shape = shape
            .line([16.0, 9.0], [22.0, 15.0], LINE, LineCap::Round, colour)
            .line([22.0, 9.0], [16.0, 15.0], LINE, LineCap::Round, colour);
    }
    canvas(shape).size(ICON, ICON).a11y_hidden()
}

fn close_button(key: &str, label: &'static str, on: fn(&mut AppState)) -> El {
    row()
        .key(key.to_owned())
        .size(space::TAP, space::TAP)
        .center()
        .radius(radius::BUTTON)
        .hover(PAL.panel_hover)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("button")
        .label(label)
        .on_press(on)
        .child(text("×").font(pty::CLOSE).color(PANEL.text).a11y_hidden())
}

fn check_box(on: bool, enabled: bool) -> El {
    let mut b = col()
        .size(22.0, 22.0)
        .center()
        .radius(radius::CHECKBOX)
        .border(1.5, PANEL.accent)
        .fill(if on { PANEL.accent } else { PANEL.rail })
        .a11y_hidden();
    if on {
        b = b.child(text("✓").font(pty::SMALL).color(PANEL.rail));
    }
    if enabled { b } else { b.opacity(0.55) }
}

fn link_row(key: &str, label: &'static str, go: Route) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .min_h(space::TAP)
        .py(space::SM + 2.0)
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], PANEL.rule)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("link")
        .label(label)
        .on_press_tx(move |s: &mut AppState, tx| s.navigate(go.clone(), tx))
        .child(
            text(label)
                .font(pty::BODY)
                .color(PANEL.text)
                .spans(vec![Span::new(0, label.len()).underline(PANEL.text)])
                .a11y_hidden(),
        )
}

/// The settings rail (06 §5.9). `overlay` below [`RAIL_INLINE_MIN_W`]: a modal over the
/// table; otherwise an inline complementary panel beside it.
pub fn settings_rail(state: &AppState, table: &TableScreen, overlay: bool, reduced: bool) -> El {
    let view = table.store.display_view();
    let store = &table.store;
    let prefs = &state.prefs;
    let heading = row()
        .key("settings-heading")
        .w_full()
        .items_start()
        .child(
            col()
                .grow(1.0)
                .w(0.0)
                .gap(space::XS)
                .child(text(RAIL_EYEBROW).font(pty::EYEBROW).color(PANEL.eyebrow))
                .child(
                    text(RAIL_H2)
                        .key("settings-h2")
                        .font(pty::H2)
                        .color(PANEL.text)
                        .role("heading")
                        .level(2),
                ),
        )
        .child(close_button("settings-close", CLOSE_SETTINGS, |s| {
            s.toggle_settings()
        }));
    let sound_on = prefs.sound;
    let sound = row()
        .key("settings-sound")
        .w_full()
        .min_h(space::TAP)
        .center_y()
        .justify_between()
        .child(text(TABLE_SOUNDS).font(pty::BODY).color(PANEL.text))
        .child(
            row()
                .key("settings-sound-toggle")
                .min_w(space::TAP)
                .h(space::TAP)
                .px(space::SM)
                .center()
                .radius(radius::BUTTON)
                .border(1.0, PANEL.settings_border)
                .hover(PAL.panel_hover)
                .cursor(Cursor::Pointer)
                .focusable(true)
                .role("button")
                .checked(sound_on)
                .label(if sound_on {
                    "Mute table sound"
                } else {
                    "Unmute table sound"
                })
                .description(if sound_on { "Sound on" } else { "Sound off" })
                .help(if sound_on { "Sound on" } else { "Sound off" })
                .on_press(|s: &mut AppState| s.toggle_sound())
                .child(speaker(sound_on, PANEL.text)),
        );
    let review_on = view.and_then(|v| v.review_tricks).unwrap_or(false);
    let review_enabled = view.is_some() && store.is_connected() && !store.review_busy();
    let mut wait = row()
        .key("settings-review")
        .w_full()
        .py(space::SM)
        .gap(space::SM)
        .items_start()
        .border_sides([1.0, 0.0, 1.0, 0.0], PANEL.rule)
        .role("checkbox")
        .label(WAIT_FOR_ME)
        .description(WAIT_FOR_ME_SMALL)
        .checked(review_on)
        .disabled(!review_enabled)
        .child(check_box(review_on, review_enabled).mt(2.0))
        .child(
            col()
                .grow(1.0)
                .w(0.0)
                .gap(2.0)
                .a11y_hidden()
                .child(text(WAIT_FOR_ME).font(pty::BODY).color(PANEL.text).wrap())
                .child(
                    text(WAIT_FOR_ME_SMALL)
                        .font(pty::SMALL)
                        .color(PANEL.sub)
                        .wrap(),
                ),
        );
    if review_enabled {
        wait = wait
            .cursor(Cursor::Pointer)
            .focusable(true)
            .on_press(move |s: &mut AppState| s.set_review_tricks(!review_on));
    }
    let launcher = row()
        .key("settings-review-hands")
        .min_h(space::TAP)
        .px(space::M)
        .center()
        .radius(radius::BUTTON)
        .fill(PANEL.rail)
        .border(1.0, PANEL.launcher_border)
        .hover(PAL.panel_hover)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("button")
        .label(REVIEW_HANDS)
        .on_press(|s: &mut AppState| s.open_hand_review())
        .child(text(REVIEW_HANDS).font(pty::LAUNCHER).color(PANEL.text));
    let hints_on = prefs.hints;
    let hints = row()
        .key("settings-hints")
        .w_full()
        .min_h(space::TAP)
        .gap(space::SM)
        .center_y()
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("checkbox")
        .label(HINTS)
        .checked(hints_on)
        .on_press(|s: &mut AppState| s.update_prefs(|p| p.hints = !p.hints))
        .child(check_box(hints_on, true))
        .child(text(HINTS).font(pty::BODY).color(PANEL.text).a11y_hidden());
    let mut body = col()
        .key("settings-body")
        .w_full()
        .gap(space::M)
        .child(heading)
        .child(sound)
        .child(wait)
        .child(row().child(launcher))
        .child(hints);
    // The coach's suggestion box sits under the Hints toggle (06 §13).
    if let Some(coach) = view
        .and_then(|v| hint::for_view(v, prefs))
        .and_then(|h| hint::panel(&h))
    {
        body = body.child(coach);
    }
    if let Some(v) = view
        && let Some(players) = v.players.as_ref().filter(|p| !p.is_empty())
    {
        let mut section = col()
            .key("settings-players")
            .w_full()
            .gap(space::SM)
            .role("group")
            .label("Players at this table")
            .child(
                text(AROUND_THE_TABLE)
                    .font(pty::H3)
                    .color(PANEL.text)
                    .role("heading")
                    .level(3),
            );
        for p in players {
            let initial = p.name.chars().next().map(String::from).unwrap_or_default();
            section = section.child(
                row()
                    .key(format!("settings-player-{}", p.seat.index()))
                    .w_full()
                    .gap(space::SM)
                    .items_start()
                    .child(
                        col()
                            .size(36.0, 36.0)
                            .center()
                            .radius(radius::PILL)
                            .fill(PANEL.accent)
                            .a11y_hidden()
                            .child(text(initial).font(pty::AVATAR).color(PANEL.avatar_text)),
                    )
                    .child(
                        col()
                            .grow(1.0)
                            .w(0.0)
                            .gap(2.0)
                            .child(text(p.name.clone()).font(pty::BODY_BOLD).color(PANEL.text))
                            .child(
                                text(format!("{} · {}", copy::seat_name(v, p.seat), p.style))
                                    .font(pty::SMALL)
                                    .color(PANEL.sub),
                            )
                            .child(
                                text(p.blurb.clone())
                                    .font(pty::SMALL)
                                    .color(PANEL.text)
                                    .wrap(),
                            ),
                    ),
            );
        }
        section = section
            .child(
                row()
                    .key("settings-personalities")
                    .min_h(space::TAP)
                    .center_y()
                    .cursor(Cursor::Pointer)
                    .focusable(true)
                    .role("link")
                    .label(CHOOSE_PERSONALITIES)
                    .on_press_tx(|s: &mut AppState, tx| s.navigate(Route::Settings, tx))
                    .child(
                        text(CHOOSE_PERSONALITIES)
                            .font(pty::BODY_BOLD)
                            .color(PANEL.accent)
                            .a11y_hidden(),
                    ),
            )
            .child(text(NEXT_GAME_NOTE).font(pty::SMALL).color(PANEL.sub));
        body = body.child(section);
    }
    body = body.child(
        col()
            .key("settings-links")
            .w_full()
            .child(link_row("settings-new", START_NEW_GAME, Route::NewGame))
            .child(link_row("settings-home", BACK_TO_HOME, Route::Home))
            .child(link_row("settings-games", SAVED_GAMES, Route::Games)),
    );
    body = body.child(table_talk(table));
    let window_w = rune::memo::layout_size("root").map_or(f32::INFINITY, |[w, _]| w);
    let mut rail = col()
        .key("table-settings")
        .w(if overlay {
            rail_width(window_w)
        } else {
            RAIL_W
        })
        .h_full()
        .fill(PANEL.rail)
        .border_sides([0.0, 0.0, 0.0, 1.0], PANEL.rail_border)
        .label(RAIL_H2)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(body.padding(if overlay { 20.0 } else { space::L }));
    rail = if overlay {
        theme::shadows(
            rail.role("dialog")
                .modal(true, |s: &mut AppState| s.toggle_settings())
                .focus_return("table-settings-open"),
            &[(-16.0, 0.0, 48.0, 0.0, "#10251d30")],
        )
    } else {
        rail.role("complementary")
    };
    // The rail slides in from the right edge (09 §4.12, S9 offset presence).
    rail.transition(if reduced {
        Transition::opacity().springs(SpringPair::new(motion::RAIL, motion::RAIL))
    } else {
        Transition::offset(RAIL_W, 0.0)
            .combined(Transition::opacity())
            .springs(SpringPair::new(motion::RAIL, motion::RAIL))
    })
}

/// The rail's width in a window `width` wide (09 §5.6): [`RAIL_W`], or the window less 32
/// points when narrower, so the overlay never runs off the window.
pub fn rail_width(width: f32) -> f32 {
    RAIL_W.min(width - 32.0).max(0.0)
}

fn table_talk(table: &TableScreen) -> El {
    let open = table.present.hooks.talk_open;
    let toggle = row()
        .key("settings-talk")
        .w_full()
        .min_h(space::TAP)
        .gap(space::S)
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], PANEL.rule)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("button")
        .label(TABLE_TALK)
        .expanded(open)
        .on_press(|s: &mut AppState| s.toggle_table_talk())
        .child(
            text(if open { "▾" } else { "▸" })
                .font(pty::BODY)
                .color(PANEL.text)
                .a11y_hidden(),
        )
        .child(
            text(TABLE_TALK)
                .font(pty::BODY_BOLD)
                .color(PANEL.text)
                .a11y_hidden(),
        );
    let mut section = col().key("settings-talk-section").w_full().child(toggle);
    if open {
        let lines = table.store.chat();
        let view = table.store.display_view();
        let mut list = col().key("settings-talk-lines").w_full().gap(space::XS);
        if lines.is_empty() {
            list = list.child(text(TALK_EMPTY).font(pty::SMALL).color(PANEL.sub).wrap());
        }
        for line in lines.iter().skip(lines.len().saturating_sub(TALK_LINES)) {
            let speaker =
                view.map_or_else(|| "Table".to_owned(), |v| copy::player_name(v, line.seat));
            let all = format!("{speaker} {}", line.text);
            list = list.child(
                text(all)
                    .key(format!("talk-{}", line.msg_id))
                    .font(pty::BODY)
                    .color(PANEL.text)
                    .spans(vec![Span::new(0, speaker.len()).weight(700.0)])
                    .wrap(),
            );
        }
        section = section.child(list);
    }
    section
}

/// A scrim behind a dialog (09 §4.12): the table blurs and darkens behind it, fading in on
/// `SCRIM_IN` and out on `SCRIM_OUT`; its opacity fades the blur with it. Reduce motion keeps the
/// darkening and drops the blur.
pub fn scrim(key: &str, reduced: bool) -> El {
    let e = el()
        .key(key.to_owned())
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(color(SCRIM).alpha(SCRIM_ALPHA))
        .a11y_hidden()
        .transition(
            Transition::opacity().springs(SpringPair::new(motion::SCRIM_IN, motion::SCRIM_OUT)),
        );
    if reduced {
        e
    } else {
        e.backdrop_blur(SCRIM_BLUR)
    }
}

/// A light scrim without the blur (the match-end panel, 09 §4.10): it quiets the table at
/// [`PLAIN_SCRIM_ALPHA`] so the panel reads, fades in on `SCRIM_IN` and out on `SCRIM_OUT`,
/// and takes no press, so the header and Play again under it still work.
pub fn plain_scrim(key: &str) -> El {
    el().key(key.to_owned())
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(color(SCRIM).alpha(PLAIN_SCRIM_ALPHA))
        .a11y_hidden()
        .transition(
            Transition::opacity().springs(SpringPair::new(motion::SCRIM_IN, motion::SCRIM_OUT)),
        )
}

/// The settings overlay's scrim.
pub fn settings_scrim(reduced: bool) -> El {
    scrim("settings-scrim", reduced)
}

/// How a dialog panel arrives and leaves (09 §4.12, S10): it rises from 0.9 and 30 points low
/// on `PANEL_IN`, and fades a little smaller on `PANEL_OUT`. Reduce motion fades only.
pub fn panel_transition(reduced: bool) -> Transition {
    if reduced {
        return Transition::opacity().springs(SpringPair::new(motion::SCRIM_IN, motion::SCRIM_OUT));
    }
    Transition::asymmetric(
        Transition::scale(0.9)
            .combined(Transition::offset(0.0, 30.0))
            .combined(Transition::opacity()),
        Transition::scale(0.96).combined(Transition::opacity()),
    )
    .springs(SpringPair::new(motion::PANEL_IN, motion::PANEL_OUT))
}

/// A dialog sheet (09 §5.6): cream, radius 18, a soft white edge and the E5 shadow; `max` is
/// its width and height cap.
fn sheet(key: &str, max: [f32; 2], title: &'static str, reduced: bool) -> El {
    let sheet = col()
        .key(key.to_owned())
        .w_full()
        .max_w(max[0])
        .max_h(max[1])
        .radius(SHEET_RADIUS)
        .fill(PAL.cream)
        .border(1.0, Color::WHITE.alpha(0.6))
        .role("dialog")
        .label(title)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .transition(panel_transition(reduced));
    motion::e5(sheet)
}

fn context_card(view: &PublicGameView) -> Option<(&'static str, String)> {
    if view.status != GameStatus::Active {
        return None;
    }
    let up_suit = view.up_card.map(suit_of);
    let suit_or = |s: Option<Suit>| s.map_or("that suit", suit_name);
    Some(match view.phase {
        GamePhase::Cutting => (
            "Right now: cutting for the deal",
            "The player to the dealer’s right may cut (“bump”) or say “run ’em” to leave the deck as shuffled."
                .to_owned(),
        ),
        GamePhase::BidRound1 => (
            "Right now: the up-card is showing",
            format!(
                "{} \"order it up\" — {} become trump, the dealer picks the card into hand and buries one of their own — or pass. Ordering it up alone leaves your partner out of the hand for a bigger score if you win it all.",
                if view.turn_seat == Some(view.you) {
                    "You may"
                } else {
                    "Each player in turn may"
                },
                suit_or(up_suit)
            ),
        ),
        GamePhase::DealerDiscard => (
            "Right now: the dealer discards",
            "The dealer has 6 cards after picking up the up-card. One goes face down, unseen, for the rest of the hand — back down to 5."
                .to_owned(),
        ),
        GamePhase::BidRound2 => (
            "Right now: round two",
            format!(
                "The turned-down suit ({}) may not be named again. Each player may name any of the other three suits as trump, or pass. If it comes all the way back around to the dealer with everyone else passed, {}",
                suit_or(up_suit),
                if view.rules.is_none_or(|r| r.stick_the_dealer) {
                    "the dealer must name a suit — no more passing."
                } else {
                    "the dealer may pass too; four passes mean a fresh deal."
                }
            ),
        ),
        GamePhase::TrickPlay => {
            let mut body = match view.trick.led_suit {
                None => "You're on lead — play anything. Whatever suit you lead, everyone else must follow if they can.".to_owned(),
                Some(led) => format!("{} was led. Follow it if you're holding one.", suit_name(led)),
            };
            if let Some(trump) = view.trump {
                body.push_str(&format!(
                    " {} are trump — the {} counts as {} right now, not its printed suit.",
                    suit_name(trump),
                    left_bower_lower(trump),
                    suit_name(trump).to_lowercase()
                ));
            }
            ("Right now: playing a trick", body)
        }
        GamePhase::TrickResolve => (
            "Right now: the trick just finished",
            "Watch the felt — the winner leads next.".to_owned(),
        ),
        GamePhase::HandScore => (
            "Right now: scoring the hand",
            "The next hand deals in a moment.".to_owned(),
        ),
        GamePhase::GameOver => ("Game over", "First to 10 points takes the match.".to_owned()),
        GamePhase::Lobby | GamePhase::Deal => return None,
    })
}

fn left_bower_lower(trump: Suit) -> String {
    card_name_lower(Card::new(euchre_protocol::Rank::Jack, color_mate(trump)))
}

fn bowers_body(view: &PublicGameView) -> String {
    let base = RULES[1].1;
    match view.trump {
        Some(trump) => format!(
            "{base}Right now {} are trump, so the {} counts as {}, not its printed suit, for the rest of this hand.",
            suit_name(trump),
            left_bower_lower(trump),
            suit_name(trump).to_lowercase()
        ),
        None => format!(
            "{base}For example: if Hearts were trump, the jack of diamonds would count as hearts, not its printed suit, for the rest of that hand."
        ),
    }
}

/// The search box's placeholder (06 §5.10).
pub const SEARCH_PLACEHOLDER: &str = "Search the rules…";
/// The search box's screen-reader label (06 §5.10).
pub const SEARCH_LABEL: &str = "Search the rules";
/// The rules dialog's last button (06 §5.10, §14.3).
pub const TOUR_AGAIN: &str = "New to euchre? Take the quick tour again";

/// "No section matches "{query}"." (06 §5.10).
pub fn no_match(query: &str) -> String {
    format!("No section matches \"{query}\".")
}

/// How the rules search box reaches its editor in app state (for `El::on_edit`).
pub type SearchAccess = fn(&mut AppState) -> &mut TextEdit;

/// The rules dialog's search and accordion (06 §5.10), read from `Presentation.hooks`.
#[derive(Clone, Copy)]
pub struct RulesState<'a> {
    /// The search text (`hooks.rules_query`).
    pub query: &'a str,
    /// Sections the player toggled away from the default, one bit each (`hooks.rules_flipped`).
    pub flipped: u8,
    /// The search box's editor and its access; the box is drawn only with one.
    pub search: Option<(&'a TextEdit, SearchAccess)>,
}

/// Whether section `i` is open: closed by default, every one open while there is a query (06
/// §5.10, `<details open={query.trim().length > 0}>`), and a toggle flips that default.
pub fn section_open(query: &str, flipped: u8, i: usize) -> bool {
    let default = !query.trim().is_empty();
    default ^ (i < 8 && flipped >> i & 1 == 1)
}

/// Whether a section's title or body holds `query`, ignoring case and the query's outer spaces.
pub fn section_matches(title: &str, body: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || title.to_lowercase().contains(&q) || body.to_lowercase().contains(&q)
}

/// The rules sections for `view` in order, with the bowers body built for its trump.
fn sections(view: &PublicGameView) -> impl Iterator<Item = (usize, &'static str, String)> + '_ {
    RULES.into_iter().enumerate().map(|(i, (title, plain))| {
        let body = if i == 1 {
            bowers_body(view)
        } else {
            plain.to_owned()
        };
        (i, title, body)
    })
}

/// How a section's body appears and goes as it opens and closes.
const SECTION_BODY: SpringPair = SpringPair::new(Spring::new(0.28, 0.0), Spring::new(0.16, 0.0));
/// How the sections below an opening one move down.
const SECTION_MOVE: Spring = Spring::new(0.30, 0.0);

/// One accordion section: a title button with a chevron, and the body while open. Without
/// `toggles` it is a plain open section (no state to keep it in).
fn rule_section(
    i: usize,
    title: &'static str,
    body: String,
    open: bool,
    toggles: bool,
    reduced: bool,
) -> El {
    let mut head = row()
        .key(format!("rules-{i}-head"))
        .w_full()
        .min_h(space::TAP)
        .gap(space::S)
        .center_y()
        .child(
            text(title)
                .font(pty::H3)
                .color(PANEL.text)
                .role("heading")
                .level(3)
                .grow(1.0)
                .w(0.0),
        );
    if toggles {
        head = head
            .radius(radius::BUTTON)
            .cursor(Cursor::Pointer)
            .focusable(true)
            .role("button")
            .label(title)
            .expanded(open)
            .on_press(move |s: &mut AppState| s.rules(RulesMsg::Toggle(i)))
            .child(
                text("›")
                    .key(format!("rules-{i}-chevron"))
                    .font(pty::H3)
                    .color(PANEL.sub)
                    .rotate(if open {
                        std::f32::consts::FRAC_PI_2
                    } else {
                        0.0
                    })
                    .motion(motion::HOVER)
                    .a11y_hidden(),
            );
    }
    let mut section = col()
        .key(format!("rules-{i}"))
        .w_full()
        .gap(space::XS)
        .pt(space::S)
        .border_sides([1.0, 0.0, 0.0, 0.0], PANEL.rule)
        .motion(SECTION_MOVE)
        .child(head);
    if open {
        section = section.child(
            text(body)
                .key(format!("rules-{i}-body"))
                .font(pty::BODY)
                .color(PANEL.text)
                .wrap()
                .transition(if reduced {
                    Transition::opacity().springs(SECTION_BODY)
                } else {
                    Transition::offset(0.0, -6.0)
                        .combined(Transition::opacity())
                        .springs(SECTION_BODY)
                }),
        );
    }
    section
}

/// The rules search box (06 §5.10): typing filters the sections. When the text goes from empty
/// to not (or back) the default open state flips, so the player's toggles reset, as the web
/// `<details open>` binding does.
fn search_box(edit: &TextEdit, access: SearchAccess) -> El {
    let field = text_input(edit, pty::BODY)
        .key("rules-search")
        .single_line()
        .w_full()
        .color(PANEL.text)
        .placeholder(SEARCH_PLACEHOLDER, PANEL.sub)
        .caret(PANEL.text, PAL.panel_hover)
        .label(SEARCH_LABEL)
        .on_edit(access)
        .on_input_change(move |s: &mut AppState, _| {
            let query = access(s).text.clone();
            let (was, flipped) = s.table.as_ref().map_or((String::new(), 0), |t| {
                let hooks = &t.present.hooks;
                (hooks.rules_query.clone(), hooks.rules_flipped)
            });
            if was.trim().is_empty() != query.trim().is_empty() {
                for n in (0..8).filter(|n| flipped >> n & 1 == 1) {
                    s.rules(RulesMsg::Toggle(n));
                }
            }
            s.rules(RulesMsg::Query(query));
        });
    row()
        .key("rules-search-box")
        .w_full()
        .min_h(space::TAP)
        .px(space::M)
        .center_y()
        .radius(radius::BUTTON)
        .fill(PANEL.rail)
        .border(1.0, PANEL.settings_border)
        .child(field)
}

/// The rules dialog without its search and accordion state: every section shows, open.
pub fn rules_dialog(view: &PublicGameView, height: f32, reduced: bool) -> El {
    rules_dialog_with(view, None, height, reduced)
}

/// The rules dialog (06 §5.10): the table's rules, what is happening now, the search box, the
/// sections (an accordion, with `rules`) and the tour button. `height` is the window's: the
/// sheet keeps to 85% of it and scrolls.
pub fn rules_dialog_with(
    view: &PublicGameView,
    rules: Option<RulesState<'_>>,
    height: f32,
    reduced: bool,
) -> El {
    let mut body = col()
        .key("rules-body")
        .w_full()
        .padding(space::L)
        .gap(space::M)
        .child(
            row()
                .w_full()
                .center_y()
                .child(
                    text(HOW_TO_PLAY)
                        .key("rules-title")
                        .font(pty::H2)
                        .color(PANEL.text)
                        .role("heading")
                        .level(2)
                        .grow(1.0)
                        .w(0.0),
                )
                .child(close_button("rules-close", CLOSE_RULES, |s| {
                    s.set_rules_open(false)
                })),
        );
    if let Some(rules) = view.rules {
        body = body.child(
            text(format!(
                "This table: stick the dealer {} · Natural trump to call {}.",
                if rules.stick_the_dealer { "on" } else { "off" },
                if rules.require_natural_trump {
                    "required (left bower alone does not qualify)"
                } else {
                    "not required"
                }
            ))
            .key("rules-table")
            .font(pty::SMALL)
            .color(PANEL.sub)
            .wrap(),
        );
    }
    if let Some((title, line)) = context_card(view) {
        body = body.child(
            col()
                .key("rules-now")
                .w_full()
                .gap(space::XS)
                .padding(space::M)
                .radius(radius::BUTTON)
                .fill(PANEL.context)
                .child(text(title).font(pty::BODY_BOLD).color(PANEL.text).wrap())
                .child(text(line).font(pty::BODY).color(PANEL.text).wrap()),
        );
    }
    let query = rules.map_or("", |r| r.query);
    if let Some((edit, access)) = rules.and_then(|r| r.search) {
        body = body.child(search_box(edit, access));
    }
    let mut shown = 0;
    for (i, title, line) in sections(view).filter(|(_, t, b)| section_matches(t, b, query)) {
        let (open, toggles) = match rules {
            Some(r) => (section_open(r.query, r.flipped, i), true),
            None => (true, false),
        };
        body = body.child(rule_section(i, title, line, open, toggles, reduced));
        shown += 1;
    }
    if shown == 0 {
        body = body.child(
            text(no_match(query))
                .key("rules-empty")
                .font(pty::BODY)
                .color(PANEL.sub)
                .wrap(),
        );
    }
    body = body.child(
        row()
            .key("rules-tour")
            .w_full()
            .min_h(space::TAP)
            .px(space::M)
            .center()
            .radius(radius::BUTTON)
            .border(1.0, PANEL.launcher_border)
            .border_dash(6.0, 4.0)
            .hover(PAL.panel_hover)
            .press_scale(0.98)
            .cursor(Cursor::Pointer)
            .focusable(true)
            .role("button")
            .label(TOUR_AGAIN)
            .on_press(|s: &mut AppState| {
                s.set_rules_open(false);
                s.tour_open();
            })
            .child(
                text(TOUR_AGAIN)
                    .font(pty::LAUNCHER)
                    .color(PANEL.text)
                    .a11y_hidden(),
            ),
    );
    let sheet = sheet(
        "rules-dialog",
        [RULES_MAX_W, (height * 0.85).min(RULES_MAX_H)],
        HOW_TO_PLAY,
        reduced,
    )
    .modal(true, |s: &mut AppState| s.set_rules_open(false))
    .focus_return("table-rules-open")
    .child(body);
    dialog_layer("rules-layer", scrim("rules-scrim", reduced), sheet)
}

fn dialog_layer(key: &str, scrim: El, sheet: El) -> El {
    el().key(key.to_owned())
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .raise()
        .child(scrim)
        .child(
            col()
                .key(format!("{key}-centre"))
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .padding(space::M)
                .center()
                .child(sheet),
        )
}

fn result_points(result: Option<HandResult>) -> &'static str {
    match result {
        Some(HandResult::Euchre) => " — euchre",
        Some(HandResult::March | HandResult::LoneMarch) => " — all five tricks",
        _ => "",
    }
}

fn summary(view: &PublicGameView, store: &TableStore) -> Vec<El> {
    let panel = store.hand_review();
    let archive = panel.archive.as_ref();
    let tricks = store.hand_review_tricks();
    let live = view.hand_no == panel.hand_no;
    let trump = archive.map_or(if live { view.trump } else { None }, |a| a.trump);
    let maker = archive.map_or(if live { view.maker_seat } else { None }, |a| a.maker_seat);
    let alone = archive.map_or(if live { view.alone_seat } else { None }, |a| a.alone_seat);
    let result = archive.and_then(|a| a.result);
    let mut out = Vec::new();
    let strong = |all: String, bold: std::ops::Range<usize>| {
        text(all)
            .font(pty::BODY)
            .color(PANEL.text)
            .spans(vec![Span::new(bold.start, bold.end).weight(700.0)])
            .wrap()
    };
    if result == Some(HandResult::ThrowIn) || (trump.is_none() && tricks.is_empty()) {
        let lead = "Everyone passed. No points were scored.";
        out.push(strong(
            format!("{lead} The deal moved to the next player."),
            0..lead.len(),
        ));
    } else {
        let lead = format!("{} were trump.", trump.map_or("Unknown suit", suit_name));
        let mut all = lead.clone();
        if let Some(m) = maker {
            all.push_str(&format!(" {} named trump", copy::seat_name(view, m)));
            if alone.is_some() {
                all.push_str(" and went alone");
            }
            all.push('.');
        }
        out.push(strong(all, 0..lead.len()));
        if !tricks.is_empty() {
            let [us, them] = {
                let t = store.hand_review_totals();
                let team = view.you.index() % 2;
                [t[team], t[1 - team]]
            };
            let a = "Tricks: your team ".len();
            let (us, them) = (us.to_string(), them.to_string());
            let b = a + us.len() + " · other team ".len();
            out.push(
                text(format!("Tricks: your team {us} · other team {them}."))
                    .font(pty::BODY)
                    .color(PANEL.text)
                    .spans(vec![
                        Span::new(a, a + us.len()).weight(700.0),
                        Span::new(b, b + them.len()).weight(700.0),
                    ]),
            );
        }
        if let Some(delta) = archive.and_then(|a| a.delta) {
            let team = view.you.index() % 2;
            let who = if delta[team] > 0 {
                "Your team"
            } else {
                "The other team"
            };
            let pts = delta[0].max(delta[1]);
            let line = format!(
                "{who} scored {pts} {}{}.",
                if pts == 1 { "point" } else { "points" },
                result_points(result)
            );
            let n = line.len();
            out.push(strong(line, 0..n));
        }
    }
    if let Some(dealer) = archive.and_then(|a| a.dealer_seat) {
        let mut line = format!("Dealer: {}.", copy::seat_name(view, dealer));
        if let Some(Some(up)) = archive.and_then(|a| a.up_card) {
            line.push_str(&format!(" Up card: {}.", card_name(up)));
        }
        out.push(text(line).font(pty::BODY).color(PANEL.text).wrap());
    }
    out
}

fn card_figure(state: &AppState, key: String, c: Card, caption: Option<String>) -> El {
    let mut fig = col()
        .key(key)
        .gap(space::XS)
        .items_center()
        .role("img")
        .label(match &caption {
            Some(cap) => format!("{cap}: {}", card_name(c)),
            None => card_name(c),
        })
        .child(face(state, c, REVIEW_CARD_W).a11y_hidden());
    if let Some(cap) = caption {
        fig = fig.child(
            text(cap)
                .font(pty::SMALL)
                .color(PANEL.sub)
                .max_lines(1)
                .a11y_hidden(),
        );
    }
    fig
}

/// The hand review (06 §11): pick a hand, read its summary and tricks, and, once it is
/// over, its buried cards. `height` is the window's.
pub fn hand_review_dialog(state: &AppState, table: &TableScreen, height: f32, reduced: bool) -> El {
    let store = &table.store;
    let panel = store.hand_review();
    let mut body = col()
        .key("review-body")
        .w_full()
        .padding(space::L)
        .gap(space::M)
        .child(
            row()
                .w_full()
                .center_y()
                .child(
                    text(REVIEW_H2)
                        .key("review-title")
                        .font(pty::H2)
                        .color(PANEL.text)
                        .role("heading")
                        .level(2)
                        .grow(1.0)
                        .w(0.0),
                )
                .child(close_button("review-close", CLOSE_REVIEW, |s| {
                    s.close_hand_review()
                })),
        );
    if let Some(view) = store.display_view() {
        let mut picker = row()
            .key("review-hands")
            .w_full()
            .gap(space::S)
            .center_y()
            .flex_wrap()
            .role("radiogroup")
            .label("Hand")
            .child(
                text("Hand")
                    .font(pty::BODY_BOLD)
                    .color(PANEL.text)
                    .a11y_hidden(),
            );
        for n in 0..=view.hand_no {
            let label = format!(
                "{} · {}",
                n + 1,
                if n == view.hand_no {
                    "Current"
                } else {
                    "Finished"
                }
            );
            let on = n == panel.hand_no;
            picker = picker.child(
                row()
                    .key(format!("review-hand-{n}"))
                    .min_h(space::TAP)
                    .px(space::SM)
                    .center()
                    .radius(radius::BUTTON)
                    .border(1.0, PANEL.launcher_border)
                    .fill(if on { PANEL.accent } else { PANEL.rail })
                    .hover(if on { PANEL.accent } else { PAL.panel_hover })
                    .cursor(Cursor::Pointer)
                    .focusable(true)
                    .role("radio")
                    .checked(on)
                    .label(label.clone())
                    .on_press(move |s: &mut AppState| s.choose_hand_review(n))
                    .child(
                        text(label)
                            .font(pty::LAUNCHER)
                            .color(if on { PANEL.avatar_text } else { PANEL.text })
                            .a11y_hidden(),
                    ),
            );
        }
        body = body.child(picker);
        if panel.busy {
            body = body.child(
                text(MSG_OPENING_HAND)
                    .key("review-loading")
                    .font(pty::BODY)
                    .color(PANEL.sub)
                    .role("status"),
            );
        } else if let Some(err) = panel.error {
            body = body.child(
                text(err)
                    .key("review-error")
                    .font(pty::BODY)
                    .color(PANEL.text)
                    .role("alert")
                    .wrap(),
            );
        }
        if !panel.busy {
            body = body.child(
                col()
                    .key("review-summary")
                    .w_full()
                    .gap(space::XS)
                    .children(summary(view, store)),
            );
            let tricks = store.hand_review_tricks();
            if tricks.is_empty() {
                body = body.child(
                    text(REVIEW_NO_TRICKS)
                        .key("review-empty")
                        .font(pty::BODY)
                        .color(PANEL.sub)
                        .wrap(),
                );
            }
            for trick in tricks {
                let winner = trick.winner_seat.map_or("", |w| copy::seat_name(view, w));
                let mut cards = row().w_full().gap(12.0).flex_wrap();
                for (i, play) in trick.plays.iter().enumerate() {
                    cards = cards.child(card_figure(
                        state,
                        format!("review-{}-{i}", trick.index),
                        play.card,
                        Some(format!("{}. {}", i + 1, copy::seat_name(view, play.seat))),
                    ));
                }
                body = body.child(
                    col()
                        .key(format!("review-trick-{}", trick.index))
                        .w_full()
                        .gap(space::S)
                        .child(
                            text(format!("Trick {} · {winner} won", trick.index + 1))
                                .font(pty::H3)
                                .color(PANEL.text)
                                .role("heading")
                                .level(3),
                        )
                        .child(cards),
                );
            }
            body = match &panel.archive {
                Some(archive) => {
                    let mut b = body.child(cards_section(
                        state,
                        "review-buried",
                        BURIED,
                        &archive.buried,
                    ));
                    if !archive.unplayed.is_empty() {
                        b = b.child(cards_section(
                            state,
                            "review-unplayed",
                            STAYED,
                            &archive.unplayed,
                        ));
                    }
                    b
                }
                None => body.child(
                    text(BURIED_LATER)
                        .key("review-buried-later")
                        .font(pty::SMALL)
                        .color(PANEL.sub)
                        .wrap(),
                ),
            };
        }
    }
    let sheet = sheet(
        "review-dialog",
        [REVIEW_MAX_W, height - 2.0 * space::M],
        REVIEW_H2,
        reduced,
    )
    .modal(true, |s: &mut AppState| s.close_hand_review())
    .focus_return("settings-review-hands")
    .child(body);
    dialog_layer("review-layer", scrim("review-scrim", reduced), sheet)
}

fn cards_section(state: &AppState, key: &str, title: &'static str, cards: &[Card]) -> El {
    let mut row_el = row().w_full().gap(12.0).flex_wrap();
    for (i, &c) in cards.iter().enumerate() {
        row_el = row_el.child(card_figure(state, format!("{key}-{i}"), c, None));
    }
    col()
        .key(key.to_owned())
        .w_full()
        .gap(space::S)
        .child(
            text(title)
                .font(pty::H3)
                .color(PANEL.text)
                .role("heading")
                .level(3),
        )
        .child(row_el)
}
