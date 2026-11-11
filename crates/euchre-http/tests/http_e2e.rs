//! Engine end-to-end tests for the router (EUCHRE_E2E=1 only).
//!
//! A real rivet-engine (through `TestEngine`) serves stand-in `euchreTable` and
//! `playerProfile` actors. The router runs on a loopback port with the engine admin
//! token, so `POST /api/table-session/{gameId}` mints real gateway tokens (D3 Option
//! A). A rivetkit client then reaches the table through `/rivet/**` with that token.

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use euchre_http::{
    AdminDialect, ApiConfig, ApiState, CallTimeouts, EnsureRivetNamespaceOptions, GatewayOptions,
    Health, RateLimitConfig, ensure_rivet_namespace, router,
};
use euchre_server_core::auth::{AuthKeys, Authenticator, GuestAudience};
use euchre_server_core::config::{OriginPolicy, Secret};
use euchre_server_core::rivet::user_error;
use euchre_server_core::testing::{TestEngine, TestEngineOptions, e2e_enabled, e2e_skip};
use rivetkit::Action;
use rivetkit::client::{Client, ClientConfig, EncodingKind, GetOptions};
use rivetkit::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ---- Stand-in actors ---------------------------------------------------------------

fn auth() -> &'static Authenticator {
    static AUTH: OnceLock<Authenticator> = OnceLock::new();
    AUTH.get_or_init(|| {
        Authenticator::new(AuthKeys {
            guest: Secret::new("g".repeat(40)),
            profile: Secret::new("p".repeat(40)),
            internal: Secret::new("i".repeat(40)),
            identity: Secret::new("d".repeat(40)),
            jwt_issuer: None,
            allow_unsigned_email: false,
        })
    })
}

#[derive(Serialize, Deserialize, Debug)]
struct Snapshot;
impl Action for Snapshot {
    type Output = Value;
    const NAME: &'static str = "snapshot";
}

/// Stand-in `euchreTable`: remembers its owner and answers `snapshot`, refusing a
/// connection token that names another guest (as the real table does).
struct Table {
    owner: Option<String>,
}

#[async_trait]
impl Actor for Table {
    type Input = Value;
    type ConnParams = Value;
    type Action = rivetkit::Raw;
    type Actions = (Snapshot,);
    type Events = ();

