//! The scripted-Jev tests of the TS suite: decision-context.test.ts "Jev owns
//! bid and loner choices", table-context.test.ts (observations through the
//! actual Jev request), partnership.test.ts "Jev integration", plus the rungs of
//! the alternate `decide()`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use euchre_ai::ai_seat::ladder::{LadderDeps, run_ladder};
use euchre_ai::decide::{
    CallModelError, CallModelOptions, CallModelRequest, DecideDeps, DecideOptions, call_model,
    decide,
};
use euchre_ai::decision_context::JEV_MODEL;
use euchre_ai::jev::{JevDecisionError, JevDecisionReason, JevError};
use euchre_ai::model::{ModelFactory, model_factory_from_env};
use euchre_ai::testing::{
    HangingJev, ManualClock, RecordingLog, ScriptedDecider, ScriptedJev, ScriptedTalk, jev_choice,
    jev_factory,
};
use euchre_ai::types::{DecisionBudgetOverrides, JevChoiceAnswer, JevDecisionOutput};
use euchre_ai::{RankParams, rank_moves};
use euchre_engine::PublicGameView;
use euchre_engine::{AiDecisionSource, ObservationKind, ObservationTiming, TableObservation};
use euchre_protocol::{AIDecideRequest, AIDecisionKind, PersonaAssignment, default_personas};
use serde_json::{Value, json};

const NOW: u64 = 1_700_000_000_000;

fn assignment(index: usize) -> PersonaAssignment {
    default_personas().remove(index)
}

fn request(view: &PublicGameView, kind: AIDecisionKind, deadline_in: u64) -> AIDecideRequest {
    AIDecideRequest {
        internal_token: String::new(),
        game_id: String::new(),
        seat: view.you,
        turn_id: String::new(),
        kind,
        view: view.clone(),
        legal: view.legal.clone(),
        ranking: None,
        deadline_at: NOW + deadline_in,
    }
}

fn answer(choice: &str, probabilities: Value) -> JevChoiceAnswer {
    JevChoiceAnswer {
        kind: "choice".to_owned(),
        choice: choice.to_owned(),
        confidence: 0.8,
        probabilities: probabilities.as_object().cloned().unwrap(),
    }
}

/// A factory whose talk model has no script: any text call would be recorded.
fn factory(jev: Arc<ScriptedJev>) -> (ModelFactory, Arc<ScriptedTalk>) {
    let talk = Arc::new(ScriptedTalk::new(Vec::new()));
    (jev_factory(jev, talk.clone()), talk)
}

fn deps<'a>(
    a: &'a PersonaAssignment,
    factory: Option<&'a ModelFactory>,
    log: &'a RecordingLog,
    clock: &'a ManualClock,
    nonce: &'a str,
) -> DecideDeps<'a> {
    DecideDeps {
        factory,
        persona: &a.persona,
        dossier: &a.dossier,
        nonce,
        log,
        clock,
        budget: None,
        cancel: None,
    }
}

const BUDGET: CallModelOptions = CallModelOptions {
    escalate: false,
    budget_ms: 4000.0,
};

/* -------------------------------------------------------------------------- */
/* Jev owns bid and loner choices                                             */
/* -------------------------------------------------------------------------- */

#[tokio::test]
async fn composes_the_two_jev_judgments_without_a_heuristic_override() {
    let rita = assignment(0);
    let view = bidding_cases().remove(0).view;
    let mut out = jev_choice("move_1", 0.8);
    out.additional_answers = Some(vec![
        (
            "support_1".to_owned(),
            answer("call", json!({ "pass": 0.1, "call": 0.9 })),
        ),
        (
            "loner_1".to_owned(),
            answer("alone", json!({ "partner": 0.1, "alone": 0.9 })),
        ),
    ]);
    let jev = Arc::new(ScriptedJev::new(vec![Ok(out)]));
    let (factory, talk) = factory(jev.clone());
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let req = request(&view, AIDecisionKind::Bid1, 5000);
    let result = call_model(
        &deps(&rita, Some(&factory), &log, &clock, "test"),
        CallModelRequest::from_request(&req),
        &view.legal,
        BUDGET,
    )
    .await
    .unwrap();
    assert_eq!(result.move_id, "orderUp+alone");
    // Intentionally bad mocked judgment is preserved: this is evidence, not a hard-coded ban.
    assert_eq!(
        jev.calls()[0].state["auctionHands"]["Hearts"]["ownHolding"]["effectiveTrumpCount"],
        json!(0)
    );
    assert!(talk.calls().is_empty());
}

