//! One table connection in the browser (web plan S4).
//!
//! The socket, the table-session mint and every reconnect belong to the rivetkit JS client
//! behind `js/rivet-bridge.js` (`web/rivet-bridge.ts`): it re-runs `getToken`/`getParams` on
//! each dial, so a fresh session rides each reconnect. This side maps the bridge's three
//! callbacks to [`ConnEvent`]s, runs the table actions, and gives up (`Ended`) after
//! [`GIVE_UP`] without a connection, since rivetkit itself retries forever.
//!
//! Every JS value stays on the page thread inside a [`Local`]; the handles in `Backend` /
//! `TableCalls` are `Send + Sync`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use euchre_protocol::{
    CompletedHandReview, MoveAck, PublicGameView, ReviewTrickAck, ReviewTrickRequest,
    SubmitMoveRequest, TableEvent, table_actions,
};
use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::mpsc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use super::http::{Call, status_error};
use super::local::{Local, on_page};
use crate::net::rivet_error::{BridgeError, RivetError, action_error, move_rejection};
use crate::net::{ConnEvent, NetError, TableCalls, TableConn, now_ms, rt};

/// How long an open waits for the mint and the socket.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one action waits for its reply.
pub const ACTION_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a table keeps reconnecting before it ends for good.
pub const GIVE_UP: Duration = Duration::from_secs(120);
/// How long `close()` waits for the socket to say goodbye.
pub const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// How often the watchdog and the open poll look at the connection.
const POLL: Duration = Duration::from_millis(250);

#[wasm_bindgen(module = "/js/rivet-bridge.js")]
extern "C" {
    /// One open table (`TableBridge` in `web/rivet-bridge.ts`).
    #[derive(Clone)]
    type TableBridge;

    /// Mints the table session and dials the actor through `<origin>/rivet`. Callbacks:
    /// `(event, json)`, `(status)`, `(errorJson)`.
    #[wasm_bindgen(js_name = openTable)]
    fn open_table(
        origin: &str,
        game_id: &str,
        on_event: &Closure<dyn FnMut(String, String)>,
        on_status: &Closure<dyn FnMut(String)>,
        on_error: &Closure<dyn FnMut(String)>,
    ) -> TableBridge;

    /// Resolves to the actor id once the session is minted; rejects with error JSON.
    #[wasm_bindgen(method)]
    fn ready(this: &TableBridge) -> js_sys::Promise;

    /// Resolves to the action's result as JSON text; rejects with error JSON.
    #[wasm_bindgen(method)]
    fn action(this: &TableBridge, name: &str, args_json: &str) -> js_sys::Promise;

    /// Closes the socket and the client.
    #[wasm_bindgen(method)]
    fn dispose(this: &TableBridge) -> js_sys::Promise;
}

/// What the callbacks and the calls share.
struct Shared {
    tx: mpsc::UnboundedSender<ConnEvent>,
    connected: AtomicBool,
    /// Closed, ended or given up: no more events, no more calls.
    ended: AtomicBool,
    /// When the socket was last seen down (`None` while up).
    down_since: Mutex<Option<u64>>,
}

impl Shared {
    fn end(&self, err: NetError) {
        if !self.ended.swap(true, Ordering::SeqCst) {
            let _ = self.tx.send(ConnEvent::Ended(err));
        }
    }
}

/// The JS callbacks; they must outlive the socket.
struct Closures {
    _event: Closure<dyn FnMut(String, String)>,
    _status: Closure<dyn FnMut(String)>,
    _error: Closure<dyn FnMut(String)>,
}

/// An open table: the bridge handle plus its shared state.
pub struct WebTable {
    bridge: Arc<Local<TableBridge>>,
    closures: Mutex<Option<Local<Closures>>>,
    shared: Arc<Shared>,
}

/// Opens `game_id`: the bridge mints the session and dials; `Status(true)` is the stream's
/// first item once the socket is connected.
pub async fn open(origin: String, game_id: String) -> Result<TableConn, NetError> {
    on_page(open_on_page(origin, game_id))
        .await
        .unwrap_or(Err(NetError::Closed))
}

