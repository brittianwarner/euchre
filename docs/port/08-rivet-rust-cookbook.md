# 08 — Rivet Rust cookbook (rivetkit 3.1.0, verified)

Every snippet here was compiled and run against a real Rivet engine 3.1.0. Last full run: 2026-11-10, **59/59 checks passed** on a fresh store. There were also about 60 recorded observations and a JS rivetkit **3.1.0** client run in all three encodings. Where a statement comes from reading source rather than from a run, it says **(source)**.

- Verification crate (session scratch, may be deleted): `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/rivet-cookbook/` (`src/proto.rs` wire types, `src/actors.rs` server, `src/main.rs` scenario driver; logs `run-*.log`, `run-final.log`)
- JS interop scripts: `.../scratchpad/js-interop/{interop.mjs,welcome.mjs}`, logs `js-{bare,json,cbor}.log`
- Crate sources quoted below live in `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/<crate>-3.1.0/`. Short forms: `rk` = rivetkit, `core` = rivetkit-core, `cl` = rivetkit-client, `ep` = rivetkit-engine-process.

Contents:

0. Versions, deps, run command
1. Mental model and API map
2. Recipe: persist a large serde struct in SQLite (and why not KV)
3. Recipe: schedules
4. Recipe: events (`conn.send` vs `ctx.emit`, iterating conns)
5. Recipe: actor-to-actor calls (`ctx.client()`, `create_with_input`)
6. Recipe: background work and locking rules
7. Recipe: actor input
8. Recipe: errors that reach the client
9. Recipe: the Rust client (status, reconnect, timeouts, typed events)
10. Recipe: `ServeConfig` for local dev, an external engine, and Railway
11. Recipe: JS client interop (wire shapes, positional args)
12. Sleep, wake, and durability
13. Gotcha index
14. Porting map for the euchre actors
15. Source reference index

---

## 0. Versions, deps, run command

Versions used:

- rivetkit `3.1.0`, rivetkit-client `3.1.0`, rivet-error `=3.1.0`, engine binary `3.1.0`
- JS client: `rivetkit@3.1.0` (bun 1.4.4)
- The Rust docs fetched for this were https://rivet.dev/actors/docs/quickstart/rust/ and the client docs on docs.rs/rivetkit-client.

### Cargo.toml (exact, the one that built)

```toml
[package]
name = "rivet-cookbook"
version = "0.0.1"
edition = "2024"

[dependencies]
rivetkit = "3.1.0"            # default feature `sqlite` -> rivetkit-core/sqlite-remote
rivetkit-client = "3.1.0"     # also re-exported as rivetkit::client
rivet-error = "=3.1.0"        # only for RivetError/RivetErrorKind::Dynamic (see section 8)
tokio = { version = "1", features = ["full"] }
tokio-util = "0.7"            # CancellationToken for serve_with_config
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
async-trait = "0.1"           # the Actor trait is #[async_trait]
parking_lot = "0.12"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

The euchre-rune workspace already pins `rivetkit = "3.1.0"` and `rivetkit-client = "3.1.0"` (`/Users/brittianwarner/goods/euchre-rune/Cargo.toml`). Add `rivet-error = "=3.1.0"` to `[workspace.dependencies]` for `apps/server`. The rivet.dev quickstart equivalent is `cargo add rivetkit anyhow async-trait; cargo add serde --features derive; cargo add tokio --features full`.

### Imports that compile

```rust
use rivetkit::prelude::*;   // Actor, Action, Event, Handler, Ctx, ConnCtx, Registry, ...
use rivetkit::{ActorConfig, ServeConfig, TypedActorConnection, TypedActorHandle, TypedClientExt};
use rivetkit::{ActorKeySegment, BindParam, ColumnValue, SqliteBatchStatement, SqliteDb};
use rivetkit::client::{Client, ClientConfig, ConnectionStatus, EncodingKind, GetOptions, GetOrCreateOptions};
use tokio::sync::Mutex as AsyncMutex;
```

### Run command (isolated engine, used for every run)

```bash
cd <crate>
RIVETKIT_STORAGE_PATH=<scratch>/rivet-store-cookbook/final \
RIVET_RUN_ENGINE_PORT=6620 RIVET_POOL=cookbook \
RIVET_ENGINE_BINARY_PATH=$HOME/.rivetkit/engine/v3/3.1.0/rivet-engine-aarch64-apple-darwin \
RUST_LOG=warn,rivetkit_engine_process=info ONLY=input,sql,events \
perl -e 'alarm 480; exec @ARGV' ./target/debug/rivet-cookbook
```

How the run behaves:

- With these variables, `ServeConfig::from_env()` gives `endpoint=http://127.0.0.1:6620 ns=default pool=cookbook token=Some("default") spawn=Always version=1`.
- The engine is spawned and healthy in about 3 s. The first actor ping succeeds about 3–6 s after start; before that the client sees `Connection refused` and then `503 Service Unavailable` metadata errors. Section 9 has a startup loop for this.
- `RIVETKIT_STORAGE_PATH` moves the engine data root to `$RIVETKIT_STORAGE_PATH/.rivetkit`. Engine logs go to `.rivetkit/var/logs/rivet-engine/engine-*-std{out,err}.log`. Use a separate storage path per project: the default `~/.rivetkit` holds a single data lock.

---

## 1. Mental model and API map

### The Actor trait (`rk/src/actor.rs:36-137`)

```rust
#[async_trait]
impl Actor for Table {
    type Input = TableInput;          // DeserializeOwned + Default; from create_with_input
    type ConnParams = TableParams;    // DeserializeOwned + Default; from client params
    type Action = rivetkit::Raw;      // legacy enum API; always Raw with the trait API
    type Actions = (GetState, Info, /* ... */);   // FLAT tuple of 1..=128 Action types
    type Events = (StateChanged, Whisper /* ... */);   // () or tuple of 1..=16 Event types

    async fn create(ctx: &Ctx<Self>) -> Result<Self> { /* runs on EVERY wake */ }
    // optional hooks (defaults are no-ops):
    // on_create (first boot only), on_start (every wake), on_before_connect(&params),
    // on_connect(conn), on_disconnect(conn), on_subscribe(conn, event_name),
    // on_sleep, on_destroy, run, on_fetch, on_websocket
}
```

- An action is a serde type with `impl Action { type Output; const NAME: &'static str }`.
- A handler is `impl Handler<A> for Table { async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: A) -> Result<A::Output> }`. The future must be `Send + 'static`.
- An event is a serde type with `impl Event { const NAME }`.
- A small macro keeps this terse (`src/proto.rs:53-60`):

```rust
macro_rules! action {
    ($ty:ty, $name:literal, $out:ty) => {
        impl Action for $ty { type Output = $out; const NAME: &'static str = $name; }
    };
}
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppendLog { pub text: String, pub repeat: u32 }
action!(AppendLog, "appendLog", u64);
```

**Gotcha G1.** `ActionSet` is implemented only for flat tuples. A nested tuple such as `((A, B), (C, D))` does not compile ("trait ActionSet not implemented"). The limits are 128 actions (`rk/src/action.rs:13`, `TUPLE_ARITY_MAX`) and 16 events.

### Hook order (observed)

| Situation | Hooks that run |
|---|---|
| First boot of a key | `create` → `on_create` → `on_start` |
| Every later wake | `create` → `on_start` (no `on_create`) |
| WebSocket connect | `on_before_connect(&params)` → `on_connect(conn)`; `on_disconnect` on close |
| **Stateless HTTP action** (`handle.call(..)`) | `on_before_connect` → `on_connect` → action → `on_disconnect`, every call (a temporary conn) |
| Sleep (idle, `ctx.sleep()`, registry shutdown) | `on_sleep` |
| Actor sleeps while a client is connected | new conn id, `on_connect` again; **no `on_disconnect` for the old conn** |

### Ctx methods (`rk/src/context.rs`)

