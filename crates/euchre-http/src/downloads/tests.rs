//! The download contract against real folders (`tempfile`), through
//! [`Downloads::serve`]. Router-level checks (Host gate, security headers, `405`)
//! live in `api/tests.rs`.

use std::path::Path;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;

const VERSION: &str = "0.1.0";
const MAC: &str = "Euchre-0.1.0-mac.dmg";
const WINDOWS: &str = "Euchre-0.1.0-windows.zip";
const LINUX: &str = "Euchre-0.1.0-linux.AppImage";

fn sha(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

/// Distinct, recognisable bytes for each file.
fn payload(tag: &str, len: usize) -> Vec<u8> {
    tag.bytes().cycle().take(len).collect()
}

fn entry(file: &str, data: &[u8]) -> Value {
    json!({ "file": file, "bytes": data.len(), "sha256": sha(data) })
}

struct Folder {
    dir: tempfile::TempDir,
    mac: Vec<u8>,
    windows: Vec<u8>,
    linux: Vec<u8>,
}

impl Folder {
    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, name: &str, data: &[u8]) -> anyhow::Result<()> {
        std::fs::write(self.path().join(name), data)?;
        Ok(())
    }

    fn manifest(&self, value: &Value) -> anyhow::Result<()> {
        self.write(MANIFEST_FILE, value.to_string().as_bytes())
    }

    fn downloads(&self) -> Downloads {
        Downloads::with_reload_every(Some(self.path().to_path_buf()), Duration::ZERO)
    }
}

/// A folder with all three files and a correct manifest.
fn full_folder() -> anyhow::Result<Folder> {
    let folder = Folder {
        dir: tempfile::tempdir()?,
        mac: payload("mac-", 4096),
        windows: payload("windows-", 3000),
        linux: payload("linux-", 5000),
    };
    folder.write(MAC, &folder.mac)?;
    folder.write(WINDOWS, &folder.windows)?;
    folder.write(LINUX, &folder.linux)?;
    folder.manifest(&json!({
        "version": VERSION,
        "platforms": {
            "mac": entry(MAC, &folder.mac),
            "windows": entry(WINDOWS, &folder.windows),
            "linux": entry(LINUX, &folder.linux),
        }
    }))?;
    Ok(folder)
}

fn request(method: Method, uri: &str, headers: &[(&str, &str)]) -> anyhow::Result<Request<Body>> {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    Ok(builder.body(Body::empty())?)
}

async fn get(d: &Downloads, uri: &str) -> anyhow::Result<Response> {
    Ok(d.serve(request(Method::GET, uri, &[])?).await)
}

async fn get_with(d: &Downloads, uri: &str, headers: &[(&str, &str)]) -> anyhow::Result<Response> {
    Ok(d.serve(request(Method::GET, uri, headers)?).await)
}

fn header<'a>(res: &'a Response, name: &str) -> Option<&'a str> {
    res.headers().get(name).and_then(|v| v.to_str().ok())
}

async fn body(res: Response) -> anyhow::Result<Vec<u8>> {
    Ok(axum::body::to_bytes(res.into_body(), usize::MAX)
        .await?
        .to_vec())
}

async fn json_body(res: Response) -> anyhow::Result<Value> {
    Ok(serde_json::from_slice(&body(res).await?)?)
}

// ---- Manifest ----------------------------------------------------------------------

#[tokio::test]
async fn manifest_lists_every_checked_platform() -> anyhow::Result<()> {
    let f = full_folder()?;
    let res = get(&f.downloads(), "/download/manifest.json").await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header(&res, "content-type"), Some("application/json"));
    assert_eq!(header(&res, "cache-control"), Some("no-cache"));
    assert_eq!(
        json_body(res).await?,
        json!({
            "version": VERSION,
            "platforms": {
                "mac": { "file": MAC, "bytes": 4096, "sha256": sha(&f.mac) },
                "windows": { "file": WINDOWS, "bytes": 3000, "sha256": sha(&f.windows) },
                "linux": { "file": LINUX, "bytes": 5000, "sha256": sha(&f.linux) },
            }
        })
    );
    let loaded = load(f.path()).await;
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    Ok(())
}

#[tokio::test]
async fn manifest_head_has_headers_and_no_body() -> anyhow::Result<()> {
    let f = full_folder()?;
    let res = f
        .downloads()
        .serve(request(Method::HEAD, "/download/manifest.json", &[])?)
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header(&res, "cache-control"), Some("no-cache"));
    assert_eq!(header(&res, "content-type"), Some("application/json"));
    Ok(())
}

