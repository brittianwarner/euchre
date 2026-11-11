//! Named states for `--scene NAME --snap out.png` (07 §4.3). Every scene is deterministic and
//! offline: no bridge, no clock, no network.
//!
//! The motion scenes (09 §6.6) are `TableScreen::showing_then(prev, then)`: t = 0 is the table
//! before the moment at rest, and the moment (a sync with its steps, or a press) lands on the
//! next frame, so `--at` and `--frames` read its motion as it plays live. Every table scene
//! marks the first-run tour seen except `table-tour`.

use std::sync::Arc;

use euchre_protocol::store::{HandReviewOutcome, StoreCommand};
use euchre_protocol::{
    ChatEvent, ChatKind, GamePhase, GameStatus, HandResult, ReviewTrickRequest, Seat, Step,
    SubmitMoveRequest, SyncEvent, True,
};

use euchre_profile_model::{ClientMatchReplay, ProfileStats};
use euchre_protocol::{MatchListPage, PublicGameView};
use serde::{Deserialize, Serialize};

use crate::app::{AppState, Route, TableScreen, TableStage, Then};
use crate::net::sim::{
    HeuristicDecider, Outgoing, SimDecider, TableCore, TableSetup, Tempo, fixture_view,
};
use crate::net::{
    BootProblem, NetError, RecentTable, RecentTables, ServerPhase, TableConfig, bridge::Bridge,
};

/// The seed behind the `table-deal` hand.
pub const TABLE_DEAL_SEED: &str = "table-deal";

/// A scene: its name and how it sets the state.
pub type Scene = (&'static str, fn(&mut AppState));

/// Every scene, by name.
pub const SCENES: &[Scene] = &[
    ("home", home),
    ("new", new_game),
    ("table-deal", table_deal),
    ("table-bid", table_bid),
    ("table-play", table_play),
    ("table-trick-won", table_trick_won),
    ("table-hand-scored", table_hand_scored),
    ("game-over", game_over),
    ("table-settings", table_settings),
    ("table-rules", table_rules),
    ("table-review", table_review),
    ("games", games),
    ("settings", settings),
    ("match-detail", match_detail),
    // The motion scenes (09 §6.6).
    ("table-deal-anim", table_deal_anim),
    ("table-upcard-flip", table_upcard_flip),
    ("table-play-opp", table_play_opp),
    ("table-play-you", table_play_you),
    ("table-trick-win", table_trick_win),
    ("table-trick-win-reduced", table_trick_win_reduced),
    ("table-pickup", table_pickup),
    ("table-trump-call", table_trump_call),
    ("table-illegal-shake", table_illegal_shake),
    ("table-hover", table_hover),
    ("table-hand-result", table_hand_result),
    ("table-euchre-stamp", table_euchre_stamp),
    ("game-over-win", game_over_win),
    ("game-over-loss", game_over_loss),
    ("table-rules-open", table_rules_open),
    ("route-home-new", home),
    ("table-reconnect", table_reconnect),
    // Lane E's seams (06 §13, §14.3, §5.8).
    ("table-hint", table_hint),
    ("table-tour", table_tour),
    ("table-banter", table_banter),
    // The embedded server's states (lane N2, contract §N2.9).
    ("server-starting", server_starting),
    ("server-downloading", server_downloading),
    ("server-stopped", server_stopped),
    ("server-stopped-table", server_stopped_table),
    ("server-failed", server_failed),
    ("server-failed-disk", server_failed_disk),
    ("server-failed-port", server_failed_port),
    ("server-relaunch", server_relaunch),
    ("server-stopped-games", server_stopped_games),
    ("games-saved-tables", games_saved_tables),
    // Real server data, from `EUCHRE_LIVE_FIXTURE` (the sim's when it is unset or unreadable).
    ("live-table", live_table),
    ("live-games", live_games),
    ("live-match", live_match),
    ("live-stopped", live_stopped),
];

/// The scenes lane N2 adds: the embedded server's states and the live fixtures.
pub const SERVER_SCENES: &[&str] = &[
    "server-starting",
    "server-downloading",
    "server-stopped",
    "server-stopped-table",
    "server-failed",
    "server-failed-disk",
    "server-failed-port",
    "server-relaunch",
    "server-stopped-games",
    "games-saved-tables",
    "live-table",
    "live-games",
    "live-match",
    "live-stopped",
];

/// The env var naming a [`LiveFixture`] JSON file for the `live-*` scenes.
pub const LIVE_FIXTURE_ENV: &str = "EUCHRE_LIVE_FIXTURE";

/// Real server data for the `live-*` scenes, as JSON (the wire's camel case inside each part):
/// a table's syncs (`live-table` shows `syncs[0]` and lands `syncs[1]`, steps and all, on the
/// next frame, so `--at` reads the moment as it played; bare `views` land step-less), a games
/// page with its stats and the saved tables (`live-games`), and one match (`live-match`). `live-stopped`
/// shows the last sync (or view) with the server stopped under it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveFixture {
    /// Table syncs as the server sent them, oldest first. Preferred over `views`.
    pub syncs: Vec<SyncEvent>,
    /// Table views, oldest first.
    pub views: Vec<PublicGameView>,
    /// A page of the match list.
    pub games: Option<MatchListPage>,
    /// The lifetime stats over the games page.
    pub stats: Option<ProfileStats>,
    /// One match's replay.
    pub replay: Option<ClientMatchReplay>,
    /// The saved tables.
    pub recent: Option<RecentTables>,
}

