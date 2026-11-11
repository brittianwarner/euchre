//! The `euchreTable` actor on rivetkit 3.1.0 (lane SERVER-TABLE, D29).
//!
//! One actor per match, key `["table", gameId]`, created with a
//! `TableCreateInput`. It owns the authoritative `GameState` (server-only),
//! applies every move through the engine, holds the read pauses and AI pacing,
//! and fans out a per-seat redacted `sync` plus `thinking`, `chat`,
//! `chatDelta` and `presence` (03).
//!
//! # Registering (integration wave)
//!
//! ```ignore
//! let table = euchre_table::TableConfig::from_server_config(&cfg);
//! euchre_table::register(&mut registry, table)?;
//! // or, with a custom rivetkit ActorConfig:
//! euchre_table::register_with(&mut registry, table, rivetkit::ActorConfig::default())?;
//! ```
//!
//! rivetkit builds actors with `Actor::create(ctx)` and no factory argument,
//! so [`register`] installs the [`TableConfig`] process-wide. **One config per
//! process:** the same config again is a no-op, a different one is refused
//! with [`ConfigureError::ConfigConflict`]. A table that wakes before any call
//! fails to start with [`actor::NOT_CONFIGURED`].
//!
//! # Calling it
//!
//! - The player connects with params `{ token, protocolVersion: 1 }` (a guest
//!   actor JWT, `GuestAudience::Actors`) from an allowed Origin. The first
//!   verified visitor of an unclaimed table becomes its owner.
//! - The table's `aiSeat`s call with params `{ internalToken }` (the per-match
//!   token) and one positional body argument.
//! - See [`actions`] for every action's arguments and result.
//!
//! # Calling other actors
//!
//! The table asks its `aiSeat`s for decisions and records the match in the
//! owner's `playerProfile`, by actor name and action name only (D29), through
//! the shared JSON client in [`TableConfig::client`]. Bind it once the registry
//! serves:
//!
//! ```ignore
//! let table = euchre_table::TableConfig::from_server_config(&cfg);
//! let client = table.client.clone();
//! euchre_table::register(&mut registry, table)?;
//! // ... after the engine is up:
//! client.bind(shared_json_client.clone());
//! ```
//!
//! Every call is detached from the table's lane and bounded (`send_bounded`).
//! Each AI seat and the profile get one ordered outbox, so a seat sees
//! `resetHand`, `decide`, `handEnd` and `gameEnd` in the order the table sent
//! them. With no client bound the table plays the parked `euchre-ai`
//! heuristic move at the think floor (D8). A match record waits on the
//! `flushProfile` retry until a client is bound. Only a deployment with no
//! profile token records nothing, as TS does with no deployment secret.

pub mod actions;
pub mod actor;
pub mod auth;
pub(crate) mod core;
pub mod create_input;
pub(crate) mod remote;
pub(crate) mod seams;
pub mod send_bounded;
pub mod state;
pub mod tempo;
pub mod test_support;
pub mod text;

use std::sync::Arc;

use euchre_server_core::auth::Authenticator;
use euchre_server_core::config::{OriginPolicy, Secret, ServerConfig};
use rivetkit::client::Client;
use rivetkit::{ActorConfig, Registry};

pub use actions::*;
pub use actor::EuchreTable;

/// The registry name (TS `euchreTable`).
pub const ACTOR_NAME: &str = "euchreTable";

/// Log event when [`configure`] refuses a second, different config.
pub const CONFIG_CONFLICT_EVENT: &str = "euchreTable config already installed";

/// The shared client the tables call other actors through: one JSON
/// [`Client`] behind an `Arc` (G26), bound after the registry serves.
///
/// Clones share one slot. Until a client is bound the tables ask no `aiSeat`
/// (the parked heuristic plays, D8), and match records wait on the
/// `flushProfile` retry.
#[derive(Clone, Default)]
pub struct TableClient {
    slot: Arc<parking_lot::RwLock<Option<Arc<Client>>>>,
}

impl std::fmt::Debug for TableClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableClient")
            .field("bound", &self.slot.read().is_some())
            .finish()
    }
}

