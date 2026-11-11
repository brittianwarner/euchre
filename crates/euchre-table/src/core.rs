//! The table's rules of motion: every TS handler of `euchre-table/index.ts`
//! (03 §8), ported over a [`Host`] so they run the same against the real actor
//! and against the unit-test host.
//!
//! A [`Session`] is one lane turn. It works on a copy of the live state;
//! [`Session::save`] persists the copy (state row plus any new hand reviews, in
//! one SQLite batch) and only then publishes it as the live state. Events go
//! out after the save that made them true (persist, then emit, then ack, 03 §13).
//!
//! The functions keep the TS names and order. The Rust-only additions are the
//! due-time [`crate::state::Ledger`] (re-armed on every wake, D20) and the
//! early-fire guard of the schedule targets ([`fire`], 03 §16.5).

use anyhow::{Result, bail};
use async_trait::async_trait;
use euchre_ai::{RankParams, top_move};
use euchre_engine::{
    ActionKind, AiDecisionSource, ApplyError, ApplyOptions, AutoReason, GamePhase, GameState,
    GameStatus, HUMAN_SEAT, HandState, LegalMove, Move, ObservationTiming, PendingDecision,
    PlayerAction, PublicGameView, Seat, Step, TableObservation, TableRules, legal_moves, project,
    project_steps, try_advance, try_apply,
};
use euchre_protocol::CompletedHandReview;
use euchre_protocol::table_actions::{
    FLUSH_PROFILE, ON_AI_TIMEOUT, ON_REAP, ON_TEMPO_GATE, RELEASE_AI_MOVE,
};
use euchre_protocol::{
    AIDecideRequest, AIDecision, AIDecisionKind, AiSayMsg, AiSeatLifecycleKind,
    AiSeatLifecycleMessage, BANTER_MAX_CHARS, ChatDeltaEvent, ChatEvent, ChatKind, ErrorCode,
    HandJournalEntry, MatchOutcome, MatchRecord, MoveAck, MoveMsg, PersonaAssignment,
    ProtocolErrorCode, ReviewAction, ReviewMsg, RuleCode, SchemaV1, SyncEvent, ThinkingEvent,
    TickKind, TickMsg, TokenUsage, True, ai_seat_key, build_hand_review, table_players,
};
use euchre_protocol::{IllegalMoveMetadata, ReviewTrickRequest, SubmitMoveRequest};
use euchre_server_core::auth::internal::match_token_ok;
use euchre_server_core::rivet::{user_error, user_error_with_metadata};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::seams::{
    AiDispatch, DecideCall, LifecycleCall, ProfileSink, RecordGameCall, RecordHandsCall,
};
use crate::state::{
    CLIENT_MOVE_ID_UNITS, JOURNAL_CAP, MOVE_ID_RING, OBSERVATIONS_CAP, TableState, TurnDue,
    TurnOpened, accumulate_stats,
};
use crate::tempo::Tempo;
use crate::text::{
    decision_kind_for, fence_nonce, log_excerpt, observation_kind, screen_line, utf16_prefix,
};

/// A schedule that fires this much early still counts as due (clock skew).
pub const EARLY_FIRE_TOLERANCE_MS: u64 = 25;
/// `settle` gives up after this many automatic advances (a reducer fault).
const SETTLE_LIMIT: usize = 64;
/// The longest a human's think time counts as observed (TS `20_000`).
const OBSERVED_MAX_MS: u64 = 20_000;
/// Observed think times are rounded to this (TS `Math.round(elapsed / 250) * 250`).
const OBSERVED_ROUNDING_MS: u64 = 250;

/// What the core needs from its actor: a clock, timers, storage, fan-out and the
/// two outbound seams.
#[async_trait]
pub trait Host: Send + Sync {
    /// Epoch milliseconds.
    fn now_ms(&self) -> u64;
    /// `ctx.schedule().after(delay, action, args)`; the schedule id.
    async fn schedule(
        &self,
        delay_ms: u64,
        action: &'static str,
        args: Vec<Value>,
    ) -> Result<String>;
    /// Cancel a schedule. A failure is logged, never raised (TS ignored the result).
    async fn cancel(&self, id: &str);
    /// Persist `state` and `reviews` atomically, then make `state` the live state.
    async fn persist(&self, state: &TableState, reviews: &[CompletedHandReview]) -> Result<()>;
    /// `ctx.broadcast(name, payload)`.
    fn broadcast(&self, name: &str, payload: Value);
    /// Send `sync` to every player connection, each with its own payload.
    fn send_sync(&self, build: &(dyn Fn(Seat) -> Option<Value> + Sync));
    /// Whether any player connection is live.
    fn player_connected(&self) -> bool;
    /// `ctx.destroy()`.
    fn destroy(&self);
    /// The AI seam.
    fn ai(&self) -> &dyn AiDispatch;
    /// The profile seam.
    fn profile(&self) -> &dyn ProfileSink;
}

/// One lane turn: a working copy of the state, the reviews to archive with the
/// next save, and the host.
pub struct Session<'h, H: Host + ?Sized> {
    /// The working copy. Published only by [`Session::save`].
    pub st: TableState,
    saved: TableState,
    reviews: Vec<CompletedHandReview>,
    /// How many saves this turn made (a failure after one is a partial commit).
    saves: u32,
    /// The actor (or test) host.
    pub host: &'h H,
    /// The pacing profile.
    pub tempo: Tempo,
}

/// How an applied move is attributed in the journal (TS `Attribution`).
#[derive(Clone, Copy, Debug)]
enum Attribution {
    Human,
    Ai(AiDecisionSource),
    Auto(AutoReason),
}

/// One call a hand boundary owes other actors.
#[derive(Debug)]
enum BoundaryCall {
    /// `recordHands` for the owner's profile.
    Hand(RecordHandsCall),
    /// `handEnd` / `resetHand` for every AI seat.
    Notify(AiSeatLifecycleKind, Option<u32>),
}

/// What a hand boundary owes, sent after the save that records it.
#[derive(Debug, Default)]
#[must_use]
struct Boundary(Vec<BoundaryCall>);

impl Boundary {
    fn push_hand(&mut self, call: Option<RecordHandsCall>) {
        if let Some(call) = call {
            self.0.push(BoundaryCall::Hand(call));
        }
    }
}

fn apply_error(e: ApplyError) -> anyhow::Error {
    match e {
        ApplyError::Rule(r) => anyhow::Error::new(r),
        ApplyError::Fault(f) => anyhow::Error::new(f),
    }
}

/// TS `commit(c, state)`: `v` is the table's counter, one up per commit.
fn commit(st: &mut TableState, next: GameState) {
    let v = st.game.v;
    st.game = next;
    st.game.v = v + 1;
}

fn is_ai_seat(seat: Seat) -> bool {
    seat != HUMAN_SEAT
}

fn review_ready(st: &TableState) -> bool {
    matches!(&st.review_ready_turn_id, Some(Some(t)) if *t == st.game.turn_id)
}

