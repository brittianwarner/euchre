//! The deterministic euchre judgement engine (`ai/heuristic.ts`).
//!
//! Pure and total: no clock, no randomness, no I/O, no mutation of its
//! arguments. It never decides what is legal; it only orders the legal set the
//! engine already computed. Four consumers share it: the model's prior, the
//! decision ladder's fallback, coach hints and the table's watchdog auto-play
//! (ARCHITECTURE D8), so it must never panic and never name a move outside
//! `legal`.
//!
//! Every floating-point expression keeps the TS operation order so scores are
//! bit-identical to the TS engine (the differential test checks this).

use euchre_engine::{
    Card, DEFAULT_ENGINE_CONFIG, LegalMove, LegalMoveId, PublicGameView, Rank, SUITS, Seat,
    SeatRole, Suit, Team, card_value, color_mate, currently_winning, effective_suit, partner_of,
    plain_rank, plays_per_trick, rank_of, same_color, seat_role, suit_of, team_of, trump_rank,
};
use euchre_protocol::{PersonaConfig, RankedMove};

/* ========================================================================== */
/* Tunables                                                                    */
/* ========================================================================== */

/// Value of a trump card keyed by `trump_rank` (right 8 .. nine 2).
fn trump_value(rank: u8) -> f64 {
    match rank {
        8 => 1.0,
        7 => 0.9,
        6 => 0.8,
        5 => 0.5,
        4 => 0.35,
        3 => 0.15,
        2 => 0.1,
        _ => 0.0,
    }
}

/// Value of an off-suit card by rank.
fn off_value(rank: Rank) -> f64 {
    match rank {
        Rank::Ace => 0.7,
        Rank::King => 0.3,
        Rank::Queen => 0.1,
        Rank::Jack | Rank::Ten | Rank::Nine => 0.0,
    }
}

/// Round-1 order-up thresholds by seat role.
fn order_up_threshold(role: SeatRole) -> f64 {
    match role {
        SeatRole::Dealer => 2.3,
        SeatRole::Eldest => 2.6,
        SeatRole::DealerPartner => 2.2,
        SeatRole::Third => 2.55,
    }
}

/// Round-2 threshold for calling next.
fn next_threshold(role: SeatRole) -> f64 {
    match role {
        SeatRole::Dealer => 2.4,
        SeatRole::Eldest => 2.35,
        SeatRole::DealerPartner => 2.7,
        SeatRole::Third => 2.4,
    }
}

/// Round-2 threshold for crossing the creek.
fn cross_threshold(role: SeatRole) -> f64 {
    match role {
        SeatRole::Dealer => 2.5,
        SeatRole::Eldest => 2.7,
        SeatRole::DealerPartner => 2.7,
        SeatRole::Third => 2.75,
    }
}

/// Loner threshold, either round.
fn alone_threshold(role: SeatRole) -> f64 {
    match role {
        SeatRole::Dealer => 4.0,
        SeatRole::Eldest => 4.3,
        SeatRole::DealerPartner => 4.2,
        SeatRole::Third => 4.3,
    }
}

/// How far a full-throttle `aggression` dial may move a bidding threshold.
const AGGRESSION_SWING: f64 = 0.5;
/// How far a full-throttle `risk` dial may move the loner threshold.
const RISK_SWING: f64 = 0.6;
/// Separates two otherwise-equal scores.
const EPS: f64 = 1e-6;

/* ========================================================================== */
/* Public options                                                              */
/* ========================================================================== */

/// Persona flavouring (`HeuristicOptions`). Every default is neutral.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeuristicOptions {
    /// `0..=1`, default `0.5`. Above `0.5` lowers the bidding thresholds.
    pub aggression: Option<f64>,
    /// `0..=1`, default `0.5`. Above `0.5` lowers the loner threshold.
    pub risk: Option<f64>,
    /// Points to win; defaults to `DEFAULT_ENGINE_CONFIG.game_to` (10).
    pub game_to: Option<f64>,
}

impl HeuristicOptions {
    /// The neutral, persona-free options.
    pub const NEUTRAL: HeuristicOptions = HeuristicOptions {
        aggression: None,
        risk: None,
        game_to: None,
    };

