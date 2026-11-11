//! Possible remaining deals constrained by public cards, counts and voids, and
//! reference continuations over them (`ai/play-rollouts.ts`). Jev still chooses
//! every actual card.
//!
//! Synchronous and CPU-bound: call it from `spawn_blocking` on an async runtime.

use euchre_engine::{
    Card, DECK, GamePhase, GameState, LegalMove, LegalMoveId, Move, PublicGameView, Seat,
    effective_suit, team_of,
};
use serde::Serialize;

use crate::bid_rollouts::{RolloutError, base_game, run_reference, sorted_cards_json};
use crate::heuristic::void_reads;
use crate::js::round2;
use crate::rng::{random_for, shuffle_in_place};

/// `PLAY_ROLLOUT_SAMPLES`.
pub const PLAY_ROLLOUT_SAMPLES: usize = 24;

/// Shuffle attempts before giving up on more deals.
const MAX_ATTEMPTS: usize = 512;

/// One card's sampled continuation (key order as in TS).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayRollout {
    /// Deals sampled.
    pub sampled_deals: usize,
    /// Mean net points for our team, two decimals.
    #[serde(with = "euchre_protocol::serde_util::js_f64")]
    pub mean_team_net_points: f64,
    /// Mean tricks for our team, two decimals.
    #[serde(with = "euchre_protocol::serde_util::js_f64")]
    pub mean_team_tricks: f64,
    /// Deals the makers made.
    pub maker_made_hands: u32,
    /// Deals the makers swept.
    pub maker_swept_hands: u32,
    /// The standing caveat.
    pub limitation: &'static str,
}

/// Parameters of [`possible_remaining_deals`] and [`play_rollouts`].
#[derive(Clone, Copy, Debug)]
pub struct PlayRolloutParams<'a> {
    /// The player's view.
    pub view: &'a PublicGameView,
    /// The legal (candidate) set; unused by [`possible_remaining_deals`].
    pub legal: &'a [LegalMove],
    /// Target deals; TS default [`PLAY_ROLLOUT_SAMPLES`].
    pub target_samples: usize,
}

impl<'a> PlayRolloutParams<'a> {
    /// The default sample count.
    pub fn new(view: &'a PublicGameView, legal: &'a [LegalMove]) -> PlayRolloutParams<'a> {
        PlayRolloutParams {
            view,
            legal,
            target_samples: PLAY_ROLLOUT_SAMPLES,
        }
    }
}

const PLAY_LIMITATION: &str = "Possible remaining hands consistent with public cards, counts, dealer pickup and proved voids; reference heuristic continuation. Small-sample estimate, not actual holdings or a guarantee. Use exact partnership facts for equivalent outcomes.";

fn clamp_slice(cards: &[Card], start: usize, end: usize) -> Vec<Card> {
    let len = cards.len();
    let s = start.min(len);
    let e = end.min(len).max(s);
    cards[s..e].to_vec()
}

