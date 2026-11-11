//! The seam between the aiSeat ladder and the model layer (`ai-seat/llm-bridge.ts`).
//!
//! The seat owns the ladder (when to call, how long to wait, what to do when
//! the answer is wrong or late); this module adapts one attempt onto
//! [`call_model`] and one line of talk onto [`generate_banter`]. Every guarded
//! call resolves to `None` on failure and never errors.
//!
//! Instead of TS's module-level `setLlmProvider` override, the provider is an
//! injected [`LlmProvider`] value.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use euchre_engine::{LegalMove, PublicGameView, Seat};
use euchre_protocol::{AIDecisionKind, PersonaConfig, RankedMove};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::banter::{BanterOutcome, BanterRequest, generate_banter};
use crate::decide::{CallModelError, CallModelOptions, CallModelRequest, DecideDeps, call_model};
use crate::jev::JevError;
use crate::model::ModelFactory;
use crate::types::{AiLog, Clock, DecisionBudgetOverrides, LlmUsage, SeatAiLog, SeatLog};

/// The `jev_failure` event name.
pub const JEV_FAILURE: &str = "jev_failure";

/// Everything one constrained decision needs (`LlmDecideInput`). Self-contained.
#[derive(Clone, Debug, PartialEq)]
pub struct LlmDecideInput {
    /// The decision kind.
    pub kind: AIDecisionKind,
    /// The deciding seat.
    pub seat: Seat,
    /// `project(state, seat)`: the redaction boundary, never widened.
    pub view: PublicGameView,
    /// The candidate set the answer must come from.
    pub candidates: Vec<LegalMove>,
    /// The heuristic's ordering over the same set.
    pub ranking: Vec<RankedMove>,
    /// The persona (prompt and house prompt untrusted).
    pub persona: PersonaConfig,
    /// Semi-trusted cross-game notes.
    pub dossier: String,
    /// The per-game fence nonce.
    pub nonce: String,
    /// Wall-clock budget for the whole call, already clamped.
    pub budget_ms: f64,
    /// The bid-path escalation attempt.
    pub escalate: bool,
    /// The table deadline (epoch ms).
    pub deadline_at: u64,
}

/// What the model chose (`LlmDecideResult`).
#[derive(Clone, Debug, PartialEq)]
pub struct LlmDecideResult {
    /// The chosen id; checked against the candidates by the seat.
    pub move_id: String,
    /// Free text; screened by the seat.
    pub rationale: Option<String>,
    /// Advisory confidence.
    pub confidence: Option<f64>,
    /// Token usage.
    pub usage: Option<LlmUsage>,
}

/// The decision function the seat calls (`LlmDecider`).
#[async_trait]
pub trait LlmDecider: Send + Sync {
    /// One attempt. `log` is the seat's sink (diagnostics only).
    async fn decide(
        &self,
        input: &LlmDecideInput,
        log: &(dyn SeatLog + Send + Sync),
    ) -> Result<LlmDecideResult, CallModelError>;
}

/// Everything one line of table talk needs (`LlmBanterInput`).
#[derive(Clone, Debug, PartialEq)]
pub struct LlmBanterInput {
    /// The speaking seat.
    pub seat: Seat,
    /// The hand-stripped view (`publicOnlyView`).
    pub view: PublicGameView,
    /// A short, card-free description of the moment.
    pub situation: String,
    /// The persona.
    pub persona: PersonaConfig,
    /// Cross-game notes (the seat passes `''`).
    pub dossier: String,
    /// The per-game fence nonce.
    pub nonce: String,
    /// Wall-clock budget.
    pub budget_ms: u64,
}

/// The banter function the seat calls (`LlmBanterWriter`). `None` = stay quiet.
#[async_trait]
pub trait LlmBanterWriter: Send + Sync {
    /// One line, or `None`.
    async fn write(&self, input: &LlmBanterInput) -> Option<String>;
}

/// The real decider: one [`call_model`] attempt per input.
#[derive(Clone)]
pub struct JevDecider {
    factory: Arc<ModelFactory>,
    clock: Arc<dyn Clock + Send + Sync>,
    cancel: Option<CancellationToken>,
}

impl std::fmt::Debug for JevDecider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JevDecider")
            .field("factory", &self.factory)
            .finish_non_exhaustive()
    }
}

