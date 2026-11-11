//! `aiSeat` status helpers (`ai-seat/index.ts`): latency average and p95, the
//! decision meter, the token budget and its circuit breaker, and the two
//! client-safe read actions.

use euchre_engine::Seat;
use euchre_protocol::serde_util::js_f64;
use euchre_protocol::{AIDecisionKind, PersonaView};
use serde::{Deserialize, Serialize};

use super::ladder::LadderOutcome;
use super::state::{AiSeatState, DecisionRecord};
use crate::config::{
    BUDGET_CALLS, BUDGET_INPUT_TOKENS, BUDGET_OUTPUT_TOKENS, DECISION_LOG_CAP, LATENCY_SAMPLE_CAP,
};
use crate::types::LlmUsage;
use euchre_engine::AiDecisionSource;

/// `average(samples)`; `0` when empty.
pub fn average(samples: &[u64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0;
    for s in samples {
        sum += *s as f64;
    }
    sum / samples.len() as f64
}

/// `p95(samples)`: `sorted[min(len - 1, max(0, ceil(len * 0.95) - 1))]`.
pub fn p95(samples: &[u64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let len = sorted.len();
    let rank = (len as f64 * 0.95).ceil() as usize;
    let idx = (len - 1).min(rank.saturating_sub(1));
    sorted.get(idx).map_or(0.0, |v| *v as f64)
}

/// `pushSample(samples, value)`, capped at `LATENCY_SAMPLE_CAP`.
pub fn push_sample(samples: &mut Vec<u64>, value: u64) {
    samples.push(value);
    while samples.len() > LATENCY_SAMPLE_CAP {
        samples.remove(0);
    }
}

/// Parameters of [`record_decision`].
#[derive(Debug)]
pub struct RecordDecisionParams<'a> {
    /// The seat state.
    pub state: &'a mut AiSeatState,
    /// The request's turn.
    pub turn_id: &'a str,
    /// The request's hand.
    pub hand_no: u32,
    /// The request's kind.
    pub kind: AIDecisionKind,
    /// The ladder outcome.
    pub outcome: &'a LadderOutcome,
    /// Wall-clock latency.
    pub latency_ms: u64,
}

/// `recordDecision(state, req, outcome, latencyMs)`.
pub fn record_decision(params: RecordDecisionParams<'_>) {
    let RecordDecisionParams {
        state,
        turn_id,
        hand_no,
        kind,
        outcome,
        latency_ms,
    } = params;
    let meter = &mut state.meter;
    meter.decisions += 1;
    meter.llm_calls += outcome.llm_calls;
    meter.illegal_attempts += outcome.illegal_attempts;
    match outcome.source {
        AiDecisionSource::Llm => meter.llm_wins += 1,
        AiDecisionSource::Fallback => meter.fallbacks += 1,
        AiDecisionSource::Forced => meter.forced += 1,
    }
    if outcome.first_legal_rescue {
        meter.first_legal_rescues += 1;
    }
    push_sample(&mut meter.samples, latency_ms);
    meter.avg_latency_ms = average(&meter.samples);
    meter.p95_latency_ms = p95(&meter.samples);
    state.decision_log.push(DecisionRecord {
        turn_id: turn_id.to_owned(),
        hand_no,
        kind,
        move_id: outcome.move_id.clone(),
        source: outcome.source,
        latency_ms,
        candidates: outcome.candidates,
        rationale: outcome.rationale.clone(),
    });
    while state.decision_log.len() > DECISION_LOG_CAP {
        state.decision_log.remove(0);
    }
}

/// `applyUsage(state, usage)`: fold usage into the match budget and trip the
/// circuit breaker once any ceiling is reached.
pub fn apply_usage(state: &mut AiSeatState, usage: LlmUsage) {
    let b = &mut state.budget;
    b.tokens_in += usage.input_tokens;
    b.tokens_out += usage.output_tokens;
    b.cache_read_tokens += usage.cache_read_tokens;
    if usage.input_tokens > 0 || usage.output_tokens > 0 {
        b.calls += 1;
    }
    if b.tokens_in >= BUDGET_INPUT_TOKENS
        || b.tokens_out >= BUDGET_OUTPUT_TOKENS
        || b.calls >= BUDGET_CALLS
    {
        b.degraded = true;
    }
}

/// What `getStatus()` returns (`AiStatus`): no card identity, no prompt text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    /// The chair.
    pub seat: Seat,
    /// The match.
    pub game_id: String,
    /// Create input arrived.
    pub provisioned: bool,
    /// Persona id.
    pub persona_id: String,
    /// Persona version.
    pub persona_version: u32,
    /// Heuristic-only.
    pub degraded: bool,
    /// Decisions answered.
    pub decisions: u64,
    /// Model calls.
    pub llm_calls: u64,
    /// Fallbacks.
    pub fallbacks: u64,
    /// Forced moves.
    pub forced: u64,
    /// Rejected attempts.
    pub illegal_attempts: u64,
    /// Mean latency.
    #[serde(with = "js_f64")]
    pub avg_latency_ms: f64,
    /// p95 latency.
    #[serde(with = "js_f64")]
    pub p95_latency_ms: f64,
    /// Prompt tokens.
    pub tokens_in: u64,
    /// Completion tokens.
    pub tokens_out: u64,
    /// Cross-hand notes held.
    pub notes: usize,
}

/// `getStatus()`.
pub fn get_status(state: &AiSeatState) -> AiStatus {
    AiStatus {
        seat: state.seat,
        game_id: state.game_id.clone(),
        provisioned: state.provisioned,
        persona_id: state.persona.id.clone(),
        persona_version: state.persona.version,
        degraded: state.budget.degraded,
        decisions: state.meter.decisions,
        llm_calls: state.meter.llm_calls,
        fallbacks: state.meter.fallbacks,
        forced: state.meter.forced,
        illegal_attempts: state.meter.illegal_attempts,
        avg_latency_ms: state.meter.avg_latency_ms,
        p95_latency_ms: state.meter.p95_latency_ms,
        tokens_in: state.budget.tokens_in,
        tokens_out: state.budget.tokens_out,
        notes: state.notes.len(),
    }
}

/// `getPersona()`: the client-safe face; omits the prompt and model ids.
pub fn get_persona(state: &AiSeatState) -> PersonaView {
    PersonaView {
        id: state.persona.id.clone(),
        version: state.persona.version,
        name: state.persona.name.clone(),
        blurb: state.persona.blurb.clone(),
        seat: state.seat,
        difficulty: state.persona.difficulty,
        chattiness: state.persona.chattiness,
        degraded: state.budget.degraded,
    }
}
