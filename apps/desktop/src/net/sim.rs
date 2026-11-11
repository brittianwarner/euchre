//! `SimBackend`: an in-process table that behaves like the `euchreTable` actor (03 §6-§9).
//!
//! The rules come from `euchre-engine`; this module only adds what the actor adds:
//!
//! - **Version.** `v` bumps by one per adopted reducer output (`commit`), and the first
//!   view a client sees is `v = 1` (the create-time settle through `lobby` and `deal`).
//! - **Holds.** `trick_resolve` is held 3300 ms (3800 when the trick seals a euchre) and
//!   `hand_score` 2500 ms, then advanced. With `reviewTricks` on, the hold ends in the
//!   review gate (`awaitingTrickReview`) and waits for `reviewTrick { continue }`.
//! - **Computer seats.** Seats 1-3 think for the 02 §9 floor plus `fnv1a(turnId+"#pace")`
//!   jitter, with `thinking` on and off around it, then play the [`SimDecider`]'s choice.
//! - **Wire.** `submitMove` validates in the actor's order and acks like it; chat lines for
//!   bids, throw-ins, euchres and marches go out before the sync that carries their steps;
//!   a new connection gets a sync with `steps: []`.
//! - **View metadata.** `players`, `observations` (this hand), `reviewTricks`, `rules` and
//!   `awaitingTrickReview` are attached to every view.
//!
//! [`TableCore`] is that logic as a synchronous state machine with an explicit clock, so tests
//! can play a whole match in virtual time. [`SimBackend`] runs one `TableCore` per table on a
//! tokio task with real timers.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use euchre_ai::{HeuristicOptions, RankParams, top_move};
use euchre_engine::{
    ApplyOptions, ApplyResult, GameInit, GameState, LegalMove, LegalMoveId, Move, Rank, Suit,
    card_value, currently_winning, effective_suit, fnv1a, led_suit_of, legal_moves, partner_of,
    plain_rank, project, project_steps, rank_of, team_of, trump_rank, try_advance, try_apply,
    try_create_game,
};
use euchre_profile_model::{
    ClientHandReplay, ClientMatchReplay, JsJson, MatchRow, ProfileSettings, ProfileStats,
    SettingsPatch, apply_settings_patch, compute_stats, default_settings, personas_from_settings,
};
use euchre_protocol::{
    ChatEvent, ChatKind, CompletedHandReview, ErrorCode, GamePhase, GameStatus, HandResult,
    MatchListPage, MatchOutcome, MatchStats, MatchSummary, MoveAck, ObservationKind,
    ObservationTiming, PersonaAssignment, PresenceEvent, ProtocolErrorCode, PublicGameView,
    ReviewTrickAck, ReviewTrickRequest, RuleCode, SchemaV1, Seat, Step, SubmitMoveRequest,
    SyncEvent, TableEvent, TableObservation, TablePlayer, TableRules, ThinkingEvent, TokenUsage,
    True, build_hand_review, default_personas, table_players,
};
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};

use super::{
    Backend, ConnEvent, Identity, NetError, TableCalls, TableConfig, TableConn, now_ms, wall_ms,
};

/// The human's seat.
pub const HUMAN: Seat = Seat::SOUTH;

/// The throw-in line (03 §6.3).
pub const MSG_THROW_IN: &str = "Throw it in. Nobody wanted it.";
/// The euchre call.
pub const MSG_EUCHRE: &str = "Euchre!";
/// The march call.
pub const MSG_MARCH: &str = "March!";

/// How many observations a view keeps (03 §8.2).
const OBSERVATIONS_KEPT: usize = 32;
/// How many recent client move ids are remembered for idempotent retries.
const MOVE_IDS_KEPT: usize = 32;
/// The longest human think that still counts as an observed timing (ms).
const OBSERVED_MAX_MS: u64 = 20_000;
/// Observed timings are rounded to this step (ms).
const OBSERVED_STEP_MS: u64 = 250;

/// The table's pacing (03 §9). [`Tempo::NORMAL`] is the real table; [`Tempo::FAST`] is
/// `EUCHRE_FAST_TEMPO=1`; [`Tempo::INSTANT`] removes every wait (tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tempo {
    /// The `trick_resolve` hold.
    pub trick_resolve_ms: u64,
    /// The `trick_resolve` hold when the trick seals a euchre.
    pub trick_resolve_seals_euchre_ms: u64,
    /// The `hand_score` hold.
    pub hand_score_ms: u64,
    /// Think floors: cut, bid1, bid2, discard, play.
    pub think_floor_ms: [u64; 5],
    /// Jitter added to a floor: `fnv1a(turnId + "#pace") % jitter`.
    pub think_jitter_ms: u64,
}

impl Tempo {
    /// The shipped table.
    pub const NORMAL: Tempo = Tempo {
        trick_resolve_ms: euchre_protocol::TRICK_PRESENTATION_MS,
        trick_resolve_seals_euchre_ms: 3800,
        hand_score_ms: 2500,
        think_floor_ms: [700, 1900, 1900, 1100, 1000],
        think_jitter_ms: 400,
    };
    /// `EUCHRE_FAST_TEMPO=1`.
    pub const FAST: Tempo = Tempo {
        trick_resolve_ms: 120,
        trick_resolve_seals_euchre_ms: 180,
        hand_score_ms: 250,
        think_floor_ms: [80, 120, 120, 120, 100],
        think_jitter_ms: 40,
    };
    /// No waiting at all.
    pub const INSTANT: Tempo = Tempo {
        trick_resolve_ms: 0,
        trick_resolve_seals_euchre_ms: 0,
        hand_score_ms: 0,
        think_floor_ms: [0; 5],
        think_jitter_ms: 0,
    };

    /// `thinkFloorMs(kind, turnId)`.
    pub fn think_ms(&self, kind: ObservationKind, turn_id: &str) -> u64 {
        let floor = self.think_floor_ms[kind_index(kind)];
        let jitter = if self.think_jitter_ms == 0 {
            0
        } else {
            u64::from(fnv1a(&format!("{turn_id}#pace"))) % self.think_jitter_ms
        };
        floor + jitter
    }
}

fn kind_index(kind: ObservationKind) -> usize {
    match kind {
        ObservationKind::Cut => 0,
        ObservationKind::Bid1 => 1,
        ObservationKind::Bid2 => 2,
        ObservationKind::Discard => 3,
        ObservationKind::Play => 4,
    }
}

/// `decisionKindFor(phase)`.
fn decision_kind(phase: GamePhase) -> Option<ObservationKind> {
    match phase {
        GamePhase::Cutting => Some(ObservationKind::Cut),
        GamePhase::BidRound1 => Some(ObservationKind::Bid1),
        GamePhase::DealerDiscard => Some(ObservationKind::Discard),
        GamePhase::BidRound2 => Some(ObservationKind::Bid2),
        GamePhase::TrickPlay => Some(ObservationKind::Play),
        _ => None,
    }
}

/// Chooses a computer seat's move from that seat's redacted view (`view.legal` is its set).
pub trait SimDecider: Send + Sync {
    /// The chosen legal move id; `None` (or an id not in `view.legal`) plays `legal[0]`.
    fn decide(&self, view: &PublicGameView) -> Option<LegalMoveId>;
}

/// A small deterministic strategy built only from engine helpers.
///
/// Bids on trump strength (bower and trump ranks plus off-suit aces), discards its weakest
/// card, leads aces or (as makers) trump, ducks under a winning partner and otherwise wins
/// as cheaply as it can.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeuristicDecider;

/// The strength at which the heuristic names trump.
const CALL_AT: u32 = 16;
/// The strength at which it goes alone.
const ALONE_AT: u32 = 28;
/// What an off-suit ace adds to trump strength.
const OFF_ACE: u32 = 3;

impl HeuristicDecider {
    fn strength(cards: &[euchre_engine::Card], trump: Suit) -> u32 {
        cards
            .iter()
            .map(|&c| match trump_rank(c, trump) {
                0 if rank_of(c) == Rank::Ace => OFF_ACE,
                t => u32::from(t),
            })
            .sum()
    }

    fn keep_value(card: euchre_engine::Card, trump: Option<Suit>) -> u32 {
        match trump {
            Some(t) if trump_rank(card, t) > 0 => 100 + u32::from(trump_rank(card, t)),
            _ if rank_of(card) == Rank::Ace => 50,
            _ => u32::from(plain_rank(card)),
        }
    }

    fn card_of(m: &LegalMove) -> Option<euchre_engine::Card> {
        match m.mv {
            Move::Play { card } | Move::Discard { card } => Some(card),
            _ => None,
        }
    }

