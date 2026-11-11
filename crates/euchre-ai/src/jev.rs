//! Jev's Decisions API client (`ai/jev.ts`), intentionally separate from every
//! text-generation SDK.
//!
//! One `POST {base}/alpha/decisions` per call, no retries. Cancellation is a
//! [`CancellationToken`]: a token cancelled before the call means nothing is
//! sent; one cancelled mid-flight drops the connection. Response bodies are
//! never retained on failure, and the errors carry only safe diagnostics.

use std::fmt;

use async_trait::async_trait;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::decision_context::JEV_MODEL;
use crate::js::stringify;
use crate::types::{DecideUsage, JevChoiceAnswer, JevDecisionInput, JevDecisionOutput};

/// The default `OPENROUTER_BASE_URL` (the existing SDK API base).
pub const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// The probability-total tolerance (`|sum - 1| > 0.02` is a violation).
pub const PROBABILITY_TOLERANCE: f64 = 0.02;

/// The slack allowed above the chosen option's probability.
pub const CHOICE_MAXIMUM_SLACK: f64 = 0.000_001;

/// `JevDecisionError.reason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JevDecisionReason {
    /// A non-2xx response.
    HttpStatus,
    /// The body did not match the response schema.
    ResponseSchema,
    /// An answer's choice or distribution was inconsistent.
    ChoiceDistribution,
}

impl JevDecisionReason {
    /// The exact TS reason code.
    pub const fn as_str(self) -> &'static str {
        match self {
            JevDecisionReason::HttpStatus => "http_status",
            JevDecisionReason::ResponseSchema => "response_schema",
            JevDecisionReason::ChoiceDistribution => "choice_distribution",
        }
    }
}

/// The per-question violation codes, in check order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JevViolation {
    /// No answer for a question.
    MissingAnswer,
    /// The choice is not one of the question's criteria.
    UnknownChoice,
    /// The probability keys differ from the criteria keys.
    OptionKeys,
    /// The probabilities do not sum to 1 (within 0.02).
    ProbabilityTotal,
    /// Another option is more probable than the choice.
    ChoiceNotMaximum,
}

impl JevViolation {
    /// The exact TS violation code.
    pub const fn as_str(self) -> &'static str {
        match self {
            JevViolation::MissingAnswer => "missing_answer",
            JevViolation::UnknownChoice => "unknown_choice",
            JevViolation::OptionKeys => "option_keys",
            JevViolation::ProbabilityTotal => "probability_total",
            JevViolation::ChoiceNotMaximum => "choice_not_maximum",
        }
    }
}

/// `JevDecisionError.diagnostic`: never a response body or private state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JevDiagnostic {
    /// The provider request id, when the response carried one.
    pub request_id: Option<String>,
    /// The question that failed.
    pub question_id: String,
    /// What was wrong.
    pub violation: JevViolation,
}

impl JevDiagnostic {
    /// The TS object shape (`requestId` omitted when absent).
    pub fn to_fields(&self) -> Map<String, Value> {
        let mut m = Map::new();
        if let Some(id) = &self.request_id {
            m.insert("requestId".to_owned(), Value::String(id.clone()));
        }
        m.insert(
            "questionId".to_owned(),
            Value::String(self.question_id.clone()),
        );
        m.insert(
            "violation".to_owned(),
            Value::String(self.violation.as_str().to_owned()),
        );
        m
    }
}

/// `JevDecisionError`: safe diagnostics only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JevDecisionError {
    /// Which check failed.
    pub reason: JevDecisionReason,
    /// The HTTP status, for `http_status`.
    pub status: Option<u16>,
    /// The failing question, for `choice_distribution`.
    pub diagnostic: Option<JevDiagnostic>,
}

impl JevDecisionError {
    /// A reason-only error.
    pub const fn new(reason: JevDecisionReason) -> JevDecisionError {
        JevDecisionError {
            reason,
            status: None,
            diagnostic: None,
        }
    }
}

