//! Bucket mode: the release lives in a private S3-compatible bucket (a Railway
//! Storage Bucket), not on disk (`docs/DEPLOY.md` section 9).
//!
//! Layout (fixed; `scripts/upload-downloads.ts` writes it): `manifest.json` at the
//! bucket root, the release files at `files/<file>`. The manifest is checked with
//! the disk rules ([`super::check_manifest`]) and served as in disk mode. A file
//! route answers `302` to a presigned URL for `files/<file>` that expires after
//! [`PRESIGN_EXPIRES_SECS`]; the bucket then serves the bytes, ranges and resumes,
//! so the server's stream caps and byte budget do not apply.
//!
//! The manifest is fetched with a presigned (SigV4) `GET`, once at boot without
//! blocking it, then again about every [`BUCKET_RELOAD_EVERY`]: a request that
//! finds it older starts a fetch in the background and is answered from the last
//! one. A fetch that fails (network, timeout, an HTTP error, or a manifest that is
//! not usable at all) keeps the last good manifest. A `404` means the bucket has
//! no manifest: no downloads are offered.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use euchre_server_core::config::Secret;
use parking_lot::Mutex;

use super::sigv4::{Presign, presign};
use super::{
    Asset, Loaded, MANIFEST_FILE, MAX_MANIFEST_BYTES, Release, check_manifest, content_type,
    not_found,
};

/// Bucket name, for example `euchre-downloads-x1y2z3`.
pub const BUCKET_ENV: &str = "EUCHRE_DOWNLOADS_S3_BUCKET";
/// S3 endpoint, for example `https://t3.storageapi.dev`.
pub const ENDPOINT_ENV: &str = "EUCHRE_DOWNLOADS_S3_ENDPOINT";
/// Signing region, for example `auto`.
pub const REGION_ENV: &str = "EUCHRE_DOWNLOADS_S3_REGION";
/// Access key ID.
pub const ACCESS_KEY_ID_ENV: &str = "EUCHRE_DOWNLOADS_S3_ACCESS_KEY_ID";
/// Secret access key. Never logged.
pub const SECRET_ACCESS_KEY_ENV: &str = "EUCHRE_DOWNLOADS_S3_SECRET_ACCESS_KEY";
/// All five, in the order a warning lists them.
pub const BUCKET_ENV_VARS: [&str; 5] = [
    BUCKET_ENV,
    ENDPOINT_ENV,
    REGION_ENV,
    ACCESS_KEY_ID_ENV,
    SECRET_ACCESS_KEY_ENV,
];

/// Key prefix of the release files in the bucket.
pub const FILES_PREFIX: &str = "files/";
/// How long a fetched manifest is used before the bucket is asked again.
pub const BUCKET_RELOAD_EVERY: Duration = Duration::from_secs(60);
/// Lifetime of a file's presigned URL, in seconds.
pub const PRESIGN_EXPIRES_SECS: u64 = 3600;
/// Lifetime of the presigned URL the server fetches the manifest with.
const MANIFEST_URL_EXPIRES_SECS: u64 = 60;
/// Bound on one manifest fetch.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// `Cache-Control` on a redirect to a presigned URL.
pub const NO_STORE: &str = "no-store";

/// The five bucket variables, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketCredentials {
    /// [`BUCKET_ENV`].
    pub bucket: String,
    /// [`ENDPOINT_ENV`].
    pub endpoint: String,
    /// [`REGION_ENV`].
    pub region: String,
    /// [`ACCESS_KEY_ID_ENV`].
    pub access_key_id: String,
    /// [`SECRET_ACCESS_KEY_ENV`].
    pub secret_access_key: Secret,
}

/// A checked bucket configuration. `Debug` never shows the keys.
#[derive(Clone, PartialEq, Eq)]
pub struct BucketSettings {
    bucket: String,
    scheme: String,
    /// `<bucket>.<endpoint host>[:port]` (virtual-host style).
    host: String,
    region: String,
    access_key_id: String,
    secret_access_key: Secret,
}

impl std::fmt::Debug for BucketSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BucketSettings")
            .field("bucket", &self.bucket)
            .field("host", &self.host)
            .field("region", &self.region)
            .field("access_key_id", &"<redacted>")
            .field("secret_access_key", &"<redacted>")
            .finish()
    }
}

/// One object URL to presign.
#[derive(Clone, Copy)]
pub(crate) struct ObjectUrl<'a> {
    /// `GET` or `HEAD`.
    pub method: &'a str,
    /// Object key.
    pub key: &'a str,
    /// Lifetime in seconds.
    pub expires_secs: u64,
    /// Signing time.
    pub at: SystemTime,
    /// Extra (unencoded) query parameters.
    pub query: &'a [(&'a str, &'a str)],
}