    fn bid(view: &PublicGameView) -> Option<LegalMoveId> {
        let has_pass = view.legal.iter().any(|m| m.mv == Move::Pass);
        let mut best: Option<(u32, &LegalMove)> = None;
        for m in &view.legal {
            let (suit, alone) = match m.mv {
                Move::OrderUp { alone } => (view.up_card.map(euchre_engine::suit_of)?, alone),
                Move::Call { suit, alone } => (suit, alone),
                _ => continue,
            };
            let mut cards = view.hand.clone();
            if matches!(m.mv, Move::OrderUp { .. })
                && view.you == view.dealer_seat
                && let Some(up) = view.up_card
            {
                cards.push(up);
            }
            let s = Self::strength(&cards, suit);
            let wants = if alone { s >= ALONE_AT } else { s >= CALL_AT };
            let forced = !has_pass && !alone;
            if (wants || forced) && best.is_none_or(|(b, _)| s > b) {
                best = Some((s, m));
            }
        }
        best.map(|(_, m)| m.id.clone()).or_else(|| {
            view.legal
                .iter()
                .find(|m| m.mv == Move::Pass)
                .map(|m| m.id.clone())
        })
    }

    fn discard(view: &PublicGameView) -> Option<LegalMoveId> {
        view.legal
            .iter()
            .filter_map(|m| Self::card_of(m).map(|c| (Self::keep_value(c, view.trump), m)))
            .min_by_key(|(v, _)| *v)
            .map(|(_, m)| m.id.clone())
    }

    fn play(view: &PublicGameView) -> Option<LegalMoveId> {
        let trump = view.trump;
        let cards: Vec<(euchre_engine::Card, &LegalMove)> = view
            .legal
            .iter()
            .filter_map(|m| Self::card_of(m).map(|c| (c, m)))
            .collect();
        let plays = &view.trick.plays;
        let lowest = |list: &[(euchre_engine::Card, &LegalMove)]| {
            list.iter()
                .min_by_key(|(c, _)| Self::keep_value(*c, trump))
                .map(|(_, m)| m.id.clone())
        };
        let Some(led) = led_suit_of(plays, trump) else {
            // Leading: an off-suit ace, else (as makers) the best trump, else the lowest card.
            let makers = view
                .maker_seat
                .is_some_and(|m| team_of(m) == team_of(view.you));
            if let Some((_, m)) = cards.iter().find(|(c, _)| {
                rank_of(*c) == Rank::Ace && trump.is_none_or(|t| effective_suit(*c, Some(t)) != t)
            }) {
                return Some(m.id.clone());
            }
            if makers
                && let Some((_, m)) = cards
                    .iter()
                    .filter(|(c, _)| trump.is_some_and(|t| trump_rank(*c, t) > 0))
                    .max_by_key(|(c, _)| Self::keep_value(*c, trump))
            {
                return Some(m.id.clone());
            }
            return lowest(&cards);
        };
        let winning = currently_winning(plays, trump);
        if winning.is_some_and(|w| w.seat == partner_of(view.you)) {
            return lowest(&cards);
        }
        let to_beat = winning.map_or(0, |w| card_value(w.card, led, trump));
        let winners: Vec<_> = cards
            .iter()
            .copied()
            .filter(|(c, _)| card_value(*c, led, trump) > to_beat)
            .collect();
        if winners.is_empty() {
            lowest(&cards)
        } else {
            winners
                .iter()
                .min_by_key(|(c, _)| card_value(*c, led, trump))
                .map(|(_, m)| m.id.clone())
        }
    }
}

impl SimDecider for HeuristicDecider {
    fn decide(&self, view: &PublicGameView) -> Option<LegalMoveId> {
        if view.legal.len() == 1 {
            return view.legal.first().map(|m| m.id.clone());
        }
        match view.phase {
            GamePhase::BidRound1 | GamePhase::BidRound2 => Self::bid(view),
            GamePhase::DealerDiscard => Self::discard(view),
            GamePhase::TrickPlay => Self::play(view),
            _ => view.legal.first().map(|m| m.id.clone()),
        }
    }
}

/// The `euchre-ai` heuristic: the same ranking the server's computer seats fall back on,
/// flavoured per seat by the persona dials.
#[derive(Clone, Copy, Debug, Default)]
pub struct AiDecider {
    by_seat: [HeuristicOptions; 4],
}

impl AiDecider {
    /// Every seat on the neutral dials.
    pub fn neutral() -> AiDecider {
        AiDecider::default()
    }

    /// Each persona's seat plays with its own aggression and risk.
    pub fn for_personas(personas: &[PersonaAssignment]) -> AiDecider {
        let mut by_seat = [HeuristicOptions::NEUTRAL; 4];
        for a in personas {
            if let Some(slot) = by_seat.get_mut(a.seat.index()) {
                *slot = HeuristicOptions::from_persona(&a.persona);
            }
        }
        AiDecider { by_seat }
    }
}

impl SimDecider for AiDecider {
    fn decide(&self, view: &PublicGameView) -> Option<LegalMoveId> {
        let opts = self
            .by_seat
            .get(view.you.index())
            .copied()
            .unwrap_or(HeuristicOptions::NEUTRAL);
        top_move(RankParams::new(view).with_opts(opts))
    }
}

/// What the table sends to its connections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outgoing {
    /// A broadcast event (thinking, chat).
    Broadcast(TableEvent),
    /// A sync for the human seat, with steps already redacted for it.
    Sync(Box<SyncEvent>),
}

/// A pending table timer.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Timer {
    /// The `trick_resolve` / `hand_score` hold for this turn.
    Tempo { turn_id: String },
    /// A computer seat's move for this turn.
    Ai { turn_id: String, seat: Seat },
}

/// One table's rules state as a synchronous machine (the actor without IO).
pub struct TableCore {
    state: GameState,
    players: Vec<TablePlayer>,
    review_tricks: bool,
    review_ready_turn_id: Option<String>,
    recent_move_ids: VecDeque<String>,
    observations: Vec<TableObservation>,
    archive: Vec<CompletedHandReview>,
    hands_played: u32,
    turn_opened: Option<(String, u64)>,
    timer: Option<(u64, Timer)>,
    tempo: Tempo,
    decider: Arc<dyn SimDecider>,
}

impl std::fmt::Debug for TableCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableCore")
            .field("game_id", &self.state.game_id)
            .field("v", &self.state.v)
            .field("phase", &self.state.hand.phase)
            .finish_non_exhaustive()
    }
}

/// Everything a new table needs.
#[derive(Clone)]
pub struct TableSetup {
    /// The game id.
    pub game_id: String,
    /// The deal seed.
    pub seed: String,
    /// The chosen rules.
    pub cfg: TableConfig,
    /// The first dealer; the engine picks one from the seed when `None`.
    pub first_dealer: Option<Seat>,
    /// Pacing.
    pub tempo: Tempo,
    /// The computer seats' strategy.
    pub decider: Arc<dyn SimDecider>,
}

impl TableCore {
    /// Creates the match and settles it to its first decision at `v = 1`.
    pub fn new(setup: TableSetup, now: u64) -> Result<TableCore, NetError> {
        let state = try_create_game(GameInit {
            game_id: setup.game_id,
            seed: setup.seed,
            cfg: Some(setup.cfg.overrides()),
            internal_token: Some("sim".to_owned()),
            first_dealer: setup.first_dealer,
        })
        .map_err(|e| NetError::Other(e.to_string()))?;
        let mut core = TableCore {
            state,
            players: table_players(&default_personas()),
            review_tricks: false,
            review_ready_turn_id: None,
            recent_move_ids: VecDeque::new(),
            observations: Vec::new(),
            archive: Vec::new(),
            hands_played: 0,
            turn_opened: None,
            timer: None,
            tempo: setup.tempo,
            decider: setup.decider,
        };
        let mut steps = Vec::new();
        core.settle(&mut steps)?;
        core.commit();
        let mut out = Vec::new();
        core.arm(now, &mut out);
        Ok(core)
    }

    /// The match version.
    pub fn v(&self) -> u64 {
        self.state.v
    }

    /// The authoritative state (tests and fixtures only; never sent anywhere).
    pub fn state(&self) -> &GameState {
        &self.state
    }

    /// Whether the match is still being played.
    pub fn is_active(&self) -> bool {
        self.state.status == GameStatus::Active
    }

    /// Finished hands so far (throw-ins excluded).
    pub fn hands_played(&self) -> u32 {
        self.hands_played
    }

