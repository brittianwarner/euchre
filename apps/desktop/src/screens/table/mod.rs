//! The table (06 §5, §6, §7, §9, §12, §14): the cream header, the felt with its four seats and
//! the trick, the hand area with its rack and controls, and the moments laid over them (the
//! reconnecting notice, outcome banners and confetti).
//!
//! It renders only from the [`TableStore`] and [`present::Presentation`]; every rule lives in
//! the store or the engine.

pub mod announcer;
pub mod choreo;
pub mod copy;
mod fx;
mod hand;
mod header;
pub mod hint;
pub mod panels;
pub mod present;
mod stage;
pub mod tour;

use euchre_protocol::store::{MSG_TABLE_OPEN_FAILED, MSG_YOUR_SAVED_GAMES};
use euchre_protocol::{GameStatus, TableStore};
use rune::{Cx, El, Key, KeyEvent, Transition, Tx, col, el, row, text};

pub use copy::{H_IDLE, H_OVER, H_REVIEW, H_SENDING, H_TURN, PLAY_AGAIN, turn_instruction};

use crate::app::{AppState, Route, TableScreen, TableStage};
use crate::net::NetError;
use crate::screens::{green_button, notice, server, text_link};
use crate::theme::felt::{shadow as fshadow, size, ty as fty};
use crate::theme::{self, FELT, PAL, motion, space, table as t, ty};

/// The Table screen at window width `width`.
pub fn view(state: &AppState, width: f32, cx: &mut Cx<AppState>) -> El {
    let height = cx
        .layout_size("root")
        .map_or(theme::window::SIZE.1, |[_, h]| h);
    let Some(table) = &state.table else {
        return loading(state, None);
    };
    let Some(view) = table.store.display_view() else {
        return loading(state, Some(table));
    };
    table.note_drawn_size([width, height]);
    let reduced = cx.is_reduced_motion();
    let compact = height < size::COMPACT_H;
    let hand_h = if compact {
        (height * size::HAND_COMPACT.0).clamp(size::HAND_COMPACT.1, size::HAND_COMPACT.2)
    } else {
        t::hand_h(height)
    };
    let ctx = stage::Ctx {
        state,
        table,
        view,
        reduced,
        compact,
        height,
        felt: cx.layout_size("felt").unwrap_or([1000.0, 400.0]),
        trick: cx.layout_size("trick").unwrap_or([560.0, 220.0]),
    };
    let rack_size = cx.layout_size("rack").unwrap_or([t::RACK_MAX_W, 200.0]);
    // An outcome banner lies over the hand area, clear of the seats, trump and score. The
    // match's last banner is the centred match-end panel instead (09 §4.10).
    let match_over = view.status != GameStatus::Active;
    let banner_layer = table
        .present
        .banner
        .as_ref()
        .filter(|_| !match_over)
        .map(|b| banner(b, reduced));
    let mut layout = col()
        .key("table-layout")
        .w_full()
        .grow(1.0)
        .h(0.0)
        .fill(PAL.table_bg)
        .px(if compact { space::SM } else { t::PAD[0] })
        .py(t::PAD[1])
        .gap(space::S)
        .child(stage::felt(&ctx))
        .child(hand::hand_area(&ctx, hand_h, rack_size, banner_layer));
    // A table that ended keeps its last view and says why, with its way out (contract
    // §N2.5); one that is only reconnecting says so. Both sit at the foot of the table, over
    // the controls they turn off, so the felt (seats, hand, tricks) stays in full view.
    if let TableStage::Failed(err) = &table.stage {
        let copy = stage_failure(state, table, err);
        layout = layout.child(
            server::ended_banner(&copy)
                .pin(Some(space::L), None, Some(space::L), Some(space::S))
                .raise(),
        );
    } else if let Some(line) = table.store.reconnect_notice() {
        layout = layout.child(
            notice("table-reconnecting", line)
                .role("status")
                .pin(Some(space::L), None, Some(space::L), Some(space::S))
                .raise(),
        );
    }
    let hooks = &table.present.hooks;
    let inline_rail = width >= panels::RAIL_INLINE_MIN_W;
    let body = if hooks.settings_open && inline_rail {
        row()
            .key("table-body")
            .w_full()
            .grow(1.0)
            .h(0.0)
            .child(layout.grow(1.0).w(0.0).h_full())
            .child(panels::settings_rail(state, table, false, reduced))
    } else {
        row()
            .key("table-body")
            .w_full()
            .grow(1.0)
            .h(0.0)
            .child(layout.grow(1.0).w(0.0).h_full())
    };
    let mut screen = col()
        .key("table")
        .w_full()
        .h_full()
        .child(header::header_with(
            view,
            width,
            compact,
            reduced,
            hooks.settings_open,
            Some((&table.present.score, state.clock)),
        ))
        .child(body);
    if match_over {
        let panel_at = cx.layout("match-end-panel");
        screen = screen.child(match_end(&ctx, [width, height], panel_at));
    }
    screen = screen.child(
        el().key("fx-layer")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .raise()
            .a11y_hidden()
            .child(fx::layer(&ctx, cx)),
    );
    if hooks.settings_open && !inline_rail {
        screen = screen.child(
            el().key("settings-layer")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .raise()
                .child(panels::settings_scrim(reduced))
                .child(
                    panels::settings_rail(state, table, true, reduced)
                        .w(panels::rail_width(width))
                        .pin(None, Some(0.0), Some(0.0), Some(0.0)),
                ),
        );
    }
    if hooks.rules_open {
        screen = screen.child(panels::rules_dialog_with(
            view,
            Some(panels::RulesState {
                query: &hooks.rules_query,
                flipped: hooks.rules_flipped,
                search: Some((&state.rules_search, AppState::rules_search)),
            }),
            height,
            reduced,
        ));
    }
    if hooks.hand_review_open {
        screen = screen.child(panels::hand_review_dialog(state, table, height, reduced));
    }
    // The confetti is the fx layer's (Lane B, 09 §4.9), posed from `present.confetti_t`.
    if let Some(step) = hooks.tour_step {
        screen = screen.child(tour::layer(step, [width, height], reduced));
    }
    screen.child(announcer::layer(table.present.announced.as_ref()))
}

