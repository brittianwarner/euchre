//! Cheap, deterministic sampling of possible deals from the bidder's view only
//! (`ai/bid-rollouts.ts`). Approximate outcomes under the reference play policy
//! ([`top_move`]), never solver truth or calibrated probabilities.
//!
//! Synchronous and CPU-bound: call it from `spawn_blocking` on an async runtime.

use euchre_engine::{
    Card, DECK, EngineConfigOverrides, GameInit, GamePhase, GameState, LegalMove, LegalMoveId,
    Move, PublicGameView, Seat, TableRules, project, team_of, try_advance, try_apply,
    try_create_game,
};
use serde::Serialize;

use crate::heuristic::{RankParams, top_move};
use crate::js::{quote, round2};
use crate::rng::{random_for, shuffle_in_place};

/// `BID_ROLLOUT_SAMPLES`.
pub const BID_ROLLOUT_SAMPLES: u32 = 32;

/// The step cap of one reference continuation.
const MAX_STEPS: usize = 40;

/// Why a reference rollout could not finish. The TS code throws; the decision
/// ladder then falls back to the heuristic.
#[derive(Debug, thiserror::Error)]
pub enum RolloutError {
    /// The reference policy named no legal move (`Invalid reference rollout move`).
    #[error("{0}")]
    InvalidMove(&'static str),
    /// The continuation did not reach `hand_score` (`Reference rollout did not finish`).
    #[error("{0}")]
    DidNotFinish(&'static str),
    /// The engine rejected a move or a state.
    #[error("rollout engine error: {0}")]
    Engine(String),
}

/// One bid's sampled outcome (key order as in TS).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BidRollout {
    /// Deals sampled.
    pub sampled_deals: u32,
    /// Deals where our team took at least three tricks.
    pub made_hands: u32,
    /// Deals where our team took all five.
    pub swept_hands: u32,
    /// Mean net points, rounded to two decimals.
    #[serde(with = "euchre_protocol::serde_util::js_f64")]
    pub mean_net_points: f64,
    /// Plain-language assessment.
    pub assessment: &'static str,
    /// The standing caveat.
    pub limitation: &'static str,
}

/// Parameters of [`bid_rollouts`].
#[derive(Clone, Copy, Debug)]
pub struct BidRolloutParams<'a> {
    /// The bidder's view.
    pub view: &'a PublicGameView,
    /// The legal (candidate) set.
    pub legal: &'a [LegalMove],
    /// Deals to sample; TS default [`BID_ROLLOUT_SAMPLES`].
    pub samples: u32,
}

impl<'a> BidRolloutParams<'a> {
    /// The default sample count.
    pub fn new(view: &'a PublicGameView, legal: &'a [LegalMove]) -> BidRolloutParams<'a> {
        BidRolloutParams {
            view,
            legal,
            samples: BID_ROLLOUT_SAMPLES,
        }
    }
}

const BID_LIMITATION: &str = "Uniform possible unseen deals, not actual hidden cards. Reference heuristic play, small sample, ignores uncertain inferences from auction passes. This is an estimate, not a guarantee or Jev confidence.";

/// `createGame({ gameId, seed: 'possible-deals-only', cfg: view.rules })`.
pub(crate) fn base_game(
    game_id: &str,
    rules: Option<TableRules>,
) -> Result<GameState, RolloutError> {
    try_create_game(GameInit {
        game_id: game_id.to_owned(),
        seed: "possible-deals-only".to_owned(),
        cfg: rules.map(|r| EngineConfigOverrides {
            stick_the_dealer: Some(r.stick_the_dealer),
            require_natural_trump: Some(r.require_natural_trump),
            ..EngineConfigOverrides::default()
        }),
        internal_token: None,
        first_dealer: None,
    })
    .map_err(|e| RolloutError::Engine(e.to_string()))
}

/// Plays `mv` for `seat`, then continues under the reference policy until
/// `hand_score` or [`MAX_STEPS`].
pub(crate) fn run_reference(
    initial: &GameState,
    seat: Seat,
    mv: Move,
    invalid: &'static str,
    unfinished: &'static str,
) -> Result<GameState, RolloutError> {
    let mut state = try_apply(initial, seat, mv, None)
        .map_err(|e| RolloutError::Engine(e.to_string()))?
        .state;
    let mut step = 0;
    while state.hand.phase != GamePhase::HandScore && step < MAX_STEPS {
        step += 1;
        let Some(turn) = state.hand.turn_seat else {
            state = try_advance(&state)
                .map_err(|e| RolloutError::Engine(e.to_string()))?
                .state;
            continue;
        };
        let player = project(&state, turn);
        let id = top_move(RankParams::new(&player));
        let Some(chosen) = player.legal.iter().find(|c| Some(&c.id) == id.as_ref()) else {
            return Err(RolloutError::InvalidMove(invalid));
        };
        state = try_apply(&state, turn, chosen.mv, None)
            .map_err(|e| RolloutError::Engine(e.to_string()))?
            .state;
    }
    if state.hand.phase != GamePhase::HandScore {
        return Err(RolloutError::DidNotFinish(unfinished));
    }
    Ok(state)
}

