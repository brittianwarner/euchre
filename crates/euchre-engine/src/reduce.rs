//! The reducer: the whole game as pure functions over [`GameState`] (port of `reduce.ts`).
//!
//! | Function        | Who calls it                                              |
//! |-----------------|-----------------------------------------------------------|
//! | [`create_game`] | the table actor's create hook, and every test             |
//! | [`start_hand`]  | the table actor's start-hand handler (engine-only)        |
//! | [`apply`]       | **the** seat-move entry point: every human, AI, auto move |
//! | [`advance`]     | the table actor's tick / tempo-gate handler (engine-only) |
//!
//! Three properties hold, as in TS:
//!
//! 1. **Purity (V21).** Inputs are borrowed and never mutated; every result is
//!    freshly allocated.
//! 2. **Determinism (V20).** No clock, no randomness, no ambient input. Even the
//!    turn nonce is a hash chain over the previous nonce (01 §11.2).
//! 3. **Totality.** Every transition of 01 §11.9 is implemented and
//!    [`advance`] always makes progress when no seat is to act.
//!
//! A rules violation is a [`RuleError`]. A broken internal invariant (a state
//! the engine could never have produced) is an [`EngineFault`]: the `try_*`
//! functions return it, the plain functions panic with its message, exactly
//! where the TS engine throws a plain `Error`.

use std::fmt;

use crate::cards::{same_color, suit_of};
use crate::deal::{
    DealHandParams, cut_for_hand, deal_hand, dealt_packets, fnv1a, mulberry32, pick_first_dealer,
    shuffle_for_hand, to_base36,
};
use crate::legal::{
    find_legal_move, get_legal_plays, is_stuck_dealer, legal_moves, order_label, suit_name,
};
use crate::score::{ScoreHandParams, apply_delta, score_hand, winner_team};
use crate::seats::{
    cutter_of, eldest_of, first_active_from, next_active_seat, next_seat, partner_of, team_of,
};
use crate::trick::{is_trick_complete, led_suit_of, trick_winner};
use crate::types::{
    Bid, BidRecord, Card, DEFAULT_ENGINE_CONFIG, EngineConfigOverrides, GamePhase, GameState,
    GameStatus, HandState, Hands, LegalMove, LegalMoveId, Move, Play, RuleCode, SchemaV1, Seat,
    Step, Suit, Team, Trick,
};

/* -------------------------------------------------------------------------- */
/* The reducer contract                                                        */
/* -------------------------------------------------------------------------- */

/// What every transition returns: the next state and the ordered, **unredacted**
/// choreography. Run `steps` through `project_steps` before it leaves the server (V12).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ApplyResult {
    /// The next state.
    pub state: GameState,
    /// What happened, first step first.
    pub steps: Vec<Step>,
}

/// Optional caller-supplied checks that `apply` performs before anything else.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ApplyOptions {
    /// The turn nonce the caller believes is current; a stale one is rejected
    /// with `stale_turn`. `None` skips the nonce check.
    pub turn_id: Option<String>,
}

/// The only error `apply` returns for a rules violation: a typed code plus the
/// mover's actual legal set, so the client can snap back without a round trip.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuleError {
    /// The machine code.
    pub code: RuleCode,
    /// The mover's legal moves at the time of rejection.
    pub legal: Vec<LegalMove>,
}

impl RuleError {
    /// A rule error with an empty legal set (the TS default).
    pub fn new(code: RuleCode) -> RuleError {
        RuleError {
            code,
            legal: Vec::new(),
        }
    }

    /// A rule error carrying the mover's legal set.
    pub fn with_legal(code: RuleCode, legal: Vec<LegalMove>) -> RuleError {
        RuleError { code, legal }
    }
}

impl fmt::Display for RuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code.as_str())
    }
}

impl std::error::Error for RuleError {}

/// A broken engine invariant: a programmer error, never a rules event, never
/// shown to a player. Display is `"euchre engine invariant violated: {message}"`,
/// the TS `Error` message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EngineFault {
    /// What was wrong (the TS `invariant()` message).
    pub message: String,
}

impl fmt::Display for EngineFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "euchre engine invariant violated: {}", self.message)
    }
}

impl std::error::Error for EngineFault {}

/// Why [`try_apply`] refused a move.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ApplyError {
    /// A rules violation: expected, recoverable, shown to a player.
    Rule(RuleError),
    /// A broken invariant: the input state is one the engine never produces.
    Fault(EngineFault),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::Rule(e) => e.fmt(f),
            ApplyError::Fault(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<RuleError> for ApplyError {
    fn from(e: RuleError) -> ApplyError {
        ApplyError::Rule(e)
    }
}

impl From<EngineFault> for ApplyError {
    fn from(e: EngineFault) -> ApplyError {
        ApplyError::Fault(e)
    }
}

/// The inputs of `create_game`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct GameInit {
    /// The match id.
    pub game_id: String,
    /// The match seed. Never reaches a client.
    pub seed: String,
    /// Rule overrides on top of `DEFAULT_ENGINE_CONFIG`.
    pub cfg: Option<EngineConfigOverrides>,
    /// The actor-to-actor secret; derived from the seed when `None`.
    pub internal_token: Option<String>,
    /// Overrides the seeded first-dealer draw (fixtures, scripted tutorial deal).
    pub first_dealer: Option<Seat>,
}

