//! One table connection (02 §0–§6; contract §N1.6).
//!
//! Open: `POST /api/table-session/{gameId}` → `{actorId, token, gatewayToken, expiresAt}`;
//! `GET /api/rivet/metadata` → `{clientEndpoint, clientNamespace}` (cached per backend); a
//! `rivetkit_client::Client` on `clientEndpoint` (json encoding, metadata lookup off,
//! `token(gatewayToken)`); `get_for_id("euchreTable", actorId)` with params
//! `TableConnectParams::player(token)`; `connect()`; one `on_event` per
//! `TABLE_EVENT_NAMES` entry; wait for `Connected` within [`OPEN_TIMEOUT`]; emit
//! `ConnEvent::Status(true)` first.
//!
//! The client's credentials are fixed per `Client`, so the supervisor owns reconnects: on a
//! drop it emits `Status(false)`, stops rivetkit's own retry (`disconnect()`), drops that
//! client, and dials again with backoff ([`RECONNECT_FIRST`] doubling to [`RECONNECT_CAP`],
//! ±20 % jitter), minting a fresh table session when the old one is stale, a dial failed or an
//! action answered `forbidden`. After [`RECONNECT_GIVE_UP`] of failing it emits
//! `ConnEvent::Ended` and ends the stream. A live socket keeps the identity it proved when it
//! connected (D33: the gateway checks its token only at the upgrade), so nothing rotates on
//! a timer. Only an action answered `forbidden` rotates the session while up,
//! make-before-break, with no `Status` edge; a rotation the server refuses for good keeps
//! the socket and ends the table at its next drop.
//!
//! rivetkit-client 3.1.0 redials by itself the moment a socket drops, with the old
//! credentials, and `disconnect()` cannot abort a WebSocket handshake already under way (its
//! driver is not stored yet), so a late `disconnect()` both hangs and leaks the redialed
//! socket. Each dial therefore stops rivetkit's retry from inside its status callback, on the
//! first `Disconnected`, before the retry loop looks at its stop flag (see [`dial_rivet`]).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use crate::net::rivet_error::{self, RivetError};
use async_trait::async_trait;
use euchre_protocol::{
    CompletedHandReview, MoveAck, PublicGameView, ReviewTrickAck, ReviewTrickRequest,
    SubmitMoveRequest, TABLE_EVENT_NAMES, TableConnectParams, TableEvent, TableSessionResponse,
    actor_names, table_actions,
};
use euchre_server_core::rivet::parse_rivet_error;
use futures::FutureExt as _;
use rivetkit_client::connection::ActorConnection;
use rivetkit_client::{Client, ClientConfig, ConnectionStatus, EncodingKind, GetOptions};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use super::http::wire::RivetMetadata;
use crate::net::{ConnEvent, NetError, TableCalls, TableConn};

/// How long an open waits for the WebSocket to reach `Connected`.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
/// The first reconnect wait after a drop.
pub const RECONNECT_FIRST: Duration = Duration::from_millis(250);
/// The longest reconnect wait.
pub const RECONNECT_CAP: Duration = Duration::from_secs(8);
/// How long reconnecting keeps failing before the connection ends for good.
pub const RECONNECT_GIVE_UP: Duration = Duration::from_secs(120);
/// How long `close()` waits for the socket to say goodbye.
pub const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// The shortest time between two session rotations, so a server that keeps answering
/// `forbidden` cannot spin them.
const ROTATE_FLOOR: Duration = Duration::from_secs(30);
/// How often a rotation checks that the old socket's calls have finished.
const DRAIN_POLL: Duration = Duration::from_millis(20);

/// The supervisor's clock settings: [`Timing::REAL`] in the app; the supervisor tests run
/// the same code on a scaled-down copy (tokio's paused clock needs `test-util`, which this
/// crate does not enable).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Timing {
    /// [`RECONNECT_FIRST`].
    pub(super) first: Duration,
    /// [`RECONNECT_CAP`].
    pub(super) cap: Duration,
    /// [`RECONNECT_GIVE_UP`].
    pub(super) give_up: Duration,
    /// [`CLOSE_TIMEOUT`].
    pub(super) close: Duration,
    /// The shortest time between two rotations.
    pub(super) rotate_floor: Duration,
    /// How often a rotation checks the old socket's calls.
    pub(super) drain_poll: Duration,
}

impl Timing {
    /// The contract's numbers.
    pub(super) const REAL: Timing = Timing {
        first: RECONNECT_FIRST,
        cap: RECONNECT_CAP,
        give_up: RECONNECT_GIVE_UP,
        close: CLOSE_TIMEOUT,
        rotate_floor: ROTATE_FLOOR,
        drain_poll: DRAIN_POLL,
    };
}

// ------------------------------------------------------------------------- seams

/// A boxed, sendable future.
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What the supervisor needs from outside: the table session mint and one dial. The real
/// one is the backend's (`remote.rs`); the supervisor tests use a scripted one.
#[async_trait]
pub(super) trait Dialer: Send + Sync + 'static {
    /// `POST /api/table-session/{gameId}`, with the device session's recovery.
    async fn mint(&self) -> Result<TableSessionResponse, NetError>;
    /// One gateway connection with `session`, `Ok` once it is `Connected`.
    async fn dial(&self, session: &TableSessionResponse) -> Result<Dialed, NetError>;
    /// Wall-clock milliseconds, the clock `expiresAt` is on.
    fn now_ms(&self) -> u64;
}

/// One connected socket and the events it receives (buffered until the supervisor reads
/// them, so a connection that is not live yet holds its events in order).
pub(super) struct Dialed {
    pub(super) link: Arc<dyn Link>,
    pub(super) events: mpsc::UnboundedReceiver<TableEvent>,
}

/// One socket.
#[async_trait]
pub(super) trait Link: Send + Sync + 'static {
    /// The socket is `Connected`.
    fn is_up(&self) -> bool;
    /// Resolves once the socket is no longer `Connected`.
    async fn lost(&self);
    /// One action; fails at once when the socket is not up, and when it drops mid-call.
    async fn action(&self, name: &str, args: Vec<Value>) -> anyhow::Result<Value>;
    /// Disconnects and drops the client (callers bound it).
    async fn close(&self);
}

// ------------------------------------------------------------------------ shared

/// What the backend, the [`RemoteTable`] and the tests tell a supervisor.
#[derive(Debug)]
enum Control {
    /// Drop the socket as a network blip would (`interrupt_tables`).
    Interrupt,
    /// End for good with this error (`end_tables`).
    End(NetError),
    /// An action answered `forbidden` / `invalid_token`: mint a fresh session now.
    Refresh,
    /// `close()`: stop, no `Ended`; answered once the socket is closed.
    Close(oneshot::Sender<()>),
}

/// The live socket and how many calls are using it (a rotation lets them finish).
#[derive(Clone)]
struct Active {
    link: Arc<dyn Link>,
    calls: Arc<AtomicUsize>,
}

