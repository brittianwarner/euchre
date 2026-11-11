//! rivetkit 3.1.0 helpers shared by every actor and client in the server (08 cookbook).
//! Owned by lane SERVER-FOUNDATION.
//!
//! | Need | Helper | Gotcha |
//! |---|---|---|
//! | client-visible errors | [`user_error`], [`user_error_with_metadata`] | G13–G15 |
//! | read errors on the client side | [`parse_rivet_error`] → [`UserError`] | G16 |
//! | scheduled actions | [`schedule_after`], [`schedule_action`], [`is_scheduled_fire`], [`require_scheduled_fire`] | G24 |
//! | connection params | [`decode_params`], [`conn_params`], [`caller`] (re-verify per action, D17) | — |
//! | SQLite JSON rows | [`JsonTable`] | G11 |
//! | one shared client | [`build_client`] (JSON encoding, D19) | G26 |
//! | bounded connect / call | [`connect_bounded`], [`call_with_timeout`], [`stateless_call_with_timeout`] | G17–G19, G29 |
//! | event callbacks | [`forward_events`] | G28 |
//! | first requests after boot | [`wait_ready`] | G36 |
//! | serialized mutations | [`ActorLane`] | G2 |
//! | a private local engine | [`LocalEngine`] | G30, G31 |
//!
//! # The mutation lane (G2)
//!
//! rivetkit runs actions of one actor concurrently. Every handler that reads, changes
//! and persists state takes the actor's [`ActorLane`] first and holds it across its
//! awaits, in this order:
//!
//! 1. **persist**: write the new state (one SQLite statement or batch);
//! 2. **emit**: broadcast events only after the commit succeeded;
//! 3. **ack**: return the action result last.
//!
//! A client that sees the ack has therefore already been sent the event, and a
//! crash between steps never emits state that was not stored.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use rivetkit::client::{Client, ClientConfig, ConnectionStatus, EncodingKind, SubscriptionHandle};
use rivetkit::{
    Action, Actor, BindParam, ColumnValue, ConnCtx, Ctx, Event, SqliteBatchStatement, SqliteDb,
    TypedActorConnection, TypedActorHandle,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Mutex, MutexGuard, mpsc};

// =============================================================== errors (G13–G16)

/// The public error group whose code and message reach clients unmasked.
pub const USER_GROUP: &str = "user";

/// A client-visible error `user/<code>` with `message` (TS `protocolError`). Plain
/// `anyhow`/`bail!` errors reach clients as `core/internal_error` (G13); this one
/// carries its code and message through.
pub fn user_error(code: &str, message: impl Into<String>) -> anyhow::Error {
    public_error(USER_GROUP, code, message.into(), None)
}

/// [`user_error`] with JSON metadata. If `metadata` cannot be serialized the error is
/// still returned, without metadata.
pub fn user_error_with_metadata<M: Serialize + ?Sized>(
    code: &str,
    message: impl Into<String>,
    metadata: &M,
) -> anyhow::Error {
    let meta = serde_json::value::to_raw_value(metadata).ok();
    public_error(USER_GROUP, code, message.into(), meta)
}

fn public_error(
    group: &str,
    code: &str,
    message: String,
    meta: Option<Box<serde_json::value::RawValue>>,
) -> anyhow::Error {
    anyhow::Error::new(rivet_error::RivetError {
        kind: rivet_error::RivetErrorKind::Dynamic {
            group: group.to_owned(),
            code: code.to_owned(),
            default_message: message,
        },
        meta,
        message: None,
        actor: None,
    })
}

/// A structured rivet error recovered from an `anyhow::Error`.
#[derive(Debug, Clone, PartialEq)]
pub struct UserError {
    /// Error group (`user`, `core`, `actor`, `auth`, ...).
    pub group: String,
    /// Error code within the group.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Metadata, when the error carried any (JSON `null` is `None`).
    pub metadata: Option<Value>,
}

impl UserError {
    /// True for `user/<code>` errors raised by [`user_error`].
    pub fn is_user(&self) -> bool {
        self.group == USER_GROUP
    }

    /// True when this is `user/<code>`.
    pub fn is(&self, code: &str) -> bool {
        self.is_user() && self.code == code
    }
}

/// Recover `{group, code, message, metadata}` from an error (G16).
///
/// Handles, in order: a [`rivet_error::RivetError`] anywhere in the chain (server
/// side, in-process); the connection format
/// `RPC Error({group}/{code}): {message}, {metadata:#}`; and the stateless format
/// `action failed ({group}/{code}): {message}, metadata={metadata:?}`.
pub fn parse_rivet_error(error: &anyhow::Error) -> Option<UserError> {
    if let Some(rivet) = error
        .chain()
        .find_map(|e| e.downcast_ref::<rivet_error::RivetError>())
    {
        let metadata = rivet
            .meta
            .as_ref()
            .and_then(|raw| serde_json::from_str::<Value>(raw.get()).ok())
            .filter(|v| !v.is_null());
        return Some(UserError {
            group: rivet.kind.group().to_owned(),
            code: rivet.kind.code().to_owned(),
            message: rivet
                .message
                .clone()
                .unwrap_or_else(|| rivet.kind.default_message().to_owned()),
            metadata,
        });
    }
    parse_rivet_error_str(&format!("{error:#}"))
}

/// [`parse_rivet_error`] over an already formatted error string.
pub fn parse_rivet_error_str(text: &str) -> Option<UserError> {
    const CONN: &str = "RPC Error(";
    const STATELESS: &str = "action failed (";
    if let Some(i) = text.rfind(CONN) {
        let (group, code, rest) = split_group_code(&text[i + CONN.len()..])?;
        let (message, metadata) = split_conn_metadata(rest);
        return Some(UserError {
            group,
            code,
            message,
            metadata,
        });
    }
    let i = text.rfind(STATELESS)?;
    let (group, code, rest) = split_group_code(&text[i + STATELESS.len()..])?;
    let (message, metadata) = split_stateless_metadata(rest);
    Some(UserError {
        group,
        code,
        message,
        metadata: metadata.filter(|v| !v.is_null()),
    })
}

/// `group/code): rest` → `(group, code, rest)` with the `: ` separator removed.
fn split_group_code(s: &str) -> Option<(String, String, &str)> {
    let close = s.find(')')?;
    let (group, code) = s[..close].split_once('/')?;
    let rest = &s[close + 1..];
    let rest = rest
        .strip_prefix(": ")
        .or_else(|| rest.strip_prefix(':'))
        .unwrap_or(rest);
    Some((group.to_owned(), code.to_owned(), rest))
}

/// `message, metadata=<Debug Option<Value>>`: the leftmost `, metadata=` whose whole
/// suffix parses (either side may itself contain `, metadata=`).
fn split_stateless_metadata(rest: &str) -> (String, Option<Value>) {
    const SEP: &str = ", metadata=";
    let mut from = 0;
    while let Some(off) = rest[from..].find(SEP) {
        let at = from + off;
        if let Some(meta) = parse_debug_option_value(&rest[at + SEP.len()..]) {
            return (rest[..at].to_owned(), meta);
        }
        from = at + SEP.len();
    }
    (rest.to_owned(), None)
}

/// `message, <pretty json>`: the leftmost `, ` whose suffix is valid JSON.
fn split_conn_metadata(rest: &str) -> (String, Option<Value>) {
    let mut from = 0;
    while let Some(off) = rest[from..].find(", ") {
        let at = from + off;
        if let Ok(v) = serde_json::from_str::<Value>(&rest[at + 2..]) {
            return (rest[..at].to_owned(), Some(v).filter(|v| !v.is_null()));
        }
        from = at + 2;
    }
    (rest.to_owned(), None)
}

/// Parse `None` / `Some(<serde_json::Value as Debug>)`. Outer `None` when unparseable.
fn parse_debug_option_value(s: &str) -> Option<Option<Value>> {
    let s = s.trim();
    if s == "None" {
        return Some(None);
    }
    let inner = s.strip_prefix("Some(")?.strip_suffix(')')?;
    let mut p = DebugValueParser { s: inner, i: 0 };
    let v = p.value()?;
    p.ws();
    (p.i == p.s.len()).then_some(Some(v))
}

