//! Bucket mode against wiremock standing in for the bucket, through
//! [`Downloads::serve`]. The signing itself is checked against AWS's published
//! example in `sigv4.rs`.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::downloads::Downloads;

const BUCKET: &str = "examplebucket";
const ENDPOINT_HOST: &str = "s3.test";
const MAC: &str = "Euchre-0.1.0-mac.dmg";
const WINDOWS: &str = "Euchre-0.1.0-windows.zip";
const SHA: &str = "3dc7aa7b0f9cbd6b3c4e3c2a2f5d1e0b9c8a7f6e5d4c3b2a1f0e9d8c7b6a9b90";

fn creds(endpoint: &str) -> BucketCredentials {
    BucketCredentials {
        bucket: BUCKET.to_owned(),
        endpoint: endpoint.to_owned(),
        region: "auto".to_owned(),
        access_key_id: "AKIDEXAMPLE".to_owned(),
        secret_access_key: Secret::new("very-secret-key"),
    }
}

fn manifest(version: &str) -> Value {
    json!({
        "version": version,
        "platforms": {
            "mac": { "file": MAC, "bytes": 4096, "sha256": SHA },
            "windows": { "file": WINDOWS, "bytes": 3000, "sha256": SHA },
        }
    })
}

async fn mount_manifest(server: &MockServer, body: &Value) {
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// A bucket-mode handler whose client sends `examplebucket.s3.test` to `server`.
fn downloads(server: &MockServer, reload_every: Duration) -> anyhow::Result<Downloads> {
    let endpoint = format!("http://{ENDPOINT_HOST}:{}", server.address().port());
    let settings = BucketSettings::new(creds(&endpoint)).map_err(anyhow::Error::msg)?;
    let http = reqwest::Client::builder()
        .no_proxy()
        .resolve(&format!("{BUCKET}.{ENDPOINT_HOST}"), *server.address())
        .build()?;
    Ok(Downloads::with_bucket(BucketOptions {
        settings,
        reload_every,
        http: Some(http),
    }))
}

async fn send(
    d: &Downloads,
    verb: Method,
    uri: &str,
    headers: &[(&str, &str)],
) -> anyhow::Result<Response> {
    let mut builder = Request::builder().method(verb).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    Ok(d.serve(builder.body(Body::empty())?).await)
}

async fn get(d: &Downloads, uri: &str) -> anyhow::Result<Response> {
    send(d, Method::GET, uri, &[]).await
}

fn header<'a>(res: &'a Response, name: &str) -> Option<&'a str> {
    res.headers().get(name).and_then(|v| v.to_str().ok())
}

async fn json_body(res: Response) -> anyhow::Result<Value> {
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn served_version(d: &Downloads) -> anyhow::Result<Value> {
    Ok(json_body(get(d, "/download/manifest.json").await?).await?["version"].clone())
}

// ---- Configuration -----------------------------------------------------------------

#[test]
fn all_none_or_some_of_the_variables() -> anyhow::Result<()> {
    let all = [
        (BUCKET_ENV, BUCKET),
        (ENDPOINT_ENV, "https://t3.storageapi.dev"),
        (REGION_ENV, "auto"),
        (ACCESS_KEY_ID_ENV, "AKIDEXAMPLE"),
        (SECRET_ACCESS_KEY_ENV, "very-secret-key"),
    ];
    let lookup = |vars: Vec<(&'static str, &'static str)>| {
        move |name: &str| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| (*v).to_owned())
        }
    };

    assert_eq!(bucket_from_lookup(lookup(Vec::new())), Ok(None));
    assert_eq!(
        bucket_from_lookup(lookup(all.iter().map(|(n, _)| (*n, "")).collect())),
        Ok(None),
        "empty values count as unset"
    );

    let settings = bucket_from_lookup(lookup(all.to_vec()))
        .map_err(anyhow::Error::msg)?
        .ok_or_else(|| anyhow::anyhow!("all five set must give the bucket"))?;
    assert_eq!(settings.bucket(), BUCKET);
    assert_eq!(settings.host(), "examplebucket.t3.storageapi.dev");
    let shown = format!("{settings:?}");
    assert!(!shown.contains("very-secret-key"), "{shown}");
    assert!(!shown.contains("AKIDEXAMPLE"), "{shown}");

    let warning = match bucket_from_lookup(lookup(all[..3].to_vec())) {
        Err(warning) => warning,
        Ok(other) => anyhow::bail!("partly set must warn, got {other:?}"),
    };
    assert!(warning.contains(ACCESS_KEY_ID_ENV), "{warning}");
    assert!(warning.contains(SECRET_ACCESS_KEY_ENV), "{warning}");
    assert!(!warning.contains(REGION_ENV), "{warning}");
    for (_, value) in &all {
        assert!(!warning.contains(value), "a value leaked: {warning}");
    }

    for endpoint in [
        "t3.storageapi.dev",
        "ftp://t3.storageapi.dev",
        "https://t3.storageapi.dev/bucket",
        "https://127.0.0.1",
        "https://user:pw@t3.storageapi.dev",
    ] {
        let mut vars = all.to_vec();
        vars[1] = (ENDPOINT_ENV, endpoint);
        let got = bucket_from_lookup(lookup(vars));
        assert!(
            got.as_ref()
                .is_err_and(|w| w.contains(ENDPOINT_ENV) && !w.contains("pw")),
            "{endpoint}: {got:?}"
        );
    }
    for bucket in ["Upper", "a", "-lead", "has space", "a..b"] {
        let mut vars = all.to_vec();
        vars[0] = (BUCKET_ENV, bucket);
        assert!(bucket_from_lookup(lookup(vars)).is_err(), "{bucket}");
    }
    Ok(())
}