| Method | Line | Notes |
|---|---|---|
| `conn() -> Option<&ConnCtx>` | 113 | `None` for scheduled and internal dispatch; `Some` for WebSocket and stateless HTTP calls |
| `input() -> Result<A::Input>` | 129 | |
| `actor_id()`, `name()`, `key() -> &ActorKey`, `region()` | 136-148 | key segments are `ActorKeySegment::{String, Number}` |
| `kv()` | 156 | **deprecated in 3.0** ("Store data in SQLite") |
| `sql() -> &SqliteDb` | 160 | remote SQLite, one database per actor |
| `schedule()`, `cron()` | 164-168 | |
| `keep_awake(fut)`, `keep_awake_region()` | 175-185 | blocks idle sleep while held |
| `register_task(fut)`, `wait_until(fut)` | 192, 264 | do **not** block idle sleep |
| `abort_signal() -> CancellationToken`, `aborted()` | 198-203 | |
| `sleep()`, `destroy()`, `stop_with_error(msg)` | 231-244 | |
| `set_prevent_sleep(bool)` | 251 | |
| `broadcast(name, &E)`, `emit(E)` | 268-274 | subscribed conns only |
| `conns() -> ConnIter`, `conns_vec()` | 278-285 | `conns()` holds a read lock |
| `disconnect_conn(id)`, `disconnect_conns(pred)` | 289-293 | |
| `client() -> Result<Client>` | 306 | see G27 |

ConnCtx methods (`rk/src/context.rs:513-539`): `id()`, `is_hibernatable()`, `params() -> Result<ConnParams>`, `send(name, &E)`, `disconnect(reason)`. `inner()` returns core `ConnHandle`, which has raw `state()` / `set_state(Vec<u8>)` bytes **(source, untested)**.

---

## 2. Recipe: persist a large serde struct in SQLite

Verified results:

- A 300 KiB log entry persisted; the state JSON row was 307,358 bytes.
- A 1.23 MB state row persisted, read back, and survived sleep.
- Column and parameter types round-trip as the tables below show.

### Schema, upsert, load (`src/actors.rs:51-107`)

```rust
const TABLE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS table_state (
  id INTEGER PRIMARY KEY CHECK (id = 0),
  version INTEGER NOT NULL,
  json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS wakes (id INTEGER PRIMARY KEY CHECK (id = 0), n INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS log (
  seq INTEGER PRIMARY KEY, kind TEXT NOT NULL, body BLOB, score REAL, note TEXT
);";

const UPSERT_STATE: &str = "INSERT INTO table_state (id, version, json) VALUES (0, ?, ?)
  ON CONFLICT(id) DO UPDATE SET version = excluded.version, json = excluded.json";

fn state_stmt(s: &TableState) -> Result<SqliteBatchStatement> {
    Ok(SqliteBatchStatement {
        sql: UPSERT_STATE.into(),
        params: Some(vec![
            BindParam::Integer(s.version as i64),
            BindParam::Text(serde_json::to_string(s)?),
        ]),
    })
}

async fn save_state(db: &SqliteDb, s: &TableState) -> Result<()> {
    let st = state_stmt(s)?;
    db.run(st.sql, st.params).await?;
    Ok(())
}

async fn load_state(db: &SqliteDb) -> Result<Option<TableState>> {
    let res = db
        .query("SELECT version, json FROM table_state WHERE id = ?", Some(vec![BindParam::Integer(0)]))
        .await?;
    let Some(row) = res.rows.first() else { return Ok(None) };
    match row.get(1) {
        Some(ColumnValue::Text(json)) => Ok(Some(serde_json::from_str(json).context("decode table_state.json")?)),
        other => bail!("table_state.json: expected text, got {other:?}"),
    }
}
```

### `create()` reloads on every wake (`src/actors.rs:134-163`)

```rust
async fn create(ctx: &Ctx<Self>) -> Result<Self> {
    let db = ctx.sql();
    db.exec(TABLE_SCHEMA).await?;                       // idempotent DDL every wake
    let r = db.execute(
        "INSERT INTO wakes (id, n) VALUES (0, 1) ON CONFLICT(id) DO UPDATE SET n = n + 1 RETURNING n",
        None,
    ).await?;                                           // execute() supports RETURNING
    let wake_no = int_cell(r.rows.first().ok_or_else(|| anyhow!("no RETURNING row"))?, 0)?;
    let input: TableInput = ctx.input()?;
    let state = match load_state(db).await? {
        Some(s) => s,
        None => { let s = TableState { table_name: input.table_name.clone(),
                                       owner_id: input.owner_id.clone(), ..Default::default() };
                  save_state(db, &s).await?; s }
    };
    Ok(Table { state: AsyncMutex::new(state), racy: parking_lot::Mutex::new(0), wake_no, input })
}
```

### Atomic multi-statement write, then emit (`src/actors.rs:241-269`)

```rust
let mut s = self.state.lock().await;                    // turn lock held across awaits
s.version += 1;
/* mutate s */
ctx.sql().execute_batch(vec![
    state_stmt(&s)?,
    SqliteBatchStatement {
        sql: "INSERT INTO log (seq, kind, body, score, note) VALUES (?, ?, ?, ?, ?)".into(),
        params: Some(vec![
            BindParam::Integer(seq as i64), BindParam::Text("append".into()),
            BindParam::Blob(body), BindParam::Float(seq as f64 / 2.0),
            if seq % 2 == 0 { BindParam::Text("even".into()) } else { BindParam::Null },
        ]),
    },
]).await?;                                              // all-or-nothing
ctx.emit(StateChanged { version: s.version, log_len: s.log.len() })?;   // only after commit
```

### SqliteDb surface (`rk/src/sqlite.rs`)

| Call | Returns | Use |
|---|---|---|
| `exec(sql)` | `QueryResult` | DDL, multi-statement scripts, no params |
| `query(sql, Option<Vec<BindParam>>)` | `QueryResult { columns: Vec<String>, rows: Vec<Vec<ColumnValue>> }` | SELECT |
| `run(sql, params)` | `ExecResult` | write, no rows |
| `execute(sql, params)` | `ExecuteResult` (rows) | write with `RETURNING` |
| `execute_batch(Vec<SqliteBatchStatement { sql, params }>)` | | atomic batch |
| `begin_transaction()` / `begin_named_transaction(name, timeout)` | `SqliteTransaction { exec, execute, commit, rollback }` | explicit transactions **(source)** |
| `rivetkit::SqliteDbExt::transaction(callback, SqliteTransactionOptions { name, timeout })` | | callback transaction **(source)** |

### Parameter and row encoding (observed)

| Rust bind | SQLite column | Read back as |
|---|---|---|
| `BindParam::Integer(i64)` | INTEGER | `ColumnValue::Integer(i64)` |
| `BindParam::Float(f64)` | REAL | `ColumnValue::Float(f64)` (0.5, 1.0, 1.5 seen) |
| `BindParam::Text(String)` | TEXT | `ColumnValue::Text(String)` |
| `BindParam::Blob(Vec<u8>)` | BLOB | `ColumnValue::Blob(Vec<u8>)` (307,219-byte blob round-tripped) |
| `BindParam::Null` | NULL | `ColumnValue::Null` |

Observed `queryLog` columns: `["seq","kind","body","score","note"]`. Placeholders are positional `?`. Do not create tables with the `_rivet_` prefix; it is reserved.

### KV (do not use for the port)

`ctx.kv()` is `#[deprecated]` (`rk/src/context.rs:152-155`). Observed limits:

- 100 KiB put/get: ok.
- 200 KiB put: `value_too_large: KV value too large (204800 bytes). Limit is 131072 bytes.` (`core/src/error.rs:117`).
- Key limit: 2048 bytes **(source)**.

Store everything in SQLite.

---

## 3. Recipe: schedules

Verified behavior:

- `after(700 ms)` fired at about 0.8–1.0 s.
- A schedule set before the actor slept woke it and fired (`ticks=["wake-me"]`).
- Schedules are persisted by the engine.

