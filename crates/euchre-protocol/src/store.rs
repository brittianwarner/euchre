//! The client table store as a pure, synchronous state machine (02 section 6).
//!
//! It ports `game/table.svelte.ts` (`TableStore`) together with the play page's
//! wiring around it (`routes/play/[gameId]/+page.svelte`): steps capture, the
//! optimistic preview keyed by `forV`, the last-good-view latch, table talk, the
//! 10 s visible-tab poll, the connect-time resync and the review guard.
//!
//! It also holds the hand-review panel's loading state (`ui/HandReview.svelte`:
//! the opening rule, latest-request-wins and its copy) and the pre-view loading
//! copy of the play page.
//!
//! Nothing here does IO, sleeps or reads a clock. Time is injected as `now_ms`,
//! and every effect is returned as data: [`StoreCommand`]s the host must run
//! (each with a [`RequestId`] it reports back through `on_snapshot`,
//! `on_submit`, `on_review` or `on_hand_review`) and [`StoreEvent`]s that settle
//! what a TS `Promise` would have resolved.
//!
//! **Time.** Every `now_ms` is *monotonic* milliseconds on one store clock (for
//! example elapsed ms since a fixed `Instant`), never epoch time: TS uses
//! `setTimeout`, which a wall-clock step does not move. All deadlines fire from
//! [`TableStore::tick`]: the 4000 ms snapshot timeout, the 12 000 ms action
//! timeout of `submitMove`, `reviewTrick` and `getHandReview` (rivetkit
//! `actionDefaults.timeout`), the 10 s poll and the 6500 ms table-talk expiry.
//! [`TableStore::next_deadline_ms`] gives the earliest one, so a host can arm a
//! single timer and call `tick` when it fires.
//!
//! **Failures.** The web client runs every action with `throwOnError: false`, so
//! a timeout, a dropped connection, a transport error and a `UserError` all
//! resolve to `undefined`. A host matches it by reporting *every* failed call as
//! [`SubmitOutcome::NoAck`], `on_review(id, None)` and
//! [`HandReviewOutcome::Loaded`]`(None)`. The thrown branches
//! ([`SubmitOutcome::Failed`], [`HandReviewOutcome::Failed`]) are kept for
//! parity with the TS source but are unreachable in the web client.
//!
//! Methods named after the TS store (`bind`, `apply_sync`, `apply_chat`,
//! `resync`, `play`) keep its exact semantics; the page-level methods
//! (`attach`, `set_connected`, `on_sync_event`, `on_chat_event`, `submit_move`,
//! `review_trick`, `open_hand_review`, `choose_hand_review`, `tick`) add the
//! page's behaviour on top.

use euchre_engine::{LegalMoveId, PublicGameView, Seat, Step, Trick, apply_optimistic};

use crate::actions::{MoveAck, ReviewTrickAck, ReviewTrickRequest, SubmitMoveRequest};
use crate::events::{ChatEvent, ChatKind, SyncEvent};
use crate::hand_review::CompletedHandReview;

/// Shown when a move id is not in `view.legal`.
pub const MSG_ILLEGAL_MOVE: &str = "That move is not legal right now.";
/// Shown when a move was refused or its ack lost and the same turn is still current.
pub const MSG_COULD_NOT_CONFIRM: &str =
    "The table could not confirm your move. Your game is saved. Please try again.";
/// Shown when submitting threw and the same turn is still current. Unreachable
/// in the web client (`throwOnError: false`); see the module docs.
pub const MSG_CONNECTION_INTERRUPTED: &str =
    "Connection interrupted. Your game is saved. Reconnect and try again.";
/// Shown when a snapshot failed, was empty or timed out.
pub const MSG_WAITING_FOR_TABLE: &str = "Waiting for the table to reconnect. Your game is saved.";
/// Shown when `reviewTrick` failed or did not answer `{ queued: true }`.
pub const MSG_REVIEW_FAILED: &str = "Could not update the table. Please try again.";
/// The status notice while the transport is down and a view is on screen.
pub const MSG_RECONNECTING: &str =
    "Reconnecting… Your table is saved. Controls will return when you’re connected.";
/// Loading-screen heading before the first view, while connected.
pub const MSG_LOADING_CONNECTED_HEADING: &str = "Your hand is on its way.";
/// Loading-screen line before the first view, while connected.
pub const MSG_LOADING_CONNECTED_BODY: &str = "Dealing your cards…";
/// Loading-screen heading before the first view, while disconnected.
pub const MSG_LOADING_DISCONNECTED_HEADING: &str = "A seat at the table.";
/// Loading-screen line before the first view, while disconnected.
pub const MSG_LOADING_DISCONNECTED_BODY: &str = "Connecting to your game…";
/// Loading-screen error line when the transport reported an error and no
/// session error message is available.
pub const MSG_TABLE_OPEN_FAILED: &str =
    "We couldn’t open this table. Try reconnecting, or open Your saved games.";
