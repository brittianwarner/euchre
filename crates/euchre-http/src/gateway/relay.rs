//! The WebSocket relay between the browser leg (axum) and the engine leg
//! (`tokio-tungstenite`), TS `websocket: {open, message, close, drain}`.
//!
//! - Frames are relayed in order, text and binary kept apart; pings and pongs are
//!   answered per leg and never relayed.
//! - Both relay queues share one byte budget (`max_buffered_bytes`). A frame that
//!   would overshoot it closes both legs with `1013 backpressure limit`.
//! - Close codes and reasons are mirrored both ways; a code that cannot be sent in a
//!   close frame becomes `1011` toward the browser and `1001` toward the engine.
//! - The browser leg is pinged every `ping_interval` and closed after
//!   `idle_timeout` of silence.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::extract::ws::{self as axws, WebSocket};
use bytes::Bytes;
use futures::{Sink, SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{self as ts};

use super::{cap_ws_close_reason, normalized_close_code, redact_path};
use crate::config::GatewayOptions;

/// How long a leg gets to accept a close frame or flush before it is dropped.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// Where and how to dial the engine leg.
#[derive(Debug, Clone)]
pub struct RelayTarget {
    /// `ws://engine:6420/gateway/...?...`.
    pub url: String,
    /// The browser's offered subprotocols, forwarded verbatim (the engine reads the
    /// actor target, encoding, token and conn params from them).
    pub protocols: Vec<String>,
    /// The browser's Origin, forwarded so the engine sees the same caller.
    pub origin: Option<String>,
    /// The trusted client IP, sent as `X-Forwarded-For` (the Guard's bucket key).
    pub client_ip: Option<String>,
}

#[derive(Debug)]
enum Data {
    Text(String),
    Binary(Bytes),
}

impl Data {
    fn len(&self) -> usize {
        match self {
            Self::Text(t) => t.len(),
            Self::Binary(b) => b.len(),
        }
    }
}

#[derive(Debug)]
enum Out {
    Data(Data),
    Ping,
    Close(u16, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leg {
    Browser,
    Upstream,
}

/// A close of both legs at once (TS `closeBoth`).
#[derive(Debug, Clone)]
struct Abort {
    code: u16,
    reason: String,
}

fn to_axum(out: Out) -> axws::Message {
    match out {
        Out::Data(Data::Text(t)) => axws::Message::Text(t.into()),
        Out::Data(Data::Binary(b)) => axws::Message::Binary(b),
        Out::Ping => axws::Message::Ping(Bytes::new()),
        Out::Close(code, reason) => axws::Message::Close(Some(axws::CloseFrame {
            code,
            reason: reason.into(),
        })),
    }
}

fn to_tungstenite(out: Out) -> ts::Message {
    match out {
        Out::Data(Data::Text(t)) => ts::Message::Text(t.into()),
        Out::Data(Data::Binary(b)) => ts::Message::Binary(b),
        Out::Ping => ts::Message::Ping(Bytes::new()),
        Out::Close(code, reason) => ts::Message::Close(Some(CloseFrame {
            code: CloseCode::from(code),
            reason: reason.into(),
        })),
    }
}

/// Why the engine leg could not be opened.
enum DialFailure {
    Timeout,
    /// The request could not even be built (TS `new WebSocket` threw).
    Threw(String),
    /// The dial ran and failed (refused, TLS, or a non-101 answer such as a Guard
    /// 429): TS saw this as the upstream `error` event.
    Error(String),
}

type Upstream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A tungstenite error reduced to text that can carry no credential: HTTP answers
/// keep only their status (the request URL never appears).
fn describe_ts_error(err: &ts::Error) -> String {
    match err {
        ts::Error::Http(res) => format!("Unexpected server response: {}", res.status().as_u16()),
        ts::Error::Io(io) => io.kind().to_string(),
        ts::Error::Url(_) => "invalid url".to_owned(),
        other => other.to_string(),
    }
}

async fn dial(target: &RelayTarget, opts: &GatewayOptions) -> Result<Upstream, DialFailure> {
    let mut request = target
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| DialFailure::Threw(describe_ts_error(&e)))?;
    let headers = request.headers_mut();
    if !target.protocols.is_empty() {
        let value = HeaderValue::from_str(&target.protocols.join(", "))
            .map_err(|_| DialFailure::Threw("invalid subprotocol header".to_owned()))?;
        headers.insert("sec-websocket-protocol", value);
    }
    if let Some(origin) = target
        .origin
        .as_deref()
        .and_then(|o| HeaderValue::from_str(o).ok())
    {
        headers.insert("origin", origin);
    }
    if let Some(ip) = target
        .client_ip
        .as_deref()
        .and_then(|ip| HeaderValue::from_str(ip).ok())
    {
        headers.insert("x-forwarded-for", ip);
    }
    let config = WebSocketConfig::default()
        .max_message_size(Some(opts.max_payload_bytes))
        .max_frame_size(Some(opts.max_payload_bytes));
    let connect = tokio_tungstenite::connect_async_with_config(request, Some(config), false);
    match tokio::time::timeout(opts.upstream_dial_timeout, connect).await {
        Err(_) => Err(DialFailure::Timeout),
        Ok(Err(err)) => Err(DialFailure::Error(describe_ts_error(&err))),
        Ok(Ok((stream, _response))) => Ok(stream),
    }
}

/// Close a browser leg that never got an engine leg.
async fn close_browser(mut browser: WebSocket, code: u16, reason: &str) {
    let frame = to_axum(Out::Close(code, cap_ws_close_reason(reason)));
    let _ = tokio::time::timeout(CLOSE_GRACE, browser.send(frame)).await;
}

/// The shared byte budget of both relay queues.
#[derive(Debug, Default)]
struct Budget {
    queued: AtomicUsize,
}

impl Budget {
    fn try_take(&self, bytes: usize, max: usize) -> bool {
        let mut current = self.queued.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_add(bytes).filter(|n| *n <= max) else {
                return false;
            };
            match self.queued.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    fn release(&self, bytes: usize) {
        self.queued.fetch_sub(bytes, Ordering::AcqRel);
    }
}

/// Wait for a both-legs close decision. A dropped sender (the relay finished)
/// counts as one, with no frame to send.
async fn aborted(abort: &mut watch::Receiver<Option<Abort>>) {
    let _ = abort.wait_for(Option::is_some).await;
}

struct Writer<S, M> {
    sink: S,
    rx: mpsc::UnboundedReceiver<Out>,
    abort: watch::Receiver<Option<Abort>>,
    budget: Arc<Budget>,
    convert: fn(Out) -> M,
    failed: mpsc::UnboundedSender<Leg>,
    leg: Leg,
}

impl<S, M> Writer<S, M>
where
    S: Sink<M> + Unpin + Send + 'static,
    M: Send + 'static,
{
    fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(self.run())
    }

    async fn run(mut self) {
        loop {
            let out = tokio::select! {
                biased;
                () = aborted(&mut self.abort) => break,
                out = self.rx.recv() => out,
            };
            let Some(out) = out else {
                // The relay is done with this leg: flush (including a queued close
                // reply) and shut it.
                let _ = tokio::time::timeout(CLOSE_GRACE, self.sink.close()).await;
                return;
            };
            let bytes = match &out {
                Out::Data(d) => d.len(),
                Out::Ping | Out::Close(..) => 0,
            };
            let is_close = matches!(out, Out::Close(..));
            let message = (self.convert)(out);
            let sent = if is_close {
                tokio::time::timeout(CLOSE_GRACE, self.sink.send(message))
                    .await
                    .map_err(|_| ())
                    .and_then(|r| r.map_err(|_| ()))
            } else {
                tokio::select! {
                    biased;
                    () = aborted(&mut self.abort) => {
                        self.budget.release(bytes);
                        break;
                    }
                    r = self.sink.send(message) => r.map_err(|_| ()),
                }
            };
            self.budget.release(bytes);
            if is_close {
                return;
            }
            if sent.is_err() {
                let _ = self.failed.send(self.leg);
                return;
            }
        }
        let decision = self.abort.borrow().clone();
        if let Some(Abort { code, reason }) = decision {
            let frame = (self.convert)(Out::Close(code, reason));
            let _ = tokio::time::timeout(CLOSE_GRACE, self.sink.send(frame)).await;
        }
        let _ = tokio::time::timeout(CLOSE_GRACE, self.sink.close()).await;
    }
}

/// Relay one upgraded browser socket to the engine until either side closes.
pub async fn relay(browser: WebSocket, target: RelayTarget, opts: GatewayOptions) {
    let path = redact_path(
        target
            .url
            .split_once("/gateway")
            .map_or("", |(_, rest)| rest),
    );
    let upstream = match dial(&target, &opts).await {
        Ok(upstream) => upstream,
        Err(DialFailure::Timeout) => {
            tracing::warn!(
                path = %path,
                timeout_ms = opts.upstream_dial_timeout.as_millis() as u64,
                "rivet proxy: upstream dial timeout"
            );
            close_browser(browser, 1011, "upstream dial timeout").await;
            return;
        }
        Err(DialFailure::Threw(error)) => {
            tracing::error!(path = %path, error = %error, "rivet proxy: upstream dial threw");
            close_browser(browser, 1011, "upstream dial failed").await;
            return;
        }
        Err(DialFailure::Error(error)) => {
            tracing::warn!(path = %path, error = %error, "rivet proxy: upstream WS error");
            close_browser(browser, 1011, &format!("upstream error: {error}")).await;
            return;
        }
    };

    let (browser_sink, mut browser_rx) = browser.split();
    let (upstream_sink, mut upstream_rx) = upstream.split();
    let budget = Arc::new(Budget::default());
    let (abort_tx, abort_rx) = watch::channel::<Option<Abort>>(None);
    let (failed_tx, mut failed_rx) = mpsc::unbounded_channel::<Leg>();
    let (to_browser, browser_queue) = mpsc::unbounded_channel::<Out>();
    let (to_upstream, upstream_queue) = mpsc::unbounded_channel::<Out>();

    let browser_writer = Writer {
        sink: browser_sink,
        rx: browser_queue,
        abort: abort_rx.clone(),
        budget: Arc::clone(&budget),
        convert: to_axum,
        failed: failed_tx.clone(),
        leg: Leg::Browser,
    }
    .spawn();
    let upstream_writer = Writer {
        sink: upstream_sink,
        rx: upstream_queue,
        abort: abort_rx,
        budget: Arc::clone(&budget),
        convert: to_tungstenite,
        failed: failed_tx,
        leg: Leg::Upstream,
    }
    .spawn();

    let close_both = |code: u16, reason: &str| {
        abort_tx.send_replace(Some(Abort {
            code,
            reason: cap_ws_close_reason(reason),
        }));
    };
    let enqueue = |tx: &mpsc::UnboundedSender<Out>, data: Data| -> bool {
        let bytes = data.len();
        if !budget.try_take(bytes, opts.max_buffered_bytes) {
            return false;
        }
        if tx.send(Out::Data(data)).is_err() {
            budget.release(bytes);
        }
        true
    };

    let mut ping = tokio::time::interval_at(
        tokio::time::Instant::now() + opts.ping_interval,
        opts.ping_interval,
    );
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let idle = tokio::time::sleep(opts.idle_timeout);
    tokio::pin!(idle);

    loop {
        tokio::select! {
            msg = browser_rx.next() => {
                idle.as_mut().reset(tokio::time::Instant::now() + opts.idle_timeout);
                let data = match msg {
                    Some(Ok(axws::Message::Text(t))) => Data::Text(t.as_str().to_owned()),
                    Some(Ok(axws::Message::Binary(b))) => Data::Binary(b),
                    Some(Ok(axws::Message::Ping(_) | axws::Message::Pong(_))) => continue,
                    Some(Ok(axws::Message::Close(frame))) => {
                        let (code, reason) = frame
                            .map_or((1005, String::new()), |f| (f.code, f.reason.as_str().to_owned()));
                        let reason = if reason.is_empty() { "client closed".to_owned() } else { reason };
                        let _ = to_upstream.send(Out::Close(
                            normalized_close_code(code, 1001),
                            cap_ws_close_reason(&reason),
                        ));
                        break;
                    }
                    // Abrupt termination: TS saw 1006, which no close frame can carry.
                    Some(Err(_)) | None => {
                        let _ = to_upstream.send(Out::Close(1001, "client closed".to_owned()));
                        break;
                    }
                };
                if !enqueue(&to_upstream, data) {
                    tracing::warn!(
                        queued_bytes = budget.queued.load(Ordering::Acquire) as u64,
                        "rivet proxy: upstream-leg buffer cap exceeded, closing"
                    );
                    close_both(1013, "backpressure limit");
                    break;
                }
            }
            msg = upstream_rx.next() => {
                let data = match msg {
                    Some(Ok(ts::Message::Text(t))) => Data::Text(t.as_str().to_owned()),
                    Some(Ok(ts::Message::Binary(b))) => Data::Binary(b),
                    Some(Ok(ts::Message::Ping(_) | ts::Message::Pong(_) | ts::Message::Frame(_))) => continue,
                    Some(Ok(ts::Message::Close(frame))) => {
                        let (code, reason) = frame.map_or((1005, String::new()), |f| {
                            (u16::from(f.code), f.reason.as_str().to_owned())
                        });
                        let reason = if reason.is_empty() { "upstream closed".to_owned() } else { reason };
                        let _ = to_browser.send(Out::Close(
                            normalized_close_code(code, 1011),
                            cap_ws_close_reason(&reason),
                        ));
                        break;
                    }
                    Some(Err(err)) => {
                        let error = describe_ts_error(&err);
                        tracing::warn!(path = %path, error = %error, "rivet proxy: upstream WS error");
                        close_both(1011, &format!("upstream error: {error}"));
                        break;
                    }
                    None => {
                        let _ = to_browser.send(Out::Close(1011, "upstream closed".to_owned()));
                        break;
                    }
                };
                if !enqueue(&to_browser, data) {
                    tracing::warn!(
                        queued_bytes = budget.queued.load(Ordering::Acquire) as u64,
                        "rivet proxy: browser-leg buffer cap exceeded, closing"
                    );
                    close_both(1013, "backpressure limit");
                    break;
                }
            }
            leg = failed_rx.recv() => {
                match leg {
                    Some(Leg::Browser) => close_both(1011, "browser send failed"),
                    Some(Leg::Upstream) | None => close_both(1011, "upstream error"),
                }
                break;
            }
            _ = ping.tick() => {
                let _ = to_browser.send(Out::Ping);
            }
            () = &mut idle => {
                // Bun's idleTimeout closed the browser socket; its close handler then
                // mirrored 1006 upstream as 1001.
                let _ = to_browser.send(Out::Close(1001, String::new()));
                let _ = to_upstream.send(Out::Close(1001, "client closed".to_owned()));
                break;
            }
        }
    }

    // Let each writer flush its queue and close frame; dropping the senders ends
    // the writers that have nothing left to send.
    drop(to_browser);
    drop(to_upstream);
    let mut writers = [browser_writer, upstream_writer];
    let joined = tokio::time::timeout(CLOSE_GRACE * 2, async {
        for writer in &mut writers {
            let _ = writer.await;
        }
    })
    .await;
    if joined.is_err() {
        // A leg that cannot even take a close frame: drop both.
        for writer in &writers {
            writer.abort();
        }
    }
    drop(abort_tx);
}