#[tokio::test]
async fn missing_folder_or_manifest_is_the_empty_manifest_without_warnings() -> anyhow::Result<()> {
    let empty = json!({ "version": null, "platforms": {} });

    let none = Downloads::new(None);
    let res = get(&none, "/download/manifest.json").await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header(&res, "cache-control"), Some("no-cache"));
    assert_eq!(json_body(res).await?, empty);

    let dir = tempfile::tempdir()?;
    let gone = dir.path().join("not-here");
    let d = Downloads::with_reload_every(Some(gone.clone()), Duration::ZERO);
    assert_eq!(
        json_body(get(&d, "/download/manifest.json").await?).await?,
        empty
    );
    assert_eq!(load(&gone).await, Loaded::default());

    // A folder with files but no manifest.json.
    std::fs::write(dir.path().join(MAC), b"bytes")?;
    let d = Downloads::with_reload_every(Some(dir.path().to_path_buf()), Duration::ZERO);
    assert_eq!(
        json_body(get(&d, "/download/manifest.json").await?).await?,
        empty
    );
    assert_eq!(load(dir.path()).await, Loaded::default());
    for platform in Platform::ALL {
        let res = get(&d, &format!("/download/{}", platform.key())).await?;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
    let res = get(&d, &format!("/download/files/{MAC}")).await?;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn a_file_instead_of_a_folder_warns_and_serves_nothing() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("downloads");
    std::fs::write(&file, b"not a folder")?;
    let loaded = load(&file).await;
    assert_eq!(loaded.release, Release::default());
    assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
    assert!(loaded.warnings[0].contains("not a directory"));
    Ok(())
}