/// An outcome banner (06 §9): a star and one line, centred over the rack.
fn banner(b: &present::Banner, reduced: bool) -> El {
    let card = banner_card(b).transition(if reduced {
        Transition::opacity()
    } else {
        Transition::offset(0.0, -12.0).combined(Transition::opacity())
    });
    row()
        .key("banner-layer")
        .pin(Some(0.0), Some(size::BANNER_TOP), Some(0.0), Some(0.0))
        .center()
        .raise()
        .child(theme::shadows(card, &fshadow::BANNER))
}

/// The match-end panel (09 §4.10): once the last trick has swept and the hand's score has
/// rolled (the `MatchEnd` shot), a cream panel centred at 38% of the window's height arrives on
/// `PANEL_IN` over a light scrim with no blur. It holds the match's line, both final scores
/// (rolling to the last hand's points, then the winners' number pops) and Play again, so
/// the next step sits in the panel, above the scrim. On a win the confetti rises from behind
/// its top edge once it has landed. It stays while the match is over. `panel_at` is the
/// panel's last laid-out rect.
fn match_end(ctx: &stage::Ctx<'_>, [width, height]: [f32; 2], panel_at: Option<[f32; 4]>) -> El {
    let present = ctx.present();
    let layer = el()
        .key("match-end-layer")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .raise();
    if !present.match_end_shown(ctx.view) {
        return layer;
    }
    let w = size::MATCH_END_W.min(width - 2.0 * space::L);
    let t = present.match_end_t(ctx.state.clock);
    let mut layer = layer.child(panels::plain_scrim("match-end-scrim"));
    if let Some((t, true)) = present.confetti_t(ctx.state.clock) {
        // Until the panel is laid out, its top edge is where a panel of its usual height sits.
        let [x, y, pw, _] = panel_at.unwrap_or([
            (width - w) / 2.0,
            height * size::MATCH_END_AT - size::MATCH_END_H / 2.0,
            w,
            0.0,
        ]);
        layer = layer.child(fx::confetti_canvas(fx::fountain(t, [x, x + pw, y])));
    }
    layer.child(
        // A zero-height anchor keeps the panel centred on the 38% line whatever its size.
        row()
            .key("match-end-anchor")
            .absolute(0.0, height * size::MATCH_END_AT)
            .size(width, 0.0)
            .center()
            .child(match_end_panel(ctx, w, t)),
    )
}

