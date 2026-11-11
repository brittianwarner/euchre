//! App state, the router and the chrome every screen shares: the global nav (06 §2.1),
//! toasts and keys.
//!
//! The table store lives here, on the UI thread. Handlers call it and pass its effects to
//! [`TableScreen::apply`], which sends the commands over the [`Bridge`] and re-arms the one
//! timer the store asks for. Replies come back through [`AppState::receive`].

use std::cell::Cell;
use std::time::Duration;

use euchre_protocol::store::RequestId;
use euchre_protocol::{
    Card, GamePhase, ReviewTrickRequest, StoreEffects, StoreEvent, Suit, SyncEvent, TableEvent,
    TableStore,
};
use rune::{Cx, El, Key, KeyEvent, Tx, col, el, row, text};

use crate::assets::Art;
use crate::net::bridge::{Bridge, Intent, Reply};
use crate::net::{ConnEvent, Identity, NetError, RecentTables, ServerPhase, TableConfig, now_ms};
use crate::prefs::Prefs;
use crate::screens;
use crate::screens::other::{MatchDetailData, SettingsData, SettingsEdit};
use crate::screens::table::present::{Cue, Presentation};
use crate::screens::table::{announcer, copy, hint, tour};
use crate::sound::Sound;
use crate::theme::{self, PAL, motion, radius, space, ty};
use crate::url_route::{self, Deep};
use crate::web_history;

/// The screens (06 §2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// `/`.
    Home,
    /// `/new`.
    NewGame,
    /// `/play/[gameId]`: the table in [`AppState::table`].
    Table,
    /// `/games`.
    Games,
    /// `/games/[matchId]`.
    MatchDetail(String),
    /// `/settings`.
    Settings,
}

impl Route {
    /// Whether the global nav shows (everywhere but the live table).
    pub fn has_nav(&self) -> bool {
        *self != Route::Table
    }
}

/// How the next route change reaches the address bar (web plan §3.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UrlMode {
    /// A new history entry.
    #[default]
    Push,
    /// The current entry's URL changes (a redirect, a deep link's first screen).
    Replace,
    /// The browser already moved (back, forward): nothing is written.
    Silent,
}

/// Which way the last route change went (09 §4.12), for the screen transition
/// ([`screens::route_transition`], Lane D).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RouteDir {
    /// A link or a button went somewhere ([`AppState::navigate`]).
    #[default]
    Forward,
    /// Back or Escape returned ([`AppState::back`]).
    Back,
}

/// A change in the rules dialog (06 §5.10). Lane C draws the search box and the accordion and
/// sends these through [`AppState::rules`]; the state lives in `Presentation.hooks`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RulesMsg {
    /// The search text is now this.
    Query(String),
    /// Section `n` (0-based, of the 8) was opened or closed.
    Toggle(usize),
}

/// What a scene does on its second frame ([`TableScreen::showing_then`]), so the change
/// animates from the first frame's picture as it would live.
#[derive(Debug)]
pub enum Then {
    /// A table sync arrives, as from the server.
    Sync(Box<SyncEvent>),
    /// The player does something.
    Do(fn(&mut AppState)),
}

/// What the app knows about its server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerStatus {
    /// Not asked yet.
    Unknown,
    /// Identity answered.
    Ready,
    /// Identity failed; the reason in plain words.
    Unavailable(String),
}

/// How far a table has got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableStage {
    /// `createTable` is in flight.
    Creating,
    /// Connecting to it.
    Opening,
    /// Connected at least once; the store drives the screen.
    Open,
    /// It could not be created or opened, or its connection ended for good; why, in the
    /// player's words (`Display`), and which way out the screen offers (06 §14, contract
    /// §N2.5).
    Failed(NetError),
}

/// The live table: the protocol store plus what the screen adds.
#[derive(Debug)]
pub struct TableScreen {
    /// The bridge connection this screen owns.
    pub conn_id: u64,
    /// The game id, once created.
    pub game_id: Option<String>,
    /// The rules chosen for it, kept for a retry.
    pub cfg: TableConfig,
    /// How far it has got.
    pub stage: TableStage,
    /// The protocol store (02 §6).
    pub store: TableStore,
    /// Which seats are thinking.
    pub thinking: [bool; 4],
    /// The card the player picked (06 §5.6).
    pub selected: Option<Card>,
    /// What the screen adds to the store: slots, gathers, banners, messages.
    pub present: Presentation,
    armed: Option<u64>,
    move_seq: u64,
    /// The in-flight Continue (06 §6.4): only its failure puts the gathered trick back.
    continue_req: Option<RequestId>,
    /// The hand, phase and trick last shown, and whether the link has dropped since: a
    /// step-less push snaps into place only when it moves the table on (or after a reconnect).
    shown: Option<(u32, GamePhase, u8)>,
    reconnected: bool,
    /// A scene's second-frame change and whether the first frame has armed it.
    then: Option<(Then, bool)>,
    /// Commits still owed to the layout once a view shows or the window resizes. Pieces placed
    /// from laid-out rects (the turn ring, 09 §4.5) read the previous layout; when one commit
    /// runs out of measuring passes they would stay where an earlier pass put them (a fresh
    /// table at 1280x800 put the ring beside your seat), so `poll` asks for these.
    layout_settle: u8,
    /// The window size the table view last drew at, and whether it changed since `poll`.
    drawn_size: Cell<([f32; 2], bool)>,
}

/// The commits [`TableScreen::layout_settle`] owes after a view shows or the window resizes.
const LAYOUT_SETTLE: u8 = 2;

impl TableScreen {
    /// A table being created.
    pub fn creating(conn_id: u64, cfg: TableConfig) -> TableScreen {
        TableScreen {
            conn_id,
            game_id: None,
            cfg,
            stage: TableStage::Creating,
            store: TableStore::new(),
            thinking: [false; 4],
            selected: None,
            present: Presentation::default(),
            armed: None,
            move_seq: 0,
            continue_req: None,
            shown: None,
            reconnected: false,
            then: None,
            layout_settle: LAYOUT_SETTLE,
            drawn_size: Cell::new(([0.0, 0.0], false)),
        }
    }

    /// The table view notes the window size it draws at; a change owes the layout its settle
    /// commits ([`TableScreen::layout_settle`]).
    pub fn note_drawn_size(&self, size: [f32; 2]) {
        let (seen, changed) = self.drawn_size.get();
        if seen != size {
            self.drawn_size.set((size, changed || seen != [0.0, 0.0]));
        }
    }

    /// A table showing `sync` with no connection (scenes and tests).
    pub fn showing(sync: SyncEvent) -> TableScreen {
        let mut screen = TableScreen::creating(0, TableConfig::default());
        screen.stage = TableStage::Open;
        let _ = screen.store.attach(true, 0);
        screen.store.on_sync_event(sync);
        screen.after_store();
        screen
    }

    /// A table showing `prev` that does `then` on the next frame (scenes, 09 §6.6): t = 0 is
    /// `prev` at rest and the change lands one frame later (1/60 s at 60 fps), animating as it
    /// would live.
    pub fn showing_then(prev: SyncEvent, then: Then) -> TableScreen {
        let mut screen = TableScreen::showing(prev);
        screen.then = Some((then, false));
        screen
    }

    /// The step-less push that follows a reconnect will snap into place (as after a dropped
    /// link). Scenes and tests use it for `table-reconnect`.
    pub fn mark_reconnected(&mut self) {
        self.reconnected = true;
    }