    async fn create(ctx: &Ctx<Self>) -> Result<Self> {
        let input = ctx.input()?;
        Ok(Table {
            owner: input
                .get("ownerUserId")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
}

impl Handler<Snapshot> for Table {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, _a: Snapshot) -> Result<Value> {
        let token = ctx
            .conn()
            .and_then(|c| c.params().ok())
            .and_then(|p| p.get("token").and_then(Value::as_str).map(str::to_owned));
        if let (Some(token), Some(owner)) = (token, self.owner.as_deref()) {
            let user = auth()
                .verify_guest(&token, GuestAudience::Actors)
                .map_err(|_| user_error("forbidden", "Not your table."))?;
            if user != owner {
                return Err(user_error("forbidden", "Not your table."));
            }
        }
        Ok(json!({
            "status": "playing",
            "score": [0, 0],
            "handNo": 1,
            "actor": ctx.actor_id(),
        }))
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct GetStats;
impl Action for GetStats {
    type Output = Value;
    const NAME: &'static str = "getStats";
}

/// Stand-in `playerProfile`.
struct Profile;

#[async_trait]
impl Actor for Profile {
    type Input = Value;
    type ConnParams = Value;
    type Action = rivetkit::Raw;
    type Actions = (GetStats,);
    type Events = ();

    async fn create(_ctx: &Ctx<Self>) -> Result<Self> {
        Ok(Profile)
    }
}

impl Handler<GetStats> for Profile {
    async fn handle(self: Arc<Self>, ctx: Ctx<Self>, _a: GetStats) -> Result<Value> {
        let token = ctx
            .conn()
            .and_then(|c| c.params().ok())
            .and_then(|p| p.get("token").and_then(Value::as_str).map(str::to_owned));
        if let Some(token) = token {
            auth()
                .verify_profile_token(&token)
                .map_err(|_| user_error("unauthorized", "Bad profile token."))?;
        }
        Ok(json!({ "gamesPlayed": 3 }))
    }
}

// ---- Harness -----------------------------------------------------------------------

struct App {
    base: String,
    origin: String,
    http: reqwest::Client,
}

/// One raw HTTP/1.1 request to `base` (`http://host:port`), so no client library
/// normalizes the path. Returns the status and the body.
async fn raw_request(
    base: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> Result<(u16, String)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let host = base.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(host).await?;
    let mut req = format!("{method} {path} HTTP/1.1\r\nhost: {host}\r\nconnection: close\r\n");
    for (name, value) in headers {
        req.push_str(&format!("{name}: {value}\r\n"));
    }
    req.push_str("content-length: 0\r\n\r\n");
    stream.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut buf)).await??;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .context("no status line")?
        .parse()?;
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_owned())
        .unwrap_or_default();
    Ok((status, body))
}

async fn start_app(engine: &TestEngine) -> Result<App> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let origin = format!("http://127.0.0.1:{}", addr.port());
    let config = ApiConfig {
        origins: OriginPolicy::strict(vec![origin.clone()])?,
        engine_endpoint: engine.endpoint().to_owned(),
        namespace: engine.namespace().to_owned(),
        pool_name: engine.pool_name().to_owned(),
        engine_token: Some(engine.token().clone()),
        secure_cookies: false,
        static_dir: None,
        downloads_dir: None,
        downloads_bucket: None,
        trust_forwarded_for: false,
        trusted_proxies: Vec::new(),
        rate_limits: RateLimitConfig::unlimited(),
        gateway: GatewayOptions::default(),
        timeouts: CallTimeouts::default(),
    };
    let health = Health::new();
    health.set_ready();
    let state = ApiState::new(config, auth().clone(), engine.client(), health)?;
    let app = router(state);
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    Ok(App {
        base: origin.clone(),
        origin,
        http: reqwest::Client::builder().no_proxy().build()?,
    })
}

impl App {
    async fn post(&self, path: &str, bearer: &str) -> Result<(u16, Value)> {
        let res = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(bearer)
            .send()
            .await?;
        let status = res.status().as_u16();
        let text = res.text().await?;
        Ok((
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        ))
    }

    async fn enroll(&self) -> Result<(String, String)> {
        let res = self
            .http
            .post(format!("{}/api/device/enroll", self.base))
            .header("origin", &self.origin)
            .send()
            .await?;
        if res.status() != 200 {
            bail!("enroll -> {}", res.status());
        }
        let body: Value = res.json().await?;
        Ok((
            body["userId"].as_str().context("userId")?.to_owned(),
            body["sessionToken"]
                .as_str()
                .context("sessionToken")?
                .to_owned(),
        ))
    }

    /// `POST /api/tables` until the runner serves the create.
    async fn create_table(&self, engine: &TestEngine, bearer: &str) -> Result<String> {
        engine
            .ready(|| async {
                let (status, body) = self.post("/api/tables", bearer).await?;
                if status != 201 {
                    bail!("create -> {status} {body}");
                }
                Ok(body["gameId"].as_str().context("gameId")?.to_owned())
            })
            .await
    }

    fn gateway_client(&self, ns: &str, token: Option<&str>) -> Client {
        let mut cfg = ClientConfig::new(format!("{}/rivet", self.base))
            .namespace(ns)
            .encoding(EncodingKind::Json)
            .disable_metadata_lookup(true);
        if let Some(token) = token {
            cfg = cfg.token(token);
        }
        Client::new(cfg)
    }
}

fn engine_client(engine: &TestEngine, token: Option<&str>) -> Client {
    let mut cfg = ClientConfig::new(engine.endpoint())
        .namespace(engine.namespace())
        .pool_name(engine.pool_name())
        .encoding(EncodingKind::Json)
        .disable_metadata_lookup(true);
    if let Some(token) = token {
        cfg = cfg.token(token);
    }
    Client::new(cfg)
}

