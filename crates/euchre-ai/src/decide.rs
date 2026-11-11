//! One bounded Jev decision ([`call_model`]) and the alternate full ladder
//! ([`decide`], `ai/decide.ts`).
//!
//! The aiSeat actor drives its own ladder (`ai_seat::ladder::run_ladder`) and
//! calls only [`call_model`] per attempt, through the bridge. [`decide`] is the
//! library's own ladder with the `ai/config.ts` budgets (6500 / 3000 / 400 /
//! 600 ms); it never fails for an operational reason, only for an empty legal set.
//!
//! Jev owns bid and loner choices: [`call_model`] composes the primary `move`
//! answer with the `support_<i>` and `loner_<i>` sub-answers, and no heuristic
//! can veto or select a move on this path.

use std::time::Duration;

use euchre_engine::{AiDecisionSource, LegalMove, PublicGameView, Seat};
use euchre_protocol::{AIDecideRequest, AIDecisionKind, PersonaConfig, RankedMove};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::bid_rollouts::RolloutError;
use crate::config::{allows_escalation, resolve_budget};
use crate::decision_context::{
    DECISION_PROMPT_VERSION, DecisionQuestion, DecisionQuestionParams, JEV_MODEL,
    build_decision_question,
};
use crate::jev::JevError;
use crate::js::num;
use crate::model::ModelFactory;
use crate::types::{
    AiLog, AiLogEvent, Clock, DecideOutcome, DecisionBudgetOverrides, JevChoiceAnswer,
    JevDecisionInput, JevDecisionOutput,
};
use crate::{HeuristicOutcomeParams, heuristic_outcome};

/// What [`call_model`] and [`decide`] need besides the request (`DecideDeps`).
#[derive(Clone)]
pub struct DecideDeps<'a> {
    /// `None` when no provider is configured.
    pub factory: Option<&'a ModelFactory>,
    /// Snapshotted at match create.
    pub persona: &'a PersonaConfig,
    /// Rendered cross-game episodes (banter only).
    pub dossier: &'a str,
    /// The per-game fence nonce (banter only).
    pub nonce: &'a str,
    /// Structured logging.
    pub log: &'a (dyn AiLog + Send + Sync),
    /// The injected clock.
    pub clock: &'a (dyn Clock + Send + Sync),
    /// Budget overrides.
    pub budget: Option<DecisionBudgetOverrides>,
    /// The caller's cancellation, combined with each call's own deadline.
    pub cancel: Option<CancellationToken>,
}

impl std::fmt::Debug for DecideDeps<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecideDeps")
            .field("factory", &self.factory)
            .field("persona", &self.persona.id)
            .field("budget", &self.budget)
            .finish_non_exhaustive()
    }
}

/// The parts of a `DecisionRequest` that [`call_model`] reads.
#[derive(Clone, Copy, Debug)]
pub struct CallModelRequest<'a> {
    /// The decision kind.
    pub kind: AIDecisionKind,
    /// The deciding seat.
    pub seat: Seat,
    /// The seat's redacted view.
    pub view: &'a PublicGameView,
    /// The request's legal set.
    pub legal: &'a [LegalMove],
    /// The heuristic ranking (supplies the rationale).
    pub ranking: Option<&'a [RankedMove]>,
    /// The table deadline (epoch ms).
    pub deadline_at: u64,
}

impl<'a> CallModelRequest<'a> {
    /// Borrows a wire request.
    pub fn from_request(req: &'a AIDecideRequest) -> CallModelRequest<'a> {
        CallModelRequest {
            kind: req.kind,
            seat: req.seat,
            view: &req.view,
            legal: &req.legal,
            ranking: req.ranking.as_deref(),
            deadline_at: req.deadline_at,
        }
    }
}

/// `CallModelOptions`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CallModelOptions {
    /// The escalation attempt (advisory on the Jev path).
    pub escalate: bool,
    /// Wall-clock budget for the whole call.
    pub budget_ms: f64,
}

