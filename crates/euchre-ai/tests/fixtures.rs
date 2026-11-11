//! The converted TS regression positions: counts, engine legality, and TS parity
//! of `rankMoves` and the Jev request body for every case.

mod common;

use common::*;
use euchre_ai::decision_context::{
    DecisionQuestionParams, build_decision_question, jev_request_body,
};
use euchre_ai::{RankParams, rank_moves, top_move};
use euchre_protocol::serde_util::js_number_string;

fn rank_line(view: &euchre_engine::PublicGameView) -> String {
    let ranked = rank_moves(RankParams::new(view));
    let top = top_move(RankParams::new(view)).unwrap_or_else(|| "null".to_owned());
    let mut line: String = ranked
        .iter()
        .map(|r| format!("{}|{}|{};", r.id, js_number_string(r.score), r.why))
        .collect();
    line.push_str(&format!("top={top}"));
    line
}

fn check(cases: &[Case]) {
    let rita = &personas()[0];
    for case in cases {
        assert_eq!(rank_line(&case.view), case.rank, "{}: rankMoves", case.name);
        let q = build_decision_question(DecisionQuestionParams {
            view: &case.view,
            legal: &case.view.legal,
            persona: rita,
        })
        .unwrap();
        assert_eq!(jev_request_body(&q), case.jev, "{}: Jev body", case.name);
    }
}

#[test]
fn fixture_counts_match_the_ts_modules() {
    assert_eq!(bidding_cases().len(), 10);
    assert_eq!(acceptance_cases().len(), 15);
    assert_eq!(partnership_cases().len(), 7);
    assert_eq!(strategy_cases().len(), 6);
}

#[test]
fn bidding_cases_match_ts() {
    check(&bidding_cases());
}

#[test]
fn acceptance_cases_match_ts() {
    check(&acceptance_cases());
}

#[test]
fn partnership_cases_match_ts() {
    check(&partnership_cases());
}

#[test]
fn strategy_cases_match_ts() {
    check(&strategy_cases());
}

#[test]
fn expected_moves_are_legal() {
    for case in bidding_cases()
        .into_iter()
        .chain(acceptance_cases())
        .chain(partnership_cases())
        .chain(strategy_cases())
    {
        let legal = ids(&case.view);
        let expected: Vec<String> = match &case.expected {
            serde_json::Value::String(s) => vec![s.clone()],
            v => serde_json::from_value(v.clone()).unwrap(),
        };
        for id in expected {
            assert!(legal.contains(&id), "{}: {id} not legal", case.name);
        }
    }
}

#[test]
fn bid_view_port_matches_the_ts_fixture() {
    // biddingCases[1]: `bidView(['TH','9C','KC','JD','AS'], 'QS', { score: [2, 1] })`.
    let ts = &bidding_cases()[1].view;
    let rust = bid_view(
        &["TH", "9C", "KC", "JD", "AS"],
        "QS",
        BidOpts::default(),
        |v| v.score = [2, 1],
    );
    assert_eq!(
        serde_json::to_value(&rust).unwrap(),
        serde_json::to_value(ts).unwrap()
    );
}

#[test]
fn play_view_port_matches_the_ts_fixture() {
    // partnershipCases[0].
    let ts = &partnership_cases()[0].view;
    let rust = play_view(
        &["AS", "TS", "9C", "KC", "AD"],
        &[play(3, "9S"), play(0, "KS"), play(1, "QS")],
        None,
        |_| {},
    );
    assert_eq!(
        serde_json::to_value(&rust).unwrap(),
        serde_json::to_value(ts).unwrap()
    );
}