    /// `{ aggression: persona.aggression, risk: persona.risk }`, as `rankingFor` builds it.
    pub fn from_persona(persona: &PersonaConfig) -> HeuristicOptions {
        HeuristicOptions {
            aggression: Some(persona.aggression),
            risk: Some(persona.risk),
            game_to: None,
        }
    }
}

/// Which bidding round a strength evaluation is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrengthPhase {
    /// Adds the up-card term.
    Round1,
    /// Adds next / cross.
    Round2,
    /// Adds neither.
    None,
}

/// The context a strength evaluation needs beyond the cards (`StrengthCtx`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrengthCtx {
    /// The evaluating seat.
    pub seat: Seat,
    /// The dealer.
    pub dealer_seat: Seat,
    /// Which round.
    pub phase: StrengthPhase,
    /// The up-card.
    pub up_card: Option<Card>,
    /// The turned-down suit.
    pub turned_down_suit: Option<Suit>,
    /// This seat's team score.
    pub my_score: f64,
    /// The opponents' score.
    pub opp_score: f64,
    /// Points to win.
    pub game_to: f64,
}

/* ========================================================================== */
/* Small helpers                                                               */
/* ========================================================================== */

fn clamp01(n: f64) -> f64 {
    n.clamp(0.0, 1.0)
}

/// `true` when this card counts as trump (bower-aware).
fn is_trump_card(card: Card, trump: Suit) -> bool {
    effective_suit(card, Some(trump)) == trump
}

/// The seven trump under `trump`, highest first.
fn trump_cards(trump: Suit) -> [Card; 7] {
    let mate = color_mate(trump);
    [
        Card::new(Rank::Jack, trump),
        Card::new(Rank::Jack, mate),
        Card::new(Rank::Ace, trump),
        Card::new(Rank::King, trump),
        Card::new(Rank::Queen, trump),
        Card::new(Rank::Ten, trump),
        Card::new(Rank::Nine, trump),
    ]
}

/// Power inside the current trick, `0..=1`.
fn power(card: Card, trump: Suit) -> f64 {
    if is_trump_card(card, trump) {
        f64::from(trump_rank(card, trump)) / 8.0
    } else {
        f64::from(plain_rank(card)) / 6.0
    }
}

/// Every card visible this hand: completed tricks plus the live one, deduped by index.
fn played_cards(view: &PublicGameView) -> Vec<Card> {
    let mut out = Vec::with_capacity(20);
    let live = view.trick.index;
    for t in &view.trick_log {
        if t.index == live {
            continue;
        }
        out.extend(t.plays.iter().map(|p| p.card));
    }
    out.extend(view.trick.plays.iter().map(|p| p.card));
    out
}

/// Off-suit cards grouped by effective suit, in first-seen order (a JS `Map`).
fn group_off_suits(hand: &[Card], trump: Suit) -> (usize, Vec<(Suit, Vec<Card>)>) {
    let mut trump_count = 0usize;
    let mut groups: Vec<(Suit, Vec<Card>)> = Vec::with_capacity(3);
    for &c in hand {
        let es = effective_suit(c, Some(trump));
        if es == trump {
            trump_count += 1;
            continue;
        }
        match groups.iter_mut().find(|(s, _)| *s == es) {
            Some((_, cards)) => cards.push(c),
            None => groups.push((es, vec![c])),
        }
    }
    (trump_count, groups)
}

/* ========================================================================== */
/* Hand strength                                                               */
/* ========================================================================== */

/// The structural value of a holding under `trump` (`rawHandValue`).
pub fn raw_hand_value(hand: &[Card], trump: Suit) -> f64 {
    let mut s = 0.0f64;
    // Trump values are added in hand order, before any off-suit value.
    for &c in hand {
        if effective_suit(c, Some(trump)) == trump {
            s += trump_value(trump_rank(c, trump));
        }
    }
    let (trump_count, groups) = group_off_suits(hand, trump);
    for (_, cards) in &groups {
        for &c in cards {
            let v = off_value(rank_of(c));
            s += if cards.len() == 1 { v * 0.5 } else { v };
        }
    }

    if trump_count >= 4 {
        s += 0.45;
    } else if trump_count == 3 {
        s += 0.15;
    } else if trump_count <= 1 {
        s -= 0.6;
    }

    let voids = 3usize.saturating_sub(groups.len());
    let void_rate = if trump_count >= 3 {
        0.3
    } else if trump_count == 2 {
        0.15
    } else {
        0.0
    };
    s += voids.min(2) as f64 * void_rate;
    s
}