/// Why one model call failed. [`CallModelError::name`] is the JS `error.name`.
#[derive(Debug, thiserror::Error)]
pub enum CallModelError {
    /// No decision provider.
    #[error("Jev decision provider is not configured")]
    NotConfigured,
    /// `NoLegalMovesError`.
    #[error("decide() was called with an empty legal set")]
    NoLegalMoves,
    /// The deadline had already passed.
    #[error("Decision deadline expired")]
    DeadlineExpired,
    /// The Jev call failed.
    #[error("{0}")]
    Jev(JevError),
    /// A reference rollout failed while building the question.
    #[error("{0}")]
    Rollout(RolloutError),
    /// A support answer was missing or not `pass` / `call`.
    #[error("Jev omitted the bid support decision")]
    MissingBidSupport,
    /// The chosen bid was unsupported and no distribution came back.
    #[error("Jev omitted the move distribution")]
    MissingDistribution,
    /// The chosen bid was unsupported and nothing was supported.
    #[error("Reduce of empty array with no initial value")]
    NothingSupported,
    /// The choice did not index a candidate.
    #[error("Jev returned a choice outside the legal set")]
    OutsideLegalSet,
    /// A loner answer was missing or not `partner` / `alone`.
    #[error("Jev omitted the applicable loner decision")]
    MissingLoner,
    /// The question-building task panicked or was cancelled.
    #[error("decision question task failed")]
    Join,
}

impl CallModelError {
    /// The JS `error.name` this failure would carry.
    pub const fn name(&self) -> &'static str {
        match self {
            CallModelError::NoLegalMoves => "NoLegalMovesError",
            CallModelError::DeadlineExpired => "TimeoutError",
            CallModelError::Jev(e) => e.name(),
            CallModelError::NothingSupported => "TypeError",
            _ => "Error",
        }
    }

    /// `true` for `TimeoutError` / `AbortError`.
    pub const fn is_timeout_or_abort(&self) -> bool {
        matches!(
            self,
            CallModelError::DeadlineExpired
                | CallModelError::Jev(JevError::Timeout)
                | CallModelError::Jev(JevError::Aborted)
        )
    }
}

impl From<JevError> for CallModelError {
    fn from(e: JevError) -> CallModelError {
        CallModelError::Jev(e)
    }
}

fn seat_json(seat: Seat) -> Value {
    json!(seat.get())
}

fn answer_json(a: &JevChoiceAnswer) -> Value {
    json!({
        "type": a.kind,
        "choice": a.choice,
        "confidence": num(a.confidence),
        "probabilities": Value::Object(a.probabilities.clone()),
    })
}

fn additional_answer<'a>(out: &'a JevDecisionOutput, id: &str) -> Option<&'a JevChoiceAnswer> {
    out.additional_answers
        .as_ref()?
        .iter()
        .find(|(k, _)| k == id)
        .map(|(_, a)| a)
}

fn probability(out: &JevDecisionOutput, key: &str) -> f64 {
    out.probabilities
        .as_ref()
        .and_then(|p| p.get(key))
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN)
}

/// Composes the primary choice with the support and loner sub-answers.
fn compose(
    q: &DecisionQuestion,
    out: &JevDecisionOutput,
) -> Result<(String, LegalMove), CallModelError> {
    let check_for = |key: &str| {
        q.bid_checks
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, c)| c.as_str())
            .filter(|c| !c.is_empty())
    };
    let mut chosen_key = out.choice.clone();
    if check_for(&chosen_key).is_some() {
        let mut supported: Vec<&str> = Vec::new();
        for key in q.criteria.keys() {
            match check_for(key) {
                None => supported.push(key),
                Some(check) => match additional_answer(out, check) {
                    Some(a) if a.choice == "call" => supported.push(key),
                    Some(a) if a.choice == "pass" => {}
                    _ => return Err(CallModelError::MissingBidSupport),
                },
            }
        }
        if !supported.contains(&chosen_key.as_str()) {
            if out.probabilities.is_none() {
                return Err(CallModelError::MissingDistribution);
            }
            let mut iter = supported.into_iter();
            let mut best = iter.next().ok_or(CallModelError::NothingSupported)?;
            for key in iter {
                if probability(out, key) > probability(out, best) {
                    best = key;
                }
            }
            chosen_key = best.to_owned();
        }
    }
    let index = q.criteria.keys().position(|k| *k == chosen_key);
    let mut chosen = index
        .and_then(|i| q.choices.get(i))
        .cloned()
        .ok_or(CallModelError::OutsideLegalSet)?;
    if let Some((_, loner)) = q.alone_choices.iter().find(|(k, _)| *k == chosen_key) {
        chosen = match additional_answer(out, &loner.question_id) {
            Some(a) if a.choice == "alone" => loner.alone.clone(),
            Some(a) if a.choice == "partner" => loner.partner.clone(),
            _ => return Err(CallModelError::MissingLoner),
        };
    }
    Ok((chosen_key, chosen))
}

