//! The embedded game server on screen (contract §N2.4, §N2.5): the copy for a server that
//! stopped or could not start, the notice every screen with nav shows while it is down, the
//! loading line while it boots, and what a table that ended says and offers.
//!
//! Every line here is the player's (06 voice): short, calm, and never an error text.

use euchre_protocol::store::{MSG_RECONNECT_TO_GAME, MSG_YOUR_SAVED_GAMES};
use rune::tok::TextStyle;
use rune::{Color, Cursor, El, SpringPair, Transition, Tx, col, row, text};

use crate::app::{AppState, Route};
use crate::net::{BootProblem, NetError, ServerPhase};
use crate::theme::{self, PAL, motion, radius, space, ty};

/// The loading line while the server boots.
pub const BOOTING_LINE: &str = "Getting the table ready…";
/// The loading line while the engine downloads on a first start.
pub const DOWNLOADING_LINE: &str = "Getting the table ready. The first start takes a minute.";
/// The loading line once that download turns slow; it keeps going.
pub const DOWNLOADING_SLOW_LINE: &str = "The download is slow. Euchre keeps trying.";
/// A table that could not be created.
pub const CREATE_FAILED_LINE: &str = "We couldn’t start a game. Try again.";
/// An ended table the server could not reach.
pub const UNREACHABLE_LINE: &str = "We couldn’t reach this table. Your game is saved.";
/// The New game action of an ended table.
pub const NEW_GAME: &str = "New game";

/// The notice's heading.
const HEADING: TextStyle = TextStyle::new(17.0, 600.0).lh(23.0);
/// A notice button's label.
const ACTION: TextStyle = TextStyle::new(15.0, 600.0);
/// The notice's widest measure.
const NOTICE_MAX_W: f32 = 640.0;
/// How far the notice slides as it enters.
const SLIDE: f32 = 12.0;
/// Its top in the band beside the pill: level with the pill's middle for a two-line notice.
const BESIDE_NAV: f32 = 4.0;
/// A two-line notice's height (two 23 pt lines, their gap and the padding), to centre it in
/// Home's header.
const NOTICE_TWO_LINE_H: f32 = 2.0 * 23.0 + space::XS + 2.0 * space::S;
/// The narrowest notice that sits beside the pill; narrower, it goes under it.
const BESIDE_NAV_MIN_W: f32 = 480.0;
/// The dark pages' and New game's content column (42rem) and its inner pad.
const COLUMN_W: f32 = 672.0;

/// What a server notice's button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerAction {
    /// "Try again": boot again (`restart_server`).
    TryAgain,
    /// "Restart": the server stopped; boot again (`restart_server`).
    Restart,
    /// "Reopen Euchre": this process cannot start its server again; a new one can
    /// (`Tx::relaunch`).
    Reopen,
}

impl ServerAction {
    /// The button's label.
    pub fn label(self) -> &'static str {
        match self {
            ServerAction::TryAgain => "Try again",
            ServerAction::Restart => "Restart",
            ServerAction::Reopen => "Reopen Euchre",
        }
    }

    /// Runs it.
    pub fn run(self, state: &mut AppState, tx: &mut Tx) {
        match self {
            ServerAction::TryAgain | ServerAction::Restart => state.restart_server(),
            ServerAction::Reopen => {
                tracing::info!("reopening Euchre to start the game server again");
                tx.relaunch();
            }
        }
    }
}

/// A server notice: heading, line and button (contract §N2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerCopy {
    /// The heading.
    pub heading: &'static str,
    /// The line under it.
    pub line: &'static str,
    /// The button.
    pub action: ServerAction,
}

/// The notice for a server that stopped by itself.
pub const STOPPED: ServerCopy = ServerCopy {
    heading: "The game server stopped.",
    line: "Restart to keep playing. Your games are saved.",
    action: ServerAction::Restart,
};