/// The session-fetch error message when the failure carried no message.
pub const MSG_UNABLE_TO_OPEN_TABLE: &str = "Unable to open this table.";
/// Loading-screen button that reloads the page.
pub const MSG_RECONNECT_TO_GAME: &str = "Reconnect to this game";
/// Loading-screen link to the saved-games list.
pub const MSG_YOUR_SAVED_GAMES: &str = "Your saved games";
/// Hand-review status while a hand loads.
pub const MSG_OPENING_HAND: &str = "Opening the hand…";
/// Hand-review error when an earlier hand has no archive.
pub const MSG_OLDER_HAND_NOT_SAVED: &str = "This older hand was not saved by the previous version.";
/// Hand-review error when loading threw (unreachable in the web client).
pub const MSG_HAND_LOAD_FAILED: &str = "Could not load this hand. Please try again.";

/// How long a snapshot may take before it counts as failed (ms).
pub const SNAPSHOT_TIMEOUT_MS: u64 = 4000;
/// How long any other action may take before it resolves to nothing (ms):
/// rivetkit `actionDefaults.timeout` in `client/rivet.ts`.
pub const ACTION_TIMEOUT_MS: u64 = 12_000;
/// The visible-tab resync poll period (ms).
pub const RESYNC_POLL_MS: u64 = 10_000;
/// How long a banter bubble stays up (ms).
pub const TABLE_TALK_MS: u64 = 6500;
/// How many spoken lines the chat log keeps.
pub const CHAT_LOG_LEN: usize = 6;

/// The store's connection status (`TableStore.status`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StoreStatus {
    /// Not bound yet.
    Idle,
    /// Bound while the transport was down.
    Connecting,
    /// A view has been applied or the transport was up at bind.
    Ready,
    /// Declared by the TS store but never assigned.
    Error,
}

/// Correlates a [`StoreCommand`] with its reply.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct RequestId(pub u64);

/// Work the host must perform and report back.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StoreCommand {
    /// Call `snapshot()`; report with [`TableStore::on_snapshot`].
    FetchSnapshot {
        /// Reply correlation.
        id: RequestId,
    },
    /// Call `submitMove(request)`; report with [`TableStore::on_submit`].
    SubmitMove {
        /// Reply correlation.
        id: RequestId,
        /// The request to send.
        request: SubmitMoveRequest,
    },
    /// Call `reviewTrick(request)`; report with [`TableStore::on_review`].
    ReviewTrick {
        /// Reply correlation.
        id: RequestId,
        /// The request to send.
        request: ReviewTrickRequest,
    },
    /// Call `getHandReview(hand_no)`; report with [`TableStore::on_hand_review`].
    GetHandReview {
        /// Reply correlation.
        id: RequestId,
        /// The hand to load.
        hand_no: u32,
    },
}

/// Something that finished, in place of a resolved TS promise.
///
/// A settlement carries the [`RequestId`] of the command it settles, so merged
/// effects never mix a refusal up with the result of a request in flight.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StoreEvent {
    /// `play()` resolved for the `SubmitMove` command `id`: `true` when the
    /// move landed (including a lost ack).
    PlaySettled {
        /// The settled `SubmitMove` command.
        id: RequestId,
        /// The resolved value.
        ok: bool,
    },
    /// `play()` resolved `false` at once, without submitting (unbound, no view,
    /// already submitting, or an id not in `legal`). An in-flight play, if any,
    /// is unaffected.
    PlayRefused,
    /// A `resync()` finished (applied, ignored as older, failed or skipped).
    ResyncSettled {
        /// The `FetchSnapshot` command it settles, or `None` when it settled at
        /// once without fetching (unbound or disconnected).
        id: Option<RequestId>,
    },
    /// The page's `reviewTrick` resolved for the `ReviewTrick` command `id`.
    ReviewSettled {
        /// The settled `ReviewTrick` command.
        id: RequestId,
        /// The resolved value.
        ok: bool,
    },
    /// The page's `reviewTrick` resolved `false` at once (disconnected or a
    /// review already in flight). The in-flight review is unaffected.
    ReviewRefused,
}

/// The effects of one store call.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct StoreEffects {
    /// Commands to run, in order.
    pub commands: Vec<StoreCommand>,
    /// Settlements, in order.
    pub events: Vec<StoreEvent>,
}

impl StoreEffects {
    /// Whether nothing needs doing.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty() && self.events.is_empty()
    }

    /// Appends another call's effects.
    pub fn extend(&mut self, other: StoreEffects) {
        self.commands.extend(other.commands);
        self.events.extend(other.events);
    }

    /// The id of the `SubmitMove` command these effects issue, if any.
    pub fn submit_id(&self) -> Option<RequestId> {
        self.commands.iter().find_map(|c| match c {
            StoreCommand::SubmitMove { id, .. } => Some(*id),
            _ => None,
        })
    }

    /// The id of the `ReviewTrick` command these effects issue, if any.
    pub fn review_id(&self) -> Option<RequestId> {
        self.commands.iter().find_map(|c| match c {
            StoreCommand::ReviewTrick { id, .. } => Some(*id),
            _ => None,
        })
    }

    /// The resolved value of the play submitted as `id`, if these effects settle it.
    pub fn play_settled(&self, id: RequestId) -> Option<bool> {
        self.events.iter().find_map(|e| match e {
            StoreEvent::PlaySettled { id: got, ok } if *got == id => Some(*ok),
            _ => None,
        })
    }

    /// Whether these effects refuse a `play()` without submitting.
    pub fn play_refused(&self) -> bool {
        self.events.contains(&StoreEvent::PlayRefused)
    }

    /// The resolved value of the review requested as `id`, if these effects settle it.
    pub fn review_settled(&self, id: RequestId) -> Option<bool> {
        self.events.iter().find_map(|e| match e {
            StoreEvent::ReviewSettled { id: got, ok } if *got == id => Some(*ok),
            _ => None,
        })
    }

    /// Whether these effects refuse a `reviewTrick` without sending it.
    pub fn review_refused(&self) -> bool {
        self.events.contains(&StoreEvent::ReviewRefused)
    }

    fn command(&mut self, c: StoreCommand) {
        self.commands.push(c);
    }

    fn event(&mut self, e: StoreEvent) {
        self.events.push(e);
    }
}