#[tokio::test]
async fn bad_json_is_the_empty_manifest_and_one_warning() -> anyhow::Result<()> {
    let f = full_folder()?;
    let cases: [(&[u8], &str); 7] = [
        (b"{not json", "not valid JSON"),
        (b"", "not valid JSON"),
        (b"[1,2,3]", "not a JSON object"),
        (
            br#"{"version":1,"platforms":{}}"#,
            "\"version\" must be a string",
        ),
        (br#"{"platforms":{}}"#, "\"version\" must be a string"),
        (br#"{"version":"v1","platforms":{}}"#, "not a semver"),
        (
            br#"{"version":"0.1.0","platforms":[]}"#,
            "\"platforms\" must be an object",
        ),
    ];
    for (raw, expected) in cases {
        f.write(MANIFEST_FILE, raw)?;
        let loaded = load(f.path()).await;
        assert_eq!(loaded.release, Release::default(), "{expected}");
        assert_eq!(
            loaded.warnings.len(),
            1,
            "{expected}: {:?}",
            loaded.warnings
        );
        assert!(
            loaded.warnings[0].contains(expected),
            "{expected}: {:?}",
            loaded.warnings
        );
        let res = get(&f.downloads(), "/download/manifest.json").await?;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            json_body(res).await?,
            json!({ "version": null, "platforms": {} })
        );
        let res = get(&f.downloads(), "/download/mac").await?;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
    Ok(())
}

#[tokio::test]
async fn an_oversized_manifest_is_refused() -> anyhow::Result<()> {
    let f = full_folder()?;
    let mut big = format!(r#"{{"version":"{VERSION}","platforms":{{}},"pad":""#).into_bytes();
    big.resize(usize::try_from(MAX_MANIFEST_BYTES)? + 10, b'x');
    big.extend_from_slice(b"\"}");
    f.write(MANIFEST_FILE, &big)?;
    let loaded = load(f.path()).await;
    assert_eq!(loaded.release, Release::default());
    assert_eq!(loaded.warnings.len(), 1);
    assert!(
        loaded.warnings[0].contains("limit"),
        "{:?}",
        loaded.warnings
    );
    Ok(())
}

#[tokio::test]
async fn bad_entries_drop_only_their_platform() -> anyhow::Result<()> {
    let f = full_folder()?;
    let cases = [
        (json!("Euchre.dmg"), "not an object"),
        (
            json!({ "bytes": 4096, "sha256": sha(&f.mac) }),
            "\"file\" must be a string",
        ),
        (
            json!({ "file": MAC, "bytes": "4096", "sha256": sha(&f.mac) }),
            "\"bytes\" must be a whole number",
        ),
        (
            json!({ "file": MAC, "bytes": -1, "sha256": sha(&f.mac) }),
            "\"bytes\" must be a whole number",
        ),
        (
            json!({ "file": MAC, "bytes": 0, "sha256": sha(&f.mac) }),
            "\"bytes\" is 0",
        ),
        (
            json!({ "file": MAC, "bytes": 4096 }),
            "\"sha256\" must be a string",
        ),
        (
            json!({ "file": MAC, "bytes": 4096, "sha256": "abc" }),
            "not 64 hex digits",
        ),
        (
            json!({ "file": MAC, "bytes": 4096, "sha256": "z".repeat(64) }),
            "not 64 hex digits",
        ),
        (
            json!({ "file": "Euchre-9.9.9-mac.dmg", "bytes": 4096, "sha256": sha(&f.mac) }),
            "not in the downloads folder",
        ),
    ];
    for (mac, expected) in cases {
        f.manifest(&json!({
            "version": VERSION,
            "platforms": { "mac": mac, "linux": entry(LINUX, &f.linux) }
        }))?;
        let loaded = load(f.path()).await;
        let keys: Vec<Platform> = loaded.release.platforms.iter().map(|(p, _)| *p).collect();
        assert_eq!(keys, vec![Platform::Linux], "{expected}");
        assert_eq!(
            loaded.warnings.len(),
            1,
            "{expected}: {:?}",
            loaded.warnings
        );
        assert!(
            loaded.warnings[0].contains("mac left out") && loaded.warnings[0].contains(expected),
            "{expected}: {:?}",
            loaded.warnings
        );
        let d = f.downloads();
        assert_eq!(
            get(&d, "/download/mac").await?.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get(&d, "/download/linux").await?.status(),
            StatusCode::FOUND
        );
    }
    Ok(())
}

#[tokio::test]
async fn unknown_platform_keys_warn_and_are_ignored() -> anyhow::Result<()> {
    let f = full_folder()?;
    f.manifest(&json!({
        "version": VERSION,
        "platforms": { "mac": entry(MAC, &f.mac), "ios": entry(MAC, &f.mac), "Mac": entry(MAC, &f.mac) }
    }))?;
    let loaded = load(f.path()).await;
    assert_eq!(loaded.release.platforms.len(), 1);
    assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
    assert!(
        loaded
            .warnings
            .iter()
            .all(|w| w.contains("unknown platform"))
    );
    let body = json_body(get(&f.downloads(), "/download/manifest.json").await?).await?;
    assert_eq!(body["platforms"].as_object().map(Map::len), Some(1));
    Ok(())
}

#[tokio::test]
async fn sha256_is_served_lowercase() -> anyhow::Result<()> {
    let f = full_folder()?;
    f.manifest(&json!({
        "version": VERSION,
        "platforms": { "mac": { "file": MAC, "bytes": 4096, "sha256": sha(&f.mac).to_uppercase() } }
    }))?;
    let body = json_body(get(&f.downloads(), "/download/manifest.json").await?).await?;
    assert_eq!(body["platforms"]["mac"]["sha256"], json!(sha(&f.mac)));
    Ok(())
}

#[tokio::test]
async fn a_size_mismatch_drops_the_platform() -> anyhow::Result<()> {
    let f = full_folder()?;
    f.manifest(&json!({
        "version": VERSION,
        "platforms": {
            "mac": { "file": MAC, "bytes": 4097, "sha256": sha(&f.mac) },
            "windows": entry(WINDOWS, &f.windows),
        }
    }))?;
    let loaded = load(f.path()).await;
    assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
    assert!(loaded.warnings[0].contains("4096 bytes on disk but the manifest says 4097"));
    let d = f.downloads();
    let body = json_body(get(&d, "/download/manifest.json").await?).await?;
    assert_eq!(
        body["platforms"]
            .as_object()
            .map(|m| m.keys().cloned().collect::<Vec<_>>()),
        Some(vec!["windows".to_owned()])
    );
    assert_eq!(
        get(&d, "/download/mac").await?.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&d, &format!("/download/files/{MAC}")).await?.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&d, "/download/windows").await?.status(),
        StatusCode::FOUND
    );
    Ok(())
}

// ---- Redirects and 404s ------------------------------------------------------------

#[tokio::test]
async fn each_platform_redirects_to_its_file() -> anyhow::Result<()> {
    let f = full_folder()?;
    let d = f.downloads();
    for (platform, file) in [("mac", MAC), ("windows", WINDOWS), ("linux", LINUX)] {
        for method in [Method::GET, Method::HEAD] {
            let res = d
                .serve(request(
                    method.clone(),
                    &format!("/download/{platform}"),
                    &[],
                )?)
                .await;
            assert_eq!(res.status(), StatusCode::FOUND, "{platform} {method}");
            assert_eq!(
                header(&res, "location"),
                Some(format!("/download/files/{file}").as_str())
            );
            assert_eq!(header(&res, "cache-control"), Some("no-cache"));
        }
    }
    Ok(())
}

#[tokio::test]
async fn everything_else_under_download_is_not_found() -> anyhow::Result<()> {
    let f = full_folder()?;
    // On disk but not listed: never served.
    f.write("Euchre-0.0.9-mac.dmg", b"old")?;
    f.write("notes.txt", b"secret")?;
    let d = f.downloads();
    for uri in [
        "/download/",
        "/download/ios",
        "/download/Mac",
        "/download/MAC",
        "/download/mac/",
        "/download/files",
        "/download/files/",
        "/download/files/Euchre-0.0.9-mac.dmg",
        "/download/files/notes.txt",
        "/download/files/manifest.json",
        "/download/files/euchre-0.1.0-mac.dmg",
        "/download/files/Euchre-0.1.0-mac.dmg/",
        "/download/files/x/Euchre-0.1.0-mac.dmg",
        "/download/manifest.json/",
        "/download/manifest.JSON",
        "/download/Euchre-0.1.0-mac.dmg",
        "/downloads/mac",
    ] {
        let res = get(&d, uri).await?;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(
            header(&res, "content-type"),
            Some("text/plain;charset=UTF-8")
        );
    }
    Ok(())
}

// ---- Files -------------------------------------------------------------------------

#[tokio::test]
async fn a_full_get_streams_the_file_with_download_headers() -> anyhow::Result<()> {
    let f = full_folder()?;
    // A compressed sibling must never be picked, whatever the client accepts.
    f.write(&format!("{MAC}.gz"), b"GZIP")?;
    f.write(&format!("{MAC}.br"), b"BROTLI")?;
    let d = f.downloads();
    for (file, data, kind) in [
        (MAC, &f.mac, "application/x-apple-diskimage"),
        (WINDOWS, &f.windows, "application/zip"),
        (LINUX, &f.linux, "application/octet-stream"),
    ] {
        let res = get_with(
            &d,
            &format!("/download/files/{file}"),
            &[("accept-encoding", "br, gzip, deflate")],
        )
        .await?;
        assert_eq!(res.status(), StatusCode::OK, "{file}");
        assert_eq!(header(&res, "content-type"), Some(kind));
        assert_eq!(
            header(&res, "content-disposition"),
            Some(format!("attachment; filename=\"{file}\"").as_str())
        );
        assert_eq!(
            header(&res, "cache-control"),
            Some("public, max-age=31536000, immutable")
        );
        assert_eq!(
            header(&res, "etag"),
            Some(format!("\"{}\"", sha(data)).as_str())
        );
        assert_eq!(header(&res, "accept-ranges"), Some("bytes"));
        assert_eq!(
            header(&res, "content-length"),
            Some(data.len().to_string().as_str())
        );
        assert_eq!(header(&res, "content-encoding"), None);
        assert!(header(&res, "last-modified").is_some());
        assert_eq!(&body(res).await?, data);
    }
    Ok(())
}

#[tokio::test]
async fn head_sends_the_headers_without_the_body() -> anyhow::Result<()> {
    let f = full_folder()?;
    let res = f
        .downloads()
        .serve(request(
            Method::HEAD,
            &format!("/download/files/{MAC}"),
            &[],
        )?)
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header(&res, "content-length"), Some("4096"));
    assert_eq!(
        header(&res, "content-type"),
        Some("application/x-apple-diskimage")
    );
    assert_eq!(
        header(&res, "content-disposition"),
        Some(format!("attachment; filename=\"{MAC}\"").as_str())
    );
    assert!(header(&res, "etag").is_some());
    assert!(body(res).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn ranges_give_206_and_unsatisfiable_ones_416() -> anyhow::Result<()> {
    let f = full_folder()?;
    let d = f.downloads();
    let uri = format!("/download/files/{MAC}");

    let res = get_with(&d, &uri, &[("range", "bytes=0-9")]).await?;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(header(&res, "content-range"), Some("bytes 0-9/4096"));
    assert_eq!(header(&res, "content-length"), Some("10"));
    assert_eq!(
        header(&res, "content-disposition"),
        Some(format!("attachment; filename=\"{MAC}\"").as_str())
    );
    assert!(header(&res, "etag").is_some());
    assert_eq!(body(res).await?, f.mac[0..10]);

    let res = get_with(&d, &uri, &[("range", "bytes=4000-")]).await?;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(header(&res, "content-range"), Some("bytes 4000-4095/4096"));
    assert_eq!(body(res).await?, f.mac[4000..]);

    let res = get_with(&d, &uri, &[("range", "bytes=-6")]).await?;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(body(res).await?, f.mac[4090..]);

    // The unit is case-insensitive (RFC 9110 14.1).
    let res = get_with(&d, &uri, &[("range", "BYTES=0-1")]).await?;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(header(&res, "content-range"), Some("bytes 0-1/4096"));

    // A suffix longer than the file is the whole file (RFC 9110 14.1.3).
    let res = get_with(&d, &uri, &[("range", "bytes=-1000000")]).await?;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(header(&res, "content-range"), Some("bytes 0-4095/4096"));
    assert_eq!(body(res).await?, f.mac);

    for range in ["bytes=4096-", "bytes=9000-9999", "bytes=5-2", "bytes=-0"] {
        let res = get_with(&d, &uri, &[("range", range)]).await?;
        assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE, "{range}");
        assert_eq!(
            header(&res, "content-range"),
            Some("bytes */4096"),
            "{range}"
        );
        assert_eq!(header(&res, "content-disposition"), None, "{range}");
        assert_eq!(header(&res, "cache-control"), Some("no-cache"), "{range}");
    }

    // Other units, lists of ranges and malformed values are ignored: the whole
    // file, 200 (RFC 9110 14.2).
    for range in [
        "lines=0-1",
        "items=0-1",
        "bytes=0-1,5-6",
        "bytes=0-1,3-4",
        "bytes=abc",
    ] {
        let res = get_with(&d, &uri, &[("range", range)]).await?;
        assert_eq!(res.status(), StatusCode::OK, "{range}");
        assert_eq!(header(&res, "content-range"), None, "{range}");
        assert_eq!(body(res).await?, f.mac, "{range}");
    }
    Ok(())
}

/// The quadratic multi-range check in `http-range-header` is never reached: a
/// 30 KB list of ranges (under Railway's 32 KB header cap) is ignored at once.
#[tokio::test]
async fn a_huge_multi_range_header_costs_nothing() -> anyhow::Result<()> {
    let f = full_folder()?;
    let d = f.downloads();
    let uri = format!("/download/files/{MAC}");
    let huge = std::iter::once("bytes=0-0".to_owned())
        .chain((1..3200).map(|i| format!("{}-{}", i * 2, i * 2)))
        .collect::<Vec<_>>()
        .join(",");
    assert!(huge.len() > 30_000 && huge.len() < 32 * 1024);
    let started = Instant::now();
    for _ in 0..20 {
        let res = get_with(&d, &uri, &[("range", huge.as_str())]).await?;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body(res).await?, f.mac);
    }
    let per_request = started.elapsed() / 20;
    assert!(
        per_request < Duration::from_millis(20),
        "{per_request:?} per request"
    );
    Ok(())
}