async fn http_action(client: &Client, actor_id: &str, params: Value) -> Result<Value> {
    let handle = client.get_for_id(
        "euchreTable",
        actor_id,
        GetOptions {
            params: Some(params),
        },
    )?;
    tokio::time::timeout(Duration::from_secs(15), handle.action("snapshot", vec![]))
        .await
        .context("http action timed out")?
}

async fn ws_action(client: &Client, actor_id: &str, params: Value) -> Result<Value> {
    let handle = client.get_for_id(
        "euchreTable",
        actor_id,
        GetOptions {
            params: Some(params),
        },
    )?;
    let conn = handle.connect();
    let result =
        tokio::time::timeout(Duration::from_secs(15), conn.action("snapshot", vec![])).await;
    conn.disconnect().await;
    result.context("ws action timed out")?
}

fn engine_options() -> Result<TestEngineOptions> {
    TestEngineOptions::from_env(|r| {
        r.register_actor::<Table>("euchreTable");
        r.register_actor::<Profile>("playerProfile");
    })
}

// ---- Tests -------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn table_session_mints_a_gateway_token_the_gateway_and_engine_accept() -> Result<()> {
    if !e2e_enabled() {
        e2e_skip("table_session_mints_a_gateway_token_the_gateway_and_engine_accept");
        return Ok(());
    }
    let engine = TestEngine::start(engine_options()?).await?;
    let result = run_table_session(&engine).await;
    engine.shutdown().await?;
    result
}