    /// A table sync (`TableEvent::Sync`). One with no steps (connect, resync) that moves the
    /// table on lands in place; one with steps, or one that only refreshes the same moment,
    /// animates on each element's own spring (06 §6).
    pub fn on_sync(&mut self, sync: SyncEvent, bridge: Option<&Bridge>, tx: &mut Tx) {
        let stepless = sync.steps.is_empty();
        self.store.on_sync_event(sync);
        let first = self.shown.is_none();
        if self.should_snap() && stepless {
            // 09 §2.4: everything in flight lands and nothing in this change flies.
            self.snap_in_place(first, tx);
        }
        if let Some(view) = self.store.display_view() {
            // Thinking ends with the move; a seat that is no longer on turn is not thinking.
            for seat in 0..4 {
                if view.turn_seat.map(|s| s.index()) != Some(seat) {
                    self.thinking[seat] = false;
                }
            }
        }
        self.rearm(bridge);
        self.after_store();
    }

    /// Follows a store change: the presentation catches up and a stale selection drops.
    pub fn after_store(&mut self) {
        self.present.after_change(&self.store);
        if let Some(view) = self.store.display_view()
            && self.selected.is_some_and(|c| !view.hand.contains(&c))
        {
            self.selected = None;
        }
    }

    /// Runs a store call's effects: commands go to the bridge, settlements update the screen,
    /// and the timer follows the store's next deadline.
    pub fn apply(&mut self, fx: StoreEffects, bridge: Option<&Bridge>) {
        for event in &fx.events {
            match event {
                StoreEvent::PlaySettled { ok: true, .. } => self.selected = None,
                StoreEvent::ReviewSettled { id, ok } if self.continue_req == Some(*id) => {
                    self.continue_req = None;
                    if !ok {
                        self.present.review_failed();
                    }
                }
                _ => {}
            }
        }
        if let Some(bridge) = bridge {
            for command in fx.commands {
                bridge.send(Intent::Store {
                    conn_id: self.conn_id,
                    command,
                });
            }
        }
        self.rearm(bridge);
        self.after_store();
    }

    fn rearm(&mut self, bridge: Option<&Bridge>) {
        let next = self.store.next_deadline_ms();
        if next != self.armed {
            self.armed = next;
            if let Some(bridge) = bridge {
                bridge.send(Intent::ArmTimer {
                    conn_id: self.conn_id,
                    at_ms: next,
                });
            }
        }
    }

    /// Starts the Continue that answers a reviewed trick and remembers its id; a refused one
    /// puts the trick back at once.
    pub fn send_continue(&mut self, turn_id: String, bridge: Option<&Bridge>) {
        let fx = self
            .store
            .review_trick(ReviewTrickRequest::Continue { turn_id }, now_ms());
        if fx.review_refused() {
            self.present.review_failed();
        }
        self.continue_req = fx.review_id();
        self.apply(fx, bridge);
    }

    /// Lands a step-less change in place (09 §2.4): the director clears, and `settle` also lands
    /// springs an earlier commit started (a fan closing up, a seat line). The table's `first`
    /// view keeps the window's running tracks: the route's wipe is still crossing the old page,
    /// and the table builds in under it (09 §4.12).
    fn snap_in_place(&mut self, first: bool, tx: &mut Tx) {
        self.present.snap();
        if !first {
            tx.settle();
        }
    }

    /// Whether a step-less update should snap the screen into place: the displayed hand,
    /// phase or trick changed, or the link came back since the last one. Records what is shown.
    fn should_snap(&mut self) -> bool {
        let now = self
            .store
            .display_view()
            .map(|v| (v.hand_no, v.phase, v.trick.index));
        let snap = std::mem::take(&mut self.reconnected) || now != self.shown;
        self.shown = now;
        snap
    }

    /// A fresh client move id: unique per table and process run.
    pub fn next_client_move_id(&mut self) -> String {
        self.move_seq += 1;
        format!("{}-{}", uuid::Uuid::new_v4(), self.move_seq)
    }
}

/// One toast (bottom-right, 06 §1.4 elevation).
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    /// Stable id for keys and dismissal.
    pub id: u64,
    /// The line.
    pub title: String,
    /// When it leaves, on the poll clock (seconds).
    pub until: Option<f64>,
}

/// The match history screen's data.
#[derive(Clone, Debug, Default)]
pub struct GamesState {
    /// The page shown.
    pub page: Option<euchre_protocol::MatchListPage>,
    /// A load is in flight.
    pub loading: bool,
    /// The last load failed.
    pub error: Option<String>,
    /// The lifetime stats (06 §17.1), when read.
    pub stats: Option<euchre_profile_model::ProfileStats>,
    /// The cursors of the pages before the one shown, oldest first (empty on the first page):
    /// `← Newer games` goes back to the last.
    pub newer: Vec<Option<String>>,
    /// The cursor of the page on screen (`None` for the first page), set when its `ListGames`
    /// is sent: `Older games →` pushes it onto [`Self::newer`].
    pub shown: Option<String>,
}

/// The profile settings screen's data (shape owned by lane SERVER-PROFILE; read through
/// Lane D's [`screens::other::parse_settings`]).
#[derive(Clone, Debug, Default)]
pub struct SettingsState {
    /// The settings, as the server returned them.
    pub value: Option<SettingsData>,
    /// A load is in flight.
    pub loading: bool,
    /// The last load failed.
    pub error: Option<String>,
}

/// The New game form (06 §4); both options start off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NewGameForm {
    /// Stick the dealer.
    pub stick: bool,
    /// Require a natural trump to call.
    pub natural: bool,
}

/// Everything the window shows.
pub struct AppState {
    /// The current screen.
    pub route: Route,
    /// Which way the last route change went (09 §4.12).
    pub route_dir: RouteDir,
    /// The screen the last route change left, if any: the window under the screens keeps its
    /// background while the new screen wipes in over it (09 §4.12).
    pub route_from: Option<Route>,
    /// Counts route changes, so the screen that enters paints over the one that leaves.
    pub route_seq: u16,
    /// Where Back goes.
    pub history: Vec<Route>,
    /// The path the address bar last showed (web; empty before the first write).
    pub url: String,
    /// How the next URL change is written.
    pub url_mode: UrlMode,
    /// Who is playing, once known.
    pub identity: Option<Identity>,
    /// The server's state.
    pub server: ServerStatus,
    /// The embedded server's phase (`Reply::Server`); `None` when the backend runs no local
    /// server (sim, remote). Lane N2 draws the boot, stopped and failed states from it.
    pub local_server: Option<ServerPhase>,
    /// The embedded server stopped or failed since it last served: the next `Ready` picks up
    /// where the app was (contract §N2.5).
    pub server_lost: bool,
    /// The saved-tables band (06 §17.1): the last `Reply::RecentTables`, `Err` in plain words.
    pub recent_tables: Option<Result<RecentTables, String>>,
    /// The backend's name (`sim` or `remote`).
    pub backend_name: &'static str,
    /// The live table, if one is open.
    pub table: Option<TableScreen>,
    /// The match history.
    pub games: GamesState,
    /// One match's detail: its id and the server's answer (read through Lane D's
    /// [`screens::other::parse_match_detail`]).
    pub match_detail: Option<(String, Result<MatchDetailData, String>)>,
    /// The profile settings.
    pub settings: SettingsState,
    /// The New game form.
    pub new_game: NewGameForm,
    /// Home: the jacks disclosure is open.
    pub jacks_open: bool,
    /// Settings: the card-art license disclosure is open.
    pub license_open: bool,
    /// Toasts, oldest first.
    pub toasts: Vec<Toast>,
    /// Per-device preferences.
    pub prefs: Prefs,
    /// The sound player (06 §15): off until the player turns sound on (D14), and heard only
    /// while the window shows.
    pub sound: Sound,
    /// The decoded art (set in `setup`).
    pub art: Option<Art>,
    /// The UI's end of the bridge; `None` renders without a network.
    pub bridge: Option<Bridge>,
    /// Whether the window is on screen (the store polls only while it is).
    pub visible: bool,
    /// `setup` blocks until the art decodes (tests; headless renders always do).
    pub wait_for_art: bool,
    /// The poll clock's last reading, in seconds.
    pub clock: f64,
    /// When the route last changed, on the poll clock: the nav ignores presses until
    /// [`motion::ROUTE_SETTLE`] after it.
    pub routed_at: f64,
    /// Reduce motion as the last view resolved it (pref or OS, D15); polls read it.
    pub motion_reduced: Cell<bool>,
    /// The rules dialog's search box (06 §5.10). It outlives the dialog and the table, as the
    /// web keeps the query when the dialog closes.
    pub rules_search: rune::TextEdit,
    next_conn_id: u64,
    next_toast_id: u64,
}

