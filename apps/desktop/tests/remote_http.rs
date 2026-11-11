//! The remote backend's HTTP side against a scripted server (wiremock): every call's method,
//! path, headers and body; the device session (enroll, refresh at 80 %, enroll again, one
//! recovery per `401`, single-flight); the create retry; the status mapping; recent tables;
//! transport failures and redirects. No engine, no network beyond loopback: always runs.
//!
//! Every test's files live in a fresh temp dir under `$EUCHRE_TEST_TMP` (else the system
//! temp dir).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use euchre_desktop::net::remote::device::{DeviceRecord, DeviceStore};
use euchre_desktop::net::remote::http::{Api, wire};
use euchre_desktop::net::remote::recent::RecentStore;
use euchre_desktop::net::remote::{RemoteBackend, RemoteConfig};
use euchre_desktop::net::{Backend, NetError, RecentTable, RecentTables, TableConfig};
use euchre_profile_model::{SettingsPatch, compute_stats, default_settings};
use euchre_protocol::GameStatus;
use euchre_server_core::auth::guest::SESSION_TTL_SECS;
use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::matchers::{header, method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const LIFETIME_MS: i64 = SESSION_TTL_SECS * 1000;

fn now_ms() -> i64 {
    euchre_server_core::auth::now_ms()
}

/// A fresh temp dir under `$EUCHRE_TEST_TMP` (else the system temp dir).
fn temp() -> anyhow::Result<TempDir> {
    let root = std::env::var_os("EUCHRE_TEST_TMP")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&root)?;
    Ok(tempfile::Builder::new()
        .prefix("n1-http-")
        .tempdir_in(root)?)
}

/// What `POST /api/device/enroll` and `/refresh` answer: guest `n`, token `tok-<n>`, a full
/// lifetime from now.
fn enrollment(user: u32, token: u32) -> Value {
    json!({
        "userId": format!("guest-{user}"),
        "sessionToken": format!("tok-{token}"),
        "expiresAt": now_ms() + LIFETIME_MS,
    })
}

fn bearer(token: u32) -> String {
    format!("Bearer tok-{token}")
}

