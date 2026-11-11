//! Conversational streaming (`ModelFactory.talk` + `streamText`): one
//! text-only generation, streamed as server-sent events.
//!
//! Two upstreams: OpenRouter chat completions (the shipped path) and the
//! Anthropic Messages API (the text-only fallback; it has no decision
//! capability). Both are parsed by one byte-level [`SseParser`] that tolerates
//! chunk boundaries anywhere (mid-line, mid-UTF-8, mid-CRLF), `:` keep-alive
//! comments, `[DONE]`, and in-band error events.

use std::collections::VecDeque;
use std::fmt;
use std::pin::Pin;

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::model::ModelParams;
use crate::prompt::SystemLayerMessage;
use crate::types::DecideUsage;

/// The default Anthropic API base.
pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1";

/// The `anthropic-version` header value.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// One streamed text generation.
#[derive(Clone, Debug, PartialEq)]
pub struct TalkRequest {
    /// The upstream model slug (already mapped by `slugFor`).
    pub model: String,
    /// The system layers, in order.
    pub system: Vec<SystemLayerMessage>,
    /// The single user message.
    pub user: String,
    /// `maxOutputTokens`.
    pub max_output_tokens: u64,
    /// Sampling and provider options from [`crate::model::model_params`].
    pub params: ModelParams,
}

/// One item of a talk stream.
#[derive(Clone, Debug, PartialEq)]
pub enum TalkChunk {
    /// A text delta.
    Text(String),
    /// Token usage (the last one wins).
    Usage(DecideUsage),
}

/// A talk failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TalkError {
    /// A non-2xx response (the body is dropped unread).
    #[error("AI_APICallError: HTTP {status}")]
    Http {
        /// The HTTP status.
        status: u16,
    },
    /// An in-band error event or an unparseable event.
    #[error("AI_APICallError: {0}")]
    Stream(String),
    /// The request could not be sent or the body could not be read.
    #[error("TypeError: fetch failed ({0})")]
    Transport(String),
    /// The caller's cancellation fired.
    #[error("AbortError: This operation was aborted")]
    Aborted,
}

/// A boxed stream of talk chunks.
pub type TalkStream = Pin<Box<dyn Stream<Item = Result<TalkChunk, TalkError>> + Send>>;

/// `ModelFactory.talk`: a text-only model. Never a decision path.
#[async_trait]
pub trait TalkClient: Send + Sync {
    /// Starts one streamed generation. Cancelling `cancel` ends the stream with
    /// [`TalkError::Aborted`] and drops the connection.
    async fn stream(
        &self,
        request: TalkRequest,
        cancel: CancellationToken,
    ) -> Result<TalkStream, TalkError>;
}

/* ========================================================================== */
/* SSE                                                                        */
/* ========================================================================== */

/// One dispatched server-sent event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` field, when present.
    pub event: Option<String>,
    /// The `data:` lines joined by `\n`.
    pub data: String,
}

/// An incremental SSE parser over raw bytes.
///
/// Lines end at `\n` (a preceding `\r` is dropped); a blank line dispatches.
/// Lines starting `:` are comments. At [`SseParser::finish`] a pending line and
/// a pending event are flushed rather than discarded.
#[derive(Clone, Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    /// A fresh parser.
    pub fn new() -> SseParser {
        SseParser::default()
    }

    /// Feeds bytes; returns every event completed by them.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.line(&line, &mut out);
        }
        out
    }

    /// Ends the input: flushes a trailing unterminated line and any pending event.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let mut line = std::mem::take(&mut self.buf);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.line(&line, &mut out);
        }
        self.dispatch(&mut out);
        out
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        let event = self.event.take();
        if self.data.is_empty() {
            return;
        }
        let data = std::mem::take(&mut self.data).join("\n");
        out.push(SseEvent { event, data });
    }

    fn line(&mut self, line: &[u8], out: &mut Vec<SseEvent>) {
        if line.is_empty() {
            self.dispatch(out);
            return;
        }
        if line.first() == Some(&b':') {
            return;
        }
        let text = String::from_utf8_lossy(line);
        let (field, value) = match text.find(':') {
            Some(i) => {
                let v = &text[i + 1..];
                (&text[..i], v.strip_prefix(' ').unwrap_or(v))
            }
            None => (text.as_ref(), ""),
        };
        match field {
            "data" => self.data.push(value.to_owned()),
            "event" => self.event = Some(value.to_owned()),
            _ => {}
        }
    }
}

/// Turns SSE events into talk chunks for one upstream.
pub trait SseDecoder: Send {
    /// Handles one event; returns `true` when the stream is finished.
    fn on_event(
        &mut self,
        event: SseEvent,
        out: &mut VecDeque<Result<TalkChunk, TalkError>>,
    ) -> bool;
    /// The body ended without a terminal event.
    fn on_eof(&mut self, out: &mut VecDeque<Result<TalkChunk, TalkError>>);
}