impl LiveFixture {
    /// The fixture `EUCHRE_LIVE_FIXTURE` names, or `None` (with a warning when it is set but
    /// unreadable).
    pub fn from_env() -> Option<LiveFixture> {
        let path = std::env::var_os(LIVE_FIXTURE_ENV)?;
        let read = std::fs::read(&path)
            .map_err(|err| err.to_string())
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|err| err.to_string()));
        match read {
            Ok(fixture) => Some(fixture),
            Err(err) => {
                tracing::warn!(%err, path = %std::path::Path::new(&path).display(), "live fixture unreadable; showing the sim's");
                None
            }
        }
    }
}

/// Seeds the scenes that need a rarer moment (a euchre, a won or lost match) try, in order.
pub const SEARCH_SEEDS: &[&str] = &[
    DRIVEN_SEED,
    "scene-2",
    "scene-3",
    "scene-4",
    "scene-5",
    "scene-6",
    "scene-7",
    "scene-8",
    "scene-9",
    "scene-10",
    "scene-11",
    "scene-12",
];

/// The seed every driven table scene plays.
pub const DRIVEN_SEED: &str = "table-scenes";

/// The most table events a driven scene plays before it settles for what it has.
const DRIVE_LIMIT: usize = 4000;

fn home(state: &mut AppState) {
    state.route = Route::Home;
}

fn new_game(state: &mut AppState) {
    state.route = Route::NewGame;
}

/// Your games with the sim's seeded history and the lifetime stats (Lane D's fixture).
fn games(state: &mut AppState) {
    crate::screens::other::fixture_games(state);
}

/// Settings with a new profile's defaults loaded (Lane D's fixture).
fn settings(state: &mut AppState) {
    crate::screens::other::fixture_settings(state);
}

/// One finished match with its hands (Lane D's fixture).
fn match_detail(state: &mut AppState) {
    crate::screens::other::fixture_match_detail(state);
}

/// A freshly dealt hand: the up card on the felt and five cards in the rack.
fn table_deal(state: &mut AppState) {
    state.route = Route::Table;
    state.prefs.tour_seen = true;
    state.table = Some(
        match fixture_view(TABLE_DEAL_SEED, TableConfig::default()) {
            Ok(view) => TableScreen::showing(SyncEvent {
                v: view.v,
                view,
                steps: Vec::new(),
            }),
            Err(err) => failed(err.to_string()),
        },
    );
}

/// Plays a fixed match (the human seat on the heuristic too) until `until` holds for a sync,
/// and returns that sync. With `review`, finished tricks wait for Continue, which the driver
/// sends unless the sync it is looking for is the waiting one.
pub fn drive(
    seed: &str,
    review: bool,
    until: impl Fn(&SyncEvent) -> bool,
) -> Result<SyncEvent, String> {
    drive_core(seed, review, until).map(|(sync, _)| sync)
}

/// [`drive`], also handing back the table it played (for its hand archive).
pub fn drive_core(
    seed: &str,
    review: bool,
    until: impl Fn(&SyncEvent) -> bool,
) -> Result<(SyncEvent, TableCore), String> {
    drive_full(seed, review, until).map(|(_, sync, core)| (sync, core))
}

