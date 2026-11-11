//! The euchre scoring table, and game end (port of `score.ts`).
//!
//! | Situation                           | `HandResult` | Points | To        |
//! |-------------------------------------|--------------|--------|-----------|
//! | Makers take 3 or 4 tricks           | `point`      | 1      | Makers    |
//! | Makers take all 5 (march)           | `march`      | 2      | Makers    |
//! | Lone maker takes 3 or 4             | `lone_point` | 1      | Makers    |
//! | Lone maker takes all 5              | `lone_march` | 4      | Makers    |
//! | Makers take 0-2 (euchred)           | `euchre`     | 2      | Defenders |
//! | Four passes, `stickTheDealer:false` | `throw_in`   | 0      | -         |

use crate::seats::{opponent_team, team_of};
use crate::types::{HandResult, Seat, Team};

/// The result of applying the table to one finished hand.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ScoreOutcome {
    /// The table row.
    pub result: HandResult,
    /// **Post-clamp**, so `score[i] + delta[i]` is always the new score.
    pub delta: [u8; 2],
}

/// The inputs of [`score_hand`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ScoreHandParams {
    /// The maker.
    pub maker_seat: Seat,
    /// The loner, if any.
    pub alone_seat: Option<Seat>,
    /// Tricks by team.
    pub tricks_won: [u8; 2],
    /// The match score before this hand.
    pub score: [u8; 2],
    /// Points to win.
    pub game_to: u8,
}

/// Applies the scoring table to a completed hand. `delta` is clamped at
/// `game_to`, non-negative, and has at most one non-zero component (V11).
pub fn score_hand(params: ScoreHandParams) -> ScoreOutcome {
    let ScoreHandParams {
        maker_seat,
        alone_seat,
        tricks_won,
        score,
        game_to,
    } = params;
    let makers = team_of(maker_seat);
    let defenders = opponent_team(makers);
    let made = tricks_won[makers.index()];
    let alone = alone_seat.is_some();

    let mut raw = [0u8; 2];
    let result = if made <= 2 {
        raw[defenders.index()] = 2;
        HandResult::Euchre
    } else if made <= 4 {
        raw[makers.index()] = 1;
        if alone {
            HandResult::LonePoint
        } else {
            HandResult::Point
        }
    } else {
        raw[makers.index()] = if alone { 4 } else { 2 };
        if alone {
            HandResult::LoneMarch
        } else {
            HandResult::March
        }
    };

    // u16 so `score + raw` cannot overflow; a score already above `game_to`
    // (impossible under V11) saturates the delta to 0 instead of going negative.
    let clamp = |i: usize| -> u8 {
        let next = (u16::from(score[i]) + u16::from(raw[i])).min(u16::from(game_to));
        u8::try_from(next.saturating_sub(u16::from(score[i]))).unwrap_or(u8::MAX)
    };
    ScoreOutcome {
        result,
        delta: [clamp(0), clamp(1)],
    }
}

/// The outcome of a thrown-in hand: nobody scores.
pub const fn throw_in_outcome() -> ScoreOutcome {
    ScoreOutcome {
        result: HandResult::ThrowIn,
        delta: [0, 0],
    }
}

/// The new score after applying an (already clamped) delta.
pub const fn apply_delta(score: [u8; 2], delta: [u8; 2]) -> [u8; 2] {
    [
        score[0].saturating_add(delta[0]),
        score[1].saturating_add(delta[1]),
    ]
}

/// The team that has won, or `None` while the match is live. Team 0 is checked first.
pub const fn winner_team(score: [u8; 2], game_to: u8) -> Option<Team> {
    if score[0] >= game_to {
        Some(Team::NS)
    } else if score[1] >= game_to {
        Some(Team::EW)
    } else {
        None
    }
}

/// Whether the match has been won.
pub const fn is_game_over(score: [u8; 2], game_to: u8) -> bool {
    winner_team(score, game_to).is_some()
}

/// Whether a team is one point from winning ("in the barn").
pub fn is_in_the_barn(score: [u8; 2], team: Team, game_to: u8) -> bool {
    game_to.checked_sub(1) == Some(score[team.index()])
}

/// Whether the makers were euchred.
pub fn is_euchre(result: HandResult) -> bool {
    result == HandResult::Euchre
}

/// The points a result is worth, before clamping.
pub const fn raw_points(result: HandResult) -> u8 {
    match result {
        HandResult::Point | HandResult::LonePoint => 1,
        HandResult::March | HandResult::Euchre => 2,
        HandResult::LoneMarch => 4,
        HandResult::ThrowIn => 0,
    }
}
