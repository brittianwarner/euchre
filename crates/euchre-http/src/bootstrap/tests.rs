//! Port of `server/bootstrap-rivet-namespace.test.ts`, with the engine mocked by
//! wiremock (the TS `fetchImpl` seam), plus the 3.0 dialect.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::*;

fn opts(server: &MockServer) -> EnsureRivetNamespaceOptions {
    EnsureRivetNamespaceOptions {
        retry_delay: Duration::from_millis(1),
        dialect: AdminDialect::V2,
        ..EnsureRivetNamespaceOptions::new(server.uri(), "admin-token", "layerr-dev", "default")
    }
}

fn json_response(body: Value, status: u16) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(body)
}

async fn mount(server: &MockServer, verb: &str, route: &str, res: ResponseTemplate) {
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(res)
        .mount(server)
        .await;
}

fn calls(requests: &[Request]) -> Vec<String> {
    requests
        .iter()
        .map(|r| format!("{} {}", r.method, r.url.path()))
        .collect()
}

async fn requests(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap_or_default()
}

/// A responder that answers in sequence, repeating the last answer.
struct Sequence {
    count: Arc<AtomicUsize>,
    answers: Vec<ResponseTemplate>,
}

impl Respond for Sequence {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let n = self.count.fetch_add(1, Ordering::SeqCst);
        let i = n.min(self.answers.len() - 1);
        self.answers[i].clone()
    }
}

#[tokio::test]
async fn namespace_exists_skips_post_asserts_runner_config() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/namespaces",
        json_response(
            json!({"namespaces": [{"name": "layerr-dev"}], "pagination": {"cursor": null}}),
            200,
        ),
    )
    .await;
    mount(
        &server,
        "PUT",
        "/runner-configs/default",
        json_response(json!({"endpoint_config_changed": true}), 200),
    )
    .await;
    let result = ensure_rivet_namespace(&reqwest::Client::new(), &opts(&server)).await?;
    assert!(!result.namespace_created);
    assert!(result.runner_config_ensured);
    assert_eq!(result.attempts, 1);
    let reqs = requests(&server).await;
    assert_eq!(
        calls(&reqs),
        vec!["GET /namespaces", "PUT /runner-configs/default"]
    );
    // The admin token rides as Bearer, never in the URL.
    for r in &reqs {
        assert!(!r.url.as_str().contains("admin-token"));
        assert_eq!(
            r.headers.get("authorization").and_then(|v| v.to_str().ok()),
            Some("Bearer admin-token")
        );
    }
    let put = reqs
        .iter()
        .find(|r| r.method.as_str() == "PUT")
        .ok_or_else(|| anyhow::anyhow!("no PUT"))?;
    assert_eq!(put.url.query(), Some("namespace=layerr-dev"));
    assert_eq!(
        serde_json::from_slice::<Value>(&put.body)?,
        json!({"datacenters": {"default": {"normal": envoy_runner_pool_config()}}})
    );
    let cfg = envoy_runner_pool_config();
    assert_eq!(cfg["drain_on_version_upgrade"], json!(true));
    assert_eq!(cfg["actor_eviction_period"], json!(60));
    assert_eq!(cfg["actor_eviction_rate"], json!(1));
    Ok(())
}

#[tokio::test]
async fn asserts_the_fixed_services_pool_when_durable_streams_is_enabled() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/namespaces",
        json_response(
            json!({"namespaces": [{"name": "layerr-dev"}], "pagination": {"cursor": null}}),
            200,
        ),
    )
    .await;
    for pool in ["default", "services"] {
        mount(
            &server,
            "PUT",
            &format!("/runner-configs/{pool}"),
            json_response(json!({"endpoint_config_changed": true}), 200),
        )
        .await;
    }
    let mut o = opts(&server);
    o.additional_runner_pools = vec!["services".into(), "default".into(), " services ".into()];
    let result = ensure_rivet_namespace(&reqwest::Client::new(), &o).await?;
    assert!(result.runner_config_ensured);
    let puts: Vec<String> = requests(&server)
        .await
        .iter()
        .filter(|r| r.method.as_str() == "PUT")
        .map(|r| r.url.path().to_owned())
        .collect();
    assert_eq!(
        puts,
        vec!["/runner-configs/default", "/runner-configs/services"]
    );
    Ok(())
}

