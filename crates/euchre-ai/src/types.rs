//! Decision inputs, outputs and seams (`ai/types.ts`).
//!
//! The model plumbing itself (HTTP, retries, streaming) belongs to the LLM
//! stage; this module only fixes the shapes and the narrow traits it plugs into.

use euchre_engine::AiDecisionSource;
use euchre_protocol::LegalMoveId;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `DecisionDescription`: a string, object, array or `null` handed to Jev.
pub type DecisionDescription = Value;

/// Token usage of one model call (`DecideUsage`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecideUsage {
    /// Prompt tokens.
    pub input_tokens: u64,
    /// Completion tokens.
    pub output_tokens: u64,
    /// Prompt-cache reads.
    pub cache_read_tokens: u64,
    /// Prompt-cache writes.
    pub cache_write_tokens: u64,
}

/// `ZERO_USAGE`.
pub const ZERO_USAGE: DecideUsage = DecideUsage {
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
};

/// The aiSeat bridge's usage shape (`LlmUsage`, no cache writes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmUsage {
    /// Prompt tokens.
    pub input_tokens: u64,
    /// Completion tokens.
    pub output_tokens: u64,
    /// Prompt-cache reads.
    pub cache_read_tokens: u64,
}

/// `ZERO_USAGE` of the aiSeat ladder.
pub const ZERO_LLM_USAGE: LlmUsage = LlmUsage {
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
};

impl From<DecideUsage> for LlmUsage {
    fn from(u: DecideUsage) -> LlmUsage {
        LlmUsage {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_read_tokens: u.cache_read_tokens,
        }
    }
}

/// The result of one decision (`DecideOutcome`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecideOutcome {
    /// The chosen move.
    pub move_id: LegalMoveId,
    /// Model, heuristic fallback, or forced.
    pub source: AiDecisionSource,
    /// Screened, clamped rationale.
    pub rationale: String,
    /// `0..=1`.
    #[serde(with = "euchre_protocol::serde_util::js_f64")]
    pub confidence: f64,
    /// Token usage.
    pub usage: DecideUsage,
    /// The model that answered, or `None` for the heuristic.
    pub model_id: Option<String>,
    /// Model attempts made.
    pub attempts: u32,
}

/// Per-decision budgets (`DecisionBudget`), in ms and tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionBudget {
    /// First attempt wall clock.
    pub first_attempt_ms: u64,
    /// Escalation wall clock.
    pub escalation_ms: u64,
    /// Kept back from the deadline.
    pub reserve_ms: u64,
    /// Attempts shorter than this are skipped.
    pub min_attempt_ms: u64,
    /// Banter wall clock.
    pub banter_ms: u64,
    /// Bid output tokens.
    pub max_output_tokens_bid: u64,
    /// Play output tokens.
    pub max_output_tokens_play: u64,
    /// Banter output tokens.
    pub max_output_tokens_banter: u64,
}

/// `Partial<DecisionBudget>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionBudgetOverrides {
    /// Override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_attempt_ms: Option<u64>,
    /// Override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation_ms: Option<u64>,
    /// Override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserve_ms: Option<u64>,
    /// Override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_attempt_ms: Option<u64>,
    /// Override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banter_ms: Option<u64>,
    /// Override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens_bid: Option<u64>,
    /// Override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens_play: Option<u64>,
    /// Override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens_banter: Option<u64>,
}