impl JevDecider {
    /// A decider over `factory` (which must carry a decision client) reading `clock`.
    pub fn new(factory: Arc<ModelFactory>, clock: Arc<dyn Clock + Send + Sync>) -> JevDecider {
        JevDecider {
            factory,
            clock,
            cancel: None,
        }
    }

    /// Adds a caller cancellation (the actor's shutdown signal).
    pub fn with_cancel(mut self, cancel: CancellationToken) -> JevDecider {
        self.cancel = Some(cancel);
        self
    }
}

#[async_trait]
impl LlmDecider for JevDecider {
    async fn decide(
        &self,
        input: &LlmDecideInput,
        log: &(dyn SeatLog + Send + Sync),
    ) -> Result<LlmDecideResult, CallModelError> {
        let ai_log = SeatAiLog(log);
        let deps = DecideDeps {
            factory: Some(&self.factory),
            persona: &input.persona,
            dossier: &input.dossier,
            nonce: &input.nonce,
            log: &ai_log,
            clock: self.clock.as_ref(),
            budget: None,
            cancel: self.cancel.clone(),
        };
        let outcome = call_model(
            &deps,
            CallModelRequest {
                kind: input.kind,
                seat: input.seat,
                view: &input.view,
                legal: &input.candidates,
                ranking: Some(&input.ranking),
                deadline_at: input.deadline_at,
            },
            &input.candidates,
            CallModelOptions {
                escalate: input.escalate,
                budget_ms: input.budget_ms,
            },
        )
        .await?;
        Ok(LlmDecideResult {
            move_id: outcome.move_id,
            rationale: Some(outcome.rationale),
            confidence: Some(outcome.confidence),
            usage: Some(outcome.usage.into()),
        })
    }
}

/// The real banter writer: [`generate_banter`] with salience 1, forced, seq =
/// `view.handNo`, and the input's budget.
#[derive(Clone)]
pub struct FactoryBanterWriter {
    factory: Arc<ModelFactory>,
    clock: Arc<dyn Clock + Send + Sync>,
    log: Arc<dyn AiLog + Send + Sync>,
}

impl std::fmt::Debug for FactoryBanterWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FactoryBanterWriter")
            .field("factory", &self.factory)
            .finish_non_exhaustive()
    }
}

impl FactoryBanterWriter {
    /// A writer over `factory`.
    pub fn new(
        factory: Arc<ModelFactory>,
        clock: Arc<dyn Clock + Send + Sync>,
        log: Arc<dyn AiLog + Send + Sync>,
    ) -> FactoryBanterWriter {
        FactoryBanterWriter {
            factory,
            clock,
            log,
        }
    }
}

#[async_trait]
impl LlmBanterWriter for FactoryBanterWriter {
    async fn write(&self, input: &LlmBanterInput) -> Option<String> {
        let deps = DecideDeps {
            factory: Some(&self.factory),
            persona: &input.persona,
            dossier: &input.dossier,
            nonce: &input.nonce,
            log: self.log.as_ref(),
            clock: self.clock.as_ref(),
            budget: Some(DecisionBudgetOverrides {
                banter_ms: Some(input.budget_ms),
                ..DecisionBudgetOverrides::default()
            }),
            cancel: None,
        };
        let out = generate_banter(
            &deps,
            BanterRequest {
                view: &input.view,
                situation: &input.situation,
                salience: 1.0,
                force: true,
                seq: i64::from(input.view.hand_no),
                on_delta: None,
                cancel: None,
            },
        )
        .await;
        match out {
            BanterOutcome::Spoke { text, .. } => Some(text),
            BanterOutcome::NotSalient => None,
        }
    }
}

/// The resolved model seam for one seat (TS `resolveDecider` /
/// `resolveBanterWriter` plus the `setLlmProvider` override).
#[derive(Clone, Default)]
pub struct LlmProvider {
    /// `None` = no provider: the ladder plays the heuristic.
    pub decider: Option<Arc<dyn LlmDecider>>,
    /// `None` = no banter.
    pub banter: Option<Arc<dyn LlmBanterWriter>>,
}

impl std::fmt::Debug for LlmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmProvider")
            .field("decider", &self.decider.is_some())
            .field("banter", &self.banter.is_some())
            .finish()
    }
}

/// Dependencies of [`LlmProvider::from_factory`].
#[derive(Clone)]
pub struct ProviderDeps {
    /// The clock decisions measure their deadline against.
    pub clock: Arc<dyn Clock + Send + Sync>,
    /// The sink for banter's AI log events.
    pub log: Arc<dyn AiLog + Send + Sync>,
}