impl BucketSettings {
    /// Check the variables. The error names what is wrong, never a value.
    pub fn new(creds: BucketCredentials) -> Result<Self, String> {
        let bucket = creds.bucket.trim().to_owned();
        if !is_bucket_name(&bucket) {
            return Err(format!(
                "{BUCKET_ENV} is not a bucket name (3 to 63 lowercase letters, digits, '-' or '.')"
            ));
        }
        let bad_endpoint = || {
            format!(
                "{ENDPOINT_ENV} must be an http(s) URL with a host name and no path, \
                 like https://t3.storageapi.dev"
            )
        };
        let endpoint = url::Url::parse(creds.endpoint.trim()).map_err(|_| bad_endpoint())?;
        let Some(url::Host::Domain(domain)) = endpoint.host() else {
            return Err(bad_endpoint());
        };
        if !matches!(endpoint.scheme(), "https" | "http")
            || domain.is_empty()
            || !matches!(endpoint.path(), "" | "/")
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
        {
            return Err(bad_endpoint());
        }
        let host = match endpoint.port() {
            Some(port) => format!("{bucket}.{domain}:{port}"),
            None => format!("{bucket}.{domain}"),
        };
        let region = creds.region.trim().to_owned();
        if region.is_empty()
            || !region
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(format!(
                "{REGION_ENV} must be a region name such as auto or us-east-1"
            ));
        }
        let access_key_id = creds.access_key_id.trim().to_owned();
        if access_key_id.is_empty() || access_key_id.contains('/') {
            return Err(format!("{ACCESS_KEY_ID_ENV} is not an access key ID"));
        }
        let secret = creds.secret_access_key.expose().trim();
        if secret.is_empty() {
            return Err(format!("{SECRET_ACCESS_KEY_ENV} is empty"));
        }
        Ok(Self {
            scheme: endpoint.scheme().to_owned(),
            secret_access_key: Secret::new(secret),
            bucket,
            host,
            region,
            access_key_id,
        })
    }

    /// The bucket name.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// The virtual-host `Host`: `<bucket>.<endpoint host>[:port]`.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// A presigned URL for one object.
    pub(crate) fn presign(&self, object: &ObjectUrl<'_>) -> String {
        presign(&Presign {
            method: object.method,
            scheme: &self.scheme,
            host: &self.host,
            key: object.key,
            region: &self.region,
            access_key_id: &self.access_key_id,
            secret_access_key: self.secret_access_key.expose(),
            at: object.at,
            expires_secs: object.expires_secs,
            query: object.query,
        })
    }
}

/// A DNS-safe bucket name: 3 to 63 of `a-z 0-9 - .`, starting and ending with a
/// letter or digit, with no `..`.
fn is_bucket_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (3..=63).contains(&bytes.len())
        && bytes
            .iter()
            .copied()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.'))
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && !name.contains("..")
}

/// The bucket from the five variables read through `lookup` (an empty value is
/// unset): `Ok(None)` when none is set, `Ok(Some)` when all are and they check
/// out, and otherwise the warning to log, which names variables, never values.
pub fn bucket_from_lookup(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Option<BucketSettings>, String> {
    let values: Vec<Option<String>> = BUCKET_ENV_VARS
        .iter()
        .map(|name| lookup(name).filter(|v| !v.trim().is_empty()))
        .collect();
    let missing: Vec<&str> = BUCKET_ENV_VARS
        .iter()
        .zip(&values)
        .filter(|(_, v)| v.is_none())
        .map(|(name, _)| *name)
        .collect();
    if missing.len() == BUCKET_ENV_VARS.len() {
        return Ok(None);
    }
    if !missing.is_empty() {
        return Err(format!(
            "the bucket settings are only partly set (missing: {}), so no downloads are \
             offered; set all five EUCHRE_DOWNLOADS_S3_* variables, or none to serve the \
             downloads folder",
            missing.join(", ")
        ));
    }
    let mut values = values.into_iter().map(Option::unwrap_or_default);
    let mut next = || values.next().unwrap_or_default();
    let creds = BucketCredentials {
        bucket: next(),
        endpoint: next(),
        region: next(),
        access_key_id: next(),
        secret_access_key: Secret::new(next()),
    };
    BucketSettings::new(creds)
        .map(Some)
        .map_err(|why| format!("{why}, so no downloads are offered"))
}

/// Where the desktop downloads come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadSource {
    /// A folder on disk (dev, tests, a self-hosted server).
    Disk(std::path::PathBuf),
    /// A private S3-compatible bucket (production).
    Bucket(BucketSettings),
    /// None: the empty manifest and `404`s (the bucket settings are broken).
    Off,
}