// ---- Manifest ----------------------------------------------------------------------

#[tokio::test]
async fn the_manifest_comes_from_the_bucket_with_a_signed_get() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount_manifest(&server, &manifest("0.1.0")).await;
    let d = downloads(&server, Duration::from_secs(3600))?;
    assert!(d.is_bucket());

    let res = get(&d, "/download/manifest.json").await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header(&res, "content-type"), Some("application/json"));
    assert_eq!(header(&res, "cache-control"), Some("no-cache"));
    let etag = header(&res, "etag")
        .ok_or_else(|| anyhow::anyhow!("no etag"))?
        .to_owned();
    assert_eq!(json_body(res).await?, manifest("0.1.0"));

    let res = send(
        &d,
        Method::GET,
        "/download/manifest.json",
        &[("if-none-match", etag.as_str())],
    )
    .await?;
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(header(&res, "etag"), Some(etag.as_str()));
    assert_eq!(header(&res, "cache-control"), Some("no-cache"));

    let res = send(&d, Method::HEAD, "/download/manifest.json", &[]).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header(&res, "etag"), Some(etag.as_str()));

    // One fetch (boot and the first request share it), presigned virtual-host style.
    let requests = server.received_requests().await.unwrap_or_default();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let request = &requests[0];
    assert_eq!(
        request.headers.get("host").and_then(|v| v.to_str().ok()),
        Some(format!("{BUCKET}.{ENDPOINT_HOST}:{}", server.address().port()).as_str())
    );
    let query = request.url.query().unwrap_or_default();
    assert!(
        query.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"),
        "{query}"
    );
    assert!(query.contains("X-Amz-Credential=AKIDEXAMPLE%2F"), "{query}");
    assert!(query.contains("%2Fauto%2Fs3%2Faws4_request"), "{query}");
    assert!(query.contains("X-Amz-SignedHeaders=host"), "{query}");
    assert!(query.contains("X-Amz-Signature="), "{query}");
    assert!(!query.contains("very-secret-key"), "{query}");
    Ok(())
}

#[tokio::test]
async fn no_manifest_in_the_bucket_is_the_empty_manifest() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    let d = downloads(&server, Duration::from_secs(3600))?;
    let res = get(&d, "/download/manifest.json").await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        json_body(res).await?,
        json!({ "version": null, "platforms": {} })
    );
    for uri in ["/download/mac", &format!("/download/files/{MAC}")] {
        assert_eq!(get(&d, uri).await?.status(), StatusCode::NOT_FOUND, "{uri}");
    }
    Ok(())
}

#[tokio::test]
async fn the_manifest_rules_still_apply() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount_manifest(
        &server,
        &json!({
            "version": "0.1.0",
            "platforms": {
                "mac": { "file": MAC, "bytes": 4096, "sha256": SHA },
                "windows": { "file": "../evil.zip", "bytes": 3000, "sha256": SHA },
                "linux": { "file": "Euchre-0.1.0-linux.AppImage", "bytes": 1, "sha256": "nope" },
                "ios": { "file": "Euchre-0.1.0.ipa", "bytes": 1, "sha256": SHA },
            }
        }),
    )
    .await;
    let d = downloads(&server, Duration::from_secs(3600))?;
    let body = json_body(get(&d, "/download/manifest.json").await?).await?;
    assert_eq!(
        body,
        json!({ "version": "0.1.0", "platforms": { "mac": { "file": MAC, "bytes": 4096, "sha256": SHA } } })
    );
    for uri in ["/download/windows", "/download/linux", "/download/ios"] {
        assert_eq!(get(&d, uri).await?.status(), StatusCode::NOT_FOUND, "{uri}");
    }
    Ok(())
}