/// The notice for a server that could not start.
pub fn problem_copy(problem: &BootProblem) -> ServerCopy {
    let (heading, line, action) = match problem {
        BootProblem::AlreadyOpen => (
            "Euchre is already open.",
            "Close the other copy of Euchre, then try again.",
            ServerAction::TryAgain,
        ),
        BootProblem::PortInUse { .. } => (
            "Euchre can’t start its game server.",
            "Another app is in the way. Quit it or restart your computer, then try again.",
            ServerAction::TryAgain,
        ),
        // The engine's floor is 4 GiB (`euchre_server::MIN_FREE_DISK_BYTES`; a test holds the
        // copy to it).
        BootProblem::LowDisk { .. } => (
            "Euchre needs more disk space.",
            "The game server needs 4 GB free. Free up some space, then try again.",
            ServerAction::TryAgain,
        ),
        BootProblem::EngineMissing => (
            "One part of Euchre is missing.",
            "The first start downloads it. Connect to the internet, then try again.",
            ServerAction::TryAgain,
        ),
        BootProblem::DownloadStalled => (
            "The first-start download stopped.",
            "Check your connection, then try again. It picks up where it left off.",
            ServerAction::TryAgain,
        ),
        BootProblem::TooSlow => (
            "The game server is taking too long to start.",
            "Try again in a moment. Your games are saved.",
            ServerAction::TryAgain,
        ),
        BootProblem::DataConflict => (
            "Euchre can’t open its saved data.",
            "Check that your disk isn’t full or locked, then try again.",
            ServerAction::TryAgain,
        ),
        BootProblem::Relaunch => (
            "Euchre needs to reopen.",
            "Reopen Euchre to start the game server again. Your games are saved.",
            ServerAction::Reopen,
        ),
        BootProblem::Other => (
            "The game server didn’t start.",
            "Try again. Your games are saved.",
            ServerAction::TryAgain,
        ),
    };
    ServerCopy {
        heading,
        line,
        action,
    }
}

/// The notice for `phase`: `None` unless the server is down.
pub fn notice_copy(phase: &ServerPhase) -> Option<ServerCopy> {
    match phase {
        ServerPhase::Stopped => Some(STOPPED),
        ServerPhase::Failed(problem) => Some(problem_copy(problem)),
        ServerPhase::Starting
        | ServerPhase::Downloading
        | ServerPhase::DownloadingSlow
        | ServerPhase::Ready => None,
    }
}

/// The table's loading line while the server boots, or `None` once it serves (or when there
/// is no local server).
pub fn booting_line(phase: Option<&ServerPhase>) -> Option<&'static str> {
    match phase? {
        ServerPhase::Starting => Some(BOOTING_LINE),
        ServerPhase::Downloading => Some(DOWNLOADING_LINE),
        ServerPhase::DownloadingSlow => Some(DOWNLOADING_SLOW_LINE),
        ServerPhase::Ready | ServerPhase::Stopped | ServerPhase::Failed(_) => None,
    }
}

/// The way out an ended table offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndedAction {
    /// Restart the server (`restart_server`).
    Restart,
    /// Go to Your games, where the saved tables are.
    SavedGames,
    /// Go to New game.
    NewGame,
    /// Open the same table again (`reconnect`).
    Reconnect,
    /// Create the table again (`reconnect`, which creates when there is no game yet).
    TryAgain,
}

impl EndedAction {
    /// The button's label.
    pub fn label(self) -> &'static str {
        match self {
            EndedAction::Restart => ServerAction::Restart.label(),
            EndedAction::SavedGames => MSG_YOUR_SAVED_GAMES,
            EndedAction::NewGame => NEW_GAME,
            EndedAction::Reconnect => MSG_RECONNECT_TO_GAME,
            EndedAction::TryAgain => ServerAction::TryAgain.label(),
        }
    }

    /// Runs it.
    pub fn run(self, state: &mut AppState, tx: &mut Tx) {
        match self {
            EndedAction::Restart => state.restart_server(),
            EndedAction::SavedGames => state.navigate(Route::Games, tx),
            EndedAction::NewGame => state.navigate(Route::NewGame, tx),
            EndedAction::Reconnect | EndedAction::TryAgain if state.table.is_some() => {
                state.reconnect();
            }
            EndedAction::Reconnect | EndedAction::TryAgain => state.navigate(Route::NewGame, tx),
        }
    }
}