impl DownloadSource {
    /// From the environment: the bucket when all five `EUCHRE_DOWNLOADS_S3_*`
    /// variables are set, the folder ([`super::dir_from_env`]) when none is, and
    /// [`DownloadSource::Off`] with one warning otherwise.
    pub fn from_env() -> Self {
        match bucket_from_lookup(|name| std::env::var(name).ok()) {
            Ok(Some(bucket)) => {
                tracing::info!(
                    bucket = %bucket.bucket,
                    host = %bucket.host,
                    "desktop downloads: serving from the bucket"
                );
                Self::Bucket(bucket)
            }
            Ok(None) => Self::Disk(super::dir_from_env()),
            Err(warning) => {
                tracing::warn!("desktop downloads: {warning}");
                Self::Off
            }
        }
    }
}

/// How a bucket-mode handler is set up.
#[derive(Debug, Clone)]
pub struct BucketOptions {
    /// The bucket.
    pub settings: BucketSettings,
    /// How long a fetched manifest is used ([`BUCKET_RELOAD_EVERY`]).
    pub reload_every: Duration,
    /// HTTP client for the manifest fetch; `None` builds one with [`FETCH_TIMEOUT`].
    pub http: Option<reqwest::Client>,
}

impl BucketOptions {
    /// The production defaults for `settings`.
    pub fn new(settings: BucketSettings) -> Self {
        Self {
            settings,
            reload_every: BUCKET_RELOAD_EVERY,
            http: None,
        }
    }
}

#[derive(Default)]
struct State {
    /// The last manifest fetched and accepted.
    good: Option<Arc<Release>>,
    /// When the last fetch finished (accepted or not).
    checked: Option<Instant>,
    /// A background fetch is running.
    refreshing: bool,
    /// Warnings already logged.
    warned: BTreeSet<String>,
}

/// The bucket side of [`super::Downloads`].
pub(super) struct BucketSource {
    settings: BucketSettings,
    http: reqwest::Client,
    reload_every: Duration,
    state: Mutex<State>,
    /// One fetch at a time.
    fetch: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for BucketSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BucketSource")
            .field("settings", &self.settings)
            .field("reload_every", &self.reload_every)
            .finish_non_exhaustive()
    }
}

impl BucketSource {
    /// The source, with a first fetch started in the background when a tokio
    /// runtime is running (boot is never blocked on the bucket).
    pub(super) fn start(options: BucketOptions) -> Arc<Self> {
        let http = options.http.unwrap_or_else(|| {
            reqwest::Client::builder()
                .timeout(FETCH_TIMEOUT)
                .connect_timeout(FETCH_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default()
        });
        let source = Arc::new(Self {
            settings: options.settings,
            http,
            reload_every: options.reload_every,
            state: Mutex::new(State::default()),
            fetch: tokio::sync::Mutex::new(()),
        });
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let first = Arc::clone(&source);
            runtime.spawn(async move { first.refresh(true).await });
        }
        source
    }

    /// The current manifest. Only the very first call waits for the bucket.
    pub(super) async fn release(self: &Arc<Self>) -> Arc<Release> {
        {
            let mut state = self.state.lock();
            if let Some(checked) = state.checked {
                if checked.elapsed() >= self.reload_every && !state.refreshing {
                    state.refreshing = true;
                    let again = Arc::clone(self);
                    tokio::spawn(async move { again.refresh(false).await });
                }
                return state.good.clone().unwrap_or_default();
            }
        }
        self.refresh(true).await;
        self.state.lock().good.clone().unwrap_or_default()
    }

