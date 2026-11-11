//! The `playerProfile` actor on rivetkit 3.1.0 (lane SERVER-PROFILE, D29).
//!
//! One actor per human, key `["user", userId]`: settings, the live match id,
//! today's model spend, lifetime totals and the SQLite history (matches, hands,
//! AI episodes). The pure rules live in `euchre_profile_model`; this crate runs
//! them against `ctx.sql()` and enforces who may call what (05 §3).
//!
//! # Registering (integration wave)
//!
//! ```ignore
//! let auth = euchre_server_core::auth::Authenticator::from_config(&cfg);
//! euchre_profile::register(&mut registry, auth)?;
//! // or, with a custom rivetkit ActorConfig:
//! euchre_profile::register_with(&mut registry, auth, rivetkit::ActorConfig::default())?;
//! ```
//!
//! [`register`] installs the [`Authenticator`] process-wide (the profile token
//! key and `EUCHRE_INTERNAL_TOKEN`), because rivetkit builds actors with
//! `Actor::create(ctx)` and no factory argument. Call it before the registry
//! serves. **One key set per process:** a later call with the same keys is a
//! no-op, and a call with different keys is refused with
//! [`ConfigureError::KeysConflict`] (and logged), so a second server in one
//! test binary cannot swap the keys under the first one's profiles. An actor
//! that wakes before any call fails to start with [`actor::NOT_CONFIGURED`].
//!
//! # Calling it
//!
//! Browser callers connect with params `{ token, protocolVersion: 1 }` (a
//! profile JWT from `Authenticator::mint_profile_token`). Other actors call
//! with no token and pass `internalToken` (in the body for the former queues,
//! as the first argument for the internal reads). See [`actions`] for every
//! action's arguments and result.

pub mod actions;
pub mod actor;
pub mod auth;
pub mod repository;
pub mod test_support;

use euchre_server_core::auth::Authenticator;
use rivetkit::{ActorConfig, Registry};

pub use actions::*;
pub use actor::PlayerProfile;
pub use auth::ConnAuth;

/// The registry name (TS `playerProfile`).
pub const ACTOR_NAME: &str = "playerProfile";

/// Log event when [`configure`] refuses a second, different key set.
pub const KEYS_CONFLICT_EVENT: &str = "playerProfile keys already configured";

static AUTH: parking_lot::RwLock<Option<Authenticator>> = parking_lot::RwLock::new(None);

/// Why [`configure`] refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigureError {
    /// Another key set is already installed in this process. The installed
    /// keys stay; the profile is not registered.
    #[error("playerProfile is already configured with different keys: one key set per process")]
    KeysConflict,
}

/// Install the keys every `playerProfile` in this process uses.
///
/// One key set per process: the same keys again is `Ok`; different keys are
/// refused (the first set stays) and logged, never swapped in silently.
pub fn configure(auth: Authenticator) -> Result<(), ConfigureError> {
    let mut slot = AUTH.write();
    match slot.as_ref() {
        None => {
            *slot = Some(auth);
            Ok(())
        }
        Some(existing) if existing.keys() == auth.keys() => Ok(()),
        Some(_) => {
            tracing::error!("{KEYS_CONFLICT_EVENT}");
            Err(ConfigureError::KeysConflict)
        }
    }
}

/// The installed keys, if [`configure`] ran.
pub fn configured() -> Option<Authenticator> {
    AUTH.read().clone()
}

/// The actor key of `user_id`'s profile: `["user", userId]`.
pub fn actor_key(user_id: &str) -> [String; 2] {
    [
        euchre_profile_model::PROFILE_KEY_NAMESPACE.to_owned(),
        user_id.to_owned(),
    ]
}

/// [`configure`] `auth`, then register [`PlayerProfile`] as `playerProfile`
/// with rivetkit's default [`ActorConfig`].
///
/// One key set per process (see [`configure`]): on
/// [`ConfigureError::KeysConflict`] nothing is registered.
pub fn register(registry: &mut Registry, auth: Authenticator) -> Result<(), ConfigureError> {
    register_with(registry, auth, ActorConfig::default())
}

/// [`register`] with an explicit [`ActorConfig`] (tests shorten `sleep_timeout`).
pub fn register_with(
    registry: &mut Registry,
    auth: Authenticator,
    config: ActorConfig,
) -> Result<(), ConfigureError> {
    configure(auth)?;
    registry.register_actor_with::<PlayerProfile>(ACTOR_NAME, config);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use euchre_server_core::auth::AuthKeys;
    use euchre_server_core::config::Secret;

    fn auth(profile: &str) -> Authenticator {
        Authenticator::new(AuthKeys {
            guest: Secret::new("unit-guest-key-0123456789abcdef"),
            profile: Secret::new(profile),
            internal: Secret::new("unit-internal-token-0123456789ab"),
            identity: Secret::new("unit-identity-key-0123456789abcd"),
            jwt_issuer: None,
            allow_unsigned_email: false,
        })
    }

    #[test]
    fn configure_keeps_one_key_set_per_process() {
        let first = auth("unit-profile-key-a-0123456789abcd");
        assert_eq!(configure(first.clone()), Ok(()));
        // The same keys again (a second registry, same config) is fine.
        assert_eq!(configure(first.clone()), Ok(()));
        // Different keys are refused and the first set stays.
        let other = auth("unit-profile-key-b-0123456789abcd");
        assert_eq!(configure(other), Err(ConfigureError::KeysConflict));
        let installed = configured().map(|a| a.keys().clone());
        assert_eq!(installed.as_ref(), Some(first.keys()));
    }
}