fn u64_at(v: &Value, path: &[&str]) -> Option<u64> {
    let mut cur = v;
    for k in path {
        cur = cur.get(k)?;
    }
    cur.as_u64()
}

fn error_message(v: &Value) -> String {
    v.get("error")
        .and_then(|e| {
            e.get("message")
                .and_then(Value::as_str)
                .or_else(|| e.as_str())
        })
        .unwrap_or("stream error")
        .to_owned()
}

/// OpenRouter chat-completions stream decoding.
#[derive(Clone, Debug, Default)]
pub struct OpenRouterDecoder;

impl SseDecoder for OpenRouterDecoder {
    fn on_event(
        &mut self,
        event: SseEvent,
        out: &mut VecDeque<Result<TalkChunk, TalkError>>,
    ) -> bool {
        let data = event.data.trim();
        if data == "[DONE]" {
            return true;
        }
        if data.is_empty() {
            return false;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            out.push_back(Err(TalkError::Stream("invalid JSON chunk".to_owned())));
            return true;
        };
        if v.get("error").is_some() {
            out.push_back(Err(TalkError::Stream(error_message(&v))));
            return true;
        }
        if let Some(text) = v
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("delta"))
            .and_then(|d| d.get("content"))
            .and_then(Value::as_str)
            && !text.is_empty()
        {
            out.push_back(Ok(TalkChunk::Text(text.to_owned())));
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            out.push_back(Ok(TalkChunk::Usage(DecideUsage {
                input_tokens: u64_at(u, &["prompt_tokens"]).unwrap_or(0),
                output_tokens: u64_at(u, &["completion_tokens"]).unwrap_or(0),
                cache_read_tokens: u64_at(u, &["prompt_tokens_details", "cached_tokens"])
                    .unwrap_or(0),
                cache_write_tokens: u64_at(u, &["prompt_tokens_details", "cache_write_tokens"])
                    .unwrap_or(0),
            })));
        }
        false
    }

    fn on_eof(&mut self, _out: &mut VecDeque<Result<TalkChunk, TalkError>>) {}
}

/// Anthropic Messages stream decoding.
#[derive(Clone, Debug, Default)]
pub struct AnthropicDecoder {
    usage: DecideUsage,
    seen_usage: bool,
}

impl AnthropicDecoder {
    fn flush_usage(&mut self, out: &mut VecDeque<Result<TalkChunk, TalkError>>) {
        if self.seen_usage {
            out.push_back(Ok(TalkChunk::Usage(self.usage)));
            self.seen_usage = false;
        }
    }
}

impl SseDecoder for AnthropicDecoder {
    fn on_event(
        &mut self,
        event: SseEvent,
        out: &mut VecDeque<Result<TalkChunk, TalkError>>,
    ) -> bool {
        let Ok(v) = serde_json::from_str::<Value>(&event.data) else {
            out.push_back(Err(TalkError::Stream("invalid JSON chunk".to_owned())));
            return true;
        };
        let kind = v
            .get("type")
            .and_then(Value::as_str)
            .or(event.event.as_deref())
            .unwrap_or("");
        match kind {
            "message_start" => {
                if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                    self.usage.input_tokens = u64_at(u, &["input_tokens"]).unwrap_or(0);
                    self.usage.output_tokens = u64_at(u, &["output_tokens"]).unwrap_or(0);
                    self.usage.cache_read_tokens =
                        u64_at(u, &["cache_read_input_tokens"]).unwrap_or(0);
                    self.usage.cache_write_tokens =
                        u64_at(u, &["cache_creation_input_tokens"]).unwrap_or(0);
                    self.seen_usage = true;
                }
                false
            }
            "content_block_delta" => {
                let delta = v.get("delta");
                let is_text = delta
                    .and_then(|d| d.get("type"))
                    .and_then(Value::as_str)
                    .is_some_and(|t| t == "text_delta");
                if is_text
                    && let Some(text) = delta.and_then(|d| d.get("text")).and_then(Value::as_str)
                    && !text.is_empty()
                {
                    out.push_back(Ok(TalkChunk::Text(text.to_owned())));
                }
                false
            }
            "message_delta" => {
                if let Some(n) = v.get("usage").and_then(|u| u64_at(u, &["output_tokens"])) {
                    self.usage.output_tokens = n;
                    self.seen_usage = true;
                }
                false
            }
            "message_stop" => {
                self.flush_usage(out);
                true
            }
            "error" => {
                out.push_back(Err(TalkError::Stream(error_message(&v))));
                true
            }
            _ => false,
        }
    }

    fn on_eof(&mut self, out: &mut VecDeque<Result<TalkChunk, TalkError>>) {
        self.flush_usage(out);
    }
}

