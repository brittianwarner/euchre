//! Self-hosted engine bootstrap and the Rivet env containment guards: a port of TS
//! `server/bootstrap-rivet-namespace.ts` (05 §5.4).
//!
//! [`ensure_rivet_namespace`] makes sure the namespace and every runner pool exist
//! before the envoy runner dials out:
//!
//! - Idempotent: list before create, and `name_not_unique` counts as success (both
//!   replicas boot through this at once).
//! - Fail loud: transient errors retry until a deadline (the engine cold start races
//!   server boot), then fail boot. `401`/`403` fail at once.
//!
//! Engine 3.1.0 changed the admin API (`docs/port/requests/done/server-http-engine-admin-api.md`):
//! `POST /namespaces` takes only `{name}`, and pools are `PUT /pools/{pool}` with a
//! flat body. [`AdminDialect::V3`] speaks that; [`AdminDialect::V2`] keeps the TS
//! wire for an engine on the 2.3 API.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::Instant;

/// Default ceiling for the whole bootstrap (engine cold-start race).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(60_000);
/// Delay between attempts after a transient failure.
pub const DEFAULT_RETRY_DELAY: Duration = Duration::from_millis(1_000);

/// The envoy (`normal`) runner-pool config asserted on every boot (TS
/// `ENVOY_RUNNER_POOL_CONFIG`): drain older runners on a version upgrade, but
/// rate-limit eviction so a new fleet can register before the old one empties.
pub fn envoy_runner_pool_config() -> Value {
    json!({
        "drain_on_version_upgrade": true,
        "actor_eviction_period": 60,
        "actor_eviction_rate": 1,
    })
}

/// Which engine admin API to speak.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AdminDialect {
    /// rivet-engine 2.3: `POST /namespaces {name, display_name}` and
    /// `PUT /runner-configs/{pool} {datacenters: {default: {normal: …}}}` (TS).
    V2,
    /// rivet-engine 3.0: `POST /namespaces {name}` and `PUT /pools/{pool}` with the
    /// pool config as the body.
    #[default]
    V3,
}

/// Inputs of [`ensure_rivet_namespace`]. `Debug` redacts the token.
#[derive(Clone)]
pub struct EnsureRivetNamespaceOptions {
    /// Bare engine URL, for example `http://rivet-engine.railway.internal:6420`.
    /// No userinfo.
    pub endpoint: String,
    /// Engine admin token (`RIVET__AUTH__ADMIN_TOKEN` on the engine).
    pub token: String,
    /// Namespace to ensure.
    pub namespace: String,
    /// Runner pool the envoy runner registers under.
    pub runner_pool: String,
    /// Further pools to assert in the same namespace.
    pub additional_runner_pools: Vec<String>,
    /// Overall deadline of the retry loop.
    pub timeout: Duration,
    /// Delay between attempts.
    pub retry_delay: Duration,
    /// Admin API dialect.
    pub dialect: AdminDialect,
}

impl std::fmt::Debug for EnsureRivetNamespaceOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnsureRivetNamespaceOptions")
            .field("endpoint", &self.endpoint)
            .field("token", &"<redacted>")
            .field("namespace", &self.namespace)
            .field("runner_pool", &self.runner_pool)
            .field("additional_runner_pools", &self.additional_runner_pools)
            .field("timeout", &self.timeout)
            .field("retry_delay", &self.retry_delay)
            .field("dialect", &self.dialect)
            .finish()
    }
}

impl EnsureRivetNamespaceOptions {
    /// Options with the default deadline, delay and dialect.
    pub fn new(
        endpoint: impl Into<String>,
        token: impl Into<String>,
        namespace: impl Into<String>,
        runner_pool: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            token: token.into(),
            namespace: namespace.into(),
            runner_pool: runner_pool.into(),
            additional_runner_pools: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
            retry_delay: DEFAULT_RETRY_DELAY,
            dialect: AdminDialect::default(),
        }
    }
}

/// What [`ensure_rivet_namespace`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnsureRivetNamespaceResult {
    /// True when the namespace had to be created this boot.
    pub namespace_created: bool,
    /// Runner-pool config was (re)asserted: always true on success.
    pub runner_config_ensured: bool,
    /// Attempts the retry loop took.
    pub attempts: u32,
}