/// [`drive`], also handing back the sync just before the one found (what the table showed
/// before that moment): `(before, found)`. When only the last sync matched, both are it.
pub fn drive_pair(
    seed: &str,
    review: bool,
    until: impl Fn(&SyncEvent) -> bool,
) -> Result<(SyncEvent, SyncEvent), String> {
    drive_full(seed, review, until).map(|(before, sync, _)| (before, sync))
}

/// [`drive_pair`] over each of `seeds` in turn until one reaches the moment.
pub fn search_pair(
    seeds: &[&str],
    review: bool,
    until: impl Fn(&SyncEvent) -> bool,
) -> Result<(SyncEvent, SyncEvent), String> {
    let mut last = Err("no seeds to search".to_owned());
    for seed in seeds {
        last = drive_pair(seed, review, &until);
        if last.is_ok() {
            break;
        }
    }
    last
}

fn drive_full(
    seed: &str,
    review: bool,
    until: impl Fn(&SyncEvent) -> bool,
) -> Result<(SyncEvent, SyncEvent, TableCore), String> {
    let decider = HeuristicDecider;
    let mut core = TableCore::new(
        TableSetup {
            game_id: format!("scene-{seed}"),
            seed: seed.to_owned(),
            cfg: TableConfig::default(),
            first_dealer: Some(euchre_protocol::Seat::EAST),
            tempo: Tempo::INSTANT,
            decider: Arc::new(HeuristicDecider),
        },
        0,
    )
    .map_err(|e| e.to_string())?;
    let mut now = 0u64;
    let mut last = core.connect_sync();
    if review {
        for out in core.review(&ReviewTrickRequest::Set { enabled: true }, now) {
            if let Outgoing::Sync(sync) = out {
                last = *sync;
            }
        }
    }
    for n in 0..DRIVE_LIMIT {
        now += 1;
        let view = core.view();
        let outs = if view.status != GameStatus::Active {
            break;
        } else if view.awaiting_trick_review == Some(true) {
            core.review(
                &ReviewTrickRequest::Continue {
                    turn_id: view.turn_id.clone(),
                },
                now,
            )
        } else if let Some(id) = decider
            .decide(&view)
            .or_else(|| view.legal.first().map(|m| m.id.clone()))
        {
            core.submit(
                &SubmitMoveRequest {
                    move_id: id,
                    turn_id: Some(view.turn_id.clone()),
                    client_move_id: format!("scene-{n}"),
                },
                now,
            )
            .1
        } else if let Some(at) = core.next_due() {
            now = now.max(at);
            core.fire(now)
        } else {
            break;
        };
        for out in outs {
            if let Outgoing::Sync(sync) = out {
                if until(&sync) {
                    return Ok((last, *sync, core));
                }
                last = *sync;
            }
        }
    }
    if until(&last) {
        Ok((last.clone(), last, core))
    } else {
        Err("the scene table never reached its moment".to_owned())
    }
}

fn show(state: &mut AppState, sync: Result<SyncEvent, String>) {
    state.route = Route::Table;
    state.prefs.tour_seen = true;
    state.table = Some(match sync {
        Ok(sync) => TableScreen::showing(sync),
        Err(err) => failed(err),
    });
}

/// A table whose fixture could not be built: the loading view's failure line (the detail goes
/// to the log, never to the screen).
fn failed(err: String) -> TableScreen {
    tracing::warn!(%err, "scene fixture not built");
    let mut screen = TableScreen::creating(0, TableConfig::default());
    screen.stage = TableStage::Failed(NetError::Other(err));
    screen
}

/// Shows `prev` and lands `then` on the next frame.
fn show_then(state: &mut AppState, prev: Result<SyncEvent, String>, then: Then) {
    state.route = Route::Table;
    state.prefs.tour_seen = true;
    state.table = Some(match prev {
        Ok(prev) => TableScreen::showing_then(prev, then),
        Err(err) => failed(err),
    });
}

/// Shows the sync before the moment and lands the moment's sync on the next frame.
fn show_moment(state: &mut AppState, pair: Result<(SyncEvent, SyncEvent), String>) {
    match pair {
        Ok((before, found)) => show_then(state, Ok(before), Then::Sync(Box::new(found))),
        Err(err) => show(state, Err(err)),
    }
}