/// TS `tableView(c, seat)`: the seat's projection plus the actor metadata.
pub fn table_view(st: &TableState, seat: Seat) -> PublicGameView {
    let game = &st.game;
    let mut view = project(game, seat);
    view.players = Some(table_players(&st.personas));
    view.observations = Some(
        st.observations
            .iter()
            .filter(|o| o.hand_no == game.hand.hand_no)
            .cloned()
            .collect(),
    );
    view.review_tricks = Some(st.review_tricks.unwrap_or(false));
    view.rules = Some(TableRules {
        stick_the_dealer: game.cfg.stick_the_dealer,
        require_natural_trump: game.cfg.require_natural_trump.unwrap_or(false),
    });
    view.awaiting_trick_review =
        Some(game.hand.phase == GamePhase::TrickResolve && review_ready(st));
    view
}

/// The `sync` payload for `seat` (TS `pushSync` body): `{ v, view, steps }`.
pub fn sync_payload(st: &TableState, seat: Seat, steps: &[Step]) -> Option<Value> {
    let event = SyncEvent {
        v: st.game.v,
        view: table_view(st, seat),
        steps: project_steps(steps, seat),
    };
    match serde_json::to_value(&event) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::error!(err = %e, "sync payload failed to serialize");
            None
        }
    }
}

/// TS `aiSeatCreateInput`: the seat's create input, keys in TS order.
pub fn ai_seat_create_input(st: &TableState, assignment: &PersonaAssignment) -> Value {
    let game = &st.game;
    let mut m = Map::new();
    m.insert("gameId".into(), json!(game.game_id));
    m.insert("seat".into(), json!(assignment.seat.get()));
    m.insert("internalToken".into(), json!(game.internal_token));
    m.insert("fenceNonce".into(), json!(fence_nonce(&st.match_id)));
    m.insert(
        "persona".into(),
        serde_json::to_value(&assignment.persona).unwrap_or(Value::Null),
    );
    m.insert("dossier".into(), json!(assignment.dossier));
    m.insert("matchId".into(), json!(st.match_id));
    if !st.owner_user_id.is_empty() {
        m.insert("profileKey".into(), json!(["user", st.profile_user()]));
    }
    Value::Object(m)
}

fn outcome_of(game: &GameState) -> MatchOutcome {
    match game.winner_team.map(|t| t.get()) {
        Some(0) => MatchOutcome::Won,
        Some(1) => MatchOutcome::Lost,
        _ => MatchOutcome::Abandoned,
    }
}

impl<'h, H: Host + ?Sized> Session<'h, H> {
    /// A turn over `st` (normally a clone of the live state).
    pub fn new(host: &'h H, st: TableState, tempo: Tempo) -> Self {
        Session {
            saved: st.clone(),
            st,
            reviews: Vec::new(),
            saves: 0,
            host,
            tempo,
        }
    }

    /// Whether this turn has persisted anything. A turn that fails after its
    /// first save left the live state advanced past what started it, so a
    /// same-args retry of its trigger is stale: the caller recovers with
    /// [`Session::reconcile`] in a fresh turn instead (D20).
    pub fn committed(&self) -> bool {
        self.saves > 0
    }

    /// The last persisted copy (the live state).
    pub fn saved(&self) -> &TableState {
        &self.saved
    }

    /// Drop the unsaved edits and reviews: the working copy goes back to the
    /// last persisted state.
    pub fn rollback(&mut self) {
        self.st = self.saved.clone();
        self.reviews.clear();
    }

    /// Persist the working copy and the pending reviews, then publish (TS
    /// `saveState({ immediate: true })`).
    pub async fn save(&mut self) -> Result<()> {
        self.host.persist(&self.st, &self.reviews).await?;
        self.saved = self.st.clone();
        self.reviews.clear();
        self.saves += 1;
        Ok(())
    }

    /// Save only when something changed since the last save.
    pub async fn flush(&mut self) -> Result<()> {
        if self.st != self.saved || !self.reviews.is_empty() {
            self.save().await?;
        }
        Ok(())
    }

    fn now(&self) -> u64 {
        self.host.now_ms()
    }

    // ---------------------------------------------------------------- fan-out

    fn broadcast<T: Serialize>(&self, name: &str, payload: &T) {
        match serde_json::to_value(payload) {
            Ok(v) => self.host.broadcast(name, v),
            Err(e) => tracing::error!(name, err = %e, "event failed to serialize"),
        }
    }

    fn thinking(&self, seat: Seat, on: bool) {
        self.broadcast(
            "thinking",
            &ThinkingEvent {
                seat,
                on,
                extended: false,
            },
        );
    }

    /// TS `emitChat`: screened to 140 UTF-16 units; an empty line is dropped.
    fn emit_chat(&self, seat: Seat, kind: ChatKind, text: &str, msg_id: String) {
        let clean = screen_line(text, BANTER_MAX_CHARS);
        if clean.is_empty() {
            return;
        }
        self.broadcast(
            "chat",
            &ChatEvent {
                msg_id,
                seat,
                kind,
                text: clean,
                is_final: True,
            },
        );
    }

    /// TS `emitScript`: the table's own calls for bids, throw-ins and scores.
    fn emit_script(&self, steps: &[Step]) {
        let game = &self.st.game;
        for (n, step) in steps.iter().enumerate() {
            let msg_id = format!("{}:{}:{n}", game.game_id, game.v);
            match step {
                Step::Bid { seat, say, .. } => self.emit_chat(*seat, ChatKind::Call, say, msg_id),
                Step::ThrowIn => self.emit_chat(
                    game.hand.dealer_seat,
                    ChatKind::System,
                    "Throw it in. Nobody wanted it.",
                    msg_id,
                ),
                Step::HandScored {
                    result, maker_seat, ..
                } => {
                    use euchre_engine::HandResult;
                    let line = match result {
                        HandResult::Euchre => Some("Euchre!"),
                        HandResult::March | HandResult::LoneMarch => Some("March!"),
                        _ => None,
                    };
                    if let (Some(line), Some(maker)) = (line, maker_seat) {
                        self.emit_chat(*maker, ChatKind::Call, line, msg_id);
                    }
                }
                _ => {}
            }
        }
    }

    /// TS `pushSync`: each player connection gets its own redacted view.
    fn push_sync(&self, steps: &[Step]) {
        let st = &self.st;
        self.host.send_sync(&|seat| sync_payload(st, seat, steps));
    }

    // -------------------------------------------------------------- bookkeeping

    /// TS `journal`.
    fn journal(&mut self, seat: Seat, mv: &LegalMove, attribution: Attribution) {
        let (kind, source, reason) = match attribution {
            Attribution::Human => (ActionKind::Human, None, None),
            Attribution::Ai(s) => (ActionKind::Ai, Some(s), None),
            Attribution::Auto(r) => (ActionKind::Auto, None, Some(r)),
        };
        self.st.journal.push(PlayerAction {
            seat,
            id: mv.id.clone(),
            mv: mv.mv,
            kind,
            source,
            reason,
        });
        let extra = self.st.journal.len().saturating_sub(JOURNAL_CAP);
        self.st.journal.drain(..extra);
    }