```rust
// schedule (src/actors.rs:371-376)
impl Handler<ScheduleTick> for Table {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: ScheduleTick) -> Result<String> {
        let args = rivetkit::action::encode_positional(&Tick { label: a.label })?;   // same encoding a client uses
        ctx.schedule().after(Duration::from_millis(a.delay_ms), Tick::NAME, &args).await  // -> event id
    }
}

// the scheduled action (src/actors.rs:378-390)
impl Handler<Tick> for Table {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: Tick) -> Result<()> {
        if ctx.conn().is_some() {                       // client calls (WS and HTTP) carry a conn
            return Err(user_error("internal_only", "tick is internal"));
        }
        let mut s = self.state.lock().await;
        s.ticks.push(a.label.clone()); s.version += 1;
        save_state(ctx.sql(), &s).await?;
        ctx.emit(Ticked { label: a.label, wake_no: self.wake_no })
    }
}
```

The full API (`rk/src/context.rs:359-388`):

- `after(Duration, action, &args) -> String`
- `at(timestamp_ms, action, &args) -> String`
- `cancel(id) -> bool`
- `get(id) -> Option<ScheduledEventInfo { id, action, args, run_at }>`
- `list()`
- `cron()`: `set`, `every`, `get`, `list`, `delete`, `history` **(source)**
- `ActorConfig::max_schedules` defaults to 1000.

**How an action knows it was scheduled.** In the trait API, `ctx.conn()` is `None` only for scheduled and internal dispatch.

- Observed for a direct client call: `RPC Error(user/internal_only): tick is internal, null` over WS and `action failed (user/internal_only): tick is internal, metadata=None` over HTTP.
- `ScheduledFireInfo { kind, id, name, scheduled_at, fired_at }` exists but is only reachable from the legacy event-loop API (`rk/src/event.rs:209`). `handle_actor_event` drops it (`rk/src/start.rs:337-343`).
- This replaces the TS internal-token check on `onTempoGate`, `onAiTimeout` and similar actions.

---

## 4. Recipe: events (`conn.send` vs `ctx.emit`, iterating conns)

| API | Who receives it | Observed |
|---|---|---|
| `ctx.emit(E)` / `ctx.broadcast(name, &E)` | every conn **subscribed** to that event name (`core/src/actor/context.rs:1022-1028` filters with `is_subscribed`) | both clients got `shouted` |
| `conn.send(name, &E)` | that one conn, **whether or not it subscribed** | only the caller got `whisper` |
| `conn.send` inside a **stateless HTTP** action | nobody: logs `ERROR failed to send event to connection error=not_configured: Connection event sender is not configured.` and still returns `Ok(())` | silent drop |
| `conn.send` inside `on_connect` | the connecting client, Rust and JS, on first connect **and** on every reconnect | `["whisper:welcome","whisper:welcome"]` |
| `ctx.emit` inside `on_connect` | **not** the connecting client (it has not subscribed yet); other subscribed conns only | never received by the joiner |

```rust
// private reply (src/actors.rs:331-341)
let Some(conn) = ctx.conn() else { return Ok(WhoamiOut { conn_id: None, player_id: None }) };
let p = conn.params()?;
conn.send(Whisper::NAME, &Whisper { text: format!("only for {:?}", p.player_id) })?;

// iterate conns with params (src/actors.rs:349-369)
ctx.conns()                                  // read lock: collect, never hold across .await
    .map(|c| Ok(ConnInfo { id: c.id().to_owned(), player_id: c.params()?.player_id }))
    .collect()
for c in ctx.conns_vec() {                   // owned Vec: safe to use around awaits
    if c.params().ok().and_then(|p| p.player_id).as_deref() == Some(a.player_id.as_str()) {
        c.send(Whisper::NAME, &Whisper { text: a.text.clone() })?;
    }
}

// TS `conn.send('sync', payload)` on connect -> Rust (src/actors.rs:189-199)
async fn on_connect(self: Arc<Self>, ctx: Ctx<Self>, conn: ConnCtx<Self>) -> Result<()> {
    let p = conn.params()?;
    conn.send("sync", &self.view_for(&p).await)?;       // reaches the joiner
    ctx.emit(Presence { seat, online: true })?;          // reaches everyone else
    Ok(())
}
```

**Event payload encoding (G10).** Events go through `encode_varargs` (`rk/src/action.rs:29-33`): JSON array → positional args unchanged, `null` → zero args, anything else → one arg.

- A struct payload `{..}` arrives as one listener arg. This matches TS `c.broadcast('x', obj)`.
- `Hand(Vec<String>)` arrived as **3 JS listener args** (`["hand",["9H","TD","JS"]]`). It decoded fine on the Rust typed `on::<Hand>`.
- To match TS `c.broadcast('hand', cards)`, wrap the payload in a 1-tuple: `ctx.broadcast("hand", &(cards,))`.

Size limits are covered in section 9 ("Limits").

---

## 5. Recipe: actor-to-actor calls

Verified behavior:

- `ctx.client()` works in envoy mode.
- `get_or_create_typed` with `create_with_input` creates the seat with its input. The second spawn reaches the same actor (`create_count 1 -> 1`).
- A re-entrant call back into the caller works.
- An external `get_typed(["t1","2"])` finds the seat.

```rust
// src/actors.rs:474-496 (table spawns a seat)
let client = ctx.client()?;
let seat = client.get_or_create_typed::<AiSeat>(
    "aiSeat",
    [table_key.clone(), a.seat.to_string()],             // key = array of strings (critic: the euchre key is 4 segments ["table", gameId, "seat", "1"|"2"|"3"], ai-seat/index.ts:112)
    GetOrCreateOptions {
        create_with_input: Some(serde_json::to_value(SeatInput { table_key, seat: a.seat })?),
        ..Default::default()                              // params, create_in_region, pool_name
    },
)?;
let info = seat.call(Describe).await?;                    // stateless HTTP action
let n = seat.call(PokeTable).await?;                      // seat calls back into this table

// src/actors.rs:580-593 (seat calls the table; on_before_connect still runs)
let table = ctx.client()?.get_typed::<Table>(
    "table",
    [self.input.table_key.clone()],
    rivetkit::client::GetOptions {
        params: Some(serde_json::json!({ "token": "secret", "playerId": format!("seat-{}", self.input.seat) })),
    },
)?;
table.call(SeatReady { seat: self.input.seat }).await
```

Rules:

- **Never hold the turn lock while awaiting another actor.** If that actor calls back (`seatReady` takes the same lock), you deadlock until `action_timeout` (60 s). `SpawnSeat` deliberately takes no lock.
- The internal client is Bare encoding, WebSocket transport, `disable_metadata_lookup(true)`, and the envoy's endpoint, namespace, pool and token (`rk/src/context.rs:306-347`).
- Use stateless `call`s between actors. Do not keep long-lived `connect()`s; see G27.

---

## 6. Recipe: background work and locking rules

### Fire-and-forget work that mutates state and emits (`src/actors.rs:392-423`)

The action returned in 0.7 ms. It produced 5 `bgProgress` events and `bg_steps=5`. Holding `keep_awake_region` kept the actor awake past a 2 s `sleep_timeout` with no conns (`wake_no 1 -> 1`).

```rust
impl Handler<StartBackground> for Table {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, a: StartBackground) -> Result<()> {
        let me = self.clone();
        let c = ctx.clone();
        ctx.register_task(async move {
            let _awake = c.keep_awake_region();        // register_task alone does NOT block idle sleep
            let abort = c.abort_signal();
            for step in 1..=a.steps {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(a.interval_ms)) => {}
                    _ = abort.cancelled() => return,     // sleep/destroy/shutdown
                }
                let res: Result<()> = async {
                    let mut s = me.state.lock().await;
                    s.bg_steps += 1; s.version += 1;
                    save_state(c.sql(), &s).await?;
                    let version = s.version;
                    drop(s);                              // release before emit/next await
                    c.emit(BgProgress { step, version })
                }.await;
                if res.is_err() { return; }
            }
        });
        Ok(())
    }
}
```