/// The match-end panel's card: the star and the match's line (06 §9), the final score and
/// Play again (06 §5.5). `t` is seconds into its `MatchEnd` shot, while it runs.
fn match_end_panel(ctx: &stage::Ctx<'_>, w: f32, t: Option<f32>) -> El {
    let view = ctx.view;
    let line = if copy::you_won(view) {
        copy::BANNER_YOU_WIN
    } else {
        copy::BANNER_THEY_WIN
    };
    let team = view.you.index() % 2;
    let delta = view.delta.unwrap_or([0, 0]);
    // The score shows the last hand's points arriving: the totals before it, then a roll.
    let rolled = t.is_none_or(|t| t >= motion::MATCH_ROLL_AT);
    let value = |i: usize| {
        if rolled {
            view.score[i]
        } else {
            view.score[i].saturating_sub(delta[i])
        }
    };
    let landed = t
        .map(|t| t - motion::MATCH_ROLL_AT - motion::ROLL.duration)
        .filter(|s| *s >= 0.0);
    let pop = |i: usize| landed.filter(|_| delta[i] > 0);
    let score = row()
        .key("match-end-score")
        .role("group")
        .label(format!(
            "Your team {}, other team {}",
            view.score[team],
            view.score[1 - team]
        ))
        .gap(space::SM)
        .center_y()
        .child(
            text("Your team")
                .font(fty::SCORE_LABEL)
                .color(PAL.text_secondary)
                .a11y_hidden(),
        )
        .child(
            header::number("match-end-us", value(team), false, ctx.reduced, pop(team))
                .a11y_hidden(),
        )
        .child(
            text("—")
                .font(fty::SCORE_LABEL)
                .color(FELT.score_divider)
                .a11y_hidden(),
        )
        .child(
            header::number(
                "match-end-them",
                value(1 - team),
                false,
                ctx.reduced,
                pop(1 - team),
            )
            .a11y_hidden(),
        )
        .child(
            text("Other team")
                .font(fty::SCORE_LABEL)
                .color(PAL.text_secondary)
                .a11y_hidden(),
        );
    let mut title = row()
        .key("match-end-title")
        .gap(space::S)
        .center_y()
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .label(line);
    // The star is for a win; a loss reads plainly, with no mark (06 voice: calm, no alarm).
    if copy::you_won(view) {
        title = title.child(
            text("★")
                .font(fty::BANNER)
                .color(FELT.banner_star)
                .a11y_hidden(),
        );
    }
    let title = title.child(
        text(line)
            .font(fty::BANNER)
            .color(FELT.banner_text)
            .a11y_hidden(),
    );
    let again = green_button(
        "match-end-again",
        PLAY_AGAIN,
        ty::LOADING_BUTTON,
        |s: &mut AppState, tx| s.navigate(Route::NewGame, tx),
    );
    motion::e5(
        col()
            .key("match-end-panel")
            .w(w)
            .px(space::XL)
            .py(space::L)
            .gap(space::M)
            .items_center()
            .radius(size::MATCH_END_RADIUS)
            .fill(FELT.banner)
            .border(1.0, rune::Color::WHITE.alpha(0.6))
            .role("group")
            .label(line)
            .transition(panels::panel_transition(ctx.reduced))
            .child(title)
            .child(score)
            .child(again),
    )
}

/// The banner's card: the star and the line, keyed `banner-{id}`.
fn banner_card(b: &present::Banner) -> El {
    row()
        .key(format!("banner-{}", b.id))
        .px(space::L)
        .py(18.0)
        .gap(space::S)
        .center_y()
        .radius(size::BANNER_RADIUS)
        .fill(FELT.banner)
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .label(b.text.clone())
        .child(
            text("★")
                .font(fty::BANNER)
                .color(FELT.banner_star)
                .a11y_hidden(),
        )
        .child(
            text(b.text.clone())
                .font(fty::BANNER)
                .color(FELT.banner_text),
        )
}

/// The table's keys when no control took them (`AppSpec::on_key`): 1 to 6 press that rack slot,
/// arrows bring focus to the rack, Return or P plays the selected card.
pub fn on_key(state: &mut AppState, e: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(table) = state.table.as_ref() else {
        return false;
    };
    match &e.key {
        Key::Text(t) if t.len() == 1 => {
            let ch = t.chars().next().unwrap_or(' ');
            if let Some(d) = ch.to_digit(10).filter(|d| (1..=6).contains(d)) {
                let Some(card) = table.present.rack.card_at(d as usize - 1) else {
                    return false;
                };
                state.press_card(card);
                tx.focus(format!("hand-{card}"), true);
                return true;
            }
            if ch.eq_ignore_ascii_case(&'p') && table.selected.is_some() {
                state.play_selected();
                return true;
            }
            false
        }
        Key::Enter if tx.focused().is_none() && table.selected.is_some() => {
            state.play_selected();
            true
        }
        Key::Left | Key::Right | Key::Home | Key::End if tx.focused().is_none() => {
            let cards: Vec<_> = table.present.rack.present().collect();
            let pick = match e.key {
                Key::Left | Key::End => cards.last(),
                _ => cards.first(),
            };
            match pick {
                Some(c) => {
                    tx.focus(format!("hand-{c}"), true);
                    true
                }
                None => false,
            }
        }
        _ => false,
    }
}