/// The TS `invariant(cond, msg)`.
fn ensure(condition: bool, message: impl FnOnce() -> String) -> Result<(), EngineFault> {
    if condition {
        Ok(())
    } else {
        Err(EngineFault { message: message() })
    }
}

/// Unwraps an infallible-in-practice result, panicking with the fault message
/// exactly where TS would throw.
fn or_panic<T>(r: Result<T, EngineFault>) -> T {
    match r {
        Ok(v) => v,
        Err(fault) => panic!("{fault}"),
    }
}

/* -------------------------------------------------------------------------- */
/* Move identity                                                               */
/* -------------------------------------------------------------------------- */

/// The stable wire id of a move (`moveId` in TS); the exact inverse of `LegalMove.mv`.
pub fn move_id(mv: &Move) -> LegalMoveId {
    mv.id()
}

/* -------------------------------------------------------------------------- */
/* The table script                                                            */
/* -------------------------------------------------------------------------- */

/// The verbatim line spoken at the table for one bid (`sayForBid` in TS).
///
/// Round 1: the dealer's fourth pass is `"Turn it down."`. Round 2: the
/// turned-down suit's colour mate is `"Next."`, the other colour is
/// `"Crossing the creek."`, and the stuck dealer says `"<Suit>, I guess."`.
/// A loner appends `" Alone."` after the period.
pub fn say_for_bid(state: &GameState, seat: Seat, mv: &Bid) -> String {
    let h = &state.hand;
    let alone = if mv.is_alone() { " Alone." } else { "" };

    match *mv {
        Bid::Pass => {
            let turning_down =
                h.phase == GamePhase::BidRound1 && seat == h.dealer_seat && h.passes == 3;
            if turning_down {
                "Turn it down.".to_owned()
            } else {
                "Pass.".to_owned()
            }
        }
        Bid::OrderUp { .. } => format!("{}.{alone}", order_label(seat, h.dealer_seat)),
        Bid::Call { suit, .. } => {
            if is_stuck_dealer(state, seat) {
                return format!("{}, I guess.{alone}", suit_name(suit));
            }
            match h.turned_down_suit {
                Some(down) if same_color(suit, down) => format!("Next.{alone}"),
                Some(_) => format!("Crossing the creek.{alone}"),
                None => format!("{}.{alone}", suit_name(suit)),
            }
        }
    }
}

/* -------------------------------------------------------------------------- */
/* Immutable update helpers                                                    */
/* -------------------------------------------------------------------------- */

/// A fresh turn nonce minted from the previous one and the private token.
fn mint_turn_id(state: &GameState) -> String {
    let a = fnv1a(&format!("{}#{}#a", state.internal_token, state.turn_id));
    let b = fnv1a(&format!("{}#{}#b", state.internal_token, state.turn_id));
    format!("{}{}", to_base36(a), to_base36(b))
}

/// A new state with `change` applied and the turn nonce advanced exactly once
/// (minted from the **old** nonce). Never mutates `state`.
fn next(state: &GameState, change: impl FnOnce(&mut GameState)) -> GameState {
    let mut out = state.clone();
    change(&mut out);
    out.turn_id = mint_turn_id(state);
    out
}

/// A new state whose hand is `hand`, with the nonce advanced.
fn next_hand(state: &GameState, hand: HandState) -> GameState {
    next(state, |s| s.hand = hand)
}

/// A fresh hands record with one seat replaced.
fn replace_hand(hands: &Hands, seat: Seat, cards: Vec<Card>) -> Hands {
    let mut out = hands.clone();
    out[seat.index()] = cards;
    out
}

/// `cards` without the first occurrence of `card`.
fn remove_card(cards: &[Card], card: Card) -> Result<Vec<Card>, EngineFault> {
    let Some(i) = cards.iter().position(|&c| c == card) else {
        return Err(EngineFault {
            message: format!("removeCard: {card} is not held"),
        });
    };
    let mut out = cards.to_vec();
    out.remove(i);
    Ok(out)
}

/// The four undealt cards of a permutation: `deck_order[20..24]`.
fn kitty_of(deck_order: &[Card]) -> Result<[Card; 4], EngineFault> {
    match deck_order {
        [
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            a,
            b,
            c,
            d,
        ] => Ok([*a, *b, *c, *d]),
        _ => Err(EngineFault {
            message: "kittyOf: deckOrder must hold 24 cards".to_owned(),
        }),
    }
}