fn game_id(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

fn stats_body() -> anyhow::Result<Value> {
    Ok(serde_json::to_value(compute_stats(&[], now_ms()))?)
}

fn settings_body() -> anyhow::Result<Value> {
    Ok(serde_json::to_value(default_settings(now_ms()))?)
}

/// One scripted server and one backend whose files live in a temp dir.
struct Rig {
    server: MockServer,
    _dir: TempDir,
    config: RemoteConfig,
    backend: RemoteBackend,
}

impl Rig {
    async fn new() -> anyhow::Result<Rig> {
        let server = MockServer::start().await;
        let dir = temp()?;
        let config = RemoteConfig::remote(&server.uri(), dir.path())?;
        let backend = RemoteBackend::new(config.clone())?;
        Ok(Rig {
            server,
            _dir: dir,
            config,
            backend,
        })
    }

    /// A second backend on the same files (the next launch).
    fn relaunch(&self) -> anyhow::Result<RemoteBackend> {
        Ok(RemoteBackend::new(self.config.clone())?)
    }

    fn device(&self) -> DeviceStore {
        DeviceStore::new(&self.config.state_dir, &self.config.server_key)
    }

    fn api(&self) -> anyhow::Result<Api> {
        Ok(Api::new(&self.server.uri())?)
    }

    /// Writes `device.json` for guest 1 with token `tok-<token>`, `gone` of its lifetime
    /// already passed.
    fn seed_device(&self, token: u32, gone: f64) -> anyhow::Result<()> {
        let elapsed = (LIFETIME_MS as f64 * gone) as i64;
        self.device().save(&DeviceRecord {
            user_id: "guest-1".into(),
            session_token: format!("tok-{token}"),
            expires_at: now_ms() - elapsed + LIFETIME_MS,
            server_url: self.config.server_key.clone(),
        })?;
        Ok(())
    }

    /// Answers `POST /api/device/enroll` with guest `user` / `tok-<token>`, exactly `times`.
    async fn enrolls(&self, user: u32, token: u32, times: u64) {
        Mock::given(method("POST"))
            .and(path("/api/device/enroll"))
            .respond_with(ResponseTemplate::new(200).set_body_json(enrollment(user, token)))
            .expect(times)
            .mount(&self.server)
            .await;
    }

    /// Answers `POST /api/device/refresh` from `tok-<from>` with `tok-<to>`, exactly `times`.
    async fn refreshes(&self, from: u32, to: u32, times: u64) {
        Mock::given(method("POST"))
            .and(path("/api/device/refresh"))
            .and(header("authorization", bearer(from).as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(enrollment(1, to)))
            .expect(times)
            .mount(&self.server)
            .await;
    }

    /// Answers `GET /api/profile/stats` for `tok-<token>` with `status`, exactly `times`.
    async fn stats_for(&self, token: u32, status: u16, times: u64) -> anyhow::Result<()> {
        let template = if status == 200 {
            ResponseTemplate::new(200).set_body_json(stats_body()?)
        } else {
            ResponseTemplate::new(status)
        };
        Mock::given(method("GET"))
            .and(path("/api/profile/stats"))
            .and(header("authorization", bearer(token).as_str()))
            .respond_with(template)
            .expect(times)
            .mount(&self.server)
            .await;
        Ok(())
    }

    async fn requests(&self) -> Vec<Request> {
        self.server.received_requests().await.unwrap_or_default()
    }

    async fn hits(&self, verb: &str, at: &str) -> usize {
        self.requests()
            .await
            .iter()
            .filter(|r| r.method.as_str() == verb && r.url.path() == at)
            .count()
    }
}

fn head(req: &Request, name: &str) -> Option<String> {
    req.headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn query(req: &Request, key: &str) -> Option<String> {
    req.url
        .query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn mode(p: &Path) -> anyhow::Result<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Ok(std::fs::metadata(p)?.permissions().mode() & 0o777)
}

// ------------------------------------------------------------------------ the calls

#[tokio::test]
async fn every_call_sends_its_method_path_headers_and_body() -> anyhow::Result<()> {
    let rig = Rig::new().await?;
    let server = &rig.server;
    let id = game_id(1);
    rig.enrolls(1, 1, 1).await;
    rig.refreshes(1, 1, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/session"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "guestUserId": "guest-1" })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/tables"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "gameId": id })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/tables/recent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "tables": [{ "gameId": id, "status": "active", "score": [3, 5], "handNo": 4 }],
            "unavailable": false,
        })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/table-session/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "actorId": "actor-1",
            "token": "actor-token",
            "gatewayToken": "gw-token",
            "expiresAt": 1_900_000_000_000_u64,
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/rivet/metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "clientEndpoint": format!("{}/rivet", server.uri()),
            "clientNamespace": "default",
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/games"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "rows": [], "cursor": null })),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/api/games/.+$"))
        .respond_with(ResponseTemplate::new(404))
        .mount(server)
        .await;
    rig.stats_for(1, 200, 1).await?;
    Mock::given(method("GET"))
        .and(path("/api/profile/settings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(settings_body()?))
        .mount(server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/profile/settings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(settings_body()?))
        .mount(server)
        .await;

    let b = &rig.backend;
    assert_eq!(b.identity().await?.user_id, "guest-1");
    let created = b
        .create_table(TableConfig {
            stick_the_dealer: true,
            require_natural_trump: false,
        })
        .await?;
    assert_eq!(created, id);
    let recent = b.recent_tables().await?;
    assert_eq!(
        recent.tables,
        vec![RecentTable {
            game_id: id.clone(),
            status: Some(GameStatus::Active),
            score: Some([3, 5]),
            hand_no: Some(4),
        }]
    );
    assert!(b.list_games(None).await?.rows.is_empty());
    assert!(b.list_games(Some("next page".into())).await.is_ok());
    assert_eq!(b.match_detail("m 1/x").await, Err(NetError::NotFound));
    assert!(b.stats().await.is_ok());
    assert!(b.get_settings().await.is_ok());
    let patch = SettingsPatch {
        house_prompt: Some("Play it straight.".into()),
        ..SettingsPatch::default()
    };
    assert!(b.patch_settings(patch.clone()).await.is_ok());
    // The three the backend calls inside `open_table` and the session, one by one.
    let api = rig.api()?;
    assert_eq!(api.refresh("tok-1").await?.session_token, "tok-1");
    let session = api.table_session("tok-1", &id).await?;
    assert_eq!(session.gateway_token.as_deref(), Some("gw-token"));
    assert_eq!(api.rivet_metadata().await?.client_namespace, "default");

    let requests = rig.requests().await;
    assert_eq!(
        requests.len(),
        13,
        "{:?}",
        requests.iter().map(|r| r.url.path()).collect::<Vec<_>>()
    );
    for req in &requests {
        let at = format!("{} {}", req.method, req.url.path());
        assert_eq!(
            head(req, "accept").as_deref(),
            Some("application/json"),
            "{at}"
        );
        let agent = head(req, "user-agent").unwrap_or_default();
        assert!(agent.starts_with("euchre-desktop/"), "{at}: {agent}");
        for never in ["origin", "cookie", "x-euchre-identity"] {
            assert_eq!(head(req, never), None, "{at} sent {never}");
        }
        let auth = head(req, "authorization");
        match req.url.path() {
            "/api/device/enroll" | "/api/rivet/metadata" => assert_eq!(auth, None, "{at}"),
            _ => assert_eq!(auth.as_deref(), Some("Bearer tok-1"), "{at}"),
        }
        let has_body = matches!(
            (req.method.as_str(), req.url.path()),
            ("POST", "/api/tables") | ("PATCH", "/api/profile/settings")
        );
        if has_body {
            assert_eq!(
                head(req, "content-type").as_deref(),
                Some("application/json"),
                "{at}"
            );
        } else {
            assert!(req.body.is_empty(), "{at} sent a body");
            assert_eq!(head(req, "content-type"), None, "{at}");
        }
        let profiled =
            req.url.path().starts_with("/api/games") || req.url.path().starts_with("/api/profile/");
        if profiled {
            assert_eq!(query(req, "profile").as_deref(), Some("browser"), "{at}");
        }
    }
    let find = |verb: &str, at: &str| -> Vec<&Request> {
        requests
            .iter()
            .filter(|r| r.method.as_str() == verb && r.url.path() == at)
            .collect()
    };
    let create = find("POST", "/api/tables");
    assert_eq!(
        serde_json::from_slice::<Value>(&create[0].body)?,
        json!({ "stick": true, "natural": false })
    );
    let patched = find("PATCH", "/api/profile/settings");
    assert_eq!(
        serde_json::from_slice::<Value>(&patched[0].body)?,
        json!({ "housePrompt": "Play it straight." })
    );
    assert_eq!(
        query(find("GET", "/api/tables/recent")[0], "ids"),
        Some(id.clone())
    );
    let games = find("GET", "/api/games");
    assert_eq!(games.len(), 2);
    assert_eq!(query(games[0], "limit").as_deref(), Some("10"));
    assert_eq!(query(games[0], "cursor"), None);
    assert_eq!(query(games[1], "cursor").as_deref(), Some("next page"));
    // The match id is one encoded path segment.
    assert_eq!(find("GET", "/api/games/m%201%2Fx").len(), 1);
    assert_eq!(find("POST", &format!("/api/table-session/{id}")).len(), 1);
    Ok(())
}

// ------------------------------------------------------------------- the device

#[tokio::test]
async fn first_call_enrolls_and_saves_the_device() -> anyhow::Result<()> {
    let rig = Rig::new().await?;
    rig.enrolls(1, 1, 1).await;
    rig.stats_for(1, 200, 3).await?;
    assert!(!rig.config.state_dir.exists(), "new() touched the disk");
    rig.backend.stats().await?;
    rig.backend.stats().await?;
    let file = rig.config.state_dir.join("device.json");
    assert_eq!(mode(&file)?, 0o600);
    assert_eq!(mode(&rig.config.state_dir)?, 0o700);
    let saved: Value = serde_json::from_slice(&std::fs::read(&file)?)?;
    assert_eq!(saved["userId"], "guest-1");
    assert_eq!(saved["sessionToken"], "tok-1");
    assert_eq!(saved["serverUrl"], Value::String(rig.server.uri()));
    // The next launch reads the record: no second enroll.
    rig.relaunch()?.stats().await?;
    rig.server.verify().await;
    Ok(())
}

#[tokio::test]
async fn refreshes_at_eighty_percent() -> anyhow::Result<()> {
    // 79 % of the lifetime gone: used as it is.
    let rig = Rig::new().await?;
    rig.seed_device(1, 0.79)?;
    rig.stats_for(1, 200, 1).await?;
    rig.backend.stats().await?;
    assert_eq!(rig.hits("POST", "/api/device/refresh").await, 0);
    assert_eq!(rig.hits("POST", "/api/device/enroll").await, 0);

    // 81 % gone: refreshed once, saved, then used.
    let rig = Rig::new().await?;
    rig.seed_device(1, 0.81)?;
    rig.refreshes(1, 2, 1).await;
    rig.stats_for(2, 200, 2).await?;
    rig.backend.stats().await?;
    rig.backend.stats().await?;
    let saved = rig.device().load();
    assert_eq!(
        saved.as_ref().map(|r| r.session_token.as_str()),
        Some("tok-2")
    );
    assert_eq!(saved.as_ref().map(|r| r.user_id.as_str()), Some("guest-1"));
    assert!(saved.is_some_and(|r| r.expires_at > now_ms() + LIFETIME_MS / 2));
    assert_eq!(rig.hits("POST", "/api/device/enroll").await, 0);

    // A refresh that fails another way keeps the current session.
    let rig = Rig::new().await?;
    rig.seed_device(1, 0.9)?;
    Mock::given(method("POST"))
        .and(path("/api/device/refresh"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&rig.server)
        .await;
    rig.stats_for(1, 200, 1).await?;
    rig.backend.stats().await?;
    assert_eq!(
        rig.device().load().map(|r| r.session_token),
        Some("tok-1".to_owned())
    );
    Ok(())
}

#[tokio::test]
async fn reenrolls_when_refresh_is_refused() -> anyhow::Result<()> {
    let rig = Rig::new().await?;
    rig.seed_device(1, 0.95)?;
    Mock::given(method("POST"))
        .and(path("/api/device/refresh"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&rig.server)
        .await;
    rig.enrolls(2, 2, 1).await;
    rig.stats_for(2, 200, 1).await?;
    rig.backend.stats().await?;
    let saved = rig.device().load();
    assert_eq!(saved.as_ref().map(|r| r.user_id.as_str()), Some("guest-2"));
    assert_eq!(saved.map(|r| r.session_token), Some("tok-2".to_owned()));
    // An expired record (or one for another server) enrolls without a refresh.
    let rig = Rig::new().await?;
    rig.seed_device(1, 1.0)?;
    rig.enrolls(3, 3, 1).await;
    rig.stats_for(3, 200, 1).await?;
    rig.backend.stats().await?;
    assert_eq!(rig.hits("POST", "/api/device/refresh").await, 0);
    rig.server.verify().await;
    Ok(())
}

#[tokio::test]
async fn a_401_recovers_once_then_reports_unauthorized() -> anyhow::Result<()> {
    // The session was revoked server-side: one 401, a refresh, the call again.
    let rig = Rig::new().await?;
    rig.seed_device(1, 0.1)?;
    rig.stats_for(1, 401, 1).await?;
    rig.refreshes(1, 2, 1).await;
    rig.stats_for(2, 200, 1).await?;
    assert!(rig.backend.stats().await.is_ok());
    rig.server.verify().await;

    // The refresh is refused too: enroll again, then the call again.
    let rig = Rig::new().await?;
    rig.seed_device(1, 0.1)?;
    rig.stats_for(1, 401, 1).await?;
    Mock::given(method("POST"))
        .and(path("/api/device/refresh"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&rig.server)
        .await;
    rig.enrolls(2, 2, 1).await;
    rig.stats_for(2, 200, 1).await?;
    assert!(rig.backend.stats().await.is_ok());
    rig.server.verify().await;

    // Still 401 after the recovery: Unauthorized, and no third try.
    let rig = Rig::new().await?;
    rig.seed_device(1, 0.1)?;
    rig.stats_for(1, 401, 1).await?;
    rig.refreshes(1, 2, 1).await;
    rig.stats_for(2, 401, 1).await?;
    assert_eq!(rig.backend.stats().await, Err(NetError::Unauthorized));
    rig.server.verify().await;
    Ok(())
}

#[tokio::test]
async fn identity_rechecks_an_unverified_session() -> anyhow::Result<()> {
    let rig = Rig::new().await?;
    rig.seed_device(1, 0.1)?;
    // The bearer no longer verifies (`guestUserId: null`): refresh, then it does.
    Mock::given(method("GET"))
        .and(path("/api/session"))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "guestUserId": null })))
        .expect(1)
        .mount(&rig.server)
        .await;
    rig.refreshes(1, 2, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/session"))
        .and(header("authorization", "Bearer tok-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "guestUserId": "guest-1" })))
        .expect(1)
        .mount(&rig.server)
        .await;
    assert_eq!(rig.backend.identity().await?.user_id, "guest-1");
    rig.server.verify().await;

    // Another guest's id never passes.
    let rig = Rig::new().await?;
    rig.seed_device(1, 0.1)?;
    rig.refreshes(1, 2, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/session"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "guestUserId": "guest-9" })))
        .expect(2)
        .mount(&rig.server)
        .await;
    assert_eq!(rig.backend.identity().await, Err(NetError::Unauthorized));
    rig.server.verify().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_calls_share_one_enroll() -> anyhow::Result<()> {
    let rig = Rig::new().await?;
    Mock::given(method("POST"))
        .and(path("/api/device/enroll"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(enrollment(1, 1))
                .set_delay(Duration::from_millis(150)),
        )
        .expect(1)
        .mount(&rig.server)
        .await;
    rig.stats_for(1, 200, 8).await?;
    let calls = (0..8).map(|_| {
        let backend = rig.backend.clone();
        tokio::spawn(async move { backend.stats().await })
    });
    for joined in futures::future::join_all(calls).await {
        assert!(joined?.is_ok());
    }
    rig.server.verify().await;
    Ok(())
}

// ------------------------------------------------------------------- create table

#[tokio::test]
async fn create_retries_503_then_succeeds() -> anyhow::Result<()> {
    let rig = Rig::new().await?;
    rig.enrolls(1, 1, 1).await;
    Mock::given(method("POST"))
        .and(path("/api/tables"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&rig.server)
        .await;
    let id = game_id(7);
    Mock::given(method("POST"))
        .and(path("/api/tables"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "gameId": id })))
        .expect(1)
        .mount(&rig.server)
        .await;
    let started = Instant::now();
    let created = rig
        .backend
        .create_table(TableConfig {
            stick_the_dealer: false,
            require_natural_trump: true,
        })
        .await?;
    assert_eq!(created, id);
    assert_eq!(rig.hits("POST", "/api/tables").await, 3);
    // 250 ms, then 500 ms.
    assert!(
        started.elapsed() >= Duration::from_millis(700),
        "{:?}",
        started.elapsed()
    );
    // Remembered for Continue.
    assert_eq!(RecentStore::new(&rig.config.state_dir).load(), vec![id]);
    Ok(())
}

#[tokio::test]
async fn create_gives_up_after_its_window() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/tables"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let api = Api::new(&server.uri())?.with_create_retry(Duration::from_millis(500));
    assert_eq!(api.create_retry_for(), Duration::from_millis(500));
    let body = wire::CreateTableBody {
        stick: false,
        natural: false,
    };
    let started = Instant::now();
    assert_eq!(
        api.create_table("tok-1", body).await,
        Err(NetError::Unavailable)
    );
    let took = started.elapsed();
    assert!(
        (Duration::from_millis(500)..Duration::from_millis(1500)).contains(&took),
        "{took:?}"
    );
    let hits = server.received_requests().await.unwrap_or_default().len();
    // 0, 250, 500 ms.
    assert!((2..=4).contains(&hits), "{hits}");

    // 429 and other refusals are not retried.
    for (status, err) in [
        (429, NetError::RateLimited),
        (400, NetError::Invalid("bad_request".into())),
        (500, NetError::Unavailable),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/tables"))
            .respond_with(ResponseTemplate::new(status))
            .expect(1)
            .mount(&server)
            .await;
        let api = Api::new(&server.uri())?;
        assert_eq!(api.create_table("tok-1", body).await, Err(err), "{status}");
        server.verify().await;
    }
    Ok(())
}

// ------------------------------------------------------------------ table session

#[tokio::test]
async fn table_session_statuses_map() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    let error_body = |code: &str| json!({ "message": "Something the browser shows", "error": { "code": code, "message": "x" } });
    let cases: Vec<(u16, Option<Value>, NetError)> = vec![
        (400, Some(error_body("invalid_table")), NetError::NotFound),
        (
            401,
            Some(error_body("unauthorized")),
            NetError::Unauthorized,
        ),
        (403, Some(error_body("forbidden")), NetError::Forbidden),
        (404, None, NetError::NotFound),
        (413, None, NetError::Invalid("too_large".into())),
        (421, None, NetError::Unavailable),
        (429, Some(error_body("rate_limited")), NetError::RateLimited),
        (500, None, NetError::Unavailable),
        (
            503,
            Some(error_body("table_unavailable")),
            NetError::Unavailable,
        ),
    ];
    for (n, (status, body, _)) in cases.iter().enumerate() {
        let mut template = ResponseTemplate::new(*status);
        if let Some(body) = body {
            template = template.set_body_json(body);
        }
        Mock::given(method("POST"))
            .and(path(format!("/api/table-session/{}", game_id(n as u32))))
            .respond_with(template)
            .mount(&server)
            .await;
    }
    // A 200 that is not a session, and one with no gateway token.
    let garbled = game_id(100);
    Mock::given(method("POST"))
        .and(path(format!("/api/table-session/{garbled}")))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
        .mount(&server)
        .await;
    let api = Api::new(&server.uri())?;
    for (n, (status, _, err)) in cases.iter().enumerate() {
        assert_eq!(
            api.table_session("tok-1", &game_id(n as u32)).await.err(),
            Some(err.clone()),
            "{status}"
        );
    }
    assert_eq!(
        api.table_session("tok-1", &garbled).await.err(),
        Some(NetError::Unavailable)
    );

    // Through the backend: a 403 mint is Forbidden and nothing dials.
    let rig = Rig::new().await?;
    rig.enrolls(1, 1, 1).await;
    let theirs = game_id(1);
    Mock::given(method("POST"))
        .and(path(format!("/api/table-session/{theirs}")))
        .respond_with(ResponseTemplate::new(403).set_body_json(error_body("forbidden")))
        .expect(1)
        .mount(&rig.server)
        .await;
    assert!(matches!(
        rig.backend.open_table(&theirs).await,
        Err(NetError::Forbidden)
    ));
    // A session with no gateway token is Unavailable: the desktop never dials without one.
    let tokenless = game_id(2);
    Mock::given(method("POST"))
        .and(path(format!("/api/table-session/{tokenless}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "actorId": "actor-2",
            "token": "actor-token",
            "gatewayToken": null,
            "expiresAt": 1_900_000_000_000_u64,
        })))
        .mount(&rig.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/rivet/metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "clientEndpoint": format!("{}/rivet", rig.server.uri()),
            "clientNamespace": "default",
        })))
        .mount(&rig.server)
        .await;
    assert!(matches!(
        rig.backend.open_table(&tokenless).await,
        Err(NetError::Unavailable)
    ));
    // Not a game id: NotFound, no call at all.
    let before = rig.requests().await.len();
    assert!(matches!(
        rig.backend.open_table("../api/device/enroll").await,
        Err(NetError::NotFound)
    ));
    assert_eq!(rig.requests().await.len(), before);
    assert!(
        rig.requests()
            .await
            .iter()
            .all(|r| !r.url.path().starts_with("/rivet")),
        "dialed the gateway"
    );
    // A failed open remembers nothing.
    assert!(RecentStore::new(&rig.config.state_dir).load().is_empty());
    rig.server.verify().await;
    Ok(())
}