/// Why the bootstrap failed. Messages are the TS ones and carry no credential.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RivetBootstrapError {
    /// Retrying cannot fix it (auth): TS `RivetBootstrapFatalError`.
    #[error("{0}")]
    Fatal(String),
    /// The deadline passed.
    #[error("{0}")]
    Timeout(String),
}

enum AttemptError {
    Fatal(String),
    Transient(String),
}

#[derive(Deserialize, Default)]
struct EngineErrorBody {
    group: Option<String>,
    code: Option<String>,
    message: Option<String>,
}

#[derive(Deserialize)]
struct NamespacesPage {
    #[serde(default)]
    namespaces: Vec<NamespaceRow>,
    #[serde(default)]
    pagination: Option<Pagination>,
}

#[derive(Deserialize)]
struct NamespaceRow {
    name: String,
}

#[derive(Deserialize)]
struct Pagination {
    cursor: Option<String>,
}

/// JS `encodeURIComponent`.
pub fn encode_uri_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            );
        if keep {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn status_text(status: reqwest::StatusCode) -> &'static str {
    status.canonical_reason().unwrap_or("")
}

/// TS `readEngineError`.
async fn read_engine_error(res: reqwest::Response) -> String {
    let status = res.status();
    match res.json::<EngineErrorBody>().await {
        Ok(body) => format!(
            "{} {}.{}: {}",
            status.as_u16(),
            body.group.as_deref().unwrap_or("api"),
            body.code.as_deref().unwrap_or("error"),
            body.message.as_deref().unwrap_or(status_text(status)),
        ),
        Err(_) => format!("{} {}", status.as_u16(), status_text(status)),
    }
}

fn transport(err: &reqwest::Error) -> AttemptError {
    AttemptError::Transient(crate::gateway::redact_error(err))
}

fn is_auth(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN
}

/// The deduplicated, trimmed, non-empty pool list.
fn runner_pools(opts: &EnsureRivetNamespaceOptions) -> Vec<String> {
    let mut pools: Vec<String> = Vec::new();
    for pool in std::iter::once(&opts.runner_pool).chain(&opts.additional_runner_pools) {
        let pool = pool.trim();
        if !pool.is_empty() && !pools.iter().any(|p| p == pool) {
            pools.push(pool.to_owned());
        }
    }
    pools
}

