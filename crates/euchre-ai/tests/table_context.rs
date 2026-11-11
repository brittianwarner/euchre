//! Port of the pure parts of `src/lib/ai/table-context.test.ts`. The
//! observations test runs through `build_decision_question` (the exact state
//! Jev receives) instead of a scripted `callModel`; the conversational
//! `buildLayers` test and the `personasFromSettings` snapshot test belong to
//! the LLM stage.

mod common;

use common::*;
use euchre_ai::decision_context::{DecisionQuestionParams, build_decision_question};
use euchre_ai::notation::{EncodeParams, encode_for_llm};
use euchre_ai::table_context::table_context;
use euchre_engine::{
    GamePhase, ObservationKind, ObservationTiming, PublicGameView, Suit, TableObservation, Trick,
};

fn base() -> PublicGameView {
    play_view(
        &["JD", "9H", "AC"],
        &[play(0, "AS"), play(1, "9C")],
        None,
        |_| {},
    )
}

#[test]
fn counts_the_left_bower_as_trump_and_records_proved_voids_from_attributed_plays() {
    let ctx = table_context(&base());
    assert_eq!(ctx.cards_in_each_effective_suit.h, 2);
    assert_eq!(ctx.cards_in_each_effective_suit.d, 0);
    assert_eq!(ctx.cards_in_each_effective_suit.c, 1);
    let read = ctx.public_reads.iter().find(|r| r.seat == seat(1)).unwrap();
    assert_eq!(read.proved_voids, vec![Suit::Spades]);
    assert_eq!(read.cards_played, cards(&["9C"]));
    assert!(!ctx.unseen_trump.contains(&c("JD")));
    assert!(ctx.unseen_trump.contains(&c("JH")));
}

#[test]
fn keeps_buried_trump_possibilities_uncertain_and_never_assumes_a_picked_up_card_is_retained() {
    let mut picked = base();
    picked.up_card = Some(c("JH"));
    picked.up_card_turned_down = false;
    picked.turned_down_suit = None;
    let ctx = table_context(&picked);
    assert!(ctx.unseen_trump.contains(&c("JH")));
    assert!(ctx.auction.up_card_location.contains("discarded"));
    let mut turned = base();
    turned.up_card = Some(c("JH"));
    turned.up_card_turned_down = true;
    assert!(!table_context(&turned).unseen_trump.contains(&c("JH")));
}

#[test]
fn remembers_who_played_each_past_card_without_doubling_a_resolved_trick() {
    let trick = Trick {
        index: 0,
        led_suit: Some(Suit::Spades),
        winner_seat: Some(seat(0)),
        plays: vec![play(0, "AS"), play(1, "9C")],
    };
    let mut view = base();
    view.phase = GamePhase::TrickResolve;
    view.trick = trick.clone();
    view.trick_log = vec![trick];
    let ctx = table_context(&view);
    let read = ctx.public_reads.iter().find(|r| r.seat == seat(1)).unwrap();
    assert_eq!(read.cards_played, cards(&["9C"]));
    let text = encode_for_llm(EncodeParams {
        view: &view,
        legal: &[],
        ranking: None,
        notes: None,
    });
    assert!(text.contains("trick1:led=S,winner=0<0:AS 1:9C"), "{text}");
}

#[test]
fn changes_the_objective_after_making_the_hand_and_when_defending_a_loner() {
    let mut made = base();
    made.tricks_won = [3, 0];
    assert!(table_context(&made).objective.contains("sweep"));
    let mut loner = base();
    loner.maker_seat = Some(seat(1));
    loner.alone_seat = Some(seat(1));
    loner.tricks_won = [0, 0];
    assert!(table_context(&loner).objective.contains("four-point"));
}

#[test]
fn sends_observations_and_their_limitations_in_the_jev_state_without_private_game_fields() {
    let mut view = base();
    view.observations = Some(vec![TableObservation {
        hand_no: view.hand_no,
        seat: seat(0),
        kind: ObservationKind::Play,
        action: "Ace of spades".to_owned(),
        elapsed_ms: Some(3250),
        timing: ObservationTiming::HumanObserved,
    }]);
    let ned = &personas()[1];
    let q = build_decision_question(DecisionQuestionParams {
        view: &view,
        legal: &view.legal,
        persona: ned,
    })
    .unwrap();
    let state = serde_json::to_string(&q.state).unwrap();
    assert!(state.contains("human-observed"));
    assert!(state.contains("Computer timing"));
    for banned in [
        "deckOrder",
        "internalToken",
        "dealerDiscard",
        "\"kitty\"",
        "seed=",
    ] {
        assert!(!state.contains(banned), "{banned}");
    }
}

#[test]
fn gives_grandpa_calculated_risk_without_degrading_partnership_play() {
    let personas = personas();
    let names: Vec<&str> = personas.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["Rita", "Ned", "Grandpa"]);
    assert!(personas[2].risk > personas[0].risk);
    assert!(personas[2].prompt.contains("never deliberately misplay"));
}