#[tokio::test]
async fn namespace_missing_posts_create_then_runner_config() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/namespaces",
        json_response(
            json!({"namespaces": [{"name": "default"}], "pagination": {"cursor": null}}),
            200,
        ),
    )
    .await;
    mount(
        &server,
        "POST",
        "/namespaces",
        json_response(json!({"namespace": {"name": "layerr-dev"}}), 200),
    )
    .await;
    mount(
        &server,
        "PUT",
        "/runner-configs/default",
        json_response(json!({"endpoint_config_changed": true}), 200),
    )
    .await;
    let result = ensure_rivet_namespace(&reqwest::Client::new(), &opts(&server)).await?;
    assert!(result.namespace_created);
    let reqs = requests(&server).await;
    let post = reqs
        .iter()
        .find(|r| r.method.as_str() == "POST")
        .ok_or_else(|| anyhow::anyhow!("no POST"))?;
    assert_eq!(
        serde_json::from_slice::<Value>(&post.body)?,
        json!({"name": "layerr-dev", "display_name": "layerr-dev"})
    );
    Ok(())
}

#[tokio::test]
async fn post_name_not_unique_race_is_treated_as_success() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/namespaces",
        json_response(
            json!({"namespaces": [], "pagination": {"cursor": null}}),
            200,
        ),
    )
    .await;
    mount(
        &server,
        "POST",
        "/namespaces",
        json_response(
            json!({"group": "namespace", "code": "name_not_unique", "message": "Namespace name must be unique."}),
            400,
        ),
    )
    .await;
    mount(
        &server,
        "PUT",
        "/runner-configs/default",
        json_response(json!({"endpoint_config_changed": true}), 200),
    )
    .await;
    let result = ensure_rivet_namespace(&reqwest::Client::new(), &opts(&server)).await?;
    assert!(result.runner_config_ensured);
    assert!(!result.namespace_created);
    Ok(())
}

#[tokio::test]
async fn follows_namespace_pagination_cursor() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path("/namespaces"))
        .respond_with(Sequence {
            count: Arc::clone(&count),
            answers: vec![
                json_response(
                    json!({"namespaces": [{"name": "default"}], "pagination": {"cursor": "a b/c"}}),
                    200,
                ),
                json_response(
                    json!({"namespaces": [{"name": "layerr-dev"}], "pagination": {"cursor": null}}),
                    200,
                ),
            ],
        })
        .mount(&server)
        .await;
    mount(
        &server,
        "PUT",
        "/runner-configs/default",
        json_response(json!({"endpoint_config_changed": true}), 200),
    )
    .await;
    let result = ensure_rivet_namespace(&reqwest::Client::new(), &opts(&server)).await?;
    assert!(!result.namespace_created);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let reqs = requests(&server).await;
    assert_eq!(reqs[1].url.query(), Some("cursor=a%20b%2Fc"));
    Ok(())
}

#[tokio::test]
async fn status_403_fails_fatally_without_retrying() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/namespaces",
        json_response(
            json!({"group": "api", "code": "forbidden", "message": "Access denied"}),
            403,
        ),
    )
    .await;
    let err = ensure_rivet_namespace(&reqwest::Client::new(), &opts(&server)).await;
    assert_eq!(
        err,
        Err(RivetBootstrapError::Fatal(
            "engine admin auth rejected the token on GET /namespaces (403 api.forbidden: Access denied) — check RIVET_TOKEN matches the engine's RIVET__AUTH__ADMIN_TOKEN".to_owned()
        ))
    );
    assert_eq!(requests(&server).await.len(), 1);
}

#[tokio::test]
async fn transient_5xx_retries_until_success() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path("/namespaces"))
        .respond_with(Sequence {
            count: Arc::clone(&count),
            answers: vec![
                json_response(json!({"message": "booting"}), 503),
                json_response(json!({"message": "booting"}), 503),
                json_response(
                    json!({"namespaces": [{"name": "layerr-dev"}], "pagination": {"cursor": null}}),
                    200,
                ),
            ],
        })
        .mount(&server)
        .await;
    mount(
        &server,
        "PUT",
        "/runner-configs/default",
        json_response(json!({"endpoint_config_changed": true}), 200),
    )
    .await;
    let result = ensure_rivet_namespace(&reqwest::Client::new(), &opts(&server)).await?;
    assert_eq!(result.attempts, 3);
    Ok(())
}

