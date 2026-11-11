//! The total order inside one trick, and who takes it (port of `trick.ts`).
//!
//! [`card_value`] collapses trump rank, led-suit rank and "cannot win" into one
//! comparable integer. It is the only comparator used by trick resolution.

use std::fmt;

use crate::cards::{effective_suit, plain_rank, trump_rank};
use crate::seats::active_seat_count;
use crate::types::{CardId, Play, Seat, Suit};

/// Strength of `card` in a trick led with `led` under `trump`: `102..=108` for
/// trump (bowers included), `51..=56` for the led suit, `0` for anything else.
pub fn card_value(card: CardId, led: Suit, trump: Option<Suit>) -> u8 {
    let eff = effective_suit(card, trump);
    match trump {
        Some(t) if eff == t => 100 + trump_rank(card, t),
        _ if eff == led => 50 + plain_rank(card),
        _ => 0,
    }
}

/// Returned by [`trick_winner`] for a trick with no plays (a programmer error, never a rules event).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmptyTrickError;

impl fmt::Display for EmptyTrickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("trickWinner: a trick has no plays")
    }
}

impl std::error::Error for EmptyTrickError {}

/// The first play with the strictly highest [`card_value`].
fn best_play(plays: &[Play], led: Suit, trump: Option<Suit>) -> Option<Play> {
    let (&first, rest) = plays.split_first()?;
    let mut best = first;
    let mut best_value = card_value(best.card, led, trump);
    for &p in rest {
        let v = card_value(p.card, led, trump);
        if v > best_value {
            best_value = v;
            best = p;
        }
    }
    Some(best)
}

/// The seat that takes the trick: highest trump if any, else highest of the led suit.
pub fn trick_winner(plays: &[Play], led: Suit, trump: Suit) -> Result<Seat, EmptyTrickError> {
    best_play(plays, led, Some(trump))
        .map(|p| p.seat)
        .ok_or(EmptyTrickError)
}

/// The **effective** suit of the first play, or `None` for an empty trick.
pub fn led_suit_of(plays: &[Play], trump: Option<Suit>) -> Option<Suit> {
    plays.first().map(|p| effective_suit(p.card, trump))
}

/// How many plays complete a trick: 4 normally, 3 under a loner.
pub const fn plays_per_trick(sitting_seat: Option<Seat>) -> u8 {
    active_seat_count(sitting_seat)
}

/// Whether every active seat has played to this trick.
pub fn is_trick_complete(plays: &[Play], sitting_seat: Option<Seat>) -> bool {
    plays.len() >= usize::from(plays_per_trick(sitting_seat))
}

/// The play currently winning an in-progress trick, or `None` if nothing is laid.
pub fn currently_winning(plays: &[Play], trump: Option<Suit>) -> Option<Play> {
    let led = led_suit_of(plays, trump)?;
    best_play(plays, led, trump)
}