/// Whether `sync` carries a step that `test` accepts.
fn has_step(sync: &SyncEvent, test: impl Fn(&Step) -> bool) -> bool {
    sync.steps.iter().any(test)
}

/// Whether `seat` sits on the human's team.
fn your_team(sync: &SyncEvent, seat: Seat) -> bool {
    seat.index() % 2 == sync.view.you.index() % 2
}

/// The human to play a card mid-hand, from the second trick on.
fn your_play(s: &SyncEvent) -> bool {
    s.view.phase == GamePhase::TrickPlay
        && s.view.trick.plays.len() >= 2
        && s.view.trick.index >= 1
        && !s.view.legal.is_empty()
}

/// The second round of bidding, the up card turned down, the human to name trump.
fn table_bid(state: &mut AppState) {
    let sync = drive(DRIVEN_SEED, false, |s| {
        s.view.phase == GamePhase::BidRound2 && !s.view.legal.is_empty()
    })
    .or_else(|_| {
        drive(DRIVEN_SEED, false, |s| {
            s.view.phase == GamePhase::BidRound1 && !s.view.legal.is_empty()
        })
    });
    show(state, sync);
}

/// Mid-trick: two or more cards on the felt and the human to play.
fn table_play(state: &mut AppState) {
    let sync = drive(DRIVEN_SEED, false, your_play);
    show(state, sync);
}

/// A finished trick held for review: the winner marked and Continue offered.
fn table_trick_won(state: &mut AppState) {
    let sync = drive(DRIVEN_SEED, true, |s| {
        s.view.awaiting_trick_review == Some(true) && s.view.trick.index >= 1
    });
    show(state, sync);
}

/// A hand just scored: the result line and the score moved.
fn table_hand_scored(state: &mut AppState) {
    let sync = drive(DRIVEN_SEED, false, |s| {
        s.view.phase == GamePhase::HandScore && s.view.score.iter().any(|&p| p > 0)
    });
    show(state, sync);
}

/// The match over: the result, the banner and Play again.
fn game_over(state: &mut AppState) {
    let sync = drive(DRIVEN_SEED, false, |s| {
        s.view.status == GameStatus::Complete
    });
    show(state, sync);
}

/// The table mid-trick with its settings rail open.
fn table_settings(state: &mut AppState) {
    table_play(state);
    if let Some(table) = state.table.as_mut() {
        table.present.hooks.settings_open = true;
    }
}

/// The table mid-trick with the rules open.
fn table_rules(state: &mut AppState) {
    table_play(state);
    if let Some(table) = state.table.as_mut() {
        table.present.hooks.rules_open = true;
    }
}

/// The second hand under way, the first one open in the hand review with its archive.
fn table_review(state: &mut AppState) {
    let driven = drive_core(DRIVEN_SEED, false, |s| {
        s.view.hand_no >= 1 && s.view.phase == GamePhase::BidRound1
    });
    let (sync, core) = match driven {
        Ok(found) => found,
        Err(err) => return show(state, Err(err)),
    };
    show(state, Ok(sync));
    let Some(table) = state.table.as_mut() else {
        return;
    };
    table.present.hooks.hand_review_open = true;
    let fx = table.store.choose_hand_review(0, 0);
    for command in fx.commands {
        if let StoreCommand::GetHandReview { id, hand_no } = command {
            table
                .store
                .on_hand_review(id, HandReviewOutcome::Loaded(core.hand_review(hand_no)));
        }
    }
}

/// The second hand dealt into the rack (09 §4.1).
fn table_deal_anim(state: &mut AppState) {
    let pair = drive_pair(DRIVEN_SEED, false, |s| {
        s.view.hand_no >= 1 && has_step(s, |st| matches!(st, Step::Dealt { .. }))
    });
    show_moment(state, pair);
}

/// The second hand's up card turned on the felt (09 §4.2).
fn table_upcard_flip(state: &mut AppState) {
    let pair = drive_pair(DRIVEN_SEED, false, |s| {
        s.view.hand_no >= 1 && has_step(s, |st| matches!(st, Step::UpCardTurned { .. }))
    });
    show_moment(state, pair);
}