impl fmt::Display for JevDecisionError {
    /// `Jev ${reason}${status ? ` (${status})` : ''}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Jev {}", self.reason.as_str())?;
        match self.status {
            Some(s) if s != 0 => write!(f, " ({s})"),
            _ => Ok(()),
        }
    }
}

impl std::error::Error for JevDecisionError {}

/// Every way a Jev call can fail.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum JevError {
    /// A validated rejection (`JevDecisionError`).
    #[error("{0}")]
    Decision(JevDecisionError),
    /// The caller's cancellation fired (`AbortError`).
    #[error("This operation was aborted")]
    Aborted,
    /// The call's own deadline fired (`TimeoutError`).
    #[error("The operation timed out.")]
    Timeout,
    /// The request could not be sent or the body could not be read.
    #[error("fetch failed: {0}")]
    Transport(String),
}

impl JevError {
    /// The JS `error.name` this failure would carry.
    pub const fn name(&self) -> &'static str {
        match self {
            JevError::Decision(_) => "JevDecisionError",
            JevError::Aborted => "AbortError",
            JevError::Timeout => "TimeoutError",
            JevError::Transport(_) => "TypeError",
        }
    }
}

impl From<JevDecisionError> for JevError {
    fn from(e: JevDecisionError) -> JevError {
        JevError::Decision(e)
    }
}

/// `ModelFactory.decision`: one Jev structured decision.
#[async_trait]
pub trait JevClient: Send + Sync {
    /// Asks the `move` question (plus `additional_questions`). A `cancel` token
    /// already cancelled means nothing is sent.
    async fn decide(
        &self,
        input: JevDecisionInput,
        cancel: CancellationToken,
    ) -> Result<JevDecisionOutput, JevError>;
}

/// `${base}/alpha/decisions`, where `base` drops one trailing `/` and then one
/// trailing `/v1` from `base_url` (default [`DEFAULT_OPENROUTER_BASE_URL`]).
pub fn jev_endpoint(base_url: Option<&str>) -> String {
    let raw = base_url.unwrap_or(DEFAULT_OPENROUTER_BASE_URL);
    let base = raw.strip_suffix('/').unwrap_or(raw);
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}/alpha/decisions")
}

/// `{ ...additionalQuestions, move: { type: 'choice', instructions, criteria } }`.
/// An existing `move` key keeps its position, as in a JS spread. A `Null`
/// `instructions` stands for `undefined` and is omitted, as `JSON.stringify` does.
pub fn jev_questions(input: &JevDecisionInput) -> Map<String, Value> {
    let mut questions = input.additional_questions.clone().unwrap_or_default();
    let mut question = Map::new();
    question.insert("type".to_owned(), json!("choice"));
    if !input.instructions.is_null() {
        question.insert("instructions".to_owned(), input.instructions.clone());
    }
    question.insert("criteria".to_owned(), Value::Object(input.criteria.clone()));
    questions.insert("move".to_owned(), Value::Object(question));
    questions
}

/// The exact request body: `JSON.stringify({ model, state, questions })`.
pub fn jev_body(input: &JevDecisionInput) -> String {
    stringify(&json!({
        "model": JEV_MODEL,
        "state": input.state,
        "questions": Value::Object(jev_questions(input)),
    }))
}

/* -------------------------------------------------------------------------- */
/* Response validation                                                        */
/* -------------------------------------------------------------------------- */

fn schema_error() -> JevDecisionError {
    JevDecisionError::new(JevDecisionReason::ResponseSchema)
}

/// `z.number().finite().min(0).max(1)`.
fn probability(v: &Value) -> Option<f64> {
    let x = v.as_f64()?;
    (x.is_finite() && (0.0..=1.0).contains(&x)).then_some(x)
}

/// `z.number().int().nonnegative()` (safe integers only).
fn token_count(v: &Value) -> Option<u64> {
    let x = v.as_f64()?;
    if !x.is_finite() || x.fract() != 0.0 || x < 0.0 || x > 9_007_199_254_740_991.0 {
        return None;
    }
    v.as_u64().or(Some(x as u64))
}