/// A trick index one past `index` (tricks are `0..=4`, so this never saturates in play).
fn next_trick_index(index: u8) -> u8 {
    index.saturating_add(1)
}

/* -------------------------------------------------------------------------- */
/* Construction                                                                */
/* -------------------------------------------------------------------------- */

/// A deterministic pseudo-token derived from the seed. See [`GameInit`].
fn derive_token(seed: &str, game_id: &str) -> String {
    (0..4)
        .map(|i| to_base36(fnv1a(&format!("{seed}#{game_id}#token#{i}"))))
        .collect()
}

/// The `HandState` every new hand starts from: nothing dealt, nothing bid.
struct FreshHand {
    hand_no: u32,
    phase: GamePhase,
    dealer_seat: Seat,
    turn_seat: Option<Seat>,
    deck_order: Vec<Card>,
}

impl FreshHand {
    fn build(self) -> Result<HandState, EngineFault> {
        let kitty = kitty_of(&self.deck_order)?;
        Ok(HandState {
            hand_no: self.hand_no,
            phase: self.phase,
            dealer_seat: self.dealer_seat,
            turn_seat: self.turn_seat,
            deck_order: self.deck_order,
            cut: None,
            hands: Default::default(),
            // Provisional: the cut rotates the permutation, so the deal re-derives both.
            kitty,
            up_card: None,
            up_card_turned_down: false,
            turned_down_suit: None,
            trump: None,
            maker_seat: None,
            alone_seat: None,
            sitting_seat: None,
            dealer_discard: None,
            passes: 0,
            bids: Vec::new(),
            trick: Trick::empty(0),
            trick_log: Vec::new(),
            tricks_won: [0, 0],
            result: None,
            delta: None,
        })
    }
}

/// [`create_game`], returning an [`EngineFault`] instead of panicking.
pub fn try_create_game(init: GameInit) -> Result<GameState, EngineFault> {
    let cfg = match &init.cfg {
        Some(o) => DEFAULT_ENGINE_CONFIG.with_overrides(o),
        None => DEFAULT_ENGINE_CONFIG,
    };
    let first_dealer = match init.first_dealer {
        Some(d) => d,
        None => pick_first_dealer(&mut mulberry32(fnv1a(&format!(
            "{}#firstDealer",
            init.seed
        )))),
    };
    let hand = FreshHand {
        hand_no: 0,
        phase: GamePhase::Lobby,
        dealer_seat: first_dealer,
        turn_seat: None,
        deck_order: shuffle_for_hand(&init.seed, 0),
    }
    .build()?;

    let internal_token = init
        .internal_token
        .unwrap_or_else(|| derive_token(&init.seed, &init.game_id));
    let turn_id = to_base36(fnv1a(&format!("{}#turn0", init.seed)));

    Ok(GameState {
        schema: SchemaV1,
        v: 0,
        game_id: init.game_id,
        internal_token,
        seed: init.seed,
        cfg,
        first_dealer,
        status: GameStatus::Active,
        score: [0, 0],
        hand_no: 0,
        misdeal_streak: 0,
        winner_team: None,
        hand,
        turn_id,
        turn_deadline_at: None,
        timer_id: None,
        watchdog_id: None,
        reveal_id: None,
        pending: None,
        recent_move_ids: Vec::new(),
        applied_seq: 0,
    })
}

/// The sole constructor: a `lobby` state with nothing dealt and no seat to act.
/// Call [`advance`] (or [`start_hand`]) to open the first hand.
///
/// `first_dealer` is drawn once from a dedicated stream (`"{seed}#firstDealer"`)
/// unless overridden, and the deal moves one seat left after every scored hand
/// and every throw-in.
///
/// # Panics
///
/// Never in practice: the seeded shuffle always yields 24 cards.
pub fn create_game(init: GameInit) -> GameState {
    or_panic(try_create_game(init))
}

/// A match from nothing but a seed: `create_game` with `game_id = "g-{seed}"`.
/// Two calls with the same seed produce identical states.
pub fn new_game(seed: &str, cfg: Option<EngineConfigOverrides>) -> GameState {
    create_game(GameInit {
        game_id: format!("g-{seed}"),
        seed: seed.to_owned(),
        cfg,
        internal_token: None,
        first_dealer: None,
    })
}

/* -------------------------------------------------------------------------- */
/* startHand - lobby | hand_score(next) -> cutting                             */
/* -------------------------------------------------------------------------- */

/// [`start_hand`], returning an [`EngineFault`] instead of panicking.
pub fn try_start_hand(state: &GameState) -> Result<ApplyResult, EngineFault> {
    let dealer_seat = state.hand.dealer_seat;
    let hand = FreshHand {
        hand_no: state.hand_no,
        phase: GamePhase::Cutting,
        dealer_seat,
        turn_seat: Some(cutter_of(dealer_seat)),
        deck_order: shuffle_for_hand(&state.seed, state.hand_no),
    }
    .build()?;
    Ok(ApplyResult {
        state: next_hand(state, hand),
        steps: Vec::new(),
    })
}