#[tokio::test]
async fn deadline_exhaustion_fails_with_attempt_count() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/namespaces",
        json_response(json!({"message": "down"}), 502),
    )
    .await;
    let mut o = opts(&server);
    o.timeout = Duration::from_millis(5);
    let err = ensure_rivet_namespace(&reqwest::Client::new(), &o)
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("expected an error"))?;
    let RivetBootstrapError::Timeout(message) = err else {
        anyhow::bail!("expected a timeout, got {err:?}");
    };
    let re = regex::Regex::new(r"did not accept .* within 5ms")?;
    assert!(re.is_match(&message), "{message}");
    assert!(
        message.contains("GET /namespaces failed: 502 api.error: down"),
        "{message}"
    );
    Ok(())
}

#[tokio::test]
async fn v3_dialect_posts_name_only_and_puts_flat_pool_config() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/namespaces",
        json_response(
            json!({"namespaces": [], "pagination": {"cursor": null}}),
            200,
        ),
    )
    .await;
    mount(
        &server,
        "POST",
        "/namespaces",
        json_response(json!({"namespace": {"name": "layerr-dev"}}), 200),
    )
    .await;
    mount(
        &server,
        "PUT",
        "/pools/default",
        json_response(json!({"pool": {}}), 200),
    )
    .await;
    let mut o = opts(&server);
    o.dialect = AdminDialect::V3;
    let result = ensure_rivet_namespace(&reqwest::Client::new(), &o).await?;
    assert!(result.namespace_created);
    let reqs = requests(&server).await;
    assert_eq!(
        calls(&reqs),
        vec!["GET /namespaces", "POST /namespaces", "PUT /pools/default"]
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&reqs[1].body)?,
        json!({"name": "layerr-dev"})
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&reqs[2].body)?,
        envoy_runner_pool_config()
    );
    Ok(())
}

#[tokio::test]
async fn v3_pool_auth_failure_names_the_route() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/namespaces",
        json_response(
            json!({"namespaces": [{"name": "layerr-dev"}], "pagination": {"cursor": null}}),
            200,
        ),
    )
    .await;
    mount(
        &server,
        "PUT",
        "/pools/default",
        ResponseTemplate::new(401).set_body_string("nope"),
    )
    .await;
    let mut o = opts(&server);
    o.dialect = AdminDialect::V3;
    let err = ensure_rivet_namespace(&reqwest::Client::new(), &o).await;
    assert_eq!(
        err,
        Err(RivetBootstrapError::Fatal(
            "engine admin auth rejected the token on PUT /pools/default (401 Unauthorized) — check RIVET_TOKEN".to_owned()
        ))
    );
}

#[test]
fn encode_uri_component_matches_js() {
    assert_eq!(encode_uri_component("a b/c?d=é"), "a%20b%2Fc%3Fd%3D%C3%A9");
    assert_eq!(encode_uri_component("A-z_0.9!~*'()"), "A-z_0.9!~*'()");
}

// ---- assertRivetEnvSafety -------------------------------------------------------

fn env<'a>() -> RivetEnv<'a> {
    RivetEnv::default()
}

#[test]
fn cloud_mode_allows_userinfo_endpoints() {
    assert!(
        assert_rivet_env_safety(RivetEnv {
            endpoint: Some("https://ns:sk_secret@api.rivet.dev"),
            public_endpoint: Some("https://ns:pk_public@api.rivet.dev"),
            ..env()
        })
        .is_ok()
    );
}

#[test]
fn always_on_token_inside_public_endpoint_fails() {
    let err = assert_rivet_env_safety(RivetEnv {
        token: Some("super-secret"),
        public_endpoint: Some("https://ns:super-secret@api.rivet.dev"),
        ..env()
    });
    assert!(err.is_err_and(|e| {
        e.0.contains("RIVET_PUBLIC_ENDPOINT contains the RIVET_TOKEN value")
    }));
}

