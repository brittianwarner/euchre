//! Port of `src/lib/ai/jev.test.ts` ("Jev decision boundary"), against a local
//! wiremock server. No real network, no real key.

use euchre_ai::decision_context::JEV_MODEL;
use euchre_ai::jev::{
    JevClient, JevClientOptions, JevDecisionReason, JevError, JevViolation,
    create_jev_decision_client, jev_body, jev_endpoint,
};
use euchre_ai::model::{
    AnthropicFactoryOptions, create_anthropic_model_factory, model_factory_from_env,
};
use euchre_ai::types::JevDecisionInput;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn valid() -> Value {
    json!({
        "answers": {
            "move": {
                "type": "choice",
                "choice": "move_1",
                "confidence": 0.8,
                "probabilities": { "move_0": 0.1, "move_1": 0.9 }
            }
        },
        "usage": { "input_tokens": 42, "output_tokens": 0 }
    })
}

fn input() -> JevDecisionInput {
    JevDecisionInput {
        state: json!("seat-visible state"),
        instructions: json!("Choose the best partnership bid using the supplied hand and auction."),
        criteria: json!({ "move_0": "Pass", "move_1": "Order up" })
            .as_object()
            .cloned()
            .unwrap(),
        additional_questions: None,
    }
}

async fn server_with(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/alpha/decisions"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

fn client(server: &MockServer) -> euchre_ai::jev::HttpJevClient {
    create_jev_decision_client(JevClientOptions {
        api_key: "test".to_owned(),
        base_url: Some(format!("{}/api/v1", server.uri())),
        http: None,
    })
}

async fn requests(server: &MockServer) -> Vec<wiremock::Request> {
    server.received_requests().await.unwrap_or_default()
}

#[test]
fn derives_the_decisions_url_from_the_sdk_base() {
    assert_eq!(
        jev_endpoint(None),
        "https://openrouter.ai/api/alpha/decisions"
    );
    assert_eq!(
        jev_endpoint(Some("https://example.test/api/v1/")),
        "https://example.test/api/alpha/decisions"
    );
    assert_eq!(
        jev_endpoint(Some("https://example.test/api")),
        "https://example.test/api/alpha/decisions"
    );
    // Only one trailing slash and one trailing /v1 are removed.
    assert_eq!(
        jev_endpoint(Some("https://example.test/v1/v1")),
        "https://example.test/v1/alpha/decisions"
    );
}

#[tokio::test]
async fn uses_the_pinned_decisions_model_and_propagates_cancellation_and_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/alpha/decisions"))
        .and(header("authorization", "Bearer test"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(valid()))
        .mount(&server)
        .await;
    let result = client(&server)
        .decide(input(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(result.choice, "move_1");
    assert_eq!(result.usage.input_tokens, 42);
    assert_eq!(result.usage.output_tokens, 0);
    let sent = requests(&server).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].url.path(), "/api/alpha/decisions");
    let body: Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(
        body,
        json!({
            "model": JEV_MODEL,
            "state": "seat-visible state",
            "questions": {
                "move": {
                    "type": "choice",
                    "instructions": "Choose the best partnership bid using the supplied hand and auction.",
                    "criteria": { "move_0": "Pass", "move_1": "Order up" }
                }
            }
        })
    );
    // Byte-exact with the TS JSON.stringify body.
    assert_eq!(String::from_utf8_lossy(&sent[0].body), jev_body(&input()));
}

fn with_move(patch: Value) -> Value {
    let mut body = valid();
    let mv = body["answers"]["move"].as_object_mut().unwrap();
    for (k, v) in patch.as_object().unwrap() {
        mv.insert(k.clone(), v.clone());
    }
    body
}

fn rejection_cases() -> Vec<(Value, JevDecisionReason, Option<JevViolation>)> {
    use JevDecisionReason::*;
    vec![
        (
            with_move(json!({ "choice": "illegal" })),
            ChoiceDistribution,
            Some(JevViolation::UnknownChoice),
        ),
        (with_move(json!({ "confidence": 2 })), ResponseSchema, None),
        (
            with_move(json!({ "probabilities": { "move_0": 1 } })),
            ChoiceDistribution,
            Some(JevViolation::OptionKeys),
        ),
        (
            with_move(json!({ "probabilities": { "move_0": 0.8, "move_1": 0.8 } })),
            ChoiceDistribution,
            Some(JevViolation::ProbabilityTotal),
        ),
        (
            with_move(json!({ "choice": "move_0" })),
            ChoiceDistribution,
            Some(JevViolation::ChoiceNotMaximum),
        ),
        (
            json!({ "answers": {}, "usage": valid()["usage"] }),
            ChoiceDistribution,
            Some(JevViolation::MissingAnswer),
        ),
        (
            json!({ "answers": valid()["answers"] }),
            ResponseSchema,
            None,
        ),
    ]
}

#[tokio::test]
async fn rejects_malformed_or_out_of_set_responses() {
    for (i, (body, reason, violation)) in rejection_cases().into_iter().enumerate() {
        let server = server_with(ResponseTemplate::new(200).set_body_json(body)).await;
        let err = client(&server)
            .decide(input(), CancellationToken::new())
            .await
            .unwrap_err();
        let JevError::Decision(d) = err else {
            panic!("case {i}: {err:?}");
        };
        assert_eq!(d.reason, reason, "case {i}");
        assert_eq!(d.diagnostic.map(|x| x.violation), violation, "case {i}");
    }
}

#[tokio::test]
async fn does_not_retry_provider_errors_or_send_a_cancelled_request() {
    let server = server_with(ResponseTemplate::new(429)).await;
    let decide = client(&server);
    let err = decide
        .decide(input(), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("429"), "{err}");
    assert_eq!(err.to_string(), "Jev http_status (429)");
    assert_eq!(requests(&server).await.len(), 1);
    let aborted = CancellationToken::new();
    aborted.cancel();
    let err = decide.decide(input(), aborted).await.unwrap_err();
    assert_eq!(err, JevError::Aborted);
    assert_eq!(requests(&server).await.len(), 1);
}

#[tokio::test]
async fn accepts_native_json_descriptions_and_validates_every_batched_judgment() {
    let mut batched = input();
    batched.state = json!({ "ownHand": ["Ace of hearts"] });
    batched.additional_questions = json!({
        "loner": {
            "type": "choice",
            "instructions": { "question": "Keep partner or go alone?" },
            "criteria": { "partner": { "meaning": "Keep partner" }, "alone": { "meaning": "Play alone" } }
        }
    })
    .as_object()
    .cloned();
    let mut body = valid();
    body["id"] = json!("test-request");
    body["model"] = json!("typesafe/jev-1.13-snapshot");
    body["answers"]["loner"] = json!({
        "type": "choice",
        "choice": "partner",
        "confidence": 0.8,
        "probabilities": { "partner": 0.9, "alone": 0.1 }
    });
    let server = server_with(ResponseTemplate::new(200).set_body_json(body)).await;
    let result = client(&server)
        .decide(batched.clone(), CancellationToken::new())
        .await
        .unwrap();
    let extra = result.additional_answers.clone().unwrap();
    assert_eq!(extra.len(), 1);
    assert_eq!(extra[0].0, "loner");
    assert_eq!(extra[0].1.choice, "partner");
    assert_eq!(result.request_id.as_deref(), Some("test-request"));
    assert_eq!(
        result.model_snapshot.as_deref(),
        Some("typesafe/jev-1.13-snapshot")
    );
    assert_eq!(
        Value::Object(result.probabilities.unwrap()),
        valid()["answers"]["move"]["probabilities"]
    );
    let sent = requests(&server).await;
    let sent_body: Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(sent_body["state"], batched.state);
    // Additional questions come first; `move` is last.
    let keys: Vec<&String> = sent_body["questions"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["loner", "move"]);

    let server = server_with(ResponseTemplate::new(200).set_body_json(valid())).await;
    let err = client(&server)
        .decide(batched, CancellationToken::new())
        .await
        .unwrap_err();
    let JevError::Decision(d) = err else {
        panic!("{err:?}")
    };
    let diag = d.diagnostic.unwrap();
    assert_eq!(diag.question_id, "loner");
    assert_eq!(diag.violation, JevViolation::MissingAnswer);
}

#[test]
fn never_enables_decisions_through_a_text_only_provider_or_legacy_model_overrides() {
    let anthropic = create_anthropic_model_factory(AnthropicFactoryOptions {
        api_key: Some("test".to_owned()),
        ..AnthropicFactoryOptions::default()
    })
    .unwrap();
    assert!(anthropic.decision.is_none());
    let factory = model_factory_from_env(&[
        ("OPENROUTER_API_KEY", "test"),
        ("OPENROUTER_MODEL_PLAY", "other/model"),
    ])
    .unwrap();
    assert!(factory.decision.is_some());
    assert!(model_factory_from_env(&[]).is_none());
}

#[tokio::test]
async fn classifies_failures_without_retaining_potentially_sensitive_response_bodies() {
    let server =
        server_with(ResponseTemplate::new(503).set_body_string("private key and request data"))
            .await;
    let err = client(&server)
        .decide(input(), CancellationToken::new())
        .await
        .unwrap_err();
    let JevError::Decision(d) = &err else {
        panic!("{err:?}")
    };
    assert_eq!(d.reason, JevDecisionReason::HttpStatus);
    assert_eq!(d.status, Some(503));
    assert!(!err.to_string().contains("private key"));
    assert!(!format!("{err:?}").contains("private key"));

    let server =
        server_with(ResponseTemplate::new(200).set_body_json(json!({ "invalid": "private hand" })))
            .await;
    let err = client(&server)
        .decide(input(), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        err,
        JevError::Decision(euchre_ai::jev::JevDecisionError::new(
            JevDecisionReason::ResponseSchema
        ))
    );
    assert!(!format!("{err:?}").contains("private hand"));
}

#[tokio::test]
async fn a_cancellation_mid_flight_drops_the_call_as_an_abort() {
    let server = server_with(
        ResponseTemplate::new(200)
            .set_body_json(valid())
            .set_delay(std::time::Duration::from_secs(30)),
    )
    .await;
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        c2.cancel();
    });
    let err = client(&server).decide(input(), cancel).await.unwrap_err();
    assert_eq!(err, JevError::Aborted);
    assert_eq!(err.name(), "AbortError");
}