#[tokio::test]
async fn etag_conditionals() -> anyhow::Result<()> {
    let f = full_folder()?;
    let d = f.downloads();
    let uri = format!("/download/files/{MAC}");
    let etag = format!("\"{}\"", sha(&f.mac));

    for inm in [
        etag.clone(),
        format!("W/{etag}"),
        format!("\"other\", {etag}"),
        "*".to_owned(),
    ] {
        for method in [Method::GET, Method::HEAD] {
            let res = d
                .serve(request(
                    method.clone(),
                    &uri,
                    &[("if-none-match", inm.as_str())],
                )?)
                .await;
            assert_eq!(res.status(), StatusCode::NOT_MODIFIED, "{method} {inm}");
            assert_eq!(header(&res, "etag"), Some(etag.as_str()));
            assert_eq!(
                header(&res, "cache-control"),
                Some("public, max-age=31536000, immutable")
            );
            assert_eq!(header(&res, "content-length"), None, "{method} {inm}");
            // No exact size, so axum adds no `Content-Length: 0` (RFC 9110 8.6).
            assert_eq!(
                axum::body::HttpBody::size_hint(res.body()).exact(),
                None,
                "{method} {inm}"
            );
            assert!(body(res).await?.is_empty());
        }
    }

    // A different tag wins over a matching If-Modified-Since.
    let res = get_with(
        &d,
        &uri,
        &[
            ("if-none-match", "\"stale\""),
            ("if-modified-since", "Fri, 01 Jan 2100 00:00:00 GMT"),
        ],
    )
    .await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body(res).await?, f.mac);

    // If-Range: our tag keeps the range; anything else sends the whole file.
    let res = get_with(
        &d,
        &uri,
        &[("range", "bytes=0-3"), ("if-range", etag.as_str())],
    )
    .await?;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    let weak = format!("W/{etag}");
    for if_range in ["\"stale\"", "Fri, 01 Jan 2100 00:00:00 GMT", weak.as_str()] {
        let res = get_with(&d, &uri, &[("range", "bytes=0-3"), ("if-range", if_range)]).await?;
        assert_eq!(res.status(), StatusCode::OK, "{if_range}");
        assert_eq!(body(res).await?, f.mac, "{if_range}");
    }
    Ok(())
}