/// Review fixes PP-1 and PP-7: a table the server says does not exist leaves the saved
/// list (a `null` row still never prunes); a device that was signed out and enrolled again
/// reads its old tables' `403` as signed out, on this launch and the next.
#[tokio::test]
async fn a_gone_table_is_forgotten_and_a_signed_out_device_says_so() -> anyhow::Result<()> {
    let error_body = |code: &str| json!({ "error": { "code": code, "message": "x" } });
    let rig = Rig::new().await?;
    rig.enrolls(1, 1, 1).await;
    let recent = RecentStore::new(&rig.config.state_dir);
    let (gone, theirs) = (game_id(7), game_id(8));
    recent.remember(&theirs)?;
    recent.remember(&gone)?;
    Mock::given(method("POST"))
        .and(path(format!("/api/table-session/{gone}")))
        .respond_with(ResponseTemplate::new(404).set_body_json(error_body("not_found")))
        .expect(1)
        .mount(&rig.server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/table-session/{theirs}")))
        .respond_with(ResponseTemplate::new(403).set_body_json(error_body("forbidden")))
        .mount(&rig.server)
        .await;
    assert!(matches!(
        rig.backend.open_table(&gone).await,
        Err(NetError::NotFound)
    ));
    assert_eq!(recent.load(), vec![theirs.clone()], "the gone table left");
    // Never signed out: a 403 is another device's table, and it stays listed.
    assert!(matches!(
        rig.backend.open_table(&theirs).await,
        Err(NetError::Forbidden)
    ));
    assert_eq!(recent.load(), vec![theirs.clone()]);
    assert_eq!(rig.device().load_reset(), None);
    rig.server.verify().await;

    // The refresh is refused: the device enrolls again as guest 2, and remembers it lost
    // guest 1.
    let rig = Rig::new().await?;
    rig.seed_device(1, 0.95)?;
    Mock::given(method("POST"))
        .and(path("/api/device/refresh"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&rig.server)
        .await;
    rig.enrolls(2, 2, 1).await;
    let old = game_id(9);
    Mock::given(method("POST"))
        .and(path(format!("/api/table-session/{old}")))
        .respond_with(ResponseTemplate::new(403).set_body_json(error_body("forbidden")))
        .mount(&rig.server)
        .await;
    assert!(matches!(
        rig.backend.open_table(&old).await,
        Err(NetError::SignedOut)
    ));
    let marker = rig.device().load_reset().expect("the reset is recorded");
    assert_eq!(marker.previous_user_id, "guest-1");
    let marker_path = rig.config.state_dir.join("device-reset.json");
    assert_eq!(mode(&marker_path)? & 0o777, 0o600);
    // The next launch still knows.
    let next = rig.relaunch()?;
    assert!(matches!(
        next.open_table(&old).await,
        Err(NetError::SignedOut)
    ));
    rig.server.verify().await;

    // A session that ran out on disk enrolls again the same way.
    let rig = Rig::new().await?;
    rig.seed_device(1, 1.0)?;
    rig.enrolls(3, 3, 1).await;
    rig.stats_for(3, 200, 1).await?;
    rig.backend.stats().await?;
    assert_eq!(
        rig.device().load_reset().map(|m| m.previous_user_id),
        Some("guest-1".to_owned())
    );
    Ok(())
}

// ------------------------------------------------------------------ recent tables

/// Answers `GET /api/tables/recent?ids=` with one row per asked id, in order: active,
/// complete, then an unreadable (all `null`) row.
struct EchoRecent;

impl Respond for EchoRecent {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let ids = query(request, "ids").unwrap_or_default();
        let rows: Vec<Value> = ids
            .split(',')
            .enumerate()
            .map(|(n, id)| match n {
                0 => json!({ "gameId": id, "status": "active", "score": [2, 1], "handNo": 3 }),
                1 => json!({ "gameId": id, "status": "complete", "score": [10, 6], "handNo": 9 }),
                _ => json!({ "gameId": id, "status": null, "score": null, "handNo": null }),
            })
            .collect();
        ResponseTemplate::new(200).set_body_json(json!({ "tables": rows, "unavailable": true }))
    }
}

#[tokio::test]
async fn recent_tables_skips_the_call_when_empty_and_keeps_order() -> anyhow::Result<()> {
    let rig = Rig::new().await?;
    // Nothing remembered: no call, not even an enroll.
    assert_eq!(
        rig.backend.recent_tables().await,
        Ok(RecentTables::default())
    );
    assert!(rig.requests().await.is_empty());

    let store = RecentStore::new(&rig.config.state_dir);
    for n in [3, 2, 1] {
        store.remember(&game_id(n))?;
    }
    rig.enrolls(1, 1, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/tables/recent"))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(EchoRecent)
        .expect(1)
        .mount(&rig.server)
        .await;
    let recent = rig.backend.recent_tables().await?;
    let asked = rig
        .requests()
        .await
        .into_iter()
        .find(|r| r.url.path() == "/api/tables/recent")
        .and_then(|r| query(&r, "ids"));
    assert_eq!(
        asked,
        Some(format!("{},{},{}", game_id(1), game_id(2), game_id(3)))
    );
    assert!(recent.unavailable);
    assert_eq!(
        recent.tables,
        vec![
            RecentTable {
                game_id: game_id(1),
                status: Some(GameStatus::Active),
                score: Some([2, 1]),
                hand_no: Some(3),
            },
            RecentTable {
                game_id: game_id(2),
                status: Some(GameStatus::Complete),
                score: Some([10, 6]),
                hand_no: Some(9),
            },
            RecentTable {
                game_id: game_id(3),
                status: None,
                score: None,
                hand_no: None,
            },
        ]
    );
    // A null row is never pruned: the table may just be asleep.
    assert_eq!(store.load(), vec![game_id(1), game_id(2), game_id(3)]);
    rig.server.verify().await;
    Ok(())
}

#[tokio::test]
async fn recent_tables_recovers_an_unverified_session() -> anyhow::Result<()> {
    let rig = Rig::new().await?;
    rig.seed_device(1, 0.1)?;
    RecentStore::new(&rig.config.state_dir).remember(&game_id(1))?;
    // The old bearer does not verify: the handler answers 200 with no rows.
    Mock::given(method("GET"))
        .and(path("/api/tables/recent"))
        .and(header("authorization", "Bearer tok-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "tables": [], "unavailable": false })),
        )
        .expect(1)
        .mount(&rig.server)
        .await;
    rig.refreshes(1, 2, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/tables/recent"))
        .and(header("authorization", "Bearer tok-2"))
        .respond_with(EchoRecent)
        .expect(1)
        .mount(&rig.server)
        .await;
    let recent = rig.backend.recent_tables().await?;
    assert_eq!(recent.tables.len(), 1);
    assert_eq!(recent.tables[0].status, Some(GameStatus::Active));
    rig.server.verify().await;

    // Still empty after the recovery: no tables, no error.
    let rig = Rig::new().await?;
    rig.seed_device(1, 0.1)?;
    RecentStore::new(&rig.config.state_dir).remember(&game_id(1))?;
    Mock::given(method("GET"))
        .and(path("/api/tables/recent"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "tables": [], "unavailable": false })),
        )
        .expect(2)
        .mount(&rig.server)
        .await;
    rig.refreshes(1, 2, 1).await;
    assert_eq!(
        rig.backend.recent_tables().await,
        Ok(RecentTables::default())
    );
    rig.server.verify().await;
    Ok(())
}

// --------------------------------------------------------------- transport failures

#[tokio::test]
async fn transport_failures_map_to_offline_and_timeout() -> anyhow::Result<()> {
    // Nothing listens: Offline, and the device keeps no record.
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.local_addr()?.port()
    };
    let dir = temp()?;
    let config = RemoteConfig::remote(&format!("http://127.0.0.1:{closed}"), dir.path())?;
    let backend = RemoteBackend::new(config.clone())?;
    assert_eq!(backend.stats().await, Err(NetError::Offline));
    assert_eq!(backend.identity().await, Err(NetError::Offline));
    assert!(!config.state_dir.join("device.json").exists());
    // Create retries a refused connect (nothing was sent), then reports Offline.
    let api = Api::new(&config.base_url)?.with_create_retry(Duration::from_millis(300));
    let body = wire::CreateTableBody {
        stick: false,
        natural: false,
    };
    let started = Instant::now();
    assert_eq!(
        api.create_table("tok-1", body).await,
        Err(NetError::Offline)
    );
    assert!(started.elapsed() >= Duration::from_millis(250));

    // A reply slower than the request timeout: Timeout, and create does not retry it (the
    // table may exist).
    let server = MockServer::start().await;
    let slow = ResponseTemplate::new(200).set_delay(Duration::from_secs(2));
    Mock::given(method("GET"))
        .and(path("/api/profile/stats"))
        .respond_with(slow.clone())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/tables"))
        .respond_with(slow)
        .mount(&server)
        .await;
    let api = Api::new(&server.uri())?
        .with_timeouts(Duration::from_secs(1), Duration::from_millis(300))?;
    let started = Instant::now();
    assert_eq!(api.stats("tok-1").await, Err(NetError::Timeout));
    assert!(started.elapsed() < Duration::from_millis(1500));
    assert_eq!(
        api.create_table("tok-1", body).await,
        Err(NetError::Timeout)
    );
    let creates = server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.url.path() == "/api/tables")
        .count();
    assert_eq!(creates, 1);

    // A 2xx body that does not decode is Unavailable.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/profile/settings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "nope": true })))
        .mount(&server)
        .await;
    assert_eq!(
        Api::new(&server.uri())?.get_settings("tok-1").await.err(),
        Some(NetError::Unavailable)
    );
    Ok(())
}