#[test]
fn selfhost_missing_token_namespace_endpoint_names_all_three() {
    let err = assert_rivet_env_safety(RivetEnv {
        selfhosted: true,
        ..env()
    });
    assert_eq!(
        err,
        Err(RivetEnvError(
            "api boot: RIVET_ENGINE_SELFHOSTED=true but RIVET_TOKEN, RIVET_NAMESPACE, RIVET_ENDPOINT are unset — the self-hosted engine requires the admin token, an explicit namespace, and the private engine URL".to_owned()
        ))
    );
    let one = assert_rivet_env_safety(RivetEnv {
        selfhosted: true,
        token: Some("admin"),
        namespace: Some("default"),
        ..env()
    });
    assert!(one.is_err_and(|e| e.0.contains("but RIVET_ENDPOINT is unset")));
}

#[test]
fn selfhost_endpoint_userinfo_fails() {
    let err = assert_rivet_env_safety(RivetEnv {
        endpoint: Some("http://default:admin@engine:6420"),
        token: Some("admin"),
        namespace: Some("default"),
        selfhosted: true,
        ..env()
    });
    assert!(err.is_err_and(|e| e.0.contains("RIVET_ENDPOINT embeds URL userinfo")));
}

#[test]
fn selfhost_public_endpoint_userinfo_fails_on_non_cloud_hosts() {
    let err = assert_rivet_env_safety(RivetEnv {
        endpoint: Some("http://engine:6420"),
        public_endpoint: Some("https://u:p@api.layerr.ai/rivet"),
        token: Some("admin"),
        namespace: Some("default"),
        selfhosted: true,
    });
    assert!(err.is_err_and(|e| e.0.contains("RIVET_PUBLIC_ENDPOINT embeds URL userinfo")));
}

#[test]
fn selfhost_cloud_userinfo_public_endpoint_allowed() {
    for public in [
        "https://my-namespace:pk_public@api.rivet.dev",
        "https://my-namespace:pk_public@api-us-east-1.rivet.dev",
    ] {
        assert!(
            assert_rivet_env_safety(RivetEnv {
                endpoint: Some("http://engine:6420"),
                public_endpoint: Some(public),
                token: Some("admin"),
                namespace: Some("default"),
                selfhosted: true,
            })
            .is_ok(),
            "{public}"
        );
    }
}

#[test]
fn selfhost_clean_config_passes() {
    assert!(
        assert_rivet_env_safety(RivetEnv {
            endpoint: Some("http://rivet-engine.railway.internal:6420"),
            public_endpoint: Some("https://api.layerr.ai/rivet"),
            token: Some("admin"),
            namespace: Some("default"),
            selfhosted: true,
        })
        .is_ok()
    );
}

#[tokio::test]
async fn a_hanging_engine_cannot_outlast_the_deadline() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/namespaces",
        json_response(json!({"namespaces": []}), 200).set_delay(Duration::from_secs(30)),
    )
    .await;
    let mut o = opts(&server);
    o.timeout = Duration::from_millis(300);
    let started = std::time::Instant::now();
    let err = ensure_rivet_namespace(&reqwest::Client::new(), &o)
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("expected an error"))?;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let RivetBootstrapError::Timeout(message) = err else {
        anyhow::bail!("expected a timeout, got {err:?}");
    };
    assert!(message.contains("within 300ms"), "{message}");
    assert!(message.contains(": timed out"), "{message}");
    Ok(())
}

#[tokio::test]
async fn an_empty_cursor_is_the_last_page() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/namespaces",
        json_response(
            json!({"namespaces": [{"name": "other"}], "pagination": {"cursor": ""}}),
            200,
        ),
    )
    .await;
    mount(
        &server,
        "POST",
        "/namespaces",
        json_response(json!({"namespace": {"name": "layerr-dev"}}), 200),
    )
    .await;
    mount(
        &server,
        "PUT",
        "/runner-configs/default",
        json_response(json!({}), 200),
    )
    .await;
    let result = ensure_rivet_namespace(&reqwest::Client::new(), &opts(&server)).await?;
    assert!(result.namespace_created);
    assert_eq!(
        calls(&requests(&server).await),
        vec![
            "GET /namespaces",
            "POST /namespaces",
            "PUT /runner-configs/default"
        ]
    );
    Ok(())
}