impl Default for AppState {
    fn default() -> Self {
        AppState::new(Prefs::default(), None, "none")
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("route", &self.route)
            .field("server", &self.server)
            .field("table", &self.table)
            .finish_non_exhaustive()
    }
}

/// The toast shown when the server cannot be reached.
pub const MSG_SERVER_UNAVAILABLE: &str = "The table server is not available right now.";
/// The toast shown when saving preferences fails.
pub const MSG_PREFS_NOT_SAVED: &str = "Your preferences could not be saved on this device.";

impl AppState {
    /// The starting state.
    pub fn new(prefs: Prefs, bridge: Option<Bridge>, backend_name: &'static str) -> AppState {
        let mut sound = Sound::new();
        sound.set_enabled(prefs.sound);
        AppState {
            route: Route::Home,
            route_dir: RouteDir::Forward,
            route_from: None,
            route_seq: 0,
            history: Vec::new(),
            url: String::new(),
            url_mode: UrlMode::Push,
            identity: None,
            server: ServerStatus::Unknown,
            local_server: None,
            server_lost: false,
            recent_tables: None,
            backend_name,
            table: None,
            games: GamesState::default(),
            match_detail: None,
            settings: SettingsState::default(),
            new_game: NewGameForm::default(),
            jacks_open: false,
            license_open: false,
            toasts: Vec::new(),
            prefs,
            sound,
            art: None,
            bridge,
            visible: true,
            wait_for_art: false,
            clock: 0.0,
            routed_at: f64::NEG_INFINITY,
            motion_reduced: Cell::new(false),
            rules_search: rune::TextEdit::new(""),
            next_conn_id: 1,
            next_toast_id: 1,
        }
    }

    /// The window drew its first frame: say hello to the server.
    pub fn on_ready(&mut self) {
        self.send(Intent::Identify);
    }

    fn send(&self, intent: Intent) {
        if let Some(bridge) = &self.bridge {
            bridge.send(intent);
        }
    }

    /// Goes to `route`, remembering where Back returns.
    pub fn navigate(&mut self, route: Route, tx: &mut Tx) {
        if route == self.route {
            return;
        }
        // On the web the marketing Home is never shown: Home is the site's own `/`.
        if route == Route::Home && web_history::IS_WEB {
            let _ = tx.open_url_same_tab("/");
            return;
        }
        let from = std::mem::replace(&mut self.route, route.clone());
        if from == Route::Table && route != Route::Table {
            self.close_table();
        }
        if from != Route::Table {
            self.history.push(from.clone());
        }
        self.route_dir = RouteDir::Forward;
        self.routed(from);
        self.enter(&route);
        self.sync_url();
        // Each element keeps its own spring (the route transition's, the Home to Table hero's
        // `HERO`); a `tx.motion` here would override them all. The hero flies outside clips.
        if route == Route::Table {
            tx.unclipped_flights();
        }
    }

    /// Goes back one screen; false at Home.
    pub fn back(&mut self, tx: &mut Tx) -> bool {
        let Some(prev) = self.history.pop() else {
            if self.route != Route::Home {
                self.navigate(Route::Home, tx);
                self.history.clear();
                self.route_dir = RouteDir::Back;
                return true;
            }
            return false;
        };
        let from = std::mem::replace(&mut self.route, prev.clone());
        if from == Route::Table {
            self.close_table();
        }
        self.route_dir = RouteDir::Back;
        self.routed(from);
        self.enter(&prev);
        self.sync_url();
        true
    }

    /// Writes the current screen's path to the address bar, as [`AppState::url_mode`] says.
    /// A table has no path until its id is known; the reply that brings the id calls this again.
    fn sync_url(&mut self) {
        let id = self.table.as_ref().and_then(|t| t.game_id.as_deref());
        let Some(path) = url_route::path_for(&self.route, id) else {
            return;
        };
        if path == self.url {
            return;
        }
        match std::mem::take(&mut self.url_mode) {
            UrlMode::Push => web_history::push(&path),
            UrlMode::Replace => web_history::replace(&path),
            UrlMode::Silent => {}
        }
        self.url = path;
    }

    /// Opens the screen a URL names (a deep link at load, or the browser's back and forward).
    /// `tx` is `None` before the first frame, where there is nothing to animate.
    pub fn open_deep(&mut self, deep: Deep, mode: UrlMode, tx: Option<&mut Tx>) {
        let route = match deep {
            // The Svelte site owns `/`; the app has nothing of its own to show there.
            Deep::Home if web_history::IS_WEB => {
                if let Some(tx) = tx {
                    let _ = tx.open_url_same_tab("/");
                    return;
                }
                Route::NewGame
            }
            Deep::Home => Route::Home,
            Deep::New => Route::NewGame,
            Deep::Games => Route::Games,
            Deep::Match(id) => Route::MatchDetail(id),
            Deep::Settings => Route::Settings,
            Deep::Create { stick, natural } => {
                self.new_game = NewGameForm { stick, natural };
                self.begin_table(None);
                Route::Table
            }
            Deep::Play(id) => {
                self.begin_table(Some(id));
                Route::Table
            }
            // An unknown path lands on the new-game screen and the bar says so.
            Deep::Unknown(_) => Route::NewGame,
        };
        let creating =
            route == Route::Table && self.table.as_ref().is_some_and(|t| t.game_id.is_none());
        self.url_mode = if creating || matches!(mode, UrlMode::Push) && self.url.is_empty() {
            UrlMode::Replace
        } else {
            mode
        };
        if route != self.route || route == Route::Table {
            let from = std::mem::replace(&mut self.route, route.clone());
            if from == Route::Table && route != Route::Table {
                self.close_table();
            }
            self.route_dir = RouteDir::Forward;
            if tx.is_some() {
                self.routed(from);
            }
            self.enter(&route);
        }
        self.sync_url();
    }

    /// Stamps a route change that left `from`.
    fn routed(&mut self, from: Route) {
        self.route_from = Some(from);
        self.route_seq = self.route_seq.wrapping_add(1);
        self.routed_at = self.clock;
    }

    /// A nav link's press: goes to `route` unless the route changed under the pointer a moment
    /// ago (the second click of a double-click lands on the new screen's nav, B1).
    pub fn nav_to(&mut self, route: Route, tx: &mut Tx) {
        if self.clock - self.routed_at < motion::ROUTE_SETTLE {
            return;
        }
        self.navigate(route, tx);
    }

    /// Loads the current screen's data again (the Try again link after a failed load).
    pub fn reload_route(&mut self) {
        let route = self.route.clone();
        self.enter(&route);
    }