    /// TS `observeMove`: the public, timing-honest record of a decision.
    fn observe_move(&mut self, before: &GameState, seat: Seat, mv: &LegalMove) {
        let Some(kind) = decision_kind_for(before.hand.phase) else {
            return;
        };
        let now = self.now();
        let opened = self.st.turn_opened.as_ref();
        let elapsed = opened
            .filter(|o| o.turn_id == before.turn_id)
            .and_then(|o| now.checked_sub(o.at));
        let human = seat == HUMAN_SEAT;
        let reliable = human
            && opened.is_some_and(|o| o.connected)
            && elapsed.is_some_and(|e| e <= OBSERVED_MAX_MS);
        let action = match mv.mv {
            Move::Play { .. } | Move::Pass | Move::OrderUp { .. } | Move::Call { .. } => {
                mv.label.clone()
            }
            _ if kind == AIDecisionKind::Discard => "Dealer discarded a private card".to_owned(),
            _ => "Cut decision".to_owned(),
        };
        let elapsed_ms = if reliable {
            elapsed.map(|e| {
                (e + OBSERVED_ROUNDING_MS / 2) / OBSERVED_ROUNDING_MS * OBSERVED_ROUNDING_MS
            })
        } else {
            None
        };
        let timing = match (human, reliable) {
            (true, true) => ObservationTiming::HumanObserved,
            (true, false) => ObservationTiming::Unreliable,
            (false, _) => ObservationTiming::ComputerPaced,
        };
        let hand_no = before.hand.hand_no;
        self.st.observations.retain(|o| o.hand_no == hand_no);
        self.st.observations.push(TableObservation {
            hand_no,
            seat,
            kind: observation_kind(kind),
            action,
            elapsed_ms,
            timing,
        });
        let extra = self.st.observations.len().saturating_sub(OBSERVATIONS_CAP);
        self.st.observations.drain(..extra);
    }

    /// TS `rememberMoveId`.
    fn remember_move_id(&mut self, client_move_id: &str) {
        let ring = &mut self.st.game.recent_move_ids;
        ring.push(client_move_id.to_owned());
        let extra = ring.len().saturating_sub(MOVE_ID_RING);
        ring.drain(..extra);
    }

    /// TS `settle`: advance through `lobby` and `deal`, which need no decision.
    fn settle(&mut self, steps: &mut Vec<Step>) -> Result<()> {
        for _ in 0..SETTLE_LIMIT {
            let phase = self.st.game.hand.phase;
            if phase != GamePhase::Lobby && phase != GamePhase::Deal {
                return Ok(());
            }
            let out = try_advance(&self.st.game)?;
            if out.steps.is_empty() && out.state == self.st.game {
                return Ok(());
            }
            commit(&mut self.st, out.state);
            steps.extend(out.steps);
        }
        bail!(
            "settle did not leave {:?} after {SETTLE_LIMIT} advances",
            self.st.game.hand.phase
        )
    }

    /// TS `cancelTurnSchedules`.
    async fn cancel_turn_schedules(&mut self) {
        let ids = [
            self.st.game.timer_id.take(),
            self.st.game.watchdog_id.take(),
            self.st.game.reveal_id.take(),
            self.st.tempo_id.take(),
            self.st.abandon_id.take(),
        ];
        for id in ids.into_iter().flatten() {
            self.host.cancel(&id).await;
        }
    }

    // ------------------------------------------------------------ turn sequencing

    /// TS `armTurn`: open the next turn's timers, or ask the next AI seat.
    async fn arm_turn(&mut self, steps: &[Step]) -> Result<()> {
        self.cancel_turn_schedules().await;
        if self.st.game.status != GameStatus::Active {
            return self.record_match_if_needed().await;
        }
        let phase = self.st.game.hand.phase;
        let turn_id = self.st.game.turn_id.clone();
        if phase == GamePhase::TrickResolve
            && self.st.review_tricks == Some(true)
            && review_ready(&self.st)
        {
            return Ok(());
        }
        if phase == GamePhase::TrickResolve || phase == GamePhase::HandScore {
            let seals = steps.iter().any(|s| {
                matches!(
                    s,
                    Step::TrickWon {
                        seals_euchre: true,
                        ..
                    }
                )
            });
            let full = if phase == GamePhase::HandScore {
                self.tempo.hand_score_ms
            } else {
                self.tempo.trick_resolve(seals)
            };
            let now = self.now();
            // On a wake the pause resumes where it was, never restarts (ledger).
            let due = match &self.st.ledger.tempo {
                Some(d) if d.turn_id == turn_id => d.at,
                _ => now + full,
            };
            let id = self
                .host
                .schedule(due.saturating_sub(now), ON_TEMPO_GATE, vec![json!(turn_id)])
                .await?;
            self.st.tempo_id = Some(id);
            self.st.ledger.tempo = Some(TurnDue { turn_id, at: due });
            return self.save().await;
        }
        self.st.ledger.tempo = None;

        let Some(seat) = self.st.game.hand.turn_seat else {
            return Ok(());
        };
        if self.st.turn_opened.as_ref().map(|o| o.turn_id.as_str()) != Some(turn_id.as_str()) {
            self.st.turn_opened = Some(TurnOpened {
                turn_id,
                at: self.now(),
                connected: self.host.player_connected(),
            });
        }
        if seat == HUMAN_SEAT {
            // TS `armHumanLadder`: a solo player may read, step away or
            // reconnect without losing their turn.
            self.st.game.turn_deadline_at = None;
            self.st.game.timer_id = None;
            self.st.abandon_id = None;
            return self.save().await;
        }
        self.dispatch_ai(seat).await
    }

    /// TS `dispatchAi`: park the heuristic move, arm the reveal and the
    /// watchdog, persist, then ask the seat (detached).
    async fn dispatch_ai(&mut self, seat: Seat) -> Result<()> {
        let Some(kind) = decision_kind_for(self.st.game.hand.phase) else {
            return Ok(());
        };
        let legal = legal_moves(&self.st.game, seat);
        let Some(first) = legal.first() else {
            tracing::error!(
                seat = seat.get(),
                phase = self.st.game.hand.phase.as_str(),
                "dispatchAi with an empty legal set"
            );
            return Ok(());
        };
        let now = self.now();
        let turn_id = self.st.game.turn_id.clone();
        let reveal_at = now + self.tempo.think_floor(kind, &turn_id);
        let view = table_view(&self.st, seat);
        let forced = legal.len() == 1;
        let fallback_id = if forced {
            first.id.clone()
        } else {
            top_move(
                RankParams::new(&view)
                    .with_hand(&view.hand)
                    .with_legal(&legal),
            )
            .unwrap_or_else(|| first.id.clone())
        };
        self.st.game.pending = Some(PendingDecision {
            seat,
            turn_id: turn_id.clone(),
            requested_at: now,
            reveal_at,
            decision: Some(fallback_id),
            source: Some(if forced {
                AiDecisionSource::Forced
            } else {
                AiDecisionSource::Fallback
            }),
            answered: Some(forced),
        });
        let deadline_at = now + self.tempo.ai_hard_cap_ms;
        self.st.game.turn_deadline_at = Some(deadline_at);
        let watchdog = self
            .host
            .schedule(
                self.tempo.ai_hard_cap_ms,
                ON_AI_TIMEOUT,
                vec![json!(turn_id), json!(seat.get())],
            )
            .await?;
        self.st.game.watchdog_id = Some(watchdog);
        let reveal = self
            .host
            .schedule(
                reveal_at.saturating_sub(now),
                RELEASE_AI_MOVE,
                vec![json!(turn_id)],
            )
            .await?;
        self.st.game.reveal_id = Some(reveal);
        self.save().await?; // PERSIST, before the seat is asked anything.
        self.thinking(seat, true);

        if forced {
            return Ok(());
        }
        let Some(assignment) = self.st.persona_for(seat) else {
            tracing::warn!(
                seat = seat.get(),
                "no persona for AI seat; keeping heuristic"
            );
            return Ok(());
        };
        let request = AIDecideRequest {
            internal_token: self.st.game.internal_token.clone(),
            game_id: self.st.game.game_id.clone(),
            seat,
            turn_id,
            kind,
            view,
            legal,
            ranking: None,
            deadline_at,
        };
        let call = DecideCall {
            seat,
            key: ai_seat_key(&self.st.game.game_id, seat),
            create_input: ai_seat_create_input(&self.st, assignment),
            request,
        };
        self.host.ai().decide(call);
        Ok(())
    }