/// Recursive-descent parser for `serde_json::Value`'s `Debug` output
/// (`Null`, `Bool(b)`, `Number(n)`, `String("..")`, `Array [..]`, `Object {..}`).
struct DebugValueParser<'a> {
    s: &'a str,
    i: usize,
}

impl DebugValueParser<'_> {
    fn rest(&self) -> &str {
        &self.s[self.i..]
    }

    fn ws(&mut self) {
        let trimmed = self.rest().trim_start();
        self.i = self.s.len() - trimmed.len();
    }

    fn eat(&mut self, token: &str) -> bool {
        self.ws();
        if self.rest().starts_with(token) {
            self.i += token.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.ws();
        if self.eat("Null") {
            return Some(Value::Null);
        }
        if self.eat("Bool(") {
            let v = if self.eat("true") {
                true
            } else if self.eat("false") {
                false
            } else {
                return None;
            };
            return self.eat(")").then_some(Value::Bool(v));
        }
        if self.eat("Number(") {
            let end = self.rest().find(')')?;
            let raw = self.rest()[..end].trim().to_owned();
            self.i += end + 1;
            let n = if let Ok(i) = raw.parse::<i64>() {
                serde_json::Number::from(i)
            } else if let Ok(u) = raw.parse::<u64>() {
                serde_json::Number::from(u)
            } else {
                serde_json::Number::from_f64(raw.parse::<f64>().ok()?)?
            };
            return Some(Value::Number(n));
        }
        if self.eat("String(") {
            let s = self.string()?;
            return self.eat(")").then_some(Value::String(s));
        }
        if self.eat("Array") {
            if !self.eat("[") {
                return None;
            }
            let mut items = Vec::new();
            if self.eat("]") {
                return Some(Value::Array(items));
            }
            loop {
                items.push(self.value()?);
                if self.eat("]") {
                    return Some(Value::Array(items));
                }
                if !self.eat(",") {
                    return None;
                }
            }
        }
        if self.eat("Object") {
            if !self.eat("{") {
                return None;
            }
            let mut map = serde_json::Map::new();
            if self.eat("}") {
                return Some(Value::Object(map));
            }
            loop {
                self.ws();
                let key = self.string()?;
                if !self.eat(":") {
                    return None;
                }
                let v = self.value()?;
                map.insert(key, v);
                if self.eat("}") {
                    return Some(Value::Object(map));
                }
                if !self.eat(",") {
                    return None;
                }
            }
        }
        None
    }

    /// A Rust `Debug` string literal: `"..."` with `\"`, `\\`, `\n`, `\r`, `\t`, `\0`,
    /// `\'` and `\u{hex}` escapes.
    fn string(&mut self) -> Option<String> {
        self.ws();
        let mut chars = self.rest().char_indices();
        if chars.next()?.1 != '"' {
            return None;
        }
        let mut out = String::new();
        while let Some((idx, c)) = chars.next() {
            match c {
                '"' => {
                    self.i += idx + 1;
                    return Some(out);
                }
                '\\' => match chars.next()?.1 {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    '0' => out.push('\0'),
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    '\'' => out.push('\''),
                    'u' => {
                        if chars.next()?.1 != '{' {
                            return None;
                        }
                        let mut hex = String::new();
                        loop {
                            let (_, h) = chars.next()?;
                            if h == '}' {
                                break;
                            }
                            hex.push(h);
                        }
                        out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                    }
                    _ => return None,
                },
                other => out.push(other),
            }
        }
        None
    }
}

// =============================================================== schedule (G24)

/// Schedule `action` with `args` (encoded positionally, as a client call would) to run
/// on this actor after `delay`. Returns the schedule event id.
pub async fn schedule_after<A: Actor, T: Serialize + ?Sized>(
    ctx: &Ctx<A>,
    delay: Duration,
    action: &str,
    args: &T,
) -> Result<String> {
    let bytes = rivetkit::action::encode_positional(&args)?;
    ctx.schedule().after(delay, action, &bytes).await
}

/// Typed [`schedule_after`]: `M::NAME` with `action` as its arguments.
pub async fn schedule_action<A: Actor, M: Action>(
    ctx: &Ctx<A>,
    delay: Duration,
    action: &M,
) -> Result<String> {
    schedule_after(ctx, delay, M::NAME, action).await
}

/// True when the running action was fired by the scheduler: scheduled actions run
/// with no connection, while every client call (WebSocket or stateless HTTP, G5) has one.
pub fn is_scheduled_fire<A: Actor>(ctx: &Ctx<A>) -> bool {
    ctx.conn().is_none()
}

/// Guard for internal, schedule-only actions: `user/internal_only` for any client call.
pub fn require_scheduled_fire<A: Actor>(ctx: &Ctx<A>) -> Result<()> {
    if is_scheduled_fire(ctx) {
        Ok(())
    } else {
        Err(user_error("internal_only", "This action is internal."))
    }
}

// =============================================================== conn params (D17)

/// Decode connection params received as JSON (`type ConnParams = serde_json::Value`)
/// into `T`. A shape mismatch is `user/bad_request` instead of the masked
/// `core/internal_error` a typed `ConnParams` decode failure produces.
pub fn decode_params<T: DeserializeOwned>(params: &Value) -> Result<T> {
    let value = if params.is_null() {
        Value::Object(serde_json::Map::new())
    } else {
        params.clone()
    };
    serde_json::from_value(value)
        .map_err(|e| user_error("bad_request", format!("Invalid connection params: {e}")))
}

/// [`decode_params`] for a connection of an actor whose `ConnParams` is JSON.
pub fn conn_params<A, T>(conn: &ConnCtx<A>) -> Result<T>
where
    A: Actor<ConnParams = Value>,
    T: DeserializeOwned,
{
    let raw = conn
        .params()
        .map_err(|_| user_error("bad_request", "Invalid connection params"))?;
    decode_params(&raw)
}

/// Re-verify the caller on every action (D17): read this call's connection params
/// and run `verify` on them. Tokens can expire between connect and action, so the
/// identity from `on_before_connect` is never trusted on its own. A scheduled fire
/// (no connection) is `user/unauthorized`.
pub fn caller<A, C>(ctx: &Ctx<A>, verify: impl FnOnce(&A::ConnParams) -> Result<C>) -> Result<C>
where
    A: Actor,
{
    let Some(conn) = ctx.conn() else {
        return Err(user_error(
            "unauthorized",
            "This action needs a connection.",
        ));
    };
    let params = conn
        .params()
        .map_err(|_| user_error("bad_request", "Invalid connection params"))?;
    verify(&params)
}

// =============================================================== SQLite JSON rows

/// A key → JSON document table in the actor's SQLite database. Each [`JsonTable::save`]
/// is a single upsert statement, so a document is replaced atomically.
///
/// The name is double-quoted in every generated statement, so SQL keywords
/// (`order`, `group`) work as names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonTable {
    name: String,
}

/// Table-name prefixes that belong to SQLite (`sqlite_`) or rivetkit (`_rivet_`).
const RESERVED_TABLE_PREFIXES: [&str; 2] = ["sqlite_", "_rivet_"];