impl Active {
    fn new(link: Arc<dyn Link>) -> Active {
        Active {
            link,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

/// Counts one call on a socket while it lives.
struct CallGuard(Arc<AtomicUsize>);

impl CallGuard {
    fn new(calls: &Arc<AtomicUsize>) -> CallGuard {
        calls.fetch_add(1, Ordering::SeqCst);
        CallGuard(Arc::clone(calls))
    }
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// State one table's supervisor and its [`RemoteTable`] share.
struct Shared {
    game_id: String,
    /// The socket actions use; `None` while down.
    active: parking_lot::Mutex<Option<Active>>,
    control: mpsc::UnboundedSender<Control>,
    /// The supervisor has stopped (close, end, give up).
    stopped: AtomicBool,
    timing: Timing,
}

impl Shared {
    /// Makes `link` the live socket; returns the one it replaces.
    fn set_active(&self, link: Arc<dyn Link>) -> Option<Active> {
        self.active.lock().replace(Active::new(link))
    }

    fn clear_active(&self) -> Option<Active> {
        self.active.lock().take()
    }

    /// The live socket, when it is up.
    fn up(&self) -> Option<Active> {
        self.active
            .lock()
            .as_ref()
            .filter(|a| a.link.is_up())
            .cloned()
    }
}

/// The backend's handle on one open table (its registry holds these).
#[derive(Clone, Debug)]
pub(super) struct TableHandle {
    control: mpsc::UnboundedSender<Control>,
}

impl TableHandle {
    /// Drops the live socket as a blip would; the table reconnects by itself.
    pub(super) fn interrupt(&self) {
        let _ = self.control.send(Control::Interrupt);
    }

    /// Ends the table for good with `err`.
    pub(super) fn end(&self, err: NetError) {
        let _ = self.control.send(Control::End(err));
    }

    /// The table's supervisor still runs.
    pub(super) fn is_live(&self) -> bool {
        !self.control.is_closed()
    }
}

// ---------------------------------------------------------------------- the open

/// Opens `game_id`: mint, one dial, then the supervisor. A failed open leaves nothing
/// running.
pub(super) async fn open(
    game_id: &str,
    dialer: Arc<dyn Dialer>,
) -> Result<(TableConn, TableHandle), NetError> {
    open_with(game_id, dialer, Timing::REAL).await
}

/// [`open`] on `timing`.
async fn open_with(
    game_id: &str,
    dialer: Arc<dyn Dialer>,
    timing: Timing,
) -> Result<(TableConn, TableHandle), NetError> {
    let session = dialer.mint().await?;
    let dialed = dialer.dial(&session).await?;
    Ok(start(game_id, dialer, session, dialed, timing))
}

/// Starts the supervisor over a connected `first` socket. `Status(true)` is the stream's
/// first item; `first`'s buffered events follow it in order.
fn start(
    game_id: &str,
    dialer: Arc<dyn Dialer>,
    session: TableSessionResponse,
    first: Dialed,
    timing: Timing,
) -> (TableConn, TableHandle) {
    let (out, events) = mpsc::unbounded_channel();
    let (control_tx, control) = mpsc::unbounded_channel();
    let shared = Arc::new(Shared {
        game_id: game_id.to_owned(),
        active: parking_lot::Mutex::new(None),
        control: control_tx.clone(),
        stopped: AtomicBool::new(false),
        timing,
    });
    shared.set_active(Arc::clone(&first.link));
    let _ = out.send(ConnEvent::Status(true));
    let supervisor = Supervisor {
        dialer,
        shared: Arc::clone(&shared),
        out,
        control,
        session,
        refresh: false,
        final_err: None,
        timing,
    };
    tokio::spawn(supervisor.run(first));
    let conn = TableConn {
        game_id: game_id.to_owned(),
        events,
        calls: Arc::new(RemoteTable { shared }),
    };
    (
        conn,
        TableHandle {
            control: control_tx,
        },
    )
}

// ------------------------------------------------------------------ supervisor

/// `d` with ±20 % jitter.
fn jittered(d: Duration) -> Duration {
    // Top 32 bits of a v4 UUID: random enough for spreading reconnects.
    let bits = (uuid::Uuid::new_v4().as_u128() >> 96) as u32;
    let unit = f64::from(bits) / f64::from(u32::MAX);
    d.mul_f64(0.8 + 0.4 * unit)
}

/// Closes `link` within `within` ([`CLOSE_TIMEOUT`]).
async fn close_bounded(link: Arc<dyn Link>, within: Duration) {
    if tokio::time::timeout(within, link.close()).await.is_err() {
        tracing::warn!("table socket did not close in time; dropped");
    }
}

/// Closes `link` in the background, within `within`.
fn spawn_close(link: Arc<dyn Link>, within: Duration) {
    tokio::spawn(close_bounded(link, within));
}

/// Lets the old socket's calls finish (up to `timing.close`), then closes it.
fn spawn_drain_close(old: Active, timing: Timing) {
    tokio::spawn(async move {
        let deadline = Instant::now() + timing.close;
        while old.calls.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            tokio::time::sleep(timing.drain_poll).await;
        }
        close_bounded(old.link, timing.close).await;
    });
}

/// Awaits `f` when there is one; never resolves otherwise.
async fn maybe<T>(f: &mut Option<BoxFuture<T>>) -> T {
    match f {
        Some(f) => f.await,
        None => std::future::pending().await,
    }
}

/// Mint (when `mint`) then dial; the error says whether the mint failed.
fn attempt(
    dialer: Arc<dyn Dialer>,
    session: TableSessionResponse,
    mint: bool,
) -> BoxFuture<Result<(TableSessionResponse, Dialed), (NetError, bool)>> {
    Box::pin(async move {
        let session = if mint {
            dialer.mint().await.map_err(|e| (e, true))?
        } else {
            session
        };
        let dialed = dialer.dial(&session).await.map_err(|e| (e, false))?;
        Ok((session, dialed))
    })
}

/// How the up state ended.
enum Up {
    /// The socket dropped: reconnect.
    Lost,
    /// Stopped for good.
    Stop,
}

/// What a control message means to the down state.
enum Flow {
    Continue,
    Stop,
}

struct Supervisor {
    dialer: Arc<dyn Dialer>,
    shared: Arc<Shared>,
    out: mpsc::UnboundedSender<ConnEvent>,
    control: mpsc::UnboundedReceiver<Control>,
    session: TableSessionResponse,
    /// An action answered `forbidden`: mint before the next dial.
    refresh: bool,
    /// A rotation's mint was refused for good (`forbidden`, `not found`, `unauthorized`):
    /// the socket stays while it lives, and the table ends with this at its next drop.
    final_err: Option<NetError>,
    timing: Timing,
}

impl Supervisor {
    async fn run(mut self, first: Dialed) {
        let mut dialed = first;
        loop {
            if let Up::Stop = self.up(dialed).await {
                break;
            }
            match self.down().await {
                Some(next) => dialed = next,
                None => break,
            }
        }
        self.shared.stopped.store(true, Ordering::SeqCst);
        if let Some(active) = self.shared.clear_active() {
            spawn_close(active.link, self.timing.close);
        }
        tracing::debug!(game_id = %self.shared.game_id, "table supervisor stopped");
    }

    /// Stops for good: nothing active, `Ended(err)` when given, the socket closed.
    async fn stop(&mut self, link: Option<Arc<dyn Link>>, ended: Option<NetError>) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        self.shared.clear_active();
        if let Some(err) = ended {
            tracing::info!(game_id = %self.shared.game_id, error = ?err, "table connection ended");
            let _ = self.out.send(ConnEvent::Ended(err));
        }
        if let Some(link) = link {
            close_bounded(link, self.timing.close).await;
        }
    }

    /// The socket is up: forward its events, rotate the session when an action was refused,
    /// watch for a drop.
    async fn up(&mut self, mut dialed: Dialed) -> Up {
        let mut events_open = true;
        let mut rotation: Option<BoxFuture<Result<(TableSessionResponse, Dialed), NetError>>> =
            None;
        // When the next rotation starts: only after a refused action (review fix PP-4).
        let mut rotate_at: Option<Instant> = None;
        let mut last_rotation: Option<Instant> = None;
        let mut retry = self.timing.first;
        loop {
            tokio::select! {
                biased;
                ctl = self.control.recv() => match ctl {
                    Some(Control::Close(ack)) => {
                        self.stop(Some(dialed.link), None).await;
                        let _ = ack.send(());
                        return Up::Stop;
                    }
                    Some(Control::End(err)) => {
                        self.stop(Some(dialed.link), Some(err)).await;
                        return Up::Stop;
                    }
                    Some(Control::Interrupt) => {
                        tracing::info!(game_id = %self.shared.game_id, "table socket interrupted");
                        self.went_down(dialed.link);
                        return Up::Lost;
                    }
                    Some(Control::Refresh) => {
                        self.refresh = true;
                        if rotation.is_none() && rotate_at.is_none() && self.final_err.is_none() {
                            let now = Instant::now();
                            let soonest =
                                last_rotation.map_or(now, |at| at + self.timing.rotate_floor);
                            rotate_at = Some(soonest.max(now));
                        }
                    }
                    None => {
                        self.stop(Some(dialed.link), None).await;
                        return Up::Stop;
                    }
                },
                () = self.out.closed() => {
                    self.stop(Some(dialed.link), None).await;
                    return Up::Stop;
                }
                ev = dialed.events.recv(), if events_open => match ev {
                    Some(ev) => {
                        if self.out.send(ConnEvent::Event(ev)).is_err() {
                            self.stop(Some(dialed.link), None).await;
                            return Up::Stop;
                        }
                    }
                    None => events_open = false,
                },
                () = dialed.link.lost() => {
                    tracing::info!(game_id = %self.shared.game_id, "table socket dropped");
                    self.went_down(dialed.link);
                    return Up::Lost;
                }
                () = tokio::time::sleep_until(rotate_at.unwrap_or_else(Instant::now)),
                    if rotation.is_none() && rotate_at.is_some() =>
                {
                    rotate_at = None;
                    let dialer = Arc::clone(&self.dialer);
                    rotation = Some(Box::pin(async move {
                        let session = dialer.mint().await?;
                        let dialed = dialer.dial(&session).await?;
                        Ok((session, dialed))
                    }));
                }
                res = maybe(&mut rotation) => {
                    rotation = None;
                    match res {
                        Ok((session, next)) => {
                            // Make-before-break: the new socket is live from here; the old
                            // one's unread events are dropped with its receiver.
                            let old = std::mem::replace(&mut dialed, next);
                            events_open = true;
                            self.session = session;
                            self.refresh = false;
                            retry = self.timing.first;
                            match self.shared.set_active(Arc::clone(&dialed.link)) {
                                Some(active) => spawn_drain_close(active, self.timing),
                                None => spawn_close(old.link, self.timing.close),
                            }
                            last_rotation = Some(Instant::now());
                            tracing::debug!(game_id = %self.shared.game_id, "table session rotated");
                        }
                        Err(err) if is_final(&err) => {
                            // Retrying cannot help (review fix PP-3): keep the socket while
                            // it lives and end the table at its next drop.
                            tracing::info!(
                                game_id = %self.shared.game_id,
                                error = ?err,
                                "table session cannot be renewed; keeping the socket until it drops"
                            );
                            self.final_err = Some(err);
                        }
                        Err(err) => {
                            tracing::warn!(
                                game_id = %self.shared.game_id,
                                error = ?err,
                                "table session rotation failed; keeping the socket"
                            );
                            rotate_at = Some(Instant::now() + jittered(retry));
                            retry = (retry * 2).min(self.timing.cap);
                        }
                    }
                }
            }
        }
    }

    /// The socket left `Connected`: nothing is live, `Status(false)`, close it (stops
    /// rivetkit's own retry, drops the client).
    fn went_down(&mut self, link: Arc<dyn Link>) {
        self.shared.clear_active();
        let _ = self.out.send(ConnEvent::Status(false));
        spawn_close(link, self.timing.close);
    }

    /// A control message while down.
    async fn on_control_down(&mut self, ctl: Option<Control>) -> Flow {
        match ctl {
            Some(Control::Interrupt) => Flow::Continue,
            Some(Control::Refresh) => {
                self.refresh = true;
                Flow::Continue
            }
            Some(Control::End(err)) => {
                self.stop(None, Some(err)).await;
                Flow::Stop
            }
            Some(Control::Close(ack)) => {
                self.stop(None, None).await;
                let _ = ack.send(());
                Flow::Stop
            }
            None => {
                self.stop(None, None).await;
                Flow::Stop
            }
        }
    }

    /// Down: dial again with backoff until one connects (`Some`), the table ends for good, or
    /// [`RECONNECT_GIVE_UP`] passes (`None`).
    async fn down(&mut self) -> Option<Dialed> {
        if let Some(err) = self.final_err.take() {
            self.stop(None, Some(err)).await;
            return None;
        }
        let give_up = Instant::now() + self.timing.give_up;
        let mut wait = self.timing.first;
        let mut last_err = NetError::Unavailable;
        let mut failed = false;
        loop {
            let until = (Instant::now() + jittered(wait)).min(give_up);
            wait = (wait * 2).min(self.timing.cap);
            loop {
                tokio::select! {
                    biased;
                    ctl = self.control.recv() => {
                        if let Flow::Stop = self.on_control_down(ctl).await {
                            return None;
                        }
                    }
                    () = self.out.closed() => {
                        self.stop(None, None).await;
                        return None;
                    }
                    () = tokio::time::sleep_until(until) => break,
                }
            }
            if Instant::now() >= give_up {
                tracing::warn!(game_id = %self.shared.game_id, error = ?last_err, "table reconnect gave up");
                self.stop(None, Some(last_err)).await;
                return None;
            }
            let mint = failed || self.refresh || self.session.is_stale(self.dialer.now_ms());
            let mut dial = attempt(Arc::clone(&self.dialer), self.session.clone(), mint);
            let result = loop {
                tokio::select! {
                    biased;
                    ctl = self.control.recv() => {
                        if let Flow::Stop = self.on_control_down(ctl).await {
                            return None;
                        }
                    }
                    () = self.out.closed() => {
                        self.stop(None, None).await;
                        return None;
                    }
                    res = &mut dial => break res,
                }
            };
            match result {
                Ok((session, dialed)) => {
                    self.session = session;
                    self.refresh = false;
                    self.shared.set_active(Arc::clone(&dialed.link));
                    if self.out.send(ConnEvent::Status(true)).is_err() {
                        self.stop(Some(dialed.link), None).await;
                        return None;
                    }
                    tracing::info!(game_id = %self.shared.game_id, "table reconnected");
                    return Some(dialed);
                }
                Err((err, true)) if is_final(&err) => {
                    self.stop(None, Some(err)).await;
                    return None;
                }
                Err((err, minted)) => {
                    tracing::warn!(
                        game_id = %self.shared.game_id,
                        error = ?err,
                        mint = minted,
                        "table reconnect failed; retrying"
                    );
                    last_err = err;
                    failed = true;
                }
            }
        }
    }
}

/// A mint answer that no retry can change: the table is another device's, gone, or the
/// device lost its session.
fn is_final(err: &NetError) -> bool {
    matches!(
        err,
        NetError::Forbidden | NetError::NotFound | NetError::Unauthorized | NetError::SignedOut
    )
}

// ------------------------------------------------------------------- the calls

/// The four actions of one open table. Every call fails fast with [`NetError::Closed`] while
/// the socket is not `Connected` (rivetkit would queue it and send it late).
pub struct RemoteTable {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for RemoteTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteTable")
            .field("game_id", &self.shared.game_id)
            .finish_non_exhaustive()
    }
}

/// `v` as an action argument.
fn arg<T: serde::Serialize>(v: &T) -> Result<Value, NetError> {
    serde_json::to_value(v).map_err(|_| NetError::Invalid("request".to_owned()))
}

/// Decodes an action's reply; a reply that does not decode is [`NetError::Unavailable`].
fn reply<T: DeserializeOwned>(name: &str, value: Value) -> Result<T, NetError> {
    serde_json::from_value(value).map_err(|err| {
        tracing::warn!(action = name, error = %err, "table reply did not decode");
        NetError::Unavailable
    })
}

/// [`action_error`], logged with the action name and the error's group/code only.
fn failed(name: &str, err: &anyhow::Error) -> NetError {
    let mapped = action_error(err);
    match parse_rivet_error(err) {
        Some(e) => {
            tracing::warn!(action = name, group = %e.group, code = %e.code, "table action failed")
        }
        None => tracing::warn!(action = name, "table action failed: the socket closed"),
    }
    mapped
}

impl RemoteTable {
    /// The table's game id.
    pub fn game_id(&self) -> &str {
        &self.shared.game_id
    }