/// `callModel(deps, req, candidates, opts)`: one bounded Jev decision.
///
/// The question (with its reference rollouts) is built under `spawn_blocking`
/// inside the call's own deadline, `max(1, floor(min(budget, deadline - now)))` ms.
pub async fn call_model(
    deps: &DecideDeps<'_>,
    req: CallModelRequest<'_>,
    candidates: &[LegalMove],
    opts: CallModelOptions,
) -> Result<DecideOutcome, CallModelError> {
    let decision = deps
        .factory
        .and_then(|f| f.decision.clone())
        .ok_or(CallModelError::NotConfigured)?;
    let allowed: Vec<LegalMove> = candidates
        .iter()
        .filter(|m| req.legal.iter().any(|l| l.id == m.id))
        .cloned()
        .collect();
    if allowed.is_empty() {
        return Err(CallModelError::NoLegalMoves);
    }
    let now = deps.clock.now_ms() as f64;
    let remaining = opts.budget_ms.min(req.deadline_at as f64 - now);
    if remaining.is_nan() || remaining <= 0.0 {
        return Err(CallModelError::DeadlineExpired);
    }
    let timeout_ms = remaining.floor().max(1.0) as u64;
    let cancel = deps
        .cancel
        .as_ref()
        .map_or_else(CancellationToken::new, CancellationToken::child_token);

    let view = req.view.clone();
    let persona = deps.persona.clone();
    let legal = allowed;
    let call_cancel = cancel.clone();
    let work = async move {
        let question = tokio::task::spawn_blocking(move || {
            build_decision_question(DecisionQuestionParams {
                view: &view,
                legal: &legal,
                persona: &persona,
            })
        })
        .await
        .map_err(|_| CallModelError::Join)?
        .map_err(CallModelError::Rollout)?;
        let input = JevDecisionInput {
            state: question.state.clone(),
            instructions: question.instructions.clone(),
            criteria: question.criteria.clone(),
            additional_questions: Some(question.additional_questions.clone()),
        };
        let out = decision.decide(input, call_cancel).await?;
        Ok::<_, CallModelError>((question, out))
    };
    let (question, out) = match tokio::time::timeout(Duration::from_millis(timeout_ms), work).await
    {
        Ok(r) => r?,
        Err(_) => {
            cancel.cancel();
            return Err(CallModelError::Jev(JevError::Timeout));
        }
    };

    let (resolved, chosen) = compose(&question, &out)?;

    // Audit both primary and composed judgments, never the private hand/request.
    let mut f = Map::new();
    f.insert("promptVersion".to_owned(), json!(DECISION_PROMPT_VERSION));
    f.insert("kind".to_owned(), json!(req.kind.as_str()));
    if let Some(id) = &out.request_id {
        f.insert("requestId".to_owned(), json!(id));
    }
    if let Some(m) = &out.model_snapshot {
        f.insert("modelSnapshot".to_owned(), json!(m));
    }
    f.insert("primaryChoice".to_owned(), json!(out.choice));
    f.insert("resolvedChoice".to_owned(), json!(resolved));
    f.insert("primaryConfidence".to_owned(), num(out.confidence));
    if let Some(p) = &out.probabilities {
        f.insert("probabilities".to_owned(), Value::Object(p.clone()));
    }
    if let Some(answers) = &out.additional_answers {
        let mut m = Map::new();
        for (k, a) in answers {
            m.insert(k.clone(), answer_json(a));
        }
        f.insert("additionalAnswers".to_owned(), Value::Object(m));
    }
    deps.log.log(AiLogEvent::JevDecision, &f);

    let rationale = req
        .ranking
        .and_then(|r| r.iter().find(|m| m.id == chosen.id))
        .map(|m| m.why.clone())
        .unwrap_or_default();
    Ok(DecideOutcome {
        move_id: chosen.id,
        source: AiDecisionSource::Llm,
        rationale,
        confidence: out.confidence,
        usage: out.usage,
        model_id: Some(JEV_MODEL.to_owned()),
        attempts: 1,
    })
}

/// `DecideOptions`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecideOptions {
    /// The seat's token circuit breaker has tripped.
    pub degraded: bool,
}

/// Context fields of [`should_continue_after`].
#[derive(Clone, Copy, Debug)]
pub struct ContinueContext {
    /// The deciding seat.
    pub seat: Seat,
    /// The escalation attempt.
    pub escalate: bool,
}

