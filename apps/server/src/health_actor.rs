//! The `health` actor (TS `registry.ts`, 05 §5.5): `ping() -> {ok: true, ts}`,
//! counting pings in its state. The server pings it to know the envoy serves
//! actors before it reports ready (G36).

use std::sync::Arc;

use async_trait::async_trait;
use euchre_server_core::rivet::JsonTable;
use rivetkit::prelude::*;
use rivetkit::{Action, ActorConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Actor name, as TS registered it.
pub const HEALTH_ACTOR: &str = "health";

/// The actor's state row (TS `state`), in its SQLite database.
const STATE_TABLE: &str = "health_state";
const STATE_KEY: &str = "state";

/// TS `state: { pings: 0 }`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
struct HealthState {
    pings: u64,
}

/// The one health actor's key.
pub fn health_key() -> Vec<String> {
    vec![HEALTH_ACTOR.to_owned()]
}

/// `ping()`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Ping(pub Vec<Value>);

impl Action for Ping {
    type Output = PingReply;
    const NAME: &'static str = "ping";
}

/// `{ ok: true, ts }`, `ts` in unix milliseconds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PingReply {
    /// Always true.
    pub ok: bool,
    /// Server time in unix milliseconds.
    pub ts: i64,
}

/// The liveness actor.
pub struct HealthActor {
    lane: tokio::sync::Mutex<()>,
    table: JsonTable,
}

#[async_trait]
impl Actor for HealthActor {
    type Input = Value;
    type ConnParams = Value;
    type Action = rivetkit::Raw;
    type Actions = (Ping,);
    type Events = ();

    async fn create(ctx: &Ctx<Self>) -> Result<Self> {
        let table = JsonTable::new(STATE_TABLE)?;
        table.ensure(ctx.sql()).await?;
        Ok(Self {
            lane: tokio::sync::Mutex::new(()),
            table,
        })
    }
}

impl Handler<Ping> for HealthActor {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, _ping: Ping) -> Result<PingReply> {
        let _turn = self.lane.lock().await;
        let db = ctx.sql();
        let mut state: HealthState = self.table.load(db, STATE_KEY).await?.unwrap_or_default();
        state.pings = state.pings.saturating_add(1);
        self.table.save(db, STATE_KEY, &state).await?;
        Ok(PingReply {
            ok: true,
            ts: euchre_server_core::auth::now_ms(),
        })
    }
}

/// Register `health` with the TS display name and icon.
pub fn register(registry: &mut Registry) {
    registry.register_actor_with::<HealthActor>(
        HEALTH_ACTOR,
        ActorConfig {
            name: Some("Health".to_owned()),
            icon: Some("heart-pulse".to_owned()),
            ..ActorConfig::default()
        },
    );
}
