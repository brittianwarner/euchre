//! `generateBanter` (`ai/banter.ts`): the salience gate, the provider gate, the
//! streaming screen on the settled prefix, the final screen, provider errors,
//! and the banter deadline (on paused tokio time).

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use euchre_ai::banter::{
    BANTER_SYSTEM, BanterOutcome, BanterRequest, BanterSource, generate_banter,
};
use euchre_ai::decide::DecideDeps;
use euchre_ai::model::ModelFactory;
use euchre_ai::talk::{TalkClient, TalkError};
use euchre_ai::testing::{
    HangingJev, HangingTalk, ManualClock, RecordingLog, ScriptedResult, ScriptedTalk,
    tiered_factory,
};
use euchre_ai::types::{DecideUsage, DecisionBudgetOverrides, ZERO_USAGE};
use euchre_engine::{PublicGameView, project};
use euchre_protocol::PersonaConfig;
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn view() -> PublicGameView {
    project(&game("banter", "banter", None), seat(1))
}

fn persona(chattiness: f64) -> PersonaConfig {
    let mut p = personas().remove(0);
    p.chattiness = chattiness;
    p
}

fn deps<'a>(
    persona: &'a PersonaConfig,
    factory: Option<&'a ModelFactory>,
    log: &'a RecordingLog,
    clock: &'a ManualClock,
) -> DecideDeps<'a> {
    DecideDeps {
        factory,
        persona,
        dossier: "",
        nonce: "banter-test",
        log,
        clock,
        budget: None,
        cancel: None,
    }
}

fn req<'a>(view: &'a PublicGameView) -> BanterRequest<'a> {
    BanterRequest {
        view,
        situation: "win_trick",
        salience: 1.0,
        force: false,
        seq: 1,
        on_delta: None,
        cancel: None,
    }
}

fn talk_factory(talk: Arc<dyn TalkClient>) -> ModelFactory {
    tiered_factory(talk.clone(), talk)
}

fn phrasebook(screened: Option<&str>, usage: DecideUsage) -> BanterOutcome {
    BanterOutcome::Spoke {
        text: "That one travels.".to_owned(),
        source: BanterSource::Phrasebook,
        screened_reason: screened.map(str::to_owned),
        usage,
    }
}

const SCRIPTED_USAGE: DecideUsage = DecideUsage {
    input_tokens: 100,
    output_tokens: 20,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
};

#[tokio::test]
async fn a_quiet_persona_skips_unremarkable_moments_unless_forced() {
    let p = persona(0.5);
    let v = view();
    let (log, clock) = (RecordingLog::new(), ManualClock::new(0));
    let talk = Arc::new(ScriptedTalk::new(vec![Ok(ScriptedResult::text("Mine."))]));
    let factory = talk_factory(talk.clone());
    let out = generate_banter(
        &deps(&p, Some(&factory), &log, &clock),
        BanterRequest {
            salience: 0.4,
            ..req(&v)
        },
    )
    .await;
    assert_eq!(out, BanterOutcome::NotSalient);
    assert_eq!(out.text(), None);
    assert_eq!(
        log.find("banter_skipped").unwrap(),
        json!({ "situation": "win_trick", "salience": 0.4 })
            .as_object()
            .cloned()
            .unwrap()
    );
    assert!(talk.calls().is_empty());
    // Exactly at the threshold speaks.
    let out = generate_banter(
        &deps(&p, Some(&factory), &log, &clock),
        BanterRequest {
            salience: 0.5,
            ..req(&v)
        },
    )
    .await;
    assert_eq!(out.text(), Some("Mine."));
    // Forced bypasses the gate.
    let out = generate_banter(
        &deps(&p, Some(&factory), &log, &clock),
        BanterRequest {
            salience: 0.0,
            force: true,
            ..req(&v)
        },
    )
    .await;
    assert_eq!(out.text(), Some("Mine."));
}

#[tokio::test]
async fn without_a_provider_speaks_a_phrasebook_line() {
    let p = persona(1.0);
    let v = view();
    let (log, clock) = (RecordingLog::new(), ManualClock::new(0));
    let out = generate_banter(&deps(&p, None, &log, &clock), req(&v)).await;
    assert_eq!(out, phrasebook(None, ZERO_USAGE));
    assert_eq!(log.events(), ["ai_no_provider"]);
    assert_eq!(
        log.find("ai_no_provider").unwrap()["where"],
        json!("banter")
    );
}

#[tokio::test]
async fn streams_a_screened_line_with_its_usage_and_request_shape() {
    let p = persona(1.0);
    let v = view();
    let (log, clock) = (RecordingLog::new(), ManualClock::new(0));
    let talk = Arc::new(ScriptedTalk::new(vec![Ok(ScriptedResult::deltas(&[
        "Nice ", "one, ", "folks.",
    ]))]));
    let factory = talk_factory(talk.clone());
    let seen = Mutex::new(Vec::<String>::new());
    let on_delta = |d: &str| seen.lock().unwrap().push(d.to_owned());
    let out = generate_banter(
        &deps(&p, Some(&factory), &log, &clock),
        BanterRequest {
            on_delta: Some(&on_delta),
            ..req(&v)
        },
    )
    .await;
    assert_eq!(
        out,
        BanterOutcome::Spoke {
            text: "Nice one, folks.".to_owned(),
            source: BanterSource::Llm,
            screened_reason: None,
            usage: SCRIPTED_USAGE,
        }
    );
    assert_eq!(*seen.lock().unwrap(), ["Nice ", "one, ", "folks."]);
    assert!(log.events().is_empty());
    let sent = &talk.calls()[0];
    assert_eq!(
        sent.system.last().map(|m| m.content.as_str()),
        Some(BANTER_SYSTEM)
    );
    assert!(sent.user.contains("SITUATION=win_trick"));
    assert_eq!(sent.max_output_tokens, 800);
    assert_eq!(sent.model, factory.slug_for(p.model_id));
}