async fn ensure_once(
    http: &reqwest::Client,
    opts: &EnsureRivetNamespaceOptions,
) -> Result<bool, AttemptError> {
    let base = opts.endpoint.trim_end_matches('/');
    let token = opts.token.as_str();

    // Namespace: list, create only if missing. Follow the cursor so a name on page 2
    // is never "re-created".
    let mut found = false;
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let url = match &cursor {
            Some(c) => format!("{base}/namespaces?cursor={}", encode_uri_component(c)),
            None => format!("{base}/namespaces"),
        };
        let res = http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| transport(&e))?;
        if is_auth(res.status()) {
            return Err(AttemptError::Fatal(format!(
                "engine admin auth rejected the token on GET /namespaces ({}) — check RIVET_TOKEN matches the engine's RIVET__AUTH__ADMIN_TOKEN",
                read_engine_error(res).await
            )));
        }
        if !res.status().is_success() {
            return Err(AttemptError::Transient(format!(
                "GET /namespaces failed: {}",
                read_engine_error(res).await
            )));
        }
        let page: NamespacesPage = res
            .json()
            .await
            .map_err(|e| AttemptError::Transient(crate::gateway::redact_error(&e)))?;
        found = page.namespaces.iter().any(|ns| ns.name == opts.namespace);
        // TS `if (!cursor) break`: an empty cursor is the last page too.
        cursor = page
            .pagination
            .and_then(|p| p.cursor)
            .filter(|c| !c.is_empty());
        if found || cursor.is_none() {
            break;
        }
    }

    let mut namespace_created = false;
    if !found {
        let body = match opts.dialect {
            AdminDialect::V2 => json!({ "name": opts.namespace, "display_name": opts.namespace }),
            AdminDialect::V3 => json!({ "name": opts.namespace }),
        };
        let res = http
            .post(format!("{base}/namespaces"))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| transport(&e))?;
        let status = res.status();
        if status.is_success() {
            namespace_created = true;
        } else {
            let body = res.json::<EngineErrorBody>().await.unwrap_or_default();
            // A concurrent replica (or a list/create race) made it: all we needed.
            if body.code.as_deref() != Some("name_not_unique") {
                if is_auth(status) {
                    return Err(AttemptError::Fatal(format!(
                        "engine admin auth rejected the token on POST /namespaces ({}) — check RIVET_TOKEN",
                        status.as_u16()
                    )));
                }
                return Err(AttemptError::Transient(format!(
                    "POST /namespaces failed: {} {}",
                    status.as_u16(),
                    body.message.as_deref().unwrap_or(status_text(status))
                )));
            }
        }
    }

    for pool in runner_pools(opts) {
        let (route, body) = match opts.dialect {
            AdminDialect::V2 => (
                "runner-configs",
                json!({ "datacenters": { "default": { "normal": envoy_runner_pool_config() } } }),
            ),
            AdminDialect::V3 => ("pools", envoy_runner_pool_config()),
        };
        let res = http
            .put(format!(
                "{base}/{route}/{}?namespace={}",
                encode_uri_component(&pool),
                encode_uri_component(&opts.namespace)
            ))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| transport(&e))?;
        if is_auth(res.status()) {
            return Err(AttemptError::Fatal(format!(
                "engine admin auth rejected the token on PUT /{route}/{pool} ({}) — check RIVET_TOKEN",
                read_engine_error(res).await
            )));
        }
        if !res.status().is_success() {
            return Err(AttemptError::Transient(format!(
                "PUT /{route}/{pool} failed: {}",
                read_engine_error(res).await
            )));
        }
    }

    Ok(namespace_created)
}

/// Ensure the namespace and runner pools exist on the self-hosted engine, retrying
/// transient failures until `timeout`. Call it after the HTTP server is listening
/// (so `/health` answers during an engine cold start) and before the envoy runner
/// starts.
pub async fn ensure_rivet_namespace(
    http: &reqwest::Client,
    opts: &EnsureRivetNamespaceOptions,
) -> Result<EnsureRivetNamespaceResult, RivetBootstrapError> {
    let deadline = Instant::now() + opts.timeout;
    let mut attempts: u32 = 0;
    let mut last_error: Option<String> = None;
    while Instant::now() < deadline {
        attempts += 1;
        // The deadline bounds each attempt too: an engine that accepts the
        // connection and never answers cannot stall boot past it.
        let remaining = deadline.saturating_duration_since(Instant::now());
        let attempt = tokio::time::timeout(remaining, ensure_once(http, opts))
            .await
            .unwrap_or_else(|_| Err(AttemptError::Transient("timed out".to_owned())));
        match attempt {
            Ok(namespace_created) => {
                return Ok(EnsureRivetNamespaceResult {
                    namespace_created,
                    runner_config_ensured: true,
                    attempts,
                });
            }
            Err(AttemptError::Fatal(message)) => return Err(RivetBootstrapError::Fatal(message)),
            Err(AttemptError::Transient(message)) => {
                last_error = Some(message);
                tokio::time::sleep(opts.retry_delay).await;
            }
        }
    }
    Err(RivetBootstrapError::Timeout(format!(
        "rivet bootstrap: engine at {} did not accept namespace/runner-config provisioning within {}ms ({attempts} attempts): {}",
        opts.endpoint,
        opts.timeout.as_millis(),
        last_error.as_deref().unwrap_or("unknown")
    )))
}