/// Decodes a complete byte sequence delivered in arbitrary pieces (the
/// chunk-boundary test seam and the core of the live stream).
pub fn decode_sse<D: SseDecoder>(
    decoder: &mut D,
    pieces: &[&[u8]],
) -> Vec<Result<TalkChunk, TalkError>> {
    let mut parser = SseParser::new();
    let mut out = VecDeque::new();
    for piece in pieces {
        for ev in parser.push(piece) {
            if decoder.on_event(ev, &mut out) {
                return out.into_iter().collect();
            }
        }
    }
    for ev in parser.finish() {
        if decoder.on_event(ev, &mut out) {
            return out.into_iter().collect();
        }
    }
    decoder.on_eof(&mut out);
    out.into_iter().collect()
}

type ByteStream = Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>;

struct SseState<D> {
    body: ByteStream,
    parser: SseParser,
    decoder: D,
    pending: VecDeque<Result<TalkChunk, TalkError>>,
    done: bool,
    cancel: CancellationToken,
}

impl<D: SseDecoder> SseState<D> {
    fn feed(&mut self, events: Vec<SseEvent>) {
        for ev in events {
            if self.done {
                return;
            }
            if self.decoder.on_event(ev, &mut self.pending) {
                self.done = true;
            }
        }
    }
}

/// Wraps a response body as a [`TalkStream`].
fn sse_stream<D: SseDecoder + 'static>(
    response: reqwest::Response,
    decoder: D,
    cancel: CancellationToken,
) -> TalkStream {
    let state = SseState {
        body: Box::pin(response.bytes_stream()),
        parser: SseParser::new(),
        decoder,
        pending: VecDeque::new(),
        done: false,
        cancel,
    };
    Box::pin(futures::stream::unfold(state, |mut st| async move {
        loop {
            if let Some(item) = st.pending.pop_front() {
                if item.is_err() {
                    st.done = true;
                    st.pending.clear();
                }
                return Some((item, st));
            }
            if st.done {
                return None;
            }
            let next = tokio::select! {
                biased;
                () = st.cancel.cancelled() => {
                    st.done = true;
                    return Some((Err(TalkError::Aborted), st));
                }
                n = st.body.next() => n,
            };
            match next {
                Some(Ok(bytes)) => {
                    let events = st.parser.push(&bytes);
                    st.feed(events);
                }
                Some(Err(_)) => {
                    st.done = true;
                    return Some((Err(TalkError::Transport("body".to_owned())), st));
                }
                None => {
                    let events = st.parser.finish();
                    st.feed(events);
                    if !st.done {
                        st.decoder.on_eof(&mut st.pending);
                        st.done = true;
                    }
                }
            }
        }
    }))
}

async fn send_streaming(
    request: reqwest::RequestBuilder,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, TalkError> {
    if cancel.is_cancelled() {
        return Err(TalkError::Aborted);
    }
    let response = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(TalkError::Aborted),
        r = request.send() => r.map_err(|e| TalkError::Transport(if e.is_connect() { "connect" } else { "request" }.to_owned()))?,
    };
    let status = response.status();
    if !status.is_success() {
        drop(response);
        return Err(TalkError::Http {
            status: status.as_u16(),
        });
    }
    Ok(response)
}

fn strip_slash(s: &str) -> &str {
    s.strip_suffix('/').unwrap_or(s)
}

/* ========================================================================== */
/* OpenRouter                                                                 */
/* ========================================================================== */

/// OpenRouter chat completions, streamed.
#[derive(Clone)]
pub struct OpenRouterTalk {
    api_key: String,
    base_url: String,
    http: reqwest::Client,
}

impl fmt::Debug for OpenRouterTalk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenRouterTalk")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl OpenRouterTalk {
    /// A client for `{base_url}/chat/completions` (default
    /// `https://openrouter.ai/api/v1`).
    pub fn new(api_key: String, base_url: Option<String>, http: Option<reqwest::Client>) -> Self {
        OpenRouterTalk {
            api_key,
            base_url: strip_slash(
                base_url
                    .as_deref()
                    .unwrap_or(crate::jev::DEFAULT_OPENROUTER_BASE_URL),
            )
            .to_owned(),
            http: http.unwrap_or_default(),
        }
    }

    /// The request URL.
    pub fn url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

