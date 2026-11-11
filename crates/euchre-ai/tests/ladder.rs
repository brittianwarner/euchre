//! The aiSeat ladder (`actors/ai-seat/decide.ts` `runLadder`) on paused tokio
//! time: every rung and the deadline edges of each attempt.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use euchre_ai::ai_seat::bridge::{JevDecider, LlmDecideResult, call_decider};
use euchre_ai::ai_seat::ladder::{LadderDeps, LadderOutcome, run_ladder};
use euchre_ai::clock::TokioClock;
use euchre_ai::decide::CallModelError;
use euchre_ai::jev::{JevDecisionError, JevDecisionReason, JevDiagnostic, JevError, JevViolation};
use euchre_ai::testing::{
    ManualClock, RecordingLog, ScriptedDecider, ScriptedJev, ScriptedTalk, decide_result,
    jev_choice, jev_factory,
};
use euchre_ai::types::LlmUsage;
use euchre_engine::{AiDecisionSource, PublicGameView};
use euchre_protocol::{AIDecideRequest, AIDecisionKind, PersonaAssignment, default_personas};
use serde_json::json;

const NOW: u64 = 1_700_000_000_000;

fn rita() -> PersonaAssignment {
    default_personas().remove(0)
}

fn request(view: &PublicGameView, kind: AIDecisionKind, deadline_in: u64) -> AIDecideRequest {
    AIDecideRequest {
        internal_token: String::new(),
        game_id: "g".to_owned(),
        seat: view.you,
        turn_id: "t1".to_owned(),
        kind,
        view: view.clone(),
        legal: view.legal.clone(),
        ranking: None,
        deadline_at: NOW + deadline_in,
    }
}

fn bid_request(deadline_in: u64) -> AIDecideRequest {
    request(
        &bidding_cases().remove(0).view,
        AIDecisionKind::Bid1,
        deadline_in,
    )
}

fn play_request(deadline_in: u64) -> AIDecideRequest {
    request(
        &partnership_cases().remove(1).view,
        AIDecisionKind::Play,
        deadline_in,
    )
}

async fn ladder(
    req: &AIDecideRequest,
    decider: Option<&ScriptedDecider>,
    clock: &TokioClock,
    log: &RecordingLog,
    degraded: bool,
) -> LadderOutcome {
    let a = rita();
    run_ladder(
        LadderDeps {
            persona: &a.persona,
            dossier: &a.dossier,
            nonce: "ladder-test",
            degraded,
            clock,
            log,
            decider: decider.map(|d| d as _),
        },
        req,
    )
    .await
    .unwrap()
}

/// A decider that never answers inside any budget.
fn hanging() -> ScriptedDecider {
    ScriptedDecider::always("pass").with_delay(Duration::from_secs(3600))
}

fn elapsed_ms(start: tokio::time::Instant) -> u128 {
    start.elapsed().as_millis()
}