### Work kinds (`core/src/actor/work_registry.rs:47-95`)

| Kind | Blocks idle sleep | Drained during shutdown grace | Aborted at shutdown deadline |
|---|---|---|---|
| Action in flight | yes | yes | yes |
| `keep_awake(fut)` / `keep_awake_region()` | yes | yes | yes |
| `wait_until(fut)` | no | yes | yes |
| `register_task(fut)` | no | yes | yes |

Idle sleep is also blocked by any of the following (`core/src/actor/sleep.rs:218-251`):

- not yet ready
- `no_sleep`
- active HTTP requests
- keep-awake count
- an active `run` handler that is **not** waiting on a queue receive
- pending disconnect callbacks
- **any open connection**
- websocket callbacks

### Locking rules (verified)

1. **Actions run concurrently.** Ten concurrent `bumpRacy` calls (read, `sleep(20ms)`, write) gave a max of **1**: lost updates. TS actors gave you a single-threaded event loop; Rust does not.
2. **Serialize with a `tokio::sync::Mutex` turn lock held across `.await`.** Ten concurrent `bumpLocked` calls went 0 → 10 in about 330 ms. Pattern: lock, mutate, persist (`execute_batch`), emit, unlock.
3. Use `parking_lot::Mutex` only for sections with no `.await`. Holding a parking_lot guard across `.await` does not compile, because the `Handler` future must be `Send`.
4. **G4: parking_lot is not re-entrant.** `f(x.lock().a, format!("{:?}", x.lock()))` deadlocks, because the first guard lives until the end of the statement. Bind the lock result to a local first. Operands of `&&` / `||` are their own temporary scope, so those are fine.
5. Do not hold the turn lock across actor-to-actor calls (section 5).
6. **Do not write a `run` loop.**
   - An active `run` blocks idle sleep (`CanSleep::ActiveRunHandler`).
   - The queue it would read from is **deprecated in 3.0** (`cl/src/handle.rs:129`: "Use actions and store pending work in SQLite").
   - The TS pipeline (schedule → action → `c.queue.send('tick')` → `run` loop) becomes schedule → action → handler under the turn lock.

---

## 7. Recipe: actor input

```rust
// client (src/main.rs:88-94)
GetOrCreateOptions {
    params: Some(json!({ "token": "secret", "playerId": player })),
    create_with_input: Some(serde_json::to_value(TableInput { table_name: "Main".into(), owner_id: "u1".into() })?),
    ..Default::default()
}
// server
let input: TableInput = ctx.input()?;   // in create(); keep a copy in Self if needed later
```

| Case | Observed |
|---|---|
| `get_or_create` + `create_with_input` | decoded; `key == ["t1"]` |
| Second `get_or_create` with a different input | **ignored**; the original input persists |
| No input | `TableInput::default()` (`Input: Default` is required) |
| `client.create(..., CreateOptions { input, .. })` | works |
| Duplicate `create` | `failed to create actor (409 Conflict): {"code":"duplicate_key","group":"actor","message":"Actor key already in use."}` |
| `get` of a missing key | `action failed (actor/not_found): The actor does not exist or was destroyed., metadata=None` |
| JS `getOrCreate([k], { createWithInput: {tableName, ownerId} })` | Rust saw `input: {"ownerId":"js-owner","tableName":"From JS"}` |

Input is available on every wake, not only the first.

---

## 8. Recipe: errors that reach the client

```rust
// src/actors.rs:22-47
pub fn user_error(code: &str, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(rivet_error::RivetError {
        kind: rivet_error::RivetErrorKind::Dynamic {
            group: "user".into(), code: code.into(), default_message: message.into(),
        },
        meta: None, message: None, actor: None,
    })
}
pub fn forbidden(message: impl Into<String>) -> anyhow::Error {   // group "auth", code "forbidden" (403)
    anyhow::Error::new(rivet_error::RivetError {
        kind: rivet_error::RivetErrorKind::Dynamic {
            group: "auth".into(), code: "forbidden".into(), default_message: message.into(),
        },
        meta: None, message: None, actor: None,
    })
}
```

### Observed client error strings

| Server side | Rust connection (`cl/src/connection.rs:455`) | Rust stateless (`cl/src/handle.rs:117`) | JS 3.1.0 (`RivetError`) |
|---|---|---|---|
| `Err(user_error("not_your_turn","It is not your turn."))` | `RPC Error(user/not_your_turn): It is not your turn., null` | `action failed (user/not_your_turn): It is not your turn., metadata=None` | `group=user code=not_your_turn message="It is not your turn."` |
| `bail!("db password is hunter2")` | `RPC Error(core/internal_error): An internal error occurred, null` | `action failed (core/internal_error): An internal error occurred, metadata=None` | `group=core code=internal_error` |
| unknown action `nope` | `RPC Error(actor/action_not_found): Action \`nope\` was not found., null` | `action failed (actor/action_not_found): ...` | same group/code |
| args that fail to decode | `core/internal_error` (masked) | `core/internal_error` | `core/internal_error` |
| `on_before_connect` → `forbidden("bad table token")` | WS retries forever (section 9) | `action failed (auth/forbidden): bad table token, metadata=None` | `group=auth code=forbidden message="bad table token"` |
| `on_before_connect` → `bail!` | | `core/internal_error` | |
| actor missing on `get` | | `actor/not_found` | `actor/not_found` (see G21) |

What these results mean:

- The connection survives action errors; status stays `Connected`.
- **Public groups** pass through unmasked (`core/src/error.rs:23-49`):
  - `auth/forbidden` (403)
  - `actor/not_found`, `actor/action_not_found` (404)
  - `method_not_allowed` (405)
  - `action_timed_out` (408)
  - `aborted`, `message/incoming_too_long`, `message/outgoing_too_long` (400)
  - `schedule/*`, some `queue/*`, `kv/*`
  - **`user/*` (400)**
- Every other error becomes `core/internal_error` "An internal error occurred" and its metadata is dropped (`client_error_message`, `core/src/error.rs:138`). `RIVET_EXPOSE_ERRORS=1` exposes `Internal error: ...` for debugging.
- **G15:** `#[derive(RivetError)]` writes `<workspace root>/engine/artifacts/errors/<group>.<code>.json` at compile time (`rivet-error-macros-3.1.0/src/lib.rs:581-660`; redirect with `RIVET_ERROR_OUTPUT_DIR`). Use the `Dynamic` constructor above instead.
- The Rust client has **no structured error type**, only these strings. Parse them (`src/main.rs:40-51`):

```rust
pub fn parse_rivet_error(e: &anyhow::Error) -> Option<(String, String, String)> {
    let s = format!("{e:#}");
    let start = s.find('(')? + 1;
    let end = start + s[start..].find(')')?;
    let (group, code) = s[start..end].split_once('/')?;
    let rest = s[end + 1..].trim_start_matches(':').trim();
    let msg = match rest.rfind(", metadata=") {
        Some(i) => &rest[..i],
        None => rest.rsplit_once(", ").map(|(m, _)| m).unwrap_or(rest),
    };
    Some((group.to_owned(), code.to_owned(), msg.to_owned()))
}
```

JS quirks:

- `e.public` was `false` even for `user/*`. Branch on `group` and `code` instead.
- `metadata` is `null` over a bare-encoded connection and `undefined` otherwise.
- The JS client logs `unhandled action promise rejection` (warn) for conn action errors even when the caller catches them.

---

## 9. Recipe: the Rust client