// ---- Names, traversal and symlinks -------------------------------------------------

#[test]
fn plain_file_names() {
    for ok in [
        MAC,
        WINDOWS,
        LINUX,
        "Euchre-1.2.3-beta.1+b5-linux.tar.gz",
        "a",
        "a.b_c-d+e",
    ] {
        assert!(is_plain_file_name(ok), "{ok}");
    }
    let long = "a".repeat(MAX_FILE_NAME_BYTES + 1);
    for bad in [
        "",
        ".",
        "..",
        "../x.dmg",
        "a/b.dmg",
        "/abs.dmg",
        "a\\b.dmg",
        "a..b.dmg",
        ".hidden.dmg",
        "x\".dmg",
        "x'.dmg",
        "x\0.dmg",
        "x\n.dmg",
        "x\r\nSet-Cookie: a=b",
        "x\t.dmg",
        "x;.dmg",
        "x,.dmg",
        "x .dmg",
        "x%2f.dmg",
        "x?.dmg",
        "x#.dmg",
        "ü.dmg",
        "manifest.json",
        long.as_str(),
    ] {
        assert!(!is_plain_file_name(bad), "{bad:?}");
    }
}

#[test]
fn versions_and_content_types() {
    for ok in [
        "0.1.0",
        "10.20.30",
        "1.2.3-beta.1",
        "1.2.3+build.5",
        "1.2.3-rc.1+b",
    ] {
        assert!(is_version(ok), "{ok}");
    }
    for bad in [
        "",
        "1",
        "1.2",
        "v1.2.3",
        "1.2.3 ",
        "1.2.3-",
        "1.2.3\n",
        "1.2.3-\"x\"",
    ] {
        assert!(!is_version(bad), "{bad:?}");
    }
    assert!(!is_version(&format!("1.2.3-{}", "a".repeat(64))));
    assert_eq!(content_type("A.dmg"), "application/x-apple-diskimage");
    assert_eq!(content_type("A.DMG"), "application/x-apple-diskimage");
    assert_eq!(content_type("A.zip"), "application/zip");
    assert_eq!(content_type("A.AppImage"), "application/octet-stream");
    assert_eq!(content_type("A.tar.gz"), "application/gzip");
    assert_eq!(content_type("A.tgz"), "application/gzip");
    assert_eq!(
        content_type("A.exe"),
        "application/vnd.microsoft.portable-executable"
    );
    assert_eq!(content_type("A.msi"), "application/octet-stream");
}

#[tokio::test]
async fn unsafe_names_in_the_manifest_are_never_served() -> anyhow::Result<()> {
    let f = full_folder()?;
    for name in [
        "../euchre-dl-outside.txt",
        "..",
        ".",
        "sub/x.dmg",
        "x\\y.dmg",
        "x\".dmg",
        "x\r\nX-Evil: 1",
        "x\0.dmg",
        "x .dmg",
        ".hidden",
        "manifest.json",
    ] {
        f.manifest(&json!({
            "version": VERSION,
            "platforms": { "mac": { "file": name, "bytes": 7, "sha256": sha(b"outside") } }
        }))?;
        let loaded = load(f.path()).await;
        assert!(loaded.release.platforms.is_empty(), "{name:?}");
        assert_eq!(loaded.warnings.len(), 1, "{name:?}");
        assert!(
            loaded.warnings[0].contains("not a plain file name"),
            "{name:?}: {:?}",
            loaded.warnings
        );
        assert_eq!(
            get(&f.downloads(), "/download/mac").await?.status(),
            StatusCode::NOT_FOUND
        );
    }
    Ok(())
}

