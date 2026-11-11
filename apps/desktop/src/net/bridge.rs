//! The bridge between the UI thread and the network (07 §3).
//!
//! The UI never waits on IO. Handlers send an [`Intent`] over an unbounded channel (a send never
//! blocks); a tokio runtime on its own thread runs the matching [`Backend`] call and hands the
//! answer back as a [`Reply`] through a [`UiSink`], which in the app is `AppHandle::update`.
//! The table store stays on the UI thread: the bridge only runs its [`StoreCommand`]s and arms
//! the one timer its `next_deadline_ms` asks for.
//!
//! Every table connection carries a `conn_id` chosen by the UI. Replies name it, so a reply for a
//! table the player already left is dropped by the receiver.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use euchre_profile_model::{ClientMatchReplay, ProfileSettings, ProfileStats, SettingsPatch};
use euchre_protocol::store::{ACTION_TIMEOUT_MS, RequestId, SNAPSHOT_TIMEOUT_MS};
use euchre_protocol::{
    CompletedHandReview, HandReviewOutcome, MatchListPage, PublicGameView, ReviewTrickAck,
    StoreCommand, SubmitOutcome,
};
use tokio::sync::mpsc;

// Tasks, sleeps and timeouts go through the runtime seam: tokio in the window, the page's
// event loop in the browser.
use super::rt::{self, Task as JoinHandle};

use super::{
    Backend, ConnEvent, Identity, NetError, RecentTables, ServerHost, ServerPhase, TableCalls,
    TableConfig, now_ms,
};
use crate::prefs::{Prefs, PrefsStore};

/// Extra time the bridge allows past the store's own deadline before it gives a call up. The
/// store has already treated the call as unanswered by then; this only frees the task.
const GRACE_MS: u64 = 1000;

/// What the UI asks the network to do.
#[derive(Clone, Debug)]
pub enum Intent {
    /// Enroll if needed and report who is playing.
    Identify,
    /// Create a table with these rules, then open it as `conn_id`.
    CreateTable {
        /// The connection the UI reserved for it.
        conn_id: u64,
        /// The New game choices.
        cfg: TableConfig,
    },
    /// Open an existing table as `conn_id`.
    OpenTable {
        /// The connection the UI reserved for it.
        conn_id: u64,
        /// The table.
        game_id: String,
    },
    /// Close a connection and cancel its timer.
    CloseTable {
        /// The connection.
        conn_id: u64,
    },
    /// Run one table-store command on a connection.
    Store {
        /// The connection.
        conn_id: u64,
        /// The command.
        command: StoreCommand,
    },
    /// Replace the connection's timer: fire [`Reply::Timer`] at `at_ms` on the [`now_ms`]
    /// clock, or cancel it with `None`.
    ArmTimer {
        /// The connection.
        conn_id: u64,
        /// When.
        at_ms: Option<u64>,
    },
    /// One page of the match history.
    ListGames {
        /// The keyset cursor from the previous page.
        cursor: Option<String>,
    },
    /// One match's detail.
    MatchDetail {
        /// The match.
        match_id: String,
    },
    /// The lifetime stats; they come back as [`Reply::Stats`].
    GetStats,
    /// The profile settings.
    GetSettings,
    /// Merge a patch into the profile settings.
    PatchSettings(SettingsPatch),
    /// Write the local preferences file.
    SavePrefs(Prefs),
    /// The saved tables this device opened; they come back as [`Reply::RecentTables`].
    RecentTables,
    /// Stop the local server (if any) and boot it again: the Restart and Try again actions.
    /// Nothing happens without one ([`Backend::host`]).
    RestartServer,
    /// The app is quitting: close every table, write any pending preferences, stop the local
    /// server (bounded) and end the loop.
    Shutdown,
}