    /// Fetch the manifest now (`only_first`: unless a fetch already finished).
    pub(super) async fn refresh(&self, only_first: bool) {
        let _one = self.fetch.lock().await;
        if only_first && self.state.lock().checked.is_some() {
            return;
        }
        let outcome = self.fetch_manifest().await;
        let (new_warnings, accepted, previous, kept) = {
            let mut state = self.state.lock();
            state.checked = Some(Instant::now());
            state.refreshing = false;
            let previous = state.good.clone();
            match outcome {
                Ok(loaded) => {
                    let next: BTreeSet<String> = loaded.warnings.into_iter().collect();
                    let new: Vec<String> = next.difference(&state.warned).cloned().collect();
                    state.warned = next;
                    let release = Arc::new(loaded.release);
                    state.good = Some(Arc::clone(&release));
                    (new, Some(release), previous, None)
                }
                Err(warning) => {
                    let new = if state.warned.insert(warning.clone()) {
                        vec![warning]
                    } else {
                        Vec::new()
                    };
                    let kept = previous
                        .as_ref()
                        .and_then(|r| r.version.clone())
                        .unwrap_or_else(|| "none".to_owned());
                    (new, None, previous, Some(kept))
                }
            }
        };
        for warning in new_warnings {
            match &kept {
                Some(kept) => tracing::warn!(
                    bucket = %self.settings.bucket,
                    kept_version = %kept,
                    "desktop downloads: {warning}; still serving the last manifest read"
                ),
                None => {
                    tracing::warn!(bucket = %self.settings.bucket, "desktop downloads: {warning}")
                }
            }
        }
        if let Some(release) = accepted
            && previous.as_deref() != Some(&*release)
        {
            let platforms: Vec<&str> = release.platforms.iter().map(|(p, _)| p.key()).collect();
            tracing::info!(
                bucket = %self.settings.bucket,
                version = release.version.as_deref().unwrap_or("none"),
                platforms = ?platforms,
                "desktop downloads loaded"
            );
        }
    }

    /// One manifest fetch. `Err` is a failed fetch (the last good manifest stays).
    async fn fetch_manifest(&self) -> Result<Loaded, String> {
        let url = self.settings.presign(&ObjectUrl {
            method: "GET",
            key: MANIFEST_FILE,
            expires_secs: MANIFEST_URL_EXPIRES_SECS,
            at: SystemTime::now(),
            query: &[],
        });
        let failed = |why: String| format!("cannot fetch {MANIFEST_FILE} from the bucket: {why}");
        let mut res = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| failed(describe(e)))?;
        let status = res.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(Loaded::default().warn(format!(
                "{MANIFEST_FILE} is not in the bucket, so no downloads are offered"
            )));
        }
        if !status.is_success() {
            let code = s3_error_code(res).await;
            return Err(failed(format!("HTTP {status}{code}")));
        }
        let too_big = || format!("{MANIFEST_FILE} is over the {MAX_MANIFEST_BYTES}-byte limit");
        if res.content_length().is_some_and(|n| n > MAX_MANIFEST_BYTES) {
            return Err(too_big());
        }
        let mut raw = Vec::new();
        while let Some(chunk) = res.chunk().await.map_err(|e| failed(describe(e)))? {
            raw.extend_from_slice(&chunk);
            if raw.len() as u64 > MAX_MANIFEST_BYTES {
                return Err(too_big());
            }
        }
        check_manifest(&raw)
    }

    /// `302` to a presigned URL for `files/<file>`, signed for the request's
    /// method (`HEAD` stays `HEAD`), never cached.
    pub(super) fn redirect(&self, asset: &Asset, method: &Method) -> Response {
        let method = if *method == Method::HEAD {
            "HEAD"
        } else {
            "GET"
        };
        let key = format!("{FILES_PREFIX}{}", asset.file);
        let disposition = format!("attachment; filename=\"{}\"", asset.file);
        let url = self.settings.presign(&ObjectUrl {
            method,
            key: &key,
            expires_secs: PRESIGN_EXPIRES_SECS,
            at: SystemTime::now(),
            query: &[
                ("response-content-disposition", &disposition),
                ("response-content-type", content_type(&asset.file)),
            ],
        });
        let Ok(location) = HeaderValue::from_str(&url) else {
            return not_found();
        };
        let mut res = StatusCode::FOUND.into_response();
        let headers = res.headers_mut();
        headers.insert(header::LOCATION, location);
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE));
        res
    }
}

/// A transport error without its URL (a presigned URL carries a signature).
fn describe(e: reqwest::Error) -> String {
    let e = e.without_url();
    let mut out = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}

/// ` (<Code>)` from an S3 error body, such as ` (SignatureDoesNotMatch)`, or nothing.
async fn s3_error_code(mut res: reqwest::Response) -> String {
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = res.chunk().await {
        body.extend_from_slice(&chunk);
        if body.len() > 4096 {
            break;
        }
    }
    let body = String::from_utf8_lossy(&body);
    let code = body
        .split_once("<Code>")
        .and_then(|(_, rest)| rest.split_once("</Code>"))
        .map(|(code, _)| code)
        .filter(|code| {
            !code.is_empty() && code.len() <= 64 && code.bytes().all(|b| b.is_ascii_alphanumeric())
        });
    code.map(|code| format!(" ({code})")).unwrap_or_default()
}

#[cfg(test)]
mod tests;