/// How a `submitMove` call ended, as the host observed it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SubmitOutcome {
    /// The action returned a [`MoveAck`].
    Ack(MoveAck),
    /// The action resolved to nothing. With `throwOnError: false` this is how
    /// the web client sees *every* failure: a `UserError`, a transport error, a
    /// dropped connection or the 12 s timeout. Hosts report all of them here.
    NoAck,
    /// The call threw. TS-parity branch only: unreachable in the web client,
    /// and a host matching it never reports this.
    Failed,
}

/// How a `GetHandReview` command ended, as the host observed it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HandReviewOutcome {
    /// The action resolved: the archive, or `None` for no archive. As with
    /// [`SubmitOutcome::NoAck`], hosts report every failure as `Loaded(None)`.
    Loaded(Option<CompletedHandReview>),
    /// The call threw. TS-parity branch only (unreachable in the web client).
    Failed,
}

/// The hand-review panel's state (`HandReview.svelte`).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct HandReviewPanel {
    /// The hand chosen in the selector.
    pub hand_no: u32,
    /// Its archive, once loaded.
    pub archive: Option<CompletedHandReview>,
    /// Whether the latest load is in flight ([`MSG_OPENING_HAND`]).
    pub busy: bool,
    /// Error copy, if any.
    pub error: Option<&'static str>,
}

/// The play page's loading screen, shown until the first view exists.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LoadingNotice {
    /// The `h1`.
    pub heading: &'static str,
    /// The line under it.
    pub body: &'static str,
}

/// The loading screen's error line: shown when the transport reported an error
/// (`table.lastError`) or the session fetch failed with a non-empty message,
/// and reading the session message when there is one (`connectionError ??
/// MSG_TABLE_OPEN_FAILED`).
pub fn loading_error_line(transport_error: bool, session_error: Option<&str>) -> Option<&str> {
    let shown = transport_error || session_error.is_some_and(|e| !e.is_empty());
    shown.then(|| session_error.unwrap_or(MSG_TABLE_OPEN_FAILED))
}

/// The hand the review panel opens on: the previous hand when the current one
/// has no completed tricks and no result yet, else the current hand.
pub fn initial_review_hand(view: &PublicGameView) -> u32 {
    if view.hand_no > 0 && view.trick_log.is_empty() && view.result.is_none() {
        view.hand_no - 1
    } else {
        view.hand_no
    }
}

/// One spoken line in the chat log.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChatLine {
    /// Dedupe key.
    pub msg_id: String,
    /// The speaker.
    pub seat: Seat,
    /// Call, banter or system.
    pub kind: ChatKind,
    /// Plain text.
    pub text: String,
}

impl From<&ChatEvent> for ChatLine {
    fn from(e: &ChatEvent) -> Self {
        ChatLine {
            msg_id: e.msg_id.clone(),
            seat: e.seat,
            kind: e.kind,
            text: e.text.clone(),
        }
    }
}

/// A banter bubble, shown only while its hand is on screen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TableTalk {
    /// The speaker.
    pub seat: Seat,
    /// The line.
    pub text: String,
    /// The line's id.
    pub msg_id: String,
    /// `store.view?.handNo ?? null` when it arrived.
    pub hand_no: Option<u32>,
    /// Store-clock ms (monotonic, the same clock as `now_ms`) at which it disappears.
    pub until_ms: u64,
}

/// A local preview, valid only while the live view is still at `for_v`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OptimisticView {
    /// The live `v` it was computed from.
    pub for_v: u64,
    /// The previewed view.
    pub view: PublicGameView,
}

/// The view and move id of the human's latest decision (for the hint panel).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LastDecision {
    /// The view the decision was made on.
    pub view: PublicGameView,
    /// The chosen move.
    pub move_id: LegalMoveId,
}

/// What a `play()` does once the resync it is waiting on finishes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AfterResync {
    /// The ack was missing or `ok: false`.
    Unconfirmed,
    /// `submitMove` threw.
    Interrupted,
    /// `ok: true` but the store had not seen `ack.v` yet.
    Confirmed,
}

/// An in-flight request and the store-clock time it times out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Pending {
    id: RequestId,
    deadline_ms: u64,
}

/// Where an in-flight `play()` is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PlayStage {
    /// Waiting for the `submitMove` reply, until the action timeout.
    AwaitingAck(Pending),
    /// Waiting for a resync, then deciding.
    AwaitingResync(AfterResync),
}