async fn open_on_page(origin: String, game_id: String) -> Result<TableConn, NetError> {
    let (tx, rx) = mpsc::unbounded_channel();
    let shared = Arc::new(Shared {
        tx,
        connected: AtomicBool::new(false),
        ended: AtomicBool::new(false),
        down_since: Mutex::new(Some(now_ms())),
    });
    let s = Arc::clone(&shared);
    let on_event = Closure::<dyn FnMut(String, String)>::new(move |name: String, json: String| {
        if s.ended.load(Ordering::SeqCst) {
            return;
        }
        let payload: Value = serde_json::from_str(&json).unwrap_or(Value::Null);
        match TableEvent::decode(&name, payload) {
            Ok(ev) => {
                let _ = s.tx.send(ConnEvent::Event(ev));
            }
            Err(err) => tracing::warn!(event = %name, error = %err, "table event did not decode"),
        }
    });
    let s = Arc::clone(&shared);
    let on_status = Closure::<dyn FnMut(String)>::new(move |status: String| {
        let up = status == "connected";
        if s.connected.swap(up, Ordering::SeqCst) != up {
            *s.down_since.lock() = if up { None } else { Some(now_ms()) };
            if !s.ended.load(Ordering::SeqCst) {
                let _ = s.tx.send(ConnEvent::Status(up));
            }
        }
    });
    let s = Arc::clone(&shared);
    let on_error = Closure::<dyn FnMut(String)>::new(move |json: String| {
        let e = BridgeError::parse(&json);
        tracing::warn!(group = ?e.group, code = ?e.code, "table connection error");
        // A refused re-mint during a reconnect is final; anything else rivetkit retries.
        if e.group.as_deref() == Some("session") {
            let err = e
                .status
                .map(|st| status_error(Call::TableSession, Some(st), false, None));
            if let Some(err @ (NetError::Forbidden | NetError::NotFound | NetError::Unauthorized)) =
                err
            {
                s.end(err);
            }
        }
    });
    let bridge = open_table(&origin, &game_id, &on_event, &on_status, &on_error);
    let closures = Closures {
        _event: on_event,
        _status: on_status,
        _error: on_error,
    };
    let dispose = |bridge: &TableBridge| {
        let _ = bridge.dispose();
    };
    // The mint, then the socket reaching `connected`, within one bound.
    let opened = rt::timeout(OPEN_TIMEOUT, async {
        JsFuture::from(bridge.ready()).await?;
        while !shared.connected.load(Ordering::SeqCst) && !shared.ended.load(Ordering::SeqCst) {
            rt::sleep(Duration::from_millis(50)).await;
        }
        Ok::<(), JsValue>(())
    })
    .await;
    match opened {
        Err(rt::Elapsed) => {
            dispose(&bridge);
            return Err(NetError::Timeout);
        }
        Ok(Err(e)) => {
            dispose(&bridge);
            let status = e.as_string().and_then(|t| BridgeError::parse(&t).status);
            return Err(status_error(Call::TableSession, status, false, None));
        }
        Ok(Ok(())) => {}
    }
    if shared.ended.load(Ordering::SeqCst) {
        dispose(&bridge);
        // The error callback ended it with the reason; the receiver holds it.
        let mut rx = rx;
        return Err(match rx.try_recv() {
            Ok(ConnEvent::Ended(err)) => err,
            _ => NetError::Closed,
        });
    }
    let bridge = Arc::new(Local::new(bridge));
    spawn_watchdog(Arc::downgrade(&shared), Arc::clone(&bridge));
    let table = WebTable {
        bridge,
        closures: Mutex::new(Some(Local::new(closures))),
        shared,
    };
    Ok(TableConn {
        game_id,
        events: rx,
        calls: Arc::new(table),
    })
}

/// Ends the table after [`GIVE_UP`] without a connection, and tears the bridge down once it
/// has ended (or its table is dropped).
fn spawn_watchdog(shared: Weak<Shared>, bridge: Arc<Local<TableBridge>>) {
    let _ = rt::spawn(async move {
        loop {
            rt::sleep(POLL).await;
            let Some(s) = shared.upgrade() else { break };
            if s.ended.load(Ordering::SeqCst) {
                break;
            }
            let down = *s.down_since.lock();
            if down.is_some_and(|t| now_ms().saturating_sub(t) > GIVE_UP.as_millis() as u64) {
                s.end(NetError::Unavailable);
                break;
            }
        }
        // Ended (give-up, refused re-mint) or closed: free the socket. A second dispose is a
        // no-op in the bridge.
        let _ = bridge.with(|b| b.dispose());
    });
}