impl JsonTable {
    /// A table named `name` (`[A-Za-z_][A-Za-z0-9_]*`, at most 64 chars, not starting
    /// with `sqlite_` or `_rivet_` in any case).
    pub fn new(name: &str) -> Result<Self> {
        let mut chars = name.chars();
        let valid = name.len() <= 64
            && chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            bail!("invalid SQLite table name `{name}`");
        }
        let lower = name.to_ascii_lowercase();
        if RESERVED_TABLE_PREFIXES.iter().any(|p| lower.starts_with(p)) {
            bail!("SQLite table name `{name}` uses a reserved prefix (sqlite_ or _rivet_)");
        }
        Ok(Self {
            name: name.to_owned(),
        })
    }

    /// The table name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The name as a quoted SQL identifier (validation guarantees it has no `"`).
    fn ident(&self) -> String {
        format!("\"{}\"", self.name)
    }

    /// `CREATE TABLE IF NOT EXISTS` for this table.
    pub fn schema_sql(&self) -> String {
        format!(
            "CREATE TABLE IF NOT EXISTS {} (key TEXT PRIMARY KEY NOT NULL, json TEXT NOT NULL, updated_at INTEGER NOT NULL)",
            self.ident()
        )
    }

    /// Create the table if missing. Call from `Actor::create`.
    pub async fn ensure(&self, db: &SqliteDb) -> Result<()> {
        db.exec(self.schema_sql())
            .await
            .context("create json table")?;
        Ok(())
    }

    /// Load and decode the document at `key`.
    pub async fn load<T: DeserializeOwned>(&self, db: &SqliteDb, key: &str) -> Result<Option<T>> {
        let res = db
            .query(
                format!("SELECT json FROM {} WHERE key = ?", self.ident()),
                Some(vec![BindParam::Text(key.to_owned())]),
            )
            .await?;
        let Some(row) = res.rows.first() else {
            return Ok(None);
        };
        match row.first() {
            Some(ColumnValue::Text(json)) => Ok(Some(
                serde_json::from_str(json)
                    .with_context(|| format!("decode {}[{key}]", self.name))?,
            )),
            other => bail!("{}[{key}].json: expected text, got {other:?}", self.name),
        }
    }

    /// The upsert for `key` = `value`, for use inside an `execute_batch`.
    pub fn save_statement<T: Serialize + ?Sized>(
        &self,
        key: &str,
        value: &T,
    ) -> Result<SqliteBatchStatement> {
        Ok(SqliteBatchStatement {
            sql: format!(
                "INSERT INTO {} (key, json, updated_at) VALUES (?, ?, ?) \
                 ON CONFLICT(key) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at",
                self.ident()
            ),
            params: Some(vec![
                BindParam::Text(key.to_owned()),
                BindParam::Text(serde_json::to_string(value)?),
                BindParam::Integer(crate::auth::now_ms()),
            ]),
        })
    }

    /// Store `value` at `key` in one statement.
    pub async fn save<T: Serialize + ?Sized>(
        &self,
        db: &SqliteDb,
        key: &str,
        value: &T,
    ) -> Result<()> {
        let st = self.save_statement(key, value)?;
        db.run(st.sql, st.params).await?;
        Ok(())
    }

    /// Delete the document at `key`.
    pub async fn delete(&self, db: &SqliteDb, key: &str) -> Result<()> {
        db.run(
            format!("DELETE FROM {} WHERE key = ?", self.ident()),
            Some(vec![BindParam::Text(key.to_owned())]),
        )
        .await?;
        Ok(())
    }
}

// =============================================================== client (D19, G26)

/// Build the one shared client (D19: JSON encoding, so payloads match the TS wire).
///
/// G26: dropping **any** clone of a `Client` ends every reconnect loop it owns, so
/// keep exactly one per process behind this `Arc` and never call `Client::clone()`.
pub fn build_client(endpoint: &str, namespace: &str, token: Option<&str>) -> Arc<Client> {
    build_client_with_pool(endpoint, namespace, token, None)
}

/// [`build_client`] with an explicit pool name.
pub fn build_client_with_pool(
    endpoint: &str,
    namespace: &str,
    token: Option<&str>,
    pool_name: Option<&str>,
) -> Arc<Client> {
    let mut cfg = ClientConfig::new(endpoint)
        .namespace(namespace)
        .token_opt(token.map(str::to_owned))
        .encoding(EncodingKind::Json);
    if let Some(pool) = pool_name {
        cfg = cfg.pool_name(pool);
    }
    Arc::new(Client::new(cfg))
}

/// Why a bounded client operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallError {
    /// No reply within the bound.
    #[error("{what} timed out after {after:?}")]
    Timeout {
        /// What timed out (`connect` or the action name).
        what: String,
        /// The bound.
        after: Duration,
    },
    /// The first WebSocket connect attempt failed: the actor refused it
    /// (`on_before_connect` error) or the engine could not be reached.
    ///
    /// rivetkit-client 3.1.0 hands only the bare error *message* to connection error
    /// callbacks, without `group/code`, so [`parse_rivet_error`] cannot classify this.
    /// A caller that needs the code (for example to tell `user/unauthorized`, re-mint
    /// the token, from a network failure) makes one stateless call with the same
    /// handle, which returns the full `group/code`.
    #[error("connect rejected: {message}")]
    Rejected {
        /// The error message the client reported, or a fixed hint when it reported
        /// none (the attempt failed before the error callback was registered, or the
        /// client sent an empty message).
        message: String,
    },
}

/// Open a WebSocket connection and wait up to `timeout` for `Connected` (G17).
///
/// A rejected connect otherwise retries forever with backoff and wakes the actor each
/// time. This fails fast instead: as soon as the first attempt ends without opening
/// (status `Disconnected` before `Connected`) it disconnects and returns
/// [`CallError::Rejected`] with the reported message; with no outcome within
/// `timeout` it disconnects and returns [`CallError::Timeout`].
///
/// rivetkit-client starts dialing inside `connect()`, before an error callback can be
/// registered, so a failure in the first microseconds can reach
/// [`CallError::Rejected`] without the client's message. The status watch never
/// misses it.
pub async fn connect_bounded<A: Actor>(
    handle: &TypedActorHandle<A>,
    timeout: Duration,
) -> Result<TypedActorConnection<A>> {
    let conn = handle.connect();
    let mut status = conn.inner().status_receiver();
    let last_error = Arc::new(parking_lot::Mutex::new(None::<String>));
    let sink = last_error.clone();
    conn.inner()
        .on_error(move |m| *sink.lock() = Some(m.to_owned()))
        .await;
    let outcome = tokio::time::timeout(
        timeout,
        status.wait_for(|s| {
            matches!(
                s,
                ConnectionStatus::Connected | ConnectionStatus::Disconnected
            )
        }),
    )
    .await
    .map(|r| r.map(|s| *s));
    match outcome {
        Ok(Ok(ConnectionStatus::Connected)) => Ok(conn),
        Ok(_) => {
            conn.disconnect().await;
            let message = last_error
                .lock()
                .clone()
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| {
                    "no reason given over the WebSocket; a stateless call classifies it".into()
                });
            Err(CallError::Rejected { message }.into())
        }
        Err(_) => {
            conn.disconnect().await;
            let last = last_error.lock().clone();
            let err = anyhow::Error::new(CallError::Timeout {
                what: "connect".into(),
                after: timeout,
            });
            Err(match last {
                Some(msg) => err.context(msg),
                None => err,
            })
        }
    }
}

/// Call `action` on a connection, bounded by `timeout` (G18, G19, G29). On timeout the
/// caller should `disconnect()`, reconnect and reload state with a snapshot action.
pub async fn call_with_timeout<A: Actor, M: Action>(
    conn: &TypedActorConnection<A>,
    action: M,
    timeout: Duration,
) -> Result<M::Output> {
    bounded(M::NAME, timeout, conn.call(action)).await
}

/// Stateless (HTTP) [`call_with_timeout`].
pub async fn stateless_call_with_timeout<A: Actor, M: Action>(
    handle: &TypedActorHandle<A>,
    action: M,
    timeout: Duration,
) -> Result<M::Output> {
    bounded(M::NAME, timeout, handle.call(action)).await
}

async fn bounded<T>(
    what: &str,
    after: Duration,
    fut: impl Future<Output = Result<T>>,
) -> Result<T> {
    match tokio::time::timeout(after, fut).await {
        Ok(r) => r,
        Err(_) => Err(CallError::Timeout {
            what: what.to_owned(),
            after,
        }
        .into()),
    }
}

