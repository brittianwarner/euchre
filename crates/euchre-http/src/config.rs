//! The router's configuration: a small struct built from a [`ServerConfig`] or by hand.

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use euchre_server_core::ServerConfig;
use euchre_server_core::config::{IpCidr, OriginPolicy, Secret, forwarded_for_trusted};

use crate::downloads::BucketSettings;

/// 1 MiB, TS `Bun.serve({ maxRequestBodySize })`.
pub const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;

/// Everything [`crate::router`] needs besides the authenticator and the actor client.
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// `ALLOWED_ORIGINS`: the Origin allowlist (CORS, gateway, enroll), and the
    /// public-origin resolver that maps a request `Host` to its origin. A host that
    /// resolves to no origin gets `421 Unknown host`. In local mode, `allow()` the
    /// server's own loopback origin once its port is known.
    pub origins: OriginPolicy,
    /// Private engine URL, for example `http://rivet-engine.railway.internal:6420`.
    pub engine_endpoint: String,
    /// Engine namespace, returned by `/api/rivet/metadata`.
    pub namespace: String,
    /// Runner pool tables are created in.
    pub pool_name: String,
    /// Engine admin token. When set, `POST /api/table-session/{gameId}` mints an
    /// engine gateway token (D3 Option A), as TS did when `RIVET_TOKEN` was set.
    pub engine_token: Option<Secret>,
    /// Add `Secure` to every cookie (off for plain-http localhost).
    pub secure_cookies: bool,
    /// Static web build (`build/client`). `None` serves no static routes.
    pub static_dir: Option<PathBuf>,
    /// Desktop downloads folder ([`crate::downloads`]): release files plus
    /// `manifest.json`, served under `/download/`. `None`, or a folder that does
    /// not exist, serves the empty manifest and `404`s.
    pub downloads_dir: Option<PathBuf>,
    /// Desktop downloads bucket ([`crate::downloads::bucket`]). When set it wins
    /// over [`ApiConfig::downloads_dir`]: the manifest comes from the bucket and
    /// file routes redirect to presigned bucket URLs.
    pub downloads_bucket: Option<BucketSettings>,
    /// Trust the last `X-Forwarded-For` entry as the client IP for the D11 buckets
    /// and the engine Guard (`EUCHRE_TRUST_FORWARDED_FOR=1`). Turn it on only when
    /// every request reaches this server through a proxy that appends that header
    /// (the Railway edge): reached directly, a client picks its own bucket by
    /// writing the header. Off unless configured; always off for loopback servers.
    pub trust_forwarded_for: bool,
    /// `EUCHRE_TRUSTED_PROXIES`: when non-empty, `X-Forwarded-For` counts only on
    /// requests whose socket peer is inside one of these networks. Empty trusts any
    /// peer once [`ApiConfig::trust_forwarded_for`] is on.
    pub trusted_proxies: Vec<IpCidr>,
    /// Per-IP token buckets (D11).
    pub rate_limits: RateLimitConfig,
    /// `/rivet/**` gateway tunables.
    pub gateway: GatewayOptions,
    /// Bounds on actor and engine calls.
    pub timeouts: CallTimeouts,
}

impl ApiConfig {
    /// Derive the router configuration from the process configuration.
    ///
    /// `static_dir`, `downloads_dir` and `downloads_bucket` stay `None`; set them
    /// when a web build or desktop downloads are served.
    pub fn from_server_config(cfg: &ServerConfig) -> Self {
        Self {
            origins: cfg.origins.clone(),
            engine_endpoint: cfg.engine.endpoint.clone(),
            namespace: cfg.engine.namespace.clone(),
            pool_name: cfg.engine.pool_name.clone(),
            engine_token: cfg.engine.token.clone(),
            secure_cookies: cfg.http.secure_cookies,
            static_dir: None,
            downloads_dir: None,
            downloads_bucket: None,
            trust_forwarded_for: cfg.http.trust_forwarded_for,
            trusted_proxies: cfg.http.trusted_proxies.clone(),
            rate_limits: RateLimitConfig::default(),
            gateway: GatewayOptions::default(),
            timeouts: CallTimeouts::default(),
        }
    }

