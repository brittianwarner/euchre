//! What every `aiSeat` in one registry shares: the client it calls other
//! actors with, the model provider, the profile secret and the log sink.
//!
//! rivetkit builds actors with `Actor::create(ctx)` and no factory argument, so
//! [`crate::register_with`] hands one [`AiSeatRuntime`] to the seats of that
//! registry (a task-local scope around rivetkit's actor entry). Two registries
//! in one process can therefore hold different runtimes.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use euchre_ai::ai_seat::bridge::{LlmProvider, ProviderDeps};
use euchre_ai::ai_seat::handlers::profile_internal_token;
use euchre_ai::clock::SystemClock;
use euchre_ai::log::TracingLog;
use euchre_ai::model::{model_factory_from_env_with, model_factory_from_process_env};
use euchre_ai::types::{Clock, SeatLog};
use euchre_server_core::config::ServerConfig;
use rivetkit::client::Client;

/// Bound on each call to another actor (`aiDecision`, `aiSay`,
/// `recordEpisodes`, `recordTokens`), so a stuck peer never wedges the seat.
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// The shared client the seat calls the table and the profile with (D19 JSON,
/// G26: one `Arc<Client>`, never `Client::clone()`).
///
/// It can be bound after construction ([`SeatClient::deferred`] then
/// [`SeatClient::bind`]), for hosts that only get the client once the registry
/// is serving (the test engine builds its client after the registry).
#[derive(Clone, Debug, Default)]
pub struct SeatClient {
    slot: Arc<OnceLock<Arc<Client>>>,
}

impl SeatClient {
    /// A client that is ready now.
    pub fn new(client: Arc<Client>) -> Self {
        let slot = OnceLock::new();
        let _ = slot.set(client);
        Self {
            slot: Arc::new(slot),
        }
    }

    /// A client to bind later with [`SeatClient::bind`]. Until then every call
    /// fails (and is logged as an undelivered send), never panics.
    pub fn deferred() -> Self {
        Self::default()
    }

    /// Bind the client. Returns `false` (and keeps the first) when one is bound.
    pub fn bind(&self, client: Arc<Client>) -> bool {
        self.slot.set(client).is_ok()
    }

    /// The bound client, if any.
    pub fn get(&self) -> Option<&Arc<Client>> {
        self.slot.get()
    }
}

/// Everything an `aiSeat` needs from its host (the composition root).
#[derive(Clone)]
pub struct AiSeatRuntime {
    pub(crate) client: SeatClient,
    pub(crate) provider: LlmProvider,
    pub(crate) profile_token: Option<String>,
    pub(crate) clock: Arc<dyn Clock + Send + Sync>,
    pub(crate) log: Arc<dyn SeatLog + Send + Sync>,
    pub(crate) call_timeout: Duration,
}

impl std::fmt::Debug for AiSeatRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the profile secret.
        f.debug_struct("AiSeatRuntime")
            .field("client_bound", &self.client.get().is_some())
            .field("decider", &self.provider.decider.is_some())
            .field("banter", &self.provider.banter.is_some())
            .field(
                "profile_token",
                &self.profile_token.as_ref().map(|_| "<set>"),
            )
            .field("call_timeout", &self.call_timeout)
            .finish()
    }
}

impl AiSeatRuntime {
    /// A runtime with an explicit provider and profile secret, the wall clock
    /// ([`SystemClock`]), the `tracing` log sink and [`DEFAULT_CALL_TIMEOUT`].
    ///
    /// `profile_token` is the `internalToken` the seat puts in its
    /// `playerProfile` writes. Pass the server's resolved internal secret
    /// (`ServerConfig.secrets.internal`); `None` keeps episodes buffered and
    /// skips the token flush, as TS did without a deployment secret.
    pub fn new(client: SeatClient, provider: LlmProvider, profile_token: Option<String>) -> Self {
        Self {
            client,
            provider,
            profile_token,
            clock: Arc::new(SystemClock),
            log: Arc::new(TracingLog),
            call_timeout: DEFAULT_CALL_TIMEOUT,
        }
    }

    /// The production composition: the model factory from the server's
    /// resolved AI settings (`OPENROUTER_API_KEY`, `OPENROUTER_BASE_URL`,
    /// `OPENROUTER_MODEL_TALK`, then the Anthropic fallback) and, as the
    /// profile secret, the server's resolved internal secret
    /// (`cfg.secrets.internal`). That is the value `playerProfile` verifies
    /// against in every mode, including local mode, where it lives in
    /// `secrets.json` rather than in the environment.
    ///
    /// With no model key the provider is empty: every decision takes the
    /// heuristic and the seat says nothing, so the game runs offline.
    pub fn from_config(client: SeatClient, cfg: &ServerConfig) -> Self {
        let ai = &cfg.ai;
        let lookup = |k: &str| -> Option<String> {
            match k {
                "OPENROUTER_API_KEY" => ai
                    .openrouter_api_key
                    .as_ref()
                    .map(|s| s.expose().to_owned()),
                "OPENROUTER_BASE_URL" => ai.openrouter_base_url.clone(),
                "OPENROUTER_MODEL_TALK" => ai.openrouter_model_talk.clone(),
                "ANTHROPIC_API_KEY" => ai.anthropic_api_key.as_ref().map(|s| s.expose().to_owned()),
                "ANTHROPIC_BASE_URL" => ai.anthropic_base_url.clone(),
                _ => None,
            }
        };
        let factory = model_factory_from_env_with(&lookup, None).map(Arc::new);
        Self::new(
            client,
            Self::provider_for(factory),
            Some(cfg.secrets.internal.value.expose().to_owned()),
        )
    }

