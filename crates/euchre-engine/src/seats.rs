//! Seat arithmetic, team membership, and loner-aware turn order (port of `seats.ts`).
//!
//! Seats are chairs, fixed for the life of the match; roles rotate with the
//! dealer. Every function here is total, allocation-free (except
//! [`bidding_order`], which returns a fixed array), and pure.

use crate::types::{Seat, SeatRole, Team};

/// The chair the human always occupies. Seat `0`, South.
pub const HUMAN_SEAT: Seat = Seat::SOUTH;

/// Every seat in clockwise order.
pub const SEATS: [Seat; 4] = Seat::ALL;

/// The seat one place clockwise. Loner-unaware; see [`next_active_seat`].
pub const fn next_seat(s: Seat) -> Seat {
    Seat::wrapping(s.get() + 1)
}

/// The seat opposite `s`: their partner.
pub const fn partner_of(s: Seat) -> Seat {
    Seat::wrapping(s.get() + 2)
}

/// The team a seat plays for: team 0 is `{0, 2}`, team 1 is `{1, 3}`.
pub const fn team_of(s: Seat) -> Team {
    if s.get() & 1 == 0 { Team::NS } else { Team::EW }
}

/// The opposing team.
pub const fn opponent_team(t: Team) -> Team {
    if t.get() == 0 { Team::EW } else { Team::NS }
}

/// A seat's role relative to `dealer`: 0 dealer, 1 eldest, 2 dealer's partner, 3 third seat.
pub const fn seat_role(s: Seat, dealer: Seat) -> SeatRole {
    match (s.get() + 4 - dealer.get()) & 3 {
        0 => SeatRole::Dealer,
        1 => SeatRole::Eldest,
        2 => SeatRole::DealerPartner,
        _ => SeatRole::Third,
    }
}

/// Eldest hand: the seat left of the dealer.
pub const fn eldest_of(dealer: Seat) -> Seat {
    next_seat(dealer)
}

/// Third seat, right of the dealer: the seat offered the cut.
pub const fn cutter_of(dealer: Seat) -> Seat {
    Seat::wrapping(dealer.get() + 3)
}

/// The bidding order for one round: eldest, dealer's partner, third seat, dealer.
pub const fn bidding_order(dealer: Seat) -> [Seat; 4] {
    let e = eldest_of(dealer);
    [e, next_seat(e), next_seat(next_seat(e)), dealer]
}

/// `s` if it is active, otherwise the next seat clockwise (the trick-1 lead rule).
pub fn first_active_from(s: Seat, sitting_seat: Option<Seat>) -> Seat {
    if Some(s) == sitting_seat {
        next_seat(s)
    } else {
        s
    }
}

/// The next seat to act after `s`, skipping the loner's sitting partner (never returns it, V8).
pub fn next_active_seat(s: Seat, sitting_seat: Option<Seat>) -> Seat {
    let n = next_seat(s);
    if Some(n) == sitting_seat {
        next_seat(n)
    } else {
        n
    }
}

/// How many seats are in play: 4 normally, 3 under a loner.
pub const fn active_seat_count(sitting_seat: Option<Seat>) -> u8 {
    if sitting_seat.is_none() { 4 } else { 3 }
}

/// Whether a seat may act at all this hand: `false` only for the sitting partner.
pub fn is_active_seat(s: Seat, sitting_seat: Option<Seat>) -> bool {
    Some(s) != sitting_seat
}