// ---- Redirects ---------------------------------------------------------------------

#[tokio::test]
async fn file_routes_redirect_to_presigned_bucket_urls() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount_manifest(&server, &manifest("0.1.0")).await;
    let d = downloads(&server, Duration::from_secs(3600))?;
    let origin = format!(
        "http://{BUCKET}.{ENDPOINT_HOST}:{}",
        server.address().port()
    );

    for uri in ["/download/mac".to_owned(), format!("/download/files/{MAC}")] {
        for verb in [Method::GET, Method::HEAD] {
            let res = send(&d, verb.clone(), &uri, &[]).await?;
            assert_eq!(res.status(), StatusCode::FOUND, "{uri} {verb}");
            assert_eq!(
                header(&res, "cache-control"),
                Some("no-store"),
                "{uri} {verb}"
            );
            let location = header(&res, "location").unwrap_or_default();
            let (base, query) = location.split_once('?').unwrap_or_default();
            assert_eq!(base, format!("{origin}/files/{MAC}"), "{uri} {verb}");
            for part in [
                "X-Amz-Algorithm=AWS4-HMAC-SHA256",
                "X-Amz-Expires=3600",
                "X-Amz-SignedHeaders=host",
                "response-content-disposition=attachment%3B%20filename%3D%22Euchre-0.1.0-mac.dmg%22",
                "response-content-type=application%2Fx-apple-diskimage",
            ] {
                assert!(
                    query.split('&').any(|kv| kv == part),
                    "{part} not in {query}"
                );
            }
            let signature = query
                .rsplit_once("&X-Amz-Signature=")
                .map(|(_, s)| s)
                .unwrap_or_default();
            assert_eq!(signature.len(), 64, "{query}");
            assert!(signature.bytes().all(|b| b.is_ascii_hexdigit()), "{query}");
            assert!(!location.contains("very-secret-key"));
        }
    }

    let res = get(&d, "/download/windows").await?;
    assert!(
        header(&res, "location")
            .unwrap_or_default()
            .contains("response-content-type=application%2Fzip")
    );

    // Only names in the current manifest, and the rest of the tree, stay 404.
    for uri in [
        "/download/linux",
        "/download/ios",
        "/download/files/Euchre-0.0.9-mac.dmg",
        "/download/files/manifest.json",
        "/download/files/",
        "/download/Euchre-0.1.0-mac.dmg",
    ] {
        let res = get(&d, uri).await?;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
    }
    Ok(())
}

// ---- Refresh -----------------------------------------------------------------------

#[tokio::test]
async fn a_failed_refresh_keeps_the_last_good_manifest() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount_manifest(&server, &manifest("0.1.0")).await;
    let d = downloads(&server, Duration::from_secs(3600))?;
    assert_eq!(served_version(&d).await?, json!("0.1.0"));

    // The bucket errors: the last good manifest and its redirects stay.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(
            ResponseTemplate::new(503)
                .set_body_string("<?xml version=\"1.0\"?><Error><Code>SlowDown</Code></Error>"),
        )
        .mount(&server)
        .await;
    d.refresh_bucket().await;
    assert_eq!(served_version(&d).await?, json!("0.1.0"));
    assert_eq!(get(&d, "/download/mac").await?.status(), StatusCode::FOUND);

    // A broken manifest is a failed refresh too.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{not json"))
        .mount(&server)
        .await;
    d.refresh_bucket().await;
    assert_eq!(served_version(&d).await?, json!("0.1.0"));

    // A good manifest replaces it.
    server.reset().await;
    mount_manifest(&server, &manifest("0.2.0")).await;
    d.refresh_bucket().await;
    assert_eq!(served_version(&d).await?, json!("0.2.0"));
    Ok(())
}

#[tokio::test]
async fn an_old_manifest_is_refreshed_in_the_background() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    mount_manifest(&server, &manifest("0.1.0")).await;
    let d = downloads(&server, Duration::ZERO)?;
    assert_eq!(served_version(&d).await?, json!("0.1.0"));

    server.reset().await;
    mount_manifest(&server, &manifest("0.2.0")).await;
    let mut seen = json!(null);
    for _ in 0..200 {
        seen = served_version(&d).await?;
        if seen == json!("0.2.0") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(seen, json!("0.2.0"));
    Ok(())
}