/// Opens a hand: shuffle from `(seed, hand_no)`, reset the whole `HandState`
/// and hand the turn to the cutter. Idempotent given `state`; rotating the deal
/// is [`advance`]'s job.
///
/// # Panics
///
/// Never in practice: the seeded shuffle always yields 24 cards.
pub fn start_hand(state: &GameState) -> ApplyResult {
    or_panic(try_start_hand(state))
}

/// The state rotated onto the next hand: counter up, deal one seat left. The
/// `HandState` itself is still the old one; `start_hand` replaces it.
fn rotate_to_next_hand(state: &GameState, misdeal_streak: u8) -> GameState {
    let hand_no = state.hand_no.saturating_add(1);
    next(state, |s| {
        s.hand_no = hand_no;
        s.misdeal_streak = misdeal_streak;
        s.hand.hand_no = hand_no;
        s.hand.dealer_seat = next_seat(state.hand.dealer_seat);
    })
}

/* -------------------------------------------------------------------------- */
/* deal - engine-only, atomic                                                  */
/* -------------------------------------------------------------------------- */

/// Deals from the persisted (and possibly cut) permutation and turns the up-card.
fn run_deal(state: &GameState) -> Result<ApplyResult, EngineFault> {
    let h = &state.hand;
    let dealt = deal_hand(DealHandParams {
        deck_order: &h.deck_order,
        dealer_seat: h.dealer_seat,
        seed: &state.seed,
        hand_no: h.hand_no,
    })
    .map_err(|e| EngineFault {
        message: e.to_string(),
    })?;

    let mut hand = h.clone();
    hand.phase = GamePhase::BidRound1;
    hand.turn_seat = Some(eldest_of(h.dealer_seat));
    hand.hands = dealt.hands;
    hand.kitty = dealt.kitty;
    hand.up_card = Some(dealt.up_card);
    hand.passes = 0;
    hand.bids = Vec::new();
    hand.trick = Trick::empty(0);

    Ok(ApplyResult {
        state: next_hand(state, hand),
        steps: vec![
            Step::Dealt {
                packets: dealt_packets(&h.deck_order, &dealt.plan),
            },
            Step::UpCardTurned {
                card: dealt.up_card,
            },
        ],
    })
}

/* -------------------------------------------------------------------------- */
/* apply - THE seat move                                                       */
/* -------------------------------------------------------------------------- */

/// Steps A to D of the validation ladder (01 §11.6), in the exact TS order.
fn validate(
    state: &GameState,
    seat: Seat,
    mv: &Move,
    opts: Option<&ApplyOptions>,
) -> Result<(), RuleError> {
    let h = &state.hand;
    let reject = |code: RuleCode| Err(RuleError::with_legal(code, legal_moves(state, seat)));

    // Who may speak at all.
    if state.status != GameStatus::Active {
        return reject(RuleCode::WrongPhase);
    }
    if let Some(turn_id) = opts.and_then(|o| o.turn_id.as_deref())
        && turn_id != state.turn_id
    {
        return reject(RuleCode::StaleTurn);
    }
    if Some(seat) == h.sitting_seat {
        return reject(RuleCode::SittingOut);
    }
    match h.turn_seat {
        None => return reject(RuleCode::WrongPhase),
        Some(t) if t != seat => return reject(RuleCode::NotYourTurn),
        Some(_) => {}
    }

    // Does this move kind belong to this phase (V1, V2).
    match h.phase {
        GamePhase::Cutting => {
            if !matches!(mv, Move::Cut { .. }) {
                return reject(RuleCode::WrongPhase);
            }
        }
        GamePhase::BidRound1 => {
            if !matches!(mv, Move::Pass | Move::OrderUp { .. }) {
                return reject(RuleCode::WrongPhase);
            }
        }
        GamePhase::BidRound2 => {
            if !matches!(mv, Move::Pass | Move::Call { .. }) {
                return reject(RuleCode::WrongPhase);
            }
        }
        GamePhase::DealerDiscard => {
            if mv.is_bid() {
                return reject(RuleCode::BiddingClosed);
            }
            if !matches!(mv, Move::Discard { .. }) {
                return reject(RuleCode::WrongPhase);
            }
        }
        GamePhase::TrickPlay => {
            if mv.is_bid() {
                return reject(RuleCode::BiddingClosed);
            }
            if !matches!(mv, Move::Play { .. }) {
                return reject(RuleCode::WrongPhase);
            }
        }
        GamePhase::Lobby
        | GamePhase::Deal
        | GamePhase::TrickResolve
        | GamePhase::HandScore
        | GamePhase::GameOver => return reject(RuleCode::WrongPhase),
    }

    // The specific codes, most specific first.
    let held = &h.hands[seat.index()];
    match *mv {
        Move::Pass if is_stuck_dealer(state, seat) => return reject(RuleCode::DealerMustCall),
        Move::Call { suit, .. } if Some(suit) == h.turned_down_suit => {
            return reject(RuleCode::IllegalMove);
        }
        Move::Discard { card } if !held.contains(&card) => {
            return reject(RuleCode::CardNotInHand);
        }
        Move::Play { card } => {
            if !held.contains(&card) {
                return reject(RuleCode::CardNotInHand);
            }
            if !get_legal_plays(held, h.trick.led_suit, h.trump).contains(&card) {
                return reject(RuleCode::MustFollowSuit);
            }
        }
        _ => {}
    }

    // And the backstop: it must be in the legal set, full stop.
    if find_legal_move(state, seat, &move_id(mv)).is_none() {
        return reject(RuleCode::IllegalMove);
    }
    Ok(())
}

