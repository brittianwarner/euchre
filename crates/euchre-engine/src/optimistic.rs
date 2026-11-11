//! The client's local move preview (port of `optimistic.ts`).
//!
//! Contract (V22): for every reachable state and every `LegalMove` of the acting
//! seat except `cut`, `apply_optimistic(project(s, seat), move)` equals
//! `project(apply(s, seat, move).state, seat)` on every field in
//! [`OPTIMISTIC_FIELDS`]. `legal` is deliberately outside the contract: every
//! branch clears it. This module computes no legality and decides no transition.

use crate::cards::{effective_suit, suit_of};
use crate::seats::partner_of;
use crate::trick::trick_winner;
use crate::types::{CardId, GamePhase, Move, Play, PublicGameView, Seat, Suit, Trick};

/// One `PublicGameView` field that [`apply_optimistic`] must get exactly right.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum OptimisticField {
    /// `hand`.
    Hand,
    /// `handCounts`.
    HandCounts,
    /// `trick`.
    Trick,
    /// `kittyCount`.
    KittyCount,
    /// `passes`.
    Passes,
    /// `trump`.
    Trump,
    /// `makerSeat`.
    MakerSeat,
    /// `aloneSeat`.
    AloneSeat,
    /// `sittingSeat`.
    SittingSeat,
    /// `upCardTurnedDown`.
    UpCardTurnedDown,
    /// `turnedDownSuit`.
    TurnedDownSuit,
}

impl OptimisticField {
    /// The camelCase JSON key of this field.
    pub const fn as_str(self) -> &'static str {
        match self {
            OptimisticField::Hand => "hand",
            OptimisticField::HandCounts => "handCounts",
            OptimisticField::Trick => "trick",
            OptimisticField::KittyCount => "kittyCount",
            OptimisticField::Passes => "passes",
            OptimisticField::Trump => "trump",
            OptimisticField::MakerSeat => "makerSeat",
            OptimisticField::AloneSeat => "aloneSeat",
            OptimisticField::SittingSeat => "sittingSeat",
            OptimisticField::UpCardTurnedDown => "upCardTurnedDown",
            OptimisticField::TurnedDownSuit => "turnedDownSuit",
        }
    }
}

/// The fields `apply_optimistic` is contractually required to get exactly right, in TS order.
pub const OPTIMISTIC_FIELDS: [OptimisticField; 11] = [
    OptimisticField::Hand,
    OptimisticField::HandCounts,
    OptimisticField::Trick,
    OptimisticField::KittyCount,
    OptimisticField::Passes,
    OptimisticField::Trump,
    OptimisticField::MakerSeat,
    OptimisticField::AloneSeat,
    OptimisticField::SittingSeat,
    OptimisticField::UpCardTurnedDown,
    OptimisticField::TurnedDownSuit,
];

/// `hand` minus the first occurrence of `card`.
fn without(hand: &[CardId], card: CardId) -> Vec<CardId> {
    let mut out = hand.to_vec();
    if let Some(i) = out.iter().position(|&c| c == card) {
        out.remove(i);
    }
    out
}

/// `counts` with `seat` decremented, floored at zero.
fn decrement(mut counts: [u8; 4], seat: Seat) -> [u8; 4] {
    counts[seat.index()] = counts[seat.index()].saturating_sub(1);
    counts
}

/// The inputs of [`append_play`].
struct AppendPlay<'a> {
    trick: &'a Trick,
    seat: Seat,
    card: CardId,
    trump: Option<Suit>,
    sitting_seat: Option<Seat>,
}

