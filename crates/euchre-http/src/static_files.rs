//! The static web build (`build/client`), TS `server/index.ts` asset branch:
//!
//! - `GET`/`HEAD` only; `HEAD` has no body.
//! - The path is percent-decoded first; a malformed one is `400 Bad path`.
//! - Only files are served, as from TS's `Bun.Glob('**/*')` asset map: never a
//!   dotfile or anything under a dot directory (the glob skips them), never a
//!   `.map` file (any letter case, trailing `/` or not), and never a directory
//!   (no `index.html` append, no redirect: a directory path is an SPA route).
//! - `Cache-Control: public,max-age=31536000,immutable` under `/immutable/`,
//!   `no-cache` for an `.html` file (`/index.html` fetched by name is the same
//!   shell as the SPA fallback, and a stale shell points at deleted chunks),
//!   else `public,max-age=86400`.
//! - `.br` / `.gz` siblings are served to clients that accept them
//!   (`Content-Encoding`, `Vary: Accept-Encoding`).
//! - A path with no file extension that matches no file is an app route and gets
//!   one of two shells with `Cache-Control: no-cache`, so a deploy reaches every
//!   tab (docs/DEPLOY.md, "Two shells"):
//!   - `/` is always the Svelte build's `index.html` (the homepage);
//!   - any other route gets the Rune loader [`RUNE_SHELL`], unless the
//!     [`UI_COOKIE`] starts with `classic`, in which case it gets the Svelte SPA
//!     fallback (`index.html`). The choice carries `Vary: Cookie`.
//!   - With no `rune.html` on disk (a build without Rune) every route gets
//!     `index.html`, as before.
//! - One byte range is served (`206`); any other `Range` header is ignored and
//!   the whole file comes back ([`crate::range`]), so a long multi-range header
//!   costs no CPU.
//! - Every HTML response refuses framing (`Content-Security-Policy:
//!   frame-ancestors 'none'`, `X-Frame-Options: DENY`), so the bid and play
//!   controls cannot be clickjacked. `nosniff` and `Referrer-Policy` are set for
//!   every response by the router ([`crate::api::router`]).

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

use crate::cookies::read_cookie;
use crate::error::text;
use crate::range::keep_single_byte_range;

/// Cache policy for hashed build output.
pub const IMMUTABLE_CACHE: &str = "public,max-age=31536000,immutable";
/// Cache policy for every other asset.
pub const DEFAULT_CACHE: &str = "public,max-age=86400";
/// Cache policy for the SPA fallback document and any `.html` file.
pub const FALLBACK_CACHE: &str = "no-cache";
/// `Content-Security-Policy` on HTML responses: never render inside a frame.
pub const HTML_CSP: &str = "frame-ancestors 'none'";
/// The UI cookie the Rune loader writes when this browser cannot draw the Rune
/// table: `classic-auto` (no usable WebGPU, 7 days), `classic-small` (a small
/// viewport, this session) or `classic` (chosen in Rune Settings, 400 days). Any
/// value starting with `classic` picks the Svelte shell. Not HttpOnly: it is a UI
/// preference the loader writes, not a credential.
pub const UI_COOKIE: &str = "euchre_ui";
/// The Svelte build's page (adapter-static `fallback: 'index.html'`, nothing
/// prerendered): the homepage at `/` and the SPA fallback for classic browsers.
pub const SVELTE_SHELL: &str = "index.html";
/// The Rune loader (`apps/desktop/web/rune.html`, filled by web-build.sh): the SPA
/// fallback for every other browser. Its runtime gates send a browser that cannot
/// draw the table back to [`SVELTE_SHELL`] by setting [`UI_COOKIE`].
pub const RUNE_SHELL: &str = "rune.html";