    /// Runs `name` on the live socket: `Err(Closed)` at once when there is none; the inner
    /// result is the action's own.
    async fn invoke(
        &self,
        name: &'static str,
        args: Vec<Value>,
    ) -> Result<anyhow::Result<Value>, NetError> {
        let Some(active) = self.shared.up() else {
            return Err(NetError::Closed);
        };
        let _call = CallGuard::new(&active.calls);
        let result = active.link.action(name, args).await;
        if let Err(err) = &result
            && parse_rivet_error(err).is_some_and(|e| e.is("forbidden") || e.is("invalid_token"))
        {
            let _ = self.shared.control.send(Control::Refresh);
        }
        Ok(result)
    }
}

#[async_trait]
impl TableCalls for RemoteTable {
    async fn submit_move(&self, request: SubmitMoveRequest) -> Result<MoveAck, NetError> {
        let name = table_actions::SUBMIT_MOVE;
        match self.invoke(name, vec![arg(&request)?]).await? {
            Ok(value) => reply(name, value),
            Err(err) => match move_rejection(&err) {
                Some(ack) => Ok(ack),
                None => Err(failed(name, &err)),
            },
        }
    }

    async fn review_trick(&self, request: ReviewTrickRequest) -> Result<ReviewTrickAck, NetError> {
        let name = table_actions::REVIEW_TRICK;
        match self.invoke(name, vec![arg(&request)?]).await? {
            Ok(value) => reply(name, value),
            Err(err) => Err(failed(name, &err)),
        }
    }