    /// Whether a request whose socket peer is `peer` may name its client in
    /// `X-Forwarded-For`: [`ApiConfig::trust_forwarded_for`] is on and, when
    /// [`ApiConfig::trusted_proxies`] is non-empty, the peer is inside it.
    pub fn trusts_forwarded_for_from(&self, peer: Option<IpAddr>) -> bool {
        forwarded_for_trusted(self.trust_forwarded_for, &self.trusted_proxies, peer)
    }
}

/// One token bucket: `capacity` requests in a burst, refilled at `per_minute`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BucketConfig {
    /// Burst size.
    pub capacity: u32,
    /// Tokens added per minute.
    pub per_minute: u32,
}

impl BucketConfig {
    /// A bucket that never empties.
    pub const UNLIMITED: Self = Self {
        capacity: u32::MAX,
        per_minute: u32::MAX,
    };

    /// `capacity` burst, refilled at `per_minute`.
    pub const fn new(capacity: u32, per_minute: u32) -> Self {
        Self {
            capacity,
            per_minute,
        }
    }
}

/// D11: per-IP token buckets, in process.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimitConfig {
    /// `POST /api/device/enroll`.
    pub enroll: BucketConfig,
    /// `POST /api/identity/identify`.
    pub identify: BucketConfig,
    /// `POST /api/tables`.
    pub tables: BucketConfig,
    /// `POST /api/table-session/{gameId}`.
    pub table_session: BucketConfig,
    /// `/rivet/**` (05 §12.2 admission step 3; the engine Guard is the backstop).
    pub gateway: BucketConfig,
    /// `GET /download/files/<file>`, counted in MiB sent, not in requests
    /// ([`crate::downloads`]): about 1 GiB at once, then about 1 GiB an hour.
    pub download: BucketConfig,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enroll: BucketConfig::new(10, 10),
            identify: BucketConfig::new(20, 20),
            tables: BucketConfig::new(30, 30),
            table_session: BucketConfig::new(60, 120),
            gateway: BucketConfig::new(600, 1200),
            download: BucketConfig::new(1024, 17),
        }
    }
}

impl RateLimitConfig {
    /// No limits (tests, embedded single-user servers).
    pub const fn unlimited() -> Self {
        Self {
            enroll: BucketConfig::UNLIMITED,
            identify: BucketConfig::UNLIMITED,
            tables: BucketConfig::UNLIMITED,
            table_session: BucketConfig::UNLIMITED,
            gateway: BucketConfig::UNLIMITED,
            download: BucketConfig::UNLIMITED,
        }
    }
}

/// `/rivet/**` gateway tunables (TS `rivet-gateway-proxy.ts` constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayOptions {
    /// `DEFAULT_MAX_BUFFERED_BYTES`: both relay queues together; over it both legs
    /// close with `1013 backpressure limit`.
    pub max_buffered_bytes: usize,
    /// `DEFAULT_MAX_PAYLOAD_BYTES`: the largest WebSocket message and frame.
    pub max_payload_bytes: usize,
    /// `DEFAULT_UPSTREAM_DIAL_TIMEOUT_MS`.
    pub upstream_dial_timeout: Duration,
    /// `HTTP_PROXY_TIMEOUT_MS`.
    pub http_timeout: Duration,
    /// Bun `idleTimeout: 960`: a browser leg silent this long is closed.
    pub idle_timeout: Duration,
    /// Bun `sendPings`: ping the browser leg this often so live clients stay busy.
    pub ping_interval: Duration,
}

/// 16 MiB.
pub const DEFAULT_MAX_BUFFERED_BYTES: usize = 16 * 1024 * 1024;
/// 16 MiB.
pub const DEFAULT_MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

