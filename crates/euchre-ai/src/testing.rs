//! The deterministic test seam (`ai/testing.ts`). No production path uses it.
//!
//! Scripted and hanging stand-ins for every network trait, a recording log, a
//! manual clock, and the factory helpers. Nothing here opens a socket or reads
//! an API key.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::ai_seat::bridge::{
    LlmBanterInput, LlmBanterWriter, LlmDecideInput, LlmDecideResult, LlmDecider,
};
use crate::decide::CallModelError;
use crate::jev::{JevClient, JevError};
use crate::model::{ModelFactory, ProviderName, SlugMap};
use crate::talk::{TalkChunk, TalkClient, TalkError, TalkRequest, TalkStream};
use crate::types::{
    AiLog, AiLogEvent, Clock, DecideUsage, JevDecisionInput, JevDecisionOutput, SeatLog,
};

/// `NO_PROVIDER`: a factory-free deps fragment.
pub const NO_PROVIDER: Option<&ModelFactory> = None;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/* -------------------------------------------------------------------------- */
/* Logs and clocks                                                            */
/* -------------------------------------------------------------------------- */

/// One recorded log line.
#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    /// `info`, `warn` or `error`.
    pub level: &'static str,
    /// The event name.
    pub event: String,
    /// The fields.
    pub fields: Map<String, Value>,
}

/// Records every [`AiLog`] and [`SeatLog`] line.
#[derive(Debug, Default)]
pub struct RecordingLog {
    lines: Mutex<Vec<LogLine>>,
}

impl RecordingLog {
    /// An empty log.
    pub fn new() -> RecordingLog {
        RecordingLog::default()
    }

    /// Every line so far.
    pub fn lines(&self) -> Vec<LogLine> {
        lock(&self.lines).clone()
    }

    /// Every event name so far.
    pub fn events(&self) -> Vec<String> {
        lock(&self.lines).iter().map(|l| l.event.clone()).collect()
    }

    /// The fields of the first line named `event`.
    pub fn find(&self, event: &str) -> Option<Map<String, Value>> {
        lock(&self.lines)
            .iter()
            .find(|l| l.event == event)
            .map(|l| l.fields.clone())
    }

    fn push(&self, level: &'static str, event: &str, fields: &Map<String, Value>) {
        lock(&self.lines).push(LogLine {
            level,
            event: event.to_owned(),
            fields: fields.clone(),
        });
    }
}

impl AiLog for RecordingLog {
    fn log(&self, event: AiLogEvent, fields: &Map<String, Value>) {
        self.push("info", event.as_str(), fields);
    }
}

impl SeatLog for RecordingLog {
    fn info(&self, event: &str, fields: &Map<String, Value>) {
        self.push("info", event, fields);
    }
    fn warn(&self, message: &str, fields: &Map<String, Value>) {
        self.push("warn", message, fields);
    }
    fn error(&self, message: &str, fields: &Map<String, Value>) {
        self.push("error", message, fields);
    }
}

/// A clock that moves only when told to.
#[derive(Debug, Default)]
pub struct ManualClock {
    now: AtomicU64,
}

impl ManualClock {
    /// A clock reading `now_ms`.
    pub fn new(now_ms: u64) -> ManualClock {
        ManualClock {
            now: AtomicU64::new(now_ms),
        }
    }

    /// Sets the time.
    pub fn set(&self, now_ms: u64) {
        self.now.store(now_ms, Ordering::SeqCst);
    }