/// [`apply`], returning an [`EngineFault`] (inside [`ApplyError`]) instead of
/// panicking when handed a state the engine could never have produced.
pub fn try_apply(
    state: &GameState,
    seat: Seat,
    mv: Move,
    opts: Option<&ApplyOptions>,
) -> Result<ApplyResult, ApplyError> {
    validate(state, seat, &mv, opts)?;
    let out = match mv {
        Move::Cut { cut } => apply_cut(state, seat, cut)?,
        Move::Pass => {
            if state.hand.phase == GamePhase::BidRound1 {
                apply_pass_round1(state, seat)?
            } else {
                apply_pass_round2(state, seat)?
            }
        }
        Move::OrderUp { alone } => apply_order_up(state, seat, alone)?,
        Move::Call { suit, alone } => apply_call(state, seat, suit, alone),
        Move::Discard { card } => apply_discard(state, seat, card)?,
        Move::Play { card } => apply_play(state, seat, card)?,
    };
    Ok(out)
}

/// Applies one seat's move: the single mutation entry point for everything a
/// player, an AI or the auto-play ladder can do.
///
/// `seat` is the caller's claim about who is acting and is **validated**: it
/// must equal `hand.turn_seat`, must not be the loner's sitting partner, and the
/// move must be in `legal_moves(state, seat)`. Returns a [`RuleError`] with the
/// most specific [`RuleCode`] (01 §11.6) and the seat's legal set otherwise.
/// Pure: `state` is never mutated.
///
/// # Panics
///
/// Only on a state the engine could never have produced (a programmer error,
/// the TS plain `Error`). Use [`try_apply`] to get an [`EngineFault`] instead.
pub fn apply(
    state: &GameState,
    seat: Seat,
    mv: Move,
    opts: Option<&ApplyOptions>,
) -> Result<ApplyResult, RuleError> {
    match try_apply(state, seat, mv, opts) {
        Ok(r) => Ok(r),
        Err(ApplyError::Rule(e)) => Err(e),
        Err(ApplyError::Fault(fault)) => panic!("{fault}"),
    }
}

/* -------------------------------------------------------------------------- */
/* cutting                                                                     */
/* -------------------------------------------------------------------------- */

/// "Bump." or "Run 'em." The cut rotates the persisted permutation at an index
/// derived from `(seed, hand_no)`, then the deal follows atomically.
fn apply_cut(state: &GameState, seat: Seat, cut: bool) -> Result<ApplyResult, EngineFault> {
    let h = &state.hand;
    let deck_order = if cut {
        cut_for_hand(&h.deck_order, &state.seed, h.hand_no)
    } else {
        h.deck_order.clone()
    };

    let mut hand = h.clone();
    hand.cut = Some(cut);
    hand.kitty = kitty_of(&deck_order)?;
    hand.deck_order = deck_order;
    hand.phase = GamePhase::Deal;
    hand.turn_seat = None;
    let after_cut = next_hand(state, hand);
    let dealt = run_deal(&after_cut)?;

    let mut steps = Vec::with_capacity(1 + dealt.steps.len());
    steps.push(Step::Cut { seat, cut });
    steps.extend(dealt.steps);
    Ok(ApplyResult {
        state: dealt.state,
        steps,
    })
}

/* -------------------------------------------------------------------------- */
/* bidding                                                                     */
/* -------------------------------------------------------------------------- */

/// `bids` with one auction entry appended. Bounded at 8: four seats by two rounds.
fn record_bid(h: &HandState, seat: Seat, mv: Bid, say: &str) -> Vec<BidRecord> {
    let mut bids = h.bids.clone();
    bids.push(BidRecord {
        seat,
        mv,
        say: say.to_owned(),
    });
    bids
}

/// A `bid` step, which the talk log and seat bubbles animate from.
fn bid_step(seat: Seat, mv: Bid, say: String) -> Step {
    Step::Bid {
        seat,
        id: mv.id(),
        say,
    }
}

