//! D3 Option A: actor-scoped engine gateway tokens, minted through the engine admin
//! API (`POST /auth/tokens`). See `docs/port/requests/done/gateway-token.md` for the
//! probe that chose it.
//!
//! The token is the TS `table.issueToken({subject, expiresIn: 900})` equivalent: it
//! grants gateway access to one actor and is refused by the admin API. The admin
//! token and the issued token never reach a log line.

use std::time::Duration;

use euchre_server_core::config::Secret;
use serde::Deserialize;
use serde_json::json;

/// Seconds an issued token lives (TS `expiresIn: 900`).
pub const GATEWAY_TOKEN_TTL_SECS: u64 = 900;

/// An issued gateway token.
#[derive(Clone, PartialEq, Eq)]
pub struct IssuedToken {
    /// The engine JWT (`typ: rivet-auth+jwt`).
    pub token: String,
    /// Unix milliseconds.
    pub issued_at_ms: u64,
    /// Unix milliseconds.
    pub expires_at_ms: u64,
}

impl std::fmt::Debug for IssuedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuedToken")
            .field("token", &"<redacted>")
            .field("issued_at_ms", &self.issued_at_ms)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// Why an issuance failed. Never carries a credential.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IssueError {
    /// The engine could not be reached or did not answer in time.
    #[error("engine unreachable: {0}")]
    Transport(String),
    /// The engine answered with a non-2xx status.
    #[error("engine refused the token request: {0}")]
    Status(u16),
    /// The engine answered 2xx with an unexpected body.
    #[error("engine sent an unreadable token response")]
    Body,
}

#[derive(Deserialize)]
struct TokenResponse {
    token: String,
    #[serde(default)]
    issued_ts: Option<u64>,
    expires_ts: u64,
}

/// Mints gateway tokens with the engine admin token.
#[derive(Clone)]
pub struct GatewayTokenIssuer {
    http: reqwest::Client,
    endpoint: String,
    namespace: String,
    admin_token: Secret,
    timeout: Duration,
}

impl std::fmt::Debug for GatewayTokenIssuer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayTokenIssuer")
            .field("endpoint", &self.endpoint)
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

/// Inputs of [`GatewayTokenIssuer::new`].
#[derive(Debug, Clone)]
pub struct GatewayTokenIssuerOptions {
    /// Private engine URL.
    pub endpoint: String,
    /// Engine namespace.
    pub namespace: String,
    /// Engine admin token (`RIVET_TOKEN`).
    pub admin_token: Secret,
    /// Bound on one issuance.
    pub timeout: Duration,
}

impl GatewayTokenIssuer {
    /// An issuer that sends with `http`.
    pub fn new(opts: GatewayTokenIssuerOptions, http: reqwest::Client) -> Self {
        Self {
            http,
            endpoint: opts.endpoint.trim_end_matches('/').to_owned(),
            namespace: opts.namespace,
            admin_token: opts.admin_token,
            timeout: opts.timeout,
        }
    }

    /// Mint a token for `subject` that opens only `actor_id` through the gateway.
    pub async fn issue(&self, subject: &str, actor_id: &str) -> Result<IssuedToken, IssueError> {
        let body = json!({
            "namespace": self.namespace,
            "subject": subject,
            "duration": GATEWAY_TOKEN_TTL_SECS,
            "grants": [{
                "resource": "actor_gateway",
                "target": { "id": actor_id },
                "operations": ["read"],
            }],
        });
        let res = self
            .http
            .post(format!("{}/auth/tokens", self.endpoint))
            .bearer_auth(self.admin_token.expose())
            .json(&body)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| IssueError::Transport(crate::gateway::redact_error(&e)))?;
        if !res.status().is_success() {
            return Err(IssueError::Status(res.status().as_u16()));
        }
        let parsed: TokenResponse = res.json().await.map_err(|_| IssueError::Body)?;
        if parsed.token.is_empty() {
            return Err(IssueError::Body);
        }
        Ok(IssuedToken {
            token: parsed.token,
            issued_at_ms: parsed.issued_ts.unwrap_or_else(|| {
                parsed
                    .expires_ts
                    .saturating_sub(GATEWAY_TOKEN_TTL_SECS * 1000)
            }),
            expires_at_ms: parsed.expires_ts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn issuer(server: &MockServer) -> GatewayTokenIssuer {
        GatewayTokenIssuer::new(
            GatewayTokenIssuerOptions {
                endpoint: format!("{}/", server.uri()),
                namespace: "euchre".into(),
                admin_token: Secret::new("admin-secret"),
                timeout: Duration::from_secs(5),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn sends_the_probed_request_and_reads_ms_timestamps() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/tokens"))
            .and(header("authorization", "Bearer admin-secret"))
            .and(body_json(json!({
                "namespace": "euchre",
                "subject": "guest-1",
                "duration": 900,
                "grants": [{"resource": "actor_gateway", "target": {"id": "actor-1"}, "operations": ["read"]}]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "expires_ts": 1_900_000, "issued_ts": 1_000_000, "token": "jwt"
            })))
            .mount(&server)
            .await;
        let issued = issuer(&server).issue("guest-1", "actor-1").await?;
        assert_eq!(issued.token, "jwt");
        assert_eq!(issued.issued_at_ms, 1_000_000);
        assert_eq!(issued.expires_at_ms, 1_900_000);
        assert!(!format!("{issued:?}").contains("jwt"));
        Ok(())
    }

    #[tokio::test]
    async fn refusals_and_bad_bodies_are_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(403))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;
        let issuer = issuer(&server);
        assert_eq!(issuer.issue("g", "a").await, Err(IssueError::Status(403)));
        assert_eq!(issuer.issue("g", "a").await, Err(IssueError::Body));
    }
}