#[test]
fn schema_edges_match_zod() {
    use euchre_ai::jev::validate_response;
    let questions = euchre_ai::jev::jev_questions(&input());
    let ok = |v: Value| validate_response(&questions, v.to_string().as_bytes());
    // `id: null` fails `z.string().optional()`.
    let mut b = valid();
    b["id"] = Value::Null;
    assert!(ok(b).is_err());
    // A fractional token count fails `.int()`; `42.0` is an integer in JS.
    let mut b = valid();
    b["usage"]["input_tokens"] = json!(4.5);
    assert!(ok(b).is_err());
    let mut b = valid();
    b["usage"]["input_tokens"] = json!(42.0);
    assert_eq!(ok(b).unwrap().usage.input_tokens, 42);
    // Negative counts fail.
    let mut b = valid();
    b["usage"]["output_tokens"] = json!(-1);
    assert!(ok(b).is_err());
    // An extra answer is validated too (z.record) even though nobody asked it.
    let mut b = valid();
    b["answers"]["extra"] = json!({ "type": "other" });
    assert!(ok(b).is_err());
    // Not JSON.
    assert!(validate_response(&questions, b"not json").is_err());
    // Probability totals within 0.02 pass.
    let b = with_move(json!({ "probabilities": { "move_0": 0.11, "move_1": 0.905 } }));
    assert!(ok(b).is_ok());
    // A tie within the 1e-6 slack passes.
    let b = with_move(
        json!({ "choice": "move_0", "probabilities": { "move_0": 0.5, "move_1": 0.5000005 } }),
    );
    assert!(ok(b).is_ok());
}

#[test]
fn undefined_instructions_are_omitted_like_json_stringify() {
    let mut i = input();
    i.instructions = Value::Null;
    let body: Value = serde_json::from_str(&jev_body(&i)).unwrap();
    assert_eq!(
        body["questions"]["move"],
        json!({ "type": "choice", "criteria": { "move_0": "Pass", "move_1": "Order up" } })
    );
}