async fn run_table_session(engine: &TestEngine) -> Result<()> {
    let app = start_app(engine).await?;
    let (user_a, bearer_a) = app.enroll().await?;
    let (_user_b, bearer_b) = app.enroll().await?;
    let game_a = app.create_table(engine, &bearer_a).await?;
    let game_b = app.create_table(engine, &bearer_b).await?;

    // The mint.
    let (status, session) = app
        .post(&format!("/api/table-session/{game_a}"), &bearer_a)
        .await?;
    assert_eq!(status, 200, "{session}");
    let actor_a = session["actorId"].as_str().context("actorId")?.to_owned();
    let actor_token = session["token"].as_str().context("token")?.to_owned();
    let gateway_token = session["gatewayToken"]
        .as_str()
        .context("gatewayToken")?
        .to_owned();
    let expires_at = session["expiresAt"].as_u64().context("expiresAt")?;
    assert_eq!(
        auth().verify_guest(&actor_token, GuestAudience::Actors)?,
        user_a
    );
    let now = u64::try_from(euchre_server_core::auth::now_ms())?;
    assert!(
        expires_at > now + 800_000 && expires_at <= now + 901_000,
        "{expires_at}"
    );
    let keys: Vec<&str> = session
        .as_object()
        .context("object")?
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["actorId", "token", "gatewayToken", "expiresAt"]);
    eprintln!("E2E table-session: actor {actor_a}, gateway token issued");

    // Another guest's table: the table refuses the actor token.
    let (status, body) = app
        .post(&format!("/api/table-session/{game_a}"), &bearer_b)
        .await?;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["message"], "This table belongs to another browser");

    let (status, session_b) = app
        .post(&format!("/api/table-session/{game_b}"), &bearer_b)
        .await?;
    assert_eq!(status, 200, "{session_b}");
    let actor_b = session_b["actorId"].as_str().context("actorId")?.to_owned();

    // Recent tables read each snapshot.
    let res = app
        .http
        .get(format!("{}/api/tables/recent?ids={game_a}", app.base))
        .bearer_auth(&bearer_a)
        .send()
        .await?;
    let recent: Value = res.json().await?;
    assert_eq!(recent["unavailable"], false, "{recent}");
    assert_eq!(recent["tables"][0]["status"], "playing", "{recent}");
    assert_eq!(recent["tables"][0]["handNo"], 1, "{recent}");

    let params = json!({ "token": actor_token });
    let ns = engine.namespace();

    // Through the /rivet/** gateway with the issued token (native caller, no Origin).
    let gw = app.gateway_client(ns, Some(&gateway_token));
    let out = http_action(&gw, &actor_a, params.clone()).await?;
    assert_eq!(out["actor"], actor_a.as_str());
    eprintln!("E2E gateway HTTP action: ok");
    let out = ws_action(&gw, &actor_a, params.clone()).await?;
    assert_eq!(out["actor"], actor_a.as_str());
    eprintln!("E2E gateway WebSocket action: ok");

    // The token is scoped to actor A: actor B is refused by the engine.
    let err = http_action(&gw, &actor_b, params.clone())
        .await
        .err()
        .context("actor B must be refused")?;
    eprintln!("E2E gateway token on actor B: {err:#}");
    assert!(
        format!("{err:#}").contains("insufficient_permissions"),
        "{err:#}"
    );

    // No token and no session: the gateway's own 401, before the engine.
    let res = app
        .http
        .post(format!(
            "{}/rivet/gateway/{actor_a}/action/snapshot",
            app.base
        ))
        .header("x-rivet-conn-params", params.to_string())
        .json(&json!({ "args": [] }))
        .send()
        .await?;
    assert_eq!(res.status(), 401);
    assert_eq!(res.text().await?, "Unauthorized");
    // A browser (Origin present) with a gateway token but no session: also 401.
    let res = app
        .http
        .post(format!(
            "{}/rivet/gateway/{actor_a}@{gateway_token}/action/snapshot",
            app.base
        ))
        .header("origin", &app.origin)
        .json(&json!({ "args": [] }))
        .send()
        .await?;
    assert_eq!(res.status(), 401);
    // A rivetkit client without a token sends the loopback `default` token, which
    // the gateway forwards and the engine refuses.
    let anon = app.gateway_client(ns, None);
    let err = http_action(&anon, &actor_a, params.clone())
        .await
        .err()
        .context("anonymous must be refused")?;
    eprintln!("E2E gateway client without token: {err:#}");
    assert!(format!("{err:#}").contains("invalid_token"), "{err:#}");

    // A cookie-session browser through the gateway, with the engine token.
    let res = app
        .http
        .post(format!(
            "{}/rivet/gateway/{actor_a}@{gateway_token}/action/snapshot",
            app.base
        ))
        .header("origin", &app.origin)
        .header(
            "cookie",
            format!(
                "euchre_guest={}",
                auth().sign_guest(&user_a, GuestAudience::Session)?
            ),
        )
        .header("x-rivet-encoding", "json")
        .header("x-rivet-conn-params", params.to_string())
        .json(&json!({ "args": [] }))
        .send()
        .await?;
    let status = res.status();
    let body = res.text().await?;
    eprintln!("E2E browser gateway action: {status} {body}");
    assert_eq!(status, 200, "{body}");

    // A bogus token passes the gateway's shape check and is refused by the engine.
    let bogus = app.gateway_client(ns, Some("bogus.token.value"));
    let err = http_action(&bogus, &actor_a, params.clone())
        .await
        .err()
        .context("a bogus token must be refused")?;
    assert!(format!("{err:#}").contains("invalid_token"), "{err:#}");

    // A query target (getOrCreate by name) never reaches the engine.
    let res = app
        .http
        .post(format!(
            "{}/rivet/gateway/euchreTable/action/snapshot?rvt-namespace={ns}&rvt-method=getOrCreate",
            app.base
        ))
        .send()
        .await?;
    assert_eq!(res.status(), 404);
    assert_eq!(res.text().await?, "Not found");

    // Dot segments and encoded `rvt-*` keys stop at the gateway, even with the
    // issued token: none of these may reach the engine's admin, token-mint, health
    // or actor-internal routes (sent raw so no client normalizes the path first).
    let token_header = [("x-rivet-token", gateway_token.as_str())];
    for (method, path) in [
        (
            "GET",
            format!(
                "/rivet/gateway/{actor_a}/request/../../../actors?namespace={ns}&name=euchreTable"
            ),
        ),
        (
            "GET",
            format!("/rivet/gateway/{actor_a}/request/%2e%2e/%2E%2E/%2e%2E/namespaces"),
        ),
        (
            "GET",
            format!("/rivet/gateway/{actor_a}/request/.%2E/.%2e/.%2E/health"),
        ),
        (
            "POST",
            format!("/rivet/gateway/{actor_a}/request/../../../auth/tokens"),
        ),
        (
            "GET",
            format!("/rivet/gateway/{actor_a}/request/../metadata"),
        ),
        (
            "GET",
            format!("/rivet/gateway/{actor_a}/request/../inspector/state"),
        ),
        (
            "POST",
            format!(
                "/rivet/gateway/{actor_a}/action/snapshot?rvt%2Dnamespace={ns}&rvt%2Dmethod=getOrCreate&rvt%2Dkey=evil&rvt%2Drunner={}&rvt%2Dcrash-policy=sleep",
                engine.pool_name()
            ),
        ),
    ] {
        let (status, body) = raw_request(&app.base, method, &path, &token_header).await?;
        assert_eq!(status, 404, "{method} {path}: {body}");
        assert_eq!(body, "Not found", "{method} {path}");
    }

    // Direct against the engine: the same acceptance table as the D3 probe.
    let direct = engine_client(engine, Some(&gateway_token));
    assert_eq!(
        http_action(&direct, &actor_a, params.clone()).await?["actor"],
        actor_a.as_str()
    );
    let none = engine_client(engine, None);
    let err = http_action(&none, &actor_a, params.clone())
        .await
        .err()
        .context("no token must be refused by the engine")?;
    assert!(format!("{err:#}").contains("invalid_token"), "{err:#}");
    let res = reqwest::Client::new()
        .get(format!("{}/namespaces", engine.endpoint()))
        .bearer_auth(&gateway_token)
        .send()
        .await?;
    assert_eq!(
        res.status(),
        403,
        "the issued token is refused by the admin API"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn profile_endpoints_reach_the_profile_actor() -> Result<()> {
    if !e2e_enabled() {
        e2e_skip("profile_endpoints_reach_the_profile_actor");
        return Ok(());
    }
    let engine = TestEngine::start(engine_options()?).await?;
    let result = async {
        let app = start_app(&engine).await?;
        let identity = auth().identity_for_email("ann@example.com")?;
        let token = auth().issue_identity_token(&identity)?;
        let body = engine
            .ready(|| async {
                let res = app
                    .http
                    .get(format!("{}/api/profile/stats", app.base))
                    .header("x-euchre-identity", &token)
                    .send()
                    .await?;
                if res.status() != 200 {
                    bail!("stats -> {}", res.status());
                }
                Ok(res.json::<Value>().await?)
            })
            .await?;
        assert_eq!(body, json!({ "gamesPlayed": 3 }));
        Ok(())
    }
    .await;
    engine.shutdown().await?;
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bootstrap_ensures_the_namespace_and_pool_on_engine_3() -> Result<()> {
    if !e2e_enabled() {
        e2e_skip("bootstrap_ensures_the_namespace_and_pool_on_engine_3");
        return Ok(());
    }
    let engine = TestEngine::start(engine_options()?).await?;
    let result = async {
        let http = reqwest::Client::new();
        let mut opts = EnsureRivetNamespaceOptions::new(
            engine.endpoint(),
            engine.token().expose(),
            engine.namespace(),
            engine.pool_name(),
        );
        assert_eq!(opts.dialect, AdminDialect::V3);
        opts.timeout = Duration::from_secs(20);
        let first = ensure_rivet_namespace(&http, &opts).await?;
        eprintln!("E2E bootstrap existing namespace: {first:?}");
        assert!(first.runner_config_ensured);
        // Idempotent.
        let again = ensure_rivet_namespace(&http, &opts).await?;
        assert!(again.runner_config_ensured);

        // A namespace that does not exist yet is created.
        let fresh = format!("euchre-e2e-{}", uuid::Uuid::new_v4().simple());
        let mut fresh_opts = opts.clone();
        fresh_opts.namespace = fresh.clone();
        let created = ensure_rivet_namespace(&http, &fresh_opts).await?;
        eprintln!("E2E bootstrap new namespace: {created:?}");
        assert!(created.namespace_created);

        // A wrong admin token is fatal at once.
        let mut bad = opts.clone();
        bad.token = "wrong-token".to_owned();
        let started = std::time::Instant::now();
        let err = ensure_rivet_namespace(&http, &bad)
            .await
            .err()
            .context("a wrong token must fail")?;
        eprintln!("E2E bootstrap wrong token: {err}");
        assert!(started.elapsed() < Duration::from_secs(10));
        Ok(())
    }
    .await;
    engine.shutdown().await?;
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn create_table_is_refused_for_a_cross_site_cookie_caller() -> Result<()> {
    if !e2e_enabled() {
        e2e_skip("create_table_is_refused_for_a_cross_site_cookie_caller");
        return Ok(());
    }
    let engine = TestEngine::start(engine_options()?).await?;
    let result = async {
        let app = start_app(&engine).await?;
        // A browser create: same origin, no session yet. It gets a guest cookie and
        // the recent-tables cookie.
        let res = engine
            .ready(|| async {
                let res = app
                    .http
                    .post(format!("{}/api/tables", app.base))
                    .header("origin", &app.origin)
                    .json(&json!({ "stick": true }))
                    .send()
                    .await?;
                if res.status() != 201 {
                    bail!("create -> {}", res.status());
                }
                Ok(res)
            })
            .await?;
        let cookies: Vec<String> = res
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_owned))
            .collect();
        assert!(
            cookies.iter().any(|c| c.starts_with("euchre_guest=")),
            "{cookies:?}"
        );
        assert!(
            cookies
                .iter()
                .any(|c| c.starts_with("euchre_recent_tables=")),
            "{cookies:?}"
        );
        let pair = |prefix: &str| {
            cookies
                .iter()
                .find(|c| c.starts_with(prefix))
                .and_then(|c| c.split(';').next())
                .map(str::to_owned)
                .with_context(|| format!("no {prefix} cookie in {cookies:?}"))
        };
        let guest = pair("euchre_guest=")?;
        let recent = pair("euchre_recent_tables=")?;
        let game_a = res.json::<Value>().await?["gameId"]
            .as_str()
            .context("gameId")?
            .to_owned();
        let res = app
            .http
            .post(format!("{}/api/tables", app.base))
            .header("origin", "https://evil.example")
            .send()
            .await?;
        assert_eq!(res.status(), 403);

        // A second table goes to the front; opening the first one again moves it
        // back (TS `rememberTable` on the `/play/[gameId]` load, W3 review).
        let res = app
            .http
            .post(format!("{}/api/tables", app.base))
            .header("origin", &app.origin)
            .header("cookie", format!("{guest}; {recent}"))
            .json(&json!({ "stick": true }))
            .send()
            .await?;
        assert_eq!(res.status(), 201);
        let recent_cookie = |res: &reqwest::Response| {
            res.headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|v| v.to_str().ok())
                .find(|c| c.starts_with("euchre_recent_tables="))
                .and_then(|c| c.split(';').next())
                .map(str::to_owned)
        };
        let recent = recent_cookie(&res).context("recent tables after the second create")?;
        let game_b = res.json::<Value>().await?["gameId"]
            .as_str()
            .context("gameId")?
            .to_owned();
        assert_eq!(recent, format!("euchre_recent_tables={game_b}%2C{game_a}"));
        let res = app
            .http
            .post(format!("{}/api/table-session/{game_a}", app.base))
            .header("origin", &app.origin)
            .header("cookie", format!("{guest}; {recent}"))
            .send()
            .await?;
        assert_eq!(res.status(), 200);
        assert_eq!(
            recent_cookie(&res).as_deref(),
            Some(format!("euchre_recent_tables={game_a}%2C{game_b}").as_str())
        );
        Ok(())
    }
    .await;
    engine.shutdown().await?;
    result
}