/// Round 1 pass. On the fourth the dealer turns it down and round 2 opens at eldest.
fn apply_pass_round1(state: &GameState, seat: Seat) -> Result<ApplyResult, EngineFault> {
    let h = &state.hand;
    let mv = Bid::Pass;
    let say = say_for_bid(state, seat, &mv);
    let bids = record_bid(h, seat, mv, &say);
    let passes = h.passes.saturating_add(1);

    if passes < 4 {
        let mut hand = h.clone();
        hand.passes = passes;
        hand.bids = bids;
        hand.turn_seat = Some(next_seat(seat));
        return Ok(ApplyResult {
            state: next_hand(state, hand),
            steps: vec![bid_step(seat, mv, say)],
        });
    }

    let Some(up_card) = h.up_card else {
        return Err(EngineFault {
            message: "round 1 ended with no up-card".to_owned(),
        });
    };
    let turned_down_suit = suit_of(up_card);
    let mut hand = h.clone();
    hand.phase = GamePhase::BidRound2;
    hand.turn_seat = Some(eldest_of(h.dealer_seat));
    hand.up_card_turned_down = true;
    hand.turned_down_suit = Some(turned_down_suit);
    hand.passes = 0;
    hand.bids = bids;
    Ok(ApplyResult {
        state: next_hand(state, hand),
        steps: vec![
            bid_step(seat, mv, say),
            Step::TurnedDown {
                suit: turned_down_suit,
            },
        ],
    })
}

/// Round 2 pass. With stick-the-dealer off, the fourth throws the hand in.
fn apply_pass_round2(state: &GameState, seat: Seat) -> Result<ApplyResult, EngineFault> {
    let h = &state.hand;
    let mv = Bid::Pass;
    let say = say_for_bid(state, seat, &mv);
    let bids = record_bid(h, seat, mv, &say);
    let passes = h.passes.saturating_add(1);

    if passes < 4 {
        let mut hand = h.clone();
        hand.passes = passes;
        hand.bids = bids;
        hand.turn_seat = Some(next_seat(seat));
        return Ok(ApplyResult {
            state: next_hand(state, hand),
            steps: vec![bid_step(seat, mv, say)],
        });
    }

    // Exhaustion: nobody scores, the streak grows, the deal moves left.
    ensure(!state.cfg.stick_the_dealer, || {
        "four round-2 passes with stick-the-dealer on".to_owned()
    })?;
    let mut hand = h.clone();
    hand.passes = passes;
    hand.bids = bids;
    let thrown_in = next_hand(state, hand);
    let streak = state
        .cfg
        .misdeal_limit
        .min(state.misdeal_streak.saturating_add(1));
    let opened = try_start_hand(&rotate_to_next_hand(&thrown_in, streak))?;

    let mut steps = vec![bid_step(seat, mv, say), Step::ThrowIn];
    steps.extend(opened.steps);
    Ok(ApplyResult {
        state: opened.state,
        steps,
    })
}

/// The shared tail of every successful call: trump, maker and (for a loner) the
/// sitting partner all land in the same transition (V4).
fn set_trump(h: &HandState, suit: Suit, maker_seat: Seat, alone: bool) -> (HandState, Step) {
    let alone_seat = alone.then_some(maker_seat);
    let sitting_seat = alone.then(|| partner_of(maker_seat));
    let mut hand = h.clone();
    hand.trump = Some(suit);
    hand.maker_seat = Some(maker_seat);
    hand.alone_seat = alone_seat;
    hand.sitting_seat = sitting_seat;
    let step = Step::TrumpSet {
        suit,
        maker_seat,
        alone_seat,
        sitting_seat,
        via_order_up: false,
    };
    (hand, step)
}

/// The trick-1 lead: eldest, or the next active seat if eldest is sitting out.
fn opening_lead(h: &HandState) -> Seat {
    first_active_from(eldest_of(h.dealer_seat), h.sitting_seat)
}

/// Round 1 acceptance: the up-card's suit is trump and the dealer picks it up,
/// always routing to `dealer_discard` unless the dealer is the sitting partner.
fn apply_order_up(state: &GameState, seat: Seat, alone: bool) -> Result<ApplyResult, EngineFault> {
    let h = &state.hand;
    let Some(up_card) = h.up_card else {
        return Err(EngineFault {
            message: "orderUp with no up-card".to_owned(),
        });
    };

    let mv = Bid::OrderUp { alone };
    let say = say_for_bid(state, seat, &mv);
    let bids = record_bid(h, seat, mv, &say);

    let (mut hand, mut trump_set) = set_trump(h, suit_of(up_card), seat, alone);
    if let Step::TrumpSet { via_order_up, .. } = &mut trump_set {
        *via_order_up = true;
    }
    let mut steps = vec![bid_step(seat, mv, say), trump_set];

    if hand.sitting_seat == Some(h.dealer_seat) {
        // A seated-out dealer cannot discard: the pickup and the discard collapse
        // into the canonical no-op, leaving the dead hand at five.
        hand.phase = GamePhase::TrickPlay;
        hand.turn_seat = Some(opening_lead(&hand));
        hand.bids = bids;
        hand.dealer_discard = Some(up_card);
        hand.trick = Trick::empty(0);
        steps.push(Step::DealerDiscarded {
            seat: h.dealer_seat,
        });
        return Ok(ApplyResult {
            state: next_hand(state, hand),
            steps,
        });
    }

    let mut dealer_hand = h.hands[h.dealer_seat.index()].clone();
    dealer_hand.push(up_card);
    hand.phase = GamePhase::DealerDiscard;
    hand.turn_seat = Some(h.dealer_seat);
    hand.bids = bids;
    hand.hands = replace_hand(&h.hands, h.dealer_seat, dealer_hand);
    Ok(ApplyResult {
        state: next_hand(state, hand),
        steps,
    })
}