/// An opponent's card flying into the trick (09 §4.3).
fn table_play_opp(state: &mut AppState) {
    let pair = drive_pair(DRIVEN_SEED, false, |s| {
        s.view.trick.index >= 1
            && !has_step(s, |st| matches!(st, Step::TrickWon { .. }))
            && has_step(
                s,
                |st| matches!(st, Step::CardPlayed { seat, .. } if !your_team(s, *seat)),
            )
    });
    show_moment(state, pair);
}

/// The human's card flying from the rack into the trick (09 §4.3).
fn table_play_you(state: &mut AppState) {
    let pair = drive_pair(DRIVEN_SEED, false, |s| {
        s.view.trick.index >= 1
            && !has_step(s, |st| matches!(st, Step::TrickWon { .. }))
            && has_step(
                s,
                |st| matches!(st, Step::CardPlayed { seat, .. } if *seat == s.view.you),
            )
    });
    show_moment(state, pair);
}

fn trick_win_pair() -> Result<(SyncEvent, SyncEvent), String> {
    drive_pair(DRIVEN_SEED, false, |s| {
        has_step(
            s,
            |st| matches!(st, Step::TrickWon { index, .. } if *index >= 1),
        )
    })
}

/// A trick won: the ring, the gather and the sweep inside the hold (09 §4.4).
fn table_trick_win(state: &mut AppState) {
    show_moment(state, trick_win_pair());
}

/// [`table_trick_win`] with the in-app Reduce motion on (D15).
fn table_trick_win_reduced(state: &mut AppState) {
    state.prefs.reduce_motion = true;
    show_moment(state, trick_win_pair());
}

/// The dealer picks up the up card after an order-up (09 §4.6).
fn table_pickup(state: &mut AppState) {
    let pair = search_pair(SEARCH_SEEDS, false, |s| {
        has_step(s, |st| {
            matches!(
                st,
                Step::TrumpSet {
                    via_order_up: true,
                    ..
                }
            )
        })
    });
    show_moment(state, pair);
}

/// Trump named in the second round (09 §4.6).
fn table_trump_call(state: &mut AppState) {
    let pair = search_pair(SEARCH_SEEDS, false, |s| {
        has_step(s, |st| {
            matches!(
                st,
                Step::TrumpSet {
                    via_order_up: false,
                    ..
                }
            )
        })
    });
    show_moment(state, pair);
}

/// Presses the first rack card the view does not allow (the shake, 09 §4.7).
fn press_first_illegal(state: &mut AppState) {
    let card = state.table.as_ref().and_then(|t| {
        let view = t.store.display_view()?;
        view.hand
            .iter()
            .copied()
            .find(|&c| crate::screens::table::copy::card_move(view, c).is_none())
    });
    if let Some(card) = card {
        state.press_card(card);
    }
}

/// The human to follow suit, with a card that does not follow, pressed (09 §4.7).
fn table_illegal_shake(state: &mut AppState) {
    let sync = search_pair(SEARCH_SEEDS, false, |s| {
        your_play(s)
            && s.view
                .hand
                .iter()
                .any(|&c| !s.view.legal.iter().any(|m| m.id == format!("play:{c}")))
    })
    .map(|(_, found)| found);
    show_then(state, sync, Then::Do(press_first_illegal));
}

/// The human to play, for `scripts/hover.json` to hover a rack card (09 §4.11).
fn table_hover(state: &mut AppState) {
    table_play(state);
}

/// A hand scored: the result card and the score roll (09 §4.8).
fn table_hand_result(state: &mut AppState) {
    let pair = drive_pair(DRIVEN_SEED, false, |s| {
        has_step(s, |st| matches!(st, Step::HandScored { .. }))
    });
    show_moment(state, pair);
}

/// The makers euchred: the stamp and its sparks (09 §4.8, §4.9).
fn table_euchre_stamp(state: &mut AppState) {
    let pair = search_pair(SEARCH_SEEDS, false, |s| {
        has_step(s, |st| {
            matches!(
                st,
                Step::HandScored {
                    result: HandResult::Euchre,
                    ..
                }
            )
        })
    });
    show_moment(state, pair);
}

fn game_over_pair(you_win: bool) -> Result<(SyncEvent, SyncEvent), String> {
    search_pair(SEARCH_SEEDS, false, |s| {
        has_step(s, |st| {
            matches!(st, Step::GameWon { team, .. }
                if (team.index() == s.view.you.index() % 2) == you_win)
        })
    })
}