/// JS `decodeURIComponent`: `None` on a malformed escape or invalid UTF-8.
pub fn decode_uri_component(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Serves one static directory.
#[derive(Debug, Clone)]
pub struct StaticFiles {
    dir: PathBuf,
    /// [`RUNE_SHELL`] was on disk when the server started.
    rune: bool,
}

impl StaticFiles {
    /// Serve `dir`. Whether it holds the Rune loader is read once, here.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let rune = dir.join(RUNE_SHELL).is_file();
        Self { dir, rune }
    }

    /// The served directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Answer one request.
    pub async fn serve(&self, req: Request) -> Response {
        if req.method() != Method::GET && req.method() != Method::HEAD {
            return text(StatusCode::NOT_FOUND, "Not found");
        }
        let Some(decoded) = decode_uri_component(req.uri().path()) else {
            return text(StatusCode::BAD_REQUEST, "Bad path");
        };
        if is_hidden(&decoded) {
            return text(StatusCode::NOT_FOUND, "Not found");
        }
        let is_head = req.method() == Method::HEAD;
        let immutable = decoded.contains("/immutable/");
        let last = decoded.rsplit('/').next().unwrap_or_default();
        let has_extension = last.contains('.');
        let html = last
            .rsplit_once('.')
            .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("html"));
        let (mut parts, _) = req.into_parts();
        keep_single_byte_range(&mut parts.headers, None);
        let probe = Request::from_parts(parts.clone(), Body::empty());

        let serve_dir = ServeDir::new(&self.dir)
            .precompressed_br()
            .precompressed_gzip()
            .append_index_html_on_directories(false);
        let res = match serve_dir.oneshot(probe).await {
            Ok(res) => res.map(Body::new),
            Err(never) => match never {},
        };
        let res = if res.status() == StatusCode::NOT_FOUND && !has_extension {
            // `/` (or only slashes) is the homepage for every browser; any other
            // route is the Rune loader unless this browser is marked classic.
            let home = decoded.trim_matches(['/', '\\']).is_empty();
            let picked = self.rune && !home;
            let classic = picked
                && read_cookie(&parts.headers, UI_COOKIE).is_some_and(|v| v.starts_with("classic"));
            let shell = if picked && !classic {
                RUNE_SHELL
            } else {
                SVELTE_SHELL
            };
            if picked {
                // Both shells answer one URL, so a validator from one must never
                // earn a `304` for the other: a cache that ignores the Vary
                // mismatch would keep the shell it had. Always send the page.
                parts.headers.remove(header::IF_MODIFIED_SINCE);
                parts.headers.remove(header::IF_NONE_MATCH);
            }
            let index = ServeFile::new(self.dir.join(shell))
                .precompressed_br()
                .precompressed_gzip();
            let fallback = Request::from_parts(parts, Body::empty());
            let mut res = match index.oneshot(fallback).await {
                Ok(res) => res.map(Body::new),
                Err(never) => match never {},
            };
            if res.status().is_success() {
                res.headers_mut().insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static(FALLBACK_CACHE),
                );
                if picked {
                    add_vary(res.headers_mut(), "Cookie");
                }
            }
            finish(res, is_head)
        } else {
            let mut res = res;
            if res.status().is_success() || res.status() == StatusCode::NOT_MODIFIED {
                res.headers_mut().insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static(if immutable {
                        IMMUTABLE_CACHE
                    } else if html {
                        FALLBACK_CACHE
                    } else {
                        DEFAULT_CACHE
                    }),
                );
            }
            finish(res, is_head)
        };
        if res.status() == StatusCode::NOT_FOUND {
            return text(StatusCode::NOT_FOUND, "Not found");
        }
        res
    }
}

/// A decoded path TS never served: a segment that starts with `.` (dotfiles,
/// `.git/`, `.env`), or a `.map` file in any letter case, with or without
/// trailing slashes (the static dir may sit on a case-insensitive filesystem, and
/// `ServeDir` drops an empty last segment).
fn is_hidden(decoded: &str) -> bool {
    if decoded
        .split(['/', '\\'])
        .any(|segment| segment.starts_with('.'))
    {
        return true;
    }
    let file = decoded.trim_end_matches(['/', '\\']);
    file.rsplit(['/', '\\'])
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("map"))
}

/// Adds `token` to the response's `Vary`, keeping what is there (one
/// comma-joined value, so `Vary: Cookie, Accept-Encoding`).
fn add_vary(headers: &mut HeaderMap, token: &str) {
    let existing: Vec<String> = headers
        .get_all(header::VARY)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect();
    if existing
        .iter()
        .any(|t| t == "*" || t.eq_ignore_ascii_case(token))
    {
        return;
    }
    let joined = existing
        .iter()
        .map(String::as_str)
        .chain([token])
        .collect::<Vec<_>>()
        .join(", ");
    if let Ok(value) = HeaderValue::from_str(&joined) {
        headers.insert(header::VARY, value);
    }
}