```rust
let client = Client::new(
    ClientConfig::new(&endpoint)        // e.g. http://127.0.0.1:6420
        .token("default")               // local engine token
        .namespace("default")
        .pool_name(&pool)               // must equal the server's RIVET_POOL
        // .encoding(EncodingKind::Json | Cbor | Bare)  default Bare; all three verified
        // .header(k, v) .max_input_size(n) .disable_metadata_lookup(true) .transport(..)
);
let h = client.get_or_create_typed::<Table>("table", ["t1"], table_opts("alice", None))?;
let v = h.call(Ping).await?;                         // stateless HTTP action (temp conn)
let conn = h.connect();                              // TypedActorConnection<Table>; returns immediately
let n = conn.call(AppendLog { text: "x".into(), repeat: 1 }).await?;
let _sub = conn.on::<StateChanged>(|e| { /* typed */ }).await;   // returns a subscription
_sub.unsubscribe().await;                            // verified: delivery stops
let raw = conn.inner();                              // ActorConnection = Arc<ActorConnectionInner>
raw.action("add", vec![json!(2), json!(3)]).await?;  // untyped positional
raw.on_event("hand", |args: Vec<serde_json::Value>| { /* raw positional args */ }).await;
conn.disconnect().await;
```

The handle and connection APIs:

- Typed handles: `get_typed`, `get_or_create_typed`, `get_or_create_typed_default` (`TypedClientExt`).
- `TypedActorHandle`: `call(A)`, `send(A)` (fire-and-forget), `connect()`.
- `TypedActorConnection`: `on::<E>`, `call`, `send`, `disconnect`, `inner()`.
- `ActorConnectionInner`:
  - events: `on_event`, `on_event_raw`, `once_event`
  - lifecycle callbacks: `on_open`, `on_close`, `on_error(&str)`, `on_status_change(ConnectionStatus)`
  - status: `conn_status()`, `status_receiver()` (a `tokio::sync::watch`)
  - `ConnectionStatus` is `{ Idle, Connecting, Connected, Disconnected }` and derives `PartialEq`.

### Connect with a deadline (a rejected connection retries forever)

```rust
// src/main.rs:66-82
async fn connect_bounded<A: Actor>(h: &TypedActorHandle<A>, timeout: Duration) -> Result<TypedActorConnection<A>> {
    let conn = h.connect();
    let mut rx = conn.inner().status_receiver();
    let last_error = Arc::new(Mutex::new(None::<String>));
    let le = last_error.clone();
    conn.inner().on_error(move |m| *le.lock() = Some(m.to_owned())).await;
    match tokio::time::timeout(timeout, rx.wait_for(|s| *s == ConnectionStatus::Connected)).await {
        Ok(Ok(_)) => Ok(conn),
        _ => { conn.disconnect().await;   // stops the retry loop
               bail!("connect timed out; last error: {:?}", last_error.lock().clone()) }
    }
}
```

A bad token gave `connect timed out; last error: Some("bad table token") after 4.0s`.

### Reconnect and retry behavior (verified)

| Scenario | Observed |
|---|---|
| Actor sleeps while connected | statuses `Disconnected → Connecting → Connected` within 0.07–2.1 s; new server conn id; `on_connect` reruns; **subscriptions re-sent automatically**; `wake_no+1` |
| Bad token | WS attempts at t = 0, 1.1, 3.1, 7.1 s (backoff 1 s doubling to 30 s, `cl/src/backoff.rs`, `cl/src/connection.rs:684-738`). Server logs `failed to establish actor websocket connection ... group="auth" code="forbidden"` each time. **Each attempt wakes the sleeping actor.** |
| Action on a never-connected conn | **hangs forever**; only `tokio::time::timeout` gets it back |
| Action in flight when the actor sleeps (`slow 5000ms` during `ctx.sleep()`) | server logs `actor/stopping`; **client RPC still pending after 8 s and after reconnect**; the handler future was dropped (its abort branch never ran) |
| `conn.disconnect()` with a hung RPC | the RPC resolves `Err("Socket closed during rpc")` (`in_flight_rpcs.clear_async()`, `cl/src/connection.rs:656-677`); status `Disconnected`; retries stop |
| 70 KiB action args over WS | **no reply**; socket dropped and reconnected (`Disconnected, Connecting, Connected`); RPC hangs |

### G26: dropping any clone of a `Client` kills reconnection

`impl Drop for Client` sends on a broadcast channel that **all clones share** (`cl/src/client.rs:358-362`; `shutdown_tx: Arc<broadcast::Sender<()>>` at :137, capacity 1 at :252). The retry loop only checks it in the backoff `select!` after a **failed** attempt (`cl/src/connection.rs:722-736`).

- **Verified, failing loop.** Attempts ran at 0, 1.1, 3.1, 7.1 s. An unrelated `client.clone()` was dropped at 9.0 s. No attempt followed in the next 20 s; status `Disconnected`.
- **Verified, live connection.** After an unrelated clone drop, a `Connected` conn still works (ping ok). But the drop is buffered in its receiver: when the actor slept and the first reconnect attempt was rejected, the connection **never came back** (`reconnected=false status=Disconnected`). The same test without the clone drop reconnected.
- **Fix:** share `Arc<Client>`. Never call `Client::clone()` in axum state, per request, per task, or in the Rune GUI.
- **G27:** `ctx.client()` returns a `Client` clone of a per-actor cached client (`Arc<OnceLock<Client>>`, `rk/src/context.rs:80-101,306-347`). Dropping it at the end of a handler broadcasts shutdown, so a long-lived `connect()` opened through `ctx.client()` will not survive a failed reconnect **(source, by the same mechanism)**. Use stateless calls between actors.

### Other client gotchas

- **G28:** event callbacks run inline on the socket receive loop **(source)**. A blocking callback stalls every RPC reply. Forward to a channel (`tokio::sync::mpsc`) and process elsewhere.
- **G29:** there is no per-call timeout. Wrap every `conn.call` in `tokio::time::timeout`. On timeout, `disconnect()` and reconnect (a new `connect()`), then reload with a `snapshot`-style action.
- Startup: the first calls can fail with `Connection refused` and then `503` metadata errors while the engine and envoy register. Retry `ping` until it succeeds (`src/main.rs:135-150`).

### Limits (observed with `ActorConfig` defaults)

| Limit | Default (`core/src/actor/config.rs:13-25`) | Stateless overflow | Connection overflow |
|---|---|---|---|
| Incoming message (args) | 65,536 B | `action failed (message/incoming_too_long): Incoming message too long` | **socket drop, RPC hangs** |
| Outgoing message (output) | 1,048,576 B | `message/outgoing_too_long: Outgoing message too long` | `RPC Error(message/outgoing_too_long): Outgoing message too long, null` |
| Event size | same 1 MiB | | 60, 200, 500 and 900 KiB delivered; **1100 KiB: socket dropped, event lost, the emitting action's reply lost** |
| `action_timeout` | 60 s | | |
| `on_before_connect` / `on_connect` timeout | 5 s each | | |
| `sleep_timeout` / `sleep_grace_period` | 30 s / 15 s | | |
| `max_schedules` | 1000 | | |

Override per actor: `registry.register_actor_with::<Table>("table", ActorConfig { sleep_timeout: Duration::from_secs(2), ..ActorConfig::default() })`.

---

## 10. Recipe: `ServeConfig` for local dev, an external engine, and Railway

### Server entry points (`rk/src/registry.rs:44-188`)

```rust
let mut registry = Registry::new();
registry.register_actor::<Table>("euchreTable");   // the registered name is the actor name clients use
registry.register_actor_with::<AiSeat>("aiSeat", ActorConfig { ..ActorConfig::default() });

// A) standalone binary: blocks until SIGINT/SIGTERM, envoy or serverless per RIVETKIT_RUNTIME_MODE
registry.start().await?;                            // = start_with_config(ServeConfig::from_env()?)

// B) embedded next to axum (what apps/server wants): own the lifecycle
let config = ServeConfig::from_env()?;              // ALWAYS start from from_env (see G31)
let shutdown = CancellationToken::new();
let serve = tokio::spawn(registry.serve_with_config(config, shutdown.clone()));
// ... run axum; on exit: shutdown.cancel(); serve.await??;   (graceful stop also stops a spawned engine)

// C) serverless handler mounted in your own HTTP server
let rt = registry.into_serverless_runtime(ServeConfig::from_env()?).await?;   // handle_request(ServerlessRequest)
```