    /// TS `notifyAiSeats`: every seat with a persona, detached.
    fn notify_ai_seats(&self, kind: AiSeatLifecycleKind, hand_no: Option<u32>) {
        let game = &self.st.game;
        for assignment in Seat::ALL
            .iter()
            .filter(|s| is_ai_seat(**s))
            .filter_map(|s| self.st.persona_for(*s))
        {
            self.host.ai().notify(LifecycleCall {
                seat: assignment.seat,
                key: ai_seat_key(&game.game_id, assignment.seat),
                create_input: ai_seat_create_input(&self.st, assignment),
                message: AiSeatLifecycleMessage {
                    internal_token: game.internal_token.clone(),
                    game_id: game.game_id.clone(),
                    kind,
                    hand_no,
                },
            });
        }
    }

    // --------------------------------------------------------------- applying

    /// TS `applyMove` (AI and auto moves).
    async fn apply_move(
        &mut self,
        seat: Seat,
        mv: &LegalMove,
        attribution: Attribution,
    ) -> Result<Vec<Step>> {
        let before = self.st.game.clone();
        let opts = ApplyOptions {
            turn_id: Some(before.turn_id.clone()),
        };
        let out = try_apply(&before, seat, mv.mv, Some(&opts)).map_err(apply_error)?;
        commit(&mut self.st, out.state);
        self.st.game.pending = None;
        self.observe_move(&before, seat, mv);
        self.journal(seat, mv, attribution);
        let mut steps = out.steps;
        self.settle(&mut steps)?;
        let boundary = self.hand_boundary(&before, &steps);
        self.save().await?; // 1. PERSIST (the move and its hand boundary, one batch)
        if is_ai_seat(seat) {
            self.thinking(seat, false);
        }
        self.emit_script(&steps);
        self.push_sync(&steps); // 2. FAN OUT
        self.send_boundary(boundary);
        Ok(steps)
    }

    /// TS `applyMoveAsHuman`.
    async fn apply_move_as_human(&mut self, mv: &LegalMove, body: &MoveMsg) -> Result<Vec<Step>> {
        let before = self.st.game.clone();
        let opts = ApplyOptions {
            turn_id: body.turn_id.clone(),
        };
        let out = try_apply(&before, HUMAN_SEAT, mv.mv, Some(&opts)).map_err(apply_error)?;
        commit(&mut self.st, out.state);
        self.st.game.applied_seq = body.seq;
        self.st.game.pending = None;
        self.observe_move(&before, HUMAN_SEAT, mv);
        self.remember_move_id(&body.client_move_id);
        self.journal(HUMAN_SEAT, mv, Attribution::Human);
        let mut steps = out.steps;
        self.settle(&mut steps)?;
        let boundary = self.hand_boundary(&before, &steps);
        self.save().await?; // 1. PERSIST (the move and its hand boundary, one batch)
        self.emit_script(&steps);
        self.push_sync(&steps); // 2. FAN OUT
        self.send_boundary(boundary);
        Ok(steps)
    }

    /// TS `autoPlay`: the first legal move for a silent seat.
    async fn auto_play(&mut self, seat: Seat, reason: AutoReason) -> Result<()> {
        let legal = legal_moves(&self.st.game, seat);
        let Some(mv) = legal.into_iter().next() else {
            tracing::error!(
                seat = seat.get(),
                phase = self.st.game.hand.phase.as_str(),
                "autoPlay found no legal move"
            );
            return Ok(());
        };
        tracing::warn!(
            seat = seat.get(),
            reason = reason.as_str(),
            move_id = %mv.id,
            "auto-playing for a silent seat"
        );
        let steps = self
            .apply_move(seat, &mv, Attribution::Auto(reason))
            .await?;
        self.arm_turn(&steps).await
    }

    // ------------------------------------------------------ hand and match ends

    /// TS `handBoundary`: archive, count and record a finished hand once;
    /// reset the journal when a new hand opens.
    ///
    /// The bookkeeping goes on the working copy, so the caller's next save
    /// persists the move and its hand boundary in one batch (a kill between two
    /// saves can no longer lose a thrown-in hand). What it owes other actors
    /// comes back as [`Boundary`] and is sent only after that save
    /// ([`Session::send_boundary`]), in TS order.
    fn hand_boundary(&mut self, before: &GameState, steps: &[Step]) -> Boundary {
        let mut out = Boundary::default();
        let thrown_in = steps.iter().any(|s| matches!(s, Step::ThrowIn));
        let hand_no = i64::from(self.st.game.hand.hand_no);
        let before_no = i64::from(before.hand.hand_no);
        if self.st.game.hand.phase == GamePhase::HandScore
            && self.st.last_journaled_hand_no != hand_no
        {
            let hand = self.st.game.hand.clone();
            self.reviews.push(build_hand_review(&hand));
            self.st.last_journaled_hand_no = hand_no;
            self.st.hands_played += 1;
            self.st.stats = accumulate_stats(&self.st.stats, &hand);
            out.push_hand(self.record_hand(&hand));
            out.0.push(BoundaryCall::Notify(
                AiSeatLifecycleKind::HandEnd,
                Some(hand.hand_no),
            ));
        } else if thrown_in && self.st.last_journaled_hand_no != before_no {
            self.reviews.push(build_hand_review(&before.hand));
            self.st.last_journaled_hand_no = before_no;
            self.st.stats.throw_ins += 1;
            out.push_hand(self.record_hand(&before.hand));
        }
        if self.st.game.hand.phase == GamePhase::Cutting && self.st.last_reset_hand_no != hand_no {
            self.st.last_reset_hand_no = hand_no;
            self.st.journal.clear();
            out.0.push(BoundaryCall::Notify(
                AiSeatLifecycleKind::ResetHand,
                Some(self.st.game.hand.hand_no),
            ));
        }
        out
    }