    /// When the next timer is due, on the caller's clock.
    pub fn next_due(&self) -> Option<u64> {
        self.timer.as_ref().map(|(at, _)| *at)
    }

    /// The `tableView(0)`: the human's projection plus the actor metadata.
    pub fn view(&self) -> PublicGameView {
        self.view_for(HUMAN)
    }

    fn view_for(&self, seat: Seat) -> PublicGameView {
        let mut view = project(&self.state, seat);
        let hand_no = self.state.hand.hand_no;
        view.players = Some(self.players.clone());
        view.observations = Some(
            self.observations
                .iter()
                .filter(|o| o.hand_no == hand_no)
                .cloned()
                .collect(),
        );
        view.review_tricks = Some(self.review_tricks);
        view.rules = Some(TableRules {
            stick_the_dealer: self.state.cfg.stick_the_dealer,
            require_natural_trump: self.state.cfg.natural_trump_required(),
        });
        view.awaiting_trick_review = Some(
            self.state.hand.phase == GamePhase::TrickResolve
                && self.review_ready_turn_id.as_deref() == Some(self.state.turn_id.as_str()),
        );
        view
    }

    /// The connect-time sync (`steps: []`).
    pub fn connect_sync(&self) -> SyncEvent {
        self.sync_with(Vec::new())
    }

    fn sync_with(&self, steps: Vec<Step>) -> SyncEvent {
        SyncEvent {
            v: self.state.v,
            view: self.view(),
            steps: project_steps(&steps, HUMAN),
        }
    }

    /// `getHandReview(handNo)`: the archive of a finished hand.
    pub fn hand_review(&self, hand_no: u32) -> Option<CompletedHandReview> {
        self.archive.iter().find(|h| h.hand_no == hand_no).cloned()
    }

    /// `submitMove` for the human seat (03 §7.1), with the actor's validation order.
    pub fn submit(&mut self, request: &SubmitMoveRequest, now: u64) -> (MoveAck, Vec<Outgoing>) {
        let mut out = Vec::new();
        if self.state.status != GameStatus::Active {
            return (
                MoveAck::Rejected {
                    code: ErrorCode::Protocol(ProtocolErrorCode::GameComplete),
                    legal: Vec::new(),
                },
                out,
            );
        }
        // The TS order: status, legality, a missing id, truncate it to 64, the dedupe ring.
        let legal = legal_moves(&self.state, HUMAN);
        let Some(entry) = legal.iter().find(|m| m.id == request.move_id).cloned() else {
            return (
                MoveAck::Rejected {
                    code: ErrorCode::Rule(RuleCode::IllegalMove),
                    legal,
                },
                out,
            );
        };
        if request.client_move_id.is_empty() {
            return (
                MoveAck::Rejected {
                    code: ErrorCode::Protocol(ProtocolErrorCode::InternalError),
                    legal: Vec::new(),
                },
                out,
            );
        }
        let client_move_id = truncate_utf16(&request.client_move_id, CLIENT_MOVE_ID_MAX);
        if self.recent_move_ids.iter().any(|id| *id == client_move_id) {
            return (MoveAck::Ok { v: self.state.v }, out);
        }
        let before = self.state.clone();
        let opts = ApplyOptions {
            turn_id: request.turn_id.clone(),
        };
        let result = match try_apply(&self.state, HUMAN, entry.mv, Some(&opts)) {
            Ok(r) => r,
            Err(euchre_engine::ApplyError::Rule(e)) => {
                return (
                    MoveAck::Rejected {
                        code: ErrorCode::Rule(e.code),
                        legal: e.legal,
                    },
                    out,
                );
            }
            Err(euchre_engine::ApplyError::Fault(fault)) => {
                tracing::error!(%fault, "sim table: engine fault on a human move");
                return (
                    MoveAck::Rejected {
                        code: ErrorCode::Protocol(ProtocolErrorCode::InternalError),
                        legal: Vec::new(),
                    },
                    out,
                );
            }
        };
        self.remember_move_id(client_move_id);
        self.observe(&before, HUMAN, &entry, now);
        if self.adopt(result, now, &mut out).is_err() {
            return (
                MoveAck::Rejected {
                    code: ErrorCode::Protocol(ProtocolErrorCode::InternalError),
                    legal: Vec::new(),
                },
                out,
            );
        }
        (MoveAck::Ok { v: self.state.v }, out)
    }

    /// `reviewTrick` (03 §8.7). Always queued.
    pub fn review(&mut self, request: &ReviewTrickRequest, now: u64) -> Vec<Outgoing> {
        let mut out = Vec::new();
        if self.state.status != GameStatus::Active {
            return out;
        }
        match request {
            ReviewTrickRequest::Set { enabled } => {
                self.review_tricks = *enabled;
                self.commit();
                out.push(Outgoing::Sync(Box::new(self.sync_with(Vec::new()))));
                if !enabled && self.review_ready_turn_id.as_deref() == Some(&self.state.turn_id) {
                    self.advance_tempo(now, &mut out);
                }
            }
            ReviewTrickRequest::Continue { turn_id } => {
                if *turn_id == self.state.turn_id
                    && self.review_ready_turn_id.as_deref() == Some(turn_id.as_str())
                    && self.state.hand.phase == GamePhase::TrickResolve
                {
                    self.advance_tempo(now, &mut out);
                }
            }
        }
        out
    }

    /// Runs the timer if it is due at `now`.
    pub fn fire(&mut self, now: u64) -> Vec<Outgoing> {
        let mut out = Vec::new();
        let Some((at, _)) = &self.timer else {
            return out;
        };
        if *at > now {
            return out;
        }
        let Some((_, timer)) = self.timer.take() else {
            return out;
        };
        if self.state.status != GameStatus::Active {
            return out;
        }
        match timer {
            Timer::Tempo { turn_id } => {
                if turn_id != self.state.turn_id {
                    return out;
                }
                if self.state.hand.phase == GamePhase::TrickResolve && self.review_tricks {
                    self.review_ready_turn_id = Some(turn_id);
                    self.commit();
                    out.push(Outgoing::Sync(Box::new(self.sync_with(Vec::new()))));
                } else {
                    self.advance_tempo(now, &mut out);
                }
            }
            Timer::Ai { turn_id, seat } => {
                if turn_id != self.state.turn_id || self.state.hand.turn_seat != Some(seat) {
                    return out;
                }
                self.play_ai(seat, now, &mut out);
            }
        }
        out
    }

    fn play_ai(&mut self, seat: Seat, now: u64, out: &mut Vec<Outgoing>) {
        let view = self.view_for(seat);
        let Some(fallback) = view.legal.first().cloned() else {
            return;
        };
        let chosen = self
            .decider
            .decide(&view)
            .and_then(|id| view.legal.iter().find(|m| m.id == id).cloned())
            .unwrap_or(fallback);
        let before = self.state.clone();
        let opts = ApplyOptions {
            turn_id: Some(self.state.turn_id.clone()),
        };
        match try_apply(&self.state, seat, chosen.mv, Some(&opts)) {
            Ok(result) => {
                self.observe(&before, seat, &chosen, now);
                out.push(Outgoing::Broadcast(TableEvent::Thinking(ThinkingEvent {
                    seat,
                    on: false,
                    extended: false,
                })));
                let _ = self.adopt(result, now, out);
            }
            Err(err) => tracing::error!(%err, "sim table: computer move refused"),
        }
    }

    /// `advanceTempo`: release a hold.
    fn advance_tempo(&mut self, now: u64, out: &mut Vec<Outgoing>) {
        let result = match try_advance(&self.state) {
            Ok(r) => r,
            Err(fault) => {
                tracing::error!(%fault, "sim table: engine fault on advance");
                return;
            }
        };
        if result.steps.is_empty() && result.state == self.state {
            return;
        }
        self.review_ready_turn_id = None;
        let _ = self.adopt(result, now, out);
    }

    /// commit, settle, emitScript, pushSync, handBoundary, armTurn.
    fn adopt(
        &mut self,
        result: ApplyResult,
        now: u64,
        out: &mut Vec<Outgoing>,
    ) -> Result<(), NetError> {
        let before_hand = self.state.hand.clone();
        let mut steps = result.steps;
        self.state = GameState {
            v: self.state.v,
            ..result.state
        };
        self.commit();
        self.settle(&mut steps)?;
        self.emit_script(&steps, out);
        out.push(Outgoing::Sync(Box::new(self.sync_with(steps.clone()))));
        self.hand_boundary(&before_hand, &steps);
        self.arm(now, out);
        Ok(())
    }