    fn enter(&mut self, route: &Route) {
        match route {
            Route::Games => {
                self.games.loading = true;
                self.games.error = None;
                self.games.newer.clear();
                self.games.shown = None;
                self.send(Intent::ListGames { cursor: None });
                self.send(Intent::GetStats);
                self.send(Intent::RecentTables);
            }
            Route::MatchDetail(id) => {
                self.match_detail = None;
                self.send(Intent::MatchDetail {
                    match_id: id.clone(),
                });
            }
            Route::Settings => {
                self.settings.loading = true;
                self.settings.error = None;
                self.send(Intent::GetSettings);
            }
            Route::Home | Route::NewGame | Route::Table => {}
        }
    }

    /// New game's submit: create the table and show it (06 §2.2, §14.1).
    pub fn start_game(&mut self, tx: &mut Tx) {
        self.begin_table(None);
        self.navigate(Route::Table, tx);
    }

    /// Replaces the open table with one being made (`None`, from the New game form's rules) or
    /// one being opened (`Some(game_id)`), and asks the server for it. Shows nothing yet.
    fn begin_table(&mut self, game_id: Option<String>) {
        let cfg = TableConfig {
            stick_the_dealer: self.new_game.stick,
            require_natural_trump: self.new_game.natural,
        };
        self.close_table();
        let conn_id = self.next_conn_id;
        self.next_conn_id += 1;
        let mut screen = TableScreen::creating(conn_id, cfg);
        match game_id {
            Some(game_id) => {
                screen.game_id = Some(game_id.clone());
                screen.stage = TableStage::Opening;
                screen.cfg = TableConfig::default();
                self.send(Intent::OpenTable { conn_id, game_id });
            }
            None => self.send(Intent::CreateTable { conn_id, cfg }),
        }
        self.table = Some(screen);
    }

    /// Restart (or try again to start) the embedded server: the server notice's action and the
    /// ended table's Restart. The phase moves to `Starting` at once, so the notice leaves.
    pub fn restart_server(&mut self) {
        self.send(Intent::RestartServer);
    }

    /// The embedded server moved to `phase`. Back to `Ready` after it stopped or failed (and
    /// started again), the app picks up where it was (contract §N2.5): who is playing and the
    /// saved tables again, the open table on a new connection (same game id, so the store
    /// resyncs to the same turn), and the current screen's data.
    pub fn on_server_phase(&mut self, phase: ServerPhase) {
        if matches!(phase, ServerPhase::Stopped | ServerPhase::Failed(_)) {
            self.server_lost = true;
        }
        let ready = phase == ServerPhase::Ready;
        self.local_server = Some(phase);
        if !ready || !std::mem::take(&mut self.server_lost) {
            return;
        }
        tracing::info!("the game server is back; reconnecting");
        self.send(Intent::Identify);
        // Games asks for the saved tables itself as it reloads.
        if self.route != Route::Games {
            self.send(Intent::RecentTables);
        }
        if self.table.is_some() {
            self.reconnect();
        }
        match self.route.clone() {
            Route::Games | Route::MatchDetail(_) => self.reload_route(),
            Route::Settings => {
                self.send(Intent::GetStats);
                self.reload_route();
            }
            Route::Home | Route::NewGame | Route::Table => {}
        }
    }

    /// Whether the embedded server cannot serve right now (stopped, or it did not start): the
    /// server notice shows and the ended table offers Restart.
    pub fn server_down(&self) -> bool {
        matches!(
            self.local_server,
            Some(ServerPhase::Stopped | ServerPhase::Failed(_))
        )
    }

    /// Reconnect (06 §14.1): reopen the current table on a new connection, or create it
    /// again if it never existed. The table keeps its screen and store, so its last view stays
    /// on the felt (with "Reconnecting…") until the new connection resyncs it, rather than
    /// blanking to the loading page (review fix PX-4).
    pub fn reconnect(&mut self) {
        let conn_id = self.next_conn_id;
        let now = now_ms();
        let bridge = self.bridge.as_ref();
        let Some(table) = self.table.as_mut() else {
            return;
        };
        if table.game_id.is_none() && table.stage == TableStage::Failed(NetError::Timeout) {
            // A create that timed out may have made a table its reply never named: never
            // create a second one by itself. The table offers New game (review fix PP-6).
            return;
        }
        self.next_conn_id += 1;
        let old = std::mem::replace(&mut table.conn_id, conn_id);
        // The old connection's timer and replies no longer reach this table.
        table.armed = None;
        table.reconnected = true;
        let fx = table.store.set_connected(false, now);
        table.apply(fx, bridge);
        let (game_id, cfg) = (table.game_id.clone(), table.cfg);
        table.stage = if game_id.is_some() {
            TableStage::Opening
        } else {
            TableStage::Creating
        };
        self.send(Intent::CloseTable { conn_id: old });
        match game_id {
            Some(game_id) => self.send(Intent::OpenTable { conn_id, game_id }),
            None => self.send(Intent::CreateTable { conn_id, cfg }),
        }
    }

    /// Opens a saved table (06 §17.1, the Games saved-tables band): closes the current table,
    /// opens `game_id` on a new connection and shows it.
    pub fn continue_table(&mut self, game_id: String, tx: &mut Tx) {
        self.begin_table(Some(game_id));
        self.navigate(Route::Table, tx);
    }

    /// Keeps the saved-tables band honest about the table on screen: its row takes the live
    /// view's status, score and hand. The band was last read from the server (at start, or when
    /// a Restart brought it back), so without this a match played on since would still read
    /// "Resume game" at the old hand until the next read lands.
    fn note_open_table(&mut self) {
        let Some(table) = self.table.as_ref() else {
            return;
        };
        let (Some(game_id), Some(view)) = (table.game_id.as_deref(), table.store.view()) else {
            return;
        };
        let Some(Ok(recent)) = self.recent_tables.as_mut() else {
            return;
        };
        if let Some(row) = recent.tables.iter_mut().find(|t| t.game_id == game_id) {
            row.status = Some(view.status);
            row.score = Some(view.score);
            row.hand_no = Some(view.hand_no);
        }
    }

    fn close_table(&mut self) {
        if let Some(table) = self.table.take() {
            self.send(Intent::CloseTable {
                conn_id: table.conn_id,
            });
        }
    }

    /// The table on `conn_id`, with the bridge, if it is still the one on screen.
    fn table_for(&mut self, conn_id: u64) -> Option<(&mut TableScreen, Option<&Bridge>)> {
        let bridge = self.bridge.as_ref();
        self.table
            .as_mut()
            .filter(|t| t.conn_id == conn_id)
            .map(|t| (t, bridge))
    }

    /// Selects `card` in the rack (06 §5.6); the Play button then plays it. Ignored while
    /// controls are off.
    pub fn select_card(&mut self, card: Card) {
        if let Some(table) = self.table.as_mut()
            && table.store.controls_enabled()
        {
            table.selected = Some(card);
        }
    }

    /// Plays (or discards) `card` if the view allows it (06 §5.6).
    pub fn play_card(&mut self, card: Card) {
        let bridge = self.bridge.as_ref();
        let Some(table) = self.table.as_mut() else {
            return;
        };
        let Some(view) = table.store.display_view() else {
            return;
        };
        let id = format!("play:{card}");
        let discard = format!("discard:{card}");
        let Some(entry) = view
            .legal
            .iter()
            .find(|m| m.id == id || m.id == discard)
            .map(|m| m.id.clone())
        else {
            if table.store.controls_enabled() {
                let reason = copy::illegal_reason(view, card);
                table.present.show_illegal(reason);
                table.present.shake_card(card);
            }
            return;
        };
        let streaks = hint::record_move(view, &entry, &self.prefs);
        table.present.director.fast_forward(self.clock);
        let client_move_id = table.next_client_move_id();
        table.selected = Some(card);
        let fx = table.store.submit_move(&entry, client_move_id, now_ms());
        table.apply(fx, bridge);
        self.save_streaks(streaks);
    }