/// Subscribe to `E` and forward each event into an unbounded channel (G28: callbacks
/// run inline on the socket receive loop, so they must never block).
///
/// Unsubscribe with the returned handle. The channel closes only when the connection
/// is closed with `disconnect()`: dropping the `TypedActorConnection` does **not** end
/// it, because rivetkit-client 3.1.0's connection task holds its own reference and
/// keeps reconnecting (and keeping the actor awake) until `disconnect()`. Events that
/// fail to decode as `E` are dropped by the typed client (logged at debug level only).
pub async fn forward_events<A: Actor, E: Event>(
    conn: &TypedActorConnection<A>,
) -> (SubscriptionHandle, mpsc::UnboundedReceiver<E>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let sub = conn
        .on::<E>(move |e| {
            let _ = tx.send(e);
        })
        .await;
    (sub, rx)
}

/// Retry `probe` until it succeeds or `deadline` passes (G36: the first requests after
/// boot fail with `Connection refused`, `no_runner_config_configured` or
/// `actor_ready_timeout`). Each attempt is bounded by 10 s or the time left, whichever
/// is less; attempts are 250 ms apart.
///
/// A `user/<code>` error is returned at once: it comes from the probe's own handler
/// (a bad token, a refused call) and retrying cannot change it. Every other error is
/// retried, because the set of transient boot and wake errors is wider than
/// [`is_startup_error`] lists.
pub async fn wait_ready<T, F, Fut>(deadline: Duration, mut probe: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    const ATTEMPT: Duration = Duration::from_secs(10);
    const BACKOFF: Duration = Duration::from_millis(250);
    let end = Instant::now() + deadline;
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        let bound = ATTEMPT.min(end.saturating_duration_since(Instant::now()));
        let err = match tokio::time::timeout(bound, probe()).await {
            Ok(Ok(v)) => return Ok(v),
            Ok(Err(e)) => e,
            Err(_) => anyhow!("attempt timed out after {bound:?}"),
        };
        if parse_rivet_error(&err).is_some_and(|e| e.is_user()) {
            return Err(err.context(format!(
                "probe refused with a user error on attempt {attempts}; not retrying"
            )));
        }
        if Instant::now() + BACKOFF >= end {
            return Err(err.context(format!(
                "not ready after {attempts} attempts ({deadline:?})"
            )));
        }
        tokio::time::sleep(BACKOFF).await;
    }
}

/// True for the errors [`wait_ready`] expects while an engine or envoy is starting.
pub fn is_startup_error(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    [
        "actor_ready_timeout",
        "no_runner_config_configured",
        "Connection refused",
        "connection refused",
        "503",
        "service_unavailable",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

// =============================================================== lane (G2)

/// The actor's turn lock (G2). Hold it across every read-modify-persist-emit sequence;
/// see the module docs for the persist-before-emit-before-ack order. Never hold a
/// `parking_lot` guard across an await instead (G3).
#[derive(Debug, Default)]
pub struct ActorLane {
    inner: Mutex<()>,
}

/// Held while a mutation runs on an [`ActorLane`].
pub type LaneGuard<'a> = MutexGuard<'a, ()>;

impl ActorLane {
    /// A new, free lane.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wait for the lane (FIFO, fair).
    pub async fn enter(&self) -> LaneGuard<'_> {
        self.inner.lock().await
    }

    /// Run `fut` while holding the lane.
    pub async fn run<T>(&self, fut: impl Future<Output = T>) -> T {
        let _guard = self.enter().await;
        fut.await
    }
}

// =============================================================== local engine (G30)

/// rivetkit's well-known loopback admin token. [`LocalEngine`] never uses it: every
/// engine it starts gets its own random token ([`LocalEngineOptions::admin_token`]).
pub const DEFAULT_ENGINE_TOKEN: &str = "default";
/// Namespace of a local engine.
pub const LOCAL_ENGINE_NAMESPACE: &str = "default";
/// Offset of the engine's api-peer port from its guard port (rivetkit: +1).
pub const PEER_PORT_OFFSET: u16 = 1;
/// Offset of the engine's metrics port (rivetkit uses +10, which collides with the
/// next lane's guard port in the port plan; see `docs/port/requests/foundation-engine-ports.md`).
pub const METRICS_PORT_OFFSET: u16 = 5;
/// Engine version this crate is built against.
pub const ENGINE_VERSION: &str = "3.1.0";
/// File under `<storage>/.rivetkit/var/engine/` recording `<engine pid> <owner pid>`.
pub const ENGINE_PID_FILE: &str = "euchre-engine.pid";
/// File under `<storage>/.rivetkit/var/engine/` recording the guard port the storage's
/// engine first became ready on (see [`recorded_engine_port`]).
pub const ENGINE_PORT_FILE: &str = "euchre-engine.port";

/// The guard port an engine on `storage_path` must keep using, or `None` for storage
/// that never ran one.
///
/// Engine 3.1.0 binds its JWT signing ring to the cluster topology, whose URLs carry
/// the guard and peer ports. The same storage started on another port refuses every
/// `POST /auth/tokens` ("JWT ring does not belong to this cluster"), so no gateway
/// token is ever issued again, and going back to the first port is the only cure (W3
/// operations review). [`LocalEngine::start`] therefore refuses a different port
/// ([`EnginePortMismatch`]), and `ServerConfig::local` reuses this one.
///
/// Reads [`ENGINE_PORT_FILE`]; storage from before that record falls back to the
/// topology `config.json` its last start wrote.
pub fn recorded_engine_port(storage_path: &Path) -> Option<u16> {
    let dir = engine_var_dir(storage_path);
    if let Some(port) = std::fs::read_to_string(dir.join(ENGINE_PORT_FILE))
        .ok()
        .and_then(|text| text.trim().parse().ok())
    {
        return Some(port);
    }
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).ok()?).ok()?;
    let url = url::Url::parse(config["topology"]["datacenters"]["default"]["public_url"].as_str()?)
        .ok()?;
    url.port()
}

/// `<storage>/.rivetkit/var/engine`.
fn engine_var_dir(storage_path: &Path) -> PathBuf {
    storage_path.join(".rivetkit").join("var").join("engine")
}

/// [`LocalEngine::start`] on storage whose engine ran on another guard port (see
/// [`recorded_engine_port`]). Nothing was started.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "this engine storage belongs to port {recorded}, not {requested}: engine 3 ties its token signing ring to the port, so a new port would refuse every gateway token; start it on port {recorded} again"
)]
pub struct EnginePortMismatch {
    /// The port the storage's engine first ran on.
    pub recorded: u16,
    /// The port this start asked for.
    pub requested: u16,
}

/// True when something holds `127.0.0.1:port`: it cannot be bound, or it accepts a
/// connection (a wildcard listener can share the port with a loopback bind).
pub fn loopback_port_in_use(port: u16) -> bool {
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));
    match std::net::TcpListener::bind(addr) {
        Err(e) => e.kind() == std::io::ErrorKind::AddrInUse,
        Ok(listener) => {
            drop(listener);
            std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
        }
    }
}

/// The three loopback ports an engine on `port` binds: guard, peer, metrics.
pub fn engine_ports(port: u16) -> [u16; 3] {
    [
        port,
        port.saturating_add(PEER_PORT_OFFSET),
        port.saturating_add(METRICS_PORT_OFFSET),
    ]
}

/// How to start a [`LocalEngine`].
#[derive(Debug, Clone)]
pub struct LocalEngineOptions {
    /// Guard port; the engine also binds `port + 1` (peer) and `port + 5` (metrics).
    pub port: u16,
    /// The storage path (rivetkit `RIVETKIT_STORAGE_PATH`); data lives under `<path>/.rivetkit`.
    pub storage_path: PathBuf,
    /// Admin token the engine accepts (`RIVET__AUTH__ADMIN_TOKEN`). Servers, registries
    /// and clients must present it. [`LocalEngineOptions::new`] generates a fresh one;
    /// embedded mode passes the persisted `secrets.json` token (`EngineConfig::token`).
    pub admin_token: crate::config::Secret,
    /// Engine binary; `None` resolves `RIVET_ENGINE_BINARY_PATH`, then the rivetkit cache,
    /// then downloads it when `auto_download`.
    pub binary_path: Option<PathBuf>,
    /// Allow downloading the engine binary.
    pub auto_download: bool,
    /// How long to wait for health and readiness.
    pub ready_timeout: Duration,
}