#[tokio::test(start_paused = true)]
async fn forces_a_single_legal_move_without_a_call() {
    let mut req = bid_request(4000);
    req.legal.truncate(1);
    let decider = ScriptedDecider::always("orderUp");
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    assert_eq!(out.source, AiDecisionSource::Forced);
    assert_eq!(out.move_id, req.legal[0].id);
    assert_eq!((out.candidates, out.llm_calls), (1, 0));
    assert!(decider.calls().is_empty());
    assert!(log.events().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_degraded_seat_uses_the_heuristic_silently() {
    let req = bid_request(4000);
    let decider = ScriptedDecider::always("orderUp");
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let out = ladder(&req, Some(&decider), &clock, &log, true).await;
    assert_eq!(out.source, AiDecisionSource::Fallback);
    assert_eq!(out.candidates, req.legal.len());
    assert_eq!(out.llm_calls, 0);
    assert!(decider.calls().is_empty());
    assert!(log.events().is_empty());
}

#[tokio::test(start_paused = true)]
async fn without_a_provider_logs_ai_no_provider_and_uses_the_heuristic() {
    let req = bid_request(4000);
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let out = ladder(&req, None, &clock, &log, false).await;
    assert_eq!(out.source, AiDecisionSource::Fallback);
    assert!(req.legal.iter().any(|m| m.id == out.move_id));
    assert_eq!(log.events(), ["ai_no_provider"]);
    assert_eq!(
        log.find("ai_no_provider").unwrap(),
        json!({ "seat": req.seat.get(), "kind": "bid1" })
            .as_object()
            .cloned()
            .unwrap()
    );
}

#[tokio::test(start_paused = true)]
async fn the_first_legal_answer_wins_with_its_usage_and_budget() {
    let req = bid_request(10_000);
    let decider = ScriptedDecider::new(|_, _| {
        Ok(LlmDecideResult {
            usage: Some(LlmUsage {
                input_tokens: 7,
                output_tokens: 1,
                cache_read_tokens: 2,
            }),
            confidence: Some(f64::NAN),
            ..decide_result("orderUp")
        })
    });
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    assert_eq!(out.move_id, "orderUp");
    assert_eq!(out.source, AiDecisionSource::Llm);
    assert_eq!((out.llm_calls, out.illegal_attempts), (1, 0));
    assert_eq!(out.usage.input_tokens, 7);
    let calls = decider.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].budget_ms, 2200.0);
    assert!(!calls[0].escalate);
    assert_eq!(calls[0].candidates, req.legal);
    assert!(log.events().is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_out_of_set_id_fails_the_attempt_and_escalates_off_the_play_path() {
    let req = bid_request(10_000);
    let decider =
        ScriptedDecider::new(|_, i| Ok(decide_result(if i == 0 { "play:AS" } else { "pass" })));
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    assert_eq!(out.move_id, "pass");
    assert_eq!(out.source, AiDecisionSource::Llm);
    assert_eq!((out.llm_calls, out.illegal_attempts), (2, 1));
    // An id outside the candidates resolves None without a jev_failure line.
    assert_eq!(log.events(), ["ai_attempt_failed"]);
    let failed = log.find("ai_attempt_failed").unwrap();
    assert_eq!(failed["escalate"], json!(false));
    assert_eq!(failed["kind"], json!("bid1"));
    let calls = decider.calls();
    assert!(calls[1].escalate);
    assert_eq!(calls[1].budget_ms, 1400.0);
}

#[tokio::test(start_paused = true)]
async fn a_play_gets_one_attempt_only() {
    let req = play_request(10_000);
    let decider = ScriptedDecider::always("play:XX");
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    assert_eq!(out.source, AiDecisionSource::Fallback);
    assert_eq!((out.llm_calls, out.illegal_attempts), (1, 1));
    assert_eq!(out.candidates, 5);
    assert_eq!(decider.calls().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn decider_errors_log_jev_failure_with_the_classified_reason() {
    let req = bid_request(10_000);
    let decider = ScriptedDecider::new(|_, i| {
        Err(if i == 0 {
            CallModelError::Jev(JevError::Decision(JevDecisionError {
                reason: JevDecisionReason::ChoiceDistribution,
                status: None,
                diagnostic: Some(JevDiagnostic {
                    request_id: Some("r1".to_owned()),
                    question_id: "move".to_owned(),
                    violation: JevViolation::ProbabilityTotal,
                }),
            }))
        } else {
            CallModelError::Jev(JevError::Transport("connect".to_owned()))
        })
    });
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    assert_eq!(out.source, AiDecisionSource::Fallback);
    assert_eq!((out.llm_calls, out.illegal_attempts), (2, 2));
    assert_eq!(
        log.events(),
        [
            "jev_failure",
            "ai_attempt_failed",
            "jev_failure",
            "ai_attempt_failed"
        ]
    );
    let lines = log.lines();
    let first = &lines[0].fields;
    assert_eq!(first["reason"], json!("choice_distribution"));
    assert_eq!(first["requestId"], json!("r1"));
    assert_eq!(first["questionId"], json!("move"));
    assert_eq!(first["violation"], json!("probability_total"));
    assert_eq!(first["handNo"], json!(req.view.hand_no));
    assert_eq!(lines[2].fields["reason"], json!("transport_or_adapter"));
    assert!(!lines[2].fields.contains_key("status"));
}

#[tokio::test(start_paused = true)]
async fn an_http_failure_carries_its_status() {
    let req = play_request(10_000);
    let decider = ScriptedDecider::new(|_, _| {
        Err(CallModelError::Jev(JevError::Decision(JevDecisionError {
            reason: JevDecisionReason::HttpStatus,
            status: Some(503),
            diagnostic: None,
        })))
    });
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    ladder(&req, Some(&decider), &clock, &log, false).await;
    let f = log.find("jev_failure").unwrap();
    assert_eq!(f["reason"], json!("http_status"));
    assert_eq!(f["status"], json!(503));
}

#[tokio::test(start_paused = true)]
async fn two_timed_out_attempts_fall_back_at_the_summed_budgets() {
    // deadline now+4000: 2200 (clamp 3700), then 1400 (clamp 1500): fallback at t=3600.
    let req = bid_request(4000);
    let decider = hanging();
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let start = tokio::time::Instant::now();
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    assert_eq!(elapsed_ms(start), 3600);
    assert_eq!(out.source, AiDecisionSource::Fallback);
    assert_eq!((out.llm_calls, out.illegal_attempts), (2, 2));
    let budgets: Vec<f64> = decider.calls().iter().map(|c| c.budget_ms).collect();
    assert_eq!(budgets, [2200.0, 1400.0]);
    assert_eq!(
        log.events(),
        [
            "jev_failure",
            "ai_attempt_failed",
            "jev_failure",
            "ai_attempt_failed"
        ]
    );
    assert_eq!(log.find("jev_failure").unwrap()["reason"], json!("timeout"));
    assert!(req.legal.iter().any(|m| m.id == out.move_id));
}

#[tokio::test(start_paused = true)]
async fn a_second_attempt_at_exactly_the_minimum_budget_runs() {
    // now+2900: 2200 then 2900 - 2200 - 300 = 400, which is the floor.
    let req = bid_request(2900);
    let decider = hanging();
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let start = tokio::time::Instant::now();
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    assert_eq!(elapsed_ms(start), 2600);
    assert_eq!(out.llm_calls, 2);
    let budgets: Vec<f64> = decider.calls().iter().map(|c| c.budget_ms).collect();
    assert_eq!(budgets, [2200.0, 400.0]);
    assert!(!log.events().contains(&"ai_no_budget".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn a_second_attempt_below_the_minimum_budget_is_skipped() {
    // now+2899: the second budget is 399.
    let req = bid_request(2899);
    let decider = hanging();
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let start = tokio::time::Instant::now();
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    assert_eq!(elapsed_ms(start), 2200);
    assert_eq!(out.llm_calls, 1);
    assert_eq!(out.source, AiDecisionSource::Fallback);
    assert_eq!(
        log.events(),
        ["jev_failure", "ai_attempt_failed", "ai_no_budget"]
    );
    assert_eq!(
        log.find("ai_no_budget").unwrap(),
        json!({ "seat": req.seat.get(), "escalate": true, "budgetMs": 399 })
            .as_object()
            .cloned()
            .unwrap()
    );
}

#[tokio::test(start_paused = true)]
async fn a_first_attempt_is_clamped_to_the_deadline_and_skipped_below_the_floor() {
    // now+1000: min(2200, 700) = 700.
    let req = bid_request(1000);
    let decider = hanging();
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let start = tokio::time::Instant::now();
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    assert_eq!(elapsed_ms(start), 700);
    assert_eq!(decider.calls()[0].budget_ms, 700.0);
    assert_eq!(out.llm_calls, 1);
    assert_eq!(
        log.events().last().map(String::as_str),
        Some("ai_no_budget")
    );

    // now+699: 399 on the first attempt; nothing is called.
    let req = bid_request(699);
    let decider = hanging();
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    assert_eq!(out.llm_calls, 0);
    assert!(decider.calls().is_empty());
    assert_eq!(log.events(), ["ai_no_budget"]);
    assert_eq!(log.find("ai_no_budget").unwrap()["escalate"], json!(false));

    // A past deadline: a negative budget, still skipped.
    let mut req = bid_request(0);
    req.deadline_at = NOW - 5000;
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    assert_eq!(out.llm_calls, 0);
    assert_eq!(log.find("ai_no_budget").unwrap()["budgetMs"], json!(-5300));
}

#[tokio::test(start_paused = true)]
async fn a_late_answer_inside_the_budget_still_counts() {
    let req = bid_request(10_000);
    let decider = ScriptedDecider::always("orderUp").with_delay(Duration::from_millis(2199));
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    assert_eq!(out.source, AiDecisionSource::Llm);
    assert_eq!(out.llm_calls, 1);
}

#[tokio::test(start_paused = true)]
async fn narrowed_kinds_offer_a_subset_of_the_legal_set() {
    let mut req = play_request(10_000);
    req.kind = AIDecisionKind::Discard;
    let decider = ScriptedDecider::new(|input, _| Ok(decide_result(&input.candidates[0].id)));
    let (clock, log) = (TokioClock::starting_at(NOW), RecordingLog::new());
    let out = ladder(&req, Some(&decider), &clock, &log, false).await;
    let offered = &decider.calls()[0].candidates;
    assert!(!offered.is_empty());
    assert!(offered.iter().all(|m| req.legal.contains(m)));
    assert_eq!(out.candidates, offered.len());
    assert_eq!(out.source, AiDecisionSource::Llm);
    // Deterministic per turn and seat.
    let again = ScriptedDecider::new(|input, _| Ok(decide_result(&input.candidates[0].id)));
    ladder(&req, Some(&again), &clock, &log, false).await;
    assert_eq!(&again.calls()[0].candidates, offered);
}

#[tokio::test(start_paused = true)]
async fn call_decider_rejects_a_non_positive_budget_without_calling() {
    let req = bid_request(10_000);
    let decider = ScriptedDecider::always("pass");
    let log = RecordingLog::new();
    let a = rita();
    let input = euchre_ai::ai_seat::bridge::LlmDecideInput {
        kind: req.kind,
        seat: req.seat,
        view: req.view.clone(),
        candidates: req.legal.clone(),
        ranking: Vec::new(),
        persona: a.persona.clone(),
        dossier: a.dossier.clone(),
        nonce: "n".to_owned(),
        budget_ms: 0.0,
        escalate: false,
        deadline_at: req.deadline_at,
    };
    assert!(call_decider(&decider, &input, &log).await.is_none());
    // Deliberate deviation (ai-llm.md): TS evaluates `decider(input)` before
    // `withDeadline` rejects, so it starts and abandons a model call. Rust
    // futures are lazy and the port skips the call; the result and the log match.
    assert!(decider.calls().is_empty());
    assert_eq!(
        log.find("jev_failure").unwrap()["reason"],
        json!("transport_or_adapter")
    );
}

/// The real bridge end to end: ladder → JevDecider → callModel → scripted Jev.
#[tokio::test]
async fn the_jev_decider_answers_through_call_model() {
    let req = bid_request(10_000);
    let jev = Arc::new(ScriptedJev::new(vec![Ok(jev_choice("move_0", 0.9))]));
    let factory = Arc::new(jev_factory(
        jev.clone(),
        Arc::new(ScriptedTalk::new(Vec::new())),
    ));
    let clock = Arc::new(ManualClock::new(NOW));
    let decider = JevDecider::new(factory, clock.clone());
    let a = rita();
    let log = RecordingLog::new();
    let out = run_ladder(
        LadderDeps {
            persona: &a.persona,
            dossier: &a.dossier,
            nonce: "n",
            degraded: false,
            clock: clock.as_ref(),
            log: &log,
            decider: Some(&decider),
        },
        &req,
    )
    .await
    .unwrap();
    assert_eq!(out.move_id, "pass");
    assert_eq!(out.source, AiDecisionSource::Llm);
    assert_eq!(out.usage.input_tokens, 10);
    assert_eq!(jev.calls().len(), 1);
    assert!(log.events().contains(&"jev_decision".to_owned()));
}