/// Round 2 call: no pickup, no discard, play begins immediately.
fn apply_call(state: &GameState, seat: Seat, suit: Suit, alone: bool) -> ApplyResult {
    let h = &state.hand;
    let mv = Bid::Call { suit, alone };
    let say = say_for_bid(state, seat, &mv);
    let bids = record_bid(h, seat, mv, &say);

    let (mut hand, trump_set) = set_trump(h, suit, seat, alone);
    hand.phase = GamePhase::TrickPlay;
    hand.turn_seat = Some(opening_lead(&hand));
    hand.bids = bids;
    hand.trick = Trick::empty(0);

    ApplyResult {
        state: next_hand(state, hand),
        steps: vec![bid_step(seat, mv, say), trump_set],
    }
}

/* -------------------------------------------------------------------------- */
/* dealer_discard                                                              */
/* -------------------------------------------------------------------------- */

/// The dealer lays one of six face down. The step carries only the seat (V12).
fn apply_discard(state: &GameState, seat: Seat, card: Card) -> Result<ApplyResult, EngineFault> {
    let h = &state.hand;
    let remaining = remove_card(&h.hands[seat.index()], card)?;
    ensure(remaining.len() == 5, || {
        "dealer hand is not five cards after the discard".to_owned()
    })?;

    let mut hand = h.clone();
    hand.phase = GamePhase::TrickPlay;
    hand.turn_seat = Some(opening_lead(h));
    hand.hands = replace_hand(&h.hands, seat, remaining);
    hand.dealer_discard = Some(card);
    hand.trick = Trick::empty(0);

    Ok(ApplyResult {
        state: next_hand(state, hand),
        steps: vec![Step::DealerDiscarded { seat }],
    })
}

/* -------------------------------------------------------------------------- */
/* trick_play                                                                  */
/* -------------------------------------------------------------------------- */

/// Lays a card. When the last active seat has played the trick resolves and
/// the machine parks in `trick_resolve`.
fn apply_play(state: &GameState, seat: Seat, card: Card) -> Result<ApplyResult, EngineFault> {
    let h = &state.hand;
    let Some(trump) = h.trump else {
        return Err(EngineFault {
            message: "a card was played before trump was set".to_owned(),
        });
    };

    let hands = replace_hand(&h.hands, seat, remove_card(&h.hands[seat.index()], card)?);
    let mut plays = h.trick.plays.clone();
    plays.push(Play { seat, card });
    let Some(led_suit) = led_suit_of(&plays, Some(trump)) else {
        return Err(EngineFault {
            message: "a trick with plays has no led suit".to_owned(),
        });
    };
    let played = Step::CardPlayed { seat, card };

    if !is_trick_complete(&plays, h.sitting_seat) {
        let mut hand = h.clone();
        hand.hands = hands;
        hand.trick = Trick {
            index: h.trick.index,
            led_suit: Some(led_suit),
            plays,
            winner_seat: h.trick.winner_seat,
        };
        hand.turn_seat = Some(next_active_seat(seat, h.sitting_seat));
        return Ok(ApplyResult {
            state: next_hand(state, hand),
            steps: vec![played],
        });
    }

    let winner_seat = trick_winner(&plays, led_suit, trump).map_err(|e| EngineFault {
        message: e.to_string(),
    })?;
    ensure(Some(winner_seat) != h.sitting_seat, || {
        "the sitting seat took a trick (V9)".to_owned()
    })?;

    let mut tricks_won = h.tricks_won;
    let w = team_of(winner_seat).index();
    tricks_won[w] = tricks_won[w].saturating_add(1);

    let trick = Trick {
        index: h.trick.index,
        led_suit: Some(led_suit),
        plays: plays.clone(),
        winner_seat: Some(winner_seat),
    };
    let Some(maker) = h.maker_seat else {
        return Err(EngineFault {
            message: "a trick resolved with no maker".to_owned(),
        });
    };
    let defenders = 1 - team_of(maker).index();
    let seals_euchre = tricks_won[defenders] == 3;

    let mut hand = h.clone();
    hand.phase = GamePhase::TrickResolve;
    hand.turn_seat = None;
    hand.hands = hands;
    hand.trick = trick.clone();
    hand.trick_log.push(trick);
    hand.tricks_won = tricks_won;

    Ok(ApplyResult {
        state: next_hand(state, hand),
        steps: vec![
            played,
            Step::TrickWon {
                index: h.trick.index,
                winner_seat,
                plays,
                seals_euchre,
            },
        ],
    })
}