/// Scoreboard pressure (`scoreAdjust`).
pub fn score_adjust(ctx: &StrengthCtx) -> f64 {
    let mut a = 0.0f64;
    if ctx.opp_score - ctx.my_score >= 4.0 {
        a += 0.25;
    }
    if ctx.my_score >= 8.0 && ctx.opp_score <= 6.0 {
        a -= 0.25;
    }
    if ctx.opp_score == ctx.game_to - 1.0 {
        a += 0.35;
    }
    a
}

/// Parameters of [`hand_strength`].
#[derive(Clone, Copy, Debug)]
pub struct HandStrengthParams<'a> {
    /// The holding.
    pub hand: &'a [Card],
    /// The candidate trump.
    pub trump: Suit,
    /// The evaluation context.
    pub ctx: &'a StrengthCtx,
}

/// The full bidding evaluation of a holding (`handStrength`).
pub fn hand_strength(params: HandStrengthParams<'_>) -> f64 {
    let HandStrengthParams { hand, trump, ctx } = params;
    let mut s = raw_hand_value(hand, trump);

    if ctx.phase == StrengthPhase::Round1
        && let Some(up) = ctx.up_card
    {
        if ctx.seat == ctx.dealer_seat {
            s += pickup_delta(hand, up, trump);
        } else {
            let dealer_is_partner = ctx.dealer_seat == partner_of(ctx.seat);
            let up_boost = trump_value(trump_rank(up, trump));
            s += (if dealer_is_partner { 0.35 } else { -0.45 }) * (0.5 + up_boost);
        }
    }

    if ctx.phase == StrengthPhase::Round2
        && let Some(down) = ctx.turned_down_suit
    {
        let is_next = same_color(trump, down) && trump != down;
        let role = seat_role(ctx.seat, ctx.dealer_seat);
        s += if is_next {
            if role == SeatRole::Eldest { 0.45 } else { 0.15 }
        } else if role == SeatRole::Dealer || role == SeatRole::DealerPartner {
            0.2
        } else {
            0.0
        };
    }

    s + score_adjust(ctx)
}

/// What a five-card holding is worth to keep (`keepValue`).
fn keep_value(kept: &[Card], trump: Suit) -> f64 {
    let mut v = raw_hand_value(kept, trump);
    let mut counts: Vec<(Suit, usize)> = Vec::with_capacity(3);
    for &c in kept {
        let es = effective_suit(c, Some(trump));
        if es == trump {
            continue;
        }
        match counts.iter_mut().find(|(s, _)| *s == es) {
            Some((_, n)) => *n += 1,
            None => counts.push((es, 1)),
        }
    }
    v += 0.18 * (3usize.saturating_sub(counts.len())) as f64;
    for (_, n) in &counts {
        if *n == 1 {
            v -= 0.05;
        }
    }
    v
}

/// A copy of `cards` without the element at `index`.
fn without_index(cards: &[Card], index: usize) -> Vec<Card> {
    let mut out = cards.to_vec();
    if index < out.len() {
        out.remove(index);
    }
    out
}

/// The dealer's discard search: argmax of [`keep_value`], ties on card id.
fn discard_search(six: &[Card], trump: Suit) -> (Option<Card>, f64) {
    let mut best_card = six.first().copied();
    let mut best_value = f64::NEG_INFINITY;
    for (i, &candidate) in six.iter().enumerate() {
        let remaining = without_index(six, i);
        let v = keep_value(&remaining, trump);
        let tie_wins = v == best_value && best_card.is_some_and(|b| candidate < b);
        if v > best_value || tie_wins {
            best_value = v;
            best_card = Some(candidate);
        }
    }
    (best_card, best_value)
}

/// Which of the dealer's six cards to bury (`bestDiscard`); `None` for an empty hand.
pub fn best_discard(six: &[Card], trump: Suit) -> Option<Card> {
    if six.is_empty() {
        return None;
    }
    discard_search(six, trump).0
}