/// What a failed table says: a table that never got a game id (nor a view) failed to be
/// created ([`server::create_failed`]); one that had a game ended ([`server::ended`]).
fn stage_failure(state: &AppState, table: &TableScreen, err: &NetError) -> server::EndedCopy {
    let phase = state.local_server.as_ref();
    if table.game_id.is_none() && table.store.display_view().is_none() {
        server::create_failed(err, phase)
    } else {
        server::ended(err, phase)
    }
}

/// 06 §14.1: before the first view. While the local server boots the line says the table is
/// getting ready; a table that failed says why and offers its way out (contract §N2.5).
fn loading(state: &AppState, table: Option<&TableScreen>) -> El {
    let fresh = TableStore::new();
    let store = table.map_or(&fresh, |t| &t.store);
    let notice_copy = store
        .loading_notice()
        .unwrap_or(euchre_protocol::LoadingNotice {
            heading: euchre_protocol::store::MSG_LOADING_DISCONNECTED_HEADING,
            body: euchre_protocol::store::MSG_LOADING_DISCONNECTED_BODY,
        });
    let phase = state.local_server.as_ref();
    let ended = match table.map(|t| &t.stage) {
        Some(TableStage::Failed(err)) => table.map(|t| stage_failure(state, t, err)),
        _ => None,
    };
    let failure = match (table, &ended) {
        (None, _) => Some(MSG_TABLE_OPEN_FAILED.to_owned()),
        (Some(_), Some(copy)) if copy.action.is_some() => Some(copy.line.clone()),
        (Some(_), Some(_)) => None,
        (Some(_), None) => store.error().map(str::to_owned),
    };
    // Booting: the heading stays and the line says the table is getting ready.
    let line = server::booting_line(phase).unwrap_or(notice_copy.body);
    let mut body = col()
        .key("loading")
        .w_full()
        .max_w(t::LOADING_MAX_W)
        .self_center()
        .items_center()
        .gap(t::LOADING_GAP)
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .child(
            text("♣")
                .key("loading-club")
                .font(ty::LOADING_CLUB)
                .color(PAL.ink)
                .a11y_hidden(),
        )
        .child(
            text(notice_copy.heading)
                .key("loading-h1")
                .font(ty::LOADING_H1)
                .color(PAL.ink)
                .role("heading")
                .level(1),
        )
        .child(
            text(line)
                .key("loading-body")
                .font(ty::LOADING_P)
                .color(PAL.text_secondary)
                .wrap(),
        );
    if let Some(line) = failure {
        body = body.child(notice("loading-error", &line));
    }
    // The primary button follows why the table failed; otherwise it reconnects, except while
    // the server boots: the table opens by itself once it is ready.
    let primary = match &ended {
        Some(copy) => copy.action,
        None if server::booting_line(phase).is_some() => None,
        None => Some(server::EndedAction::Reconnect),
    };
    let mut actions = row().key("loading-actions").gap(space::L).center_y();
    if let Some(action) = primary {
        actions = actions.child(green_button(
            "loading-reconnect",
            action.label(),
            ty::LOADING_BUTTON,
            move |s: &mut AppState, tx| action.run(s, tx),
        ));
    }
    if primary != Some(server::EndedAction::SavedGames) {
        actions = actions.child(text_link(
            "loading-games",
            MSG_YOUR_SAVED_GAMES,
            ty::LOADING_BUTTON,
            PAL.ink,
            |s: &mut AppState, tx| s.navigate(Route::Games, tx),
        ));
    }
    body = body.child(actions);
    col()
        .key("table")
        .w_full()
        .h_full()
        .fill(PAL.cream)
        .padding(space::XL)
        .justify_center()
        // The table replaces this page outright when it arrives: a fading heading or button
        // would print over the felt as it builds in (09 §4.12). A removal always fades, so it
        // fades in a blink.
        .child(body.transition(motion::blink_out()))
}
