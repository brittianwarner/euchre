//! The conversation-path text against the TS golden (`tests/fixtures/prompts.json`,
//! written by `scripts/prompts.ts`): Layer 0, the banter system line, every
//! `buildLayers` case, `toInstructions`, `cachePrefixTokens`, `sanitizePersona`,
//! and `publicStateOnly`. Plus the table-context.test.ts conversational test.

mod common;

use common::*;
use euchre_ai::banter::BANTER_SYSTEM;
use euchre_ai::notation::public_state_only;
use euchre_ai::prompt::{
    BuildLayersInput, L0_BID, L0_PLAY, MIN_CACHEABLE_PREFIX_TOKENS, SanitizePersonaParams,
    build_layers, cache_prefix_tokens, sanitize_persona, to_instructions,
};
use euchre_ai::testing::RecordingLog;
use euchre_engine::{PublicGameView, Seat};
use euchre_protocol::{AIDecisionKind, PersonaConfig};
use serde_json::{Value, json};

fn golden() -> Value {
    serde_json::from_str(include_str!("fixtures/prompts.json")).unwrap()
}

fn js_number(s: &str) -> f64 {
    s.parse::<f64>().unwrap()
}

/// The fixture records `aggression` / `risk` as `String(n)` (NaN survives).
fn persona(v: &Value) -> PersonaConfig {
    let mut raw = v.clone();
    let aggression = js_number(raw["aggression"].as_str().unwrap());
    let risk = js_number(raw["risk"].as_str().unwrap());
    raw["aggression"] = json!(0);
    raw["risk"] = json!(0);
    let mut p: PersonaConfig = serde_json::from_value(raw).unwrap();
    p.aggression = aggression;
    p.risk = risk;
    p
}

fn log_json(log: &RecordingLog) -> Value {
    Value::Array(
        log.lines()
            .into_iter()
            .map(|l| json!([l.event, Value::Object(l.fields)]))
            .collect(),
    )
}

#[test]
fn layer_zero_and_the_banter_line_are_byte_exact() {
    let g = golden();
    assert_eq!(L0_PLAY, g["L0_PLAY"].as_str().unwrap());
    assert_eq!(L0_BID, g["L0_BID"].as_str().unwrap());
    assert_eq!(BANTER_SYSTEM, g["banterSystem"].as_str().unwrap());
    assert_eq!(
        MIN_CACHEABLE_PREFIX_TOKENS,
        g["MIN_CACHEABLE_PREFIX_TOKENS"].as_u64().unwrap()
    );
}

#[test]
fn every_layer_case_matches_the_ts_golden() {
    let g = golden();
    let cases = g["layerCases"].as_array().unwrap();
    assert_eq!(cases.len(), 20);
    for (i, case) in cases.iter().enumerate() {
        let input = &case["input"];
        let p = persona(&input["persona"]);
        let kind: AIDecisionKind = serde_json::from_value(input["kind"].clone()).unwrap();
        let seat: Seat = serde_json::from_value(input["seat"].clone()).unwrap();
        let log = RecordingLog::new();
        let build = BuildLayersInput {
            kind,
            conversation: input["conversation"].as_bool().unwrap(),
            persona: &p,
            dossier: input["dossier"].as_str().unwrap(),
            nonce: input["nonce"].as_str().unwrap(),
            seat,
            body: input["body"].as_str().unwrap(),
            log: Some(&log),
        };
        let layers = build_layers(build);
        let system: Vec<Value> = layers
            .system
            .iter()
            .map(|l| json!({ "text": l.text, "cache": l.cache }))
            .collect();
        assert_eq!(
            json!({ "system": system, "user": layers.user }),
            case["layers"],
            "case {i}: layers"
        );
        let instructions: Vec<Value> = to_instructions(&layers)
            .iter()
            .map(|m| m.to_json())
            .collect();
        assert_eq!(
            Value::Array(instructions),
            case["instructions"],
            "case {i}: instructions"
        );
        assert_eq!(log_json(&log), case["logs"], "case {i}: logs");
        assert_eq!(
            cache_prefix_tokens(BuildLayersInput { log: None, ..build }),
            case["cachePrefixTokens"].as_u64().unwrap(),
            "case {i}: cachePrefixTokens"
        );
    }
}

#[test]
fn sanitize_persona_matches_the_ts_golden() {
    let g = golden();
    for (i, case) in g["sanitize"].as_array().unwrap().iter().enumerate() {
        let log = RecordingLog::new();
        let out = sanitize_persona(SanitizePersonaParams {
            raw: case["raw"].as_str().unwrap(),
            nonce: case["nonce"].as_str().unwrap(),
            cap: case["cap"].as_u64().unwrap() as usize,
            log: Some(&log),
        });
        assert_eq!(out, case["out"].as_str().unwrap(), "case {i}");
        assert_eq!(log_json(&log), case["logs"], "case {i}: logs");
    }
}

#[test]
fn the_clamp_counts_utf16_units_and_never_splits_a_surrogate_pair() {
    let clamp = |raw: &str, cap: usize| {
        sanitize_persona(SanitizePersonaParams {
            raw,
            nonce: "",
            cap,
            log: None,
        })
    };
    // Each emoji is two UTF-16 units: a cap of 3 keeps one, never half of the second.
    assert_eq!(clamp("😀😀", 3), "😀");
    assert_eq!(clamp("😀😀", 4), "😀😀");
    assert_eq!(clamp("😀😀", 1), "");
    // A BMP typographic apostrophe is one unit.
    assert_eq!(clamp("it’s fine", 4), "it’s");
}

#[test]
fn public_state_only_matches_the_ts_golden() {
    let g = golden();
    let view: PublicGameView = serde_json::from_value(g["banterView"]["view"].clone()).unwrap();
    assert_eq!(
        public_state_only(&view),
        g["banterView"]["publicStateOnly"].as_str().unwrap()
    );
}

#[test]
fn keeps_conversational_prompts_focused_on_voice_instead_of_move_selection() {
    let grandpa = default_assignment(2);
    let layers = build_layers(BuildLayersInput {
        kind: AIDecisionKind::Play,
        conversation: true,
        persona: &grandpa.persona,
        dossier: &grandpa.dossier,
        nonce: "conversation-test",
        seat: seat(3),
        body: "A public trick just ended.",
        log: None,
    });
    let instructions: Vec<&str> = layers.system.iter().map(|l| l.text.as_str()).collect();
    let instructions = instructions.join("\n");
    assert!(instructions.contains("voice and tone only"));
    assert!(instructions.contains("Do not choose a move"));
    assert!(!instructions.contains("Dials bias your preference"));
    assert!(!instructions.contains("Choose the supplied legal option"));
}

fn default_assignment(index: usize) -> euchre_protocol::PersonaAssignment {
    euchre_protocol::default_personas().remove(index)
}