#[tokio::test]
async fn does_not_consume_speculative_loner_answers_when_jev_passes() {
    let rita = assignment(0);
    let view = bidding_cases().remove(0).view;
    let jev = Arc::new(ScriptedJev::new(vec![Ok(jev_choice("move_0", 1.0))]));
    let (factory, _talk) = factory(jev);
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let req = request(&view, AIDecisionKind::Bid1, 5000);
    let result = call_model(
        &deps(&rita, Some(&factory), &log, &clock, "test"),
        CallModelRequest::from_request(&req),
        &view.legal,
        BUDGET,
    )
    .await
    .unwrap();
    assert_eq!(result.move_id, "pass");
}

#[tokio::test]
async fn lets_the_independent_jev_support_judgment_reject_its_speculative_primary_call() {
    let rita = assignment(0);
    let view = bidding_cases().remove(0).view;
    let mut out = jev_choice("move_1", 0.8);
    out.probabilities = json!({ "move_0": 0.2, "move_1": 0.8 }).as_object().cloned();
    out.additional_answers = Some(vec![(
        "support_1".to_owned(),
        JevChoiceAnswer {
            confidence: 1.0,
            ..answer("pass", json!({ "pass": 1, "call": 0 }))
        },
    )]);
    let jev = Arc::new(ScriptedJev::new(vec![Ok(out)]));
    let (factory, _talk) = factory(jev);
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let req = request(&view, AIDecisionKind::Bid1, 5000);
    let result = call_model(
        &deps(&rita, Some(&factory), &log, &clock, "test"),
        CallModelRequest::from_request(&req),
        &view.legal,
        BUDGET,
    )
    .await
    .unwrap();
    assert_eq!(result.move_id, "pass");
    assert_eq!(result.source, AiDecisionSource::Llm);
    let logged = log.find("jev_decision").unwrap();
    assert_eq!(logged["primaryChoice"], json!("move_1"));
    assert_eq!(logged["resolvedChoice"], json!("move_0"));
    let all: Vec<Value> = log
        .lines()
        .into_iter()
        .map(|l| json!([l.event, Value::Object(l.fields)]))
        .collect();
    let text = serde_json::to_string(&all).unwrap();
    for banned in [
        "ownHand",
        "ownHolding",
        "Ace of spades",
        "internalToken",
        "secret",
        "JC",
    ] {
        assert!(!text.contains(banned), "{banned} in {text}");
    }
}

#[tokio::test]
async fn composition_failures_are_typed() {
    let rita = assignment(0);
    let view = bidding_cases().remove(0).view;
    let req = request(&view, AIDecisionKind::Bid1, 5000);
    let run = |out: JevDecisionOutput| {
        let rita = rita.clone();
        let req = req.clone();
        let view = view.clone();
        async move {
            let jev = Arc::new(ScriptedJev::new(vec![Ok(out)]));
            let (factory, _talk) = factory(jev);
            let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
            call_model(
                &deps(&rita, Some(&factory), &log, &clock, "test"),
                CallModelRequest::from_request(&req),
                &view.legal,
                BUDGET,
            )
            .await
            .unwrap_err()
        }
    };
    // A bid choice without its support answer.
    let err = run(jev_choice("move_1", 0.8)).await;
    assert!(matches!(err, CallModelError::MissingBidSupport));
    assert_eq!(err.to_string(), "Jev omitted the bid support decision");
    // Support says pass, and there is no distribution to fall back on.
    let mut out = jev_choice("move_1", 0.8);
    out.additional_answers = Some(vec![(
        "support_1".to_owned(),
        answer("pass", json!({ "pass": 0.9, "call": 0.1 })),
    )]);
    let err = run(out).await;
    assert_eq!(err.to_string(), "Jev omitted the move distribution");
    // Supported, but the loner judgment is missing.
    let mut out = jev_choice("move_1", 0.8);
    out.additional_answers = Some(vec![(
        "support_1".to_owned(),
        answer("call", json!({ "pass": 0.1, "call": 0.9 })),
    )]);
    let err = run(out).await;
    assert_eq!(err.to_string(), "Jev omitted the applicable loner decision");
    // A key outside the criteria.
    let err = run(jev_choice("move_99", 0.8)).await;
    assert_eq!(
        err.to_string(),
        "Jev returned a choice outside the legal set"
    );
}