/// Appends a play, deriving `led_suit` on the opening card and `winner_seat` once full.
fn append_play(p: AppendPlay<'_>) -> Trick {
    let mut plays = p.trick.plays.clone();
    plays.push(Play {
        seat: p.seat,
        card: p.card,
    });
    let led_suit = if p.trick.plays.is_empty() {
        Some(effective_suit(p.card, p.trump))
    } else {
        p.trick.led_suit
    };
    let expected = if p.sitting_seat.is_none() { 4 } else { 3 };
    let winner_seat = match (led_suit, p.trump) {
        (Some(led), Some(trump)) if plays.len() >= expected => trick_winner(&plays, led, trump)
            .ok()
            .or(p.trick.winner_seat),
        _ => p.trick.winner_seat,
    };
    Trick {
        index: p.trick.index,
        led_suit,
        plays,
        winner_seat,
    }
}

/// The view the server is about to produce for this seat, computed locally.
///
/// `mv` must come from `view.legal`; it is not re-checked. A `play`/`discard`
/// of a card not in `view.hand`, and every `cut`, return the view unchanged.
pub fn apply_optimistic(view: &PublicGameView, mv: &Move) -> PublicGameView {
    let you = view.you;
    match *mv {
        Move::Play { card } => {
            if !view.hand.contains(&card) {
                return view.clone();
            }
            PublicGameView {
                hand: without(&view.hand, card),
                hand_counts: decrement(view.hand_counts, you),
                trick: append_play(AppendPlay {
                    trick: &view.trick,
                    seat: you,
                    card,
                    trump: view.trump,
                    sitting_seat: view.sitting_seat,
                }),
                legal: Vec::new(),
                ..view.clone()
            }
        }

        Move::Discard { card } => {
            if !view.hand.contains(&card) {
                return view.clone();
            }
            PublicGameView {
                hand: without(&view.hand, card),
                hand_counts: decrement(view.hand_counts, you),
                kitty_count: if view.up_card_turned_down {
                    view.kitty_count
                } else {
                    4
                },
                legal: Vec::new(),
                ..view.clone()
            }
        }

        Move::Pass => {
            let passes = view.passes.saturating_add(1);
            match view.up_card {
                Some(up) if passes >= 4 && view.phase == GamePhase::BidRound1 => PublicGameView {
                    passes: 0,
                    kitty_count: 4,
                    up_card_turned_down: true,
                    turned_down_suit: Some(suit_of(up)),
                    legal: Vec::new(),
                    ..view.clone()
                },
                _ => PublicGameView {
                    passes,
                    legal: Vec::new(),
                    ..view.clone()
                },
            }
        }

        Move::OrderUp { alone } => {
            let trump = view.up_card.map(suit_of).or(view.trump);
            let sitting = alone.then(|| partner_of(you));
            let mut counts = view.hand_counts;
            let dealer_sits_out = sitting == Some(view.dealer_seat);
            if !dealer_sits_out {
                let d = view.dealer_seat.index();
                counts[d] = counts[d].saturating_add(1);
            }
            if let Some(s) = sitting {
                counts[s.index()] = 0;
            }
            let mut hand = view.hand.clone();
            if let Some(up) = view.up_card
                && you == view.dealer_seat
                && !dealer_sits_out
            {
                hand.push(up);
            }
            PublicGameView {
                hand,
                trump,
                maker_seat: Some(you),
                alone_seat: alone.then_some(you),
                sitting_seat: sitting,
                hand_counts: counts,
                kitty_count: if dealer_sits_out { 4 } else { view.kitty_count },
                legal: Vec::new(),
                ..view.clone()
            }
        }

        Move::Call { suit, alone } => {
            let sitting = alone.then(|| partner_of(you));
            let mut counts = view.hand_counts;
            if let Some(s) = sitting {
                counts[s.index()] = 0;
            }
            PublicGameView {
                trump: Some(suit),
                maker_seat: Some(you),
                alone_seat: alone.then_some(you),
                sitting_seat: sitting,
                hand_counts: counts,
                legal: Vec::new(),
                ..view.clone()
            }
        }

        // Outside the contract: the deal comes from the server-held shuffle.
        Move::Cut { .. } => view.clone(),
    }
}