/// How much the dealer's hand improves by taking the up-card (`pickupDelta`); never negative.
pub fn pickup_delta(hand: &[Card], up_card: Card, trump: Suit) -> f64 {
    let baseline = raw_hand_value(hand, trump);
    let mut six = hand.to_vec();
    six.push(up_card);
    let best = discard_search(&six, trump).1;
    let d = best - baseline;
    if d.is_finite() && d > 0.0 { d } else { 0.0 }
}

/// The dealer's five cards after taking the up-card and burying the worst (`handAfterPickup`).
pub fn hand_after_pickup(hand: &[Card], up_card: Card, trump: Suit) -> Vec<Card> {
    let mut six = hand.to_vec();
    six.push(up_card);
    let Some(drop) = best_discard(&six, trump) else {
        return hand.to_vec();
    };
    if let Some(at) = six.iter().position(|c| *c == drop) {
        six.remove(at);
    }
    six
}

/// The four classic laydown shapes that justify a loner (`loneShortlist`).
pub fn lone_shortlist(hand: &[Card], trump: Suit) -> bool {
    let mut ranks = [false; 9];
    let mut off_aces = 0usize;
    let mut off_kings = 0usize;
    for &c in hand {
        if is_trump_card(c, trump) {
            let r = usize::from(trump_rank(c, trump));
            if r < ranks.len() {
                ranks[r] = true;
            }
            continue;
        }
        match rank_of(c) {
            Rank::Ace => off_aces += 1,
            Rank::King => off_kings += 1,
            _ => {}
        }
    }
    let (right, left, ace_t, king_t) = (ranks[8], ranks[7], ranks[6], ranks[5]);
    (right && left && ace_t && off_aces >= 2)
        || (right && left && ace_t && off_aces >= 1 && off_kings >= 1)
        || (right && left && king_t && off_aces >= 2)
        || (right && ace_t && king_t && off_aces >= 2)
}

/* ========================================================================== */
/* Trump census and void reads                                                 */
/* ========================================================================== */

/// Where the seven trump are, from public information plus this seat's hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrumpCensus {
    /// Trump still in my own hand.
    pub mine: Vec<Card>,
    /// Trump neither mine, nor played, nor known-buried, highest first.
    pub outstanding: Vec<Card>,
}

/// Parameters of [`trump_census`].
#[derive(Clone, Copy, Debug)]
pub struct TrumpCensusParams<'a> {
    /// The redacted view.
    pub view: &'a PublicGameView,
    /// This seat's cards.
    pub hand: &'a [Card],
    /// Trump.
    pub trump: Suit,
}

/// `trumpCensus`: a turned-down up-card is out of play; a picked-up one stays outstanding.
pub fn trump_census(params: TrumpCensusParams<'_>) -> TrumpCensus {
    let TrumpCensusParams { view, hand, trump } = params;
    let mine: Vec<Card> = hand
        .iter()
        .copied()
        .filter(|c| is_trump_card(*c, trump))
        .collect();
    let mut accounted: Vec<Card> = mine.clone();
    for c in played_cards(view) {
        if is_trump_card(c, trump) {
            accounted.push(c);
        }
    }
    if let Some(up) = view.up_card
        && view.up_card_turned_down
        && is_trump_card(up, trump)
    {
        accounted.push(up);
    }
    let outstanding = trump_cards(trump)
        .into_iter()
        .filter(|c| !accounted.contains(c))
        .collect();
    TrumpCensus { mine, outstanding }
}