    /// Keeps the hint streaks Lane E's [`hint::record_move`] returned, if it changed them.
    fn save_streaks(&mut self, streaks: Option<std::collections::BTreeMap<String, u32>>) {
        if let Some(streaks) = streaks {
            self.update_prefs(|p| p.hint_streaks = streaks);
        }
    }

    /// A press on a rack card (06 §5.6): a card with a legal move is selected; any other shows
    /// why it cannot be played for 3.2 s and leaves the selection alone (06 §5.7).
    pub fn press_card(&mut self, card: Card) {
        let Some(table) = self.table.as_mut() else {
            return;
        };
        if !table.store.controls_enabled() {
            return;
        }
        let Some(view) = table.store.display_view() else {
            return;
        };
        if copy::card_move(view, card).is_some() {
            table.selected = Some(card);
            table.present.illegal = None;
        } else {
            let reason = copy::illegal_reason(view, card);
            table.present.show_illegal(reason);
            table.present.shake_card(card);
        }
    }

    /// Plays the selected card, if any (the Play button, Return, P).
    pub fn play_selected(&mut self) {
        if let Some(card) = self.table.as_ref().and_then(|t| t.selected) {
            self.play_card(card);
        }
    }

    /// Continue after a reviewed trick (06 §6.4): gather the cards, then answer the table.
    pub fn continue_trick(&mut self) {
        let Some(table) = self.table.as_mut() else {
            return;
        };
        let ready = table.store.display_view().is_some_and(|v| {
            v.awaiting_trick_review == Some(true)
                && table.store.is_connected()
                && !table.store.review_busy()
        });
        if ready {
            table.present.director.fast_forward(self.clock);
            table.present.press_continue();
        }
    }

    /// Ticks or clears the Go alone box for the current decision.
    pub fn toggle_go_alone(&mut self) {
        if let Some(table) = self.table.as_mut()
            && let Some(view) = table.store.display_view()
        {
            let view = view.clone();
            table.present.toggle_go_alone(&view);
        }
    }

    /// Turns "review each trick" on or off for this table (06 §6.4). The settings rail's hook.
    pub fn set_review_tricks(&mut self, enabled: bool) {
        let bridge = self.bridge.as_ref();
        if let Some(table) = self.table.as_mut() {
            let fx = table
                .store
                .review_trick(ReviewTrickRequest::Set { enabled }, now_ms());
            table.apply(fx, bridge);
        }
    }

    /// Opens or closes the table settings rail (06 §5.9).
    pub fn toggle_settings(&mut self) {
        if let Some(table) = self.table.as_mut() {
            let hooks = &mut table.present.hooks;
            hooks.settings_open = !hooks.settings_open;
        }
    }

    /// Opens or closes the rules dialog (06 §5.10).
    pub fn set_rules_open(&mut self, open: bool) {
        if let Some(table) = self.table.as_mut() {
            table.present.hooks.rules_open = open;
        }
    }

    /// A change in the rules dialog's search or accordion (Lane C's controls).
    pub fn rules(&mut self, msg: RulesMsg) {
        let Some(table) = self.table.as_mut() else {
            return;
        };
        let hooks = &mut table.present.hooks;
        match msg {
            RulesMsg::Query(query) => hooks.rules_query = query,
            RulesMsg::Toggle(n) if n < 8 => hooks.rules_flipped ^= 1 << n,
            RulesMsg::Toggle(_) => {}
        }
    }

    /// Opens the first-run tour at its first slide (the rules panel's "take the quick tour
    /// again"; 06 §14.3). It does not re-arm the auto-open.
    pub fn tour_open(&mut self) {
        if let Some(table) = self.table.as_mut() {
            table.present.hooks.tour_step = Some(0);
        }
    }

    /// The tour's Next; on the last slide it closes the tour.
    pub fn tour_advance(&mut self) {
        let Some(step) = self.table.as_ref().and_then(|t| t.present.hooks.tour_step) else {
            return;
        };
        if step + 1 >= tour::SLIDES {
            self.tour_dismiss();
        } else if let Some(table) = self.table.as_mut() {
            table.present.hooks.tour_step = Some(step + 1);
        }
    }

    /// The tour's Back.
    pub fn tour_back(&mut self) {
        if let Some(table) = self.table.as_mut()
            && let Some(step) = table.present.hooks.tour_step
        {
            table.present.hooks.tour_step = Some(step.saturating_sub(1));
        }
    }

    /// Closes the tour (Skip, Escape, the backdrop, the last slide) and remembers it was seen.
    pub fn tour_dismiss(&mut self) {
        if let Some(table) = self.table.as_mut() {
            table.present.hooks.tour_step = None;
        }
        if !self.prefs.tour_seen {
            self.update_prefs(|p| p.tour_seen = true);
        }
    }

    /// Where the deck sits this frame, in window points (Lane B's `deck-{hand}` `on_layout`;
    /// the deal-in starts there, 09 §4.1).
    pub fn deck_laid_out(&mut self, rect: [f32; 4]) {
        if let Some(table) = self.table.as_mut() {
            table.present.deck_at = Some(rect);
        }
    }

    /// Opens or closes the rail's Table talk disclosure.
    pub fn toggle_table_talk(&mut self) {
        if let Some(table) = self.table.as_mut() {
            let hooks = &mut table.present.hooks;
            hooks.talk_open = !hooks.talk_open;
        }
    }

    /// Opens the hand review (06 §11) on the store's default hand and asks for its archive.
    pub fn open_hand_review(&mut self) {
        let bridge = self.bridge.as_ref();
        if let Some(table) = self.table.as_mut() {
            table.present.hooks.hand_review_open = true;
            let fx = table.store.open_hand_review(now_ms());
            table.apply(fx, bridge);
        }
    }

    /// Shows hand `hand_no` in the open hand review.
    pub fn choose_hand_review(&mut self, hand_no: u32) {
        let bridge = self.bridge.as_ref();
        if let Some(table) = self.table.as_mut() {
            let fx = table.store.choose_hand_review(hand_no, now_ms());
            table.apply(fx, bridge);
        }
    }

    /// Closes the hand review.
    pub fn close_hand_review(&mut self) {
        if let Some(table) = self.table.as_mut() {
            table.present.hooks.hand_review_open = false;
        }
    }

    /// Escape on the table closes the topmost panel: the tour, the hand review, the rules,
    /// then the settings rail. False when none was open.
    fn close_table_panel(&mut self) -> bool {
        let touring = self
            .table
            .as_ref()
            .is_some_and(|t| t.present.hooks.tour_step.is_some());
        if touring {
            self.tour_dismiss();
            return true;
        }
        let Some(table) = self.table.as_mut() else {
            return false;
        };
        let hooks = &mut table.present.hooks;
        if hooks.hand_review_open {
            hooks.hand_review_open = false;
        } else if hooks.rules_open {
            hooks.rules_open = false;
        } else if hooks.settings_open {
            hooks.settings_open = false;
        } else {
            return false;
        }
        true
    }

    /// The pointer entered or left a rack card.
    pub fn hover_card(&mut self, card: Card, inside: bool) {
        if let Some(table) = self.table.as_mut() {
            if inside {
                table.present.hover = Some(card);
            } else if table.present.hover == Some(card) {
                table.present.hover = None;
            }
        }
    }