#[tokio::test]
async fn traversal_in_the_url_is_not_found() -> anyhow::Result<()> {
    let f = full_folder()?;
    f.write("notes.txt", b"secret")?;
    let d = f.downloads();
    for uri in [
        "/download/files/../manifest.json",
        "/download/files/..%2Fmanifest.json",
        "/download/files/%2e%2e%2fmanifest.json",
        "/download/files/..%5Cnotes.txt",
        "/download/files/%2E%2E/notes.txt",
        "/download/files/.%2Fnotes.txt",
        "/download/files/notes.txt%00.dmg",
        "/download/files/Euchre-0.1.0-mac.dmg%00",
        "/download/files/Euchre-0.1.0-mac.dmg%0d%0aX-Evil:1",
        "/download/files/%2Fetc%2Fpasswd",
        "/download/files//etc/passwd",
        "/download/..%2Fdownload%2Fmac",
    ] {
        let res = get(&d, uri).await?;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
        assert!(header(&res, "content-disposition").is_none(), "{uri}");
    }
    let res = get(&d, "/download/files/%zz").await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    // Percent-encoding a plain name still finds it.
    let res = get(&d, "/download/files/Euchre-0.1.0-mac%2Edmg").await?;
    assert_eq!(res.status(), StatusCode::OK);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_must_stay_inside_the_folder() -> anyhow::Result<()> {
    use std::os::unix::fs::symlink;

    let base = tempfile::tempdir()?;
    let root = base.path().join("downloads");
    std::fs::create_dir_all(root.join("builds"))?;
    let outside = base.path().join("outside.dmg");
    let secret = payload("secret-", 64);
    std::fs::write(&outside, &secret)?;
    let inside = payload("inside-", 64);
    std::fs::write(root.join("builds/real.zip"), &inside)?;

    // Escapes: a relative and an absolute link out of the folder.
    symlink("../outside.dmg", root.join("Euchre-0.1.0-mac.dmg"))?;
    symlink(&outside, root.join("Euchre-0.1.0-linux.AppImage"))?;
    // Stays inside: a link into a subfolder.
    symlink("builds/real.zip", root.join("Euchre-0.1.0-windows.zip"))?;
    std::fs::write(
        root.join(MANIFEST_FILE),
        json!({
            "version": VERSION,
            "platforms": {
                "mac": entry("Euchre-0.1.0-mac.dmg", &secret),
                "linux": entry("Euchre-0.1.0-linux.AppImage", &secret),
                "windows": entry("Euchre-0.1.0-windows.zip", &inside),
            }
        })
        .to_string(),
    )?;
    let loaded = load(&root).await;
    let keys: Vec<Platform> = loaded.release.platforms.iter().map(|(p, _)| *p).collect();
    assert_eq!(keys, vec![Platform::Windows]);
    assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
    assert!(
        loaded
            .warnings
            .iter()
            .all(|w| w.contains("points outside the downloads folder")),
        "{:?}",
        loaded.warnings
    );
    let d = Downloads::with_reload_every(Some(root.clone()), Duration::ZERO);
    assert_eq!(
        get(&d, "/download/files/Euchre-0.1.0-mac.dmg")
            .await?
            .status(),
        StatusCode::NOT_FOUND
    );
    let res = get(&d, "/download/files/Euchre-0.1.0-windows.zip").await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body(res).await?, inside);

    // A manifest.json that is itself a link out of the folder is ignored.
    let elsewhere = base.path().join("manifest.json");
    std::fs::rename(root.join(MANIFEST_FILE), &elsewhere)?;
    symlink(&elsewhere, root.join(MANIFEST_FILE))?;
    let loaded = load(&root).await;
    assert_eq!(loaded.release, Release::default());
    assert_eq!(loaded.warnings.len(), 1);
    assert!(loaded.warnings[0].contains("points outside"));

    // A listed name that is a directory is not a file.
    std::fs::remove_file(root.join(MANIFEST_FILE))?;
    std::fs::create_dir(root.join("Euchre-0.2.0-mac.dmg"))?;
    std::fs::write(
        root.join(MANIFEST_FILE),
        json!({
            "version": VERSION,
            "platforms": { "mac": { "file": "Euchre-0.2.0-mac.dmg", "bytes": 1, "sha256": sha(b"x") } }
        })
        .to_string(),
    )?;
    let loaded = load(&root).await;
    assert!(loaded.release.platforms.is_empty());
    assert!(
        loaded.warnings[0].contains("not a regular file"),
        "{:?}",
        loaded.warnings
    );
    Ok(())
}

// ---- Cache -------------------------------------------------------------------------