/// Suits each seat has publicly shown void in (`voidReads`), indexed by seat.
pub fn void_reads(view: &PublicGameView) -> [Vec<Suit>; 4] {
    let mut acc: [Vec<Suit>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let trump = view.trump;
    let live = view.trick.index;
    let mut scan = |plays: &[euchre_engine::Play]| {
        let Some(first) = plays.first() else { return };
        let led = effective_suit(first.card, trump);
        for p in plays {
            if effective_suit(p.card, trump) == led {
                continue;
            }
            let seat = &mut acc[p.seat.index()];
            if !seat.contains(&led) {
                seat.push(led);
            }
        }
    };
    for t in &view.trick_log {
        if t.index == live {
            continue;
        }
        scan(&t.plays);
    }
    scan(&view.trick.plays);
    acc
}

/* ========================================================================== */
/* The ranking entry point                                                     */
/* ========================================================================== */

/// Parameters of [`rank_moves`] and [`top_move`].
///
/// `hand` and `legal` default to `view.hand` and `view.legal`, exactly like the
/// TS default arguments.
#[derive(Clone, Copy, Debug)]
pub struct RankParams<'a> {
    /// `project(state, seat)`: the redaction boundary.
    pub view: &'a PublicGameView,
    /// This seat's own cards; `None` means `view.hand`.
    pub hand: Option<&'a [Card]>,
    /// The engine's legal set; `None` means `view.legal`.
    pub legal: Option<&'a [LegalMove]>,
    /// Persona dials.
    pub opts: HeuristicOptions,
}

impl<'a> RankParams<'a> {
    /// Neutral options over `view.hand` and `view.legal`.
    pub fn new(view: &'a PublicGameView) -> RankParams<'a> {
        RankParams {
            view,
            hand: None,
            legal: None,
            opts: HeuristicOptions::NEUTRAL,
        }
    }

    /// The same parameters with these options.
    pub fn with_opts(self, opts: HeuristicOptions) -> RankParams<'a> {
        RankParams { opts, ..self }
    }