/// An in-flight `play()`.
#[derive(Clone, PartialEq, Eq, Debug)]
struct PendingPlay {
    /// The `SubmitMove` command's id, carried by its settlement.
    id: RequestId,
    /// `view.turnId` captured before the submit.
    turn_id: String,
    /// Where it is.
    stage: PlayStage,
    /// Started by the page's `submit_move`, which clears the preview on failure.
    from_page: bool,
}

/// The client store for one table (see the module docs).
#[derive(Clone, Debug)]
pub struct TableStore {
    view: Option<PublicGameView>,
    status: StoreStatus,
    error: Option<String>,
    submitting: bool,
    chat: Vec<ChatLine>,
    bound: bool,
    connected: bool,
    snapshot: Option<Pending>,
    play: Option<PendingPlay>,
    next_id: u64,
    // Page-level state.
    last_steps: Vec<Step>,
    optimistic: Option<OptimisticView>,
    last_good_view: Option<PublicGameView>,
    last_decision: Option<LastDecision>,
    table_talk: Option<TableTalk>,
    review: Option<Pending>,
    review_error: Option<String>,
    next_poll_ms: Option<u64>,
    hand_review: HandReviewPanel,
    hand_review_request: Option<Pending>,
}

impl Default for TableStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TableStore {
    /// An unbound store with no view.
    pub fn new() -> Self {
        TableStore {
            view: None,
            status: StoreStatus::Idle,
            error: None,
            submitting: false,
            chat: Vec::new(),
            bound: false,
            connected: false,
            snapshot: None,
            play: None,
            next_id: 0,
            last_steps: Vec::new(),
            optimistic: None,
            last_good_view: None,
            last_decision: None,
            table_talk: None,
            review: None,
            review_error: None,
            next_poll_ms: None,
            hand_review: HandReviewPanel::default(),
            hand_review_request: None,
        }
    }

    /* ------------------------------------------------------------------ */
    /* Reads                                                                */
    /* ------------------------------------------------------------------ */

    /// The latest accepted server view.
    pub fn view(&self) -> Option<&PublicGameView> {
        self.view.as_ref()
    }

    /// The connection status.
    pub fn status(&self) -> StoreStatus {
        self.status
    }

    /// User-facing error copy, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether a `submitMove` is in flight.
    pub fn submitting(&self) -> bool {
        self.submitting
    }

    /// The last [`CHAT_LOG_LEN`] spoken lines.
    pub fn chat(&self) -> &[ChatLine] {
        &self.chat
    }

    /// Whether the transport is up, as last reported.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Whether a snapshot is in flight.
    pub fn is_resyncing(&self) -> bool {
        self.snapshot.is_some()
    }

    /// The steps of the last `sync` event (not updated by snapshots).
    pub fn last_steps(&self) -> &[Step] {
        &self.last_steps
    }

    /// The current optimistic preview, if any (shown only while `for_v` is live).
    pub fn optimistic(&self) -> Option<&OptimisticView> {
        self.optimistic.as_ref()
    }

    /// The human's latest decision.
    pub fn last_decision(&self) -> Option<&LastDecision> {
        self.last_decision.as_ref()
    }

    /// What the table renders: the preview while it matches the live `v`, else
    /// the live view, else the last good frame (the table never blanks).
    pub fn display_view(&self) -> Option<&PublicGameView> {
        match &self.view {
            None => self.last_good_view.as_ref(),
            Some(live) => match &self.optimistic {
                Some(o) if o.for_v == live.v => Some(&o.view),
                _ => Some(live),
            },
        }
    }

    /// The banter bubble, only while its hand is the displayed hand.
    pub fn visible_table_talk(&self) -> Option<&TableTalk> {
        let talk = self.table_talk.as_ref()?;
        let hand_no = self.display_view()?.hand_no;
        (talk.hand_no == Some(hand_no)).then_some(talk)
    }

    /// Whether move controls are enabled (`!submitting && isConnected`).
    pub fn controls_enabled(&self) -> bool {
        !self.submitting && self.connected
    }