/// The Rivet env vars [`assert_rivet_env_safety`] checks. Empty strings count as
/// unset, as JS truthiness did.
#[derive(Debug, Clone, Copy, Default)]
pub struct RivetEnv<'a> {
    /// `RIVET_ENDPOINT`.
    pub endpoint: Option<&'a str>,
    /// `RIVET_PUBLIC_ENDPOINT`.
    pub public_endpoint: Option<&'a str>,
    /// `RIVET_TOKEN`.
    pub token: Option<&'a str>,
    /// `RIVET_NAMESPACE`.
    pub namespace: Option<&'a str>,
    /// `RIVET_ENGINE_SELFHOSTED=true`.
    pub selfhosted: bool,
}

/// The env containment error (TS threw a plain `Error`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RivetEnvError(pub String);

fn set(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

fn has_userinfo(url: &url::Url) -> bool {
    !url.username().is_empty() || url.password().is_some_and(|p| !p.is_empty())
}

/// TS `assertRivetEnvSafety`.
///
/// - Always: the `RIVET_TOKEN` value must not appear inside `RIVET_PUBLIC_ENDPOINT`
///   (that string is shipped to every browser).
/// - Self-hosted only: `RIVET_TOKEN`, `RIVET_NAMESPACE` and `RIVET_ENDPOINT` are
///   required, `RIVET_ENDPOINT` is bare, and `RIVET_PUBLIC_ENDPOINT` is bare unless it
///   still points at a `rivet.dev` host.
pub fn assert_rivet_env_safety(env: RivetEnv<'_>) -> Result<(), RivetEnvError> {
    if let (Some(token), Some(public)) = (set(env.token), set(env.public_endpoint))
        && public.contains(token)
    {
        {
            return Err(RivetEnvError(
                "api boot: RIVET_PUBLIC_ENDPOINT contains the RIVET_TOKEN value — this string is shipped verbatim to every browser via /api/client-config; refusing to start".to_owned(),
            ));
        }
    }

    if !env.selfhosted {
        return Ok(());
    }

    let mut missing: Vec<&str> = Vec::new();
    if set(env.token).is_none() {
        missing.push("RIVET_TOKEN");
    }
    if set(env.namespace).is_none() {
        missing.push("RIVET_NAMESPACE");
    }
    if set(env.endpoint).is_none() {
        missing.push("RIVET_ENDPOINT");
    }
    if !missing.is_empty() {
        return Err(RivetEnvError(format!(
            "api boot: RIVET_ENGINE_SELFHOSTED=true but {} {} unset — the self-hosted engine requires the admin token, an explicit namespace, and the private engine URL",
            missing.join(", "),
            if missing.len() == 1 { "is" } else { "are" }
        )));
    }

    let endpoint = set(env.endpoint).unwrap_or_default();
    let endpoint = url::Url::parse(endpoint)
        .map_err(|_| RivetEnvError(format!("Invalid URL: {}", redact_userinfo(endpoint))))?;
    if has_userinfo(&endpoint) {
        return Err(RivetEnvError(
            "api boot: RIVET_ENGINE_SELFHOSTED=true but RIVET_ENDPOINT embeds URL userinfo — keep the endpoint bare and pass the admin token via RIVET_TOKEN only (the endpoint-parser rejects credentials in both places, and bare endpoints keep logs token-free)".to_owned(),
        ));
    }

    if let Some(public) = set(env.public_endpoint) {
        let public = url::Url::parse(public)
            .map_err(|_| RivetEnvError(format!("Invalid URL: {}", redact_userinfo(public))))?;
        let host = public.host_str().unwrap_or_default();
        let is_cloud_host = host == "api.rivet.dev" || host.ends_with(".rivet.dev");
        if has_userinfo(&public) && !is_cloud_host {
            return Err(RivetEnvError(
                "api boot: RIVET_ENGINE_SELFHOSTED=true but RIVET_PUBLIC_ENDPOINT embeds URL userinfo on a non-Rivet-Cloud host — the self-hosted public path (https://<api-origin>/rivet) must be bare; userinfo is only tolerated while the endpoint still points at Rivet Cloud during the Phase 2 transition".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Drop anything before an `@` so an unparseable URL never echoes a credential.
fn redact_userinfo(raw: &str) -> String {
    match raw.rsplit_once('@') {
        Some((_, rest)) => format!("<redacted>@{rest}"),
        None => raw.to_owned(),
    }
}

#[cfg(test)]
mod tests;