#[tokio::test]
async fn the_settled_prefix_screen_stops_a_coaching_line_mid_stream() {
    let p = persona(1.0);
    let v = view();
    let (log, clock) = (RecordingLog::new(), ManualClock::new(0));
    let talk = Arc::new(ScriptedTalk::new(vec![Ok(ScriptedResult::deltas(&[
        "Well, ",
        "go alone ",
        "next time.",
    ]))]));
    let factory = talk_factory(talk);
    let seen = Mutex::new(Vec::<String>::new());
    let on_delta = |d: &str| seen.lock().unwrap().push(d.to_owned());
    let out = generate_banter(
        &deps(&p, Some(&factory), &log, &clock),
        BanterRequest {
            on_delta: Some(&on_delta),
            ..req(&v)
        },
    )
    .await;
    assert_eq!(out, phrasebook(Some("table_advice"), ZERO_USAGE));
    // The rejected delta never reaches the caller.
    assert_eq!(*seen.lock().unwrap(), ["Well, "]);
    assert_eq!(
        log.find("banter_screened").unwrap(),
        json!({ "persona": p.id, "reason": "table_advice", "where": "stream" })
            .as_object()
            .cloned()
            .unwrap()
    );
}

#[tokio::test]
async fn the_final_screen_catches_a_trailing_word_and_keeps_the_usage() {
    let p = persona(1.0);
    let v = view();
    let (log, clock) = (RecordingLog::new(), ManualClock::new(0));
    let talk = Arc::new(ScriptedTalk::new(vec![Ok(ScriptedResult::deltas(&[
        "Go ", "alone.",
    ]))]));
    let factory = talk_factory(talk);
    let out = generate_banter(&deps(&p, Some(&factory), &log, &clock), req(&v)).await;
    assert_eq!(out, phrasebook(Some("table_advice"), SCRIPTED_USAGE));
    assert_eq!(
        log.find("banter_screened").unwrap()["where"],
        json!("final")
    );
}

#[tokio::test]
async fn a_provider_error_logs_then_the_empty_buffer_is_screened() {
    let p = persona(1.0);
    let v = view();
    let (log, clock) = (RecordingLog::new(), ManualClock::new(0));
    let talk = Arc::new(ScriptedTalk::new(vec![Err(TalkError::Http {
        status: 500,
    })]));
    let factory = talk_factory(talk);
    let out = generate_banter(&deps(&p, Some(&factory), &log, &clock), req(&v)).await;
    assert_eq!(out, phrasebook(Some("empty"), ZERO_USAGE));
    assert_eq!(log.events(), ["banter_error", "banter_screened"]);
    assert_eq!(
        log.find("banter_error").unwrap()["e"],
        json!("AI_APICallError: HTTP 500")
    );
    assert_eq!(
        log.find("banter_screened").unwrap()["where"],
        json!("final")
    );
}

#[tokio::test(start_paused = true)]
async fn the_banter_deadline_ends_a_hung_stream_with_a_phrasebook_line() {
    let p = persona(1.0);
    let v = view();
    let (log, clock) = (RecordingLog::new(), ManualClock::new(0));
    let factory = talk_factory(Arc::new(HangingTalk));
    let start = tokio::time::Instant::now();
    let out = generate_banter(&deps(&p, Some(&factory), &log, &clock), req(&v)).await;
    assert_eq!(start.elapsed().as_millis(), 4000);
    assert_eq!(out, phrasebook(None, ZERO_USAGE));
    assert_eq!(log.events(), ["banter_error"]);
    assert_eq!(
        log.find("banter_error").unwrap()["e"],
        json!("TimeoutError")
    );

    // A budget override shortens it.
    let log = RecordingLog::new();
    let mut d = deps(&p, Some(&factory), &log, &clock);
    d.budget = Some(DecisionBudgetOverrides {
        banter_ms: Some(250),
        ..DecisionBudgetOverrides::default()
    });
    let start = tokio::time::Instant::now();
    generate_banter(&d, req(&v)).await;
    assert_eq!(start.elapsed().as_millis(), 250);
}

#[tokio::test]
async fn a_caller_cancellation_is_an_abort() {
    let p = persona(1.0);
    let v = view();
    let (log, clock) = (RecordingLog::new(), ManualClock::new(0));
    let factory = talk_factory(Arc::new(HangingTalk));
    let cancel = CancellationToken::new();
    cancel.cancel();
    let out = generate_banter(
        &deps(&p, Some(&factory), &log, &clock),
        BanterRequest {
            cancel: Some(cancel),
            ..req(&v)
        },
    )
    .await;
    assert_eq!(out, phrasebook(None, ZERO_USAGE));
    assert_eq!(log.find("banter_error").unwrap()["e"], json!("AbortError"));
}

#[tokio::test]
async fn banter_never_needs_a_decision_client() {
    // A factory with a decision client still talks through `talk` only.
    let p = persona(1.0);
    let v = view();
    let (log, clock) = (RecordingLog::new(), ManualClock::new(0));
    let talk = Arc::new(ScriptedTalk::new(vec![Ok(ScriptedResult::text("Mine."))]));
    let factory = euchre_ai::testing::jev_factory(Arc::new(HangingJev::new()), talk.clone());
    let out = generate_banter(&deps(&p, Some(&factory), &log, &clock), req(&v)).await;
    assert_eq!(out.text(), Some("Mine."));
    assert_eq!(talk.calls().len(), 1);
}
