//! Port of `src/lib/ai/bid-rollouts.test.ts`.

mod common;

use common::*;
use euchre_ai::{BidRolloutParams, bid_rollouts};
use euchre_engine::TableRules;
use serde_json::json;

fn rollouts(
    view: &euchre_engine::PublicGameView,
    legal: &[euchre_engine::LegalMove],
) -> Vec<(String, euchre_ai::BidRollout)> {
    bid_rollouts(BidRolloutParams::new(view, legal)).unwrap()
}

/// Order-insensitive equality: the TS `toEqual` on an object ignores key order.
fn sorted<T: Clone>(mut v: Vec<(String, T)>) -> Vec<(String, T)> {
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

#[test]
fn is_reproducible_and_independent_of_option_order() {
    let view = &bidding_cases()[1].view;
    let mut reversed = view.legal.clone();
    reversed.reverse();
    assert_eq!(
        sorted(rollouts(view, &view.legal)),
        sorted(rollouts(view, &reversed))
    );
}

#[test]
fn does_not_depend_on_unknown_deck_order_private_seed_or_another_hand() {
    let view = &bidding_cases()[0].view;
    let bad = poisoned(
        view,
        json!({
            "seed": "completely different",
            "deckOrder": ["AH"],
            "hands": { "0": ["JH"] },
            "kitty": ["JD"],
            "internalToken": "never-read"
        }),
    );
    assert_eq!(rollouts(&bad, &view.legal), rollouts(view, &view.legal));
}

#[test]
fn rejects_the_reported_poor_bids_but_recognizes_a_strong_team_call() {
    let cases = bidding_cases();
    for scenario in &cases[0..2] {
        let result = rollouts(&scenario.view, &scenario.view.legal);
        let order_up = entry(&result, "orderUp").unwrap();
        assert!(order_up.mean_net_points < 0.0, "{}", scenario.name);
        assert!(order_up.assessment.contains("poor"), "{}", scenario.name);
    }
    let good = &cases[3];
    let result = rollouts(&good.view, &good.view.legal);
    assert!(entry(&result, "orderUp").unwrap().mean_net_points > 0.0);
}

#[test]
fn handles_partner_alone_sitting_out_left_bowers_and_natural_trump_restrictions() {
    let view = &bidding_cases()[5].view;
    let result = rollouts(view, &view.legal);
    assert!(
        entry(&result, "orderUp").unwrap().made_hands
            > entry(&result, "orderUp+alone").unwrap().made_hands
    );
    let hand: Vec<String> = view.hand.iter().map(ToString::to_string).collect();
    let hand: Vec<&str> = hand.iter().map(String::as_str).collect();
    let restricted = bid_view(
        &hand,
        "JH",
        BidOpts {
            dealer: Some(3),
            rules: Some(TableRules {
                stick_the_dealer: true,
                require_natural_trump: true,
            }),
            ..BidOpts::default()
        },
        |_| {},
    );
    assert!(rollouts(&restricted, &restricted.legal).is_empty());
}

#[test]
fn keeps_forced_dealer_options_available_and_the_turned_down_suit_out() {
    let view = &bidding_cases()[8].view;
    let result = rollouts(view, &view.legal);
    assert!(entry(&result, "call:H").is_some());
    assert!(entry(&result, "call:D").is_none());
    assert!(entry(&result, "pass").is_none());
}