#[test]
fn each_warning_is_logged_once_until_it_goes_away() {
    let mut cache = Cache::default();
    let at = Instant::now();
    let with = |warnings: &[&str]| Loaded {
        release: Release::default(),
        warnings: warnings.iter().map(|w| (*w).to_owned()).collect(),
    };
    assert_eq!(
        cache.store(with(&["bad json"]), at).1,
        vec!["bad json".to_owned()]
    );
    assert!(cache.store(with(&["bad json"]), at).1.is_empty());
    assert_eq!(
        cache.store(with(&["bad json", "mac size"]), at).1,
        vec!["mac size".to_owned()]
    );
    assert!(cache.store(with(&[]), at).1.is_empty());
    assert_eq!(
        cache.store(with(&["bad json"]), at).1,
        vec!["bad json".to_owned()]
    );
}

#[tokio::test]
async fn the_manifest_is_reused_within_the_window_and_read_again_after() -> anyhow::Result<()> {
    let f = full_folder()?;
    let cached =
        Downloads::with_reload_every(Some(f.path().to_path_buf()), Duration::from_secs(3600));
    let fresh = f.downloads();
    assert_eq!(cached.release().await.platforms.len(), 3);
    assert_eq!(fresh.release().await.platforms.len(), 3);
    std::fs::remove_file(f.path().join(LINUX))?;
    assert_eq!(cached.release().await.platforms.len(), 3);
    assert_eq!(fresh.release().await.platforms.len(), 2);
    // A file that vanished after the read is a 404, never a 500.
    let res = get(&cached, &format!("/download/files/{LINUX}")).await?;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    Ok(())
}

// ---- Versioned names ---------------------------------------------------------------

#[tokio::test]
async fn a_name_without_the_version_is_served_with_a_warning() -> anyhow::Result<()> {
    let f = full_folder()?;
    f.write("Euchre-mac.dmg", &f.mac)?;
    f.manifest(&json!({
        "version": VERSION,
        "platforms": {
            "mac": entry("Euchre-mac.dmg", &f.mac),
            "windows": entry(WINDOWS, &f.windows),
        }
    }))?;
    let loaded = load(f.path()).await;
    assert_eq!(loaded.release.platforms.len(), 2);
    assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
    assert!(
        loaded.warnings[0].contains("does not contain the version 0.1.0"),
        "{:?}",
        loaded.warnings
    );
    Ok(())
}

// ---- Stream limits -----------------------------------------------------------------

fn limited(f: &Folder, streams: StreamLimits, budget_mib: BucketConfig) -> Downloads {
    Downloads::with_options(DownloadOptions {
        dir: Some(f.path().to_path_buf()),
        reload_every: Duration::ZERO,
        budget_mib,
        streams,
    })
}

fn ip(last: u8) -> IpAddr {
    IpAddr::from([203, 0, 113, last])
}

async fn get_from(d: &Downloads, uri: &str, client: IpAddr) -> anyhow::Result<Response> {
    Ok(d.serve_from(request(Method::GET, uri, &[])?, client).await)
}