impl LocalEngineOptions {
    /// Defaults for `port` and `storage_path`, with a fresh random admin token.
    pub fn new(port: u16, storage_path: impl Into<PathBuf>) -> Self {
        Self {
            port,
            storage_path: storage_path.into(),
            admin_token: crate::config::Secret::new(crate::config::random_secret()),
            binary_path: None,
            auto_download: true,
            ready_timeout: Duration::from_secs(60),
        }
    }

    /// Use `token` as the engine admin token instead of the generated one.
    pub fn with_admin_token(mut self, token: crate::config::Secret) -> Self {
        self.admin_token = token;
        self
    }
}

/// A `rivet-engine` process this process spawned, with its own port, storage and
/// admin token.
///
/// It refuses to start when something already answers on the port: it never adopts
/// an engine it did not start, and never stops one that is not provably its own
/// orphan (see [`LocalEngine::start`]). [`LocalEngine::shutdown`] or drop stops it:
/// SIGTERM, up to 15 s, then SIGKILL. Only this child is ever signalled.
#[derive(Debug)]
pub struct LocalEngine {
    child: Option<Child>,
    endpoint: String,
    port: u16,
    admin_token: crate::config::Secret,
    storage_root: PathBuf,
    pid_file: PathBuf,
    stdout_log: PathBuf,
    stderr_log: PathBuf,
}