### Environment read by `ServeConfig::from_env` (`core/src/registry/envoy_callbacks.rs:165-241`, `mod.rs:255-289,391-405`)

| Var | Default | Effect |
|---|---|---|
| `RIVET_ENDPOINT` | unset → `http://{RIVET_RUN_ENGINE_HOST:-127.0.0.1}:{RIVET_RUN_ENGINE_PORT:-6420}` | engine URL; may carry `https://namespace:token@host` (no query/fragment; a token needs a namespace) |
| `RIVET_NAMESPACE` | `default` | error if also set in the URL |
| `RIVET_TOKEN` | `"default"` **only** when the endpoint is localhost/loopback; otherwise none (unauthenticated) | error if also set in the URL |
| `RIVET_POOL` | `default` | runner pool name; clients must use the same value (`poolName`) |
| `RIVET_ENVOY_VERSION` (legacy `RIVET_NODE_VERSION`) | `1` | runner version; bump per deploy for drain/rollover **(source)** |
| `RIVETKIT_ENGINE_SPAWN` | `auto` | `auto`: spawn only when `RIVET_ENDPOINT` is unset. `always` + an endpoint → error `engine/spawn_endpoint_conflict`. `never`. |
| `RIVET_ENGINE_BINARY_PATH` | | engine binary to spawn |
| `RIVETKIT_ENGINE_AUTO_DOWNLOAD` | off | `1/true/yes`: download the engine |
| `RIVETKIT_STORAGE_PATH` | `~` | engine data root (`$path/.rivetkit`) |
| `RIVET_RUN_SERVICES=1`, `RIVET_SERVICES_BINARY` | off | also run the services binary |
| `RIVETKIT_RUNTIME_MODE` | envoy | `serverless` → HTTP listener on `0.0.0.0:$RIVET_PORT` (default **3000**), base path `/api/rivet` (`rk/src/serverless_listener.rs:25-58`, `core/src/serverless.rs:31`) |

from_env also sets `serverless_max_start_payload_bytes = 1_048_576` and `serverless_cache_envoy = true`.

**G31:** `ServeConfig::default()` gives `version: 0` and `serverless_max_start_payload_bytes: 0`. Build from `from_env()` and override fields.

### Profiles

| Profile | Env |
|---|---|
| Local dev (spawn an engine) | `RIVET_ENGINE_BINARY_PATH=...` or `RIVETKIT_ENGINE_AUTO_DOWNLOAD=1`; optional `RIVET_RUN_ENGINE_PORT`, `RIVETKIT_STORAGE_PATH`, `RIVET_POOL` |
| External engine, envoy (recommended for Railway: engine as its own service) | `RIVET_ENDPOINT=http://<engine>.railway.internal:6420`, `RIVET_TOKEN=<admin/runner token>`, `RIVET_NAMESPACE=<ns>`, `RIVET_POOL=<pool>`, `RIVET_ENVOY_VERSION=<deploy id int>`. With an endpoint set, auto mode never spawns. |
| Rivet Cloud / serverless runner | `RIVETKIT_RUNTIME_MODE=serverless`, `RIVET_PORT=$PORT` (Railway sets `PORT`, not `RIVET_PORT`), `RIVET_ENDPOINT=https://ns:token@api...`; register the runner URL `https://<service>/api/rivet` with the engine |

Engine-process facts:

- A healthy engine already on the port is reused.
- The engine runs in its own process group.
- `PDEATHSIG` is Linux-only, so **on macOS a killed host orphans the engine**. A graceful shutdown (`shutdown.cancel()` then awaiting serve) stops it (`ep/src/lib.rs:972-1010`). Two runs here that hit the alarm left orphan engines that had to be killed by pid.

---

## 11. Recipe: JS client interop (the wire)

### How args are carried

- **Client → actor.** The JS client sends `conn.method(...args)` as a positional args array (CBOR, or JSON in json mode). The Rust typed client serializes the action struct to JSON and normalizes it with `positional_value` (`rk/src/action.rs:436-444`): an array stays as-is, `null` becomes `[]`, anything else becomes `[value]`.
- **Actor decode.** `decode_positional` (`rk/src/action.rs:411-433`):
  1. Try to decode `T` from the whole args array.
  2. If that fails and the array is `[]`, decode from `null`.
  3. If that fails and the array is `[x]`, decode from `x`.

  Serde structs accept the seq form, so **struct fields bind positionally**.
- **Outputs** are plain JSON (`encode_cbor`, not positional). Field names follow serde: use `#[serde(rename_all = "camelCase")]`. Observed: `{"bgSteps":0,"counter":0,"log":[...],"ownerId":"js-owner","seatsReady":[],"tableName":"From JS","ticks":[],"version":1}`.
- `Option::None` serializes to `null`, never to absent. Add `#[serde(skip_serializing_if = "Option::is_none")]` where TS code checks `=== undefined` or `'k' in obj`.
- **Events:** each positional element becomes one JS listener arg (section 4).

### Argument-shape matrix (all verified; JS = rivetkit 3.1.0, bare encoding unless noted)

| Rust action type | JS call | Rust typed client | Result |
|---|---|---|---|
| `struct Add(i64, i64)` | `add(2,3)` | `Add(2,3)` | `5` both |
| `struct Double(i64)` | `double(21)` | `Double(21)` | `42` both |
| unit `struct Ping;` | `ping()` / raw `[null]` | `Ping` | `"pong"` |
| `struct AppendLog { text, repeat }` | `appendLog({text, repeat})` | `AppendLog{..}` | ok both (seq path fails, falls back to `[x]`) |
| `struct Shout { text: String }` | `shout("bare")` **and** `shout({text})` | | both ok (G33: a struct also binds from positional scalars) |
| `struct TwoArgs(String, u8)` | `twoArgs("t9", 2)` | `TwoArgs(..)` | `"t9\|2"` both |
| `struct NamedTwo { turn_id, seat }` | `namedTwo("t9", 2)` and `namedTwo({turnId, seat})` | `NamedTwo{..}` | `"t9\|2"` all |
| `struct OptArg(Option<u32>)` | `optArg()` → None, `optArg(null)` → None, `optArg(5)` → Some(5) | `OptArg(None/Some)` | ok |
| `struct SubmitMove(MoveReq)` (newtype over a struct) | `submitMove({turnId, card})`; `submitMove({turnId})` → `card: null` | `SubmitMove(..)` | ok both |
| **`struct Echo { value: Value }`** | `echo({value:{a:1}})` | `Echo{value}` | **wrong both ways**: returns `{"value":{"a":1}}` (the whole arg bound to `value`) |
| **`struct PatchNamed { patch: Value }`** | `patchNamed({theme})` → `{theme}` (correct) | `PatchNamed{patch: obj}` → **`{"patch":{theme}}`** | JS ok, **Rust client wrong** |
| **`struct PatchNewtype(Value)`** | `patchNewtype({theme})` → **`[{theme}]`** | → **`[{theme}]`** | **wrong both ways** (the newtype gets the whole args array) |
| `struct PatchOne((Value,))` (newtype over a 1-tuple) | `patchOne({theme})` → `{theme}` | `PatchOne((obj,))` → `{theme}`; `PatchOne(([1,2],))` → `[1,2]` | **correct both ways** |

**G32 rule.** If the first field of an action type can deserialize from a JSON array or object, the "whole array" path wins and binds wrongly. This covers `serde_json::Value`, `Vec<Value>`, maps, structs whose fields are all `Option` or default, and newtypes over these.

- For TS `unknown` / `any` args use the `PatchOne((Value,))` shape, or a tuple struct with 2 or more fields.
- Prefer tuple structs that mirror the TS positional signature.
- For a single object arg, use a newtype over a struct that has at least one required non-`Value` field.

### Encoding differences (JS client → Rust actor, raw args seen by Rust)

