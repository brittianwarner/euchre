//! The aiSeat queue handlers (`actors/ai-seat/index.ts`) as pure state
//! transitions: envelope checks, the decide path through the ladder, the banter
//! gate and job, lifecycle messages, and the profile flushes.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use euchre_ai::ai_seat::bridge::{LlmProvider, ProviderDeps};
use euchre_ai::ai_seat::handlers::{
    DEV_PROFILE_TOKEN, DecideDrop, DecideHandled, EpisodeFlush, SeatEnv, begin_banter,
    complete_episode_flush, internal_token_ok, messages, on_decide, on_game_end, on_hand_end,
    on_reset_hand, prepare_episode_flush, prepare_token_flush, profile_internal_token,
    run_banter_job,
};
use euchre_ai::ai_seat::state::{
    AiSeatCreateInput, AiSeatEpisode, AiSeatState, CreateStateParams, HandSnapshot, create_state,
};
use euchre_ai::testing::{
    ManualClock, RecordingLog, ScriptedBanter, ScriptedDecider, ScriptedTalk, jev_factory,
    tiered_factory,
};
use euchre_ai::types::NoopLog;
use euchre_engine::{AiDecisionSource, PublicGameView};
use euchre_protocol::{
    AIDecideRequest, AIDecisionKind, AiSeatLifecycleKind, AiSeatLifecycleMessage, EpisodeKind,
    default_personas,
};
use serde_json::json;

const TOKEN: &str = "table-token-0123456789";
const NOW: u64 = 1_700_000_000_000;

fn state_for(view: &PublicGameView) -> AiSeatState {
    let seat = view.you;
    let input = AiSeatCreateInput {
        game_id: "g1".to_owned(),
        seat,
        internal_token: TOKEN.to_owned(),
        fence_nonce: "nonce".to_owned(),
        persona: default_personas().remove(0).persona,
        dossier: None,
        profile_key: Some(vec!["profile".to_owned(), "u1".to_owned()]),
        match_id: Some("m1".to_owned()),
    };
    let key = vec![
        "table".to_owned(),
        "g1".to_owned(),
        "seat".to_owned(),
        seat.get().to_string(),
    ];
    create_state(CreateStateParams {
        key: &key,
        input: Some(&input),
    })
    .unwrap()
}

fn decide_body(view: &PublicGameView, turn: &str) -> AIDecideRequest {
    AIDecideRequest {
        internal_token: TOKEN.to_owned(),
        game_id: "g1".to_owned(),
        seat: view.you,
        turn_id: turn.to_owned(),
        kind: AIDecisionKind::Play,
        view: view.clone(),
        legal: view.legal.clone(),
        ranking: None,
        deadline_at: NOW + 10_000,
    }
}

fn play() -> PublicGameView {
    partnership_cases().remove(1).view
}

fn provider(decider: Option<ScriptedDecider>, banter: Option<ScriptedBanter>) -> LlmProvider {
    LlmProvider {
        decider: decider.map(|d| Arc::new(d) as _),
        banter: banter.map(|b| Arc::new(b) as _),
    }
}

fn lifecycle(
    kind: AiSeatLifecycleKind,
    token: &str,
    hand_no: Option<u32>,
) -> AiSeatLifecycleMessage {
    AiSeatLifecycleMessage {
        internal_token: token.to_owned(),
        game_id: "g1".to_owned(),
        kind,
        hand_no,
    }
}

/* -------------------------------------------------------------------------- */
/* Tokens                                                                      */
/* -------------------------------------------------------------------------- */

#[test]
fn internal_tokens_compare_exactly_and_an_empty_secret_never_matches() {
    assert!(internal_token_ok("abc", "abc"));
    assert!(!internal_token_ok("abc", "abd"));
    assert!(!internal_token_ok("abc", "abcd"));
    assert!(!internal_token_ok("", ""));
}