/* -------------------------------------------------------------------------- */
/* table-context: observations through the actual Jev request                  */
/* -------------------------------------------------------------------------- */

#[tokio::test]
async fn sends_observations_and_their_limitations_through_the_actual_jev_request() {
    let mut view = play_view(
        &["JD", "9H", "AC"],
        &[play(0, "AS"), play(1, "9C")],
        None,
        |_| {},
    );
    view.observations = Some(vec![TableObservation {
        hand_no: view.hand_no,
        seat: seat(0),
        kind: ObservationKind::Play,
        action: "Ace of spades".to_owned(),
        elapsed_ms: Some(3250),
        timing: ObservationTiming::HumanObserved,
    }]);
    let ned = assignment(1);
    let jev = Arc::new(ScriptedJev::new(vec![Ok(jev_choice("move_0", 0.7))]));
    let (factory, talk) = factory(jev.clone());
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let mut req = request(&view, AIDecisionKind::Play, 5000);
    req.seat = seat(2);
    req.turn_id = "test".to_owned();
    call_model(
        &deps(&ned, Some(&factory), &log, &clock, "context-test"),
        CallModelRequest::from_request(&req),
        &view.legal,
        BUDGET,
    )
    .await
    .unwrap();
    let state = serde_json::to_string(&jev.calls()[0].state).unwrap();
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
    assert!(talk.calls().is_empty());
}

/* -------------------------------------------------------------------------- */
/* partnership: Jev integration                                               */
/* -------------------------------------------------------------------------- */

fn partner() -> PersonaAssignment {
    default_personas()
        .into_iter()
        .find(|a| a.seat == seat(2))
        .unwrap()
}

#[tokio::test]
async fn sends_partnership_facts_and_option_consequences_and_preserves_jev_final_authority() {
    let partner = partner();
    let view = partnership_cases().remove(0).view;
    // Deliberately choose the overtake: evidence must inform Jev, never replace it.
    let jev = Arc::new(ScriptedJev::new(vec![Ok(jev_choice("move_0", 0.9))]));
    let talk = Arc::new(ScriptedTalk::new(Vec::new()));
    let mut factory = model_factory_from_env(&[("OPENROUTER_API_KEY", "test")]).unwrap();
    factory.decision = Some(jev.clone());
    factory.talk = talk.clone();
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let ranking = rank_moves(RankParams::new(&view));
    let req = AIDecideRequest {
        internal_token: String::new(),
        game_id: "test".to_owned(),
        seat: seat(2),
        turn_id: "test".to_owned(),
        kind: AIDecisionKind::Play,
        view: view.clone(),
        legal: view.legal.clone(),
        ranking: Some(ranking),
        deadline_at: NOW + 10_000,
    };
    let outcome = call_model(
        &deps(&partner, Some(&factory), &log, &clock, "partnership-test"),
        CallModelRequest::from_request(&req),
        &view.legal,
        CallModelOptions {
            escalate: false,
            budget_ms: 5000.0,
        },
    )
    .await
    .unwrap();
    let sent = &jev.calls()[0];
    assert_eq!(
        sent.state["partnership"]["partnerTrickSecured"],
        json!(true)
    );
    assert_eq!(sent.criteria["move_1"]["card"], json!("TS"));
    assert_eq!(sent.criteria["move_1"]["overtakesPartner"], json!(false));
    assert_eq!(outcome.move_id, "play:AS");
    assert_eq!(outcome.source, AiDecisionSource::Llm);
    assert_eq!(outcome.model_id.as_deref(), Some(JEV_MODEL));
    assert!(talk.calls().is_empty());
}

#[tokio::test]
async fn offers_every_legal_card_to_jev_even_if_a_casual_heuristic_ranks_the_cover_last() {
    let partner = partner();
    let view = partnership_cases().remove(1).view;
    let decider = ScriptedDecider::always("play:JH");
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let req = AIDecideRequest {
        internal_token: String::new(),
        game_id: "test".to_owned(),
        seat: seat(2),
        turn_id: "test".to_owned(),
        kind: AIDecisionKind::Play,
        view: view.clone(),
        legal: view.legal.clone(),
        ranking: None,
        deadline_at: NOW + 10_000,
    };
    let result = run_ladder(
        LadderDeps {
            persona: &partner.persona,
            dossier: &partner.dossier,
            nonce: "partnership-test",
            degraded: false,
            clock: &clock,
            log: &log,
            decider: Some(&decider),
        },
        &req,
    )
    .await
    .unwrap();
    assert_eq!(decider.calls()[0].candidates, view.legal);
    assert_eq!(result.move_id, "play:JH");
    assert_eq!(result.source, AiDecisionSource::Llm);
    assert_eq!(result.candidates, 5);
    assert_eq!(result.llm_calls, 1);
}