macro_rules! log_events {
    ($( $(#[$m:meta])* $v:ident => $s:literal ),+ $(,)?) => {
        /// Every AI log event name (`AiLogEvent`), byte-exact.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum AiLogEvent { $( $(#[$m])* $v ),+ }

        impl AiLogEvent {
            /// Every event, in TS order.
            pub const ALL: &'static [AiLogEvent] = &[$(AiLogEvent::$v),+];

            /// The exact event name.
            pub const fn as_str(self) -> &'static str {
                match self { $(AiLogEvent::$v => $s),+ }
            }
        }
    };
}

log_events! {
    /// `ai_no_provider`.
    NoProvider => "ai_no_provider",
    /// `ai_degraded`.
    Degraded => "ai_degraded",
    /// `ai_forced`.
    Forced => "ai_forced",
    /// `ai_illegal_id`.
    IllegalId => "ai_illegal_id",
    /// `ai_unparseable`.
    Unparseable => "ai_unparseable",
    /// `ai_timeout`.
    Timeout => "ai_timeout",
    /// `ai_fatal`.
    Fatal => "ai_fatal",
    /// `ai_error`.
    Error => "ai_error",
    /// `ai_ok`.
    Ok => "ai_ok",
    /// `jev_decision`.
    JevDecision => "jev_decision",
    /// `banter_skipped`.
    BanterSkipped => "banter_skipped",
    /// `banter_screened`.
    BanterScreened => "banter_screened",
    /// `banter_error`.
    BanterError => "banter_error",
    /// `rationale_screened`.
    RationaleScreened => "rationale_screened",
    /// `persona_truncated`.
    PersonaTruncated => "persona_truncated",
}

/// The aiSeat ladder's own log event names (`ai-seat/decide.ts`), byte-exact.
pub mod seat_events {
    /// No model provider is configured.
    pub const AI_NO_PROVIDER: &str = "ai_no_provider";
    /// An attempt could not fit inside the deadline.
    pub const AI_NO_BUDGET: &str = "ai_no_budget";
    /// An attempt failed (unparseable, late, refused).
    pub const AI_ATTEMPT_FAILED: &str = "ai_attempt_failed";
    /// The model named an id outside the candidate set.
    pub const AI_ILLEGAL_ID: &str = "ai_illegal_id";
}

/// `AiLogFn`: structured AI logging. Fields are a JSON object.
pub trait AiLog {
    /// Records one event.
    fn log(&self, event: AiLogEvent, fields: &Map<String, Value>);
}

/// `noopLog`.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopLog;

impl AiLog for NoopLog {
    fn log(&self, _event: AiLogEvent, _fields: &Map<String, Value>) {}
}

/// The aiSeat actor's log sink (`c.log`): free-form event names at three levels.
///
/// The ladder logs at `info` with the names in [`seat_events`]; the handlers log
/// their drop and failure notices at `warn` / `error`.
pub trait SeatLog {
    /// `c.log.info(event, fields)`.
    fn info(&self, event: &str, fields: &Map<String, Value>);
    /// `c.log.warn(message, fields)`; defaults to [`SeatLog::info`].
    fn warn(&self, message: &str, fields: &Map<String, Value>) {
        self.info(message, fields);
    }
    /// `c.log.error(message, fields)`; defaults to [`SeatLog::info`].
    fn error(&self, message: &str, fields: &Map<String, Value>) {
        self.info(message, fields);
    }
}

impl SeatLog for NoopLog {
    fn info(&self, _event: &str, _fields: &Map<String, Value>) {}
}

/// Forwards [`AiLog`] events into a [`SeatLog`] at `info` (the bridge hands the
/// seat's own log to `callModel`).
#[derive(Clone, Copy)]
pub struct SeatAiLog<'a>(pub &'a (dyn SeatLog + Send + Sync));

impl std::fmt::Debug for SeatAiLog<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SeatAiLog")
    }
}

impl AiLog for SeatAiLog<'_> {
    fn log(&self, event: AiLogEvent, fields: &Map<String, Value>) {
        self.0.info(event.as_str(), fields);
    }
}

/// The injected clock (`deps.now`): the only way time enters the AI.
pub trait Clock {
    /// Epoch milliseconds.
    fn now_ms(&self) -> u64;
}

/// One Jev choice question (`JevChoiceQuestion`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JevChoiceQuestion {
    /// Always `"choice"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// What to decide.
    pub instructions: DecisionDescription,
    /// Option id to description, in prompt order.
    pub criteria: Map<String, Value>,
}

/// One Jev choice answer (`JevChoiceAnswer`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JevChoiceAnswer {
    /// Always `"choice"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The chosen option.
    pub choice: String,
    /// `0..=1`.
    pub confidence: f64,
    /// Option id to probability.
    pub probabilities: Map<String, Value>,
}

/// The input of `ModelFactory.decision`.
#[derive(Clone, Debug, PartialEq)]
pub struct JevDecisionInput {
    /// The decision state object.
    pub state: DecisionDescription,
    /// The move instructions.
    pub instructions: DecisionDescription,
    /// The move criteria.
    pub criteria: Map<String, Value>,
    /// Extra questions asked alongside `move`, in order.
    pub additional_questions: Option<Map<String, Value>>,
}

/// The output of `ModelFactory.decision`.
#[derive(Clone, Debug, PartialEq)]
pub struct JevDecisionOutput {
    /// The chosen option.
    pub choice: String,
    /// `0..=1`.
    pub confidence: f64,
    /// Token usage.
    pub usage: DecideUsage,
    /// Option probabilities, when the model returned them.
    pub probabilities: Option<Map<String, Value>>,
    /// Provider request id.
    pub request_id: Option<String>,
    /// Model snapshot name.
    pub model_snapshot: Option<String>,
    /// Answers to the additional questions.
    pub additional_answers: Option<Vec<(String, JevChoiceAnswer)>>,
}