/// `z.string().optional()`: absent is fine, anything but a string fails.
fn optional_string(
    obj: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, JevDecisionError> {
    match obj.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(schema_error()),
    }
}

fn parse_answer(v: &Value) -> Result<JevChoiceAnswer, JevDecisionError> {
    let obj = v.as_object().ok_or_else(schema_error)?;
    match obj.get("type") {
        Some(Value::String(t)) if t == "choice" => {}
        _ => return Err(schema_error()),
    }
    let choice = obj
        .get("choice")
        .and_then(Value::as_str)
        .ok_or_else(schema_error)?
        .to_owned();
    let confidence = obj
        .get("confidence")
        .and_then(probability)
        .ok_or_else(schema_error)?;
    let probs = obj
        .get("probabilities")
        .and_then(Value::as_object)
        .ok_or_else(schema_error)?;
    if probs.values().any(|p| probability(p).is_none()) {
        return Err(schema_error());
    }
    Ok(JevChoiceAnswer {
        kind: "choice".to_owned(),
        choice,
        confidence,
        probabilities: probs.clone(),
    })
}

struct ParsedResponse {
    id: Option<String>,
    model: Option<String>,
    answers: Vec<(String, JevChoiceAnswer)>,
    usage: DecideUsage,
}

fn parse_response(body: &[u8]) -> Result<ParsedResponse, JevDecisionError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| schema_error())?;
    let obj = value.as_object().ok_or_else(schema_error)?;
    let id = optional_string(obj, "id")?;
    let model = optional_string(obj, "model")?;
    let answers_obj = obj
        .get("answers")
        .and_then(Value::as_object)
        .ok_or_else(schema_error)?;
    let mut answers = Vec::with_capacity(answers_obj.len());
    for (k, v) in answers_obj {
        answers.push((k.clone(), parse_answer(v)?));
    }
    let usage = obj
        .get("usage")
        .and_then(Value::as_object)
        .ok_or_else(schema_error)?;
    let input_tokens = usage
        .get("input_tokens")
        .and_then(token_count)
        .ok_or_else(schema_error)?;
    let output_tokens = usage
        .get("output_tokens")
        .and_then(token_count)
        .ok_or_else(schema_error)?;
    Ok(ParsedResponse {
        id,
        model,
        answers,
        usage: DecideUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        },
    })
}

fn prob_of(map: &Map<String, Value>, key: &str) -> Option<f64> {
    map.get(key).and_then(Value::as_f64)
}

fn violation_for(question: &Value, answer: Option<&JevChoiceAnswer>) -> Option<JevViolation> {
    let empty = Map::new();
    let criteria = question
        .get("criteria")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let Some(answer) = answer else {
        return Some(JevViolation::MissingAnswer);
    };
    if !criteria.contains_key(&answer.choice) {
        return Some(JevViolation::UnknownChoice);
    }
    let probs = &answer.probabilities;
    if criteria.keys().any(|k| !probs.contains_key(k))
        || probs.keys().any(|k| !criteria.contains_key(k))
    {
        return Some(JevViolation::OptionKeys);
    }
    let total: f64 = probs
        .values()
        .filter_map(Value::as_f64)
        .fold(0.0, |a, b| a + b);
    if (total - 1.0).abs() > PROBABILITY_TOLERANCE {
        return Some(JevViolation::ProbabilityTotal);
    }
    let chosen = prob_of(probs, &answer.choice).unwrap_or(f64::NAN);
    if criteria
        .keys()
        .any(|k| prob_of(probs, k).is_some_and(|p| p > chosen + CHOICE_MAXIMUM_SLACK))
    {
        return Some(JevViolation::ChoiceNotMaximum);
    }
    None
}

