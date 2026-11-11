//! `assert_invariants` / `assert_legal_non_empty` against the TS validator:
//! `fixtures/invariants.json` (from `scripts/invariants.ts`) holds real states
//! with one invariant broken each, and the exact TS `InvariantError` message.

mod common;

use euchre_engine::{
    GameState, InvariantError, assert_invariants, assert_legal_non_empty, legal_moves,
};
use serde::Deserialize;

const FIXTURE: &str = include_str!("fixtures/invariants.json");

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    label: String,
    legal: bool,
    message: Option<String>,
    state: GameState,
}

fn cases() -> Vec<Case> {
    let f: Fixture =
        serde_json::from_str(FIXTURE).unwrap_or_else(|e| panic!("invariants.json: {e}"));
    f.cases
}

#[test]
fn every_violation_message_matches_ts_byte_for_byte() {
    let all = cases();
    assert!(all.len() >= 20);
    for c in &all {
        let got = if c.legal {
            assert_legal_non_empty(&c.state, legal_moves)
        } else {
            assert_invariants(&c.state)
        };
        assert_eq!(
            got.err().map(|e| e.to_string()),
            c.message,
            "case {:?}",
            c.label
        );
    }
}

#[test]
fn the_error_carries_its_invariant_id_and_detail() {
    let all = cases();
    let c = all
        .iter()
        .find(|c| c.label == "V5 dealer holds six")
        .unwrap();
    let err = assert_invariants(&c.state).unwrap_err();
    assert_eq!(
        err,
        InvariantError::new("V5", "dealer holds 6 after discarding")
    );
    assert_eq!(err.invariant, "V5");
}

#[test]
fn covers_every_state_decidable_invariant() {
    let ids: std::collections::BTreeSet<String> = cases()
        .iter()
        .filter_map(|c| c.message.as_ref())
        .filter_map(|m| m.split(' ').next().map(str::to_owned))
        .collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    assert_eq!(
        ids,
        [
            "V10", "V11", "V14", "V17", "V18", "V19", "V4", "V5", "V8", "V9"
        ]
    );
}

#[test]
fn covers_the_v18_length_branches_and_the_v8_over_v9_precedence() {
    let all = cases();
    let msg = |label: &str| {
        all.iter()
            .find(|c| c.label == label)
            .unwrap_or_else(|| panic!("missing case {label:?}"))
            .message
            .clone()
    };
    assert_eq!(
        msg("V18 trickLog length").as_deref(),
        Some("V18 violated: trickLog has 6 entries")
    );
    assert_eq!(
        msg("V18 bids length").as_deref(),
        Some("V18 violated: bids has 9 entries")
    );
    // The V9 "sitting seat won a trick" branch cannot fire: the sitting seat
    // would have to lead the next trick, which V8 rejects first, in TS and Rust.
    assert_eq!(
        msg("V9 sitting seat leads the next trick (V8 fires first)").as_deref(),
        Some("V8 violated: sitting seat 3 played in trick 1")
    );
}

#[test]
fn the_v16_trump_census_holds_for_every_trump() {
    // V16 depends only on the constant DECK and the trump suit, so no state can
    // violate it; check it passes for each trump on an otherwise valid state.
    let all = cases();
    let base = &all
        .iter()
        .find(|c| c.label == "valid trick play")
        .unwrap_or_else(|| panic!("missing valid trick play"))
        .state;
    for suit in common::ALL_SUITS {
        let mut s = base.clone();
        s.hand.trump = Some(suit);
        assert_eq!(assert_invariants(&s), Ok(()), "trump {suit}");
    }
}