| JS value | bare (JS default) | json | cbor |
|---|---|---|---|
| `undefined` arg | `[["$Undefined",0]]` | `[null]` | `[null]` |
| `{x: undefined, n: 1}` | `[{"n":1,"x":["$Undefined",0]}]` | `[{"n":1}]` | `[{"n":1,"x":null}]` |
| `10n` | `[["$BigInt","10"]]` | `[["$BigInt","10"]]` | `[10]` |
| `Uint8Array[1,2,3]` | `[["$Uint8Array","AQID"]]` | same | **internal_error** |
| `["$x",1]` | `[["$$x",1]]` (escaped) | same | `[["$x",1]]` |
| `new Date(0)` | `[0]` | `["1970-01-01T00:00:00.000Z"]` | **internal_error** |
| `new Map([[1,2]])` | **internal_error** | `[{}]` | `[{}]` |
| `optArg(undefined)` → `Option<u32>` | **internal_error** | `None` | `None` |
| `submitMove({turnId, card: undefined})` | **internal_error** | `card: null` | `card: null` |

**G34 (high impact).** With the JS default `encoding: "bare"`, any `undefined`, including an object property set to `undefined`, reaches Rust as the tag `["$Undefined",0]`. An `Option<T>` field then fails to decode with a masked `core/internal_error`. Rust core passes these tags through raw (`core/src/registry/actor_connect.rs:200-300` only parses `$BigInt` ids). The fix is any one of:

- a JS client with `encoding: "json"`, or
- stripping `undefined` before calling, or
- a Rust deserializer that maps the tag to `None`.

Everything else in the interop run was identical across bare, json and cbor: ping/add/double, `createWithInput`, camelCase outputs, the full error matrix, events, and `whoami` params.

### JS client config that talked to the Rust server

```js
import { createClient } from "rivetkit/client";        // rivetkit@3.1.0
const client = createClient({ endpoint: "http://127.0.0.1:6620", namespace: "default",
                              token: "default", poolName: "cookbook", encoding: "json" });
const h = client.table.getOrCreate(["t1"], { params: { token: "secret", playerId: "js" },
                                             createWithInput: { tableName: "From JS", ownerId: "js-owner" } });
await h.add(2, 3);                                     // stateless
const conn = h.connect();
conn.on("hand", (...args) => {});                      // args = positional elements
await conn.whoami();                                   // {"connId":"...","playerId":"js"}
```

- The JS `poolName` defaults to `RIVET_POOL` / `"default"`, and `encoding` defaults to `"bare"`.
- **G21:** JS `client.table.get([missing]).ping()` retried internally (25 `actor/not_found` responses logged) before throwing `actor/not_found`.
- Connection params reach Rust `ConnParams` through serde: `{token, playerId}` → `TableParams { token, player_id }` with `rename_all = "camelCase", default`.

---

## 12. Sleep, wake, and durability

| Question | Answer (verified) |
|---|---|
| Does `create()` rerun on wake? | **Yes, every wake** (`wake_no 1 → 2` after `ctx.sleep()`; idle sleep 2 → 3). `on_create` runs only on first boot. |
| In-memory state after sleep? | Gone. Rebuild in `create()` from SQL (state reload verified, `log_len=1`). |
| Same actor id after wake? | Yes. |
| Conn ids after wake? | **New ids**; clients reconnect automatically and `on_connect` reruns. Key seats by `params.playerId`, not conn id. `can_hibernate_websocket` defaults to false. |
| Schedules across sleep? | Persisted; a due schedule **wakes** the actor and fires (`tick fired ... wake_no=4`). |
| `on_sleep`? | Runs on idle sleep, `ctx.sleep()`, and registry shutdown. Best-effort only; persist on every mutation. |
| In-flight action during sleep? | The future is **dropped** (no abort branch ran); the client RPC hangs (section 9). Bound every call. |
| What keeps it awake? | Open conns, in-flight actions, keep-awake regions (section 6). A rejected WS retry loop repeatedly wakes it. |
| Idle timeout | `sleep_timeout` (30 s default; 2 s in tests) after the last blocker clears. |

Durability rule: **persist in the same handler, before you emit or return** (`execute_batch` for multi-row). Treat `on_sleep` as an optimization.

---

## 13. Gotcha index

| # | Gotcha | Fix |
|---|---|---|
| G1 | Nested `Actions` tuples do not implement `ActionSet` | one flat tuple, ≤128 |
| G2 | Actions run concurrently (lost updates observed) | tokio `Mutex` turn lock held across awaits |
| G3 | parking_lot guard across `.await` does not compile (Send) | use only for non-await sections |
| G4 | parking_lot re-lock inside the same statement deadlocks | bind lock results to locals |
| G5 | Stateless HTTP calls run `on_before_connect` + `on_connect` + `on_disconnect` per call | keep hooks cheap; do not treat `on_connect` as "player joined" without a WS check |
| G6 | `conn.send` in a stateless call is silently dropped (logs `not_configured`) | send only to WS conns, or return the data |
| G7 | `ctx.emit` from `on_connect` does not reach the joiner | `conn.send` for the joiner's initial sync |
| G8 | `conns()` holds a read lock | collect or `conns_vec()` before awaiting |
| G9 | Actor sleep drops conns; no `on_disconnect` for old ids | key by `playerId`; rebuild presence on `on_connect` |
| G10 | Array-shaped event payload spreads into N JS listener args | wrap in a 1-tuple: `&(vec,)` |
| G11 | `ctx.kv()` deprecated, 128 KiB value limit | SQLite |
| G12 | Second `get_or_create` input ignored | input only for creation; mutate via actions |
| G13 | `bail!` / anyhow errors masked to `core/internal_error` | `user_error(code, msg)` (group `user`) |
| G14 | Bad action args are masked too (no field info reaches the client) | validate inside the handler and return `user/*` |
| G15 | `#[derive(RivetError)]` writes JSON artifacts into the workspace | `RivetErrorKind::Dynamic` |
| G16 | The Rust client has only string errors, in two formats | `parse_rivet_error` |
| G17 | A rejected WS connect retries forever with backoff and wakes the actor each time | `connect_bounded` + `disconnect()` |
| G18 | An action on a never-connected conn hangs | `tokio::time::timeout` |
| G19 | An in-flight RPC during actor sleep never resolves | timeout, then `disconnect()` (gives "Socket closed during rpc"), then reconnect |
| G20 | >64 KiB args over WS: no error, socket drop, hang | keep args small; chunk uploads via HTTP |
| G21 | JS `get` on a missing actor retries many times before `not_found` | use `getOrCreate`, or a server lookup |
| G22 | >1 MiB event: socket drop, event lost, emitting action's reply lost | keep events small; send deltas |
| G23 | >1 MiB output: `message/outgoing_too_long` | paginate (`getHandReview(n)` style) |
| G24 | Scheduled actions can only be told apart by `ctx.conn().is_none()` | guard internal actions with it |
| G25 | `register_task` / `wait_until` do not block idle sleep | hold `keep_awake_region()` inside the task |
| G26 | Dropping **any** `Client` clone kills every reconnect loop on its first failed attempt | share `Arc<Client>`; never `Client::clone()` |
| G27 | `ctx.client()` returns a clone that drops at handler end | stateless calls between actors only |
| G28 | Client event callbacks run inline on the receive loop | forward to a channel |
| G29 | No per-call timeout on the client | wrap every call |
| G30 | macOS: a killed host orphans the spawned engine | graceful shutdown; kill by pid if needed |
| G31 | `ServeConfig::default()` has `version 0`, max start payload 0 | start from `ServeConfig::from_env()` |
| G32 | Action types whose first field accepts an array/object mis-bind | tuple structs; `(Value,)` for `unknown` |
| G33 | Named structs also accept positional args (`shout("x")` works) | fine, but order fields like the TS signature |
| G34 | JS bare encoding sends `undefined` as `["$Undefined",0]` and breaks `Option` | JS `encoding: "json"`, or strip undefined |
| G35 | An active `run` handler blocks sleep; queues are deprecated | schedules + actions, no `run` loop |
| G36 | The first requests after boot fail (`Connection refused`, then 503) | readiness retry loop |
| G37 | `RIVET_TOKEN` defaults to `"default"` only for loopback endpoints | always set the token for remote engines |
| G38 | Serverless listens on `RIVET_PORT` (default 3000), not `PORT` | `RIVET_PORT=$PORT` on Railway |