    /// Submits a non-card move (a bid, a cut) by its legal id.
    pub fn submit_move(&mut self, move_id: &str) {
        let bridge = self.bridge.as_ref();
        let Some(table) = self.table.as_mut() else {
            return;
        };
        let streaks = table
            .store
            .display_view()
            .and_then(|view| hint::record_move(view, move_id, &self.prefs));
        table.present.director.fast_forward(self.clock);
        let client_move_id = table.next_client_move_id();
        let fx = table.store.submit_move(move_id, client_move_id, now_ms());
        table.apply(fx, bridge);
        self.save_streaks(streaks);
    }

    /// Flips a preference and writes the file on the bridge thread.
    pub fn update_prefs(&mut self, change: impl FnOnce(&mut Prefs)) {
        change(&mut self.prefs);
        self.send(Intent::SavePrefs(self.prefs.clone()));
    }

    /// The Sound toggle (the table rail and the Settings screen; D14: off by default).
    pub fn toggle_sound(&mut self) {
        self.update_prefs(|p| p.sound = !p.sound);
        self.sound.set_enabled(self.prefs.sound);
        // 06 §15.1: turning sound on plays the trump cue as a test.
        if self.prefs.sound && self.visible {
            self.sound.play(&Cue::TrumpCalled(Suit::Hearts));
        }
    }

    /// The rules search box's editor (`El::on_edit`, Lane C's rules dialog).
    pub fn rules_search(&mut self) -> &mut rune::TextEdit {
        &mut self.rules_search
    }

    /// Sends a change to the profile settings (Lane D's Settings screen); the saved settings
    /// come back as a settings reply.
    pub fn patch_settings(&mut self, edit: SettingsEdit) {
        self.send(Intent::PatchSettings(screens::other::settings_patch_value(
            &edit,
        )));
    }

    /// Queues a toast; it leaves after [`motion::TOAST_SECS`].
    pub fn toast(&mut self, title: impl Into<String>, tx: &mut Tx) {
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        self.toasts.push(Toast {
            id,
            title: title.into(),
            until: None,
        });
        tx.wake_after(std::time::Duration::from_secs_f64(motion::TOAST_SECS));
    }

    /// Closes one toast.
    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
    }

    /// Applies one reply from the bridge.
    pub fn receive(&mut self, reply: Reply, tx: &mut Tx) {
        let now = now_ms();
        match reply {
            Reply::Identity(Ok(identity)) => {
                self.identity = Some(identity);
                self.server = ServerStatus::Ready;
            }
            Reply::Identity(Err(err)) => {
                tracing::warn!(?err, "identity unavailable");
                // With a local server, its notice already says what is wrong and offers the way
                // out; a toast on top would say it twice.
                if self.server != ServerStatus::Unavailable(err.to_string())
                    && self.local_server.is_none()
                {
                    self.toast(MSG_SERVER_UNAVAILABLE, tx);
                }
                self.server = ServerStatus::Unavailable(err.to_string());
            }
            Reply::TableCreated { conn_id, result } => {
                if let Some((table, _)) = self.table_for(conn_id) {
                    match result {
                        Ok(game_id) => {
                            table.game_id = Some(game_id);
                            table.stage = TableStage::Opening;
                        }
                        Err(err) => table.stage = TableStage::Failed(err),
                    }
                }
                self.sync_url();
            }
            Reply::TableOpened { conn_id, result } => {
                if let Some((table, bridge)) = self.table_for(conn_id) {
                    match result {
                        Ok(game_id) => {
                            table.game_id = Some(game_id);
                            table.stage = TableStage::Open;
                            let fx = table.store.attach(false, now);
                            table.apply(fx, bridge);
                        }
                        Err(err) => table.stage = TableStage::Failed(err),
                    }
                }
            }
            Reply::Conn { conn_id, event } => {
                if let Some((table, bridge)) = self.table_for(conn_id) {
                    on_conn_event(table, bridge, event, now, tx);
                }
                self.note_open_table();
            }
            Reply::Snapshot { conn_id, id, view } => {
                if let Some((table, bridge)) = self.table_for(conn_id) {
                    let fx = table.store.on_snapshot(id, view, now);
                    let first = table.shown.is_none();
                    if table.should_snap() {
                        table.snap_in_place(first, tx);
                    }
                    table.apply(fx, bridge);
                }
                self.note_open_table();
            }
            Reply::Submit {
                conn_id,
                id,
                outcome,
            } => {
                if let Some((table, bridge)) = self.table_for(conn_id) {
                    let fx = table.store.on_submit(id, outcome, now);
                    table.apply(fx, bridge);
                }
            }
            Reply::Review { conn_id, id, ack } => {
                if let Some((table, bridge)) = self.table_for(conn_id) {
                    let fx = table.store.on_review(id, ack);
                    table.apply(fx, bridge);
                }
            }
            Reply::HandReview {
                conn_id,
                id,
                outcome,
            } => {
                if let Some((table, bridge)) = self.table_for(conn_id) {
                    table.store.on_hand_review(id, outcome);
                    table.rearm(bridge);
                    table.after_store();
                }
            }
            Reply::Timer { conn_id } => {
                let visible = self.visible;
                if let Some((table, bridge)) = self.table_for(conn_id) {
                    table.armed = None;
                    let fx = table.store.tick(now, visible);
                    table.apply(fx, bridge);
                }
            }
            Reply::Games { cursor, result } => {
                // Only the page on screen: a late reply for a page the player has left, or
                // one that lands on another route, is dropped (review fix PP-5).
                if self.route != Route::Games || cursor != self.games.shown {
                    tracing::debug!("a games page for another view; dropped");
                    return;
                }
                self.games.loading = false;
                match result {
                    Ok(page) => {
                        self.games.page = Some(page);
                        self.games.error = None;
                    }
                    Err(err) => self.games.error = Some(err.to_string()),
                }
            }
            Reply::MatchDetail { match_id, result } => {
                if self.route == Route::MatchDetail(match_id.clone()) {
                    let detail = result
                        .map_err(|e| e.to_string())
                        .and_then(screens::other::parse_match_detail);
                    self.match_detail = Some((match_id, detail));
                }
            }
            Reply::Settings(result) => {
                self.settings.loading = false;
                match result
                    .map_err(|e| e.to_string())
                    .and_then(screens::other::parse_settings)
                {
                    Ok(value) => {
                        self.settings.value = Some(value);
                        self.settings.error = None;
                    }
                    Err(err) => self.settings.error = Some(err),
                }
            }
            Reply::PrefsSaved(Ok(())) => {}
            Reply::PrefsSaved(Err(err)) => {
                tracing::warn!(%err, "prefs not saved");
                self.toast(MSG_PREFS_NOT_SAVED, tx);
            }
            Reply::Stats(Ok(stats)) => self.games.stats = Some(stats),
            // The Games screen falls back to the stats that ride with the first page.
            Reply::Stats(Err(err)) => tracing::warn!(%err, "stats not read"),
            Reply::RecentTables(result) => {
                self.recent_tables = Some(result.map_err(|err| err.to_string()));
                self.note_open_table();
            }
            Reply::Server(phase) => self.on_server_phase(phase),
            Reply::Quit => tx.quit(),
        }
    }
}