#[test]
fn the_profile_token_prefers_a_long_secret_then_falls_back_only_in_local_dev() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    };
    let secret = "0123456789abcdef";
    assert_eq!(
        profile_internal_token(&env(&[("EUCHRE_INTERNAL_TOKEN", "0123456789abcdef")])),
        Some(secret.to_owned())
    );
    // 15 units is too short; no RIVET_ENDPOINT means local dev.
    assert_eq!(
        profile_internal_token(&env(&[("EUCHRE_INTERNAL_TOKEN", "0123456789abcde")])),
        Some(DEV_PROFILE_TOKEN.to_owned())
    );
    assert_eq!(
        profile_internal_token(&env(&[("RIVET_ENDPOINT", "")])),
        Some(DEV_PROFILE_TOKEN.to_owned())
    );
    assert_eq!(
        profile_internal_token(&env(&[("RIVET_ENDPOINT", "https://rivet.example")])),
        None
    );
}

/* -------------------------------------------------------------------------- */
/* decide                                                                      */
/* -------------------------------------------------------------------------- */

#[tokio::test]
async fn a_forged_token_is_dropped_and_counted() {
    let view = play();
    let mut state = state_for(&view);
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let p = provider(Some(ScriptedDecider::always("play:JH")), None);
    let mut body = decide_body(&view, "t1");
    body.internal_token = "forged".to_owned();
    let out = on_decide(
        &mut state,
        &body,
        SeatEnv {
            clock: &clock,
            log: &log,
            provider: &p,
        },
    )
    .await;
    assert_eq!(out, DecideHandled::Dropped(DecideDrop::ForgedToken));
    assert!(out.state_changed());
    assert_eq!(state.meter.token_rejections, 1);
    let line = &log.lines()[0];
    assert_eq!(
        (line.level, line.event.as_str()),
        ("warn", messages::FORGED_DECIDE)
    );
}

#[tokio::test]
async fn another_match_is_dropped_silently_and_another_seat_is_counted() {
    let view = play();
    let mut state = state_for(&view);
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let p = provider(Some(ScriptedDecider::always("play:JH")), None);
    let env = SeatEnv {
        clock: &clock,
        log: &log,
        provider: &p,
    };

    let mut body = decide_body(&view, "t1");
    body.game_id = "other".to_owned();
    let out = on_decide(&mut state, &body, env).await;
    assert_eq!(out, DecideHandled::Dropped(DecideDrop::WrongGame));
    assert!(!out.state_changed());
    assert!(log.lines().is_empty());

    let mut body = decide_body(&view, "t1");
    body.seat = seat((view.you.get() + 1) % 4);
    let out = on_decide(&mut state, &body, env).await;
    assert_eq!(out, DecideHandled::Dropped(DecideDrop::WrongSeat));
    assert_eq!(state.meter.seat_rejections, 1);
    assert_eq!(log.lines()[0].event, messages::WRONG_SEAT);
}

#[tokio::test]
async fn an_answer_records_the_turn_and_a_redelivery_is_stale() {
    let view = play();
    let mut state = state_for(&view);
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let p = provider(Some(ScriptedDecider::always("play:JH")), None);
    let env = SeatEnv {
        clock: &clock,
        log: &log,
        provider: &p,
    };
    let body = decide_body(&view, "t1");
    let DecideHandled::Answered(answered) = on_decide(&mut state, &body, env).await else {
        panic!("expected an answer");
    };
    let d = &answered.decision;
    assert_eq!(d.move_id, "play:JH");
    assert_eq!(d.source, AiDecisionSource::Llm);
    assert_eq!(d.internal_token, TOKEN);
    assert_eq!(d.game_id, "g1");
    assert_eq!(d.turn_id, "t1");
    assert_eq!(d.seat, state.seat);
    assert!(d.banter.is_none());
    assert_eq!(state.last_turn_id.as_deref(), Some("t1"));
    assert_eq!(state.decision_log.len(), 1);
    assert_eq!(state.meter.decisions, 1);

    let again = on_decide(&mut state, &body, env).await;
    assert_eq!(again, DecideHandled::Dropped(DecideDrop::StaleTurn));
    assert_eq!(state.meter.stale_turns, 1);
}