impl TableClient {
    /// A slot holding `client`.
    pub fn new(client: Arc<Client>) -> Self {
        let me = Self::deferred();
        me.bind(client);
        me
    }

    /// An empty slot, bound later with [`TableClient::bind`].
    pub fn deferred() -> Self {
        Self::default()
    }

    /// Install `client` for every clone of this slot, replacing any earlier one
    /// (a test binds each engine's client in turn).
    pub fn bind(&self, client: Arc<Client>) {
        *self.slot.write() = Some(client);
    }

    /// Empty the slot: tables go back to the heuristic, and records wait for a
    /// new bind.
    pub fn unbind(&self) {
        *self.slot.write() = None;
    }

    /// The bound client.
    pub fn get(&self) -> Option<Arc<Client>> {
        self.slot.read().clone()
    }

    fn same_slot(&self, other: &TableClient) -> bool {
        Arc::ptr_eq(&self.slot, &other.slot)
    }
}

/// What every table in this process needs: the token keys, the Origin
/// allowlist, the pacing profile, the shared client and the profile token.
#[derive(Clone, Debug)]
pub struct TableConfig {
    /// Verifies guest actor JWTs.
    pub auth: Authenticator,
    /// The Origin allowlist for player connections.
    pub origins: OriginPolicy,
    /// `EUCHRE_FAST_TEMPO=1` pacing.
    pub fast_tempo: bool,
    /// Where `aiSeat` and `playerProfile` calls go.
    pub client: TableClient,
    /// The `internalToken` of `playerProfile` writes (TS `profileInternalToken()`:
    /// `EUCHRE_INTERNAL_TOKEN`, or the dev key in local dev). `None` records
    /// nothing, as TS does with no deployment secret.
    pub profile_token: Option<Secret>,
}

impl TableConfig {
    /// The table's slice of the server config, with an unbound client.
    pub fn from_server_config(cfg: &ServerConfig) -> Self {
        TableConfig {
            auth: Authenticator::from_config(cfg),
            origins: cfg.origins.clone(),
            fast_tempo: cfg.fast_tempo,
            client: TableClient::deferred(),
            profile_token: Some(cfg.secrets.internal.value.clone()),
        }
    }

    fn same(&self, other: &TableConfig) -> bool {
        self.auth.keys() == other.auth.keys()
            && self.origins == other.origins
            && self.fast_tempo == other.fast_tempo
            && self.client.same_slot(&other.client)
            && self.profile_token == other.profile_token
    }
}

static CONFIG: parking_lot::RwLock<Option<TableConfig>> = parking_lot::RwLock::new(None);

/// Why [`configure`] refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigureError {
    /// Another config is already installed in this process. It stays; the
    /// table is not registered.
    #[error("euchreTable is already configured differently: one config per process")]
    ConfigConflict,
}

/// Install the config every `euchreTable` in this process uses.
pub fn configure(config: TableConfig) -> Result<(), ConfigureError> {
    let mut slot = CONFIG.write();
    match slot.as_ref() {
        None => {
            *slot = Some(config);
            Ok(())
        }
        Some(existing) if existing.same(&config) => Ok(()),
        Some(_) => {
            tracing::error!("{CONFIG_CONFLICT_EVENT}");
            Err(ConfigureError::ConfigConflict)
        }
    }
}

/// The installed config, if [`configure`] ran.
pub fn configured() -> Option<TableConfig> {
    CONFIG.read().clone()
}

/// The actor key of a match: `["table", gameId]`.
pub fn actor_key(game_id: &str) -> [String; 2] {
    euchre_protocol::table_key(game_id)
}

/// [`configure`], then register [`EuchreTable`] as `euchreTable` with
/// rivetkit's default [`ActorConfig`].
pub fn register(registry: &mut Registry, config: TableConfig) -> Result<(), ConfigureError> {
    register_with(registry, config, ActorConfig::default())
}

/// [`register`] with an explicit [`ActorConfig`] (tests shorten `sleep_timeout`).
pub fn register_with(
    registry: &mut Registry,
    config: TableConfig,
    actor_config: ActorConfig,
) -> Result<(), ConfigureError> {
    configure(config)?;
    registry.register_actor_with::<EuchreTable>(ACTOR_NAME, actor_config);
    Ok(())
}
