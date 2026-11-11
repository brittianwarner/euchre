//! The projection: the security boundary (port of `index.ts:247-321`).
//!
//! [`project`] cuts the one view a client may see. [`PublicGameView`] has no
//! field capable of holding a foreign hand, a buried kitty card or the dealer's
//! discard, so a leak is a compile error rather than a review catch (V12).
//! Never widen it. `kitty_count` depends only on **whether** a discard exists
//! (V13); `up_card` is never nulled (V14).

use crate::legal::legal_moves;
use crate::types::{DealtPacket, GameState, PublicGameView, Seat, Step};

/// The view of `state` that `seat` is entitled to. Pure; every array is fresh.
pub fn project(state: &GameState, seat: Seat) -> PublicGameView {
    let h = &state.hand;
    let is_sitting_out = Some(seat) == h.sitting_seat;
    let count = |s: Seat| -> u8 {
        if Some(s) == h.sitting_seat {
            0
        } else {
            u8::try_from(h.hands[s.index()].len()).unwrap_or(u8::MAX)
        }
    };

    PublicGameView {
        players: None,
        observations: None,
        review_tricks: None,
        rules: None,
        awaiting_trick_review: None,
        v: state.v,
        you: seat,
        status: state.status,
        phase: h.phase,
        hand_no: h.hand_no,
        dealer_seat: h.dealer_seat,
        turn_seat: h.turn_seat,
        turn_id: state.turn_id.clone(),
        score: state.score,
        trump: h.trump,
        maker_seat: h.maker_seat,
        alone_seat: h.alone_seat,
        sitting_seat: h.sitting_seat,
        up_card: h.up_card,
        up_card_turned_down: h.up_card_turned_down,
        turned_down_suit: h.turned_down_suit,
        // Your own cards, and only ever your own: indexed by the seat asked for.
        hand: if is_sitting_out {
            Vec::new()
        } else {
            h.hands[seat.index()].clone()
        },
        hand_counts: [
            count(Seat::SOUTH),
            count(Seat::WEST),
            count(Seat::NORTH),
            count(Seat::EAST),
        ],
        kitty_count: 3 + u8::from(h.up_card_turned_down || h.dealer_discard.is_some()),
        trick: h.trick.clone(),
        trick_log: h.trick_log.clone(),
        tricks_won: h.tricks_won,
        passes: h.passes,
        bids: h.bids.clone(),
        legal: legal_moves(state, seat),
        result: h.result,
        delta: h.delta,
        winner_team: state.winner_team,
    }
}

/// Redacts a transition's choreography for one seat: every `dealt` packet not
/// addressed to `seat` has its `cards` blanked to `None`. Other steps are public.
pub fn project_steps(steps: &[Step], seat: Seat) -> Vec<Step> {
    steps
        .iter()
        .map(|step| match step {
            Step::Dealt { packets } => Step::Dealt {
                packets: packets
                    .iter()
                    .map(|p| DealtPacket {
                        seat: p.seat,
                        count: p.count,
                        cards: if p.seat == seat {
                            p.cards.clone()
                        } else {
                            None
                        },
                    })
                    .collect(),
            },
            other => other.clone(),
        })
        .collect()
}
