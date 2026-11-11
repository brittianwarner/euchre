//! Port of `src/lib/ai/play-rollouts.test.ts`.

mod common;

use common::*;
use euchre_ai::heuristic::void_reads;
use euchre_ai::play_rollouts::possible_remaining_deals;
use euchre_ai::{PlayRolloutParams, RankParams, play_rollouts, top_move};
use euchre_engine::{
    GamePhase, Move, PublicGameView, Seat, advance, apply, assert_invariants, effective_suit,
    project,
};
use serde_json::json;
use std::sync::OnceLock;

fn positions() -> &'static [PublicGameView] {
    static VIEWS: OnceLock<Vec<PublicGameView>> = OnceLock::new();
    VIEWS.get_or_init(|| {
        let mut views = Vec::new();
        for s in 0..12u8 {
            let mut state = game("eval", &format!("rollout-constraint-{s}"), Some(s % 4));
            let mut step = 0;
            while step < 150 && state.hand.hand_no < 2 {
                step += 1;
                let Some(turn) = state.hand.turn_seat else {
                    state = advance(&state).state;
                    continue;
                };
                let view = project(&state, turn);
                if view.phase == GamePhase::TrickPlay && view.legal.len() > 1 {
                    views.push(view.clone());
                }
                let want = if view.phase == GamePhase::Cutting {
                    "cut:no".to_owned()
                } else {
                    top_move(RankParams::new(&view)).unwrap()
                };
                let mv = view.legal.iter().find(|m| m.id == want).unwrap().mv;
                state = apply(&state, turn, mv, None).unwrap().state;
            }
        }
        views
    })
}

fn deals(view: &PublicGameView, target: usize) -> Vec<euchre_engine::GameState> {
    possible_remaining_deals(PlayRolloutParams {
        view,
        legal: &view.legal,
        target_samples: target,
    })
    .unwrap()
}

fn rollouts(
    view: &PublicGameView,
    legal: &[euchre_engine::LegalMove],
    target: usize,
) -> Vec<(String, euchre_ai::PlayRollout)> {
    play_rollouts(PlayRolloutParams {
        view,
        legal,
        target_samples: target,
    })
    .unwrap()
}

#[test]
fn accounts_for_every_card_and_respects_proved_voids_and_dealer_pickup_across_real_positions() {
    let views = positions();
    assert!(views.len() > 80, "{} positions", views.len());
    let mut sampled = 0;
    for view in views.iter().step_by(3) {
        let voids = void_reads(view);
        for deal in deals(view, 4) {
            sampled += 1;
            assert_invariants(&deal).unwrap();
            assert_eq!(deal.hand.hands[view.you.index()], view.hand);
            for s in Seat::ALL {
                if Some(s) == view.sitting_seat {
                    continue;
                }
                assert!(
                    !deal.hand.hands[s.index()]
                        .iter()
                        .any(|card| voids[s.index()].contains(&effective_suit(*card, view.trump)))
                );
            }
            if let (false, Some(up)) = (view.up_card_turned_down, view.up_card) {
                let owner = Seat::ALL
                    .into_iter()
                    .find(|s| deal.hand.hands[s.index()].contains(&up));
                assert!(owner.is_none() || owner == Some(view.dealer_seat));
            }
        }
    }
    assert!(sampled > 80, "{sampled} deals");
}

#[test]
fn is_independent_of_option_order_and_cannot_read_private_cards_or_seeds() {
    let view = &positions()[0];
    let expected = rollouts(view, &view.legal, 6);
    assert_eq!(expected.len(), view.legal.len());
    let mut reversed = view.legal.clone();
    reversed.reverse();
    let mut got = rollouts(view, &reversed, 6);
    got.reverse();
    assert_eq!(got, expected);
    let bad = poisoned(
        view,
        json!({ "seed": "secret", "hands": { "0": ["JH"] }, "dealerDiscard": "AH" }),
    );
    assert_eq!(rollouts(&bad, &view.legal, 6), expected);
}

#[test]
fn does_not_invent_reference_evidence_for_incomplete_fixtures_or_forced_moves() {
    let view = &positions()[0];
    let mut empty = view.clone();
    empty.hand_counts = [0, 0, 0, 0];
    assert!(
        possible_remaining_deals(PlayRolloutParams::new(&empty, &empty.legal))
            .unwrap()
            .is_empty()
    );
    assert!(
        play_rollouts(PlayRolloutParams::new(view, &view.legal[..1]))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn buries_the_accepted_up_card_when_the_dealer_sits_out_on_partners_loner() {
    let mut state = game("sitting-dealer", "sitting-dealer", Some(2));
    state = advance(&state).state;
    state = apply(&state, seat(1), Move::Cut { cut: false }, None)
        .unwrap()
        .state;
    state = advance(&state).state;
    state = apply(&state, seat(3), Move::Pass, None).unwrap().state;
    state = apply(&state, seat(0), Move::OrderUp { alone: true }, None)
        .unwrap()
        .state;
    let view = project(&state, seat(3));
    assert_eq!(view.phase, GamePhase::TrickPlay);
    assert_eq!(view.sitting_seat, Some(seat(2)));
    assert!(!view.up_card_turned_down);
    let deals = deals(&view, 8);
    assert_eq!(deals.len(), 8);
    for deal in deals {
        assert_invariants(&deal).unwrap();
        assert_eq!(deal.hand.dealer_discard, view.up_card);
        assert!(!deal.hand.hands[2].contains(&view.up_card.unwrap()));
    }
}

#[test]
fn gives_bounded_estimates_and_never_changes_the_view() {
    let view = positions().iter().find(|v| v.trick.index == 2).unwrap();
    let before = serde_json::to_string(view).unwrap();
    let result = rollouts(view, &view.legal, 8);
    assert!(result.len() > 1);
    for (_, estimate) in &result {
        assert!(estimate.sampled_deals > 0);
        assert!(estimate.sampled_deals <= 8);
        assert!((-4.0..=4.0).contains(&estimate.mean_team_net_points));
        assert!((0.0..=5.0).contains(&estimate.mean_team_tricks));
    }
    assert_eq!(serde_json::to_string(view).unwrap(), before);
}