impl std::fmt::Debug for ProviderDeps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderDeps")
    }
}

impl LlmProvider {
    /// No decider, no banter.
    pub fn none() -> LlmProvider {
        LlmProvider::default()
    }

    /// `resolveDecider()` + `resolveBanterWriter()` over one memoised factory:
    /// a decider only when the factory has a decision client; a writer whenever
    /// a factory exists.
    pub fn from_factory(factory: Option<Arc<ModelFactory>>, deps: ProviderDeps) -> LlmProvider {
        let Some(factory) = factory else {
            return LlmProvider::none();
        };
        let decider: Option<Arc<dyn LlmDecider>> = factory
            .decision
            .is_some()
            .then(|| Arc::new(JevDecider::new(factory.clone(), deps.clock.clone())) as _);
        let banter: Option<Arc<dyn LlmBanterWriter>> = Some(Arc::new(FactoryBanterWriter::new(
            factory, deps.clock, deps.log,
        )));
        LlmProvider { decider, banter }
    }
}

/// `parseDecideResult(value, allowedIds)`: `None` when the id is outside the
/// candidate set; a non-finite confidence is dropped.
pub fn parse_decide_result(
    result: LlmDecideResult,
    candidates: &[LegalMove],
) -> Option<LlmDecideResult> {
    if !candidates.iter().any(|m| m.id == result.move_id) {
        return None;
    }
    Some(LlmDecideResult {
        confidence: result.confidence.filter(|c| c.is_finite()),
        ..result
    })
}

/// The `jev_failure` reason for an error.
fn failure_fields(input: &LlmDecideInput, error: Option<&CallModelError>) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("seat".to_owned(), json!(input.seat.get()));
    f.insert("kind".to_owned(), json!(input.kind.as_str()));
    f.insert("handNo".to_owned(), json!(input.view.hand_no));
    let reason = match error {
        Some(CallModelError::Jev(JevError::Decision(d))) => d.reason.as_str(),
        Some(e) if e.is_timeout_or_abort() => "timeout",
        // `None` is the bridge's own `llm_timeout`.
        None => "timeout",
        Some(_) => "transport_or_adapter",
    };
    f.insert("reason".to_owned(), json!(reason));
    if let Some(CallModelError::Jev(JevError::Decision(d))) = error {
        if let Some(status) = d.status {
            f.insert("status".to_owned(), json!(status));
        }
        if let Some(diag) = &d.diagnostic {
            f.extend(diag.to_fields());
        }
    }
    f
}

/// `withDeadline`'s duration: `None` for a non-finite or non-positive budget.
fn deadline(ms: f64) -> Option<Duration> {
    (ms.is_finite() && ms > 0.0).then(|| Duration::from_secs_f64(ms / 1000.0))
}

/// `callDecider(decider, input)`: one guarded call, hard-bounded by
/// `input.budget_ms`. Every failure logs `jev_failure` and resolves `None`; an
/// id outside the candidates resolves `None` without a log.
pub async fn call_decider(
    decider: &dyn LlmDecider,
    input: &LlmDecideInput,
    log: &(dyn SeatLog + Send + Sync),
) -> Option<LlmDecideResult> {
    let Some(limit) = deadline(input.budget_ms) else {
        // `new Error('no budget')` is a plain `Error`: `transport_or_adapter`.
        log.info(
            JEV_FAILURE,
            &failure_fields(input, Some(&CallModelError::Join)),
        );
        return None;
    };
    match tokio::time::timeout(limit, decider.decide(input, log)).await {
        Ok(Ok(result)) => parse_decide_result(result, &input.candidates),
        Ok(Err(e)) => {
            log.info(JEV_FAILURE, &failure_fields(input, Some(&e)));
            None
        }
        Err(_) => {
            log.info(JEV_FAILURE, &failure_fields(input, None));
            None
        }
    }
}

/// `callBanterWriter(writer, input)`: a non-empty line or `None`; never errors.
pub async fn call_banter_writer(
    writer: &dyn LlmBanterWriter,
    input: &LlmBanterInput,
) -> Option<String> {
    let limit = deadline(input.budget_ms as f64)?;
    match tokio::time::timeout(limit, writer.write(input)).await {
        Ok(Some(text)) if !text.is_empty() => Some(text),
        _ => None,
    }
}