/// What a table that ended (or never opened) says, and its way out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndedCopy {
    /// The line.
    pub line: String,
    /// The button; `None` while the server is starting again (the table reopens by itself).
    pub action: Option<EndedAction>,
}

/// The copy for a table whose stage failed with `err`, with the local server at `phase`
/// (contract §N2.5). A server that is starting again says so instead of asking for a restart:
/// the table reopens by itself once it is ready.
pub fn ended(err: &NetError, phase: Option<&ServerPhase>) -> EndedCopy {
    let reopening = reopening(phase);
    let (line, action) = match err {
        NetError::ServerStopped if reopening => {
            (booting_line(phase).unwrap_or(BOOTING_LINE).to_owned(), None)
        }
        NetError::ServerStopped => (err.to_string(), Some(EndedAction::Restart)),
        NetError::Forbidden | NetError::NotFound => {
            (err.to_string(), Some(EndedAction::SavedGames))
        }
        NetError::Unauthorized | NetError::SignedOut => {
            (err.to_string(), Some(EndedAction::NewGame))
        }
        NetError::Unavailable
        | NetError::Offline
        | NetError::Timeout
        | NetError::RateLimited
        | NetError::Closed => (UNREACHABLE_LINE.to_owned(), Some(EndedAction::Reconnect)),
        NetError::NotWired | NetError::Invalid(_) | NetError::Other(_) => {
            (err.to_string(), Some(EndedAction::Reconnect))
        }
    };
    EndedCopy { line, action }
}

/// Whether the local server is starting again (the table reopens by itself once ready).
fn reopening(phase: Option<&ServerPhase>) -> bool {
    matches!(
        phase,
        Some(ServerPhase::Starting | ServerPhase::Downloading | ServerPhase::DownloadingSlow)
    )
}

/// The copy for a table that could not be created (no game yet), with the local server at
/// `phase` (review fixes PX-6, PP-6): never "Your game is saved", since there is none. A
/// create that timed out may have made a table the reply never named, so it offers New game
/// (a fresh, deliberate start) rather than creating again on its own; Your games lists it if
/// it exists.
pub fn create_failed(err: &NetError, phase: Option<&ServerPhase>) -> EndedCopy {
    match err {
        NetError::ServerStopped | NetError::Unauthorized => ended(err, phase),
        NetError::Timeout => EndedCopy {
            line: CREATE_FAILED_LINE.to_owned(),
            action: Some(EndedAction::NewGame),
        },
        _ => EndedCopy {
            line: CREATE_FAILED_LINE.to_owned(),
            action: Some(EndedAction::TryAgain),
        },
    }
}

/// Where the notice goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoticePlace {
    /// Its left edge, in window points.
    pub left: f32,
    /// Its width.
    pub width: f32,
    /// Its top, when it sits at the top of the window.
    pub top: Option<f32>,
    /// Its gap to the window's foot, when it sits at the foot.
    pub bottom: Option<f32>,
}

/// Where the notice goes. `band` is the free stretch of the top bar, `(from, to)` in window
/// points, once laid out: on Home between the wordmark and the links, elsewhere from the
/// content column to the pill nav. The notice sits in that band when it fits there (centred
/// on Home, from the column's edge elsewhere). Otherwise Home has room for it under its
/// header, and every other screen takes it at the window's foot, where it covers no heading
/// (review fix PX-7: under the pill it covered the page's H1 at 1024 and 720 wide).
pub fn notice_place(route: &Route, width: f32, band: Option<(f32, f32)>) -> NoticePlace {
    let home = *route == Route::Home;
    let bar_h = if width <= theme::window::NARROW_W {
        theme::home::HEADER_H_NARROW
    } else {
        space::HOME_HEADER_H
    };
    if let Some((from, to)) = band {
        let room = to - from - 2.0 * space::M;
        if room >= BESIDE_NAV_MIN_W {
            let w = NOTICE_MAX_W.min(room);
            let (left, top) = if home {
                (
                    from + space::M + (room - w) / 2.0,
                    ((bar_h - NOTICE_TWO_LINE_H) / 2.0).max(0.0),
                )
            } else {
                (from, BESIDE_NAV)
            };
            return NoticePlace {
                left,
                width: w,
                top: Some(top),
                bottom: None,
            };
        }
    }
    let w = NOTICE_MAX_W.min(width - 2.0 * space::L).max(0.0);
    let left = ((width - w) / 2.0).max(0.0);
    if home {
        NoticePlace {
            left,
            width: w,
            top: Some(bar_h + space::M),
            bottom: None,
        }
    } else {
        NoticePlace {
            left,
            width: w,
            top: None,
            bottom: Some(space::L),
        }
    }
}