#[tokio::test]
async fn an_empty_legal_set_logs_an_error() {
    let view = play();
    let mut state = state_for(&view);
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let p = LlmProvider::none();
    let mut body = decide_body(&view, "t1");
    body.legal.clear();
    let out = on_decide(
        &mut state,
        &body,
        SeatEnv {
            clock: &clock,
            log: &log,
            provider: &p,
        },
    )
    .await;
    assert_eq!(out, DecideHandled::Dropped(DecideDrop::EmptyLegal));
    let line = log.lines().pop().unwrap();
    assert_eq!(
        (line.level, line.event.as_str()),
        ("error", messages::EMPTY_LEGAL)
    );
    assert_eq!(line.fields["turnId"], json!("t1"));
    assert!(state.last_turn_id.is_none());
}

#[tokio::test]
async fn without_a_provider_the_heuristic_answers() {
    let view = play();
    let mut state = state_for(&view);
    let (log, clock) = (RecordingLog::new(), ManualClock::new(NOW));
    let p = LlmProvider::none();
    let body = decide_body(&view, "t1");
    let out = on_decide(
        &mut state,
        &body,
        SeatEnv {
            clock: &clock,
            log: &log,
            provider: &p,
        },
    )
    .await;
    let DecideHandled::Answered(a) = out else {
        panic!("expected an answer")
    };
    assert_eq!(a.decision.source, AiDecisionSource::Fallback);
    assert!(view.legal.iter().any(|m| m.id == a.decision.move_id));
    assert!(log.events().contains(&"ai_no_provider".to_owned()));
}

#[tokio::test]
async fn the_decide_future_is_send() {
    let view = play();
    let handle = tokio::spawn(async move {
        let mut state = state_for(&view);
        let log = RecordingLog::new();
        let clock = ManualClock::new(NOW);
        let p = provider(Some(ScriptedDecider::always("play:JH")), None);
        let body = decide_body(&view, "t1");
        on_decide(
            &mut state,
            &body,
            SeatEnv {
                clock: &clock,
                log: &log,
                provider: &p,
            },
        )
        .await
    });
    assert!(matches!(handle.await.unwrap(), DecideHandled::Answered(_)));
}

/* -------------------------------------------------------------------------- */
/* banter                                                                      */
/* -------------------------------------------------------------------------- */

#[tokio::test]
async fn the_banter_gate_opens_once_per_hand_and_the_job_says_the_line() {
    let view = play();
    let mut state = state_for(&view);
    let p = provider(None, Some(ScriptedBanter::new(Some("Nice one."))));
    let body = decide_body(&view, "t9");
    let job = begin_banter(&mut state, &body, &p).unwrap();
    assert_eq!(state.banter.last_hand_no, i64::from(view.hand_no));
    assert_eq!(job.msg_id, format!("t9:talk:{}", state.seat.get()));
    assert_eq!(job.input.budget_ms, 1800);
    assert!(job.input.view.hand.is_empty());
    assert!(job.input.dossier.is_empty());
    assert!(begin_banter(&mut state, &body, &p).is_none());

    let say = run_banter_job(&p, &job).await.unwrap();
    assert_eq!(say.text.as_deref(), Some("Nice one."));
    assert!(say.is_final);
    assert_eq!(say.turn_id, "t9");
    assert_eq!(say.hand_no, Some(view.hand_no));
    assert_eq!(say.internal_token, TOKEN);
}

#[tokio::test]
async fn the_banter_gate_stays_shut_without_a_writer_when_degraded_silent_or_before_trump() {
    let view = play();
    let body = decide_body(&view, "t1");
    let writer = || provider(None, Some(ScriptedBanter::new(Some("Hm."))));

    let mut state = state_for(&view);
    assert!(begin_banter(&mut state, &body, &LlmProvider::none()).is_none());

    let mut state = state_for(&view);
    state.budget.degraded = true;
    assert!(begin_banter(&mut state, &body, &writer()).is_none());

    let mut state = state_for(&view);
    state.persona.chattiness = 0.0;
    assert!(begin_banter(&mut state, &body, &writer()).is_none());

    let bid = bidding_cases().remove(0).view;
    let mut state = state_for(&bid);
    assert!(begin_banter(&mut state, &decide_body(&bid, "t1"), &writer()).is_none());
    assert_eq!(state.banter.last_hand_no, -1);
}