    /// Send what a saved hand boundary owes, in the order it was made.
    fn send_boundary(&self, boundary: Boundary) {
        for call in boundary.0 {
            match call {
                BoundaryCall::Hand(call) => self.host.profile().record_hands(call),
                BoundaryCall::Notify(kind, hand_no) => self.notify_ai_seats(kind, hand_no),
            }
        }
    }

    /// TS `recordHand`'s body: the journal entry of `hand`, or `None` with no
    /// owner or no profile. Built before the journal resets; sent after the save.
    fn record_hand(&self, hand: &HandState) -> Option<RecordHandsCall> {
        let profile = self.host.profile();
        if self.st.owner_user_id.is_empty() || !profile.available() {
            return None;
        }
        let entry = HandJournalEntry {
            match_id: self.st.match_id.clone(),
            hand_no: hand.hand_no,
            seed: self.st.game.seed.clone(),
            dealer_seat: hand.dealer_seat,
            deck_order: hand.deck_order.clone(),
            moves: self.st.journal.clone(),
            trump: hand.trump,
            maker_seat: hand.maker_seat,
            alone_seat: hand.alone_seat,
            tricks_won: hand.tricks_won,
            result: hand.result,
            delta: hand.delta,
            ended_at: self.now(),
        };
        Some(RecordHandsCall {
            profile_user: self.st.profile_user().to_owned(),
            match_id: self.st.match_id.clone(),
            entry,
        })
    }

    /// TS `recordMatchIfNeeded`: file the match once, then arm the self-reap.
    async fn record_match_if_needed(&mut self) -> Result<()> {
        if self.st.recorded_at.is_some() || self.st.owner_user_id.is_empty() {
            return Ok(());
        }
        let now = self.now();
        let game = &self.st.game;
        let record = MatchRecord {
            schema: SchemaV1,
            match_id: self.st.match_id.clone(),
            user_id: self.st.profile_user().to_owned(),
            seed: game.seed.clone(),
            cfg: game.cfg,
            first_dealer: game.first_dealer,
            started_at: self.st.started_at,
            ended_at: Some(now),
            status: game.status,
            outcome: outcome_of(game),
            score: game.score,
            winner_team: game.winner_team,
            hands_played: self.st.hands_played,
            personas: self.st.personas.clone(),
            stats: self.st.stats,
            // The table meters nothing; each aiSeat flushes its own spend.
            tokens: TokenUsage::default(),
        };
        let profile = self.host.profile();
        if !profile.available() {
            // Nothing can ever record it: mark it recorded so nothing spins.
            self.st.recorded_at = Some(now);
            self.notify_ai_seats(AiSeatLifecycleKind::GameEnd, None);
            return self.save().await;
        }
        // 03 §16.7: the call runs off the lane and its outcome comes back as
        // `on_record_game`. The retry is armed (and persisted) before the send,
        // so a crash between the send and its outcome still retries (§13).
        if self.st.profile_retry_id.is_none() {
            self.arm_profile_retry().await?;
        }
        self.save().await?;
        let profile_user = self.st.profile_user().to_owned();
        profile.record_game(RecordGameCall {
            profile_user,
            record,
        });
        Ok(())
    }

    /// The outcome of the detached `recordGame` (the rest of TS
    /// `recordMatchIfNeeded` after its `sendBounded`), in its own lane turn.
    /// `Err` carries the error text.
    pub async fn on_record_game(&mut self, outcome: std::result::Result<(), String>) -> Result<()> {
        if self.st.recorded_at.is_some() {
            return Ok(()); // an earlier send already landed
        }
        if let Err(err) = outcome {
            tracing::warn!(err = %err, "recordGame failed; will retry");
            if self.st.profile_retry_id.is_none() {
                self.arm_profile_retry().await?;
            }
            return self.save().await;
        }
        self.st.recorded_at = Some(self.now());
        if let Some(id) = self.st.profile_retry_id.take() {
            self.host.cancel(&id).await;
        }
        self.st.ledger.profile_retry_at = None;
        self.notify_ai_seats(AiSeatLifecycleKind::GameEnd, None);
        if self.st.reap_id.is_none() {
            self.arm_reap().await?;
        }
        self.save().await
    }

    async fn arm_profile_retry(&mut self) -> Result<()> {
        let ms = self.tempo.profile_retry_ms;
        let id = self.host.schedule(ms, FLUSH_PROFILE, Vec::new()).await?;
        self.st.profile_retry_id = Some(id);
        self.st.ledger.profile_retry_at = Some(self.now() + ms);
        Ok(())
    }

    async fn arm_reap(&mut self) -> Result<()> {
        let ms = self.tempo.reap_ms;
        let id = self.host.schedule(ms, ON_REAP, Vec::new()).await?;
        self.st.reap_id = Some(id);
        self.st.ledger.reap_at = Some(self.now() + ms);
        Ok(())
    }

    // ------------------------------------------------------- message handlers

    /// TS `onMove` (the `move` queue and inline `submitMove`). A `RuleError`
    /// comes back as `Err`; the caller turns it into a rejected ack.
    pub async fn on_move(&mut self, body: &MoveMsg) -> Result<MoveAck> {
        let game = &self.st.game;
        if game.status != GameStatus::Active {
            return Ok(MoveAck::Rejected {
                code: ErrorCode::Protocol(ProtocolErrorCode::GameComplete),
                legal: Vec::new(),
            });
        }
        if body.seq <= game.applied_seq || game.recent_move_ids.contains(&body.client_move_id) {
            // A redelivered message or a double-click: idempotent.
            return Ok(MoveAck::Ok { v: game.v });
        }
        let legal = legal_moves(game, HUMAN_SEAT);
        let Some(mv) = legal.iter().find(|m| m.id == body.move_id).cloned() else {
            return Ok(MoveAck::Rejected {
                code: ErrorCode::Rule(RuleCode::IllegalMove),
                legal,
            });
        };
        let steps = self.apply_move_as_human(&mv, body).await?;
        self.arm_turn(&steps).await?;
        Ok(MoveAck::Ok { v: self.st.game.v })
    }

