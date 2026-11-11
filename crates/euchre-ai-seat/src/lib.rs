//! The `aiSeat` actor glue over `euchre_ai::ai_seat` (lane SERVER-AI-SEAT, D29).
//!
//! One actor per AI chair, key `["table", gameId, "seat", "1"|"2"|"3"]`, created
//! by `euchreTable` with an `AiSeatCreateInput` (03 §10.2). It answers each
//! `decide` with an `AIDecision` sent to the table's `aiDecision` action, says
//! at most one line of table talk per hand through `aiSay`, and flushes AI
//! episodes and token spend to `playerProfile` (`recordEpisodes`,
//! `recordTokens`). See [`actions`] for every action and [`actor`] for the
//! ordering and authority rules.
//!
//! # Registering (integration wave)
//!
//! ```ignore
//! use euchre_ai_seat::{AiSeatRuntime, SeatClient};
//!
//! // The one shared JSON client of the server process (G26).
//! let client = euchre_server_core::rivet::build_client(&endpoint, &namespace, token);
//! // Models from cfg.ai; profile writes carry cfg.secrets.internal (local
//! // mode keeps it in secrets.json, not in the environment).
//! let runtime = AiSeatRuntime::from_config(SeatClient::new(client), &cfg);
//! euchre_ai_seat::register(&mut registry, runtime);
//! // or with a custom rivetkit ActorConfig:
//! euchre_ai_seat::register_with(&mut registry, runtime, rivetkit::ActorConfig::default());
//! ```
//!
//! The runtime is per registry: [`register_with`] wraps rivetkit's actor entry
//! in a scope that hands it to every seat that registry starts, so two
//! registries in one process (tests) can use different providers. With no
//! model key, [`AiSeatRuntime::from_config`] builds an empty provider:
//! decisions take the heuristic and banter is silent.
//!
//! # Calling it
//!
//! Callers of the four message actions (the table) pass params
//! `{ internalToken }` with the match's internal token; any other caller is
//! refused with `user/internal_token_mismatch`. They take the TS queue body as
//! their single argument and return `null` at once. The reads (`getStatus`,
//! `getPersona`, `getDecisionLog`) are open, as in TS.

pub mod actions;
pub mod actor;
pub mod calls;
pub mod runtime;
pub mod test_support;

use std::sync::Arc;

use rivetkit::{ActorConfig, Registry, run_actor};

pub use actions::*;
pub use actor::AiSeat;
pub use runtime::{AiSeatRuntime, DEFAULT_CALL_TIMEOUT, SeatClient};

/// The registry name (TS `aiSeat`).
pub const ACTOR_NAME: &str = "aiSeat";
/// The inspector display name (TS `options.name`).
pub const DISPLAY_NAME: &str = "AI Seat";
/// The inspector icon (TS `options.icon`).
pub const ICON: &str = "robot";

/// The actor key of seat `seat` of match `game_id`: `["table", gameId, "seat", n]`.
pub fn actor_key(game_id: &str, seat: u8) -> Vec<String> {
    vec![
        "table".to_owned(),
        game_id.to_owned(),
        "seat".to_owned(),
        seat.to_string(),
    ]
}

/// Register [`AiSeat`] as `aiSeat` with rivetkit's default [`ActorConfig`]
/// (plus the TS display name and icon).
pub fn register(registry: &mut Registry, runtime: AiSeatRuntime) {
    register_with(registry, runtime, ActorConfig::default());
}

/// [`register`] with an explicit [`ActorConfig`] (tests shorten
/// `sleep_timeout`). An unset `name` / `icon` takes the TS `AI Seat` / `robot`.
pub fn register_with(registry: &mut Registry, runtime: AiSeatRuntime, mut config: ActorConfig) {
    config.name.get_or_insert_with(|| DISPLAY_NAME.to_owned());
    config.icon.get_or_insert_with(|| ICON.to_owned());
    let runtime = Arc::new(runtime);
    // `register_with` (deprecated in favour of `register_actor_with`) is the
    // only entry that lets the runtime ride along: it runs the same
    // `run_actor` inside a task-local scope that `AiSeat::create` reads.
    #[allow(deprecated)]
    registry.register_with::<AiSeat, _, _>(ACTOR_NAME, config, move |start| {
        actor::RUNTIME.scope(runtime.clone(), run_actor::<AiSeat>(start))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_the_ts_shape() {
        assert_eq!(actor_key("g1", 3), vec!["table", "g1", "seat", "3"]);
        assert_eq!(ACTOR_NAME, "aiSeat");
    }
}