impl WebTable {
    /// Runs one action on the live socket; `Err(Closed)` at once when it is down.
    async fn invoke(
        &self,
        name: &'static str,
        args: Vec<Value>,
    ) -> Result<Result<Value, Option<RivetError>>, NetError> {
        if self.shared.ended.load(Ordering::SeqCst) || !self.shared.connected.load(Ordering::SeqCst)
        {
            return Err(NetError::Closed);
        }
        let args = serde_json::to_string(&args).map_err(|e| NetError::Other(e.to_string()))?;
        let bridge = Arc::clone(&self.bridge);
        let call = on_page(async move {
            let promise = bridge.with(|b| b.action(name, &args))?;
            Some(
                JsFuture::from(promise)
                    .await
                    .map(|v| v.as_string().unwrap_or_else(|| "null".to_owned()))
                    .map_err(|e| e.as_string().unwrap_or_default()),
            )
        });
        match rt::timeout(ACTION_TIMEOUT, call).await {
            Err(rt::Elapsed) => Err(NetError::Timeout),
            Ok(None | Some(None)) => Err(NetError::Closed),
            Ok(Some(Some(Ok(text)))) => Ok(Ok(serde_json::from_str(&text).unwrap_or(Value::Null))),
            Ok(Some(Some(Err(text)))) => Ok(Err(BridgeError::parse(&text).rivet())),
        }
    }
}

fn reply<T: DeserializeOwned>(name: &str, v: Value) -> Result<T, NetError> {
    serde_json::from_value(v).map_err(|e| NetError::Other(format!("{name} reply: {e}")))
}

fn arg<T: serde::Serialize>(v: &T) -> Result<Value, NetError> {
    serde_json::to_value(v).map_err(|e| NetError::Other(e.to_string()))
}

#[async_trait]
impl TableCalls for WebTable {
    async fn submit_move(&self, request: SubmitMoveRequest) -> Result<MoveAck, NetError> {
        let name = table_actions::SUBMIT_MOVE;
        match self.invoke(name, vec![arg(&request)?]).await? {
            Ok(v) => reply(name, v),
            Err(e) => match e.as_ref().and_then(move_rejection) {
                Some(ack) => Ok(ack),
                None => Err(action_error(e.as_ref())),
            },
        }
    }

    async fn review_trick(&self, request: ReviewTrickRequest) -> Result<ReviewTrickAck, NetError> {
        let name = table_actions::REVIEW_TRICK;
        match self.invoke(name, vec![arg(&request)?]).await? {
            Ok(v) => reply(name, v),
            Err(e) => Err(action_error(e.as_ref())),
        }
    }

    async fn snapshot(&self) -> Result<Option<PublicGameView>, NetError> {
        let name = table_actions::SNAPSHOT;
        match self.invoke(name, Vec::new()).await? {
            Ok(Value::Null) => Ok(None),
            Ok(v) => reply(name, v).map(Some),
            Err(e) => Err(action_error(e.as_ref())),
        }
    }

    async fn get_hand_review(&self, hand_no: u32) -> Result<Option<CompletedHandReview>, NetError> {
        let name = table_actions::GET_HAND_REVIEW;
        match self.invoke(name, vec![Value::from(hand_no)]).await? {
            Ok(Value::Null) => Ok(None),
            Ok(v) => reply(name, v).map(Some),
            Err(e) => Err(action_error(e.as_ref())),
        }
    }

    async fn close(&self) {
        self.shared.ended.store(true, Ordering::SeqCst);
        let bridge = Arc::clone(&self.bridge);
        let disposed = on_page(async move {
            if let Some(promise) = bridge.with(|b| b.dispose()) {
                let _ = JsFuture::from(promise).await;
            }
        });
        let _ = rt::timeout(CLOSE_TIMEOUT, disposed).await;
        // Free the callbacks only after the socket is gone, so JS never calls a freed closure.
        self.closures.lock().take();
    }
}