/// The OpenRouter request body for one [`TalkRequest`].
pub fn openrouter_body(request: &TalkRequest) -> Value {
    let mut messages: Vec<Value> = request
        .system
        .iter()
        .map(|m| {
            // `@openrouter/ai-sdk-provider` sends every system message as a
            // one-part text array, with the `providerOptions.anthropic.cacheControl`
            // that `toInstructions` sets mapped to `cache_control`.
            let part = if m.cache {
                json!({
                    "type": "text",
                    "text": m.content,
                    "cache_control": { "type": "ephemeral", "ttl": "1h" },
                })
            } else {
                json!({ "type": "text", "text": m.content })
            };
            json!({ "role": "system", "content": [part] })
        })
        .collect();
    messages.push(json!({ "role": "user", "content": request.user }));
    let mut body = Map::new();
    body.insert("model".to_owned(), json!(request.model));
    body.insert("messages".to_owned(), Value::Array(messages));
    body.insert("max_tokens".to_owned(), json!(request.max_output_tokens));
    if let Some(t) = request.params.temperature {
        body.insert("temperature".to_owned(), crate::js::num(t));
    }
    if let Some(Value::Object(or)) = request.params.provider_options.get("openrouter") {
        for (k, v) in or {
            body.insert(k.clone(), v.clone());
        }
    }
    // `createOpenRouter` defaults to `compatibility: 'compatible'`, which sends
    // no `stream_options`; OpenRouter reports usage in the final chunk anyway.
    body.insert("stream".to_owned(), Value::Bool(true));
    Value::Object(body)
}

#[async_trait]
impl TalkClient for OpenRouterTalk {
    async fn stream(
        &self,
        request: TalkRequest,
        cancel: CancellationToken,
    ) -> Result<TalkStream, TalkError> {
        let body = crate::js::stringify(&openrouter_body(&request));
        let builder = self
            .http
            .post(self.url())
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .body(body);
        let response = send_streaming(builder, &cancel).await?;
        Ok(sse_stream(response, OpenRouterDecoder, cancel))
    }
}

/* ========================================================================== */
/* Anthropic (text-only fallback)                                             */
/* ========================================================================== */

/// The Anthropic Messages API, streamed. Text only: it has no decision capability.
#[derive(Clone)]
pub struct AnthropicTalk {
    api_key: String,
    base_url: String,
    http: reqwest::Client,
}

impl fmt::Debug for AnthropicTalk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicTalk")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl AnthropicTalk {
    /// A client for `{base_url}/messages` (default [`DEFAULT_ANTHROPIC_BASE_URL`]).
    pub fn new(api_key: String, base_url: Option<String>, http: Option<reqwest::Client>) -> Self {
        AnthropicTalk {
            api_key,
            base_url: strip_slash(base_url.as_deref().unwrap_or(DEFAULT_ANTHROPIC_BASE_URL))
                .to_owned(),
            http: http.unwrap_or_default(),
        }
    }

    /// The request URL.
    pub fn url(&self) -> String {
        format!("{}/messages", self.base_url)
    }
}

/// The Anthropic request body for one [`TalkRequest`]. Cached system layers get
/// `cache_control: { type: "ephemeral", ttl: "1h" }`; `providerOptions.anthropic`
/// maps `thinking` through and `effort` to `output_config.effort`.
pub fn anthropic_body(request: &TalkRequest) -> Value {
    let system: Vec<Value> = request
        .system
        .iter()
        .map(|m| {
            if m.cache {
                json!({
                    "type": "text",
                    "text": m.content,
                    "cache_control": { "type": "ephemeral", "ttl": "1h" },
                })
            } else {
                json!({ "type": "text", "text": m.content })
            }
        })
        .collect();
    let mut body = Map::new();
    body.insert("model".to_owned(), json!(request.model));
    body.insert("max_tokens".to_owned(), json!(request.max_output_tokens));
    body.insert("system".to_owned(), Value::Array(system));
    body.insert(
        "messages".to_owned(),
        json!([{ "role": "user", "content": request.user }]),
    );
    if let Some(t) = request.params.temperature {
        body.insert("temperature".to_owned(), crate::js::num(t));
    }
    if let Some(Value::Object(a)) = request.params.provider_options.get("anthropic") {
        if let Some(thinking) = a.get("thinking") {
            body.insert("thinking".to_owned(), thinking.clone());
        }
        if let Some(effort) = a.get("effort") {
            body.insert("output_config".to_owned(), json!({ "effort": effort }));
        }
    }
    body.insert("stream".to_owned(), Value::Bool(true));
    Value::Object(body)
}

#[async_trait]
impl TalkClient for AnthropicTalk {
    async fn stream(
        &self,
        request: TalkRequest,
        cancel: CancellationToken,
    ) -> Result<TalkStream, TalkError> {
        let body = crate::js::stringify(&anthropic_body(&request));
        let builder = self
            .http
            .post(self.url())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .body(body);
        let response = send_streaming(builder, &cancel).await?;
        Ok(sse_stream(response, AnthropicDecoder::default(), cancel))
    }
}