    async fn snapshot(&self) -> Result<Option<PublicGameView>, NetError> {
        let name = table_actions::SNAPSHOT;
        match self.invoke(name, Vec::new()).await? {
            Ok(Value::Null) => Ok(None),
            Ok(value) => reply(name, value).map(Some),
            Err(err) => Err(failed(name, &err)),
        }
    }

    async fn get_hand_review(&self, hand_no: u32) -> Result<Option<CompletedHandReview>, NetError> {
        let name = table_actions::GET_HAND_REVIEW;
        match self.invoke(name, vec![Value::from(hand_no)]).await? {
            Ok(Value::Null) => Ok(None),
            Ok(value) => reply(name, value).map(Some),
            Err(err) => Err(failed(name, &err)),
        }
    }

    async fn close(&self) {
        if self.shared.stopped.load(Ordering::SeqCst) {
            return;
        }
        let (ack, done) = oneshot::channel();
        if self.shared.control.send(Control::Close(ack)).is_ok() {
            // The supervisor closes the socket within its close bound; a little grace on top.
            let _ =
                tokio::time::timeout(self.shared.timing.close + Duration::from_millis(500), done)
                    .await;
        }
    }
}

/// The structured error inside a failed native rivetkit action, in the shape both backends
/// share (`net::rivet_error`); `None` when the socket closed under the call.
fn structured(err: &anyhow::Error) -> Option<RivetError> {
    parse_rivet_error(err).map(|e| RivetError {
        group: e.group,
        code: e.code,
        message: e.message,
        metadata: e.metadata,
    })
}

/// `submitMove`'s thrown refusals as the ack the sim returns (`net::rivet_error::move_rejection`
/// over the native client's error); `None` for anything else, which [`action_error`] maps.
pub fn move_rejection(err: &anyhow::Error) -> Option<MoveAck> {
    rivet_error::move_rejection(&structured(err)?)
}

/// Maps a failed rivetkit action to the player's error (contract §N1.6, shared mapping in
/// `net::rivet_error::action_error`).
pub fn action_error(err: &anyhow::Error) -> NetError {
    rivet_error::action_error(structured(err).as_ref())
}

// ---------------------------------------------------------- the rivetkit socket

/// Disconnects a half-built connection and drops its client when a dial fails or is
/// cancelled (a dropped `ActorConnection` keeps running: its task holds a clone).
struct CloseOnDrop {
    conn: Option<ActorConnection>,
    client: Option<Client>,
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else {
            return;
        };
        let client = self.client.take();
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn(async move {
                    let _ = tokio::time::timeout(CLOSE_TIMEOUT, conn.disconnect()).await;
                    drop(client);
                });
            }
            Err(_) => {
                // No runtime (process teardown): stop rivetkit's retry loop at least.
                let _ = conn.disconnect().now_or_never();
            }
        }
    }
}

/// One rivetkit connection to the table actor through the server's `/rivet` gateway.
struct RivetLink {
    conn: ActorConnection,
    client: parking_lot::Mutex<Option<Client>>,
}

#[async_trait]
impl Link for RivetLink {
    fn is_up(&self) -> bool {
        self.conn.conn_status() == ConnectionStatus::Connected
    }

    async fn lost(&self) {
        let mut status = self.conn.status_receiver();
        let _ = status
            .wait_for(|s| *s != ConnectionStatus::Connected)
            .await
            .map(|_| ());
    }

    async fn action(&self, name: &str, args: Vec<Value>) -> anyhow::Result<Value> {
        if !self.is_up() {
            anyhow::bail!("the table socket is not connected");
        }
        tokio::select! {
            res = self.conn.action(name, args) => res,
            () = self.lost() => Err(anyhow::anyhow!("Socket closed during rpc")),
        }
    }

    async fn close(&self) {
        self.conn.disconnect().await;
        drop(self.client.lock().take());
    }
}