    /// The same parameters with an explicit legal set.
    pub fn with_legal(self, legal: &'a [LegalMove]) -> RankParams<'a> {
        RankParams {
            legal: Some(legal),
            ..self
        }
    }

    /// The same parameters with an explicit hand.
    pub fn with_hand(self, hand: &'a [Card]) -> RankParams<'a> {
        RankParams {
            hand: Some(hand),
            ..self
        }
    }
}

struct Scored {
    score: f64,
    why: &'static str,
}

impl Scored {
    const ZERO: Scored = Scored {
        score: 0.0,
        why: "",
    };
}

/// Order a legal set from best to worst (`rankMoves`).
///
/// The result is a permutation of `legal` by id, never empty when `legal` is
/// not, deterministic, with ties broken on id ascending. Non-finite scores read
/// as `0`.
pub fn rank_moves(params: RankParams<'_>) -> Vec<RankedMove> {
    let view = params.view;
    let hand = params.hand.unwrap_or(&view.hand);
    let legal = params.legal.unwrap_or(&view.legal);
    if legal.is_empty() {
        return Vec::new();
    }
    let scored = score_legal(view, hand, legal, &params.opts);
    // `byId` keeps the first score for a duplicated id.
    let mut out: Vec<RankedMove> = legal
        .iter()
        .map(|m| {
            let first = legal
                .iter()
                .position(|o| o.id == m.id)
                .and_then(|i| scored.get(i));
            match first {
                Some(s) => RankedMove {
                    id: m.id.clone(),
                    score: if s.score.is_finite() { s.score } else { 0.0 },
                    why: s.why.to_owned(),
                },
                None => RankedMove {
                    id: m.id.clone(),
                    score: 0.0,
                    why: String::new(),
                },
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// The single best legal move id (`topMove`), or `None` when there are none.
pub fn top_move(params: RankParams<'_>) -> Option<LegalMoveId> {
    let legal = params.legal.unwrap_or(&params.view.legal);
    rank_moves(params)
        .into_iter()
        .next()
        .map(|r| r.id)
        .or_else(|| legal.first().map(|m| m.id.clone()))
}

/// Scores aligned index-for-index with `legal`.
fn score_legal(
    view: &PublicGameView,
    hand: &[Card],
    legal: &[LegalMove],
    opts: &HeuristicOptions,
) -> Vec<Scored> {
    use euchre_engine::GamePhase;
    match view.phase {
        GamePhase::Cutting => score_cut(legal),
        GamePhase::BidRound1 => score_round1(view, hand, legal, opts),
        GamePhase::DealerDiscard => score_discard(view, hand, legal),
        GamePhase::BidRound2 => score_round2(view, hand, legal, opts),
        GamePhase::TrickPlay => score_plays(view, hand, legal),
        _ => legal.iter().map(|_| Scored::ZERO).collect(),
    }
}

/* -------------------------------------------------------------------------- */
/* Cutting                                                                     */
/* -------------------------------------------------------------------------- */

fn score_cut(legal: &[LegalMove]) -> Vec<Scored> {
    legal
        .iter()
        .map(|m| {
            if m.id == "cut:no" {
                Scored {
                    score: 0.01,
                    why: "run 'em",
                }
            } else {
                Scored {
                    score: 0.0,
                    why: "give it a cut",
                }
            }
        })
        .collect()
}

/* -------------------------------------------------------------------------- */
/* Bidding                                                                     */
/* -------------------------------------------------------------------------- */

fn dials(opts: &HeuristicOptions) -> (f64, f64) {
    (
        clamp01(opts.aggression.unwrap_or(0.5)),
        clamp01(opts.risk.unwrap_or(0.5)),
    )
}

fn ctx_for(view: &PublicGameView, phase: StrengthPhase, opts: &HeuristicOptions) -> StrengthCtx {
    let me = team_of(view.you);
    let opp = if me == Team::NS { Team::EW } else { Team::NS };
    StrengthCtx {
        seat: view.you,
        dealer_seat: view.dealer_seat,
        phase,
        up_card: view.up_card,
        turned_down_suit: view.turned_down_suit,
        my_score: f64::from(view.score[me.index()]),
        opp_score: f64::from(view.score[opp.index()]),
        game_to: opts
            .game_to
            .unwrap_or(f64::from(DEFAULT_ENGINE_CONFIG.game_to)),
    }
}

/// `(call, alone)` scores; `pass` sits at `0`.
fn bid_scores(strength: f64, call_threshold: f64, alone_thr: f64, shortlist: bool) -> (f64, f64) {
    let call = strength - call_threshold;
    let alone = if shortlist {
        call + 0.5
    } else {
        2.0 * strength - alone_thr - call_threshold + EPS
    };
    (call, alone)
}

fn call_threshold_for(base: f64, agg: f64) -> f64 {
    base - (agg - 0.5) * AGGRESSION_SWING
}

fn alone_threshold_for(base: f64, risk: f64) -> f64 {
    base - (risk - 0.5) * RISK_SWING
}

fn score_round1(
    view: &PublicGameView,
    hand: &[Card],
    legal: &[LegalMove],
    opts: &HeuristicOptions,
) -> Vec<Scored> {
    let Some(up) = view.up_card else {
        return legal.iter().map(|_| Scored::ZERO).collect();
    };
    let trump = suit_of(up);
    let role = seat_role(view.you, view.dealer_seat);
    let (agg, risk) = dials(opts);
    let ctx = ctx_for(view, StrengthPhase::Round1, opts);
    let strength = hand_strength(HandStrengthParams {
        hand,
        trump,
        ctx: &ctx,
    });
    let dealer = view.you == view.dealer_seat;
    let shortlist = if dealer {
        lone_shortlist(&hand_after_pickup(hand, up, trump), trump)
    } else {
        lone_shortlist(hand, trump)
    };
    let (call, alone) = bid_scores(
        strength,
        call_threshold_for(order_up_threshold(role), agg),
        alone_threshold_for(alone_threshold(role), risk),
        shortlist,
    );
    legal
        .iter()
        .map(|m| match m.id.as_str() {
            "pass" => Scored {
                score: 0.0,
                why: "not enough here",
            },
            "orderUp+alone" => Scored {
                score: alone,
                why: if shortlist {
                    "this one plays itself"
                } else {
                    "I can see three"
                },
            },
            "orderUp" => Scored {
                score: call,
                why: if dealer {
                    "worth picking up"
                } else {
                    "good enough to call"
                },
            },
            _ => Scored::ZERO,
        })
        .collect()
}

fn score_round2(
    view: &PublicGameView,
    hand: &[Card],
    legal: &[LegalMove],
    opts: &HeuristicOptions,
) -> Vec<Scored> {
    let role = seat_role(view.you, view.dealer_seat);
    let (agg, risk) = dials(opts);
    let turned_down = view.turned_down_suit;
    let ctx = ctx_for(view, StrengthPhase::Round2, opts);
    let is_next_suit = |suit: Suit| turned_down.is_some_and(|d| same_color(suit, d) && suit != d);

    // (suit, call, alone, shortlist)
    let mut per_suit: Vec<(Suit, f64, f64, bool)> = Vec::with_capacity(4);
    for suit in SUITS {
        if turned_down == Some(suit) {
            continue;
        }
        let strength = hand_strength(HandStrengthParams {
            hand,
            trump: suit,
            ctx: &ctx,
        });
        let base = if is_next_suit(suit) {
            next_threshold(role)
        } else {
            cross_threshold(role)
        };
        let shortlist = lone_shortlist(hand, suit);
        let (call, alone) = bid_scores(
            strength,
            call_threshold_for(base, agg),
            alone_threshold_for(alone_threshold(role), risk),
            shortlist,
        );
        per_suit.push((suit, call, alone, shortlist));
    }

    legal
        .iter()
        .map(|m| {
            if m.id == "pass" {
                return Scored {
                    score: 0.0,
                    why: "nothing to name",
                };
            }
            let Some(rest) = m.id.strip_prefix("call:") else {
                return Scored::ZERO;
            };
            let alone = rest.ends_with("+alone");
            let Some(suit) = rest.chars().next().and_then(Suit::from_char) else {
                return Scored::ZERO;
            };
            let Some(&(_, call, alone_score, shortlist)) =
                per_suit.iter().find(|(s, ..)| *s == suit)
            else {
                return Scored::ZERO;
            };
            Scored {
                score: if alone { alone_score } else { call },
                why: if alone {
                    if shortlist {
                        "this one plays itself"
                    } else {
                        "I can see three"
                    }
                } else if is_next_suit(suit) {
                    "next is the call here"
                } else {
                    "crossing the creek"
                },
            }
        })
        .collect()
}

fn score_discard(view: &PublicGameView, hand: &[Card], legal: &[LegalMove]) -> Vec<Scored> {
    let Some(trump) = view.trump else {
        return legal.iter().map(|_| Scored::ZERO).collect();
    };
    legal
        .iter()
        .map(|m| {
            let Some(raw) = m.id.strip_prefix("discard:") else {
                return Scored::ZERO;
            };
            let card = Card::parse(raw);
            let at = card.and_then(|c| hand.iter().position(|h| *h == c));
            let kept = match at {
                Some(i) => without_index(hand, i),
                None => hand.to_vec(),
            };
            Scored {
                score: keep_value(&kept, trump),
                why: if card.is_some_and(|c| is_trump_card(c, trump)) {
                    "hate to break trump"
                } else {
                    "bury the useless one"
                },
            }
        })
        .collect()
}

/* -------------------------------------------------------------------------- */
/* Card play                                                                   */
/* -------------------------------------------------------------------------- */

fn score_plays(view: &PublicGameView, hand: &[Card], legal: &[LegalMove]) -> Vec<Scored> {
    let Some(trump) = view.trump else {
        return legal.iter().map(|_| Scored::ZERO).collect();
    };
    let me = view.you;
    let my_team = team_of(me);
    let partner = partner_of(me);
    let plays = &view.trick.plays;
    let leading = plays.is_empty();
    let seats_after_me =
        (i32::from(plays_per_trick(view.sitting_seat)) - plays.len() as i32 - 1).max(0);
    let census = trump_census(TrumpCensusParams { view, hand, trump });
    let voids = void_reads(view);
    let trick_no = view.trick.index;

    let i_am_maker = view.maker_seat.is_some_and(|m| team_of(m) == my_team);
    let defending_loner = view.alone_seat.is_some_and(|a| team_of(a) != my_team);
    let my_tricks = view.tricks_won[my_team.index()];
    let winning = currently_winning(plays, Some(trump));
    let partner_winning = winning.is_some_and(|w| w.seat == partner);

    let mut by_suit: Vec<(Suit, usize)> = Vec::with_capacity(4);
    for &c in hand {
        let es = effective_suit(c, Some(trump));
        match by_suit.iter_mut().find(|(s, _)| *s == es) {
            Some((_, n)) => *n += 1,
            None => by_suit.push((es, 1)),
        }
    }
    let suit_len = |s: Suit| by_suit.iter().find(|(x, _)| *x == s).map(|(_, n)| *n);
    let my_trump_count = census.mine.len();

    let base = |card: Card| -> Scored {
        let p = power(card, trump);
        let es = effective_suit(card, Some(trump));
        let is_trump = es == trump;
        let cost = if is_trump {
            (8.0 + f64::from(trump_rank(card, trump))) / 16.0
        } else {
            f64::from(plain_rank(card)) / 16.0
        };

        if leading {
            if i_am_maker && trick_no == 0 && my_trump_count >= 3 && is_trump {
                return Scored {
                    score: 1.6 + p,
                    why: "pull their trump",
                };
            }
            if i_am_maker
                && trick_no == 0
                && my_trump_count <= 2
                && !is_trump
                && rank_of(card) == Rank::Ace
            {
                return Scored {
                    score: 1.5 + p,
                    why: "cash it before the ruff",
                };
            }
            let highest_out = census.outstanding.first();
            if is_trump
                && census.outstanding.len() <= 1
                && highest_out.is_none_or(|h| trump_rank(card, trump) > trump_rank(*h, trump))
            {
                return Scored {
                    score: 1.5,
                    why: "this one is good",
                };
            }
            if !is_trump && rank_of(card) == Rank::Ace && suit_len(es).unwrap_or(0) >= 2 {
                let risky = Seat::ALL.iter().any(|s| {
                    team_of(*s) != my_team
                        && voids[s.index()].contains(&es)
                        && !census.outstanding.is_empty()
                });
                return if risky {
                    Scored {
                        score: 1.15 + p,
                        why: "they may be out of it",
                    }
                } else {
                    Scored {
                        score: 1.3 + p,
                        why: "the ace is good",
                    }
                };
            }
            if !is_trump {
                let len = suit_len(es).unwrap_or(1) as f64;
                return Scored {
                    score: 0.5 + 0.12 * len + 0.2 * p,
                    why: "lead from length",
                };
            }
            return Scored {
                score: 0.4 + 0.3 * p,
                why: "ask for a bower",
            };
        }

        let led = plays
            .first()
            .map_or(es, |f| effective_suit(f.card, Some(trump)));
        let can_win = winning.is_none_or(|w| {
            card_value(card, led, Some(trump)) > card_value(w.card, led, Some(trump))
        });

        if partner_winning {
            return if seats_after_me == 0 {
                Scored {
                    score: 1.4 - cost,
                    why: "lay off, it is theirs",
                }
            } else {
                Scored {
                    score: 1.0 - 0.8 * cost,
                    why: "save the good one",
                }
            };
        }
        if can_win {
            return match seats_after_me {
                0 => Scored {
                    score: 1.7 - 0.5 * p,
                    why: "cheapest card that takes it",
                },
                1 => Scored {
                    score: 1.2 + 0.4 * p,
                    why: "third hand high",
                },
                _ => Scored {
                    score: 0.9 + 0.3 * p,
                    why: "second hand, only if it matters",
                },
            };
        }
        Scored {
            score: 0.6 - cost,
            why: "cannot win, keep the good ones",
        }
    };

    legal
        .iter()
        .map(|m| {
            let Some(raw) = m.id.strip_prefix("play:") else {
                return Scored::ZERO;
            };
            // An id the engine never produces scores NaN in TS, which reads as 0.
            // TS keeps the branch why; this gives "" (ai-core deviation 4).
            let Some(card) = Card::parse(raw) else {
                return Scored::ZERO;
            };
            let s = base(card);
            let mut score = s.score;
            let mut why = s.why;
            if !partner_winning
                && i_am_maker
                && my_tricks == 2
                && is_trump_card(card, trump)
                && trump_rank(card, trump) >= 6
            {
                score += 0.35;
                why = "take the third and be done";
            }
            if !partner_winning
                && defending_loner
                && my_tricks == 0
                && is_trump_card(card, trump)
                && trump_rank(card, trump) >= 7
            {
                score += 0.4;
                why = "stop the loner sweep";
            }
            Scored { score, why }
        })
        .collect()
}