/// What the network hands back to the UI.
#[derive(Debug)]
pub enum Reply {
    /// [`Intent::Identify`] finished.
    Identity(Result<Identity, NetError>),
    /// [`Intent::CreateTable`]'s table exists (or could not be made).
    TableCreated {
        /// The connection.
        conn_id: u64,
        /// The game id.
        result: Result<String, NetError>,
    },
    /// The connection is open (its events follow) or failed to open.
    TableOpened {
        /// The connection.
        conn_id: u64,
        /// The game id.
        result: Result<String, NetError>,
    },
    /// One item from the connection's stream, in order.
    Conn {
        /// The connection.
        conn_id: u64,
        /// The item.
        event: ConnEvent,
    },
    /// A `FetchSnapshot` finished; `None` for no reply.
    Snapshot {
        /// The connection.
        conn_id: u64,
        /// The store request.
        id: RequestId,
        /// The view.
        view: Option<PublicGameView>,
    },
    /// A `SubmitMove` finished.
    Submit {
        /// The connection.
        conn_id: u64,
        /// The store request.
        id: RequestId,
        /// The outcome; every failure is [`SubmitOutcome::NoAck`].
        outcome: SubmitOutcome,
    },
    /// A `ReviewTrick` finished; `None` for no reply.
    Review {
        /// The connection.
        conn_id: u64,
        /// The store request.
        id: RequestId,
        /// The ack.
        ack: Option<ReviewTrickAck>,
    },
    /// A `GetHandReview` finished; every failure is `Loaded(None)`.
    HandReview {
        /// The connection.
        conn_id: u64,
        /// The store request.
        id: RequestId,
        /// The outcome.
        outcome: HandReviewOutcome,
    },
    /// The connection's timer fired: call `TableStore::tick`.
    Timer {
        /// The connection.
        conn_id: u64,
    },
    /// [`Intent::ListGames`] finished.
    Games {
        /// The cursor it asked for (`None`: the first page), so a late reply for a page the
        /// player has left is dropped (review fix PP-5).
        cursor: Option<String>,
        /// The page.
        result: Result<MatchListPage, NetError>,
    },
    /// [`Intent::MatchDetail`] finished.
    MatchDetail {
        /// The match.
        match_id: String,
        /// Its detail.
        result: Result<ClientMatchReplay, NetError>,
    },
    /// [`Intent::GetStats`] finished.
    Stats(Result<ProfileStats, NetError>),
    /// [`Intent::GetSettings`] or [`Intent::PatchSettings`] finished.
    Settings(Result<SettingsReply, NetError>),
    /// [`Intent::SavePrefs`] finished; the error is for a toast.
    PrefsSaved(Result<(), String>),
    /// [`Intent::RecentTables`] finished.
    RecentTables(Result<RecentTables, NetError>),
    /// The local server changed phase (only backends with a [`Backend::host`]). The first one
    /// arrives as the bridge starts.
    Server(ServerPhase),
    /// A signal (SIGINT or SIGTERM) asked the app to quit; the app answers with `tx.quit()`, the
    /// same path as Cmd-Q, so the server stops cleanly (contract §N2.5). Lane N2 posts it.
    Quit,
}

/// The profile settings the server holds, and whether they come from a save.
#[derive(Clone, Debug)]
pub struct SettingsReply {
    /// The settings, as saved (boxed: they are the largest reply by far).
    pub settings: Box<ProfileSettings>,
    /// True for an answer to [`Intent::PatchSettings`]: the screen says they were saved.
    pub saved: bool,
}

/// Where replies go: `AppHandle::update` in the app, a channel in tests.
#[derive(Clone)]
pub struct UiSink(Arc<dyn Fn(Reply) + Send + Sync>);

impl std::fmt::Debug for UiSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UiSink")
    }
}

impl UiSink {
    /// A sink that calls `post` with every reply.
    pub fn new(post: impl Fn(Reply) + Send + Sync + 'static) -> UiSink {
        UiSink(Arc::new(post))
    }

    /// A sink that queues replies on a channel; tests drain it into the runtime.
    pub fn channel() -> (UiSink, std::sync::mpsc::Receiver<Reply>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = parking_lot::Mutex::new(tx);
        (
            UiSink::new(move |reply| {
                let _ = tx.lock().send(reply);
            }),
            rx,
        )
    }

    fn post(&self, reply: Reply) {
        (self.0)(reply);
    }
}

/// The UI's end of the bridge. Cheap to clone; sending never blocks.
#[derive(Clone, Debug)]
pub struct Bridge {
    tx: mpsc::UnboundedSender<Intent>,
}