---

## 14. Porting map for the euchre actors

TS sources: `/Users/brittianwarner/goods/euchre/src/lib/actors/{euchre-table,ai-seat,player-profile}/index.ts` (read-only).

| TS (file:line) | Rust shape |
|---|---|
| `snapshot: (c)` (euchre-table:1829) | `struct Snapshot;` unit; uses `ctx.conn()` params to pick the seat view |
| `getHandReview(c, handNo: number)` (:1837) | `struct GetHandReview(pub u32);` |
| `lastTrick(c)` (:1844) | unit struct |
| `submitMove(c, request: SubmitMoveRequest)` (:1861) | `struct SubmitMove(pub SubmitMoveRequest);` (G32: first field required, non-Value) |
| `reviewTrick(c, request: {action:'set',enabled} \| {action:'continue',turnId})` (:1798) | `struct ReviewTrick(pub ReviewTrickRequest);` with `#[serde(tag = "action")]` enum; returns `{queued:true}`. Not covered by the shape matrix: add a JS-interop test for both variants (internally tagged enums also accept a seq form). <!-- critic: row added; reviewTrick was missing from this map --> |
| `onTempoGate/releaseAiMove/onNudge/onAbandon(c, turnId: string)` (:1916-1940) | `struct OnTempoGate(pub String);` plus a `ctx.conn().is_none()` guard; the body runs inline under the turn lock, without `c.queue.send('tick')` |
| `onAiTimeout(c, turnId: string, seat: Seat)` (:1926) | `struct OnAiTimeout(pub String, pub Seat);` |
| `flushProfile(c)`, `onReap(c)` (:1941-1946) | unit structs + internal guard |
| `onBeforeConnect(c, params)` (:1628) | `on_before_connect(&TableParams)` → `forbidden(..)` |
| `createConnState(c, params)` (:1659) | no conn-state API in the typed layer: `HashMap<conn_id, ConnState>` filled in `on_connect`, removed in `on_disconnect`, or `conn.inner().set_state(bytes)` **(source)** |
| `onConnect: conn.send('sync', payload); c.broadcast('presence', ...)` (:1709-1714) | `conn.send("sync", &view)?; ctx.emit(Presence{..})?` (verified pattern, section 4) |
| `c.broadcast('chat' / 'thinking' / 'chatDelta', obj)` (:493, 821, 1365) | `ctx.emit(E)` with struct payloads (one listener arg) |
| `run: async (c)` + `c.queue.send('tick'/'review')` (:1735, 1813-1946) | removed: `ctx.schedule().after(.., "onTempoGate", &encode_positional(&OnTempoGate(turn_id))?)`; review work via `register_task` + `keep_awake_region` |
| `updateSettings(c, patch: unknown)` (player-profile:406) | `struct UpdateSettings(pub (serde_json::Value,));` (G32) |
| `listGames(c, req?: ListGamesRequest)` (:427), `listEpisodes(c, limit?: unknown)` (:437) | `struct ListGames(pub Option<ListGamesRequest>);` (G34 if JS sends `undefined`) |
| `getReplay(c, matchId: unknown)`, `forgetEpisode(c, episodeId: unknown)` (:432, 442) | `(Value,)` newtype, or `(String,)` / `String` newtype if always a string |
| `pruneEpisodes(c, internalToken)` (:469) | internal guard via `ctx.conn().is_none()` (schedule) or keep the token |
| `aiSeat createState(c, input \| undefined)` (ai-seat:536) | `type Input = AiSeatCreateInput` (Default) via `create_with_input` |

---

## 15. Source reference index

| Fact | Location |
|---|---|
| Actor trait, hooks | `rivetkit-3.1.0/src/actor.rs:36-137` |
| `Raw` action type | `rivetkit-3.1.0/src/action.rs:565-577` |
| `encode_positional` / `encode_varargs` / `to_json_value` | `rivetkit-3.1.0/src/action.rs:25-40` |
| `decode_positional` | `rivetkit-3.1.0/src/action.rs:411-433` |
| `positional_value` | `rivetkit-3.1.0/src/action.rs:436-444` |
| Ctx methods, `client()` construction | `rivetkit-3.1.0/src/context.rs:93-350` |
| `Schedule` wrapper | `rivetkit-3.1.0/src/context.rs:359-388` |
| Action dispatch (drops `scheduled_fire`) | `rivetkit-3.1.0/src/start.rs:330-357` |
| Registry `start`/`serve_with_config`/`into_serverless_runtime` | `rivetkit-3.1.0/src/registry.rs:44-188` |
| Serverless listener (`RIVET_PORT`, 3000) | `rivetkit-3.1.0/src/serverless_listener.rs:25-58` |
| Serverless base path `/api/rivet` | `rivetkit-core-3.1.0/src/serverless.rs:31` |
| `ServeSettings::from_env` | `rivetkit-core-3.1.0/src/registry/envoy_callbacks.rs:165-241` |
| `ServeConfig` fields / `from_env` | `rivetkit-core-3.1.0/src/registry/mod.rs:292-314`, `envoy_callbacks.rs:256-283` |
| `EngineSpawnMode::from_env`, `RuntimeMode::from_env` | `rivetkit-core-3.1.0/src/registry/mod.rs:255-289` |
| `resolve_engine_spawn` | `rivetkit-core-3.1.0/src/registry/mod.rs:391-405` |
| Endpoint URL auth / local check | `rivetkit-shared-types-3.1.0/src/endpoint.rs:58-121` |
| ActorConfig defaults | `rivetkit-core-3.1.0/src/actor/config.rs:10-25, 388-424, 600-614` |
| Sleep eligibility | `rivetkit-core-3.1.0/src/actor/sleep.rs:218-251` |
| Work kinds | `rivetkit-core-3.1.0/src/actor/work_registry.rs:47-95` |
| Broadcast subscription filter | `rivetkit-core-3.1.0/src/actor/context.rs:1022-1028` |
| `conns()` lock warning | `rivetkit-core-3.1.0/src/actor/context.rs:1030-1037` |
| ConnHandle `send` (logs and drops) / `state` | `rivetkit-core-3.1.0/src/actor/connection.rs:228-292` |
| Public error groups / masking | `rivetkit-core-3.1.0/src/error.rs:23-49, 138` |
| KV value-too-large message | `rivetkit-core-3.1.0/src/error.rs:117` |
| Queue API (deprecated on the client) | `rivetkit-core-3.1.0/src/actor/queue.rs:243, 363`; `rivetkit-client-3.1.0/src/handle.rs:129` |
| RivetError derive writes artifacts | `rivet-error-macros-3.1.0/src/lib.rs:581-660` |
| Client Drop broadcast / shared sender | `rivetkit-client-3.1.0/src/client.rs:137, 252, 358-362` |
| Reconnect loop / backoff | `rivetkit-client-3.1.0/src/connection.rs:684-738`; `backoff.rs` |
| `try_connect` | `rivetkit-client-3.1.0/src/connection.rs:179-260` |
| `disconnect()` clears in-flight RPCs | `rivetkit-client-3.1.0/src/connection.rs:656-677` |
| Error string formats | `rivetkit-client-3.1.0/src/connection.rs:455`, `handle.rs:117` |
| Engine stop on drop / orphan | `rivetkit-engine-process-3.1.0/src/lib.rs:972-1010` |
| JS undefined/BigInt/bytes tagging | `rivetkit@3.x common/encoding.ts:280-350` (`encodeJsonCompatValue`) |
| JS event listener arg spread | `rivetkit@3.x actor-conn.ts:1148-1156` |