/// The match won: the panel, the confetti and the final roll (09 §4.10).
fn game_over_win(state: &mut AppState) {
    show_moment(state, game_over_pair(true));
}

/// The match lost: the panel and the felt cooling (09 §4.10).
fn game_over_loss(state: &mut AppState) {
    show_moment(state, game_over_pair(false));
}

/// The rules dialog opening over the table (09 §4.12).
fn table_rules_open(state: &mut AppState) {
    let sync = drive(DRIVEN_SEED, false, your_play);
    show_then(state, sync, Then::Do(|s| s.set_rules_open(true)));
}

/// The link drops and comes back: the step-less push lands in place, so t = 0 and t = 1
/// match to the pixel (09 §2.4).
fn table_reconnect(state: &mut AppState) {
    let sync = match drive(DRIVEN_SEED, false, your_play) {
        Ok(sync) => sync,
        Err(err) => return show(state, Err(err)),
    };
    let mut again = sync.clone();
    again.v += 1;
    again.view.v = again.v;
    again.steps.clear();
    show_then(state, Ok(sync), Then::Sync(Box::new(again)));
    if let Some(table) = state.table.as_mut() {
        table.mark_reconnected();
    }
}

/// Coach hints on, the human to play with a choice (06 §13; Lane E draws the hint).
fn table_hint(state: &mut AppState) {
    state.prefs.hints = true;
    let sync = drive(DRIVEN_SEED, false, |s| {
        your_play(s) && s.view.legal.len() > 1
    });
    show(state, sync);
}

/// The first-run tour's first slide over a fresh hand (06 §14.3; Lane E draws it).
fn table_tour(state: &mut AppState) {
    table_deal(state);
    state.prefs.tour_seen = false;
    if let Some(table) = state.table.as_mut() {
        table.present.hooks.tour_step = Some(0);
    }
}

/// A seat's table talk over the felt (06 §5.8; Lane B draws the bubble).
fn table_banter(state: &mut AppState) {
    table_play(state);
    if let Some(table) = state.table.as_mut() {
        table.store.on_chat_event(
            &ChatEvent {
                msg_id: "scene-banter".to_owned(),
                seat: Seat::WEST,
                kind: ChatKind::Banter,
                text: "Somebody brought their good cards tonight.".to_owned(),
                is_final: True,
            },
            0,
        );
    }
}

/* ------------------------------------------------------------------------ */
/* The embedded server (lane N2, contract §N2.9)                             */
/* ------------------------------------------------------------------------ */

/// A game id for the server scenes' tables.
const SCENE_GAME_ID: &str = "g7k2m9q4x1c8v5b3n6h0";

/// A New game while the server boots: the table's loading view, no view yet.
fn booting_table(state: &mut AppState, phase: ServerPhase) {
    state.route = Route::Table;
    state.prefs.tour_seen = true;
    state.local_server = Some(phase);
    let mut screen = TableScreen::creating(0, TableConfig::default());
    screen.game_id = Some(SCENE_GAME_ID.to_owned());
    screen.stage = TableStage::Opening;
    state.table = Some(screen);
}

fn server_starting(state: &mut AppState) {
    booting_table(state, ServerPhase::Starting);
}

fn server_downloading(state: &mut AppState) {
    booting_table(state, ServerPhase::Downloading);
}

/// Home with the server down for `phase`.
fn home_with(state: &mut AppState, phase: ServerPhase) {
    state.route = Route::Home;
    state.local_server = Some(phase);
}

fn server_stopped(state: &mut AppState) {
    home_with(state, ServerPhase::Stopped);
}

/// A table in play when the server stopped: the last view stays, the controls are off and
/// the banner offers Restart.
fn server_stopped_table(state: &mut AppState) {
    table_play(state);
    state.local_server = Some(ServerPhase::Stopped);
    if let Some(table) = state.table.as_mut()
        && table.store.display_view().is_some()
    {
        let _ = table.store.set_connected(false, 0);
        table.stage = TableStage::Failed(NetError::ServerStopped);
    }
}

fn server_failed(state: &mut AppState) {
    home_with(state, ServerPhase::Failed(BootProblem::Other));
}