/* -------------------------------------------------------------------------- */
/* decide(): every rung                                                       */
/* -------------------------------------------------------------------------- */

fn bid_request() -> AIDecideRequest {
    request(
        &bidding_cases().remove(0).view,
        AIDecisionKind::Bid1,
        10_000,
    )
}

#[tokio::test]
async fn decide_forces_a_single_legal_move_without_a_call() {
    let rita = assignment(0);
    let mut req = bid_request();
    req.legal.truncate(1);
    let jev = Arc::new(ScriptedJev::new(vec![Ok(jev_choice("move_0", 1.0))]));
    let (factory, _talk) = factory(jev.clone());
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let out = decide(
        &deps(&rita, Some(&factory), &log, &clock, "n"),
        &req,
        &req.legal,
        DecideOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(out.source, AiDecisionSource::Forced);
    assert_eq!(out.move_id, req.legal[0].id);
    assert_eq!(log.events(), ["ai_forced"]);
    assert!(jev.calls().is_empty());
}

#[tokio::test]
async fn decide_without_a_decision_client_or_when_degraded_uses_the_heuristic() {
    let rita = assignment(0);
    let req = bid_request();
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let out = decide(
        &deps(&rita, None, &log, &clock, "n"),
        &req,
        &req.legal,
        DecideOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(out.source, AiDecisionSource::Fallback);
    assert_eq!(out.attempts, 0);
    assert_eq!(log.events(), ["ai_no_provider"]);
    assert_eq!(
        log.find("ai_no_provider").unwrap(),
        json!({ "seat": req.seat.get(), "kind": "bid1" })
            .as_object()
            .cloned()
            .unwrap()
    );

    let jev = Arc::new(ScriptedJev::new(vec![Ok(jev_choice("move_0", 1.0))]));
    let (factory, _talk) = factory(jev.clone());
    let log = RecordingLog::new();
    let out = decide(
        &deps(&rita, Some(&factory), &log, &clock, "n"),
        &req,
        &req.legal,
        DecideOptions { degraded: true },
    )
    .await
    .unwrap();
    assert_eq!(out.source, AiDecisionSource::Fallback);
    assert_eq!(log.events(), ["ai_degraded"]);
    assert!(jev.calls().is_empty());
}

#[tokio::test]
async fn decide_accepts_the_first_answer_and_logs_ai_ok() {
    let rita = assignment(0);
    let req = bid_request();
    let jev = Arc::new(ScriptedJev::new(vec![Ok(jev_choice("move_0", 1.0))]));
    let (factory, _talk) = factory(jev.clone());
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let out = decide(
        &deps(&rita, Some(&factory), &log, &clock, "n"),
        &req,
        &req.legal,
        DecideOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(out.move_id, "pass");
    assert_eq!(out.source, AiDecisionSource::Llm);
    assert_eq!(out.attempts, 1);
    assert_eq!(log.events(), ["jev_decision", "ai_ok"]);
    let ok = log.find("ai_ok").unwrap();
    assert_eq!(ok["escalate"], json!(false));
    assert_eq!(ok["attempts"], json!(1));
}

fn short_budget() -> DecisionBudgetOverrides {
    DecisionBudgetOverrides {
        first_attempt_ms: Some(60),
        escalation_ms: Some(60),
        reserve_ms: Some(0),
        min_attempt_ms: Some(10),
        ..DecisionBudgetOverrides::default()
    }
}

#[tokio::test]
async fn decide_continues_after_a_timeout_and_escalates_once_on_deliberate_kinds() {
    let rita = assignment(0);
    let req = bid_request();
    let jev = Arc::new(HangingJev::new());
    let talk = Arc::new(ScriptedTalk::new(Vec::new()));
    let factory = jev_factory(jev.clone(), talk);
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let mut d = deps(&rita, Some(&factory), &log, &clock, "n");
    d.budget = Some(short_budget());
    let out = decide(&d, &req, &req.legal, DecideOptions::default())
        .await
        .unwrap();
    assert_eq!(out.source, AiDecisionSource::Fallback);
    assert_eq!(out.attempts, 2);
    assert_eq!(log.events(), ["ai_timeout", "ai_timeout"]);
    let second = &log.lines()[1].fields;
    assert_eq!(second["escalate"], json!(true));
    assert_eq!(second["name"], json!("TimeoutError"));
}

#[tokio::test]
async fn decide_never_escalates_a_play() {
    let rita = assignment(0);
    let view = partnership_cases().remove(1).view;
    let req = request(&view, AIDecisionKind::Play, 10_000);
    let jev = Arc::new(HangingJev::new());
    let factory = jev_factory(jev.clone(), Arc::new(ScriptedTalk::new(Vec::new())));
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let mut d = deps(&rita, Some(&factory), &log, &clock, "n");
    d.budget = Some(short_budget());
    let out = decide(&d, &req, &req.legal, DecideOptions::default())
        .await
        .unwrap();
    assert_eq!(out.attempts, 1);
    assert_eq!(log.events(), ["ai_timeout"]);
}

#[tokio::test]
async fn decide_stops_on_a_jev_decision_error() {
    let rita = assignment(0);
    let req = bid_request();
    let jev = Arc::new(ScriptedJev::new(vec![Err(JevError::Decision(
        JevDecisionError::new(JevDecisionReason::ResponseSchema),
    ))]));
    let (factory, _talk) = factory(jev.clone());
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let out = decide(
        &deps(&rita, Some(&factory), &log, &clock, "n"),
        &req,
        &req.legal,
        DecideOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(out.source, AiDecisionSource::Fallback);
    assert_eq!(out.attempts, 1);
    assert_eq!(log.events(), ["ai_error"]);
    assert_eq!(
        log.find("ai_error").unwrap()["name"],
        json!("JevDecisionError")
    );
    assert_eq!(jev.calls().len(), 1);
}

#[tokio::test]
async fn decide_skips_every_attempt_when_the_deadline_leaves_no_budget() {
    let rita = assignment(0);
    let mut req = bid_request();
    // 6500 nominal, 600 reserve, 400 floor: 999 ms left is 399 after the reserve.
    req.deadline_at = NOW + 999;
    let jev = Arc::new(ScriptedJev::new(vec![Ok(jev_choice("move_0", 1.0))]));
    let (factory, _talk) = factory(jev.clone());
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let out = decide(
        &deps(&rita, Some(&factory), &log, &clock, "n"),
        &req,
        &req.legal,
        DecideOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(out.attempts, 0);
    assert_eq!(out.source, AiDecisionSource::Fallback);
    assert!(jev.calls().is_empty());
    // At exactly the floor, the attempt runs.
    req.deadline_at = NOW + 1000;
    let out = decide(
        &deps(&rita, Some(&factory), &log, &clock, "n"),
        &req,
        &req.legal,
        DecideOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(out.attempts, 1);
}

#[tokio::test]
async fn decide_rejects_an_empty_legal_set() {
    let rita = assignment(0);
    let mut req = bid_request();
    req.legal.clear();
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let err = decide(
        &deps(&rita, None, &log, &clock, "n"),
        &req,
        &[],
        DecideOptions::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CallModelError::NoLegalMoves));
}

#[tokio::test]
async fn call_model_times_out_and_cancels_the_jev_call() {
    let rita = assignment(0);
    let req = bid_request();
    let jev = Arc::new(
        ScriptedJev::new(vec![Ok(jev_choice("move_0", 1.0))]).with_delay(Duration::from_secs(30)),
    );
    let (factory, _talk) = factory(jev.clone());
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let err = call_model(
        &deps(&rita, Some(&factory), &log, &clock, "n"),
        CallModelRequest::from_request(&req),
        &req.legal,
        CallModelOptions {
            escalate: false,
            budget_ms: 80.0,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CallModelError::Jev(JevError::Timeout)));
    assert!(err.is_timeout_or_abort());
    // An expired deadline never calls.
    let mut late = req.clone();
    late.deadline_at = NOW;
    let err = call_model(
        &deps(&rita, Some(&factory), &log, &clock, "n"),
        CallModelRequest::from_request(&late),
        &late.legal,
        BUDGET,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CallModelError::DeadlineExpired));
    // Without a decision client.
    let err = call_model(
        &deps(&rita, None, &log, &clock, "n"),
        CallModelRequest::from_request(&req),
        &req.legal,
        BUDGET,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CallModelError::NotConfigured));
}