fn on_conn_event(
    table: &mut TableScreen,
    bridge: Option<&Bridge>,
    event: ConnEvent,
    now: u64,
    tx: &mut Tx,
) {
    match event {
        ConnEvent::Status(up) => {
            if !up {
                table.reconnected = true;
            }
            let fx = table.store.set_connected(up, now);
            table.apply(fx, bridge);
        }
        ConnEvent::Event(TableEvent::Sync(sync)) => table.on_sync(*sync, bridge, tx),
        ConnEvent::Event(TableEvent::Chat(chat)) => {
            // A line redelivered after a reconnect is already in the log: it neither joins
            // it again nor pops the table-talk bubble a second time (review fix PP-4).
            if table
                .store
                .chat()
                .iter()
                .any(|line| line.msg_id == chat.msg_id)
            {
                return;
            }
            table.store.on_chat_event(&chat, now);
            table.rearm(bridge);
        }
        ConnEvent::Event(TableEvent::Thinking(thinking)) => {
            table.thinking[thinking.seat.index()] = thinking.on;
        }
        ConnEvent::Event(TableEvent::Presence(p)) => {
            table.present.hooks.offline[p.seat.index()] = !p.online;
        }
        ConnEvent::Event(TableEvent::ChatDelta(_)) => {}
        ConnEvent::Ended(err) => {
            tracing::warn!(%err, "table connection ended");
            let fx = table.store.set_connected(false, now);
            table.apply(fx, bridge);
            table.stage = TableStage::Failed(err);
        }
    }
}

/// `AppSpec::poll`: expires toasts on the frame clock, runs a scene's second-frame change,
/// and steps the table's presentation (its director, the sound and announcer queues and the
/// tour's auto-open).
pub fn poll(state: &mut AppState, t: f64, tx: &mut Tx) -> bool {
    state.clock = t;
    let mut changed = false;
    for toast in &mut state.toasts {
        if toast.until.is_none() {
            toast.until = Some(t + motion::TOAST_SECS);
        }
    }
    let before = state.toasts.len();
    state
        .toasts
        .retain(|toast| toast.until.is_none_or(|u| u > t));
    if state.toasts.len() != before {
        changed = true;
    }
    if let Some(next) = state
        .toasts
        .iter()
        .filter_map(|toast| toast.until)
        .reduce(f64::min)
    {
        tx.wake_after(Duration::from_secs_f64((next - t).max(0.0)));
    }
    changed |= run_then(state, tx);
    let reduced = state.motion_reduced.get();
    let audible = state.prefs.sound && state.visible && !rune::is_headless();
    let bridge = state.bridge.as_ref();
    if let Some(table) = state.table.as_mut() {
        let out = table.present.poll(t, reduced);
        changed |= out.changed;
        if let Some(turn_id) = out.continue_turn {
            table.send_continue(turn_id, bridge);
        }
        if let Some(due) = out.wake {
            tx.wake_at(due);
        }
        if out.frame {
            tx.wake_after(Duration::ZERO);
        }
        if out.unclipped {
            tx.unclipped_flights();
        }
        if out.instant {
            tx.instant();
        }
        // The queues drain whether or not they are heard, so nothing replays later.
        for cue in table.present.hooks.sound.drain(..) {
            if audible {
                state.sound.play(&cue);
            }
        }
        if let (size, true) = table.drawn_size.get() {
            table.drawn_size.set((size, false));
            table.layout_settle = LAYOUT_SETTLE;
        }
        if table.layout_settle > 0 && table.store.display_view().is_some() {
            table.layout_settle -= 1;
            changed = true;
            tx.wake_after(Duration::ZERO);
        }
        if let Some(view) = table.store.display_view() {
            if let Some(line) = announcer::take(&mut table.present.hooks, view) {
                table.present.announce_line(line);
                changed = true;
            }
            if table.present.hooks.tour_step.is_none() && tour::should_auto_open(view, &state.prefs)
            {
                table.present.hooks.tour_step = Some(0);
                changed = true;
            }
        }
    }
    changed
}

/// A scene's second-frame change ([`TableScreen::showing_then`]): the first poll arms it and
/// asks for the next frame, the second runs it. True when it ran.
fn run_then(state: &mut AppState, tx: &mut Tx) -> bool {
    let Some(table) = state.table.as_mut() else {
        return false;
    };
    match &mut table.then {
        None => return false,
        Some((_, armed)) if !*armed => {
            *armed = true;
            tx.wake_after(Duration::ZERO);
            return false;
        }
        Some(_) => {}
    }
    let Some((then, _)) = table.then.take() else {
        return false;
    };
    match then {
        Then::Sync(sync) => {
            let bridge = state.bridge.as_ref();
            if let Some(table) = state.table.as_mut() {
                table.on_sync(*sync, bridge, tx);
            }
        }
        Then::Do(act) => act(state),
    }
    true
}

/// `App::on_visibility`: the store polls only while the window shows. Coming back, whatever
/// was mid-flight jumps to its end (09 §2.3).
pub fn on_visibility(state: &mut AppState, visible: bool, _tx: &mut Tx) {
    if visible
        && !state.visible
        && let Some(table) = state.table.as_mut()
    {
        table.present.director.fast_forward(state.clock);
    }
    state.visible = visible;
}

/// `App::on_quit`: the app starts to quit (Cmd-Q, the Dock, logout, the last window closing).
/// The bridge starts closing its tables and stopping the server now, so the stop overlaps the
/// window's teardown; `lib.rs` sends `Shutdown` again after the loop (it is idempotent).
pub fn on_quit(state: &mut AppState, _tx: &mut Tx) {
    state.send(Intent::Shutdown);
}

/// `AppSpec::on_key`: Escape goes back (on the table it closes the open panel); the table
/// handles its own keys.
pub fn on_key(state: &mut AppState, e: &KeyEvent, tx: &mut Tx) -> bool {
    match &e.key {
        Key::Escape if state.route != Route::Table => state.back(tx),
        Key::Escape => state.close_table_panel(),
        _ if state.route == Route::Table => screens::table::on_key(state, e, tx),
        _ => false,
    }
}

/// Under this size (width, height) the web table does not fit; a holding panel shows instead.
pub const HOLD_MIN: (f32, f32) = (720.0, 520.0);

impl AppState {
    /// Settings and the holding panel: this browser plays on the classic site from now on.
    pub fn use_classic_table(&mut self) {
        web_history::use_classic_table();
    }
}

/// The root view: the screen, the nav over it and the toasts over both.
pub fn view(state: &AppState, cx: &mut Cx<AppState>) -> El {
    let reduced = state.prefs.reduce_motion || cx.system_reduced_motion();
    cx.reduced_motion(reduced);
    state.motion_reduced.set(reduced);
    cx.focus_ring(3.0, 4.0, PAL.focus);
    let width = cx
        .layout_size("root")
        .map_or(theme::window::SIZE.0, |[w, _]| w);
    let too_small = web_history::IS_WEB
        && state.route == Route::Table
        && cx
            .layout_size("root")
            .is_some_and(|[w, h]| w < HOLD_MIN.0 || h < HOLD_MIN.1);
    let screen = match &state.route {
        _ if too_small => screens::holding_panel(),
        Route::Home => screens::home::view(state, width, cx),
        Route::NewGame => screens::new_game::view(state, cx),
        Route::Table => screens::table::view(state, width, cx),
        Route::Games => screens::other::games(state, cx),
        Route::MatchDetail(id) => screens::other::match_detail(state, id, cx),
        Route::Settings => screens::other::settings(state, cx),
    };
    // Each screen is one opaque sheet in its own background, the pill nav included, so a route
    // change wipes the whole page in over the old one (09 §4.12): one nav, one background,
    // nothing printed twice.
    let mut sheet = col()
        .key(format!("screen-{}", route_key(&state.route)))
        .w_full()
        .h_full()
        .fill(screen_bg(&state.route))
        .paint_order((state.route_seq % 0x4000) as i16)
        .child(screen);
    if state.route.has_nav() && state.route != Route::Home {
        sheet = sheet.child(pill_nav(&state.route));
    }
    // The first screen (launch, a scene) is simply there; only a route change moves screens.
    let enter = state
        .routed_at
        .is_finite()
        .then(|| screens::route_transition(state.route_dir, reduced))
        .flatten();
    if let Some(t) = enter {
        sheet = sheet.transition(t);
    }
    // Under the sheets, the window keeps the old screen's background until the new one has
    // covered it, so the old page dissolves into its own colour, never into the new one's.
    let under = state.route_from.as_ref().unwrap_or(&state.route);
    let mut root = col()
        .key("root")
        .w_full()
        .h_full()
        .fill(screen_bg(under))
        .child(sheet);
    // The screen that leaves goes the way this change says, not the way it came in.
    if enter.is_some() {
        let left_table = state.route_from == Some(Route::Table);
        root = root.child_exit(screens::route_exit(left_table, reduced));
    }
    // The server notice lies over the top of the content, in no screen's sheet: it stays put
    // while the screens change under it.
    // It sits in the top bar's free stretch when that is wide enough (read only while it shows).
    let band = if state.server_down() && state.route.has_nav() {
        let (left, right) = if state.route == Route::Home {
            (cx.layout("home-brand"), cx.layout("home-nav"))
        } else {
            (None, cx.layout("nav"))
        };
        screens::server::notice_band(&state.route, width, left, right)
    } else {
        None
    };
    if let Some(notice) = screens::server::notice(state, width, band, reduced) {
        root = root.child(notice);
    }
    if !state.toasts.is_empty() {
        root = root.child(toasts(state));
    }
    root
}