    /// Dev and tooling only: the model factory and the profile secret straight
    /// from the process environment (TS `profileInternalToken`, which falls
    /// back to the well-known dev secret when `RIVET_ENDPOINT` is unset).
    ///
    /// A local `euchre-server` does **not** verify profile writes against that
    /// dev secret (it keeps a generated one in `secrets.json`), so a server
    /// composes with [`AiSeatRuntime::from_config`] instead.
    pub fn dev_from_process_env(client: SeatClient) -> Self {
        let factory = model_factory_from_process_env().map(Arc::new);
        let token = profile_internal_token(&|k| std::env::var(k).ok());
        Self::new(client, Self::provider_for(factory), token)
    }

    fn provider_for(factory: Option<Arc<euchre_ai::model::ModelFactory>>) -> LlmProvider {
        LlmProvider::from_factory(
            factory,
            ProviderDeps {
                clock: Arc::new(SystemClock),
                log: Arc::new(TracingLog),
            },
        )
    }

    /// Replace the profile secret (for example with `ServerConfig.secrets.internal`,
    /// which local mode keeps in `secrets.json`, not in the environment).
    pub fn with_profile_token(mut self, token: Option<String>) -> Self {
        self.profile_token = token;
        self
    }

    /// Replace the model provider.
    pub fn with_provider(mut self, provider: LlmProvider) -> Self {
        self.provider = provider;
        self
    }

    /// Replace the log sink (tests record events with `euchre_ai::testing::RecordingLog`).
    pub fn with_log(mut self, log: Arc<dyn SeatLog + Send + Sync>) -> Self {
        self.log = log;
        self
    }

    /// Replace the bound on each call to another actor.
    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// The client handle (bind a deferred one through it).
    pub fn client(&self) -> &SeatClient {
        &self.client
    }

    /// The model provider.
    pub fn provider(&self) -> &LlmProvider {
        &self.provider
    }

    /// Whether a profile secret is set.
    pub fn has_profile_token(&self) -> bool {
        self.profile_token.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deferred_client_binds_once() {
        let c = SeatClient::deferred();
        assert!(c.get().is_none());
        let clone = c.clone();
        let client = euchre_server_core::rivet::build_client("http://127.0.0.1:1", "default", None);
        assert!(c.bind(client.clone()));
        // Clones share the slot; a second bind keeps the first.
        assert!(clone.get().is_some());
        assert!(!clone.bind(client));
    }

    #[test]
    fn debug_never_prints_the_profile_secret() {
        let rt = AiSeatRuntime::new(
            SeatClient::deferred(),
            LlmProvider::none(),
            Some("super-secret-profile-token".into()),
        );
        let text = format!("{rt:?}");
        assert!(!text.contains("super-secret"));
        assert!(text.contains("<set>"));
        assert!(rt.has_profile_token());
    }

    fn config(vars: &[(&str, &str)]) -> ServerConfig {
        let map = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        match ServerConfig::from_map(&map) {
            Ok(cfg) => cfg,
            Err(e) => panic!("config: {e:#}"),
        }
    }

    #[test]
    fn from_config_uses_the_servers_internal_secret() {
        let cfg = config(&[("EUCHRE_INTERNAL_TOKEN", "server-internal-0123456789abcdef")]);
        let rt = AiSeatRuntime::from_config(SeatClient::deferred(), &cfg);
        assert_eq!(
            rt.profile_token.as_deref(),
            Some(cfg.secrets.internal.value.expose())
        );
        assert_eq!(
            rt.profile_token.as_deref(),
            Some("server-internal-0123456789abcdef")
        );
        // No model key: heuristic only, silent.
        assert!(rt.provider.decider.is_none() && rt.provider.banter.is_none());
    }

    #[test]
    fn from_config_follows_a_generated_local_secret() {
        let dir = std::env::temp_dir().join(format!(
            "ai-seat-runtime-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let cfg = ServerConfig::local(dir.clone());
        let rt = AiSeatRuntime::from_config(SeatClient::deferred(), &cfg);
        let _ = std::fs::remove_dir_all(&dir);
        let secret = cfg.secrets.internal.value.expose();
        assert_eq!(rt.profile_token.as_deref(), Some(secret));
        assert_ne!(secret, euchre_ai::ai_seat::handlers::DEV_PROFILE_TOKEN);
    }

    #[test]
    fn from_config_builds_the_model_provider_from_the_ai_settings() {
        let cfg = config(&[
            ("EUCHRE_INTERNAL_TOKEN", "server-internal-0123456789abcdef"),
            ("OPENROUTER_API_KEY", "sk-or-test"),
        ]);
        let rt = AiSeatRuntime::from_config(SeatClient::deferred(), &cfg);
        assert!(rt.provider.banter.is_some(), "{rt:?}");
    }
}