/// `possibleRemainingDeals(view, targetSamples)`: hypothetical full states, or
/// none when the view is not a consistent trick-play position.
pub fn possible_remaining_deals(
    params: PlayRolloutParams<'_>,
) -> Result<Vec<GameState>, RolloutError> {
    let view = params.view;
    let (Some(trump), Some(up)) = (view.trump, view.up_card) else {
        return Ok(Vec::new());
    };
    if view.phase != GamePhase::TrickPlay || view.maker_seat.is_none() {
        return Ok(Vec::new());
    }
    let tricks: Vec<&euchre_engine::Trick> = view
        .trick_log
        .iter()
        .chain(std::iter::once(&view.trick))
        .collect();
    let played: Vec<Card> = tricks
        .iter()
        .flat_map(|t| t.plays.iter().map(|p| p.card))
        .collect();
    let counts_ok = Seat::ALL.iter().all(|seat| {
        Some(*seat) == view.sitting_seat || {
            let by_seat = tricks
                .iter()
                .flat_map(|t| t.plays.iter())
                .filter(|p| p.seat == *seat)
                .count();
            usize::from(view.hand_counts[seat.index()]) == 5usize.wrapping_sub(by_seat)
        }
    });
    if usize::from(view.trick.index) != view.trick_log.len()
        || usize::from(view.hand_counts[view.you.index()]) != view.hand.len()
        || !counts_ok
    {
        return Ok(Vec::new());
    }
    let mut known: Vec<Card> = Vec::with_capacity(24);
    for c in view.hand.iter().chain(played.iter()) {
        if !known.contains(c) {
            known.push(*c);
        }
    }
    if known.len() != view.hand.len() + played.len() {
        return Ok(Vec::new());
    }
    if view.up_card_turned_down && !known.contains(&up) {
        known.push(up);
    }
    let unknown: Vec<Card> = DECK
        .iter()
        .copied()
        .filter(|c| !known.contains(c))
        .collect();
    let voids = void_reads(view);
    let trick_json =
        serde_json::to_string(&view.trick).map_err(|e| RolloutError::Engine(e.to_string()))?;
    let log_json =
        serde_json::to_string(&view.trick_log).map_err(|e| RolloutError::Engine(e.to_string()))?;
    let seed_text = format!(
        "[{},{},{},{},{},[{}]]",
        view.you.get(),
        sorted_cards_json(&view.hand),
        crate::js::quote(&trump.as_char().to_string()),
        trick_json,
        log_json,
        view.hand_counts
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut rng = random_for(&seed_text);
    let base = base_game("play-estimate", view.rules)?;
    let mut deals: Vec<GameState> = Vec::new();
    let mut attempt = 0;
    while deals.len() < params.target_samples && attempt < MAX_ATTEMPTS {
        attempt += 1;
        let mut shuffled = unknown.clone();
        shuffle_in_place(&mut shuffled, &mut rng);
        let mut hands: [Vec<Card>; 4] = base.hand.hands.clone();
        hands[view.you.index()] = view.hand.clone();
        let mut offset = 0usize;
        let mut valid = true;
        for seat in Seat::ALL {
            if seat == view.you {
                continue;
            }
            let count = if Some(seat) == view.sitting_seat {
                5
            } else {
                usize::from(view.hand_counts[seat.index()])
            };
            let cards = clamp_slice(&shuffled, offset, offset + count);
            offset += count;
            if Some(seat) != view.sitting_seat
                && cards
                    .iter()
                    .any(|c| voids[seat.index()].contains(&effective_suit(*c, Some(trump))))
            {
                valid = false;
            }
            if !view.up_card_turned_down
                && (seat != view.dealer_seat || Some(seat) == view.sitting_seat)
                && cards.contains(&up)
            {
                valid = false;
            }
            hands[seat.index()] = cards;
        }
        if !valid {
            continue;
        }
        let buried = clamp_slice(&shuffled, offset, shuffled.len());
        let dealer_discard = if view.up_card_turned_down {
            None
        } else if buried.contains(&up) {
            Some(up)
        } else {
            buried.first().copied()
        };
        let rest: Vec<Card> = buried
            .iter()
            .copied()
            .filter(|c| *c != up && Some(*c) != dealer_discard)
            .collect();
        let [k1, k2, k3] = rest.as_slice() else {
            continue;
        };
        let mut state = base.clone();
        let h = &mut state.hand;
        h.phase = GamePhase::TrickPlay;
        h.deck_order = DECK.to_vec();
        h.cut = Some(false);
        h.dealer_seat = view.dealer_seat;
        h.turn_seat = Some(view.you);
        h.trump = Some(trump);
        h.maker_seat = view.maker_seat;
        h.alone_seat = view.alone_seat;
        h.sitting_seat = view.sitting_seat;
        h.hands = hands;
        h.up_card = Some(up);
        h.up_card_turned_down = view.up_card_turned_down;
        h.turned_down_suit = view.turned_down_suit;
        h.dealer_discard = dealer_discard;
        h.trick = view.trick.clone();
        h.trick_log = view.trick_log.clone();
        h.tricks_won = view.tricks_won;
        h.kitty = [up, *k1, *k2, *k3];
        deals.push(state);
    }
    Ok(deals)
}

/// `playRollouts(view, legal, targetSamples)`: one entry per card play, in
/// legal order; empty (the TS `{}`) with fewer than two plays or no deals.
pub fn play_rollouts(
    params: PlayRolloutParams<'_>,
) -> Result<Vec<(LegalMoveId, PlayRollout)>, RolloutError> {
    let view = params.view;
    let plays: Vec<&LegalMove> = params
        .legal
        .iter()
        .filter(|m| matches!(m.mv, Move::Play { .. }))
        .collect();
    if plays.len() < 2 {
        return Ok(Vec::new());
    }
    let deals = possible_remaining_deals(params)?;
    let Some(maker) = view.maker_seat else {
        return Ok(Vec::new());
    };
    if deals.is_empty() {
        return Ok(Vec::new());
    }
    let maker_team = team_of(maker).index();
    let my_team = team_of(view.you).index();
    // (points, tricks, made, swept)
    let mut counters: Vec<(i64, i64, u32, u32)> = vec![(0, 0, 0, 0); plays.len()];
    for initial in &deals {
        for (i, play) in plays.iter().enumerate() {
            let end = run_reference(
                initial,
                view.you,
                play.mv,
                "Invalid play rollout move",
                "Play rollout did not score",
            )?;
            let result = end.hand.tricks_won;
            let maker_points: i64 = if result[maker_team] < 3 {
                -2
            } else if result[maker_team] < 5 {
                1
            } else if view.alone_seat.is_some() {
                4
            } else {
                2
            };
            let c = &mut counters[i];
            c.0 += if my_team == maker_team {
                maker_points
            } else {
                -maker_points
            };
            c.1 += i64::from(result[my_team]);
            c.2 += u32::from(result[maker_team] >= 3);
            c.3 += u32::from(result[maker_team] == 5);
        }
    }
    let n = deals.len() as f64;
    Ok(plays
        .iter()
        .zip(counters)
        .map(|(play, (points, tricks, made, swept))| {
            (
                play.id.clone(),
                PlayRollout {
                    sampled_deals: deals.len(),
                    mean_team_net_points: round2(points as f64 / n),
                    mean_team_tricks: round2(tricks as f64 / n),
                    maker_made_hands: made,
                    maker_swept_hands: swept,
                    limitation: PLAY_LIMITATION,
                },
            )
        })
        .collect())
}
