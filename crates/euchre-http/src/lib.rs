//! The axum HTTP API, the `/rivet/**` gateway and namespace bootstrap (lane
//! SERVER-HTTP, D29; 05 §11-§12).
//!
//! # Composition
//!
//! ```ignore
//! let cfg = ServerConfig::from_env()?;
//! let auth = Authenticator::from_config(&cfg);
//! let client = euchre_server_core::rivet::build_client_with_pool(
//!     &cfg.engine.endpoint, &cfg.engine.namespace,
//!     cfg.engine.token.as_ref().map(Secret::expose), Some(&cfg.engine.pool_name));
//! let mut api = ApiConfig::from_server_config(&cfg);
//! api.static_dir = Some("build/client".into());
//! api.downloads_dir = Some(euchre_http::downloads::dir_from_env());
//! let health = Health::new();
//! let state = ApiState::new(api, auth, client, health.clone())?;
//! let app = euchre_http::router(state);
//! // ... ensure_rivet_namespace, start the registry, then:
//! health.set_ready();
//! axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
//!     .with_graceful_shutdown(async move { signal.await; health.start_draining(); })
//!     .await?;
//! ```
//!
//! - [`router`] builds the whole app: `/health`, the JSON API, `/rivet/**`, the
//!   desktop downloads under `/download/` ([`downloads`]) and the static build.
//! - [`Health`] is the draining control: `/health` is `503 draining` until
//!   [`Health::set_ready`] and after [`Health::start_draining`].
//! - [`ensure_rivet_namespace`] and [`assert_rivet_env_safety`] are the boot checks.
//! - [`GatewayProxy`] is usable on its own (`GatewayProxy::router`).

pub mod api;
pub mod bootstrap;
pub mod caller;
pub mod config;
pub mod cookies;
pub mod downloads;
pub mod error;
pub mod gateway;
pub mod origin;
pub mod range;
pub mod rate_limit;
pub mod state;
pub mod static_files;
pub mod token;

pub use api::router;
pub use bootstrap::{
    AdminDialect, EnsureRivetNamespaceOptions, EnsureRivetNamespaceResult, RivetBootstrapError,
    RivetEnv, RivetEnvError, assert_rivet_env_safety, ensure_rivet_namespace,
};
pub use config::{ApiConfig, BucketConfig, CallTimeouts, GatewayOptions, RateLimitConfig};
pub use error::ApiError;
pub use gateway::{AllowedOrigins, GatewayProxy, GatewayProxyOptions};
pub use state::{ApiState, Health};
pub use token::{GatewayTokenIssuer, GatewayTokenIssuerOptions, IssuedToken};