    /// Advances the time.
    pub fn advance(&self, ms: u64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

/* -------------------------------------------------------------------------- */
/* Jev                                                                        */
/* -------------------------------------------------------------------------- */

/// A Jev client that returns each scripted result in turn (repeating the last).
#[derive(Debug)]
pub struct ScriptedJev {
    script: Vec<Result<JevDecisionOutput, JevError>>,
    next: AtomicUsize,
    calls: Mutex<Vec<JevDecisionInput>>,
    delay: Option<Duration>,
}

impl ScriptedJev {
    /// A scripted client. An empty script answers [`JevError::Transport`].
    pub fn new(script: Vec<Result<JevDecisionOutput, JevError>>) -> ScriptedJev {
        ScriptedJev {
            script,
            next: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
            delay: None,
        }
    }

    /// Waits `delay` (tokio time) before answering.
    pub fn with_delay(mut self, delay: Duration) -> ScriptedJev {
        self.delay = Some(delay);
        self
    }

    /// Every input received.
    pub fn calls(&self) -> Vec<JevDecisionInput> {
        lock(&self.calls).clone()
    }
}

#[async_trait]
impl JevClient for ScriptedJev {
    async fn decide(
        &self,
        input: JevDecisionInput,
        cancel: CancellationToken,
    ) -> Result<JevDecisionOutput, JevError> {
        if cancel.is_cancelled() {
            return Err(JevError::Aborted);
        }
        lock(&self.calls).push(input);
        if let Some(d) = self.delay {
            tokio::select! {
                () = cancel.cancelled() => return Err(JevError::Aborted),
                () = tokio::time::sleep(d) => {}
            }
        }
        let i = self.next.fetch_add(1, Ordering::SeqCst);
        let last = self.script.len().saturating_sub(1);
        self.script
            .get(i.min(last))
            .cloned()
            .unwrap_or_else(|| Err(JevError::Transport("empty script".to_owned())))
    }
}

/// A Jev client that never answers until cancelled (`hangingModel`).
#[derive(Debug, Default)]
pub struct HangingJev {
    calls: AtomicUsize,
}

impl HangingJev {
    /// A hanging client.
    pub fn new() -> HangingJev {
        HangingJev::default()
    }

    /// Calls received.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl JevClient for HangingJev {
    async fn decide(
        &self,
        _input: JevDecisionInput,
        cancel: CancellationToken,
    ) -> Result<JevDecisionOutput, JevError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        cancel.cancelled().await;
        Err(JevError::Aborted)
    }
}

/// A `JevDecisionOutput` with just a choice (`{ choice, confidence, usage }`).
pub fn jev_choice(choice: &str, confidence: f64) -> JevDecisionOutput {
    JevDecisionOutput {
        choice: choice.to_owned(),
        confidence,
        usage: DecideUsage {
            input_tokens: 10,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        },
        probabilities: None,
        request_id: None,
        model_snapshot: None,
        additional_answers: None,
    }
}

/* -------------------------------------------------------------------------- */
/* Talk                                                                       */
/* -------------------------------------------------------------------------- */

/// One scripted generation (`ScriptedResult`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptedResult {
    /// The text, delivered as these deltas.
    pub deltas: Vec<String>,
    /// Prompt tokens (default 100).
    pub input_tokens: u64,
    /// Completion tokens (default 20).
    pub output_tokens: u64,
    /// Cache reads.
    pub cache_read_tokens: u64,
    /// Cache writes.
    pub cache_write_tokens: u64,
}

impl ScriptedResult {
    /// One delta with the default usage (100 in, 20 out).
    pub fn text(text: &str) -> ScriptedResult {
        ScriptedResult::deltas(&[text])
    }

    /// Several deltas with the default usage.
    pub fn deltas(deltas: &[&str]) -> ScriptedResult {
        ScriptedResult {
            deltas: deltas.iter().map(|d| (*d).to_owned()).collect(),
            input_tokens: 100,
            output_tokens: 20,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }
    }
}

/// A talk model returning each scripted result in turn (`scriptedModel`).
#[derive(Debug)]
pub struct ScriptedTalk {
    script: Vec<Result<ScriptedResult, TalkError>>,
    next: AtomicUsize,
    calls: Mutex<Vec<TalkRequest>>,
}