/// Dials one gateway connection for `session` (contract §N1.6 steps 4–7): a client of its
/// own (the gateway token is fixed per client), `get_for_id` with the player's params, the
/// five events, and `Connected` within `timeout`. Any failure disconnects it again.
pub(super) async fn dial_rivet(
    meta: &RivetMetadata,
    session: &TableSessionResponse,
    timeout: Duration,
) -> Result<Dialed, NetError> {
    let Some(gateway) = session.gateway_token.clone() else {
        tracing::warn!("table session has no gateway token; not dialing");
        return Err(NetError::Unavailable);
    };
    let client = Client::new(
        ClientConfig::new(meta.client_endpoint.clone())
            .namespace(meta.client_namespace.clone())
            .encoding(EncodingKind::Json)
            .disable_metadata_lookup(true)
            .token(gateway),
    );
    let params = arg(&TableConnectParams::player(session.token.clone()))?;
    let handle = client
        .get_for_id(
            actor_names::EUCHRE_TABLE,
            &session.actor_id,
            GetOptions {
                params: Some(params),
            },
        )
        .map_err(|err| {
            tracing::warn!(error = %err, "table handle not built");
            NetError::Unavailable
        })?;
    let conn = handle.connect();
    let mut guard = CloseOnDrop {
        conn: Some(Arc::clone(&conn)),
        client: Some(client),
    };
    // On the first drop, stop rivetkit's own retry before its loop checks the stop flag:
    // the first poll of `disconnect()` sets that flag synchronously. Without this rivetkit
    // redials at once with this socket's credentials, and a later `disconnect()` cannot
    // abort that handshake. The supervisor dials again with a fresh session.
    let weak = Arc::downgrade(&conn);
    conn.on_status_change(move |status| {
        if status == ConnectionStatus::Disconnected
            && let Some(conn) = weak.upgrade()
        {
            let _ = conn.disconnect().now_or_never();
        }
    })
    .await;
    let (tx, events) = mpsc::unbounded_channel();
    for name in TABLE_EVENT_NAMES {
        let tx = tx.clone();
        // Dropping the subscription handle keeps the subscription (rivetkit 3.1.0).
        let _ = conn
            .on_event(name, move |args| {
                let payload = args.first().cloned().unwrap_or(Value::Null);
                match TableEvent::decode(name, payload) {
                    Ok(event) => {
                        let _ = tx.send(event);
                    }
                    Err(_) => tracing::warn!(event = name, "table event did not decode; dropped"),
                }
            })
            .await;
    }
    drop(tx);
    let mut status = conn.status_receiver();
    let settled = tokio::time::timeout(timeout, async {
        status
            .wait_for(|s| {
                matches!(
                    s,
                    ConnectionStatus::Connected | ConnectionStatus::Disconnected
                )
            })
            .await
            .map(|s| *s)
    })
    .await;
    match settled {
        Ok(Ok(ConnectionStatus::Connected)) => {}
        Ok(_) => {
            tracing::warn!("table socket closed while opening");
            return Err(NetError::Unavailable);
        }
        Err(_) => {
            tracing::warn!(
                timeout_ms = timeout.as_millis() as u64,
                "table socket did not open in time"
            );
            return Err(NetError::Timeout);
        }
    }
    let client = guard.client.take();
    guard.conn = None;
    Ok(Dialed {
        link: Arc::new(RivetLink {
            conn,
            client: parking_lot::Mutex::new(client),
        }),
        events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use euchre_protocol::{ErrorCode, ProtocolErrorCode};
    use std::collections::VecDeque;
    use tokio::sync::watch;

    // ------------------------------------------------------------ error mapping

    /// An error formatted the way rivetkit-client 3.1.0 surfaces a thrown action error:
    /// `RPC Error({group}/{code}): {message}, {metadata:#}`.
    fn rpc_error(group: &str, code: &str, message: &str, metadata: &Value) -> anyhow::Error {
        anyhow::anyhow!("RPC Error({group}/{code}): {message}, {metadata:#}")
    }

    #[test]
    fn illegal_move_becomes_a_rejected_ack() -> anyhow::Result<()> {
        let legal = serde_json::json!([
            { "id": "pass", "move": { "t": "pass" }, "label": "Pass" },
        ]);
        let legal_moves: Vec<euchre_protocol::LegalMove> = serde_json::from_value(legal.clone())?;
        let err = rpc_error(
            "user",
            "illegal_move",
            "That move is not legal now",
            &serde_json::json!({ "legal": legal }),
        );
        let Some(MoveAck::Rejected { code, legal }) = move_rejection(&err) else {
            anyhow::bail!("not a rejection: {:?}", move_rejection(&err));
        };
        assert_eq!(code.as_str(), "illegal_move");
        assert_eq!(legal, legal_moves);
        // `game_complete` has no legal set.
        let err = rpc_error("user", "game_complete", "Game over", &Value::Null);
        assert_eq!(
            move_rejection(&err),
            Some(MoveAck::Rejected {
                code: ErrorCode::Protocol(ProtocolErrorCode::GameComplete),
                legal: Vec::new(),
            })
        );
        // `forbidden` is not a rejection, nor are other groups or unknown codes.
        assert_eq!(
            move_rejection(&rpc_error(
                "user",
                "forbidden",
                "Not your game",
                &Value::Null
            )),
            None
        );
        assert_eq!(
            move_rejection(&rpc_error("actor", "internal_error", "x", &Value::Null)),
            None
        );
        assert_eq!(
            move_rejection(&rpc_error("user", "made_up", "x", &Value::Null)),
            None
        );
        assert_eq!(
            move_rejection(&anyhow::anyhow!("Socket closed during rpc")),
            None
        );
        Ok(())
    }

    #[test]
    fn forbidden_and_other_errors_map() {
        assert_eq!(
            action_error(&rpc_error(
                "user",
                "forbidden",
                "Not your game",
                &Value::Null
            )),
            NetError::Forbidden
        );
        assert_eq!(
            action_error(&rpc_error("user", "invalid_token", "expired", &Value::Null)),
            NetError::Invalid("invalid_token".into())
        );
        assert_eq!(
            action_error(&rpc_error(
                "user",
                "rate_limited",
                "slow down",
                &serde_json::json!({ "retryAfterMs": 500 })
            )),
            NetError::Invalid("rate_limited".into())
        );
        assert_eq!(
            action_error(&rpc_error("actor", "action_timed_out", "x", &Value::Null)),
            NetError::Unavailable
        );
        assert_eq!(
            action_error(&rpc_error("core", "internal_error", "x", &Value::Null)),
            NetError::Unavailable
        );
    }

    #[test]
    fn a_closed_socket_is_closed() {
        assert_eq!(
            action_error(&anyhow::anyhow!("Socket closed during rpc")),
            NetError::Closed
        );
        assert_eq!(
            action_error(&anyhow::anyhow!("the table socket is not connected")),
            NetError::Closed
        );
    }

    // ----------------------------------------------------------------- doubles

    /// The supervisor tests' clock: the contract's shape, scaled down so a run takes a
    /// couple of seconds on the real clock.
    const TEST: Timing = Timing {
        first: Duration::from_millis(40),
        cap: Duration::from_millis(160),
        give_up: Duration::from_millis(1500),
        close: Duration::from_millis(150),
        rotate_floor: Duration::from_millis(100),
        drain_poll: Duration::from_millis(5),
    };

    /// Scheduling slack on top of a timer.
    const SLACK_MS: u64 = 60;

    /// A socket the test drives: `drop_socket()` is a drop.
    struct FakeLink {
        up: watch::Sender<bool>,
        closed: AtomicBool,
        actions: parking_lot::Mutex<Vec<String>>,
        /// Scripted replies; empty → `Ok(null)`.
        replies: parking_lot::Mutex<VecDeque<anyhow::Result<Value>>>,
        /// When set, the next action waits for this before it answers.
        hold: parking_lot::Mutex<Option<oneshot::Receiver<()>>>,
    }

    impl FakeLink {
        fn new() -> Arc<FakeLink> {
            Arc::new(FakeLink {
                up: watch::Sender::new(true),
                closed: AtomicBool::new(false),
                actions: parking_lot::Mutex::new(Vec::new()),
                replies: parking_lot::Mutex::new(VecDeque::new()),
                hold: parking_lot::Mutex::new(None),
            })
        }

        /// The next action waits until the returned sender fires (or drops).
        fn hold_next(&self) -> oneshot::Sender<()> {
            let (release, held) = oneshot::channel();
            *self.hold.lock() = Some(held);
            release
        }

        fn drop_socket(&self) {
            self.up.send_replace(false);
        }

        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }

        fn actions(&self) -> Vec<String> {
            self.actions.lock().clone()
        }
    }

    #[async_trait]
    impl Link for FakeLink {
        fn is_up(&self) -> bool {
            *self.up.borrow()
        }

        async fn lost(&self) {
            let mut rx = self.up.subscribe();
            let _ = rx.wait_for(|up| !*up).await.map(|_| ());
        }

        async fn action(&self, name: &str, _args: Vec<Value>) -> anyhow::Result<Value> {
            if !self.is_up() {
                anyhow::bail!("the table socket is not connected");
            }
            self.actions.lock().push(name.to_owned());
            let held = self.hold.lock().take();
            if let Some(held) = held {
                let _ = held.await;
            }
            self.replies.lock().pop_front().unwrap_or(Ok(Value::Null))
        }

        async fn close(&self) {
            self.closed.store(true, Ordering::SeqCst);
            self.up.send_replace(false);
        }
    }

    /// One dial the test scripted: a link (with its event receiver) or an error.
    enum DialStep {
        Link(Arc<FakeLink>, mpsc::UnboundedReceiver<TableEvent>),
        Fail(NetError),
    }

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        /// A mint at this many ms since the dialer was built.
        Mint(u64),
        /// A dial at this many ms, with this session's token.
        Dial(u64, String),
    }

    /// The scripted dialer; its wall clock is `BASE + elapsed`.
    struct FakeDialer {
        start: Instant,
        mints: parking_lot::Mutex<VecDeque<Result<TableSessionResponse, NetError>>>,
        dials: parking_lot::Mutex<VecDeque<DialStep>>,
        calls: parking_lot::Mutex<Vec<Call>>,
    }

    const BASE: u64 = 1_900_000_000_000;

    /// A session whose gateway token expires `expires_in` after the dialer's start.
    fn session(token: &str, expires_in: Duration) -> TableSessionResponse {
        TableSessionResponse {
            actor_id: "actor-1".into(),
            token: token.into(),
            gateway_token: Some(format!("gw-{token}")),
            expires_at: BASE + expires_in.as_millis() as u64,
        }
    }

    impl FakeDialer {
        fn new() -> Arc<FakeDialer> {
            Arc::new(FakeDialer {
                start: Instant::now(),
                mints: parking_lot::Mutex::new(VecDeque::new()),
                dials: parking_lot::Mutex::new(VecDeque::new()),
                calls: parking_lot::Mutex::new(Vec::new()),
            })
        }

        fn ms(&self) -> u64 {
            self.start.elapsed().as_millis() as u64
        }

        fn mint_ok(&self, s: TableSessionResponse) {
            self.mints.lock().push_back(Ok(s));
        }

        fn mint_err(&self, e: NetError) {
            self.mints.lock().push_back(Err(e));
        }

        /// Scripts a dial that connects; returns the link and its event sender.
        fn dial_ok(&self) -> (Arc<FakeLink>, mpsc::UnboundedSender<TableEvent>) {
            let link = FakeLink::new();
            let (tx, rx) = mpsc::unbounded_channel();
            self.dials
                .lock()
                .push_back(DialStep::Link(Arc::clone(&link), rx));
            (link, tx)
        }

        fn dial_err(&self, e: NetError) {
            self.dials.lock().push_back(DialStep::Fail(e));
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().clone()
        }

        fn dial_times(&self) -> Vec<u64> {
            self.calls()
                .iter()
                .filter_map(|c| match c {
                    Call::Dial(at, _) => Some(*at),
                    Call::Mint(_) => None,
                })
                .collect()
        }

        fn dial_tokens(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter_map(|c| match c {
                    Call::Dial(_, token) => Some(token),
                    Call::Mint(_) => None,
                })
                .collect()
        }
    }

    #[async_trait]
    impl Dialer for FakeDialer {
        async fn mint(&self) -> Result<TableSessionResponse, NetError> {
            self.calls.lock().push(Call::Mint(self.ms()));
            self.mints
                .lock()
                .pop_front()
                .unwrap_or(Err(NetError::Unavailable))
        }

        async fn dial(&self, session: &TableSessionResponse) -> Result<Dialed, NetError> {
            self.calls
                .lock()
                .push(Call::Dial(self.ms(), session.token.clone()));
            match self.dials.lock().pop_front() {
                Some(DialStep::Link(link, events)) => Ok(Dialed { link, events }),
                Some(DialStep::Fail(e)) => Err(e),
                None => Err(NetError::Unavailable),
            }
        }

        fn now_ms(&self) -> u64 {
            BASE + self.ms()
        }
    }

    /// A small distinct event per `n` (seat `n % 4`, online on even `n`).
    fn presence(n: u32) -> TableEvent {
        match TableEvent::decode(
            "presence",
            serde_json::json!({ "seat": n % 4, "online": n.is_multiple_of(2) }),
        ) {
            Ok(event) => event,
            Err(err) => panic!("presence fixture: {err}"),
        }
    }

    /// Opens a table over `dialer`, whose first mint and dial the test scripted.
    async fn open_fake(dialer: &Arc<FakeDialer>) -> (TableConn, TableHandle) {
        let dyn_dialer: Arc<dyn Dialer> = dialer.clone();
        match open_with("4f1c2b0e-8d7a-4f61-9a3e-2b1c0d9e8f7a", dyn_dialer, TEST).await {
            Ok(opened) => opened,
            Err(err) => panic!("open failed: {err:?}"),
        }
    }

    /// The next stream item within `within`; `None` when nothing came or the stream ended.
    async fn next(conn: &mut TableConn, within: Duration) -> Option<ConnEvent> {
        tokio::time::timeout(within, conn.events.recv())
            .await
            .ok()
            .flatten()
    }

    const SOON: Duration = Duration::from_secs(2);

    /// `gap` is `base` ±20 %, plus scheduling slack on the late side.
    fn within_jitter(gap: u64, base: Duration) -> bool {
        let base = base.as_millis() as u64;
        (base * 8 / 10..=base * 12 / 10 + SLACK_MS).contains(&gap)
    }

    // -------------------------------------------------------------- supervisor

    #[tokio::test]
    async fn drop_sends_status_false_then_reconnects_with_backoff() {
        let dialer = FakeDialer::new();
        dialer.mint_ok(session("a", Duration::from_secs(900)));
        let (first, _tx1) = dialer.dial_ok();
        let (mut conn, _handle) = open_fake(&dialer).await;
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));

        // Two failed dials, then one that connects. The second and third mint first: the
        // previous dial failed.
        dialer.dial_err(NetError::Unavailable);
        dialer.mint_ok(session("b", Duration::from_secs(900)));
        dialer.dial_err(NetError::Timeout);
        dialer.mint_ok(session("c", Duration::from_secs(900)));
        let (third, _tx3) = dialer.dial_ok();
        let dropped_at = dialer.ms();
        first.drop_socket();
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(false)));
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        assert!(first.is_closed(), "the dropped socket is closed");
        assert!(third.is_up());

        let at = dialer.dial_times();
        // The open's dial, then three reconnect dials.
        assert_eq!(at.len(), 4, "{:?}", dialer.calls());
        let gaps = [at[1] - dropped_at, at[2] - at[1], at[3] - at[2]];
        for (gap, base) in gaps
            .into_iter()
            .zip([TEST.first, TEST.first * 2, TEST.first * 4])
        {
            assert!(
                within_jitter(gap, base),
                "gap {gap} ms vs {base:?}: {:?}",
                dialer.calls()
            );
        }
        // The first redial reused the session; the next two minted first.
        assert_eq!(dialer.dial_tokens(), ["a", "a", "b", "c"]);
        conn.calls.close().await;
        assert_eq!(conn.events.recv().await, None);
    }

    #[tokio::test]
    async fn stale_session_is_minted_again_before_a_dial() {
        let dialer = FakeDialer::new();
        // Expires in 9 s: inside the 10 s margin, so stale already.
        dialer.mint_ok(session("a", Duration::from_secs(9)));
        let (first, _tx1) = dialer.dial_ok();
        let (mut conn, _handle) = open_fake(&dialer).await;
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        dialer.mint_ok(session("b", Duration::from_secs(900)));
        let (_second, _tx2) = dialer.dial_ok();
        first.drop_socket();
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(false)));
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        let calls = dialer.calls();
        assert!(
            matches!(
                calls.as_slice(),
                [Call::Mint(_), Call::Dial(_, a), Call::Mint(_), Call::Dial(_, b)]
                    if a == "a" && b == "b"
            ),
            "{calls:?}"
        );
        conn.calls.close().await;
    }

    #[tokio::test]
    async fn rotation_swaps_without_a_status_edge() -> anyhow::Result<()> {
        let dialer = FakeDialer::new();
        dialer.mint_ok(session("a", Duration::from_secs(900)));
        let (first, tx1) = dialer.dial_ok();
        let (mut conn, _handle) = open_fake(&dialer).await;
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        dialer.mint_ok(session("b", Duration::from_secs(900)));
        let (second, tx2) = dialer.dial_ok();
        // A refused action rotates the session.
        first.replies.lock().push_back(Err(forbidden()));
        let asked = dialer.ms();
        assert_eq!(conn.calls.snapshot().await, Err(NetError::Forbidden));
        // The new socket's first event waits in its buffer until the swap.
        tx2.send(presence(2))?;
        tx1.send(presence(1))?;
        assert_eq!(
            next(&mut conn, SOON).await,
            Some(ConnEvent::Event(presence(1)))
        );
        // Across the rotation only the new socket's event comes: no Status edge.
        assert_eq!(
            next(&mut conn, SOON).await,
            Some(ConnEvent::Event(presence(2)))
        );
        let calls = dialer.calls();
        assert!(
            matches!(
                calls.as_slice(),
                [_, _, Call::Mint(at), Call::Dial(_, b)] if *at < asked + SLACK_MS && b == "b"
            ),
            "{calls:?}"
        );
        // From the swap on, the old socket's events are dropped.
        let _ = tx1.send(presence(9));
        tx2.send(presence(3))?;
        assert_eq!(
            next(&mut conn, SOON).await,
            Some(ConnEvent::Event(presence(3)))
        );
        // The old socket closes once its calls finish; actions use the new one.
        tokio::time::sleep(TEST.close + Duration::from_millis(SLACK_MS)).await;
        assert!(first.is_closed());
        assert!(!second.is_closed());
        assert_eq!(conn.calls.snapshot().await, Ok(None));
        assert_eq!(second.actions(), ["snapshot"]);
        assert_eq!(first.actions(), ["snapshot"], "only the refused call");
        assert_eq!(next(&mut conn, Duration::from_millis(100)).await, None);
        conn.calls.close().await;
        Ok(())
    }

    #[tokio::test]
    async fn rotation_lets_the_old_sockets_calls_finish() -> anyhow::Result<()> {
        let dialer = FakeDialer::new();
        dialer.mint_ok(session("a", Duration::from_secs(900)));
        let (first, _tx1) = dialer.dial_ok();
        let (mut conn, _handle) = open_fake(&dialer).await;
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        // A call that starts on the old socket keeps it open past the swap.
        let release = first.hold_next();
        let calls = Arc::clone(&conn.calls);
        let call = tokio::spawn(async move { calls.snapshot().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(first.actions(), ["snapshot"]);
        dialer.mint_ok(session("b", Duration::from_secs(900)));
        let (second, tx2) = dialer.dial_ok();
        // Another call is refused, which rotates the session under the held one.
        first.replies.lock().push_back(Err(forbidden()));
        assert_eq!(conn.calls.snapshot().await, Err(NetError::Forbidden));
        tx2.send(presence(2))?;
        // The new socket's event comes after the swap.
        assert_eq!(
            next(&mut conn, SOON).await,
            Some(ConnEvent::Event(presence(2)))
        );
        tokio::time::sleep(TEST.close / 3).await;
        assert!(!first.is_closed(), "closed under a running call");
        let _ = release.send(());
        assert_eq!(call.await?, Ok(None));
        tokio::time::sleep(TEST.drain_poll + Duration::from_millis(SLACK_MS)).await;
        assert!(first.is_closed());
        assert!(!second.is_closed());
        conn.calls.close().await;
        Ok(())
    }

    #[tokio::test]
    async fn forbidden_mint_ends_the_stream() {
        let dialer = FakeDialer::new();
        dialer.mint_ok(session("a", Duration::from_secs(900)));
        let (first, _tx1) = dialer.dial_ok();
        let (mut conn, handle) = open_fake(&dialer).await;
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        // The first redial fails, so the second one mints: 403.
        dialer.dial_err(NetError::Unavailable);
        dialer.mint_err(NetError::Forbidden);
        first.drop_socket();
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(false)));
        assert_eq!(
            next(&mut conn, SOON).await,
            Some(ConnEvent::Ended(NetError::Forbidden))
        );
        // The stream ends after Ended, and the table stops.
        assert_eq!(conn.events.recv().await, None);
        assert!(!handle.is_live());
        assert_eq!(conn.calls.snapshot().await, Err(NetError::Closed));
    }

    #[tokio::test]
    async fn not_found_and_unauthorized_mints_end_the_stream_too() {
        for (err, ended) in [
            (NetError::NotFound, NetError::NotFound),
            (NetError::Unauthorized, NetError::Unauthorized),
        ] {
            let dialer = FakeDialer::new();
            // Stale, so the first redial mints.
            dialer.mint_ok(session("a", Duration::from_secs(5)));
            let (first, _tx1) = dialer.dial_ok();
            let (mut conn, _handle) = open_fake(&dialer).await;
            assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
            dialer.mint_err(err);
            first.drop_socket();
            assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(false)));
            assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Ended(ended)));
            assert_eq!(conn.events.recv().await, None);
        }
    }

    #[tokio::test]
    async fn gives_up_after_two_minutes_with_the_last_error() {
        let dialer = FakeDialer::new();
        dialer.mint_ok(session("a", Duration::from_secs(900)));
        let (first, _tx1) = dialer.dial_ok();
        let (mut conn, _handle) = open_fake(&dialer).await;
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        // Every mint answers, every dial fails: the first few as unavailable, then timeouts.
        for n in 0..64 {
            dialer.dial_err(if n < 4 {
                NetError::Unavailable
            } else {
                NetError::Timeout
            });
            dialer.mint_ok(session("b", Duration::from_secs(900)));
        }
        let dropped = Instant::now();
        first.drop_socket();
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(false)));
        assert_eq!(
            next(&mut conn, TEST.give_up * 2).await,
            Some(ConnEvent::Ended(NetError::Timeout))
        );
        let after = dropped.elapsed();
        assert!(
            (TEST.give_up..TEST.give_up + Duration::from_millis(SLACK_MS)).contains(&after),
            "gave up after {after:?}"
        );
        assert_eq!(conn.events.recv().await, None);
        // The backoff is capped: no gap between redials is over the cap + 20 %.
        let at = dialer.dial_times();
        let cap = TEST.cap.as_millis() as u64;
        assert!(at.len() > 6, "{at:?}");
        assert!(
            at.windows(2)
                .skip(1)
                .all(|w| w[1] - w[0] <= cap * 12 / 10 + SLACK_MS),
            "{at:?}"
        );
    }

    #[tokio::test]
    async fn events_before_status_true_are_buffered_in_order() -> anyhow::Result<()> {
        let dialer = FakeDialer::new();
        dialer.mint_ok(session("a", Duration::from_secs(900)));
        let (first, tx1) = dialer.dial_ok();
        // These arrive while the open is still in flight.
        tx1.send(presence(1))?;
        tx1.send(presence(2))?;
        let (mut conn, _handle) = open_fake(&dialer).await;
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        assert_eq!(
            next(&mut conn, SOON).await,
            Some(ConnEvent::Event(presence(1)))
        );
        assert_eq!(
            next(&mut conn, SOON).await,
            Some(ConnEvent::Event(presence(2)))
        );
        // The same after a reconnect: the new socket's early events follow its Status(true).
        let (_second, tx2) = dialer.dial_ok();
        tx2.send(presence(3))?;
        tx2.send(presence(4))?;
        first.drop_socket();
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(false)));
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        assert_eq!(
            next(&mut conn, SOON).await,
            Some(ConnEvent::Event(presence(3)))
        );
        assert_eq!(
            next(&mut conn, SOON).await,
            Some(ConnEvent::Event(presence(4)))
        );
        conn.calls.close().await;
        Ok(())
    }

    #[tokio::test]
    async fn actions_fail_fast_while_down() {
        let dialer = FakeDialer::new();
        dialer.mint_ok(session("a", Duration::from_secs(900)));
        let (first, _tx1) = dialer.dial_ok();
        let (mut conn, _handle) = open_fake(&dialer).await;
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        first
            .replies
            .lock()
            .push_back(Ok(serde_json::json!({ "queued": true })));
        assert!(
            conn.calls
                .review_trick(ReviewTrickRequest::Set { enabled: true })
                .await
                .is_ok()
        );
        // Down, with every redial failing: calls answer Closed at once.
        for _ in 0..16 {
            dialer.dial_err(NetError::Offline);
            dialer.mint_ok(session("b", Duration::from_secs(900)));
        }
        first.drop_socket();
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(false)));
        let before = Instant::now();
        assert_eq!(conn.calls.snapshot().await, Err(NetError::Closed));
        assert_eq!(conn.calls.get_hand_review(0).await, Err(NetError::Closed));
        assert!(
            conn.calls
                .submit_move(SubmitMoveRequest {
                    move_id: "pass".to_owned(),
                    turn_id: None,
                    client_move_id: "m-1".into(),
                })
                .await
                == Err(NetError::Closed)
        );
        assert!(before.elapsed() < Duration::from_millis(20));
        assert_eq!(first.actions(), ["reviewTrick"]);
        // Close is quick and idempotent; the stream ends with no Ended.
        conn.calls.close().await;
        conn.calls.close().await;
        assert!(before.elapsed() < Duration::from_millis(SLACK_MS));
        assert_eq!(conn.events.recv().await, None);
    }

    #[tokio::test]
    async fn interrupt_and_end_drive_the_same_paths() {
        let dialer = FakeDialer::new();
        dialer.mint_ok(session("a", Duration::from_secs(900)));
        let (first, _tx1) = dialer.dial_ok();
        let (mut conn, handle) = open_fake(&dialer).await;
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        let (second, _tx2) = dialer.dial_ok();
        handle.interrupt();
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(false)));
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        assert!(first.is_closed());
        handle.end(NetError::ServerStopped);
        assert_eq!(
            next(&mut conn, SOON).await,
            Some(ConnEvent::Ended(NetError::ServerStopped))
        );
        assert_eq!(conn.events.recv().await, None);
        assert!(!handle.is_live());
        tokio::time::sleep(Duration::from_millis(SLACK_MS)).await;
        assert!(second.is_closed());
    }

    #[tokio::test]
    async fn a_dropped_stream_stops_the_supervisor() {
        let dialer = FakeDialer::new();
        dialer.mint_ok(session("a", Duration::from_secs(900)));
        let (first, _tx1) = dialer.dial_ok();
        let (conn, handle) = open_fake(&dialer).await;
        drop(conn);
        tokio::time::sleep(Duration::from_millis(SLACK_MS)).await;
        assert!(first.is_closed());
        assert!(!handle.is_live());
    }

    /// D33: an up socket never rotates on a timer, however close its session is to
    /// expiring; the next drop mints a fresh one.
    #[tokio::test]
    async fn nothing_rotates_without_a_refused_action() {
        let dialer = FakeDialer::new();
        dialer.mint_ok(session("a", Duration::from_millis(10_200)));
        let (first, _tx1) = dialer.dial_ok();
        let (mut conn, _handle) = open_fake(&dialer).await;
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(dialer.calls().len(), 2, "{:?}", dialer.calls());
        assert!(!first.is_closed());
        conn.calls.close().await;
    }

    /// A rotation refused for good is never retried; the socket stays while it lives and
    /// the table ends with the refusal at its next drop.
    #[tokio::test]
    async fn a_refused_rotation_keeps_the_socket_then_ends_at_the_drop() {
        let dialer = FakeDialer::new();
        dialer.mint_ok(session("a", Duration::from_secs(900)));
        let (first, _tx1) = dialer.dial_ok();
        let (mut conn, _handle) = open_fake(&dialer).await;
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        first.replies.lock().push_back(Err(forbidden()));
        dialer.mint_err(NetError::Forbidden);
        assert_eq!(conn.calls.snapshot().await, Err(NetError::Forbidden));
        tokio::time::sleep(Duration::from_millis(400)).await;
        let calls = dialer.calls();
        assert_eq!(calls.len(), 3, "one refused mint, no retries: {calls:?}");
        assert!(matches!(calls.last(), Some(Call::Mint(_))), "{calls:?}");
        // Still up: the socket serves.
        assert!(!first.is_closed());
        assert_eq!(conn.calls.snapshot().await, Ok(None));
        first.drop_socket();
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(false)));
        assert_eq!(
            next(&mut conn, SOON).await,
            Some(ConnEvent::Ended(NetError::Forbidden))
        );
        assert_eq!(dialer.calls().len(), 3, "no redial after the refusal");
    }

    fn forbidden() -> anyhow::Error {
        rpc_error("user", "forbidden", "Not your game", &Value::Null)
    }

    #[tokio::test]
    async fn a_forbidden_action_rotates_the_session_now() {
        let dialer = FakeDialer::new();
        dialer.mint_ok(session("a", Duration::from_secs(900)));
        let (first, _tx1) = dialer.dial_ok();
        let (mut conn, _handle) = open_fake(&dialer).await;
        assert_eq!(next(&mut conn, SOON).await, Some(ConnEvent::Status(true)));
        first.replies.lock().push_back(Err(rpc_error(
            "user",
            "forbidden",
            "Not your game",
            &Value::Null,
        )));
        dialer.mint_ok(session("b", Duration::from_secs(900)));
        let (second, _tx2) = dialer.dial_ok();
        let asked = dialer.ms();
        assert_eq!(conn.calls.snapshot().await, Err(NetError::Forbidden));
        tokio::time::sleep(Duration::from_millis(SLACK_MS)).await;
        let calls = dialer.calls();
        assert!(
            matches!(
                calls.as_slice(),
                [_, _, Call::Mint(at), Call::Dial(_, b)] if *at < asked + SLACK_MS && b == "b"
            ),
            "{calls:?}"
        );
        assert_eq!(conn.calls.snapshot().await, Ok(None));
        assert_eq!(second.actions(), ["snapshot"]);
        // No Status edge.
        assert_eq!(next(&mut conn, Duration::from_millis(100)).await, None);
        conn.calls.close().await;
    }
}