    /// `commit`: the only place `v` moves.
    fn commit(&mut self) {
        self.state.v += 1;
    }

    /// `settle`: advance through `lobby` and `deal` only.
    fn settle(&mut self, steps: &mut Vec<Step>) -> Result<(), NetError> {
        while matches!(self.state.hand.phase, GamePhase::Lobby | GamePhase::Deal)
            && self.state.status == GameStatus::Active
        {
            let r = try_advance(&self.state).map_err(|e| NetError::Other(e.to_string()))?;
            if r.steps.is_empty() && r.state == self.state {
                break;
            }
            let v = self.state.v;
            self.state = GameState { v, ..r.state };
            steps.extend(r.steps);
        }
        Ok(())
    }

    fn emit_script(&self, steps: &[Step], out: &mut Vec<Outgoing>) {
        let chat = |n: usize, seat: Seat, kind: ChatKind, text: &str| {
            Outgoing::Broadcast(TableEvent::Chat(ChatEvent {
                msg_id: format!("{}:{}:{n}", self.state.game_id, self.state.v),
                seat,
                kind,
                text: text.to_owned(),
                is_final: True,
            }))
        };
        for (n, step) in steps.iter().enumerate() {
            match step {
                Step::Bid { seat, say, .. } if !say.trim().is_empty() => {
                    out.push(chat(n, *seat, ChatKind::Call, say));
                }
                Step::ThrowIn => {
                    out.push(chat(
                        n,
                        self.state.hand.dealer_seat,
                        ChatKind::System,
                        MSG_THROW_IN,
                    ));
                }
                Step::HandScored {
                    result,
                    maker_seat: Some(maker),
                    ..
                } => {
                    let line = match result {
                        HandResult::Euchre => Some(MSG_EUCHRE),
                        HandResult::March | HandResult::LoneMarch => Some(MSG_MARCH),
                        _ => None,
                    };
                    if let Some(line) = line {
                        out.push(chat(n, *maker, ChatKind::Call, line));
                    }
                }
                _ => {}
            }
        }
    }

    fn hand_boundary(&mut self, before: &euchre_engine::HandState, steps: &[Step]) {
        let hand = &self.state.hand;
        if hand.phase == GamePhase::HandScore
            && !self.archive.iter().any(|h| h.hand_no == hand.hand_no)
        {
            self.archive.push(build_hand_review(hand));
            self.hands_played += 1;
        } else if steps.iter().any(|s| matches!(s, Step::ThrowIn))
            && !self.archive.iter().any(|h| h.hand_no == before.hand_no)
        {
            self.archive.push(build_hand_review(before));
        }
    }

    /// `armTurn`: one timer for whatever happens next.
    fn arm(&mut self, now: u64, out: &mut Vec<Outgoing>) {
        self.timer = None;
        if self.state.status != GameStatus::Active {
            return;
        }
        let phase = self.state.hand.phase;
        let turn_id = self.state.turn_id.clone();
        if phase == GamePhase::TrickResolve
            && self.review_tricks
            && self.review_ready_turn_id.as_deref() == Some(turn_id.as_str())
        {
            return;
        }
        if matches!(phase, GamePhase::TrickResolve | GamePhase::HandScore) {
            let wait = if phase == GamePhase::HandScore {
                self.tempo.hand_score_ms
            } else if self.last_trick_seals_euchre() {
                self.tempo.trick_resolve_seals_euchre_ms
            } else {
                self.tempo.trick_resolve_ms
            };
            self.timer = Some((now + wait, Timer::Tempo { turn_id }));
            return;
        }
        let Some(seat) = self.state.hand.turn_seat else {
            return;
        };
        if self.turn_opened.as_ref().is_none_or(|(t, _)| *t != turn_id) {
            self.turn_opened = Some((turn_id.clone(), now));
        }
        if seat == HUMAN {
            return;
        }
        let Some(kind) = decision_kind(phase) else {
            return;
        };
        let wait = self.tempo.think_ms(kind, &turn_id);
        out.push(Outgoing::Broadcast(TableEvent::Thinking(ThinkingEvent {
            seat,
            on: true,
            extended: false,
        })));
        self.timer = Some((now + wait, Timer::Ai { turn_id, seat }));
    }

    fn last_trick_seals_euchre(&self) -> bool {
        // The engine marks the sealing trick on its `trickWon` step; the view keeps only the
        // tricks, so recompute: the defenders now hold three tricks against the makers.
        let h = &self.state.hand;
        h.maker_seat.is_some_and(|maker| {
            let defenders = 1 - team_of(maker).index();
            h.tricks_won[defenders] == 3
                && h.trick_log.len() == 3 + h.tricks_won[1 - defenders] as usize
        })
    }

    fn remember_move_id(&mut self, id: String) {
        self.recent_move_ids.push_back(id);
        while self.recent_move_ids.len() > MOVE_IDS_KEPT {
            self.recent_move_ids.pop_front();
        }
    }

    /// `observeMove` (03 §8.2).
    fn observe(&mut self, before: &GameState, seat: Seat, entry: &LegalMove, now: u64) {
        let Some(kind) = decision_kind(before.hand.phase) else {
            return;
        };
        let action = match kind {
            ObservationKind::Discard => "Dealer discarded a private card".to_owned(),
            ObservationKind::Cut => "Cut decision".to_owned(),
            _ => entry.label.clone(),
        };
        let (elapsed_ms, timing) = if seat == HUMAN {
            let elapsed = self
                .turn_opened
                .as_ref()
                .filter(|(t, _)| *t == before.turn_id)
                .map(|(_, at)| now.saturating_sub(*at));
            match elapsed {
                Some(e) if e <= OBSERVED_MAX_MS => (
                    Some((e + OBSERVED_STEP_MS / 2) / OBSERVED_STEP_MS * OBSERVED_STEP_MS),
                    ObservationTiming::HumanObserved,
                ),
                _ => (None, ObservationTiming::Unreliable),
            }
        } else {
            (None, ObservationTiming::ComputerPaced)
        };
        self.observations.push(TableObservation {
            hand_no: before.hand.hand_no,
            seat,
            kind,
            action,
            elapsed_ms,
            timing,
        });
        let hand_no = before.hand.hand_no;
        self.observations.retain(|o| o.hand_no == hand_no);
        if self.observations.len() > OBSERVATIONS_KEPT {
            let excess = self.observations.len() - OBSERVATIONS_KEPT;
            self.observations.drain(..excess);
        }
    }

    /// Seats the computer players under these names (the profile's personas). The engine
    /// never reads them; only the views carry them.
    pub fn set_players(&mut self, players: Vec<TablePlayer>) {
        self.players = players;
    }

    /// The archived hands, in order (thrown-in hands included).
    pub fn archive(&self) -> &[CompletedHandReview] {
        &self.archive
    }

    /// The match as the history keeps it: its list row and its replay, played between
    /// `started_at` and `ended_at` (epoch ms). A match still in play reads as left unfinished.
    pub fn record(&self, started_at: u64, ended_at: u64) -> SimMatch {
        let mut summary = self.summary(ended_at);
        if summary.status == GameStatus::Active {
            summary.status = GameStatus::Abandoned;
        }
        let n = self.archive.len().max(1) as u64;
        let span = ended_at.saturating_sub(started_at);
        let mut stats = MatchStats::default();
        let hands = self
            .archive
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let hand = client_hand(h, started_at + span * (i as u64 + 1) / n);
                stats = add_hand(&stats, &hand);
                hand
            })
            .collect();
        let replay = ClientMatchReplay {
            match_id: summary.match_id.clone(),
            started_at: to_i64(started_at),
            ended_at: Some(to_i64(ended_at)),
            played_at: to_i64(ended_at),
            status: summary.status,
            outcome: summary.outcome,
            score: summary.score.map(i64::from),
            winner_team: self.state.winner_team.map(|t| i64::from(t.get())),
            hands_played: i64::from(summary.hands_played),
            opponents: summary.opponents.clone(),
            stats,
            hands,
        };
        SimMatch { summary, replay }
    }

    /// A summary row for the games list.
    pub fn summary(&self, played_at: u64) -> MatchSummary {
        let outcome = match (self.state.status, self.state.winner_team) {
            (GameStatus::Complete, Some(t)) if t == team_of(HUMAN) => MatchOutcome::Won,
            (GameStatus::Complete, Some(_)) => MatchOutcome::Lost,
            _ => MatchOutcome::Abandoned,
        };
        MatchSummary {
            match_id: self.state.game_id.clone(),
            played_at,
            status: self.state.status,
            outcome,
            score: self.state.score,
            hands_played: self.hands_played,
            opponents: self.players.iter().map(|p| p.name.clone()).collect(),
        }
    }
}