fn finish(mut res: Response, is_head: bool) -> Response {
    if res.headers().contains_key(header::CONTENT_ENCODING) {
        add_vary(res.headers_mut(), "Accept-Encoding");
    }
    let is_html = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim_start().to_ascii_lowercase().starts_with("text/html"));
    if is_html {
        let headers = res.headers_mut();
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(HTML_CSP),
        );
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    if res.status() == StatusCode::NOT_MODIFIED {
        // An empty body of unknown length: axum would add `Content-Length: 0`
        // from an exact size hint, and a 304 must not carry it (RFC 9110 8.6).
        let (parts, _) = res.into_parts();
        let empty = futures::stream::empty::<std::io::Result<axum::body::Bytes>>();
        return Response::from_parts(parts, Body::from_stream(empty));
    }
    if is_head {
        let (parts, _) = res.into_parts();
        return Response::from_parts(parts, Body::empty()).into_response();
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: &str = "0195f0c2-7a3e-7d10-9b1a-2f6c1d8e4a90";

    /// A merged client: the Svelte page and, with `rune`, the Rune loader, each with
    /// a brotli sibling, plus one hashed asset and one Rune asset.
    fn site(rune: bool) -> anyhow::Result<tempfile::TempDir> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        std::fs::create_dir_all(root.join("_app/immutable"))?;
        std::fs::create_dir_all(root.join("_rune/immutable/h1"))?;
        std::fs::create_dir_all(root.join("art"))?;
        std::fs::write(root.join("index.html"), "SVELTE")?;
        std::fs::write(root.join("index.html.br"), "SVELTE-BR")?;
        std::fs::write(root.join("_app/immutable/app.js"), "svelte()")?;
        std::fs::write(root.join("_rune/immutable/h1/app.js"), "rune()")?;
        std::fs::write(root.join("art/card.png"), "PNG")?;
        if rune {
            std::fs::write(root.join(RUNE_SHELL), "RUNE")?;
            std::fs::write(root.join("rune.html.br"), "RUNE-BR")?;
        }
        Ok(dir)
    }

    struct Answer {
        status: StatusCode,
        headers: HeaderMap,
        body: String,
    }

    impl Answer {
        fn header(&self, name: header::HeaderName) -> Option<&str> {
            self.headers.get(name).and_then(|v| v.to_str().ok())
        }
    }

    async fn ask(
        files: &StaticFiles,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
    ) -> anyhow::Result<Answer> {
        let mut req = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        let res = files.serve(req.body(Body::empty())?).await;
        let status = res.status();
        let (parts, body) = res.into_parts();
        let body = axum::body::to_bytes(body, usize::MAX).await?;
        Ok(Answer {
            status,
            headers: parts.headers,
            body: String::from_utf8(body.to_vec())?,
        })
    }

    fn app_routes() -> Vec<String> {
        [
            "/new",
            "/play",
            "/play?stick=true&natural=false",
            "/play/not-a-table",
            "/games",
            "/games/",
            "/settings",
            "/scrapbook",
            "/art",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain([format!("/play/{GAME}"), format!("/games/{GAME}")])
        .collect()
    }

    #[tokio::test]
    async fn app_routes_get_the_rune_loader_with_no_ui_cookie() -> anyhow::Result<()> {
        let dir = site(true)?;
        let files = StaticFiles::new(dir.path());
        for path in app_routes() {
            for cookie in [None, Some("euchre_ui=rune"), Some("other=classic")] {
                let headers: Vec<_> = cookie.map(|c| ("cookie", c)).into_iter().collect();
                let res = ask(&files, Method::GET, &path, &headers).await?;
                assert_eq!(res.status, StatusCode::OK, "{path} {cookie:?}");
                assert_eq!(res.body, "RUNE", "{path} {cookie:?}");
                assert_eq!(res.header(header::VARY), Some("Cookie"), "{path}");
                assert_eq!(res.header(header::CACHE_CONTROL), Some(FALLBACK_CACHE));
                assert_eq!(res.header(header::CONTENT_SECURITY_POLICY), Some(HTML_CSP));
                assert_eq!(res.header(header::X_FRAME_OPTIONS), Some("DENY"));
                assert!(
                    res.header(header::CONTENT_TYPE)
                        .is_some_and(|t| t.starts_with("text/html")),
                    "{path}"
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_classic_cookie_gets_the_svelte_page_on_app_routes() -> anyhow::Result<()> {
        let dir = site(true)?;
        let files = StaticFiles::new(dir.path());
        for path in app_routes() {
            for cookie in [
                "euchre_ui=classic",
                "euchre_ui=classic-auto",
                "euchre_ui=classic-small",
                "a=1; euchre_ui=classic-auto; b=2",
            ] {
                let res = ask(&files, Method::GET, &path, &[("cookie", cookie)]).await?;
                assert_eq!(res.status, StatusCode::OK, "{path} {cookie}");
                assert_eq!(res.body, "SVELTE", "{path} {cookie}");
                assert_eq!(res.header(header::VARY), Some("Cookie"), "{path}");
                assert_eq!(res.header(header::CACHE_CONTROL), Some(FALLBACK_CACHE));
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn the_home_page_is_svelte_for_every_browser() -> anyhow::Result<()> {
        let dir = site(true)?;
        let files = StaticFiles::new(dir.path());
        for path in ["/", "/index.html"] {
            for cookie in [None, Some("euchre_ui=classic")] {
                let headers: Vec<_> = cookie.map(|c| ("cookie", c)).into_iter().collect();
                let res = ask(&files, Method::GET, path, &headers).await?;
                assert_eq!(res.status, StatusCode::OK, "{path} {cookie:?}");
                assert_eq!(res.body, "SVELTE", "{path} {cookie:?}");
                assert_eq!(res.header(header::VARY), None, "{path}");
                assert_eq!(res.header(header::CACHE_CONTROL), Some(FALLBACK_CACHE));
                assert_eq!(res.header(header::CONTENT_SECURITY_POLICY), Some(HTML_CSP));
            }
        }
        // The loader by name is a plain file: no cookie choice.
        let res = ask(&files, Method::GET, "/rune.html", &[]).await?;
        assert_eq!(res.body, "RUNE");
        assert_eq!(res.header(header::VARY), None);
        assert_eq!(res.header(header::CACHE_CONTROL), Some(FALLBACK_CACHE));
        Ok(())
    }

    #[tokio::test]
    async fn without_the_loader_every_route_is_svelte() -> anyhow::Result<()> {
        let dir = site(false)?;
        let files = StaticFiles::new(dir.path());
        for path in app_routes() {
            for cookie in [None, Some("euchre_ui=classic")] {
                let headers: Vec<_> = cookie.map(|c| ("cookie", c)).into_iter().collect();
                let res = ask(&files, Method::GET, &path, &headers).await?;
                assert_eq!(res.status, StatusCode::OK, "{path} {cookie:?}");
                assert_eq!(res.body, "SVELTE", "{path} {cookie:?}");
                assert_eq!(res.header(header::VARY), None, "{path}");
                assert_eq!(res.header(header::CACHE_CONTROL), Some(FALLBACK_CACHE));
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn the_shells_vary_on_cookie_and_accept_encoding() -> anyhow::Result<()> {
        let dir = site(true)?;
        let files = StaticFiles::new(dir.path());
        let br = ("accept-encoding", "br, gzip");
        let res = ask(&files, Method::GET, "/games", &[br]).await?;
        assert_eq!(res.body, "RUNE-BR");
        assert_eq!(res.header(header::CONTENT_ENCODING), Some("br"));
        assert_eq!(res.header(header::VARY), Some("Cookie, Accept-Encoding"));
        assert_eq!(res.headers.get_all(header::VARY).iter().count(), 1);

        let classic = ("cookie", "euchre_ui=classic");
        let res = ask(&files, Method::GET, "/games", &[br, classic]).await?;
        assert_eq!(res.body, "SVELTE-BR");
        assert_eq!(res.header(header::VARY), Some("Cookie, Accept-Encoding"));

        let res = ask(&files, Method::GET, "/", &[br]).await?;
        assert_eq!(res.body, "SVELTE-BR");
        assert_eq!(res.header(header::VARY), Some("Accept-Encoding"));
        Ok(())
    }

    #[tokio::test]
    async fn head_has_the_get_headers_and_no_body() -> anyhow::Result<()> {
        let dir = site(true)?;
        let files = StaticFiles::new(dir.path());
        for headers in [
            vec![("accept-encoding", "br")],
            vec![
                ("accept-encoding", "br"),
                ("cookie", "euchre_ui=classic-auto"),
            ],
            vec![],
        ] {
            let get = ask(&files, Method::GET, "/play/abc", &headers).await?;
            let head = ask(&files, Method::HEAD, "/play/abc", &headers).await?;
            assert_eq!(head.status, StatusCode::OK);
            assert_eq!(head.body, "");
            for name in [
                header::VARY,
                header::CACHE_CONTROL,
                header::CONTENT_TYPE,
                header::CONTENT_ENCODING,
                header::CONTENT_LENGTH,
                header::CONTENT_SECURITY_POLICY,
            ] {
                assert_eq!(
                    head.header(name.clone()),
                    get.header(name.clone()),
                    "{name}"
                );
            }
        }
        Ok(())
    }

    /// A validator taken from one shell never earns a `304` for the other.
    #[tokio::test]
    async fn the_shells_ignore_validators() -> anyhow::Result<()> {
        let dir = site(true)?;
        let files = StaticFiles::new(dir.path());
        let future = ("if-modified-since", "Fri, 01 Jan 2100 00:00:00 GMT");
        for cookie in ["euchre_ui=classic", "x=y"] {
            let res = ask(&files, Method::GET, "/games", &[future, ("cookie", cookie)]).await?;
            assert_eq!(res.status, StatusCode::OK, "{cookie}");
        }
        // Files by name keep their conditional answers.
        let res = ask(&files, Method::GET, "/index.html", &[future]).await?;
        assert_eq!(res.status, StatusCode::NOT_MODIFIED);
        Ok(())
    }

    #[tokio::test]
    async fn assets_and_misses_are_unchanged_by_the_shells() -> anyhow::Result<()> {
        let dir = site(true)?;
        let files = StaticFiles::new(dir.path());
        for (path, body) in [
            ("/_app/immutable/app.js", "svelte()"),
            ("/_rune/immutable/h1/app.js", "rune()"),
        ] {
            let res = ask(
                &files,
                Method::GET,
                path,
                &[("cookie", "euchre_ui=classic")],
            )
            .await?;
            assert_eq!(res.body, body);
            assert_eq!(res.header(header::CACHE_CONTROL), Some(IMMUTABLE_CACHE));
            assert_eq!(res.header(header::VARY), None);
        }
        let res = ask(&files, Method::GET, "/art/card.png", &[]).await?;
        assert_eq!(res.header(header::CACHE_CONTROL), Some(DEFAULT_CACHE));
        for path in [
            "/missing.js",
            "/play/art/cards/AS.png",
            "/_rune/immutable/h1/app.js.map",
        ] {
            let res = ask(&files, Method::GET, path, &[]).await?;
            assert_eq!(
                (res.status, res.body.as_str()),
                (StatusCode::NOT_FOUND, "Not found"),
                "{path}"
            );
        }
        Ok(())
    }

    #[test]
    fn add_vary_keeps_what_is_there() {
        let mut headers = HeaderMap::new();
        add_vary(&mut headers, "Accept-Encoding");
        assert_eq!(
            headers.get(header::VARY).map(|v| v.as_bytes()),
            Some(&b"Accept-Encoding"[..])
        );
        add_vary(&mut headers, "accept-encoding");
        add_vary(&mut headers, "Cookie");
        assert_eq!(
            headers.get(header::VARY).map(|v| v.as_bytes()),
            Some(&b"Accept-Encoding, Cookie"[..])
        );
        let mut star = HeaderMap::new();
        star.insert(header::VARY, HeaderValue::from_static("*"));
        add_vary(&mut star, "Cookie");
        assert_eq!(
            star.get(header::VARY).map(|v| v.as_bytes()),
            Some(&b"*"[..])
        );
    }

    #[test]
    fn decode_uri_component_matches_js() {
        assert_eq!(decode_uri_component("/a%20b").as_deref(), Some("/a b"));
        assert_eq!(
            decode_uri_component("/%E2%80%99").as_deref(),
            Some("/\u{2019}")
        );
        assert_eq!(decode_uri_component("/%"), None);
        assert_eq!(decode_uri_component("/%zz"), None);
        assert_eq!(decode_uri_component("/%C3"), None);
    }

    #[test]
    fn hidden_paths_are_dotfiles_and_maps() {
        for path in [
            "/.env",
            "/.git/config",
            "/_app/.vite/manifest.json",
            "/a/../b",
            "/app.js.map",
            "/app.js.MAP",
            "/app.js.Map",
            "/app.js.map/",
            "/app.js.map//",
            "/dir\\.env",
        ] {
            assert!(is_hidden(path), "{path}");
        }
        for path in [
            "/",
            "/index.html",
            "/_app/immutable/app.js",
            "/play/abc",
            "/map",
            "/a.mapx",
        ] {
            assert!(!is_hidden(path), "{path}");
        }
    }
}