impl ScriptedTalk {
    /// A scripted model (repeats the last entry).
    pub fn new(script: Vec<Result<ScriptedResult, TalkError>>) -> ScriptedTalk {
        ScriptedTalk {
            script,
            next: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Every request received.
    pub fn calls(&self) -> Vec<TalkRequest> {
        lock(&self.calls).clone()
    }
}

#[async_trait]
impl TalkClient for ScriptedTalk {
    async fn stream(
        &self,
        request: TalkRequest,
        _cancel: CancellationToken,
    ) -> Result<TalkStream, TalkError> {
        lock(&self.calls).push(request);
        let i = self.next.fetch_add(1, Ordering::SeqCst);
        let last = self.script.len().saturating_sub(1);
        let r = self
            .script
            .get(i.min(last))
            .cloned()
            .unwrap_or_else(|| Err(TalkError::Stream("empty script".to_owned())))?;
        let mut items: VecDeque<Result<TalkChunk, TalkError>> = r
            .deltas
            .into_iter()
            .map(|d| Ok(TalkChunk::Text(d)))
            .collect();
        items.push_back(Ok(TalkChunk::Usage(DecideUsage {
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            cache_read_tokens: r.cache_read_tokens,
            cache_write_tokens: r.cache_write_tokens,
        })));
        Ok(Box::pin(futures::stream::iter(items)))
    }
}

/// A talk model whose stream never yields until cancelled.
#[derive(Debug, Default)]
pub struct HangingTalk;

#[async_trait]
impl TalkClient for HangingTalk {
    async fn stream(
        &self,
        _request: TalkRequest,
        cancel: CancellationToken,
    ) -> Result<TalkStream, TalkError> {
        Ok(Box::pin(futures::stream::once(async move {
            cancel.cancelled().await;
            Err(TalkError::Aborted)
        })))
    }
}

/// `tieredFactory(fast, deliberate)`: talk always uses `fast` (as in TS),
/// identity slugs, no decision capability.
pub fn tiered_factory(fast: Arc<dyn TalkClient>, _deliberate: Arc<dyn TalkClient>) -> ModelFactory {
    ModelFactory {
        provider: ProviderName::OpenRouter,
        decision: None,
        talk: fast,
        slugs: SlugMap::Identity,
    }
}

/// A factory with a Jev client and a talk model (identity slugs, OpenRouter).
pub fn jev_factory(decision: Arc<dyn JevClient>, talk: Arc<dyn TalkClient>) -> ModelFactory {
    ModelFactory {
        provider: ProviderName::OpenRouter,
        decision: Some(decision),
        talk,
        slugs: SlugMap::Identity,
    }
}

/* -------------------------------------------------------------------------- */
/* Seat bridge                                                                */
/* -------------------------------------------------------------------------- */

type DecideFn =
    dyn Fn(&LlmDecideInput, usize) -> Result<LlmDecideResult, CallModelError> + Send + Sync;

/// A decider driven by a closure (`setLlmProvider({ decide })`).
pub struct ScriptedDecider {
    f: Box<DecideFn>,
    calls: Mutex<Vec<LlmDecideInput>>,
    delay: Option<Duration>,
}

impl std::fmt::Debug for ScriptedDecider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedDecider").finish_non_exhaustive()
    }
}

impl ScriptedDecider {
    /// `f(input, callIndex)` answers each call.
    pub fn new(
        f: impl Fn(&LlmDecideInput, usize) -> Result<LlmDecideResult, CallModelError>
        + Send
        + Sync
        + 'static,
    ) -> ScriptedDecider {
        ScriptedDecider {
            f: Box::new(f),
            calls: Mutex::new(Vec::new()),
            delay: None,
        }
    }

    /// Always answers `move_id` (no rationale, no usage).
    pub fn always(move_id: &str) -> ScriptedDecider {
        let id = move_id.to_owned();
        ScriptedDecider::new(move |_, _| Ok(decide_result(&id)))
    }

    /// Waits `delay` (tokio time) before answering.
    pub fn with_delay(mut self, delay: Duration) -> ScriptedDecider {
        self.delay = Some(delay);
        self
    }

    /// Every input received.
    pub fn calls(&self) -> Vec<LlmDecideInput> {
        lock(&self.calls).clone()
    }
}

/// `{ moveId }`.
pub fn decide_result(move_id: &str) -> LlmDecideResult {
    LlmDecideResult {
        move_id: move_id.to_owned(),
        rationale: None,
        confidence: None,
        usage: None,
    }
}

#[async_trait]
impl LlmDecider for ScriptedDecider {
    async fn decide(
        &self,
        input: &LlmDecideInput,
        _log: &(dyn SeatLog + Send + Sync),
    ) -> Result<LlmDecideResult, CallModelError> {
        let index = {
            let mut calls = lock(&self.calls);
            calls.push(input.clone());
            calls.len() - 1
        };
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        (self.f)(input, index)
    }
}

/// A banter writer returning a fixed line (or `None`), optionally after a delay.
#[derive(Debug, Default)]
pub struct ScriptedBanter {
    /// The line.
    pub line: Option<String>,
    /// Tokio-time delay before answering.
    pub delay: Option<Duration>,
    calls: Mutex<Vec<LlmBanterInput>>,
}

impl ScriptedBanter {
    /// A writer answering `line`.
    pub fn new(line: Option<&str>) -> ScriptedBanter {
        ScriptedBanter {
            line: line.map(str::to_owned),
            delay: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Every input received.
    pub fn calls(&self) -> Vec<LlmBanterInput> {
        lock(&self.calls).clone()
    }
}

#[async_trait]
impl LlmBanterWriter for ScriptedBanter {
    async fn write(&self, input: &LlmBanterInput) -> Option<String> {
        lock(&self.calls).push(input.clone());
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        self.line.clone()
    }
}