fn to_i64(ms: u64) -> i64 {
    i64::try_from(ms).unwrap_or(i64::MAX)
}

/// One archived hand as `getReplay` shows it. A thrown-in hand is archived before the
/// engine scores it, so it reads `throw_in` with no points, as the server records it.
fn client_hand(h: &CompletedHandReview, ended_at: u64) -> ClientHandReplay {
    let thrown_in = h.result.is_none() && h.trump.is_none();
    ClientHandReplay {
        hand_no: i64::from(h.hand_no),
        dealer_seat: h.dealer_seat.map_or(0, |s| i64::from(s.get())),
        trump: h.trump,
        maker_seat: h.maker_seat.map(|s| i64::from(s.get())),
        alone_seat: h.alone_seat.map(|s| i64::from(s.get())),
        tricks_won: h.tricks_won.unwrap_or([0, 0]).map(i64::from),
        result: if thrown_in {
            Some(HandResult::ThrowIn)
        } else {
            h.result
        },
        delta: if thrown_in {
            Some([0, 0])
        } else {
            h.delta.map(|d| d.map(i64::from))
        },
        ended_at: to_i64(ended_at),
        moves: Vec::new(),
    }
}

/// `accumulateStats` (the table's per-hand counters, team 0's side) over one replayed hand.
fn add_hand(prev: &MatchStats, hand: &ClientHandReplay) -> MatchStats {
    let maker_team = hand.maker_seat.map(|s| s % 2);
    let euchre = hand.result == Some(HandResult::Euchre);
    let lone = hand.alone_seat.is_some();
    let add = |cond: bool| u32::from(cond);
    let tricks = |n: i64| u32::try_from(n).unwrap_or(0);
    MatchStats {
        euchres_for: prev.euchres_for + add(euchre && maker_team == Some(1)),
        euchres_against: prev.euchres_against + add(euchre && maker_team == Some(0)),
        loners_attempted: prev.loners_attempted + add(lone),
        loners_made: prev.loners_made
            + add(lone
                && matches!(
                    hand.result,
                    Some(HandResult::LonePoint | HandResult::LoneMarch)
                )),
        marches: prev.marches
            + add(matches!(
                hand.result,
                Some(HandResult::March | HandResult::LoneMarch)
            )),
        throw_ins: prev.throw_ins + add(hand.result == Some(HandResult::ThrowIn)),
        tricks: [
            prev.tricks[0] + tricks(hand.tricks_won[0]),
            prev.tricks[1] + tricks(hand.tricks_won[1]),
        ],
    }
}

/// One finished match the sim remembers: the games-list row and the replay.
#[derive(Clone, Debug, PartialEq)]
pub struct SimMatch {
    /// The `listGames` row.
    pub summary: MatchSummary,
    /// The `getReplay` body.
    pub replay: ClientMatchReplay,
}

impl SimMatch {
    /// The stored row `computeStats` folds (secrets and opaque parts left empty).
    fn row(&self) -> MatchRow {
        let r = &self.replay;
        MatchRow {
            schema: SchemaV1,
            match_id: r.match_id.clone(),
            user_id: SIM_USER.to_owned(),
            seed: String::new(),
            cfg: JsJson::default(),
            first_dealer: 0,
            started_at: r.started_at,
            ended_at: r.ended_at,
            status: r.status,
            outcome: r.outcome,
            score: r.score,
            winner_team: r.winner_team,
            hands_played: r.hands_played,
            personas: Vec::new(),
            stats: r.stats,
            tokens: TokenUsage::default(),
        }
    }
}

/// The sim's player id.
const SIM_USER: &str = "sim-player";

/// How many past matches a new sim starts with, so Your games has something to show.
pub const SIM_HISTORY: usize = 12;

/// The games list's page size (06 §17.1).
pub const GAMES_PAGE: usize = 10;

/// The clock fixtures date their history from: Tuesday 10 November 2026, 19:20 UTC.
pub const FIXTURE_NOW_MS: u64 = 1_794_338_400_000;

/// Which past match is the one left unfinished.
const ABANDONED_AT: usize = 4;
/// How many scored hands the unfinished one got through.
const ABANDONED_HANDS: u32 = 3;
/// About how long a hand takes at the table.
const HAND_MS: u64 = 150_000;
/// The most moves and timers a past match runs before it stops.
const PAST_STEPS: usize = 20_000;
const HOUR_MS: u64 = 3_600_000;

/// `n` past matches, newest first, the newest ending a little before `latest_ms` and the rest
/// spread a day or three apart. Every match is played out by the engine (both teams on the
/// heuristic), so the scores, hands and counters are real. Deterministic for a seed.
pub fn seeded_history(seed: &str, n: usize, latest_ms: u64) -> Vec<SimMatch> {
    let players = table_players(&personas_from_settings(&default_settings(0)));
    let mut ended_at = latest_ms;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let deal_seed = format!("{seed}:past:{i}");
        let jitter = u64::from(fnv1a(&deal_seed));
        ended_at = ended_at.saturating_sub(if i == 0 {
            2 * HOUR_MS + jitter % (3 * HOUR_MS)
        } else {
            20 * HOUR_MS + jitter % (50 * HOUR_MS)
        });
        let cfg = TableConfig {
            stick_the_dealer: i % 3 == 1,
            require_natural_trump: i % 5 == 3,
        };
        let Ok(mut core) = TableCore::new(
            TableSetup {
                game_id: format!("{seed}-past-{:02}", n - i),
                seed: deal_seed,
                cfg,
                first_dealer: None,
                tempo: Tempo::INSTANT,
                decider: Arc::new(HeuristicDecider),
            },
            0,
        ) else {
            continue;
        };
        core.set_players(players.clone());
        let stop_after = (i == ABANDONED_AT).then_some(ABANDONED_HANDS);
        play_through(&mut core, stop_after);
        let hands = u64::try_from(core.archive.len()).unwrap_or(0);
        let started_at = ended_at.saturating_sub(hands * HAND_MS + jitter % (4 * 60_000));
        out.push(core.record(started_at, ended_at));
    }
    out
}

/// Plays `core` in virtual time until the match ends (or `stop_after` scored hands), the
/// human seat on the heuristic too.
fn play_through(core: &mut TableCore, stop_after: Option<u32>) {
    let human = HeuristicDecider;
    let mut now = 0;
    let mut n = 0u64;
    for _ in 0..PAST_STEPS {
        if !core.is_active() || stop_after.is_some_and(|h| core.hands_played >= h) {
            break;
        }
        let view = core.view();
        if view.turn_seat == Some(HUMAN) && !view.legal.is_empty() {
            let Some(id) = human
                .decide(&view)
                .or_else(|| view.legal.first().map(|m| m.id.clone()))
            else {
                break;
            };
            n += 1;
            let request = SubmitMoveRequest {
                move_id: id,
                turn_id: Some(view.turn_id.clone()),
                client_move_id: format!("past-{n}"),
            };
            let _ = core.submit(&request, now);
        } else if let Some(due) = core.next_due() {
            now = now.max(due);
            let _ = core.fire(now);
        } else {
            break;
        }
    }
}

/// One `listGames` page over `rows` (newest first); the cursor is the next row's index.
pub fn history_page(rows: &[SimMatch], cursor: Option<&str>) -> Result<MatchListPage, NetError> {
    let start = match cursor {
        None => 0,
        Some(c) => c.parse::<usize>().map_err(|_| NetError::NotFound)?,
    };
    let end = rows.len().min(start.saturating_add(GAMES_PAGE));
    let page = rows.get(start..end).unwrap_or_default();
    Ok(MatchListPage {
        rows: page.iter().map(|m| m.summary.clone()).collect(),
        cursor: (end < rows.len()).then(|| end.to_string()),
    })
}

/// `getStats` over `rows`.
pub fn history_stats(rows: &[SimMatch], now: u64) -> ProfileStats {
    let rows: Vec<MatchRow> = rows.iter().map(SimMatch::row).collect();
    compute_stats(&rows, to_i64(now))
}