impl Bridge {
    /// A bridge and the receiver its loop drains. The intents queue until [`run`] starts.
    pub fn channel() -> (Bridge, mpsc::UnboundedReceiver<Intent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Bridge { tx }, rx)
    }

    /// A bridge with no loop behind it (scenes and fixtures): every intent is dropped.
    pub fn detached() -> Bridge {
        let (tx, _) = mpsc::unbounded_channel();
        Bridge { tx }
    }

    /// Queues an intent. A closed bridge (app shutting down) drops it.
    pub fn send(&self, intent: Intent) {
        if self.tx.send(intent).is_err() {
            tracing::debug!("bridge closed; intent dropped");
        }
    }
}

/// Everything the loop needs.
pub struct BridgeConfig {
    /// The backend.
    pub backend: Arc<dyn Backend>,
    /// Where replies go.
    pub sink: UiSink,
    /// Where preferences are written; `None` keeps them in memory.
    pub prefs: Option<PrefsStore>,
    /// Whether SIGINT, SIGTERM and SIGHUP quit the app (with a local server). The app sets
    /// it; tests leave it off, so a signal to the test process is never swallowed and a
    /// killed test run stops at once (review fix LDS-5).
    pub quit_on_signals: bool,
}

/// The bridge thread: a tokio runtime that runs [`run`] until every [`Bridge`] is dropped.
/// In the browser it is a task on the page's event loop instead (no threads there).
pub struct BridgeThread {
    #[cfg(not(target_arch = "wasm32"))]
    join: Option<std::thread::JoinHandle<()>>,
}

/// The browser has no threads and no tokio runtime. The loop runs as a task on the page's
/// event loop, beside Rune's frames; nothing there blocks, so nothing waits on it.
#[cfg(target_arch = "wasm32")]
impl BridgeThread {
    /// Starts the loop on the page's event loop.
    pub fn spawn(
        config: BridgeConfig,
        intents: mpsc::UnboundedReceiver<Intent>,
    ) -> std::io::Result<BridgeThread> {
        drop(rt::spawn(run(config, intents)));
        Ok(BridgeThread {})
    }

    /// Nothing to wait for in the browser.
    pub fn join(self) {}