/* -------------------------------------------------------------------------- */
/* advance - the engine-only transitions                                       */
/* -------------------------------------------------------------------------- */

/// [`advance`], returning an [`EngineFault`] instead of panicking.
pub fn try_advance(state: &GameState) -> Result<ApplyResult, EngineFault> {
    let identity = || ApplyResult {
        state: state.clone(),
        steps: Vec::new(),
    };
    if state.status != GameStatus::Active {
        return Ok(identity());
    }
    match state.hand.phase {
        GamePhase::Lobby => try_start_hand(state),
        GamePhase::Deal => run_deal(state),
        GamePhase::TrickResolve => advance_trick_resolve(state),
        GamePhase::HandScore => advance_hand_score(state),
        GamePhase::Cutting
        | GamePhase::BidRound1
        | GamePhase::DealerDiscard
        | GamePhase::BidRound2
        | GamePhase::TrickPlay
        | GamePhase::GameOver => Ok(identity()),
    }
}

/// Advances out of a phase with no acting seat. Total: the identity (an equal
/// state, no steps) in any phase with a seat to act or once the match is over,
/// and always progress otherwise, so the machine cannot stall.
///
/// - `lobby` opens the first hand; `deal` deals (defensive);
/// - `trick_resolve` gives the lead to the winner or scores the hand;
/// - `hand_score` ends the match or moves the deal left and opens a hand.
///
/// # Panics
///
/// Only on a state the engine could never have produced. Use [`try_advance`]
/// to get an [`EngineFault`] instead.
pub fn advance(state: &GameState) -> ApplyResult {
    or_panic(try_advance(state))
}

/// Releases the trick-resolve hold: the winner leads, or the hand is scored.
fn advance_trick_resolve(state: &GameState) -> Result<ApplyResult, EngineFault> {
    let h = &state.hand;
    let Some(winner_seat) = h.trick.winner_seat else {
        return Err(EngineFault {
            message: "trick_resolve with an unresolved trick".to_owned(),
        });
    };

    if h.trick_log.len() < 5 {
        let mut hand = h.clone();
        hand.phase = GamePhase::TrickPlay;
        hand.turn_seat = Some(winner_seat);
        hand.trick = Trick::empty(next_trick_index(h.trick.index));
        return Ok(ApplyResult {
            state: next_hand(state, hand),
            steps: Vec::new(),
        });
    }
    enter_hand_score(state)
}

/// Scores the hand (post-clamp delta) and parks in `hand_score` for the recap.
fn enter_hand_score(state: &GameState) -> Result<ApplyResult, EngineFault> {
    let h = &state.hand;
    let Some(maker_seat) = h.maker_seat else {
        return Err(EngineFault {
            message: "a hand was scored with no maker".to_owned(),
        });
    };
    ensure(h.trick_log.len() == 5, || {
        "a hand was scored before five tricks".to_owned()
    })?;

    let outcome = score_hand(ScoreHandParams {
        maker_seat,
        alone_seat: h.alone_seat,
        tricks_won: h.tricks_won,
        score: state.score,
        game_to: state.cfg.game_to,
    });
    let score = apply_delta(state.score, outcome.delta);

    let mut hand = h.clone();
    hand.phase = GamePhase::HandScore;
    hand.turn_seat = None;
    hand.result = Some(outcome.result);
    hand.delta = Some(outcome.delta);
    let winner: Option<Team> = winner_team(score, state.cfg.game_to);

    Ok(ApplyResult {
        state: next(state, |s| {
            s.hand = hand;
            s.score = score;
            s.winner_team = winner;
        }),
        steps: vec![Step::HandScored {
            result: outcome.result,
            delta: outcome.delta,
            score,
            maker_seat: Some(maker_seat),
            alone_seat: h.alone_seat,
            tricks_won: h.tricks_won,
        }],
    })
}

/// Releases the recap hold: game over, or the next hand opens with the streak cleared.
fn advance_hand_score(state: &GameState) -> Result<ApplyResult, EngineFault> {
    if let Some(team) = state.winner_team {
        let mut hand = state.hand.clone();
        hand.phase = GamePhase::GameOver;
        hand.turn_seat = None;
        return Ok(ApplyResult {
            state: next(state, |s| {
                s.hand = hand;
                s.status = GameStatus::Complete;
            }),
            steps: vec![Step::GameWon {
                team,
                score: state.score,
            }],
        });
    }
    try_start_hand(&rotate_to_next_hand(state, 0))
}