/// A fixture view for scenes: the table right after the deal, with the human to act.
pub fn fixture_view(seed: &str, cfg: TableConfig) -> Result<PublicGameView, NetError> {
    let core = TableCore::new(
        TableSetup {
            game_id: format!("fixture-{seed}"),
            seed: seed.to_owned(),
            cfg,
            first_dealer: Some(Seat::EAST),
            tempo: Tempo::INSTANT,
            decider: Arc::new(HeuristicDecider),
        },
        0,
    )?;
    let mut core = core;
    // Play the robots through the cut until the human holds a dealt hand and the turn.
    for _ in 0..FIXTURE_STEPS {
        if core.view().turn_seat == Some(HUMAN) {
            break;
        }
        let Some(at) = core.next_due() else { break };
        core.fire(at);
    }
    Ok(core.view())
}

/// The most timer firings [`fixture_view`] runs before it gives up waiting for the human's turn.
const FIXTURE_STEPS: usize = 64;

/* ------------------------------------------------------------------------ */
/* The async table and backend                                               */
/* ------------------------------------------------------------------------ */

enum TableMsg {
    Connect(u64, mpsc::UnboundedSender<ConnEvent>),
    Disconnect(u64),
    Submit(SubmitMoveRequest, oneshot::Sender<MoveAck>),
    Review(ReviewTrickRequest, oneshot::Sender<ReviewTrickAck>),
    Snapshot(oneshot::Sender<Option<PublicGameView>>),
    HandReview(u32, oneshot::Sender<Option<CompletedHandReview>>),
}

#[derive(Clone)]
struct TableHandle {
    tx: mpsc::UnboundedSender<TableMsg>,
}

/// Options for a [`SimBackend`].
#[derive(Clone)]
pub struct SimOptions {
    /// Seeds are `{seed}:{n}` for the n-th table, so runs are repeatable.
    pub seed: String,
    /// Pacing.
    pub tempo: Tempo,
    /// The computer seats' strategy.
    pub decider: Arc<dyn SimDecider>,
    /// The first dealer of every table; the engine picks from the seed when `None`.
    pub first_dealer: Option<Seat>,
    /// How many past matches the history starts with ([`seeded_history`]); played out the
    /// first time the history is read.
    pub history: usize,
}

impl Default for SimOptions {
    fn default() -> Self {
        SimOptions {
            seed: "euchre-desktop".to_owned(),
            tempo: Tempo::NORMAL,
            decider: Arc::new(HeuristicDecider),
            first_dealer: None,
            history: SIM_HISTORY,
        }
    }
}

struct SimShared {
    options: SimOptions,
    tables: Mutex<HashMap<String, TableHandle>>,
    finished: Mutex<Vec<SimMatch>>,
    seeded: std::sync::atomic::AtomicBool,
    settings: Mutex<ProfileSettings>,
    next_table: AtomicU64,
    next_conn: AtomicU64,
}

/// The in-process backend (D28).
#[derive(Clone)]
pub struct SimBackend {
    shared: Arc<SimShared>,
}

impl SimBackend {
    /// A backend with these options.
    pub fn new(options: SimOptions) -> SimBackend {
        SimBackend {
            shared: Arc::new(SimShared {
                options,
                tables: Mutex::new(HashMap::new()),
                finished: Mutex::new(Vec::new()),
                seeded: std::sync::atomic::AtomicBool::new(false),
                settings: Mutex::new(default_settings(to_i64(wall_ms()))),
                next_table: AtomicU64::new(1),
                next_conn: AtomicU64::new(1),
            }),
        }
    }
}

impl SimBackend {
    /// The history, newest first, with the past matches played in on the first read.
    fn history(&self) -> Vec<SimMatch> {
        let mut rows = self.shared.finished.lock();
        if !self.shared.seeded.swap(true, Ordering::AcqRel) {
            let options = &self.shared.options;
            rows.extend(seeded_history(&options.seed, options.history, wall_ms()));
        }
        let mut out = rows.clone();
        drop(rows);
        out.sort_by_key(|m| std::cmp::Reverse(m.summary.played_at));
        out
    }
}

/// Sends `out` to every live connection; drops the closed ones.
fn broadcast(conns: &mut Vec<(u64, mpsc::UnboundedSender<ConnEvent>)>, out: Vec<Outgoing>) {
    for item in out {
        let event = match item {
            Outgoing::Broadcast(e) => e,
            Outgoing::Sync(s) => TableEvent::Sync(s),
        };
        conns.retain(|(_, tx)| tx.send(ConnEvent::Event(event.clone())).is_ok());
    }
}

async fn run_table(
    mut core: TableCore,
    mut rx: mpsc::UnboundedReceiver<TableMsg>,
    shared: Arc<SimShared>,
    started_at: u64,
) {
    let mut conns: Vec<(u64, mpsc::UnboundedSender<ConnEvent>)> = Vec::new();
    let mut recorded = false;
    loop {
        let due = core.next_due();
        let timer = async move {
            match due {
                Some(at) => {
                    super::rt::sleep(Duration::from_millis(at.saturating_sub(now_ms()))).await
                }
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    TableMsg::Connect(id, tx) => {
                        let first = tx.send(ConnEvent::Status(true)).is_ok()
                            && tx.send(ConnEvent::Event(TableEvent::Sync(Box::new(core.connect_sync())))).is_ok();
                        if first {
                            // Presence changes only on the edges: the first connection in.
                            let was_empty = conns.is_empty();
                            conns.push((id, tx));
                            if was_empty {
                                let presence = ConnEvent::Event(TableEvent::Presence(PresenceEvent { seat: HUMAN, online: true }));
                                conns.retain(|(_, c)| c.send(presence.clone()).is_ok());
                            }
                        }
                    }
                    TableMsg::Disconnect(id) => {
                        let before = conns.len();
                        conns.retain(|(c, _)| *c != id);
                        if before > 0 && conns.is_empty() {
                            // ...and the last one out: the player is offline. Nobody is left to
                            // hear it; a finished table with nobody at it is put away.
                            tracing::debug!(game_id = %core.state.game_id, "sim table: player offline");
                            if !core.is_active() {
                                shared.tables.lock().remove(&core.state.game_id);
                                if !recorded {
                                    shared.finished.lock().push(core.record(started_at, wall_ms()));
                                }
                                break;
                            }
                        }
                    }
                    TableMsg::Submit(req, reply) => {
                        let (ack, out) = core.submit(&req, now_ms());
                        broadcast(&mut conns, out);
                        let _ = reply.send(ack);
                    }
                    TableMsg::Review(req, reply) => {
                        let _ = reply.send(ReviewTrickAck { queued: True });
                        let out = core.review(&req, now_ms());
                        broadcast(&mut conns, out);
                    }
                    TableMsg::Snapshot(reply) => { let _ = reply.send(Some(core.view())); }
                    TableMsg::HandReview(n, reply) => { let _ = reply.send(core.hand_review(n)); }
                }
            }
            () = timer => {
                let out = core.fire(now_ms());
                broadcast(&mut conns, out);
            }
        }
        if !core.is_active() && !recorded {
            recorded = true;
            shared
                .finished
                .lock()
                .push(core.record(started_at, wall_ms()));
        }
    }
}

struct SimCalls {
    table: TableHandle,
    conn_id: u64,
}

impl SimCalls {
    async fn ask<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<T>) -> TableMsg,
    ) -> Result<T, NetError> {
        let (tx, rx) = oneshot::channel();
        self.table.tx.send(make(tx)).map_err(|_| NetError::Closed)?;
        rx.await.map_err(|_| NetError::Closed)
    }
}

#[async_trait]
impl TableCalls for SimCalls {
    async fn submit_move(&self, request: SubmitMoveRequest) -> Result<MoveAck, NetError> {
        self.ask(|tx| TableMsg::Submit(request, tx)).await
    }

    async fn review_trick(&self, request: ReviewTrickRequest) -> Result<ReviewTrickAck, NetError> {
        self.ask(|tx| TableMsg::Review(request, tx)).await
    }

    async fn snapshot(&self) -> Result<Option<PublicGameView>, NetError> {
        self.ask(TableMsg::Snapshot).await
    }

    async fn get_hand_review(&self, hand_no: u32) -> Result<Option<CompletedHandReview>, NetError> {
        self.ask(|tx| TableMsg::HandReview(hand_no, tx)).await
    }

    async fn close(&self) {
        let _ = self.table.tx.send(TableMsg::Disconnect(self.conn_id));
    }
}