#[tokio::test(start_paused = true)]
async fn a_silent_or_late_writer_says_nothing() {
    let view = play();
    let mut state = state_for(&view);
    let silent = provider(None, Some(ScriptedBanter::new(Some(""))));
    let job = begin_banter(&mut state, &decide_body(&view, "t1"), &silent).unwrap();
    assert!(run_banter_job(&silent, &job).await.is_none());

    let mut late = ScriptedBanter::new(Some("Too late."));
    late.delay = Some(Duration::from_millis(1801));
    let late = provider(None, Some(late));
    let start = tokio::time::Instant::now();
    assert!(run_banter_job(&late, &job).await.is_none());
    assert_eq!(start.elapsed().as_millis(), 1800);

    let mut on_time = ScriptedBanter::new(Some("Just in time."));
    on_time.delay = Some(Duration::from_millis(1799));
    let on_time = provider(None, Some(on_time));
    assert!(run_banter_job(&on_time, &job).await.is_some());

    assert!(run_banter_job(&LlmProvider::none(), &job).await.is_none());
}

#[test]
fn a_provider_from_a_factory_decides_only_with_a_decision_client() {
    let deps = || ProviderDeps {
        clock: Arc::new(ManualClock::new(NOW)),
        log: Arc::new(NoopLog),
    };
    let talk = Arc::new(ScriptedTalk::new(Vec::new()));
    let text_only = LlmProvider::from_factory(
        Some(Arc::new(tiered_factory(talk.clone(), talk.clone()))),
        deps(),
    );
    assert!(text_only.decider.is_none());
    assert!(text_only.banter.is_some());
    let jev = Arc::new(euchre_ai::testing::HangingJev::new());
    let full = LlmProvider::from_factory(Some(Arc::new(jev_factory(jev, talk))), deps());
    assert!(full.decider.is_some());
    assert!(full.banter.is_some());
    let none = LlmProvider::from_factory(None, deps());
    assert!(none.decider.is_none() && none.banter.is_none());
}

/* -------------------------------------------------------------------------- */
/* Lifecycle and flushes                                                       */
/* -------------------------------------------------------------------------- */

fn episode(hand_no: u32) -> AiSeatEpisode {
    AiSeatEpisode {
        kind: EpisodeKind::Euchre,
        summary: format!("hand {hand_no}"),
        salience: 0.5,
        hand_no,
        ts: NOW,
    }
}

#[test]
fn reset_hand_checks_the_envelope_and_advances_the_hand() {
    let view = play();
    let mut state = state_for(&view);
    state.memory.hand_no = 4;
    assert!(!on_reset_hand(
        &mut state,
        &lifecycle(AiSeatLifecycleKind::ResetHand, "bad", None)
    ));
    assert_eq!(state.memory.hand_no, 4);
    assert!(on_reset_hand(
        &mut state,
        &lifecycle(AiSeatLifecycleKind::ResetHand, TOKEN, None)
    ));
    assert_eq!(state.memory.hand_no, 5);
    assert!(on_reset_hand(
        &mut state,
        &lifecycle(AiSeatLifecycleKind::ResetHand, TOKEN, Some(9))
    ));
    assert_eq!(state.memory.hand_no, 9);
    let mut other = lifecycle(AiSeatLifecycleKind::ResetHand, TOKEN, Some(1));
    other.game_id = "other".to_owned();
    assert!(!on_reset_hand(&mut state, &other));
}