    /// TS `onAiDecision`: the first accepted answer replaces the parked move.
    pub async fn on_ai_decision(&mut self, body: &AIDecision) -> Result<()> {
        let game = &self.st.game;
        if !match_token_ok(&game.internal_token, Some(&body.internal_token)) {
            tracing::warn!(
                code = ProtocolErrorCode::InternalTokenMismatch.as_str(),
                "dropped forged aiDecision"
            );
            return Ok(());
        }
        if body.game_id != game.game_id || game.status != GameStatus::Active {
            return Ok(());
        }
        if body.turn_id != game.turn_id {
            return Ok(()); // a stale reply for a turn that moved on
        }
        let Some(seat) = game.hand.turn_seat.filter(|s| is_ai_seat(*s)) else {
            return Ok(());
        };
        if body.seat != seat {
            tracing::warn!(
                claimed = body.seat.get(),
                actual = seat.get(),
                code = ProtocolErrorCode::SeatMismatch.as_str(),
                "dropped aiDecision with a mismatched seat"
            );
            return Ok(());
        }
        let Some(pending) = game.pending.as_ref().filter(|p| p.turn_id == game.turn_id) else {
            return Ok(()); // nothing outstanding
        };
        if pending.answered == Some(true) || pending.source == Some(AiDecisionSource::Forced) {
            return Ok(()); // the first accepted answer wins
        }
        let deadline = game
            .turn_deadline_at
            .unwrap_or(pending.requested_at + self.tempo.ai_hard_cap_ms);
        if self.now() >= deadline {
            return self.release_pending(true).await;
        }

        let legal = legal_moves(game, seat);
        let chosen = legal.iter().find(|m| m.id == body.move_id);
        let Some(mv) = chosen.or(legal.first()) else {
            return Ok(());
        };
        if chosen.is_none() {
            tracing::warn!(
                seat = seat.get(),
                move_id = %body.move_id,
                "aiSeat returned a move outside the legal set"
            );
        }
        let source = if chosen.is_some() {
            body.source
        } else {
            AiDecisionSource::Fallback
        };
        let move_id = mv.id.clone();
        let kind = decision_kind_for(game.hand.phase);
        if let Some(banter) = &body.banter {
            let msg_id = format!("{}:{}:{}", game.game_id, game.turn_id, seat.get());
            self.emit_chat(seat, ChatKind::Banter, banter, msg_id);
        }
        let reveal_at = {
            let Some(pending) = self.st.game.pending.as_mut() else {
                return Ok(());
            };
            pending.decision = Some(move_id);
            pending.source = Some(source);
            pending.answered = Some(true);
            pending.reveal_at
        };
        tracing::info!(
            seat = seat.get(),
            source = source.as_str(),
            kind = kind.map(|k| k.as_str()),
            "AI decision accepted"
        );
        let now = self.now();
        if reveal_at > now {
            // The new reveal is scheduled and saved before the old timers go,
            // so a failure on the way leaves the old watchdog and reveal armed.
            let turn_id = self.st.game.turn_id.clone();
            let id = self
                .host
                .schedule(reveal_at - now, RELEASE_AI_MOVE, vec![json!(turn_id)])
                .await?;
            let old_reveal = self.st.game.reveal_id.replace(id);
            let old_watchdog = self.st.game.watchdog_id.take();
            self.save().await?;
            for id in [old_watchdog, old_reveal].into_iter().flatten() {
                self.host.cancel(&id).await;
            }
            return Ok(());
        }
        self.release_pending(false).await
    }

    /// TS `releasePending(force)`: play the parked move once it may be shown.
    async fn release_pending(&mut self, force: bool) -> Result<()> {
        let game = &self.st.game;
        let Some(pending) = game.pending.as_ref() else {
            return Ok(());
        };
        if pending.turn_id != game.turn_id {
            return Ok(());
        }
        let Some(decision) = pending.decision.clone() else {
            return Ok(());
        };
        // The think floor is presentation, not an inference deadline: keep
        // waiting for an asked seat until its answer or the watchdog.
        let deadline = game
            .turn_deadline_at
            .unwrap_or(pending.requested_at + self.tempo.ai_hard_cap_ms);
        if !force
            && pending.answered != Some(true)
            && pending.source != Some(AiDecisionSource::Forced)
            && self.st.persona_for(pending.seat).is_some()
            && self.host.ai().asks()
            && self.now() < deadline
        {
            return Ok(());
        }
        let pending_seat = pending.seat;
        let source = pending.source.unwrap_or(AiDecisionSource::Fallback);
        let Some(seat) = game.hand.turn_seat.filter(|s| *s == pending_seat) else {
            self.st.game.pending = None;
            return self.save().await;
        };
        let legal = legal_moves(game, seat);
        let mv = legal
            .iter()
            .find(|m| m.id == decision)
            .or(legal.first())
            .cloned();
        let Some(mv) = mv else {
            self.st.game.pending = None;
            return self.save().await;
        };
        let steps = self.apply_move(seat, &mv, Attribution::Ai(source)).await?;
        self.arm_turn(&steps).await
    }

    /// TS `onAiSay`: streamed or final table talk from a seat.
    /// `null_hand_no` is an explicit `handNo: null` in the raw body, which TS
    /// compared against the hand number and dropped (the wire type reads it as absent).
    pub fn on_ai_say(&self, body: &AiSayMsg, null_hand_no: bool) {
        let game = &self.st.game;
        if !match_token_ok(&game.internal_token, Some(&body.internal_token)) {
            tracing::warn!(
                code = ProtocolErrorCode::InternalTokenMismatch.as_str(),
                "dropped forged aiSay"
            );
            return;
        }
        if body.game_id != game.game_id || !is_ai_seat(body.seat) {
            return;
        }
        let current = match body.hand_no {
            None if null_hand_no => false,
            None => body.turn_id == game.turn_id,
            Some(n) => n == game.hand.hand_no,
        };
        if !current {
            return;
        }
        if body.is_final {
            let text = body.text.as_deref().unwrap_or("");
            self.emit_chat(body.seat, ChatKind::Banter, text, body.msg_id.clone());
            return;
        }
        let delta = screen_line(body.delta.as_deref().unwrap_or(""), BANTER_MAX_CHARS);
        if delta.is_empty() {
            return;
        }
        self.broadcast(
            "chatDelta",
            &ChatDeltaEvent {
                msg_id: body.msg_id.clone(),
                seat: body.seat,
                delta,
            },
        );
    }

    /// TS `advanceTempo`: leave a read pause.
    async fn advance_tempo(&mut self) -> Result<()> {
        let before = self.st.game.clone();
        let out = try_advance(&before)?;
        if out.steps.is_empty() && out.state == before {
            return Ok(());
        }
        self.st.review_ready_turn_id = Some(None);
        commit(&mut self.st, out.state);
        let mut steps = out.steps;
        self.settle(&mut steps)?;
        let boundary = self.hand_boundary(&before, &steps);
        self.save().await?;
        self.emit_script(&steps);
        self.push_sync(&steps);
        self.send_boundary(boundary);
        self.arm_turn(&steps).await
    }

    /// TS `onReview`: the trick-review preference and its Continue.
    pub async fn on_review(&mut self, body: &ReviewMsg) -> Result<()> {
        let game = &self.st.game;
        if !match_token_ok(&game.internal_token, Some(&body.internal_token))
            || body.game_id != game.game_id
            || game.status != GameStatus::Active
        {
            return Ok(());
        }
        match (body.action, body.enabled) {
            (ReviewAction::Set, Some(enabled)) => {
                self.st.review_tricks = Some(enabled);
                let same = self.st.game.clone();
                commit(&mut self.st, same);
                self.save().await?;
                self.push_sync(&[]);
                if !enabled && review_ready(&self.st) {
                    self.advance_tempo().await?;
                }
                Ok(())
            }
            (ReviewAction::Continue, _)
                if body.turn_id == game.turn_id
                    && review_ready(&self.st)
                    && game.hand.phase == GamePhase::TrickResolve =>
            {
                self.advance_tempo().await
            }
            _ => Ok(()),
        }
    }