    /// Nothing to wait for in the browser.
    pub fn join_within(self, _limit: Duration) -> bool {
        true
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl BridgeThread {
    /// Starts the runtime on its own thread.
    pub fn spawn(
        config: BridgeConfig,
        intents: mpsc::UnboundedReceiver<Intent>,
    ) -> std::io::Result<BridgeThread> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("euchre-net")
            .enable_all()
            .build()?;
        let join = std::thread::Builder::new()
            .name("euchre-bridge".to_owned())
            .spawn(move || {
                runtime.block_on(run(config, intents));
                runtime.shutdown_timeout(Duration::from_millis(500));
            })?;
        Ok(BridgeThread { join: Some(join) })
    }

    /// Waits for the loop to end (after the last [`Bridge`] dropped).
    pub fn join(mut self) {
        if let Some(join) = self.join.take()
            && join.join().is_err()
        {
            tracing::error!("bridge thread panicked");
        }
    }

    /// Waits at most `limit` for the loop to end (after [`Intent::Shutdown`]); a thread still
    /// busy past it is left to the process exit. Returns whether it ended in time.
    pub fn join_within(mut self, limit: Duration) -> bool {
        let Some(join) = self.join.take() else {
            return true;
        };
        let deadline = std::time::Instant::now() + limit;
        while !join.is_finished() {
            if std::time::Instant::now() >= deadline {
                tracing::warn!("bridge thread still busy at quit");
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if join.join().is_err() {
            tracing::error!("bridge thread panicked");
        }
        true
    }
}

/// The open connections' calls, shared with the tasks that run commands. Each open gets an
/// epoch; a pump registers its calls only while its epoch is still the live one, so a
/// [`Intent::CloseTable`] that lands while the table is still opening wins the race.
#[derive(Default)]
struct Calls {
    live: HashMap<u64, u64>,
    open: HashMap<u64, Arc<dyn TableCalls>>,
    next_epoch: u64,
}

impl Calls {
    /// Starts an open of `conn_id` and returns its epoch.
    fn begin(&mut self, conn_id: u64) -> u64 {
        self.next_epoch += 1;
        self.live.insert(conn_id, self.next_epoch);
        self.next_epoch
    }

    /// Registers an opened table's calls; false when the connection was closed meanwhile.
    fn register(&mut self, conn_id: u64, epoch: u64, calls: Arc<dyn TableCalls>) -> bool {
        if self.live.get(&conn_id) != Some(&epoch) {
            return false;
        }
        self.open.insert(conn_id, calls);
        true
    }

    /// Ends `conn_id`: no later register succeeds, and its calls (if any) come back to close.
    fn close(&mut self, conn_id: u64) -> Option<Arc<dyn TableCalls>> {
        self.live.remove(&conn_id);
        self.open.remove(&conn_id)
    }
}

/// Writes preferences one at a time, always the newest: a burst of toggles never races two
/// writers onto the same file, and the last choice is the one that lands.
async fn prefs_writer(
    store: Option<PrefsStore>,
    sink: UiSink,
    mut rx: mpsc::UnboundedReceiver<Prefs>,
) {
    while let Some(mut next) = rx.recv().await {
        while let Ok(newer) = rx.try_recv() {
            next = newer;
        }
        let store = store.clone();
        let save = move || match store {
            Some(store) => store.save(&next).map_err(|e| e.to_string()),
            None => Ok(()),
        };
        // A localStorage write is synchronous and cheap; the browser has no blocking pool.
        #[cfg(target_arch = "wasm32")]
        let result = save();
        #[cfg(not(target_arch = "wasm32"))]
        let result = tokio::task::spawn_blocking(save)
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        sink.post(Reply::PrefsSaved(result));
    }
}

/// Runs intents until the channel closes. Each call is its own task, so a slow call never
/// holds up the next intent; replies for one connection keep the order its stream produced.
pub async fn run(config: BridgeConfig, mut intents: mpsc::UnboundedReceiver<Intent>) {
    let BridgeConfig {
        backend,
        sink,
        prefs,
        quit_on_signals: watch_signals,
    } = config;
    // Signals are unix only; elsewhere the window's close is the only quit path.
    #[cfg(not(unix))]
    let _ = watch_signals;
    let calls: Arc<parking_lot::Mutex<Calls>> = Arc::default();
    let mut timers: HashMap<u64, JoinHandle> = HashMap::new();
    let mut pumps: HashMap<u64, JoinHandle> = HashMap::new();
    let (prefs_tx, prefs_rx) = mpsc::unbounded_channel();
    let writer = rt::spawn(prefs_writer(prefs, sink.clone(), prefs_rx));
    let host = backend.host();
    let phases = host
        .as_ref()
        .map(|host| start_host(Arc::clone(host), sink.clone()));
    // With a local server, a quit signal takes the Cmd-Q path, so the engine stops with the app.
    #[cfg(unix)]
    let signals = host
        .as_ref()
        .filter(|_| watch_signals)
        .map(|_| rt::spawn(quit_on_signals(sink.clone())));
    while let Some(intent) = intents.recv().await {
        match intent {
            Intent::Shutdown => break,
            Intent::Identify => {
                let (backend, sink) = (Arc::clone(&backend), sink.clone());
                rt::spawn(async move {
                    let identity = backend.identity().await;
                    // A first-time browser has no guest cookie until its first table: not an
                    // outage, so say nothing (the first create enrolls it).
                    if cfg!(target_arch = "wasm32") && identity == Err(NetError::Unauthorized) {
                        return;
                    }
                    sink.post(Reply::Identity(identity));
                });
            }
            Intent::CreateTable { conn_id, cfg } => {
                let (backend, sink, calls) =
                    (Arc::clone(&backend), sink.clone(), Arc::clone(&calls));
                let epoch = calls.lock().begin(conn_id);
                let pump = rt::spawn(async move {
                    let created = backend.create_table(cfg).await;
                    let game_id = created.clone();
                    sink.post(Reply::TableCreated {
                        conn_id,
                        result: created,
                    });
                    if let Ok(game_id) = game_id {
                        open_and_pump(backend, sink, calls, (conn_id, epoch), game_id).await;
                    }
                });
                replace(&mut pumps, conn_id, pump);
            }
            Intent::OpenTable { conn_id, game_id } => {
                let (backend, sink, calls) =
                    (Arc::clone(&backend), sink.clone(), Arc::clone(&calls));
                let epoch = calls.lock().begin(conn_id);
                let pump = rt::spawn(open_and_pump(
                    backend,
                    sink,
                    calls,
                    (conn_id, epoch),
                    game_id,
                ));
                replace(&mut pumps, conn_id, pump);
            }
            Intent::CloseTable { conn_id } => {
                if let Some(t) = timers.remove(&conn_id) {
                    t.abort();
                }
                if let Some(p) = pumps.remove(&conn_id) {
                    p.abort();
                }
                let closing = calls.lock().close(conn_id);
                if let Some(c) = closing {
                    rt::spawn(async move { c.close().await });
                }
            }
            Intent::Store { conn_id, command } => {
                let table = calls.lock().open.get(&conn_id).cloned();
                let sink = sink.clone();
                rt::spawn(run_command(table, sink, conn_id, command));
            }
            Intent::ArmTimer { conn_id, at_ms } => {
                if let Some(t) = timers.remove(&conn_id) {
                    t.abort();
                }
                if let Some(at) = at_ms {
                    let sink = sink.clone();
                    let timer = rt::spawn(async move {
                        let wait = at.saturating_sub(now_ms());
                        rt::sleep(Duration::from_millis(wait)).await;
                        sink.post(Reply::Timer { conn_id });
                    });
                    timers.insert(conn_id, timer);
                }
            }
            Intent::ListGames { cursor } => {
                let (backend, sink) = (Arc::clone(&backend), sink.clone());
                rt::spawn(async move {
                    let result = backend.list_games(cursor.clone()).await;
                    sink.post(Reply::Games { cursor, result });
                });
            }
            Intent::GetStats => {
                let (backend, sink) = (Arc::clone(&backend), sink.clone());
                rt::spawn(async move { sink.post(Reply::Stats(backend.stats().await)) });
            }
            Intent::MatchDetail { match_id } => {
                let (backend, sink) = (Arc::clone(&backend), sink.clone());
                rt::spawn(async move {
                    let result = backend.match_detail(&match_id).await;
                    sink.post(Reply::MatchDetail { match_id, result });
                });
            }
            Intent::GetSettings => {
                let (backend, sink) = (Arc::clone(&backend), sink.clone());
                rt::spawn(async move {
                    let result = backend.get_settings().await;
                    sink.post(Reply::Settings(result.map(|settings| SettingsReply {
                        settings: Box::new(settings),
                        saved: false,
                    })));
                });
            }
            Intent::PatchSettings(patch) => {
                let (backend, sink) = (Arc::clone(&backend), sink.clone());
                rt::spawn(async move {
                    let result = backend.patch_settings(patch).await;
                    sink.post(Reply::Settings(result.map(|settings| SettingsReply {
                        settings: Box::new(settings),
                        saved: true,
                    })));
                });
            }
            Intent::SavePrefs(next) => {
                let _ = prefs_tx.send(next);
            }
            Intent::RecentTables => {
                let (backend, sink) = (Arc::clone(&backend), sink.clone());
                rt::spawn(async move {
                    sink.post(Reply::RecentTables(backend.recent_tables().await));
                });
            }
            Intent::RestartServer => {
                if let Some(host) = host.clone() {
                    rt::spawn(async move { host.restart().await });
                }
            }
        }
    }
    for (_, t) in timers {
        t.abort();
    }
    for (_, p) in pumps {
        p.abort();
    }
    let open: Vec<_> = {
        let mut calls = calls.lock();
        calls.live.clear();
        calls.open.drain().map(|(_, c)| c).collect()
    };
    for c in open {
        c.close().await;
    }
    // The last preferences still land: the writer drains its queue and ends.
    drop(prefs_tx);
    writer.join().await;
    // The local server goes last, once no table talks to it; its own budget bounds it.
    if let Some(host) = host {
        host.shutdown().await;
    }
    if let Some(phases) = phases {
        phases.abort();
    }
    #[cfg(unix)]
    if let Some(signals) = signals {
        signals.abort();
    }
}

/// SIGINT, SIGTERM or SIGHUP (a closed terminal, review fix LDS-11): the first posts
/// [`Reply::Quit`] (the app answers `tx.quit()`, the Cmd-Q path, so the server stops
/// cleanly); a second exits at once with 130 (the engine's watchdog stops the engine).
#[cfg(unix)]
async fn quit_on_signals(sink: UiSink) {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut interrupt), Ok(mut terminate), Ok(mut hangup)) = (
        signal(SignalKind::interrupt()),
        signal(SignalKind::terminate()),
        signal(SignalKind::hangup()),
    ) else {
        tracing::warn!("quit signals cannot be watched");
        return;
    };
    tokio::select! {
        _ = interrupt.recv() => {}
        _ = terminate.recv() => {}
        _ = hangup.recv() => {}
    }
    tracing::info!("quit signal: stopping the game server");
    sink.post(Reply::Quit);
    tokio::select! {
        _ = interrupt.recv() => {}
        _ = terminate.recv() => {}
        _ = hangup.recv() => {}
    }
    tracing::warn!("second quit signal: exiting now");
    std::process::exit(130);
}

/// Boots the local server and forwards every phase change to the UI, starting with the
/// current one. Returns the forwarder, which the loop stops at quit.
fn start_host(host: Arc<dyn ServerHost>, sink: UiSink) -> JoinHandle {
    let mut phases = host.phases();
    let forwarder = rt::spawn(async move {
        loop {
            let phase = phases.borrow_and_update().clone();
            sink.post(Reply::Server(phase));
            if phases.changed().await.is_err() {
                break;
            }
        }
    });
    rt::spawn(async move { host.boot().await });
    forwarder
}

fn replace(tasks: &mut HashMap<u64, JoinHandle>, conn_id: u64, task: JoinHandle) {
    if let Some(old) = tasks.insert(conn_id, task) {
        old.abort();
    }
}

/// Opens the table, registers its calls, then forwards its stream until it ends.
async fn open_and_pump(
    backend: Arc<dyn Backend>,
    sink: UiSink,
    calls: Arc<parking_lot::Mutex<Calls>>,
    (conn_id, epoch): (u64, u64),
    game_id: String,
) {
    match backend.open_table(&game_id).await {
        Ok(mut conn) => {
            let registered = calls
                .lock()
                .register(conn_id, epoch, Arc::clone(&conn.calls));
            if !registered {
                // Closed while it was opening: hang up and say nothing.
                conn.calls.close().await;
                return;
            }
            sink.post(Reply::TableOpened {
                conn_id,
                result: Ok(conn.game_id.clone()),
            });
            while let Some(event) = conn.events.recv().await {
                sink.post(Reply::Conn { conn_id, event });
            }
            sink.post(Reply::Conn {
                conn_id,
                event: ConnEvent::Status(false),
            });
        }
        Err(err) => sink.post(Reply::TableOpened {
            conn_id,
            result: Err(err),
        }),
    }
}

/// Runs one store command and reports it; a missing connection or any failure is "no reply".
async fn run_command(
    table: Option<Arc<dyn TableCalls>>,
    sink: UiSink,
    conn_id: u64,
    command: StoreCommand,
) {
    let bound = |ms: u64| Duration::from_millis(ms + GRACE_MS);
    match command {
        StoreCommand::FetchSnapshot { id } => {
            let view = match table {
                Some(t) => rt::timeout(bound(SNAPSHOT_TIMEOUT_MS), t.snapshot())
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .flatten(),
                None => None,
            };
            sink.post(Reply::Snapshot { conn_id, id, view });
        }
        StoreCommand::SubmitMove { id, request } => {
            let outcome = match table {
                Some(t) => {
                    match rt::timeout(bound(ACTION_TIMEOUT_MS), t.submit_move(request)).await {
                        Ok(Ok(ack)) => SubmitOutcome::Ack(ack),
                        _ => SubmitOutcome::NoAck,
                    }
                }
                None => SubmitOutcome::NoAck,
            };
            sink.post(Reply::Submit {
                conn_id,
                id,
                outcome,
            });
        }
        StoreCommand::ReviewTrick { id, request } => {
            let ack = match table {
                Some(t) => rt::timeout(bound(ACTION_TIMEOUT_MS), t.review_trick(request))
                    .await
                    .ok()
                    .and_then(Result::ok),
                None => None,
            };
            sink.post(Reply::Review { conn_id, id, ack });
        }
        StoreCommand::GetHandReview { id, hand_no } => {
            let review: Option<CompletedHandReview> = match table {
                Some(t) => rt::timeout(bound(ACTION_TIMEOUT_MS), t.get_hand_review(hand_no))
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .flatten(),
                None => None,
            };
            sink.post(Reply::HandReview {
                conn_id,
                id,
                outcome: HandReviewOutcome::Loaded(review),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::remote::RemoteBackend;
    use crate::net::sim::{SimBackend, SimOptions, Tempo};
    use euchre_protocol::{SubmitMoveRequest, TableEvent};

    fn start(
        backend: Arc<dyn Backend>,
    ) -> (Bridge, std::sync::mpsc::Receiver<Reply>, BridgeThread) {
        let (sink, replies) = UiSink::channel();
        let (bridge, intents) = Bridge::channel();
        let thread = BridgeThread::spawn(
            BridgeConfig {
                backend,
                sink,
                prefs: None,
                quit_on_signals: false,
            },
            intents,
        )
        .unwrap();
        (bridge, replies, thread)
    }

    fn next(replies: &std::sync::mpsc::Receiver<Reply>) -> Reply {
        replies.recv_timeout(Duration::from_secs(10)).unwrap()
    }

    #[test]
    fn create_opens_streams_and_submits() {
        let backend = Arc::new(SimBackend::new(SimOptions {
            tempo: Tempo::INSTANT,
            ..SimOptions::default()
        }));
        let (bridge, replies, thread) = start(backend);
        bridge.send(Intent::CreateTable {
            conn_id: 7,
            cfg: TableConfig::default(),
        });
        let Reply::TableCreated {
            conn_id: 7,
            result: Ok(game_id),
        } = next(&replies)
        else {
            panic!("created")
        };
        assert!(
            matches!(next(&replies), Reply::TableOpened { conn_id: 7, result: Ok(ref g) } if *g == game_id)
        );
        assert!(matches!(
            next(&replies),
            Reply::Conn {
                event: ConnEvent::Status(true),
                ..
            }
        ));
        // Find a view where it is the human's turn.
        let view = loop {
            if let Reply::Conn {
                event: ConnEvent::Event(TableEvent::Sync(s)),
                ..
            } = next(&replies)
                && s.view.turn_seat == Some(crate::net::sim::HUMAN)
            {
                break s.view;
            }
        };
        bridge.send(Intent::Store {
            conn_id: 7,
            command: StoreCommand::SubmitMove {
                id: RequestId(1),
                request: SubmitMoveRequest {
                    move_id: view.legal[0].id.clone(),
                    turn_id: Some(view.turn_id.clone()),
                    client_move_id: "b-1".into(),
                },
            },
        });
        let ack = loop {
            if let Reply::Submit { id, outcome, .. } = next(&replies) {
                break (id, outcome);
            }
        };
        assert_eq!(ack.0, RequestId(1));
        assert!(matches!(ack.1, SubmitOutcome::Ack(ref a) if a.is_ok()));
        bridge.send(Intent::ArmTimer {
            conn_id: 7,
            at_ms: Some(now_ms()),
        });
        while !matches!(next(&replies), Reply::Timer { conn_id: 7 }) {}
        bridge.send(Intent::CloseTable { conn_id: 7 });
        drop(bridge);
        thread.join();
    }

    #[test]
    fn your_games_pages_and_reads_the_lifetime_stats() {
        let backend = Arc::new(SimBackend::new(SimOptions::default()));
        let (bridge, replies, thread) = start(backend);
        bridge.send(Intent::ListGames { cursor: None });
        let Reply::Games {
            cursor: None,
            result: Ok(page),
        } = next(&replies)
        else {
            panic!("a page")
        };
        assert_eq!(page.rows.len(), crate::net::sim::GAMES_PAGE);
        bridge.send(Intent::ListGames {
            cursor: page.cursor.clone(),
        });
        let Reply::Games {
            cursor: Some(_),
            result: Ok(older),
        } = next(&replies)
        else {
            panic!("page two")
        };
        assert_eq!(
            older.rows.len(),
            crate::net::sim::SIM_HISTORY - crate::net::sim::GAMES_PAGE
        );
        bridge.send(Intent::GetStats);
        let Reply::Stats(Ok(read)) = next(&replies) else {
            panic!("stats")
        };
        assert_eq!(read.matches, crate::net::sim::SIM_HISTORY as i64);
        bridge.send(Intent::GetSettings);
        let Reply::Settings(Ok(got)) = next(&replies) else {
            panic!("settings")
        };
        assert!(!got.saved);
        bridge.send(Intent::PatchSettings(SettingsPatch {
            house_prompt: Some("Quiet table.".into()),
            ..SettingsPatch::default()
        }));
        let Reply::Settings(Ok(saved)) = next(&replies) else {
            panic!("saved settings")
        };
        assert!(saved.saved);
        assert_eq!(saved.settings.house_prompt, "Quiet table.");
        drop(bridge);
        thread.join();
    }

    #[test]
    fn unknown_connection_is_no_reply_and_remote_is_not_wired() {
        let (bridge, replies, thread) = start(Arc::new(RemoteBackend::default()));
        bridge.send(Intent::Store {
            conn_id: 1,
            command: StoreCommand::FetchSnapshot { id: RequestId(3) },
        });
        assert!(matches!(
            next(&replies),
            Reply::Snapshot {
                id: RequestId(3),
                view: None,
                ..
            }
        ));
        bridge.send(Intent::CreateTable {
            conn_id: 2,
            cfg: TableConfig::default(),
        });
        assert!(matches!(
            next(&replies),
            Reply::TableCreated {
                result: Err(NetError::NotWired),
                ..
            }
        ));
        drop(bridge);
        thread.join();
    }

    struct NoCalls;

    #[async_trait::async_trait]
    impl TableCalls for NoCalls {
        async fn submit_move(
            &self,
            _: SubmitMoveRequest,
        ) -> Result<euchre_protocol::MoveAck, NetError> {
            Err(NetError::Closed)
        }
        async fn review_trick(
            &self,
            _: euchre_protocol::ReviewTrickRequest,
        ) -> Result<ReviewTrickAck, NetError> {
            Err(NetError::Closed)
        }
        async fn snapshot(&self) -> Result<Option<PublicGameView>, NetError> {
            Ok(None)
        }
        async fn get_hand_review(&self, _: u32) -> Result<Option<CompletedHandReview>, NetError> {
            Ok(None)
        }
        async fn close(&self) {}
    }

    #[test]
    fn a_close_during_the_open_wins() {
        let mut calls = Calls::default();
        let epoch = calls.begin(4);
        assert!(calls.close(4).is_none(), "nothing registered yet");
        assert!(
            !calls.register(4, epoch, Arc::new(NoCalls)),
            "the late open is refused"
        );
        assert!(calls.open.is_empty());
        let again = calls.begin(4);
        assert!(
            !calls.register(4, epoch, Arc::new(NoCalls)),
            "an old epoch stays refused"
        );
        assert!(calls.register(4, again, Arc::new(NoCalls)));
        assert!(calls.close(4).is_some());
    }

    #[test]
    fn shutdown_ends_the_loop_while_a_bridge_is_alive() {
        let (bridge, _replies, thread) = start(Arc::new(RemoteBackend::default()));
        bridge.send(Intent::SavePrefs(Prefs::default()));
        bridge.send(Intent::Shutdown);
        assert!(thread.join_within(Duration::from_secs(5)));
        drop(bridge);
    }
}