#[async_trait]
impl Backend for SimBackend {
    fn name(&self) -> &'static str {
        "sim"
    }

    async fn identity(&self) -> Result<Identity, NetError> {
        Ok(Identity {
            user_id: SIM_USER.to_owned(),
            display_name: None,
        })
    }

    async fn create_table(&self, cfg: TableConfig) -> Result<String, NetError> {
        let n = self.shared.next_table.fetch_add(1, Ordering::Relaxed);
        let options = &self.shared.options;
        let game_id = format!("sim-{n}");
        // The table seats the profile's opponents, as the server does (`personasFromSettings`).
        let players = table_players(&personas_from_settings(&self.shared.settings.lock()));
        let mut core = TableCore::new(
            TableSetup {
                game_id: game_id.clone(),
                seed: format!("{}:{n}", options.seed),
                cfg,
                first_dealer: options.first_dealer,
                tempo: options.tempo,
                decider: Arc::clone(&options.decider),
            },
            now_ms(),
        )?;
        core.set_players(players);
        let (tx, rx) = mpsc::unbounded_channel();
        // The runtime seam: tokio in the window, the page's event loop in the browser.
        drop(super::rt::spawn(run_table(
            core,
            rx,
            Arc::clone(&self.shared),
            wall_ms(),
        )));
        self.shared
            .tables
            .lock()
            .insert(game_id.clone(), TableHandle { tx });
        Ok(game_id)
    }

    async fn open_table(&self, game_id: &str) -> Result<TableConn, NetError> {
        let table = self
            .shared
            .tables
            .lock()
            .get(game_id)
            .cloned()
            .ok_or(NetError::NotFound)?;
        let conn_id = self.shared.next_conn.fetch_add(1, Ordering::Relaxed);
        let (tx, events) = mpsc::unbounded_channel();
        table
            .tx
            .send(TableMsg::Connect(conn_id, tx))
            .map_err(|_| NetError::Closed)?;
        Ok(TableConn {
            game_id: game_id.to_owned(),
            events,
            calls: Arc::new(SimCalls { table, conn_id }),
        })
    }

    async fn list_games(&self, cursor: Option<String>) -> Result<MatchListPage, NetError> {
        history_page(&self.history(), cursor.as_deref())
    }

    async fn match_detail(&self, match_id: &str) -> Result<ClientMatchReplay, NetError> {
        self.history()
            .into_iter()
            .find(|m| m.summary.match_id == match_id)
            .map(|m| m.replay)
            .ok_or(NetError::NotFound)
    }

    async fn stats(&self) -> Result<ProfileStats, NetError> {
        Ok(history_stats(&self.history(), wall_ms()))
    }

    async fn get_settings(&self) -> Result<ProfileSettings, NetError> {
        Ok(self.shared.settings.lock().clone())
    }

    async fn patch_settings(&self, patch: SettingsPatch) -> Result<ProfileSettings, NetError> {
        let mut settings = self.shared.settings.lock();
        let next = apply_settings_patch(&settings, &patch.to_value(), to_i64(wall_ms()))
            .map_err(|e| NetError::Other(e.to_string()))?;
        *settings = next.clone();
        Ok(next)
    }
}

/// The longest `clientMoveId` the table keeps (`slice(0, 64)` in the TS, in UTF-16 units).
const CLIENT_MOVE_ID_MAX: usize = 64;