/// Validates a 2xx response body against the questions that were asked, in
/// question order: schema first, then per question `missing_answer` →
/// `unknown_choice` → `option_keys` → `probability_total` → `choice_not_maximum`.
pub fn validate_response(
    questions: &Map<String, Value>,
    body: &[u8],
) -> Result<JevDecisionOutput, JevDecisionError> {
    let parsed = parse_response(body)?;
    let find = |id: &str| parsed.answers.iter().find(|(k, _)| k == id).map(|(_, a)| a);
    for (id, question) in questions {
        if let Some(violation) = violation_for(question, find(id)) {
            return Err(JevDecisionError {
                reason: JevDecisionReason::ChoiceDistribution,
                status: None,
                diagnostic: Some(JevDiagnostic {
                    request_id: parsed.id.clone(),
                    question_id: id.clone(),
                    violation,
                }),
            });
        }
    }
    let Some(answer) = find("move").cloned() else {
        // Unreachable: `move` is always asked, so a missing answer failed above.
        return Err(schema_error());
    };
    let additional: Vec<(String, JevChoiceAnswer)> = parsed
        .answers
        .iter()
        .filter(|(k, _)| k != "move")
        .cloned()
        .collect();
    Ok(JevDecisionOutput {
        choice: answer.choice,
        confidence: answer.confidence,
        usage: parsed.usage,
        probabilities: Some(answer.probabilities),
        request_id: parsed.id,
        model_snapshot: parsed.model,
        additional_answers: Some(additional),
    })
}

/* -------------------------------------------------------------------------- */
/* The reqwest client                                                         */
/* -------------------------------------------------------------------------- */

/// Options of [`create_jev_decision_client`].
#[derive(Clone, Default)]
pub struct JevClientOptions {
    /// The OpenRouter key (already trimmed by the factory).
    pub api_key: String,
    /// `OPENROUTER_BASE_URL`, when set.
    pub base_url: Option<String>,
    /// An injected HTTP client (tests, shared pools).
    pub http: Option<reqwest::Client>,
}

impl fmt::Debug for JevClientOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JevClientOptions")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

/// The reqwest-backed [`JevClient`].
#[derive(Clone)]
pub struct HttpJevClient {
    api_key: String,
    endpoint: String,
    http: reqwest::Client,
}

impl fmt::Debug for HttpJevClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpJevClient")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl HttpJevClient {
    /// The resolved `.../alpha/decisions` URL.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

/// `createJevDecisionClient({ apiKey, baseURL, fetch })`.
pub fn create_jev_decision_client(options: JevClientOptions) -> HttpJevClient {
    HttpJevClient {
        endpoint: jev_endpoint(options.base_url.as_deref()),
        api_key: options.api_key,
        http: options.http.unwrap_or_default(),
    }
}

/// A transport failure, reduced to a coarse kind: never a URL, header or body.
fn transport(e: &reqwest::Error) -> JevError {
    let kind = if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect"
    } else if e.is_body() || e.is_decode() {
        "body"
    } else {
        "request"
    };
    JevError::Transport(kind.to_owned())
}

#[async_trait]
impl JevClient for HttpJevClient {
    async fn decide(
        &self,
        input: JevDecisionInput,
        cancel: CancellationToken,
    ) -> Result<JevDecisionOutput, JevError> {
        // `signal.throwIfAborted()`: a cancelled call never sends.
        if cancel.is_cancelled() {
            return Err(JevError::Aborted);
        }
        let questions = jev_questions(&input);
        let body = stringify(&json!({
            "model": JEV_MODEL,
            "state": input.state,
            "questions": Value::Object(questions.clone()),
        }));
        let send = self
            .http
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .body(body)
            .send();
        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(JevError::Aborted),
            r = send => r.map_err(|e| transport(&e))?,
        };
        let status = response.status();
        if !status.is_success() {
            // `response.body?.cancel()`: drop the body unread.
            drop(response);
            return Err(JevError::Decision(JevDecisionError {
                reason: JevDecisionReason::HttpStatus,
                status: Some(status.as_u16()),
                diagnostic: None,
            }));
        }
        let bytes = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(JevError::Aborted),
            b = response.bytes() => match b {
                Ok(b) => b,
                Err(_) => return Err(JevError::Decision(schema_error())),
            },
        };
        let out = validate_response(&questions, &bytes);
        drop(bytes);
        out.map_err(JevError::Decision)
    }
}