/// `shouldContinueAfter(e, log, ctx)`: `true` means try the next rung.
///
/// Timeouts and aborts (`ai_timeout`) continue; everything else on the Jev
/// path, `JevDecisionError` included, logs `ai_error` and stops. (The ai-sdk
/// `NoObjectGeneratedError` / `APICallError` branches have no source here.)
pub fn should_continue_after(
    e: &CallModelError,
    log: &(dyn AiLog + Send + Sync),
    ctx: ContinueContext,
) -> bool {
    let mut f = Map::new();
    f.insert("seat".to_owned(), seat_json(ctx.seat));
    f.insert("escalate".to_owned(), json!(ctx.escalate));
    f.insert("name".to_owned(), json!(e.name()));
    if e.is_timeout_or_abort() {
        log.log(AiLogEvent::Timeout, &f);
        return true;
    }
    log.log(AiLogEvent::Error, &f);
    false
}

/// `decide(deps, req, candidates, opts)`: forced → no provider → degraded →
/// one call (+ one escalation on deliberate kinds) → heuristic.
///
/// Fails only for an empty legal set ([`CallModelError::NoLegalMoves`]).
pub async fn decide(
    deps: &DecideDeps<'_>,
    req: &AIDecideRequest,
    candidates: &[LegalMove],
    opts: DecideOptions,
) -> Result<DecideOutcome, CallModelError> {
    let budget = resolve_budget(deps.budget.as_ref());
    if req.legal.is_empty() {
        return Err(CallModelError::NoLegalMoves);
    }
    let narrowed: Vec<LegalMove> = candidates
        .iter()
        .filter(|m| req.legal.iter().any(|l| l.id == m.id))
        .cloned()
        .collect();
    let set: &[LegalMove] = if narrowed.is_empty() {
        &req.legal
    } else {
        &narrowed
    };
    let heuristic = heuristic_outcome(HeuristicOutcomeParams {
        legal: &req.legal,
        ranking: req.ranking.as_deref(),
        set,
    })
    .ok_or(CallModelError::NoLegalMoves)?;

    let mut base = Map::new();
    base.insert("seat".to_owned(), seat_json(req.seat));
    base.insert("kind".to_owned(), json!(req.kind.as_str()));

    if req.legal.len() == 1 {
        deps.log.log(AiLogEvent::Forced, &base);
        let mut out = heuristic;
        out.move_id = req.legal[0].id.clone();
        out.source = AiDecisionSource::Forced;
        return Ok(out);
    }
    if deps.factory.and_then(|f| f.decision.as_ref()).is_none() {
        deps.log.log(AiLogEvent::NoProvider, &base);
        return Ok(heuristic);
    }
    if opts.degraded {
        deps.log.log(AiLogEvent::Degraded, &base);
        return Ok(heuristic);
    }

    let mut attempts: u32 = 0;
    for escalate in [false, true] {
        if escalate && !allows_escalation(req.kind) {
            break;
        }
        let nominal = if escalate {
            budget.escalation_ms
        } else {
            budget.first_attempt_ms
        } as f64;
        let budget_ms = nominal
            .min(req.deadline_at as f64 - deps.clock.now_ms() as f64 - budget.reserve_ms as f64);
        if budget_ms.is_nan() || budget_ms < budget.min_attempt_ms as f64 {
            break;
        }
        attempts += 1;
        match call_model(
            deps,
            CallModelRequest::from_request(req),
            set,
            CallModelOptions {
                escalate,
                budget_ms,
            },
        )
        .await
        {
            Ok(out) => {
                if set.iter().any(|m| m.id == out.move_id) {
                    let mut f = base.clone();
                    f.insert("escalate".to_owned(), json!(escalate));
                    f.insert("attempts".to_owned(), json!(attempts));
                    deps.log.log(AiLogEvent::Ok, &f);
                    return Ok(DecideOutcome { attempts, ..out });
                }
                let mut f = Map::new();
                f.insert("seat".to_owned(), seat_json(req.seat));
                f.insert("got".to_owned(), json!(out.move_id));
                deps.log.log(AiLogEvent::IllegalId, &f);
            }
            Err(e) => {
                if !should_continue_after(
                    &e,
                    deps.log,
                    ContinueContext {
                        seat: req.seat,
                        escalate,
                    },
                ) {
                    break;
                }
            }
        }
    }
    Ok(DecideOutcome {
        attempts,
        ..heuristic
    })
}