/// The free stretch of the top bar for the notice (see [`notice_place`]), from the laid-out
/// rects `[x, y, w, h]` of the bar's left part (Home's wordmark; elsewhere none: the content
/// column's edge stands in) and its right part (Home's links, or the pill nav).
pub fn notice_band(
    route: &Route,
    width: f32,
    left_part: Option<[f32; 4]>,
    right_part: Option<[f32; 4]>,
) -> Option<(f32, f32)> {
    let to = right_part?[0];
    let from = if *route == Route::Home {
        let [x, _, w, _] = left_part?;
        x + w
    } else {
        ((width - COLUMN_W) / 2.0).max(0.0) + space::L
    };
    Some((from, to))
}

/// The server notice (contract §N2.5), laid over the top of every screen with nav while the
/// server is down; `None` otherwise. `width` is the window's, `band` the top bar's free
/// stretch ([`notice_band`]) and `reduced` effective reduce motion. It takes no space in the
/// page, so nothing moves when it comes.
pub fn notice(state: &AppState, width: f32, band: Option<(f32, f32)>, reduced: bool) -> Option<El> {
    if !state.route.has_nav() {
        return None;
    }
    let copy = notice_copy(state.local_server.as_ref()?)?;
    let place = notice_place(&state.route, width, band);
    let card_w = place.width;
    let action = copy.action;
    let card = row()
        .key("server-notice")
        .w(card_w)
        .gap(space::M)
        .center_y()
        .px(space::M)
        .py(space::S)
        .radius(radius::BUTTON)
        .fill(PAL.notice_bg)
        .border(1.0, PAL.notice_border)
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .child(
            col()
                .grow(1.0)
                .w(0.0)
                .gap(space::XS)
                .child(text(copy.heading).font(HEADING).color(PAL.ink).wrap())
                .child(text(copy.line).font(ty::NOTICE).color(PAL.ink).wrap()),
        )
        .child(button(
            "server-notice-action",
            action.label(),
            move |s: &mut AppState, tx| action.run(s, tx),
        ));
    // It slides in from the edge it sits on.
    let slide = if place.bottom.is_some() {
        SLIDE
    } else {
        -SLIDE
    };
    let enter = if reduced {
        Transition::opacity()
    } else {
        Transition::offset(0.0, slide).combined(Transition::opacity())
    };
    Some(
        theme::shadows(card, &theme::shadow::TOAST)
            .pin(Some(place.left), place.top, None, place.bottom)
            .raise()
            .transition(enter.springs(SpringPair::new(motion::UI, motion::UI))),
    )
}

/// The banner over a table that ended (contract §N2.5): it takes the place of
/// "Reconnecting…", in the same notice style, with the way out beside the line.
pub fn ended_banner(copy: &EndedCopy) -> El {
    let mut banner = row()
        .key("table-ended")
        .gap(space::M)
        .center_y()
        .px(space::M)
        .py(space::SM)
        .radius(radius::BUTTON)
        .fill(PAL.notice_bg)
        .border(1.0, PAL.notice_border)
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .child(
            text(copy.line.clone())
                .font(ty::NOTICE)
                .color(PAL.ink)
                .wrap()
                .grow(1.0)
                .w(0.0),
        );
    if let Some(action) = copy.action {
        banner = banner.child(button(
            "table-ended-action",
            action.label(),
            move |s: &mut AppState, tx| action.run(s, tx),
        ));
    }
    banner
}