#[test]
fn episode_flushes_send_defer_drop_or_do_nothing() {
    let view = play();
    let mut state = state_for(&view);
    assert_eq!(
        prepare_episode_flush(&mut state, Some("p")),
        EpisodeFlush::Nothing
    );

    state.episode_buffer.push(episode(1));
    assert_eq!(
        prepare_episode_flush(&mut state, None),
        EpisodeFlush::Deferred
    );
    assert_eq!(state.episode_buffer.len(), 1);

    let EpisodeFlush::Send { profile_key, body } = prepare_episode_flush(&mut state, Some("p"))
    else {
        panic!("expected a send");
    };
    assert_eq!(profile_key, ["profile", "u1"]);
    assert_eq!(body.internal_token, "p");
    assert_eq!(
        serde_json::to_value(&body).unwrap(),
        json!({
            "internalToken": "p",
            "episodes": [{
                "role": state.persona.id,
                "matchId": "m1",
                "handNo": 1,
                "ts": NOW,
                "kind": "euchre",
                "summary": "hand 1",
                "salience": 0.5
            }]
        })
    );

    let log = RecordingLog::new();
    let long = "e".repeat(300);
    complete_episode_flush(&mut state, Err(&long), &log);
    assert_eq!(state.episode_buffer.len(), 1);
    let line = &log.lines()[0];
    assert_eq!(
        (line.level, line.event.as_str()),
        ("warn", messages::FLUSH_EPISODES_FAILED)
    );
    assert_eq!(line.fields["err"].as_str().map(str::len), Some(200));
    complete_episode_flush(&mut state, Ok(()), &log);
    assert!(state.episode_buffer.is_empty());

    state.episode_buffer.push(episode(2));
    state.profile_key = None;
    assert_eq!(
        prepare_episode_flush(&mut state, Some("p")),
        EpisodeFlush::Dropped
    );
    assert!(state.episode_buffer.is_empty());
}

#[test]
fn token_flushes_need_a_key_some_spend_and_a_secret() {
    let view = play();
    let mut state = state_for(&view);
    assert!(prepare_token_flush(&state, Some("p")).is_none());
    state.budget.tokens_in = 10;
    state.budget.tokens_out = 2;
    state.budget.cache_read_tokens = 4;
    state.budget.calls = 1;
    assert!(prepare_token_flush(&state, None).is_none());
    let flush = prepare_token_flush(&state, Some("p")).unwrap();
    assert_eq!(
        serde_json::to_value(&flush.body).unwrap(),
        json!({ "internalToken": "p", "tokensIn": 10, "tokensOut": 2, "cacheReadTokens": 4, "calls": 1 })
    );
    state.profile_key = None;
    assert!(prepare_token_flush(&state, Some("p")).is_none());
}

#[test]
fn hand_end_and_game_end_check_the_envelope_then_flush_in_order() {
    let view = play();
    let mut state = state_for(&view);
    assert!(
        on_hand_end(
            &mut state,
            &lifecycle(AiSeatLifecycleKind::HandEnd, "bad", Some(0)),
            NOW,
            Some("p")
        )
        .is_none()
    );
    state.episode_buffer.push(episode(0));
    let flush = on_hand_end(
        &mut state,
        &lifecycle(AiSeatLifecycleKind::HandEnd, TOKEN, Some(0)),
        NOW,
        Some("p"),
    )
    .unwrap();
    assert!(matches!(flush, EpisodeFlush::Send { .. }));

    // The buffer is capped at 12, oldest first out: a 13th episode pushes
    // hand 0 out and lands last.
    let mut state = state_for(&view);
    for n in 0..12 {
        state.episode_buffer.push(episode(n));
    }
    state.memory.snapshot = Some(HandSnapshot {
        hand_no: 12,
        trump: None,
        maker_seat: Some(seat(0)),
        alone_seat: None,
        tricks_won: [3, 2],
        result: None,
    });
    on_hand_end(
        &mut state,
        &lifecycle(AiSeatLifecycleKind::HandEnd, TOKEN, Some(12)),
        NOW,
        None,
    );
    assert_eq!(state.episode_buffer.len(), 12);
    assert_eq!(state.episode_buffer[0].hand_no, 1);
    let last = state.episode_buffer.last().unwrap();
    assert_eq!((last.hand_no, last.ts), (12, NOW));
    assert!(!state.episode_buffer.iter().any(|e| e.hand_no == 0));

    let mut state = state_for(&view);
    assert!(
        on_game_end(
            &mut state,
            &lifecycle(AiSeatLifecycleKind::GameEnd, "bad", None),
            Some("p")
        )
        .is_none()
    );
    state.episode_buffer.push(episode(3));
    state.budget.calls = 2;
    let end = on_game_end(
        &mut state,
        &lifecycle(AiSeatLifecycleKind::GameEnd, TOKEN, None),
        Some("p"),
    )
    .unwrap();
    assert!(matches!(end.episodes, EpisodeFlush::Send { .. }));
    assert_eq!(end.tokens.unwrap().body.calls, 2);
}