#[tokio::test]
async fn redirects_are_not_followed() -> anyhow::Result<()> {
    let rig = Rig::new().await?;
    let followed = Arc::new(AtomicUsize::new(0));
    struct Count(Arc<AtomicUsize>);
    impl Respond for Count {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            self.0.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(enrollment(1, 1))
        }
    }
    Mock::given(path("/elsewhere"))
        .respond_with(Count(Arc::clone(&followed)))
        .mount(&rig.server)
        .await;
    for status in [301, 302, 307, 308] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/device/enroll"))
            .respond_with(ResponseTemplate::new(status).insert_header(
                "location",
                format!("{}/elsewhere", rig.server.uri()).as_str(),
            ))
            .mount(&server)
            .await;
        let dir = temp()?;
        let backend = RemoteBackend::new(RemoteConfig::remote(&server.uri(), dir.path())?)?;
        assert_eq!(
            backend.stats().await,
            Err(NetError::Unavailable),
            "{status}"
        );
    }
    assert_eq!(followed.load(Ordering::SeqCst), 0);
    // A redirect on a call with a bearer is not followed either: the bearer never travels
    // to another URL.
    let api = rig.api()?;
    Mock::given(method("GET"))
        .and(path("/api/profile/stats"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "location",
            format!("{}/elsewhere", rig.server.uri()).as_str(),
        ))
        .mount(&rig.server)
        .await;
    assert_eq!(api.stats("tok-1").await.err(), Some(NetError::Unavailable));
    assert_eq!(followed.load(Ordering::SeqCst), 0);
    Ok(())
}