/// The notice's button: the strong green, sized to sit in a notice line.
fn button(
    key: &str,
    label: &'static str,
    on_press: impl Fn(&mut AppState, &mut Tx) + 'static,
) -> El {
    row()
        .key(key.to_owned())
        .min_h(space::TAP)
        .px(space::M)
        .center()
        .radius(radius::BUTTON)
        .fill(PAL.green_button)
        .hover(PAL.green_button_hover)
        .press_scale(0.98)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .role("button")
        .label(label)
        .on_press_tx(on_press)
        .child(text(label).font(ACTION).color(Color::WHITE))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY_PROBLEM: [BootProblem; 9] = [
        BootProblem::AlreadyOpen,
        BootProblem::PortInUse { port: 6420 },
        BootProblem::LowDisk {
            free_bytes: 2 << 30,
            need_bytes: 4 << 30,
        },
        BootProblem::EngineMissing,
        BootProblem::DownloadStalled,
        BootProblem::TooSlow,
        BootProblem::DataConflict,
        BootProblem::Relaunch,
        BootProblem::Other,
    ];

    /// Words that would mean an internal error leaked into the copy.
    const INTERNAL: [&str; 10] = [
        "error",
        "Error",
        "rivet",
        "engine",
        "http",
        "port",
        "panic",
        "::",
        "{",
        "BootError",
    ];

    #[test]
    fn every_problem_has_plain_copy() {
        for problem in &EVERY_PROBLEM {
            let copy = problem_copy(problem);
            for part in [copy.heading, copy.line] {
                assert!(part.ends_with('.'), "{problem:?}: {part}");
                assert!(part.len() < 90, "{problem:?}: {part}");
                for word in INTERNAL {
                    assert!(!part.contains(word), "{problem:?} says {word}: {part}");
                }
            }
            let reopen = matches!(problem, BootProblem::Relaunch);
            assert_eq!(copy.action == ServerAction::Reopen, reopen, "{problem:?}");
        }
        assert_eq!(STOPPED.action.label(), "Restart");
        assert_eq!(ServerAction::TryAgain.label(), "Try again");
        assert_eq!(ServerAction::Reopen.label(), "Reopen Euchre");
    }

    #[test]
    fn low_disk_copy_matches_the_engine_floor() {
        assert_eq!(euchre_server::MIN_FREE_DISK_BYTES, 4 << 30);
        let copy = problem_copy(&BootProblem::LowDisk {
            free_bytes: 1,
            need_bytes: euchre_server::MIN_FREE_DISK_BYTES,
        });
        assert!(copy.line.contains("4 GB"));
    }

    #[test]
    fn the_notice_shows_only_while_the_server_is_down() {
        assert_eq!(notice_copy(&ServerPhase::Stopped), Some(STOPPED));
        assert_eq!(
            notice_copy(&ServerPhase::Failed(BootProblem::TooSlow)),
            Some(problem_copy(&BootProblem::TooSlow))
        );
        for phase in [
            ServerPhase::Starting,
            ServerPhase::Downloading,
            ServerPhase::DownloadingSlow,
            ServerPhase::Ready,
        ] {
            assert_eq!(notice_copy(&phase), None, "{phase:?}");
        }
        assert_eq!(
            booting_line(Some(&ServerPhase::DownloadingSlow)),
            Some(DOWNLOADING_SLOW_LINE)
        );
        assert_eq!(
            booting_line(Some(&ServerPhase::Starting)),
            Some(BOOTING_LINE)
        );
        assert_eq!(
            booting_line(Some(&ServerPhase::Downloading)),
            Some(DOWNLOADING_LINE)
        );
        assert_eq!(booting_line(Some(&ServerPhase::Ready)), None);
        assert_eq!(booting_line(None), None);
    }

    /// Another copy of Euchre and another program behind the port read differently, and
    /// a failed create never claims a saved game.
    #[test]
    fn port_owner_and_create_failures_read_plainly() {
        assert_eq!(
            problem_copy(&BootProblem::AlreadyOpen).heading,
            "Euchre is already open."
        );
        let foreign = problem_copy(&BootProblem::PortInUse { port: 20_000 });
        assert!(!foreign.line.contains("copy of Euchre"), "{}", foreign.line);
        assert!(foreign.line.contains("Another app"));
        let ready = Some(&ServerPhase::Ready);
        for err in [
            NetError::Unavailable,
            NetError::Offline,
            NetError::RateLimited,
            NetError::Closed,
            NetError::Other("boom".into()),
        ] {
            let copy = create_failed(&err, ready);
            assert_eq!(copy.line, CREATE_FAILED_LINE, "{err:?}");
            assert!(!copy.line.contains("saved"), "{err:?}");
            assert_eq!(copy.action, Some(EndedAction::TryAgain), "{err:?}");
        }
        assert_eq!(EndedAction::TryAgain.label(), "Try again");
        // A timed-out create may have made a table: a fresh start, not a silent second one.
        let timeout = create_failed(&NetError::Timeout, ready);
        assert_eq!(timeout.line, CREATE_FAILED_LINE);
        assert_eq!(timeout.action, Some(EndedAction::NewGame));
        // A stopped server still asks for a restart, or says it is starting again.
        assert_eq!(
            create_failed(&NetError::ServerStopped, Some(&ServerPhase::Stopped)).action,
            Some(EndedAction::Restart)
        );
        assert_eq!(
            create_failed(
                &NetError::ServerStopped,
                Some(&ServerPhase::DownloadingSlow)
            ),
            EndedCopy {
                line: DOWNLOADING_SLOW_LINE.to_owned(),
                action: None
            }
        );
    }

    #[test]
    fn ended_copy_follows_the_contract() {
        let ready = Some(&ServerPhase::Ready);
        let stopped = ended(&NetError::ServerStopped, Some(&ServerPhase::Stopped));
        assert_eq!(
            stopped.line,
            "The game server stopped. Restart to keep playing."
        );
        assert_eq!(stopped.action, Some(EndedAction::Restart));
        let forbidden = ended(&NetError::Forbidden, ready);
        assert_eq!(
            forbidden.line,
            "This table belongs to another device. Open Your games to find your table."
        );
        assert_eq!(forbidden.action, Some(EndedAction::SavedGames));
        let missing = ended(&NetError::NotFound, ready);
        assert_eq!(missing.line, "That table could not be found.");
        assert_eq!(missing.action, Some(EndedAction::SavedGames));
        let refused = ended(&NetError::Unauthorized, None);
        assert_eq!(refused.line, "Please start a game on this device first.");
        assert_eq!(refused.action, Some(EndedAction::NewGame));
        for err in [
            NetError::Unavailable,
            NetError::Offline,
            NetError::Timeout,
            NetError::RateLimited,
            NetError::Closed,
        ] {
            let copy = ended(&err, ready);
            assert_eq!(copy.line, UNREACHABLE_LINE, "{err:?}");
            assert_eq!(copy.action, Some(EndedAction::Reconnect), "{err:?}");
        }
        // Internal detail never reaches the line.
        let other = ended(&NetError::Other("rivet: connection reset".into()), None);
        assert_eq!(other.line, "Something went wrong. Please try again.");
        assert_eq!(other.action, Some(EndedAction::Reconnect));
        // While the server starts again the table waits for it instead.
        let booting = ended(&NetError::ServerStopped, Some(&ServerPhase::Downloading));
        assert_eq!(booting.line, DOWNLOADING_LINE);
        assert_eq!(booting.action, None);
        assert_eq!(EndedAction::SavedGames.label(), MSG_YOUR_SAVED_GAMES);
        assert_eq!(EndedAction::Reconnect.label(), MSG_RECONNECT_TO_GAME);
    }
}