    /// A `tick` message from a connection (the former queue). The envelope is
    /// checked as TS `onTick` did, then the tick goes through [`Session::fire`],
    /// so the ledger's early-fire guard applies: an internal caller cannot skip
    /// a read pause, force an AI timeout or reap a table before its due time.
    pub async fn on_tick_message(&mut self, body: &TickMsg) -> Result<()> {
        let game = &self.st.game;
        if !match_token_ok(&game.internal_token, Some(&body.internal_token)) {
            tracing::warn!(
                code = ProtocolErrorCode::InternalTokenMismatch.as_str(),
                "dropped forged tick"
            );
            return Ok(());
        }
        if body.game_id != game.game_id {
            return Ok(());
        }
        let turn_id = match body.kind {
            TickKind::FlushProfile | TickKind::Reap => None,
            _ => Some(body.turn_id.clone()),
        };
        self.fire(body.kind, turn_id, body.seat).await
    }

    /// TS `onTick`: every timer, after the schedule target's checks.
    pub async fn on_tick(&mut self, body: &TickMsg) -> Result<()> {
        let game = &self.st.game;
        if !match_token_ok(&game.internal_token, Some(&body.internal_token)) {
            tracing::warn!(
                code = ProtocolErrorCode::InternalTokenMismatch.as_str(),
                "dropped forged tick"
            );
            return Ok(());
        }
        if body.game_id != game.game_id {
            return Ok(());
        }
        // Match-scoped, so they skip the turn check every other tick passes.
        match body.kind {
            TickKind::FlushProfile => {
                self.st.profile_retry_id = None;
                self.st.ledger.profile_retry_at = None;
                return self.record_match_if_needed().await;
            }
            TickKind::Reap => {
                self.st.reap_id = None;
                self.st.ledger.reap_at = None;
                if self.st.recorded_at.is_none() {
                    // The durable copy does not exist yet. Never destroy over history.
                    self.arm_reap().await?;
                    return self.save().await;
                }
                self.host.destroy();
                return Ok(());
            }
            _ => {}
        }
        if body.turn_id != game.turn_id || game.status != GameStatus::Active {
            return Ok(()); // the turn moved on; this timer is stale
        }
        match body.kind {
            TickKind::Tempo => {
                self.st.tempo_id = None;
                self.st.ledger.tempo = None;
                if self.st.game.hand.phase == GamePhase::TrickResolve
                    && self.st.review_tricks == Some(true)
                {
                    self.st.review_ready_turn_id = Some(Some(self.st.game.turn_id.clone()));
                    let same = self.st.game.clone();
                    commit(&mut self.st, same);
                    self.save().await?;
                    self.push_sync(&[]);
                    return Ok(());
                }
                self.advance_tempo().await
            }
            TickKind::ReleaseAi => {
                self.st.game.reveal_id = None;
                self.release_pending(false).await
            }
            TickKind::AiTimeout => {
                self.st.game.watchdog_id = None;
                let Some(seat) = self.st.game.hand.turn_seat.filter(|s| is_ai_seat(*s)) else {
                    return Ok(());
                };
                if body.seat.is_some_and(|s| s != seat) {
                    return Ok(());
                }
                self.thinking(seat, false);
                let parked = self
                    .st
                    .game
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.turn_id == self.st.game.turn_id && p.decision.is_some());
                if parked {
                    tracing::warn!(
                        seat = seat.get(),
                        "AI deadline reached; releasing parked safety move"
                    );
                    self.release_pending(true).await
                } else {
                    self.auto_play(seat, AutoReason::AiTimeout).await
                }
            }
            TickKind::Nudge | TickKind::Abandon => {
                // Legacy human timers: they have no game effect.
                self.st.game.timer_id = None;
                self.st.abandon_id = None;
                self.save().await
            }
            TickKind::FlushProfile | TickKind::Reap => Ok(()),
        }
    }

    // ----------------------------------------------------------------- waking

    /// Re-arm the match timers recorded in the ledger (D20: a fire is
    /// at-most-once, so every wake re-derives them).
    async fn rearm_match_timers(&mut self) -> Result<()> {
        let now = self.now();
        if let Some(at) = self.st.ledger.reap_at {
            if let Some(id) = self.st.reap_id.take() {
                self.host.cancel(&id).await;
            }
            let id = self
                .host
                .schedule(at.saturating_sub(now), ON_REAP, Vec::new())
                .await?;
            self.st.reap_id = Some(id);
        }
        if let Some(at) = self.st.ledger.profile_retry_at {
            if let Some(id) = self.st.profile_retry_id.take() {
                self.host.cancel(&id).await;
            }
            let id = self
                .host
                .schedule(at.saturating_sub(now), FLUSH_PROFILE, Vec::new())
                .await?;
            self.st.profile_retry_id = Some(id);
        }
        Ok(())
    }

    /// TS `reconcile`, run in `on_start` on every wake.
    pub async fn reconcile(&mut self) -> Result<()> {
        self.rearm_match_timers().await?;
        if self.st.game.status != GameStatus::Active {
            self.record_match_if_needed().await?;
            return self.flush().await;
        }
        let game = &self.st.game;
        let parked = game
            .pending
            .as_ref()
            .filter(|p| p.turn_id == game.turn_id && p.decision.is_some())
            .cloned();
        if let Some(pending) = parked {
            let now = self.now();
            let turn_id = game.turn_id.clone();
            let waiting = pending.answered != Some(true)
                && pending.source != Some(AiDecisionSource::Forced)
                && self.st.persona_for(pending.seat).is_some()
                && self.host.ai().asks();
            if waiting {
                let deadline = game
                    .turn_deadline_at
                    .unwrap_or(pending.requested_at + self.tempo.ai_hard_cap_ms);
                if deadline <= now {
                    return self.release_pending(true).await;
                }
                if let Some(id) = self.st.game.watchdog_id.take() {
                    self.host.cancel(&id).await;
                }
                let id = self
                    .host
                    .schedule(
                        deadline - now,
                        ON_AI_TIMEOUT,
                        vec![json!(turn_id), json!(pending.seat.get())],
                    )
                    .await?;
                self.st.game.watchdog_id = Some(id);
                return self.save().await;
            }
            if pending.reveal_at <= now {
                return self.release_pending(false).await;
            }
            if let Some(id) = self.st.game.reveal_id.take() {
                self.host.cancel(&id).await;
            }
            let id = self
                .host
                .schedule(
                    pending.reveal_at - now,
                    RELEASE_AI_MOVE,
                    vec![json!(turn_id)],
                )
                .await?;
            self.st.game.reveal_id = Some(id);
            return self.save().await;
        }

        let before = self.st.game.clone();
        let mut steps = Vec::new();
        self.settle(&mut steps)?;
        let boundary = self.hand_boundary(&before, &steps);
        self.save().await?;
        if !steps.is_empty() {
            self.emit_script(&steps);
            self.push_sync(&steps);
        }
        self.send_boundary(boundary);
        self.arm_turn(&steps).await
    }

    /// A schedule target fired (03 §16.5): the stale-turn check of the TS
    /// action, then the Rust early-fire guard against the ledger, then the
    /// tick. `turn_id` is `None` for the match-scoped `flushProfile` / `onReap`.
    pub async fn fire(
        &mut self,
        kind: TickKind,
        turn_id: Option<String>,
        seat: Option<Seat>,
    ) -> Result<()> {
        let game = &self.st.game;
        let turn_id = match turn_id {
            Some(t) => {
                if t != game.turn_id {
                    return Ok(());
                }
                t
            }
            None => game.turn_id.clone(),
        };
        let now = self.now();
        let early = |at: u64| at > now + EARLY_FIRE_TOLERANCE_MS;
        match kind {
            TickKind::Tempo => {
                if let Some(due) = self
                    .st
                    .ledger
                    .tempo
                    .as_ref()
                    .filter(|d| d.turn_id == turn_id && early(d.at))
                {
                    let delay = due.at - now;
                    let id = self
                        .host
                        .schedule(delay, ON_TEMPO_GATE, vec![json!(turn_id)])
                        .await?;
                    self.st.tempo_id = Some(id);
                    return self.save().await;
                }
            }
            TickKind::ReleaseAi => {
                if let Some(p) = game
                    .pending
                    .as_ref()
                    .filter(|p| p.turn_id == turn_id && early(p.reveal_at))
                {
                    let delay = p.reveal_at - now;
                    let id = self
                        .host
                        .schedule(delay, RELEASE_AI_MOVE, vec![json!(turn_id)])
                        .await?;
                    self.st.game.reveal_id = Some(id);
                    return self.save().await;
                }
            }
            TickKind::AiTimeout => {
                if let Some(at) = game.turn_deadline_at.filter(|at| early(*at)) {
                    let mut args = vec![json!(turn_id)];
                    if let Some(s) = seat {
                        args.push(json!(s.get()));
                    }
                    let id = self.host.schedule(at - now, ON_AI_TIMEOUT, args).await?;
                    self.st.game.watchdog_id = Some(id);
                    return self.save().await;
                }
            }
            TickKind::Reap => {
                if let Some(at) = self.st.ledger.reap_at.filter(|at| early(*at)) {
                    let id = self.host.schedule(at - now, ON_REAP, Vec::new()).await?;
                    self.st.reap_id = Some(id);
                    return self.save().await;
                }
            }
            TickKind::FlushProfile => {
                if let Some(at) = self.st.ledger.profile_retry_at.filter(|at| early(*at)) {
                    let id = self
                        .host
                        .schedule(at - now, FLUSH_PROFILE, Vec::new())
                        .await?;
                    self.st.profile_retry_id = Some(id);
                    return self.save().await;
                }
            }
            TickKind::Nudge | TickKind::Abandon => {}
        }
        let tick = TickMsg {
            internal_token: self.st.game.internal_token.clone(),
            game_id: self.st.game.game_id.clone(),
            kind,
            turn_id,
            seat,
        };
        self.on_tick(&tick).await
    }
}