#[tokio::test]
async fn each_client_network_gets_a_few_streams_at_once() -> anyhow::Result<()> {
    let f = full_folder()?;
    let d = limited(
        &f,
        StreamLimits {
            max_streams: 100,
            max_streams_per_client: 2,
            idle_timeout: None,
        },
        BucketConfig::UNLIMITED,
    );
    let uri = format!("/download/files/{MAC}");
    let first = get_from(&d, &uri, ip(1)).await?;
    let second = get_from(&d, &uri, ip(1)).await?;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);

    let third = get_from(&d, &uri, ip(1)).await?;
    assert_eq!(third.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(header(&third, "retry-after"), Some("30"));
    assert_eq!(header(&third, "cache-control"), Some("no-store"));

    // Another network is not affected, and HEAD is never counted.
    assert_eq!(get_from(&d, &uri, ip(2)).await?.status(), StatusCode::OK);
    let head = d.serve_from(request(Method::HEAD, &uri, &[])?, ip(1)).await;
    assert_eq!(head.status(), StatusCode::OK);

    // A finished stream gives its slot back.
    assert_eq!(body(first).await?, f.mac);
    assert_eq!(get_from(&d, &uri, ip(1)).await?.status(), StatusCode::OK);
    // So does a dropped one (the connection closed).
    drop(second);
    assert_eq!(get_from(&d, &uri, ip(1)).await?.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn past_the_global_cap_downloads_get_503() -> anyhow::Result<()> {
    let f = full_folder()?;
    let d = limited(
        &f,
        StreamLimits {
            max_streams: 1,
            max_streams_per_client: 8,
            idle_timeout: None,
        },
        BucketConfig::UNLIMITED,
    );
    let uri = format!("/download/files/{MAC}");
    let held = get_from(&d, &uri, ip(1)).await?;
    assert_eq!(held.status(), StatusCode::OK);

    let res = get_from(&d, &uri, ip(2)).await?;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(header(&res, "retry-after"), Some("30"));
    assert_eq!(header(&res, "cache-control"), Some("no-store"));
    // The refused client's own slot was given back.
    drop(res);

    // The manifest, redirects, HEAD, 304 and 416 need no stream.
    assert_eq!(
        get(&d, "/download/manifest.json").await?.status(),
        StatusCode::OK
    );
    assert_eq!(get(&d, "/download/mac").await?.status(), StatusCode::FOUND);
    let head = d.serve_from(request(Method::HEAD, &uri, &[])?, ip(2)).await;
    assert_eq!(head.status(), StatusCode::OK);
    let etag = format!("\"{}\"", sha(&f.mac));
    let res = get_with(&d, &uri, &[("if-none-match", etag.as_str())]).await?;
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);

    drop(held);
    assert_eq!(get_from(&d, &uri, ip(2)).await?.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn a_stream_whose_client_stops_reading_is_closed() -> anyhow::Result<()> {
    let f = full_folder()?;
    let idle = Duration::from_millis(150);
    let d = limited(
        &f,
        StreamLimits {
            max_streams: 1,
            max_streams_per_client: 1,
            idle_timeout: Some(idle),
        },
        BucketConfig::UNLIMITED,
    );
    let uri = format!("/download/files/{MAC}");
    let stalled = get_from(&d, &uri, ip(1)).await?;
    assert_eq!(stalled.status(), StatusCode::OK);
    assert_eq!(
        get_from(&d, &uri, ip(2)).await?.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );

    tokio::time::sleep(idle * 3).await;
    // The global slot is back for everyone else...
    let other = get_from(&d, &uri, ip(2)).await?;
    assert_eq!(other.status(), StatusCode::OK);
    assert_eq!(body(other).await?, f.mac);
    // ...but the stalled network keeps its own slot until its connection goes.
    assert_eq!(
        get_from(&d, &uri, ip(1)).await?.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // The stalled body ends with an error, so the server closes the connection.
    assert!(
        axum::body::to_bytes(stalled.into_body(), usize::MAX)
            .await
            .is_err()
    );
    assert_eq!(get_from(&d, &uri, ip(1)).await?.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn a_slow_reader_is_not_cut() -> anyhow::Result<()> {
    let f = full_folder()?;
    // 16 chunks of 64 KiB, read one every 40 ms: 640 ms in all, idle 150 ms.
    let big = payload("slow-", 1024 * 1024);
    let name = "Euchre-0.1.0-big.AppImage";
    f.write(name, &big)?;
    f.manifest(&json!({ "version": VERSION, "platforms": { "linux": entry(name, &big) } }))?;
    let d = limited(
        &f,
        StreamLimits {
            max_streams: 1,
            max_streams_per_client: 1,
            idle_timeout: Some(Duration::from_millis(150)),
        },
        BucketConfig::UNLIMITED,
    );
    let res = get_from(&d, &format!("/download/files/{name}"), ip(1)).await?;
    assert_eq!(res.status(), StatusCode::OK);
    let mut stream = res.into_body().into_data_stream();
    let mut got = Vec::new();
    while let Some(chunk) = stream.next().await {
        got.extend_from_slice(&chunk?);
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    assert_eq!(got, big);
    Ok(())
}

#[tokio::test]
async fn each_client_network_has_a_byte_budget() -> anyhow::Result<()> {
    let f = full_folder()?;
    // Two MiB at once, one more each minute. Each small file costs one MiB.
    let d = limited(&f, StreamLimits::default(), BucketConfig::new(2, 1));
    let uri = format!("/download/files/{MAC}");
    for _ in 0..2 {
        let res = get_from(&d, &uri, ip(1)).await?;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body(res).await?, f.mac);
    }
    let res = get_from(&d, &uri, ip(1)).await?;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = header(&res, "retry-after").unwrap_or_default().parse()?;
    assert!((1..=61).contains(&wait), "{wait}");
    assert_eq!(header(&res, "cache-control"), Some("no-store"));
    // HEAD costs nothing; another network has its own budget.
    let head = d.serve_from(request(Method::HEAD, &uri, &[])?, ip(1)).await;
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(get_from(&d, &uri, ip(2)).await?.status(), StatusCode::OK);
    // One IPv6 /64 is one network.
    let a: IpAddr = "2001:db8:1:2::1".parse()?;
    let b: IpAddr = "2001:db8:1:2::ffff".parse()?;
    assert_eq!(get_from(&d, &uri, a).await?.status(), StatusCode::OK);
    assert_eq!(get_from(&d, &uri, b).await?.status(), StatusCode::OK);
    assert_eq!(
        get_from(&d, &uri, a).await?.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    Ok(())
}

#[tokio::test]
async fn a_range_costs_its_own_length() -> anyhow::Result<()> {
    let f = full_folder()?;
    let big = payload("big-", 3 * 1024 * 1024);
    let name = "Euchre-0.1.0-big.AppImage";
    f.write(name, &big)?;
    f.manifest(&json!({ "version": VERSION, "platforms": { "linux": entry(name, &big) } }))?;
    // Four MiB: the whole 3 MiB file fits once, then only 1 MiB ranges.
    let d = limited(&f, StreamLimits::default(), BucketConfig::new(4, 1));
    let uri = format!("/download/files/{name}");
    let res = get_from(&d, &uri, ip(1)).await?;
    assert_eq!(res.status(), StatusCode::OK);
    drop(res);
    let range = d
        .serve_from(
            request(Method::GET, &uri, &[("range", "bytes=0-1048575")])?,
            ip(1),
        )
        .await;
    assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
    drop(range);
    assert_eq!(
        get_from(&d, &uri, ip(1)).await?.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    Ok(())
}