fn server_failed_disk(state: &mut AppState) {
    home_with(
        state,
        ServerPhase::Failed(BootProblem::LowDisk {
            free_bytes: 2 << 30,
            need_bytes: 4 << 30,
        }),
    );
}

fn server_failed_port(state: &mut AppState) {
    home_with(
        state,
        ServerPhase::Failed(BootProblem::PortInUse {
            #[cfg(not(target_arch = "wasm32"))]
            port: euchre_server_core::config::DEFAULT_LOCAL_ENGINE_PORT,
            // The server crates are native-only; the same port.
            #[cfg(target_arch = "wasm32")]
            port: 6420,
        }),
    );
}

fn server_relaunch(state: &mut AppState) {
    home_with(state, ServerPhase::Failed(BootProblem::Relaunch));
}

/// Your games while the server is stopped: the notice under the pill nav.
fn server_stopped_games(state: &mut AppState) {
    games(state);
    state.local_server = Some(ServerPhase::Stopped);
}

/// Your games with three saved tables (one live, one finished, one unreadable) over the sim's
/// history.
fn games_saved_tables(state: &mut AppState) {
    games(state);
    state.recent_tables = Some(Ok(RecentTables {
        tables: vec![
            RecentTable {
                game_id: SCENE_GAME_ID.to_owned(),
                status: Some(GameStatus::Active),
                score: Some([6, 4]),
                hand_no: Some(7),
            },
            RecentTable {
                game_id: "p3w8e1r6t2y9u4i7o0a5".to_owned(),
                status: Some(GameStatus::Complete),
                score: Some([10, 8]),
                hand_no: Some(13),
            },
            RecentTable {
                game_id: "z5x1c7v3b9n2m8k4j6h0".to_owned(),
                status: None,
                score: None,
                hand_no: None,
            },
        ],
        unavailable: true,
    }));
}

/// `view` as a sync with no steps.
fn sync_of(view: PublicGameView) -> SyncEvent {
    SyncEvent {
        v: view.v,
        view,
        steps: Vec::new(),
    }
}

/// The fixture's table moments, oldest first: its syncs, or its views as step-less syncs.
fn live_syncs() -> Option<Vec<SyncEvent>> {
    let fixture = LiveFixture::from_env()?;
    let syncs = if fixture.syncs.is_empty() {
        fixture.views.into_iter().map(sync_of).collect()
    } else {
        fixture.syncs
    };
    (!syncs.is_empty()).then_some(syncs)
}

/// A real table: the first sync at rest, then the second landing on the next frame.
fn live_table(state: &mut AppState) {
    let Some(syncs) = live_syncs() else {
        return table_play(state);
    };
    let mut syncs = syncs.into_iter();
    let Some(first) = syncs.next() else {
        return table_play(state);
    };
    match syncs.next() {
        Some(next) => show_then(state, Ok(first), Then::Sync(Box::new(next))),
        None => show(state, Ok(first)),
    }
}

/// A real table when the server stopped under it: the last view stays, the controls are off
/// and the banner offers Restart.
fn live_stopped(state: &mut AppState) {
    let Some(last) = live_syncs().and_then(|syncs| syncs.into_iter().last()) else {
        return server_stopped_table(state);
    };
    show(state, Ok(sync_of(last.view)));
    state.local_server = Some(ServerPhase::Stopped);
    if let Some(table) = state.table.as_mut()
        && table.store.display_view().is_some()
    {
        let _ = table.store.set_connected(false, 0);
        table.stage = TableStage::Failed(NetError::ServerStopped);
    }
}

/// A real games page with its saved tables.
fn live_games(state: &mut AppState) {
    let Some(fixture) = LiveFixture::from_env().filter(|f| f.games.is_some()) else {
        return games(state);
    };
    state.route = Route::Games;
    state.games.page = fixture.games;
    state.games.stats = fixture.stats;
    state.games.loading = false;
    state.games.error = None;
    state.games.newer.clear();
    state.games.shown = None;
    state.recent_tables = fixture.recent.map(Ok);
    state.bridge = Some(Bridge::detached());
}

/// A real match.
fn live_match(state: &mut AppState) {
    let Some(replay) = LiveFixture::from_env().and_then(|f| f.replay) else {
        return match_detail(state);
    };
    let id = replay.match_id.clone();
    state.route = Route::MatchDetail(id.clone());
    state.match_detail = Some((id, Ok(replay)));
}
