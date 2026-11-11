//! `ai/model.ts` against the TS golden (sampling allowlist, `modelParams`,
//! `clampTemperature`) plus the env resolution of `modelFactoryFromEnv`.

use euchre_ai::model::{
    DEFAULT_OPENROUTER_MODEL, ModelFactory, ProviderName, allowed_sampling_params,
    clamp_temperature, model_factory_from_env, model_factory_from_env_with, model_params,
    no_sampling_list_is_consistent,
};
use euchre_ai::types::JevDecisionInput;
use euchre_protocol::AnthropicModelId;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn golden() -> Value {
    serde_json::from_str(include_str!("fixtures/prompts.json")).unwrap()
}

fn provider(s: &str) -> ProviderName {
    match s {
        "anthropic" => ProviderName::Anthropic,
        _ => ProviderName::OpenRouter,
    }
}

#[test]
fn the_sampling_allowlist_matches_the_ts_golden() {
    let g = golden();
    for case in g["allowed"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let got: Vec<&str> = allowed_sampling_params(id)
            .iter()
            .map(|p| p.as_str())
            .collect();
        assert_eq!(json!(got), case["allowed"], "{id}");
    }
    assert_eq!(
        no_sampling_list_is_consistent(),
        g["noSamplingListIsConsistent"].as_bool().unwrap()
    );
    assert_eq!(
        DEFAULT_OPENROUTER_MODEL,
        g["defaultOpenRouterModel"].as_str().unwrap()
    );
}

#[test]
fn model_params_match_the_ts_golden() {
    let g = golden();
    let cases = g["params"].as_array().unwrap();
    assert_eq!(cases.len(), 88);
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let p = case["provider"].as_str().unwrap();
        let t: f64 = case["t"].as_str().unwrap().parse().unwrap();
        assert_eq!(
            model_params(id, t, provider(p)).to_json(),
            case["out"],
            "{id} {p} {t}"
        );
    }
}

#[test]
fn clamp_temperature_matches_the_ts_golden() {
    let g = golden();
    for case in g["clamp"].as_array().unwrap() {
        let t: f64 = case["t"].as_str().unwrap().parse().unwrap();
        assert_eq!(json!(clamp_temperature(t)), case["out"], "{t}");
    }
    assert_eq!(clamp_temperature(f64::NEG_INFINITY), 0.2);
}

fn slugs(f: &ModelFactory) -> (String, String) {
    (
        f.slug_for(AnthropicModelId::ClaudeHaiku45),
        f.slug_for(AnthropicModelId::ClaudeOpus5),
    )
}

#[test]
fn env_resolution_prefers_openrouter_then_anthropic() {
    assert!(model_factory_from_env(&[]).is_none());
    // Blank keys are no provider.
    assert!(
        model_factory_from_env(&[("OPENROUTER_API_KEY", ""), ("ANTHROPIC_API_KEY", "")]).is_none()
    );

    let f = model_factory_from_env(&[("OPENROUTER_API_KEY", "k")]).unwrap();
    assert_eq!(f.provider, ProviderName::OpenRouter);
    assert!(f.decision.is_some());
    let default = DEFAULT_OPENROUTER_MODEL.to_owned();
    assert_eq!(slugs(&f), (default.clone(), default));

    let f = model_factory_from_env(&[
        ("OPENROUTER_API_KEY", "k"),
        ("OPENROUTER_MODEL_TALK", "x/talk"),
        ("OPENROUTER_MODEL_PLAY", "ignored/legacy"),
        ("ANTHROPIC_API_KEY", "a"),
    ])
    .unwrap();
    assert_eq!(f.provider, ProviderName::OpenRouter);
    assert_eq!(slugs(&f), ("x/talk".to_owned(), "x/talk".to_owned()));

    // An empty talk override is still an override.
    let f = model_factory_from_env(&[("OPENROUTER_API_KEY", "k"), ("OPENROUTER_MODEL_TALK", "")])
        .unwrap();
    assert_eq!(slugs(&f), (String::new(), String::new()));

    let f = model_factory_from_env(&[("ANTHROPIC_API_KEY", "a")]).unwrap();
    assert_eq!(f.provider, ProviderName::Anthropic);
    assert!(f.decision.is_none());
    assert_eq!(
        slugs(&f),
        ("claude-haiku-4-5".to_owned(), "claude-opus-5".to_owned())
    );

    let lookup = |k: &str| (k == "OPENROUTER_API_KEY").then(|| "k".to_owned());
    assert!(model_factory_from_env_with(&lookup, None).is_some());
}

#[tokio::test]
async fn the_env_base_url_reaches_the_decision_client() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/alpha/decisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answers": { "move": { "type": "choice", "choice": "move_0", "confidence": 1, "probabilities": { "move_0": 1 } } },
            "usage": { "input_tokens": 1, "output_tokens": 0 }
        })))
        .mount(&server)
        .await;
    let base = format!("{}/api/v1", server.uri());
    let f = model_factory_from_env(&[
        ("OPENROUTER_API_KEY", "test"),
        ("OPENROUTER_BASE_URL", base.as_str()),
    ])
    .unwrap();
    let decision = f.decision.clone().unwrap();
    let out = decision
        .decide(
            JevDecisionInput {
                state: json!("s"),
                instructions: json!("i"),
                criteria: json!({ "move_0": "Pass" }).as_object().cloned().unwrap(),
                additional_questions: None,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(out.choice, "move_0");
    assert_eq!(
        server.received_requests().await.unwrap_or_default().len(),
        1
    );
}