/// The first `max` UTF-16 code units of `s`, never splitting a character.
fn truncate_utf16(s: &str, max: usize) -> String {
    let mut units = 0;
    s.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= max
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core(seed: &str, review: bool) -> TableCore {
        let mut c = TableCore::new(
            TableSetup {
                game_id: format!("t-{seed}"),
                seed: seed.to_owned(),
                cfg: TableConfig::default(),
                first_dealer: None,
                tempo: Tempo::NORMAL,
                decider: Arc::new(HeuristicDecider),
            },
            0,
        )
        .unwrap();
        c.review_tricks = review;
        c
    }

    fn submit(c: &mut TableCore, id: &str, n: u64, now: u64) -> (MoveAck, Vec<Outgoing>) {
        let req = SubmitMoveRequest {
            move_id: id.to_owned(),
            turn_id: Some(c.state.turn_id.clone()),
            client_move_id: format!("m-{n}"),
        };
        c.submit(&req, now)
    }

    /// Plays a whole match in virtual time: the human plays the heuristic's choice too.
    fn play_out(c: &mut TableCore) -> (u64, Vec<Outgoing>) {
        let mut now = 0;
        let mut n = 0;
        let mut all = Vec::new();
        let human = HeuristicDecider;
        for _ in 0..20_000 {
            if !c.is_active() {
                break;
            }
            let view = c.view();
            if view.turn_seat == Some(HUMAN) && !view.legal.is_empty() {
                let id = human.decide(&view).unwrap();
                n += 1;
                let (ack, out) = submit(c, &id, n, now);
                assert!(ack.is_ok(), "{ack:?}");
                all.extend(out);
            } else if let Some(due) = c.next_due() {
                now = now.max(due);
                all.extend(c.fire(now));
            } else if view.awaiting_trick_review == Some(true) {
                all.extend(c.review(
                    &ReviewTrickRequest::Continue {
                        turn_id: view.turn_id.clone(),
                    },
                    now,
                ));
            } else {
                panic!("stalled at {:?}", view.phase);
            }
        }
        (now, all)
    }

    #[test]
    fn first_view_is_v1_with_metadata() {
        let c = core("a", false);
        let view = c.view();
        assert_eq!(view.v, 1);
        assert_eq!(view.players.as_ref().map(Vec::len), Some(3));
        assert_eq!(view.review_tricks, Some(false));
        assert_eq!(view.awaiting_trick_review, Some(false));
        assert!(!matches!(view.phase, GamePhase::Lobby | GamePhase::Deal));
        assert!(c.connect_sync().steps.is_empty());
    }

    #[test]
    fn plays_a_full_match_to_game_over() {
        let mut c = core("full", false);
        let (_, out) = play_out(&mut c);
        assert_eq!(c.state.status, GameStatus::Complete);
        assert!(c.state.score.iter().any(|&s| s >= 10));
        assert!(c.hands_played() > 0);
        let mut last_v = 0;
        for o in &out {
            if let Outgoing::Sync(s) = o {
                assert!(s.v > last_v, "v is strictly monotonic across syncs");
                last_v = s.v;
            }
        }
        assert!(
            out.iter()
                .any(|o| matches!(o, Outgoing::Broadcast(TableEvent::Chat(_))))
        );
    }

    #[test]
    fn review_gate_waits_for_continue() {
        let mut c = core("review", true);
        let (_, out) = play_out(&mut c);
        assert_eq!(c.state.status, GameStatus::Complete);
        assert!(out.iter().any(|o| matches!(
            o,
            Outgoing::Sync(s) if s.view.awaiting_trick_review == Some(true) && s.steps.is_empty()
        )));
    }

    #[test]
    fn stale_and_illegal_moves_are_refused_and_duplicates_ack() {
        let mut c = core("acks", false);
        // Drive to the human's turn.
        let mut now = 0;
        while c.view().turn_seat != Some(HUMAN) {
            now = c.next_due().unwrap();
            c.fire(now);
        }
        let (ack, _) = submit(&mut c, "play:XX", 1, now);
        assert!(
            matches!(ack, MoveAck::Rejected { code: ErrorCode::Rule(RuleCode::IllegalMove), ref legal } if !legal.is_empty())
        );

        let id = c.view().legal[0].id.clone();
        let stale = SubmitMoveRequest {
            move_id: id.clone(),
            turn_id: Some("stale".into()),
            client_move_id: "s-1".into(),
        };
        let (ack, _) = c.submit(&stale, now);
        assert!(matches!(
            ack,
            MoveAck::Rejected {
                code: ErrorCode::Rule(RuleCode::StaleTurn),
                ..
            }
        ));

        let v = c.v();
        let (ack, out) = submit(&mut c, &id, 7, now);
        assert_eq!(ack, MoveAck::Ok { v: v + 1 });
        assert!(matches!(
            out.last(),
            Some(Outgoing::Sync(_)) | Some(Outgoing::Broadcast(TableEvent::Thinking(_)))
        ));
        // The TS checks legality before the dedupe ring: a repeat of a move that is no longer
        // legal is refused, and changes nothing.
        let v = c.v();
        let (dup, out) = submit(&mut c, &id, 7, now);
        assert!(matches!(
            dup,
            MoveAck::Rejected {
                code: ErrorCode::Rule(RuleCode::IllegalMove),
                ..
            }
        ));
        assert!(out.is_empty());
        assert_eq!(c.v(), v);
    }

    #[test]
    fn a_remembered_move_id_acks_and_long_ids_are_cut_to_64() {
        let mut c = core("dedupe", false);
        let mut now = 0;
        while c.view().turn_seat != Some(HUMAN) {
            now = c.next_due().unwrap();
            c.fire(now);
        }
        let long = "x".repeat(80);
        c.remember_move_id(truncate_utf16(&long, CLIENT_MOVE_ID_MAX));
        assert_eq!(c.recent_move_ids.back().map(String::len), Some(64));
        let v = c.v();
        let req = SubmitMoveRequest {
            move_id: c.view().legal[0].id.clone(),
            turn_id: Some(c.state.turn_id.clone()),
            client_move_id: long,
        };
        let (ack, out) = c.submit(&req, now);
        assert_eq!(ack, MoveAck::Ok { v }, "the cut id is already in the ring");
        assert!(out.is_empty());
        assert_eq!(
            truncate_utf16("ab\u{1F0A1}c", 3),
            "ab",
            "never splits a character"
        );
    }

    #[test]
    fn chat_precedes_its_sync() {
        let mut c = core("chat", false);
        let (_, out) = play_out(&mut c);
        let first_chat = out
            .iter()
            .position(|o| matches!(o, Outgoing::Broadcast(TableEvent::Chat(_))))
            .unwrap();
        let Outgoing::Broadcast(TableEvent::Chat(chat)) = &out[first_chat] else {
            unreachable!()
        };
        let v: u64 = chat.msg_id.split(':').nth(1).unwrap().parse().unwrap();
        let sync_at = out
            .iter()
            .position(|o| matches!(o, Outgoing::Sync(s) if s.v == v))
            .unwrap();
        assert!(first_chat < sync_at);
    }

    #[test]
    fn think_floors_follow_the_tempo() {
        let t = Tempo::NORMAL.think_ms(ObservationKind::Bid1, "abc");
        assert!((1900..2300).contains(&t));
        assert_eq!(Tempo::INSTANT.think_ms(ObservationKind::Play, "abc"), 0);
        let f = Tempo::FAST.think_ms(ObservationKind::Play, "abc");
        assert!((100..140).contains(&f));
    }

    #[test]
    fn deterministic_seeds_repeat() {
        let a = core("same", false).view();
        let b = core("same", false).view();
        assert_eq!(a, b);
    }

    #[test]
    fn seeded_history_reads_like_a_real_scrapbook() {
        let started = std::time::Instant::now();
        let rows = seeded_history("t", SIM_HISTORY, FIXTURE_NOW_MS);
        let took = started.elapsed();
        assert_eq!(rows.len(), SIM_HISTORY);
        assert!(took.as_secs_f64() < 5.0, "seeding took {took:?}");
        // Newest first, all before now, ids distinct.
        assert!(
            rows.windows(2)
                .all(|w| w[0].summary.played_at > w[1].summary.played_at)
        );
        assert!(rows.iter().all(|m| m.summary.played_at <= FIXTURE_NOW_MS));
        let ids: std::collections::HashSet<_> = rows.iter().map(|m| &m.summary.match_id).collect();
        assert_eq!(ids.len(), rows.len());
        // One match left unfinished, the rest played out; both outcomes show up.
        let abandoned: Vec<_> = rows
            .iter()
            .filter(|m| m.summary.outcome == MatchOutcome::Abandoned)
            .collect();
        assert_eq!(abandoned.len(), 1);
        assert_eq!(abandoned[0].replay.hands.len(), ABANDONED_HANDS as usize);
        assert!(rows.iter().any(|m| m.summary.outcome == MatchOutcome::Won));
        assert!(rows.iter().any(|m| m.summary.outcome == MatchOutcome::Lost));
        for m in &rows {
            let r = &m.replay;
            assert_eq!(r.match_id, m.summary.match_id);
            // The replay lists thrown-in hands too; the count leaves them out.
            assert_eq!(
                r.hands.len(),
                (m.summary.hands_played + r.stats.throw_ins) as usize
            );
            assert_eq!(r.score, m.summary.score.map(i64::from));
            assert_eq!(r.opponents, m.summary.opponents);
            if m.summary.outcome != MatchOutcome::Abandoned {
                assert!(r.score[0] >= 10 || r.score[1] >= 10, "{:?}", r.score);
            }
            let tricks: i64 = r.hands.iter().map(|h| h.tricks_won[0]).sum();
            assert_eq!(i64::from(r.stats.tricks[0]), tricks);
            let delta: i64 = r.hands.iter().filter_map(|h| h.delta).map(|d| d[0]).sum();
            assert_eq!(delta, r.score[0], "hand deltas add up to the score");
        }
        // The same seed plays the same history.
        assert_eq!(seeded_history("t", SIM_HISTORY, FIXTURE_NOW_MS), rows);
    }

    #[test]
    fn history_pages_and_stats_follow_the_rows() {
        let rows = seeded_history("p", SIM_HISTORY, FIXTURE_NOW_MS);
        let first = history_page(&rows, None).unwrap();
        assert_eq!(first.rows.len(), GAMES_PAGE);
        assert_eq!(first.rows[0], rows[0].summary);
        let next = first.cursor.clone().expect("a second page");
        let second = history_page(&rows, Some(&next)).unwrap();
        assert_eq!(second.rows.len(), SIM_HISTORY - GAMES_PAGE);
        assert_eq!(second.cursor, None);
        assert!(history_page(&rows, Some("nope")).is_err());
        let stats = history_stats(&rows, FIXTURE_NOW_MS);
        assert_eq!(stats.matches as usize, SIM_HISTORY);
        assert_eq!(stats.abandoned, 1);
        assert_eq!(stats.won + stats.lost + stats.abandoned, stats.matches);
        let hands: i64 = rows.iter().map(|m| i64::from(m.summary.hands_played)).sum();
        assert_eq!(stats.hands_played, hands);
    }

    #[tokio::test]
    async fn settings_patch_round_trips_and_refuses_bad_values() {
        let backend = SimBackend::new(SimOptions {
            history: 0,
            ..SimOptions::default()
        });
        let before = backend.get_settings().await.unwrap();
        let patch = SettingsPatch {
            house_prompt: Some("Keep it friendly.".to_owned()),
            ..SettingsPatch::default()
        };
        let after = backend.patch_settings(patch).await.unwrap();
        assert_eq!(after.house_prompt, "Keep it friendly.");
        let names = |s: &ProfileSettings| s.personas.clone().map(|p| p.name);
        assert_eq!(names(&after), names(&before));
        assert_eq!(backend.get_settings().await.unwrap(), after);
        let bad_seat = SettingsPatch {
            personas: Some(vec![euchre_profile_model::PersonaPatchEntry {
                seat: 9,
                patch: euchre_profile_model::PersonaPatch::default(),
            }]),
            ..SettingsPatch::default()
        };
        assert!(backend.patch_settings(bad_seat).await.is_err());
        assert_eq!(
            backend.get_settings().await.unwrap(),
            after,
            "a refused patch keeps the old"
        );
        // `history: 0` starts with an empty scrapbook.
        assert!(backend.list_games(None).await.unwrap().rows.is_empty());
    }

    #[tokio::test]
    async fn backend_streams_a_sync_on_connect_and_acks_moves() {
        let backend = SimBackend::new(SimOptions {
            tempo: Tempo::INSTANT,
            ..SimOptions::default()
        });
        let id = backend.create_table(TableConfig::default()).await.unwrap();
        let mut conn = backend.open_table(&id).await.unwrap();
        assert_eq!(conn.events.recv().await, Some(ConnEvent::Status(true)));
        let Some(ConnEvent::Event(TableEvent::Sync(first))) = conn.events.recv().await else {
            panic!("expected a sync");
        };
        assert!(first.steps.is_empty());
        // Wait for the human's turn, then play.
        let mut view = first.view;
        while view.turn_seat != Some(HUMAN) {
            match conn.events.recv().await {
                Some(ConnEvent::Event(TableEvent::Sync(s))) => view = s.view,
                Some(_) => {}
                None => panic!("stream ended"),
            }
        }
        let ack = conn
            .calls
            .submit_move(SubmitMoveRequest {
                move_id: view.legal[0].id.clone(),
                turn_id: Some(view.turn_id.clone()),
                client_move_id: "c-1".into(),
            })
            .await
            .unwrap();
        assert!(ack.is_ok());
        let snap = conn.calls.snapshot().await.unwrap().unwrap();
        assert!(snap.v > view.v);
        assert!(backend.open_table("nope").await.is_err());
        conn.calls.close().await;
    }
}