impl Default for GatewayOptions {
    fn default() -> Self {
        Self {
            max_buffered_bytes: DEFAULT_MAX_BUFFERED_BYTES,
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            upstream_dial_timeout: Duration::from_millis(10_000),
            http_timeout: Duration::from_millis(120_000),
            idle_timeout: Duration::from_secs(960),
            ping_interval: Duration::from_secs(30),
        }
    }
}

/// Bounds on actor and engine calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallTimeouts {
    /// TS `AbortSignal.timeout(12_000)` on table-session and profile calls.
    pub actor_call: Duration,
    /// TS `AbortSignal.timeout(5000)` per recent-table snapshot.
    pub recent_snapshot: Duration,
    /// TS 2 s race on persona settings in `/play`.
    pub persona_settings: Duration,
    /// Engine `POST /auth/tokens`.
    pub gateway_token: Duration,
}

impl Default for CallTimeouts {
    fn default() -> Self {
        Self {
            actor_call: Duration::from_millis(12_000),
            recent_snapshot: Duration::from_millis(5_000),
            persona_settings: Duration::from_millis(2_000),
            gateway_token: Duration::from_millis(10_000),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_server_never_trusts_forwarded_for() {
        let cfg = ServerConfig::local(std::env::temp_dir());
        assert!(!cfg.production);
        let api = ApiConfig::from_server_config(&cfg);
        assert!(!api.trust_forwarded_for);
        assert!(api.trusted_proxies.is_empty());
        assert!(!api.trusts_forwarded_for_from(None));
    }

    fn server_config(vars: &[(&str, &str)]) -> anyhow::Result<ServerConfig> {
        let mut map: std::collections::HashMap<String, String> = [
            ("NODE_ENV", "production"),
            (
                "RIVET_ENDPOINT",
                "http://rivet-engine.railway.internal:6420",
            ),
            ("RIVET_NAMESPACE", "euchre"),
            ("RIVET_TOKEN", "admin-token"),
            ("APP_URL", "https://euchre.example"),
            ("ALLOWED_ORIGINS", "https://euchre.example"),
            (
                "EUCHRE_ACTOR_JWT_SECRET",
                "0123456789abcdef0123456789abcdef-guest",
            ),
            (
                "EUCHRE_INTERNAL_TOKEN",
                "0123456789abcdef0123456789abcdef-internal",
            ),
            (
                "EUCHRE_IDENTITY_SECRET",
                "0123456789abcdef0123456789abcdef-identity",
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        for (k, v) in vars {
            map.insert((*k).to_owned(), (*v).to_owned());
        }
        ServerConfig::from_map(&map)
    }

    #[test]
    fn from_server_config_reads_the_explicit_setting_not_the_mode() -> anyhow::Result<()> {
        // A production server with a remote engine: the old inference said "trust".
        let api = ApiConfig::from_server_config(&server_config(&[])?);
        assert!(
            !api.trust_forwarded_for,
            "unset means off, even in production"
        );
        assert!(!api.trusts_forwarded_for_from(Some("10.0.0.1".parse()?)));

        let api =
            ApiConfig::from_server_config(&server_config(&[("EUCHRE_TRUST_FORWARDED_FOR", "1")])?);
        assert!(api.trust_forwarded_for);
        assert!(api.trusted_proxies.is_empty());
        assert!(api.trusts_forwarded_for_from(Some("203.0.113.9".parse()?)));

        let api = ApiConfig::from_server_config(&server_config(&[
            ("EUCHRE_TRUST_FORWARDED_FOR", "1"),
            ("EUCHRE_TRUSTED_PROXIES", "100.64.0.0/10"),
        ])?);
        assert_eq!(
            api.trusted_proxies,
            vec!["100.64.0.0/10".parse::<IpCidr>()?]
        );
        assert!(api.trusts_forwarded_for_from(Some("100.64.3.4".parse()?)));
        assert!(!api.trusts_forwarded_for_from(Some("203.0.113.9".parse()?)));
        assert!(!api.trusts_forwarded_for_from(None));
        Ok(())
    }
}