    /// The reconnecting notice while the transport is down. Shown only over a
    /// view; before the first view the page shows [`Self::loading_notice`].
    pub fn reconnect_notice(&self) -> Option<&'static str> {
        (!self.connected && self.display_view().is_some()).then_some(MSG_RECONNECTING)
    }

    /// The loading screen's copy until the first view exists (`None` after).
    /// Its error line comes from [`loading_error_line`], with the buttons
    /// [`MSG_RECONNECT_TO_GAME`] and [`MSG_YOUR_SAVED_GAMES`].
    pub fn loading_notice(&self) -> Option<LoadingNotice> {
        if self.display_view().is_some() {
            return None;
        }
        Some(if self.connected {
            LoadingNotice {
                heading: MSG_LOADING_CONNECTED_HEADING,
                body: MSG_LOADING_CONNECTED_BODY,
            }
        } else {
            LoadingNotice {
                heading: MSG_LOADING_DISCONNECTED_HEADING,
                body: MSG_LOADING_DISCONNECTED_BODY,
            }
        })
    }

    /// Whether a `reviewTrick` is in flight.
    pub fn review_busy(&self) -> bool {
        self.review.is_some()
    }

    /// Review failure copy, if any.
    pub fn review_error(&self) -> Option<&str> {
        self.review_error.as_deref()
    }

    /// The hand-review panel.
    pub fn hand_review(&self) -> &HandReviewPanel {
        &self.hand_review
    }

    /// The tricks the review panel lists: the archive's, else the live
    /// `trickLog` when the chosen hand is the displayed hand, else none.
    pub fn hand_review_tricks(&self) -> &[Trick] {
        if let Some(archive) = &self.hand_review.archive {
            return &archive.tricks;
        }
        match self.display_view() {
            Some(v) if v.hand_no == self.hand_review.hand_no => &v.trick_log,
            _ => &[],
        }
    }

    /// Tricks per team for the review panel: the archive's `tricksWon`, else a
    /// count of the listed tricks by winner team.
    pub fn hand_review_totals(&self) -> [u8; 2] {
        if let Some(won) = self.hand_review.archive.as_ref().and_then(|a| a.tricks_won) {
            return won;
        }
        let mut totals = [0u8; 2];
        for trick in self.hand_review_tricks() {
            if let Some(winner) = trick.winner_seat {
                let team = winner.index() % 2;
                totals[team] = totals[team].saturating_add(1);
            }
        }
        totals
    }

    /// The earliest store-clock time at which [`Self::tick`] has work: a
    /// snapshot, action or hand-review timeout, the table-talk expiry or the
    /// next poll. Arm one timer for it.
    pub fn next_deadline_ms(&self) -> Option<u64> {
        let ack = self.play.as_ref().and_then(|p| match p.stage {
            PlayStage::AwaitingAck(pending) => Some(pending.deadline_ms),
            PlayStage::AwaitingResync(_) => None,
        });
        [
            self.snapshot.map(|p| p.deadline_ms),
            ack,
            self.review.map(|p| p.deadline_ms),
            self.hand_review_request.map(|p| p.deadline_ms),
            self.table_talk.as_ref().map(|t| t.until_ms),
            self.next_poll_ms,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /* ------------------------------------------------------------------ */
    /* TS TableStore                                                        */
    /* ------------------------------------------------------------------ */

    /// `bind(handle)`: remember the transport; `Ready` if it is up, else `Connecting`.
    /// Arms the 10 s poll from `now_ms`. Issues nothing.
    pub fn bind(&mut self, connected: bool, now_ms: u64) {
        self.bound = true;
        self.connected = connected;
        self.status = if connected {
            StoreStatus::Ready
        } else {
            StoreStatus::Connecting
        };
        self.next_poll_ms = Some(now_ms.saturating_add(RESYNC_POLL_MS));
    }

    /// `applySync`: drop a view older than the current one (equal `v` is accepted).
    pub fn apply_sync(&mut self, payload: &SyncEvent) {
        if let Some(view) = &self.view
            && payload.view.v < view.v
        {
            return;
        }
        self.view = Some(payload.view.clone());
        self.status = StoreStatus::Ready;
        self.error = None;
        self.latch();
    }

    /// `applyChat`: ignore a redelivered `msgId`; keep the last six lines.
    pub fn apply_chat(&mut self, line: ChatLine) {
        if self.chat.iter().any(|m| m.msg_id == line.msg_id) {
            return;
        }
        self.chat.push(line);
        if self.chat.len() > CHAT_LOG_LEN {
            let excess = self.chat.len() - CHAT_LOG_LEN;
            self.chat.drain(..excess);
        }
    }

    /// `resync()`: join the in-flight snapshot, or fetch one when connected.
    /// When the transport is down it settles at once and changes nothing.
    pub fn resync(&mut self, now_ms: u64) -> StoreEffects {
        let mut fx = StoreEffects::default();
        if self.snapshot.is_some() {
            return fx;
        }
        if !self.bound || !self.connected {
            self.resync_settled(None, &mut fx);
            return fx;
        }
        let id = self.next_request_id();
        self.snapshot = Some(Pending {
            id,
            deadline_ms: now_ms.saturating_add(SNAPSHOT_TIMEOUT_MS),
        });
        fx.command(StoreCommand::FetchSnapshot { id });
        fx
    }

    /// `play(moveId)` with the caller's fresh `clientMoveId` (a UUID).
    ///
    /// Emits [`StoreEvent::PlayRefused`] at once when unbound, viewless, already
    /// submitting or when the id is not in `view.legal`; otherwise issues
    /// `SubmitMove` against the current `turnId`, timing out after
    /// [`ACTION_TIMEOUT_MS`].
    pub fn play(
        &mut self,
        move_id: &str,
        client_move_id: impl Into<String>,
        now_ms: u64,
    ) -> StoreEffects {
        self.start_play(move_id, client_move_id.into(), false, now_ms)
    }

    /// Reports a snapshot reply (`None` for an empty or failed snapshot).
    /// A reply for a request that already timed out or was superseded is ignored.
    pub fn on_snapshot(
        &mut self,
        id: RequestId,
        snapshot: Option<PublicGameView>,
        now_ms: u64,
    ) -> StoreEffects {
        let mut fx = StoreEffects::default();
        let Some(pending) = self.snapshot else {
            return fx;
        };
        if pending.id != id {
            return fx;
        }
        self.snapshot = None;
        match snapshot {
            Some(snap) if now_ms < pending.deadline_ms => {
                if self.view.as_ref().is_none_or(|v| snap.v >= v.v) {
                    self.view = Some(snap);
                    self.status = StoreStatus::Ready;
                    self.error = None;
                    self.latch();
                }
            }
            _ => self.error = Some(MSG_WAITING_FOR_TABLE.to_owned()),
        }
        self.resync_settled(Some(id), &mut fx);
        fx
    }

    /// Reports how a `SubmitMove` command ended. A reply after the action
    /// timeout, or for another request, is ignored.
    pub fn on_submit(
        &mut self,
        id: RequestId,
        outcome: SubmitOutcome,
        now_ms: u64,
    ) -> StoreEffects {
        let mut fx = StoreEffects::default();
        let Some(play) = &mut self.play else {
            return fx;
        };
        if !matches!(play.stage, PlayStage::AwaitingAck(p) if p.id == id) {
            return fx;
        }
        let after = match outcome {
            SubmitOutcome::Ack(MoveAck::Ok { v }) => {
                if self.view.as_ref().is_some_and(|view| view.v >= v) {
                    self.settle_play(true, &mut fx);
                    return fx;
                }
                AfterResync::Confirmed
            }
            SubmitOutcome::Ack(MoveAck::Rejected { .. }) | SubmitOutcome::NoAck => {
                AfterResync::Unconfirmed
            }
            SubmitOutcome::Failed => AfterResync::Interrupted,
        };
        play.stage = PlayStage::AwaitingResync(after);
        fx.extend(self.resync(now_ms));
        fx
    }

    /* ------------------------------------------------------------------ */
    /* Page wiring                                                          */
    /* ------------------------------------------------------------------ */

    /// Page mount: `bind`, then the `isConnected` effect's initial resync.
    pub fn attach(&mut self, connected: bool, now_ms: u64) -> StoreEffects {
        self.bind(connected, now_ms);
        if connected {
            self.resync(now_ms)
        } else {
            StoreEffects::default()
        }
    }

    /// The transport came up or went down; coming up triggers a resync.
    pub fn set_connected(&mut self, connected: bool, now_ms: u64) -> StoreEffects {
        let rising = connected && !self.connected;
        self.connected = connected;
        if rising {
            self.resync(now_ms)
        } else {
            StoreEffects::default()
        }
    }

    /// The `sync` event handler: `applySync`, then record its steps even when the
    /// view was dropped as older.
    pub fn on_sync_event(&mut self, payload: SyncEvent) {
        self.apply_sync(&payload);
        self.last_steps = payload.steps;
    }

    /// The `chat` event handler: `applyChat`, and a banter line (even a
    /// redelivered one) becomes the table-talk bubble for [`TABLE_TALK_MS`].
    pub fn on_chat_event(&mut self, payload: &ChatEvent, now_ms: u64) {
        self.apply_chat(ChatLine::from(payload));
        if payload.kind == ChatKind::Banter {
            self.table_talk = Some(TableTalk {
                seat: payload.seat,
                text: payload.text.clone(),
                msg_id: payload.msg_id.clone(),
                hand_no: self.view.as_ref().map(|v| v.hand_no),
                until_ms: now_ms.saturating_add(TABLE_TALK_MS),
            });
        }
    }

    /// The page's `submitMove`: guard on the displayed view, `submitting` and the
    /// transport; find the entry in the displayed `legal`; record the decision;
    /// preview it with `applyOptimistic`; then `play`. A failed play clears the
    /// preview. Returns empty effects when a guard refuses (the TS returns early).
    pub fn submit_move(
        &mut self,
        move_id: &str,
        client_move_id: impl Into<String>,
        now_ms: u64,
    ) -> StoreEffects {
        let Some(view) = self.display_view() else {
            return StoreEffects::default();
        };
        if self.submitting || !self.connected {
            return StoreEffects::default();
        }
        let Some(entry) = view.legal.iter().find(|m| m.id == move_id) else {
            return StoreEffects::default();
        };
        let preview = apply_optimistic(view, &entry.mv);
        let decision = LastDecision {
            view: view.clone(),
            move_id: move_id.to_owned(),
        };
        let for_v = view.v;
        self.last_decision = Some(decision);
        self.optimistic = Some(OptimisticView {
            for_v,
            view: preview,
        });
        self.latch();
        self.start_play(move_id, client_move_id.into(), true, now_ms)
    }

    /// The page's `reviewTrick`: guarded by the transport and `reviewBusy`
    /// ([`StoreEvent::ReviewRefused`]); times out after [`ACTION_TIMEOUT_MS`].
    pub fn review_trick(&mut self, request: ReviewTrickRequest, now_ms: u64) -> StoreEffects {
        let mut fx = StoreEffects::default();
        if !self.connected || self.review.is_some() {
            fx.event(StoreEvent::ReviewRefused);
            return fx;
        }
        let id = self.next_request_id();
        self.review = Some(Pending {
            id,
            deadline_ms: now_ms.saturating_add(ACTION_TIMEOUT_MS),
        });
        self.review_error = None;
        fx.command(StoreCommand::ReviewTrick { id, request });
        fx
    }

    /// Reports a `reviewTrick` reply; `None` for a failure or a missing ack.
    /// A reply after the action timeout, or for another request, is ignored.
    pub fn on_review(&mut self, id: RequestId, ack: Option<ReviewTrickAck>) -> StoreEffects {
        let mut fx = StoreEffects::default();
        if self.review.is_none_or(|p| p.id != id) {
            return fx;
        }
        self.review = None;
        let ok = ack.is_some();
        if !ok {
            self.review_error = Some(MSG_REVIEW_FAILED.to_owned());
        }
        fx.event(StoreEvent::ReviewSettled { id, ok });
        fx
    }

    /// Opens the hand-review panel on [`initial_review_hand`] of the displayed
    /// view. Does nothing before the first view (the panel is not rendered).
    pub fn open_hand_review(&mut self, now_ms: u64) -> StoreEffects {
        let Some(view) = self.display_view() else {
            return StoreEffects::default();
        };
        let hand_no = initial_review_hand(view);
        self.choose_hand_review(hand_no, now_ms)
    }

    /// Selects a hand in the review panel and loads it. The latest choice wins:
    /// replies to earlier choices are ignored.
    pub fn choose_hand_review(&mut self, hand_no: u32, now_ms: u64) -> StoreEffects {
        let mut fx = StoreEffects::default();
        let id = self.next_request_id();
        self.hand_review = HandReviewPanel {
            hand_no,
            archive: None,
            busy: true,
            error: None,
        };
        self.hand_review_request = Some(Pending {
            id,
            deadline_ms: now_ms.saturating_add(ACTION_TIMEOUT_MS),
        });
        fx.command(StoreCommand::GetHandReview { id, hand_no });
        fx
    }

    /// Reports a `getHandReview` reply. Only the latest request applies. No
    /// archive for a hand other than the displayed one reads
    /// [`MSG_OLDER_HAND_NOT_SAVED`]; a thrown call reads [`MSG_HAND_LOAD_FAILED`].
    pub fn on_hand_review(&mut self, id: RequestId, outcome: HandReviewOutcome) {
        if self.hand_review_request.is_none_or(|p| p.id != id) {
            return;
        }
        self.hand_review_request = None;
        self.hand_review.busy = false;
        match outcome {
            HandReviewOutcome::Loaded(archive) => {
                let current = self.display_view().map(|v| v.hand_no);
                if archive.is_none() && current != Some(self.hand_review.hand_no) {
                    self.hand_review.error = Some(MSG_OLDER_HAND_NOT_SAVED);
                }
                self.hand_review.archive = archive;
            }
            HandReviewOutcome::Failed => self.hand_review.error = Some(MSG_HAND_LOAD_FAILED),
        }
    }

    /// Advances time: times out a slow snapshot, a `submitMove` (as
    /// [`SubmitOutcome::NoAck`]), a `reviewTrick` (as no ack) and a
    /// `getHandReview` (as no archive), expires table talk, and runs the 10 s
    /// poll (a resync only while connected and `visible`).
    pub fn tick(&mut self, now_ms: u64, visible: bool) -> StoreEffects {
        let mut fx = StoreEffects::default();
        if let Some(pending) = self.snapshot
            && now_ms >= pending.deadline_ms
        {
            fx.extend(self.on_snapshot(pending.id, None, now_ms));
        }
        if let Some(PlayStage::AwaitingAck(pending)) = self.play.as_ref().map(|p| p.stage)
            && now_ms >= pending.deadline_ms
        {
            fx.extend(self.on_submit(pending.id, SubmitOutcome::NoAck, now_ms));
        }
        if let Some(pending) = self.review
            && now_ms >= pending.deadline_ms
        {
            fx.extend(self.on_review(pending.id, None));
        }
        if let Some(pending) = self.hand_review_request
            && now_ms >= pending.deadline_ms
        {
            self.on_hand_review(pending.id, HandReviewOutcome::Loaded(None));
        }
        if self
            .table_talk
            .as_ref()
            .is_some_and(|t| now_ms >= t.until_ms)
        {
            self.table_talk = None;
        }
        if let Some(mut next) = self.next_poll_ms
            && now_ms >= next
        {
            while next <= now_ms {
                next = next.saturating_add(RESYNC_POLL_MS);
            }
            self.next_poll_ms = Some(next);
            if self.connected && visible {
                fx.extend(self.resync(now_ms));
            }
        }
        fx
    }

    /* ------------------------------------------------------------------ */
    /* Internals                                                            */
    /* ------------------------------------------------------------------ */

    fn next_request_id(&mut self) -> RequestId {
        self.next_id += 1;
        RequestId(self.next_id)
    }

    /// The `lastGoodView` effect: remember every non-null displayed frame.
    fn latch(&mut self) {
        if let Some(v) = self.display_view() {
            let v = v.clone();
            self.last_good_view = Some(v);
        }
    }

    fn start_play(
        &mut self,
        move_id: &str,
        client_move_id: String,
        from_page: bool,
        now_ms: u64,
    ) -> StoreEffects {
        let mut fx = StoreEffects::default();
        let Some(view) = &self.view else {
            self.fail_page_preview(from_page);
            fx.event(StoreEvent::PlayRefused);
            return fx;
        };
        if !self.bound || self.submitting {
            self.fail_page_preview(from_page);
            fx.event(StoreEvent::PlayRefused);
            return fx;
        }
        if !view.legal.iter().any(|m| m.id == move_id) {
            self.error = Some(MSG_ILLEGAL_MOVE.to_owned());
            self.fail_page_preview(from_page);
            fx.event(StoreEvent::PlayRefused);
            return fx;
        }
        let turn_id = view.turn_id.clone();
        let id = self.next_request_id();
        self.submitting = true;
        self.error = None;
        self.play = Some(PendingPlay {
            id,
            turn_id: turn_id.clone(),
            stage: PlayStage::AwaitingAck(Pending {
                id,
                deadline_ms: now_ms.saturating_add(ACTION_TIMEOUT_MS),
            }),
            from_page,
        });
        fx.command(StoreCommand::SubmitMove {
            id,
            request: SubmitMoveRequest {
                move_id: move_id.to_owned(),
                turn_id: Some(turn_id),
                client_move_id,
            },
        });
        fx
    }

    /// A page play failed: drop the preview and re-latch the frame now shown
    /// (the TS `lastGoodView` effect re-runs whenever `displayView` changes).
    fn fail_page_preview(&mut self, from_page: bool) {
        if from_page {
            self.optimistic = None;
            self.latch();
        }
    }

    /// A resync finished: settle it, then continue a play waiting on it.
    fn resync_settled(&mut self, id: Option<RequestId>, fx: &mut StoreEffects) {
        fx.event(StoreEvent::ResyncSettled { id });
        let Some(play) = &self.play else {
            return;
        };
        let PlayStage::AwaitingResync(after) = play.stage else {
            return;
        };
        let landed = self
            .view
            .as_ref()
            .is_some_and(|v| v.turn_id != play.turn_id);
        match after {
            AfterResync::Confirmed => self.settle_play(true, fx),
            _ if landed => self.settle_play(true, fx),
            AfterResync::Unconfirmed => {
                self.error = Some(MSG_COULD_NOT_CONFIRM.to_owned());
                self.settle_play(false, fx);
            }
            AfterResync::Interrupted => {
                self.error = Some(MSG_CONNECTION_INTERRUPTED.to_owned());
                self.settle_play(false, fx);
            }
        }
    }

    fn settle_play(&mut self, ok: bool, fx: &mut StoreEffects) {
        let Some(play) = self.play.take() else {
            return;
        };
        self.submitting = false;
        if !ok {
            self.fail_page_preview(play.from_page);
        }
        fx.event(StoreEvent::PlaySettled { id: play.id, ok });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::SyncEvent;

    /// The TS fixture's lead view: seat 0 to play, with a hand and a legal set.
    fn store_test_view() -> PublicGameView {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/view.json")).expect("view.json");
        let view: PublicGameView =
            serde_json::from_value(fixtures["tableView"].clone()).expect("tableView");
        assert!(!view.legal.is_empty() && !view.hand.is_empty());
        view
    }

    #[test]
    fn a_refused_submit_relatches_the_live_view_not_the_rejected_preview() {
        let live = store_test_view();
        let move_id = live.legal[0].id.clone();
        let mut store = TableStore::new();
        store.bind(true, 0);
        store.apply_sync(&SyncEvent {
            v: live.v,
            view: live.clone(),
            steps: vec![],
        });
        let fx = store.submit_move(&move_id, "c", 0);
        let id = fx.submit_id().expect("submitted");
        assert!(
            store.display_view().is_some_and(|v| v.legal.is_empty()),
            "the preview is on screen"
        );
        let rejected = MoveAck::Rejected {
            code: crate::errors::ErrorCode::Rule(euchre_engine::RuleCode::StaleTurn),
            legal: vec![],
        };
        let fx = store.on_submit(id, SubmitOutcome::Ack(rejected), 10);
        let StoreCommand::FetchSnapshot { id: snap } = fx.commands[0] else {
            panic!("expected a resync");
        };
        let fx = store.on_snapshot(snap, Some(live.clone()), 20);
        assert_eq!(fx.play_settled(id), Some(false));
        assert!(store.optimistic().is_none());

        // Force the live view away, as a future path that clears it would.
        store.view = None;
        assert_eq!(
            store.display_view(),
            Some(&live),
            "the latch holds the live frame, not the guessed preview"
        );
    }

    #[test]
    fn a_play_refused_while_submitting_relatches_too() {
        let live = store_test_view();
        let move_id = live.legal[0].id.clone();
        let mut store = TableStore::new();
        store.bind(true, 0);
        store.apply_sync(&SyncEvent {
            v: live.v,
            view: live.clone(),
            steps: vec![],
        });
        // A page preview whose play is refused at once (the store is unbound
        // from the page's point of view: `start_play` refuses when submitting).
        store.submitting = true;
        store.connected = true;
        store.optimistic = Some(OptimisticView {
            for_v: live.v,
            view: apply_optimistic(&live, &live.legal[0].mv),
        });
        store.latch();
        let fx = store.start_play(&move_id, "c".to_owned(), true, 0);
        assert!(fx.play_refused());
        store.view = None;
        assert_eq!(store.display_view(), Some(&live));
    }
}