impl LocalEngine {
    /// Spawn the engine and wait until it is healthy and ready.
    ///
    /// The engine runs in its own process group, so a signal to this process does not
    /// reach it, and Drop does not run on SIGTERM or SIGKILL (G30). An engine left
    /// behind that way is reclaimed here: when `<storage>/.rivetkit/var/engine/`
    /// [`ENGINE_PID_FILE`] names a live process whose command line is `rivet-engine
    /// --config <this storage's config.json>` and whose recorded owner process is
    /// gone, it gets SIGTERM, then SIGKILL after 15 s, before the port check. An engine
    /// whose owner is still alive, or any other process, is never signalled.
    ///
    /// Storage that already ran an engine on another port is refused first, with
    /// [`EnginePortMismatch`] (see [`recorded_engine_port`]); a port something else
    /// holds after the reclaim fails at once instead of waiting out the ready timeout.
    /// The first start that becomes ready records its port in [`ENGINE_PORT_FILE`].
    pub async fn start(options: LocalEngineOptions) -> Result<Self> {
        let endpoint = format!("http://127.0.0.1:{}", options.port);
        let storage_root = options.storage_path.join(".rivetkit");
        let engine_dir = engine_var_dir(&options.storage_path);
        let config_path = engine_dir.join("config.json");
        let pid_file = engine_dir.join(ENGINE_PID_FILE);
        let pid_file_for_reclaim = pid_file.clone();
        let config_for_reclaim = config_path.clone();
        if let Some(recorded) = recorded_engine_port(&options.storage_path)
            && recorded != options.port
        {
            return Err(EnginePortMismatch {
                recorded,
                requested: options.port,
            }
            .into());
        }
        tokio::task::spawn_blocking(move || {
            reclaim_orphan(&pid_file_for_reclaim, &config_for_reclaim)
        })
        .await
        .context("join orphan reclaim")??;
        // Loopback only: an `HTTP_PROXY` from the environment must never see these.
        let http = reqwest::Client::builder()
            .no_proxy()
            .build()
            .context("build http client")?;
        if probe_health(&http, &endpoint).await.is_some() {
            bail!(
                "an engine already answers on {endpoint}; refusing to start or adopt one this process did not spawn"
            );
        }
        // After the reclaim nothing of ours holds the ports: anything that does would
        // keep the engine from binding, and it would only fail its ready timeout.
        if let Some(busy) = engine_ports(options.port)
            .into_iter()
            .find(|p| loopback_port_in_use(*p))
        {
            bail!(
                "port {busy} on 127.0.0.1 is already in use; the engine binds {:?}",
                engine_ports(options.port)
            );
        }
        let binary = resolve_engine_binary(&endpoint, &options).await?;
        let logs_dir = storage_root.join("var").join("logs").join("rivet-engine");
        for dir in [
            &engine_dir,
            &logs_dir,
            &engine_dir.join("depot").join("log"),
        ] {
            std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        std::fs::write(&config_path, engine_config_json(&endpoint, options.port)?)
            .with_context(|| format!("write {}", config_path.display()))?;
        let stamp = log_stamp();
        let stdout_log = logs_dir.join(format!("engine-{stamp}-stdout.log"));
        let stderr_log = logs_dir.join(format!("engine-{stamp}-stderr.log"));
        let mut command = Command::new(&binary);
        command
            .arg("--config")
            .arg(&config_path)
            .envs(engine_env(
                options.port,
                &storage_root,
                options.admin_token.expose(),
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::from(open_append(&stdout_log)?))
            .stderr(Stdio::from(open_append(&stderr_log)?));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        let child = command
            .spawn()
            .with_context(|| format!("spawn {}", binary.display()))?;
        let pid = child.id();
        if let Err(e) = std::fs::write(&pid_file, format!("{pid} {}\n", std::process::id())) {
            tracing::warn!(error = %e, path = %pid_file.display(), "could not record the engine pid");
        }
        let mut engine = Self {
            child: Some(child),
            endpoint,
            port: options.port,
            admin_token: options.admin_token,
            storage_root,
            pid_file,
            stdout_log,
            stderr_log,
        };
        if let Err(e) = engine.wait_until_ready(&http, options.ready_timeout).await {
            engine.stop_blocking();
            return Err(e.context(format!(
                "engine pid {pid} never became ready (logs: {})",
                engine.stderr_log.display()
            )));
        }
        let port_file = engine_dir.join(ENGINE_PORT_FILE);
        if !port_file.exists()
            && let Err(e) = std::fs::write(&port_file, format!("{}\n", options.port))
        {
            tracing::warn!(error = %e, path = %port_file.display(), "could not record the engine port");
        }
        tracing::info!(pid, endpoint = %engine.endpoint, "local engine ready");
        Ok(engine)
    }

    /// The engine's exit status once it has exited (it is reaped then), `None` while
    /// it runs or after [`LocalEngine::shutdown`]. Never blocks.
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        match self.child.as_mut() {
            Some(child) => child.try_wait(),
            None => Ok(None),
        }
    }

    async fn wait_until_ready(&mut self, http: &reqwest::Client, timeout: Duration) -> Result<()> {
        let end = Instant::now() + timeout;
        loop {
            if let Some(child) = self.child.as_mut()
                && let Some(status) = child.try_wait()?
            {
                bail!("engine exited with {status} before becoming healthy");
            }
            if let Some(health) = probe_health(http, &self.endpoint).await {
                if health
                    .get("architecture")
                    .and_then(Value::as_str)
                    .is_some_and(|a| a != "v3")
                {
                    bail!("engine on {} is not a v3 engine: {health}", self.endpoint);
                }
                break;
            }
            if Instant::now() >= end {
                bail!("engine did not become healthy within {timeout:?}");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let url = format!(
            "{}/actors?namespace={LOCAL_ENGINE_NAMESPACE}&name=rivetkit-readiness-probe&key=probe",
            self.endpoint
        );
        loop {
            let ok = http
                .get(&url)
                .bearer_auth(self.admin_token.expose())
                .timeout(Duration::from_secs(10))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success());
            if ok {
                return Ok(());
            }
            if Instant::now() >= end {
                bail!("engine did not become ready within {timeout:?}");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// `http://127.0.0.1:<port>`.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Guard port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The admin token this engine accepts; give it to `ServeConfig::token` and clients.
    pub fn admin_token(&self) -> &crate::config::Secret {
        &self.admin_token
    }

    /// Process id while running.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    /// `<storage_path>/.rivetkit`.
    pub fn storage_root(&self) -> &Path {
        &self.storage_root
    }

    /// Engine stderr log (the useful one when something fails).
    pub fn stderr_log(&self) -> &Path {
        &self.stderr_log
    }

    /// Engine stdout log.
    pub fn stdout_log(&self) -> &Path {
        &self.stdout_log
    }

    /// Stop the engine (SIGTERM, up to 15 s, then SIGKILL) without blocking the runtime.
    pub async fn shutdown(mut self) -> Result<()> {
        let Some(child) = self.child.take() else {
            return Ok(());
        };
        let pid_file = self.pid_file.clone();
        tokio::task::spawn_blocking(move || stop_child(child, &pid_file))
            .await
            .context("join engine stop")?;
        Ok(())
    }

    fn stop_blocking(&mut self) {
        if let Some(child) = self.child.take() {
            stop_child(child, &self.pid_file);
        }
    }
}

impl Drop for LocalEngine {
    fn drop(&mut self) {
        self.stop_blocking();
    }
}

fn stop_child(mut child: Child, pid_file: &Path) {
    stop_child_process(&mut child);
    // Only remove the record if it still names this child.
    if read_pid_file(pid_file).is_some_and(|(engine, _)| engine == child.id()) {
        let _ = std::fs::remove_file(pid_file);
    }
}

fn stop_child_process(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let pid = child.id();
    signal(pid, "TERM");
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    tracing::warn!(pid, "engine ignored SIGTERM for 15 s; killing it");
    let _ = child.kill();
    let _ = child.wait();
}

/// Send `SIG<name>` to `pid` through kill(1): std only offers SIGKILL for children,
/// and `unsafe` (libc) is forbidden.
fn signal(pid: u32, name: &str) {
    let _ = Command::new("kill")
        .arg(format!("-{name}"))
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// `(engine pid, owner pid)` from a pid file. Files from before the owner field read
/// as owner `None`.
fn read_pid_file(path: &Path) -> Option<(u32, Option<u32>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut parts = text.split_whitespace();
    let engine = parts.next()?.parse().ok()?;
    let owner = parts.next().and_then(|p| p.parse().ok());
    Some((engine, owner))
}

/// The full command line of a live process, or `None` when there is no such process.
fn process_command(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-ww", "-o", "command=", "-p", &pid.to_string()])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// True when `command` is a rivet-engine started with `--config <config_path>`.
fn is_our_engine(command: &str, config_path: &Path) -> bool {
    command.contains("rivet-engine")
        && command.contains(&format!("--config {}", config_path.display()))
}

/// Stop an engine orphaned by a previous run on this storage (see [`LocalEngine::start`]).
fn reclaim_orphan(pid_file: &Path, config_path: &Path) -> Result<()> {
    let Some((pid, owner)) = read_pid_file(pid_file) else {
        return Ok(());
    };
    let Some(command) = process_command(pid) else {
        let _ = std::fs::remove_file(pid_file);
        return Ok(());
    };
    if !is_our_engine(&command, config_path) {
        // The pid was reused by an unrelated process: the record is stale.
        let _ = std::fs::remove_file(pid_file);
        return Ok(());
    }
    if let Some(owner) = owner
        && (owner == std::process::id() || process_command(owner).is_some())
    {
        bail!(
            "engine pid {pid} on this storage is still owned by live process {owner}; refusing to start a second engine (stop that process first)"
        );
    }
    tracing::warn!(
        pid,
        "stopping an engine orphaned by a previous run on this storage"
    );
    signal(pid, "TERM");
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end && process_command(pid).is_some() {
        std::thread::sleep(Duration::from_millis(100));
    }
    if process_command(pid).is_some() {
        tracing::warn!(pid, "orphaned engine ignored SIGTERM for 15 s; killing it");
        signal(pid, "KILL");
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end && process_command(pid).is_some() {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if process_command(pid).is_some() {
        bail!("orphaned engine pid {pid} did not exit");
    }
    let _ = std::fs::remove_file(pid_file);
    Ok(())
}

/// The environment a local engine on `port` with data under `storage_root` and admin
/// token `admin_token` runs with (rivetkit-engine-process `engine_env`, with the metrics
/// port moved to `port + 5`).
pub fn engine_env(port: u16, storage_root: &Path, admin_token: &str) -> Vec<(String, String)> {
    let depot = storage_root.join("var").join("engine").join("depot");
    let pairs: [(&str, String); 21] = [
        ("RIVET__AUTH__ADMIN_TOKEN", admin_token.to_owned()),
        ("RIVET__GUARD__HOST", "127.0.0.1".to_owned()),
        ("RIVET__GUARD__PORT", port.to_string()),
        ("RIVET__API_PEER__HOST", "127.0.0.1".to_owned()),
        (
            "RIVET__API_PEER__PORT",
            port.saturating_add(PEER_PORT_OFFSET).to_string(),
        ),
        ("RIVET__METRICS__HOST", "127.0.0.1".to_owned()),
        (
            "RIVET__METRICS__PORT",
            port.saturating_add(METRICS_PORT_OFFSET).to_string(),
        ),
        (
            "RIVET__STORAGE__LOG_DIRECTORY",
            depot.join("log").to_string_lossy().into_owned(),
        ),
        (
            "RIVET__OBJECT_STORAGE__PRIMARY__FILE_SYSTEM__PATH",
            depot.join("objects").to_string_lossy().into_owned(),
        ),
        ("RIVET__STORAGE__BALANCING__ENABLED", "false".to_owned()),
        ("RIVET__STORAGE__AUTO_PROVISION_PARTITIONS", "1".to_owned()),
        (
            "RIVET__STORAGE__LOG_SHARD__MAX_DISK_BYTES",
            (4u64 << 30).to_string(),
        ),
        (
            "RIVET__STORAGE__COLD_STORE__DISK_CACHE_BYTES",
            "0".to_owned(),
        ),
        ("RIVET__STORAGE__WRITE_TO_COLD_AFTER_MS", "5000".to_owned()),
        ("RIVET__STORAGE__STOP_TIMEOUT_MS", "6000".to_owned()),
        ("RIVET__STORAGE__STOP_TEARDOWN_MS", "2000".to_owned()),
        ("RIVET__PEGBOARD__NODE_LOST_THRESHOLD", "7000".to_owned()),
        ("RIVET__RUNTIME__WORKER_SHUTDOWN_DURATION", "1".to_owned()),
        ("RIVET__RUNTIME__GUARD_SHUTDOWN_DURATION", "1".to_owned()),
        ("RIVET__RUNTIME__FORCE_SHUTDOWN_DURATION", "10".to_owned()),
        ("RUST_LOG", "warn".to_owned()),
    ];
    pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

/// The topology file the engine cannot take from env.
pub fn engine_config_json(endpoint: &str, port: u16) -> Result<String> {
    let peer = format!("http://127.0.0.1:{}", port.saturating_add(PEER_PORT_OFFSET));
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "topology": {
            "datacenter_label": 1,
            "datacenters": {
                "default": {
                    "datacenter_label": 1,
                    "is_leader": true,
                    "public_url": endpoint,
                    "peer_url": peer,
                }
            }
        }
    }))?)
}

async fn resolve_engine_binary(endpoint: &str, options: &LocalEngineOptions) -> Result<PathBuf> {
    let mut cfg = rivetkit_engine_process::EngineResolverConfig::from_parts(
        endpoint,
        options.binary_path.clone(),
        Some("127.0.0.1".to_owned()),
        Some(options.port),
        options.auto_download,
    );
    cfg.version = ENGINE_VERSION.to_owned();
    rivetkit_engine_process::resolve_engine_binary_path(&cfg)
        .await
        .context("resolve rivet-engine binary (set RIVET_ENGINE_BINARY_PATH)")
}

async fn probe_health(http: &reqwest::Client, endpoint: &str) -> Option<Value> {
    let r = http
        .get(format!("{endpoint}/health"))
        .timeout(Duration::from_secs(1))
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    Some(r.json::<Value>().await.unwrap_or(Value::Null))
}

fn open_append(path: &Path) -> Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))
}

fn log_stamp() -> String {
    format!("{}-{}", crate::auth::now_ms(), std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_error_round_trips_in_process() {
        let e = user_error("not_your_turn", "Wait for your turn.");
        let parsed = parse_rivet_error(&e.context("outer"));
        assert_eq!(
            parsed,
            Some(UserError {
                group: "user".into(),
                code: "not_your_turn".into(),
                message: "Wait for your turn.".into(),
                metadata: None,
            })
        );
        let m = user_error_with_metadata("bad_move", "Illegal.", &serde_json::json!({"seat": 2}));
        let parsed = parse_rivet_error(&m);
        assert_eq!(
            parsed.and_then(|p| p.metadata),
            Some(serde_json::json!({"seat": 2}))
        );
    }

    #[test]
    fn parses_connection_format() {
        let meta = serde_json::json!({"a": [1, "x, y"], "b": null});
        let text = format!("RPC Error(user/bad_move): Nope, not now, {meta:#}");
        let p = parse_rivet_error_str(&text);
        assert_eq!(
            p,
            Some(UserError {
                group: "user".into(),
                code: "bad_move".into(),
                message: "Nope, not now".into(),
                metadata: Some(meta),
            })
        );
        let bare = format!(
            "ctx: RPC Error(core/internal_error): An internal error occurred, {:#}",
            Value::Null
        );
        let p = parse_rivet_error_str(&bare);
        assert_eq!(p.as_ref().map(|p| p.code.as_str()), Some("internal_error"));
        assert_eq!(
            p.as_ref().map(|p| p.message.as_str()),
            Some("An internal error occurred")
        );
        assert_eq!(p.and_then(|p| p.metadata), None);
    }

    #[test]
    fn parses_stateless_format() {
        let meta: Option<Value> = Some(serde_json::json!({
            "s": "q\"uo\\te\n\u{0007}é",
            "n": -3,
            "f": 1.5,
            "big": u64::MAX,
            "t": true,
            "z": null,
            "arr": [[], {}, "x, metadata=y"],
        }));
        let text = format!("action failed (user/bad_move): Nope, metadata=x, metadata={meta:?}");
        let p = parse_rivet_error_str(&text);
        assert_eq!(
            p,
            Some(UserError {
                group: "user".into(),
                code: "bad_move".into(),
                message: "Nope, metadata=x".into(),
                metadata: meta,
            })
        );
        let none: Option<Value> = None;
        let text =
            format!("action failed (actor/not_found): The actor does not exist, metadata={none:?}");
        let p = parse_rivet_error_str(&text);
        assert_eq!(
            p.as_ref().map(|p| (p.group.as_str(), p.code.as_str())),
            Some(("actor", "not_found"))
        );
        assert_eq!(p.and_then(|p| p.metadata), None);
        assert_eq!(parse_rivet_error_str("something else"), None);
        assert!(parse_rivet_error_str("action failed (garbage").is_none());
    }

    #[test]
    fn user_error_helpers() {
        let p = UserError {
            group: "user".into(),
            code: "x".into(),
            message: String::new(),
            metadata: None,
        };
        assert!(p.is_user() && p.is("x") && !p.is("y"));
    }

    #[test]
    fn decode_params_maps_to_bad_request() {
        #[derive(serde::Deserialize, Debug, PartialEq, Default)]
        #[serde(default, rename_all = "camelCase")]
        struct P {
            token: Option<String>,
            player_id: Option<String>,
        }
        let ok: P =
            decode_params(&serde_json::json!({"token": "t", "playerId": "p"})).unwrap_or_default();
        assert_eq!(ok.player_id.as_deref(), Some("p"));
        let null: Result<P> = decode_params(&Value::Null);
        assert!(null.is_ok());
        let bad: Result<P> = decode_params(&serde_json::json!({"token": 5}));
        let code = bad
            .err()
            .and_then(|e| parse_rivet_error(&e))
            .map(|p| p.code);
        assert_eq!(code.as_deref(), Some("bad_request"));
    }

    #[test]
    fn json_table_names_are_validated() {
        assert!(JsonTable::new("game_state").is_ok());
        assert!(JsonTable::new("_x1").is_ok());
        for keyword in ["order", "group", "select"] {
            let t = JsonTable::new(keyword).unwrap_or_else(|_| unreachable!());
            assert!(t.schema_sql().contains(&format!("\"{keyword}\" (")));
        }
        for bad in [
            "",
            "1x",
            "a-b",
            "a b",
            "x;DROP",
            "x\"y",
            &"a".repeat(65),
            "sqlite_master",
            "SQLite_x",
            "_rivet_meta",
            "_RIVET_x",
        ] {
            assert!(JsonTable::new(bad).is_err(), "{bad}");
        }
        let t = JsonTable::new("doc").unwrap_or_else(|_| unreachable!());
        let st = t
            .save_statement("k", &serde_json::json!({"a": 1}))
            .unwrap_or_else(|_| unreachable!());
        assert!(
            st.sql
                .starts_with("INSERT INTO \"doc\" (key, json, updated_at)")
        );
        assert!(st.sql.contains("ON CONFLICT(key) DO UPDATE"));
        assert!(
            matches!(st.params.as_deref(), Some([BindParam::Text(k), BindParam::Text(j), BindParam::Integer(_)]) if k == "k" && j == r#"{"a":1}"#)
        );
    }

    #[tokio::test]
    async fn lane_serializes() {
        let lane = Arc::new(ActorLane::new());
        let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let mut tasks = Vec::new();
        for i in 0..4 {
            let (lane, log) = (lane.clone(), log.clone());
            tasks.push(tokio::spawn(async move {
                lane.run(async {
                    log.lock().push(("start", i));
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    log.lock().push(("end", i));
                })
                .await;
            }));
        }
        for t in tasks {
            let _ = t.await;
        }
        let log = log.lock();
        for pair in log.chunks(2) {
            assert_eq!(pair[0].0, "start");
            assert_eq!(pair[1], ("end", pair[0].1));
        }
    }

    #[tokio::test]
    async fn wait_ready_retries_then_gives_up() {
        let mut n = 0;
        let v = wait_ready(Duration::from_secs(5), || {
            n += 1;
            let k = n;
            async move {
                if k < 3 {
                    Err(anyhow!("Connection refused"))
                } else {
                    Ok(k)
                }
            }
        })
        .await;
        assert_eq!(v.ok(), Some(3));
        let e = wait_ready(Duration::from_millis(300), || async {
            Err::<(), _>(anyhow!("no_runner_config_configured"))
        })
        .await;
        assert!(e.as_ref().is_err_and(is_startup_error));
        // A user error is final: one attempt, no waiting for the deadline.
        let started = Instant::now();
        let mut tries = 0;
        let e = wait_ready(Duration::from_secs(30), || {
            tries += 1;
            async { Err::<(), _>(user_error("unauthorized", "bad token")) }
        })
        .await;
        assert_eq!(tries, 1);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            e.err().and_then(|e| parse_rivet_error(&e)).map(|p| p.code),
            Some("unauthorized".to_owned())
        );
        // The per-attempt bound is clamped to the time left.
        let started = Instant::now();
        let e = wait_ready(Duration::from_millis(300), || async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok::<_, anyhow::Error>(())
        })
        .await;
        assert!(e.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        let t = bounded("x", Duration::from_millis(10), async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(())
        })
        .await;
        assert!(t.is_err_and(|e| e.downcast_ref::<CallError>().is_some()));
    }

    #[test]
    fn pid_file_and_command_matching() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pid_file = dir.path().join(ENGINE_PID_FILE);
        std::fs::write(&pid_file, "123 456\n")?;
        assert_eq!(read_pid_file(&pid_file), Some((123, Some(456))));
        std::fs::write(&pid_file, "123")?;
        assert_eq!(read_pid_file(&pid_file), Some((123, None)));
        std::fs::write(&pid_file, "junk")?;
        assert_eq!(read_pid_file(&pid_file), None);
        let cfg = Path::new("/s/.rivetkit/var/engine/config.json");
        assert!(is_our_engine(
            "/Users/x/.rivetkit/engine/v3/3.1.0/rivet-engine-aarch64-apple-darwin --config /s/.rivetkit/var/engine/config.json",
            cfg
        ));
        assert!(!is_our_engine(
            "rivet-engine --config /other/.rivetkit/var/engine/config.json",
            cfg
        ));
        assert!(!is_our_engine("/bin/zsh", cfg));
        assert!(process_command(std::process::id()).is_some());
        Ok(())
    }

    #[test]
    fn the_recorded_port_comes_from_the_record_then_the_topology() -> Result<()> {
        let dir = tempfile::tempdir()?;
        assert_eq!(recorded_engine_port(dir.path()), None);
        let engine_dir = engine_var_dir(dir.path());
        std::fs::create_dir_all(&engine_dir)?;
        std::fs::write(
            engine_dir.join("config.json"),
            engine_config_json("http://127.0.0.1:6890", 6890)?,
        )?;
        assert_eq!(recorded_engine_port(dir.path()), Some(6890));
        std::fs::write(engine_dir.join(ENGINE_PORT_FILE), "6910\n")?;
        assert_eq!(recorded_engine_port(dir.path()), Some(6910));
        std::fs::write(engine_dir.join(ENGINE_PORT_FILE), "junk")?;
        assert_eq!(recorded_engine_port(dir.path()), Some(6890));
        Ok(())
    }

    /// Another port on storage that ran an engine is refused before anything runs.
    #[tokio::test]
    async fn a_new_port_on_used_storage_is_refused() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let engine_dir = engine_var_dir(dir.path());
        std::fs::create_dir_all(&engine_dir)?;
        std::fs::write(engine_dir.join(ENGINE_PORT_FILE), "6910\n")?;
        let started = Instant::now();
        let err = LocalEngine::start(LocalEngineOptions::new(6912, dir.path()))
            .await
            .err()
            .context("started on another port")?;
        assert_eq!(
            err.downcast_ref::<EnginePortMismatch>(),
            Some(&EnginePortMismatch {
                recorded: 6910,
                requested: 6912
            }),
            "{err:#}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            !engine_dir.join("config.json").exists(),
            "topology rewritten"
        );
        Ok(())
    }

    /// A stale pid record (its engine long gone) plus a port held by someone else
    /// fails at once, not after the 60 s ready timeout (W3 operations review).
    #[tokio::test]
    async fn a_held_port_fails_fast_after_a_stale_pid_record() -> Result<()> {
        let held = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let port = held.local_addr()?.port();
        let dir = tempfile::tempdir()?;
        let engine_dir = engine_var_dir(dir.path());
        std::fs::create_dir_all(&engine_dir)?;
        std::fs::write(engine_dir.join(ENGINE_PID_FILE), "99999991 99999992\n")?;
        let started = Instant::now();
        let err = LocalEngine::start(LocalEngineOptions::new(port, dir.path()))
            .await
            .err()
            .context("started on a held port")?;
        assert!(
            format!("{err:#}").contains(&format!("port {port} on 127.0.0.1 is already in use")),
            "{err:#}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert!(
            !engine_dir.join(ENGINE_PID_FILE).exists(),
            "stale record kept"
        );
        assert!(loopback_port_in_use(port));
        drop(held);
        assert!(!loopback_port_in_use(port));
        Ok(())
    }

    /// A fake `rivet-engine` (a shell loop) stands in for an orphan: reclaim stops it
    /// only when its owner is gone, and never touches a process that is not ours.
    #[cfg(unix)]
    #[test]
    fn reclaim_orphan_stops_only_our_orphans() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let engine_dir = dir.path().join(".rivetkit").join("var").join("engine");
        std::fs::create_dir_all(&engine_dir)?;
        let config = engine_dir.join("config.json");
        let pid_file = engine_dir.join(ENGINE_PID_FILE);
        let script = dir.path().join("rivet-engine");
        std::fs::write(
            &script,
            "#!/bin/sh
trap 'exit 0' TERM
while :; do sleep 0.1; done
",
        )?;
        let spawn = || -> Result<Child> {
            Ok(Command::new("/bin/sh")
                .arg(&script)
                .arg("--config")
                .arg(&config)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?)
        };
        // Wait until ps shows the script's arguments (exec is not instant).
        let wait_visible = |pid: u32| {
            let end = Instant::now() + Duration::from_secs(5);
            while Instant::now() < end {
                if process_command(pid).is_some_and(|c| is_our_engine(&c, &config)) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        };

        // Owner alive (this process): refused, and the engine keeps running.
        let mut live = spawn()?;
        assert!(wait_visible(live.id()));
        std::fs::write(&pid_file, format!("{} {}", live.id(), std::process::id()))?;
        let refused = reclaim_orphan(&pid_file, &config);
        assert!(refused.is_err_and(|e| e.to_string().contains("still owned")));
        assert!(matches!(live.try_wait(), Ok(None)), "not signalled");
        let _ = live.kill();
        let _ = live.wait();

        // Owner gone (99999999 is above any pid): stopped, record removed.
        let orphan = spawn()?;
        let pid = orphan.id();
        assert!(wait_visible(pid));
        std::fs::write(&pid_file, format!("{pid} 99999999"))?;
        let reaper = std::thread::spawn(move || {
            let mut orphan = orphan;
            orphan.wait()
        });
        reclaim_orphan(&pid_file, &config)?;
        assert!(process_command(pid).is_none());
        assert!(!pid_file.exists());
        let _ = reaper.join();

        // A record naming an unrelated live process is stale: dropped, nothing signalled.
        std::fs::write(&pid_file, format!("{} 99999999", std::process::id()))?;
        reclaim_orphan(&pid_file, &config)?;
        assert!(!pid_file.exists());
        Ok(())
    }

    #[test]
    fn engine_env_and_config() -> Result<()> {
        let env = engine_env(6701, Path::new("/s/.rivetkit"), "tok-123");
        let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        assert_eq!(get("RIVET__AUTH__ADMIN_TOKEN").as_deref(), Some("tok-123"));
        let opts = LocalEngineOptions::new(6701, "/s");
        assert_eq!(opts.admin_token.js_len(), 43);
        assert_ne!(opts.admin_token.expose(), DEFAULT_ENGINE_TOKEN);
        assert_ne!(
            opts.admin_token,
            LocalEngineOptions::new(6701, "/s").admin_token,
            "fresh token per engine"
        );
        assert!(!format!("{opts:?}").contains(opts.admin_token.expose()));
        assert_eq!(get("RIVET__GUARD__PORT").as_deref(), Some("6701"));
        assert_eq!(get("RIVET__API_PEER__PORT").as_deref(), Some("6702"));
        assert_eq!(get("RIVET__METRICS__PORT").as_deref(), Some("6706"));
        assert_eq!(
            get("RIVET__STORAGE__LOG_DIRECTORY").as_deref(),
            Some("/s/.rivetkit/var/engine/depot/log")
        );
        let cfg: Value = serde_json::from_str(&engine_config_json("http://127.0.0.1:6701", 6701)?)?;
        assert_eq!(
            cfg["topology"]["datacenters"]["default"]["peer_url"],
            "http://127.0.0.1:6702"
        );
        assert_eq!(
            cfg["topology"]["datacenters"]["default"]["public_url"],
            "http://127.0.0.1:6701"
        );
        Ok(())
    }
}
