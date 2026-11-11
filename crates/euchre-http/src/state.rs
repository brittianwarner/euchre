//! Shared router state and the readiness/draining switch.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use euchre_server_core::auth::Authenticator;
use rivetkit_client::Client;

use crate::config::ApiConfig;
use crate::downloads::{DownloadOptions, Downloads};
use crate::gateway::{AllowedOrigins, GatewayProxy, GatewayProxyOptions};
use crate::rate_limit::RateLimiter;
use crate::token::{GatewayTokenIssuer, GatewayTokenIssuerOptions};

/// `/health` state: `503 draining` until [`Health::set_ready`], and again after
/// [`Health::start_draining`]. Cheap to clone; every clone shares the switch.
#[derive(Debug, Clone, Default)]
pub struct Health {
    ready: Arc<AtomicBool>,
    draining: Arc<AtomicBool>,
}

impl Health {
    /// Not ready, not draining.
    pub fn new() -> Self {
        Self::default()
    }

    /// The actor runner is connected: `/health` may answer `ok`.
    pub fn set_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    /// Shutdown began: `/health` answers `503 draining` from now on.
    pub fn start_draining(&self) {
        self.draining.store(true, Ordering::Release);
    }

    /// True while ready and not draining.
    pub fn is_serving(&self) -> bool {
        self.ready.load(Ordering::Acquire) && !self.draining.load(Ordering::Acquire)
    }

    /// True after [`Health::start_draining`].
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }
}

pub(crate) struct Inner {
    pub(crate) config: ApiConfig,
    pub(crate) auth: Authenticator,
    pub(crate) client: Arc<Client>,
    pub(crate) limiter: RateLimiter,
    /// The `/download/**` handler: its manifest cache and stream limits are
    /// shared by every router built from this state.
    pub(crate) downloads: Downloads,
    pub(crate) issuer: Option<GatewayTokenIssuer>,
    pub(crate) proxy: GatewayProxy,
    pub(crate) health: Health,
}

/// Everything the handlers share. Cheap to clone.
#[derive(Clone)]
pub struct ApiState {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for ApiState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiState")
            .field("engine_endpoint", &self.inner.config.engine_endpoint)
            .field("namespace", &self.inner.config.namespace)
            .finish_non_exhaustive()
    }
}

impl ApiState {
    /// Build the state.
    ///
    /// `client` is the process's one shared actor client (G26: never clone the
    /// `Client` itself). `health` starts not ready; call [`Health::set_ready`] once
    /// the actor runner is connected.
    pub fn new(
        config: ApiConfig,
        auth: Authenticator,
        client: Arc<Client>,
        health: Health,
    ) -> anyhow::Result<Self> {
        let http = GatewayProxy::http_client()?;
        let issuer = config.engine_token.clone().map(|admin_token| {
            GatewayTokenIssuer::new(
                GatewayTokenIssuerOptions {
                    endpoint: config.engine_endpoint.clone(),
                    namespace: config.namespace.clone(),
                    admin_token,
                    timeout: config.timeouts.gateway_token,
                },
                http.clone(),
            )
        });
        let proxy = GatewayProxy::new(
            GatewayProxyOptions {
                upstream_http: config.engine_endpoint.clone(),
                allowed_origins: AllowedOrigins::Policy(config.origins.clone()),
                prefix: "/rivet".to_owned(),
                tunables: config.gateway,
            },
            http,
        );
        Ok(Self {
            inner: Arc::new(Inner {
                limiter: RateLimiter::new(config.rate_limits),
                downloads: match &config.downloads_bucket {
                    Some(bucket) => Downloads::bucket(bucket.clone()),
                    None => Downloads::with_options(DownloadOptions {
                        budget_mib: config.rate_limits.download,
                        ..DownloadOptions::new(config.downloads_dir.clone())
                    }),
                },
                config,
                auth,
                client,
                issuer,
                proxy,
                health,
            }),
        })
    }

    /// The readiness switch.
    pub fn health(&self) -> &Health {
        &self.inner.health
    }

    /// The configuration.
    pub fn config(&self) -> &ApiConfig {
        &self.inner.config
    }
}