/// `JSON.stringify([...cards].sort())`.
pub(crate) fn sorted_cards_json(cards: &[Card]) -> String {
    let mut sorted = cards.to_vec();
    sorted.sort();
    let parts: Vec<String> = sorted.iter().map(|c| quote(&c.to_string())).collect();
    format!("[{}]", parts.join(","))
}

/// `bidRollouts(view, legal, samples)`: one entry per order-up/call, in legal
/// order; empty (the TS `{}`) when the view is not a five-card bid.
pub fn bid_rollouts(
    params: BidRolloutParams<'_>,
) -> Result<Vec<(LegalMoveId, BidRollout)>, RolloutError> {
    let BidRolloutParams {
        view,
        legal,
        samples,
    } = params;
    let bids: Vec<&LegalMove> = legal
        .iter()
        .filter(|m| matches!(m.mv, Move::OrderUp { .. } | Move::Call { .. }))
        .collect();
    let Some(up) = view.up_card else {
        return Ok(Vec::new());
    };
    if bids.is_empty()
        || view.hand.len() != 5
        || !matches!(view.phase, GamePhase::BidRound1 | GamePhase::BidRound2)
    {
        return Ok(Vec::new());
    }
    let seed_text = format!(
        "[{},{},{},{},{}]",
        view.you.get(),
        view.dealer_seat.get(),
        quote(view.phase.as_str()),
        sorted_cards_json(&view.hand),
        quote(&up.to_string())
    );
    let mut rng = random_for(&seed_text);
    let unseen: Vec<Card> = DECK
        .iter()
        .copied()
        .filter(|c| !view.hand.contains(c) && *c != up)
        .collect();
    let base = base_game("bid-estimate", view.rules)?;
    // (made, swept, points) per bid, in bid order.
    let mut counts: Vec<(u32, u32, i64)> = vec![(0, 0, 0); bids.len()];
    let team = team_of(view.you).index();

    for _ in 0..samples {
        let mut shuffled = unseen.clone();
        shuffle_in_place(&mut shuffled, &mut rng);
        let mut initial = base.clone();
        let mut offset = 0usize;
        for seat in Seat::ALL {
            let cards = if seat == view.you {
                view.hand.clone()
            } else {
                let end = (offset + 5).min(shuffled.len());
                let slice = shuffled[offset.min(end)..end].to_vec();
                offset += 5;
                slice
            };
            initial.hand.hands[seat.index()] = cards;
        }
        let (Some(k1), Some(k2), Some(k3)) = (
            shuffled.get(offset).copied(),
            shuffled.get(offset + 1).copied(),
            shuffled.get(offset + 2).copied(),
        ) else {
            return Err(RolloutError::Engine("not enough unseen cards".to_owned()));
        };
        let h = &mut initial.hand;
        h.phase = view.phase;
        h.dealer_seat = view.dealer_seat;
        h.turn_seat = Some(view.you);
        h.up_card = Some(up);
        h.up_card_turned_down = view.up_card_turned_down;
        h.turned_down_suit = view.turned_down_suit;
        h.passes = view.passes;
        h.bids = view.bids.clone();
        h.kitty = [up, k1, k2, k3];

        for (i, bid) in bids.iter().enumerate() {
            let end = run_reference(
                &initial,
                view.you,
                bid.mv,
                "Invalid reference rollout move",
                "Reference rollout did not finish",
            )?;
            let tricks = end.hand.tricks_won[team];
            let alone = matches!(
                bid.mv,
                Move::OrderUp { alone: true } | Move::Call { alone: true, .. }
            );
            let c = &mut counts[i];
            c.0 += u32::from(tricks >= 3);
            c.1 += u32::from(tricks == 5);
            c.2 += if tricks < 3 {
                -2
            } else if tricks < 5 {
                1
            } else if alone {
                4
            } else {
                2
            };
        }
    }

    let n = f64::from(samples);
    Ok(bids
        .iter()
        .zip(counts)
        .map(|(bid, (made, swept, points))| {
            let rate = f64::from(made) / n;
            let assessment = if rate < 0.5 {
                "poor: euchred in most reference deals"
            } else if rate < 0.7 {
                "marginal: considerable euchre risk"
            } else if rate < 0.85 {
                "supported: made in most reference deals"
            } else {
                "strong: made in almost all reference deals"
            };
            (
                bid.id.clone(),
                BidRollout {
                    sampled_deals: samples,
                    made_hands: made,
                    swept_hands: swept,
                    mean_net_points: round2(points as f64 / n),
                    assessment,
                    limitation: BID_LIMITATION,
                },
            )
        })
        .collect())
}