/// TS `submitMove` after its role check, in the TS order: a finished match,
/// then a move outside the legal set (with that set as metadata), then a
/// missing `clientMoveId`. Builds the `move` body (`seq` from the live state).
pub fn submit_move_body(st: &TableState, user_id: &str, request: &Value) -> Result<MoveMsg> {
    let game = &st.game;
    if game.status != GameStatus::Active {
        return Err(user_error(
            ProtocolErrorCode::GameComplete.as_str(),
            "This match is over",
        ));
    }
    let request = SubmitMoveRequest::from_wire(request);
    let legal = legal_moves(game, HUMAN_SEAT);
    let Some(mv) = legal.iter().find(|m| m.id == request.move_id) else {
        return Err(user_error_with_metadata(
            "illegal_move",
            "That move is not legal right now",
            &IllegalMoveMetadata { legal },
        ));
    };
    if !request.has_client_move_id() {
        return Err(user_error(
            ProtocolErrorCode::InternalError.as_str(),
            "Missing clientMoveId",
        ));
    }
    Ok(MoveMsg {
        seq: game.applied_seq.saturating_add(1),
        turn_id: request.turn_id,
        move_id: mv.id.clone(),
        client_move_id: utf16_prefix(&request.client_move_id, CLIENT_MOVE_ID_UNITS),
        user_id: user_id.to_owned(),
    })
}

/// The body of a `move` action (the former queue), rebuilt server side: the
/// client's `seq` and `userId` are ignored (the next `seq` and the verified
/// caller stand in), and its `clientMoveId` is cut to 64 UTF-16 units, as
/// `submitMove` does. A client-chosen `seq` could otherwise park the game
/// behind `u64::MAX` or grow the state without bound.
pub fn queue_move_body(st: &TableState, user_id: &str, msg: &MoveMsg) -> MoveMsg {
    MoveMsg {
        seq: st.game.applied_seq.saturating_add(1),
        turn_id: msg.turn_id.clone(),
        move_id: msg.move_id.clone(),
        client_move_id: utf16_prefix(&msg.client_move_id, CLIENT_MOVE_ID_UNITS),
        user_id: user_id.to_owned(),
    }
}

/// Whether `saved` already holds the human move `body` (its `seq` is applied
/// and its `clientMoveId` remembered). A turn that failed after this point
/// acks the move: it is durable, and recovery arms what follows it.
pub fn move_persisted(saved: &TableState, body: &MoveMsg) -> bool {
    saved.game.applied_seq >= body.seq && saved.game.recent_move_ids.contains(&body.client_move_id)
}

/// The view an internal-token reader gets (D22): seat 0's projection with
/// every private field emptied (`hand` and `legal`). The only holders of the
/// per-match token are the opposing `aiSeat`s, so they never see the human's cards.
pub fn internal_view(st: &TableState) -> PublicGameView {
    let mut view = table_view(st, HUMAN_SEAT);
    view.hand.clear();
    view.legal.clear();
    view
}

/// TS `reviewTrick` after its role check: validate the request and build the
/// `review` body. `continue` carries the request's turn; `set` the current one.
pub fn review_body(st: &TableState, request: &Value) -> Result<ReviewMsg> {
    let invalid = || {
        user_error(
            ProtocolErrorCode::InternalError.as_str(),
            "Invalid review request",
        )
    };
    let request: ReviewTrickRequest =
        serde_json::from_value(request.clone()).map_err(|_| invalid())?;
    let game = &st.game;
    let (action, enabled, turn_id) = match request {
        ReviewTrickRequest::Set { enabled } => {
            (ReviewAction::Set, Some(enabled), game.turn_id.clone())
        }
        ReviewTrickRequest::Continue { turn_id } => (ReviewAction::Continue, None, turn_id),
    };
    Ok(ReviewMsg {
        internal_token: game.internal_token.clone(),
        game_id: game.game_id.clone(),
        action,
        enabled,
        turn_id,
    })
}

/// A `RuleError` carried by `e`, if any (TS `isRuleError`).
pub fn rule_error(e: &anyhow::Error) -> Option<&euchre_engine::RuleError> {
    e.downcast_ref::<euchre_engine::RuleError>()
}

/// For logs: TS `String(err).slice(0, 200)`.
pub fn err_excerpt(e: &anyhow::Error) -> String {
    log_excerpt(&format!("{e:#}"))
}

#[cfg(test)]
pub(crate) mod tests;