fn route_key(route: &Route) -> &'static str {
    match route {
        Route::Home => "home",
        Route::NewGame => "new",
        Route::Table => "table",
        Route::Games => "games",
        Route::MatchDetail(_) => "match",
        Route::Settings => "settings",
    }
}

fn screen_bg(route: &Route) -> rune::Color {
    match route {
        Route::Home | Route::NewGame => PAL.cream,
        Route::Table => PAL.table_bg,
        Route::Games | Route::MatchDetail(_) | Route::Settings => PAL.dark_bg,
    }
}

/// The three nav links (06 §2.1): label, route, title.
pub const NAV_LINKS: [(&str, NavTarget, &str); 3] = [
    ("Play", NavTarget::NewGame, "Deal a new hand"),
    ("Your games", NavTarget::Games, "Your past games"),
    ("Settings", NavTarget::Settings, "Edit the opponents"),
];

/// Where a nav link goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavTarget {
    /// New game.
    NewGame,
    /// History.
    Games,
    /// Settings.
    Settings,
}

impl NavTarget {
    /// The route.
    pub fn route(self) -> Route {
        match self {
            NavTarget::NewGame => Route::NewGame,
            NavTarget::Games => Route::Games,
            NavTarget::Settings => Route::Settings,
        }
    }

    /// `aria-current`: the link's own page, or (except Play) any page under it.
    pub fn is_current(self, here: &Route) -> bool {
        match self {
            NavTarget::NewGame => *here == Route::NewGame,
            NavTarget::Games => matches!(here, Route::Games | Route::MatchDetail(_)),
            NavTarget::Settings => *here == Route::Settings,
        }
    }

    fn key(self) -> &'static str {
        match self {
            NavTarget::NewGame => "nav-play",
            NavTarget::Games => "nav-games",
            NavTarget::Settings => "nav-settings",
        }
    }
}

/// One nav link for the home header (transparent, ink) or the pill (cream on forest).
pub fn nav_link(
    label: &'static str,
    target: NavTarget,
    title: &'static str,
    here: &Route,
    on_home: bool,
) -> El {
    let current = target.is_current(here);
    let (fg, hover, bg) = match (on_home, current) {
        (true, _) => (PAL.home_link, PAL.home_link_hover, None),
        (false, true) => (PAL.forest, PAL.lime, Some(PAL.lime)),
        (false, false) => (PAL.nav_link, PAL.nav_hover, None),
    };
    let mut link = row()
        .key(if on_home {
            format!("home-{}", target.key())
        } else {
            target.key().to_owned()
        })
        .min_w(space::TAP)
        .h(space::TAP)
        .px(space::NAV_LINK_PX)
        .center()
        .radius(if on_home {
            radius::HOME_LINK
        } else {
            radius::PILL
        })
        .hover(hover)
        .cursor(rune::Cursor::Pointer)
        .focusable(true)
        .role("link")
        .label(label)
        .description(title)
        .href(url_route::path_for(&target.route(), None).unwrap_or_default())
        .on_press_tx(move |s: &mut AppState, tx| s.nav_to(target.route(), tx))
        .child(text(label).font(ty::NAV).color(fg));
    if let Some(bg) = bg {
        link = link.fill(bg);
    }
    link
}

/// The fixed pill top-right on every screen but Home and the table (06 §2.1).
fn pill_nav(here: &Route) -> El {
    let club = row()
        .key("nav-home")
        .size(space::TAP, space::TAP)
        .center()
        .radius(radius::PILL)
        .hover(PAL.nav_hover)
        .cursor(rune::Cursor::Pointer)
        .focusable(true)
        .role("link")
        .label("Euchre home")
        .href("/")
        .on_press_tx(|s: &mut AppState, tx| s.nav_to(Route::Home, tx))
        .child(text("♣").font(ty::NAV_CLUB).color(PAL.nav_link));
    let mut pill = row()
        .key("nav")
        .role("navigation")
        .label("Main navigation")
        .padding(space::NAV_PAD)
        .gap(space::NAV_GAP)
        .center_y()
        .radius(radius::PILL)
        .fill(PAL.nav_bg)
        .border(1.0, PAL.nav_border)
        .child(club);
    for (label, target, title) in NAV_LINKS {
        pill = pill.child(nav_link(label, target, title, here, false));
    }
    // Part of its screen's sheet: between two pill screens the new pill wipes in exactly over
    // the old one, so only the current link changes.
    el().key("nav-layer")
        .pin(None, Some(space::NAV_INSET), Some(space::NAV_INSET), None)
        .paint_order(1)
        .child(pill)
}

fn toasts(state: &AppState) -> El {
    let mut stack = col()
        .key("toasts")
        .gap(space::S)
        .items_end()
        .live_region(rune::LiveRegion::Polite);
    for toast in &state.toasts {
        let id = toast.id;
        let card = row()
            .key(format!("toast-{id}"))
            .max_w(space::TOAST_MAX_W)
            .gap(space::M)
            .center_y()
            .px(space::M)
            .py(space::SM)
            .radius(radius::BUTTON)
            .fill(PAL.notice_bg)
            .border(1.0, PAL.notice_border)
            .role("status")
            .child(
                text(toast.title.clone())
                    .font(ty::TOAST)
                    .color(PAL.ink)
                    .wrap()
                    .shrink(),
            )
            .child(
                row()
                    .key(format!("toast-close-{id}"))
                    .size(space::TAP, space::TAP)
                    .center()
                    .radius(radius::BUTTON)
                    .hover(PAL.panel_hover)
                    .cursor(rune::Cursor::Pointer)
                    .focusable(true)
                    .role("button")
                    .label("Dismiss")
                    .on_press(move |s: &mut AppState| s.dismiss_toast(id))
                    .child(text("×").font(ty::TOAST).color(PAL.ink)),
            );
        stack = stack.child(theme::shadows(card, &theme::shadow::TOAST));
    }
    el().key("toast-layer")
        .pin(
            None,
            None,
            Some(space::TOAST_INSET),
            Some(space::TOAST_INSET),
        )
        .raise()
        .child(stack)
}
