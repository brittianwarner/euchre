//! Desktop downloads: the shared download contract (`docs/DEPLOY.md` section 9).
//!
//! Two sources ([`DownloadSource`]): a private S3-compatible bucket when all five
//! `EUCHRE_DOWNLOADS_S3_*` variables are set (production; [`bucket`]), else a
//! folder on disk (dev, tests). This page describes disk mode; bucket mode keeps
//! the manifest rules and the manifest route, and answers every file route with
//! a `302` to a presigned bucket URL.
//!
//! The downloads folder ([`DOWNLOADS_DIR_ENV`], default [`DEFAULT_DOWNLOADS_DIR`]
//! relative to the working directory; the image has none) holds the
//! release files and `manifest.json`:
//!
//! ```json
//! {"version":"0.1.0","platforms":{"mac":{"file":"Euchre-0.1.0-mac.dmg","bytes":31457280,"sha256":"<64 hex>"}}}
//! ```
//!
//! Platform keys are `mac`, `windows` and `linux`; any of them may be absent.
//!
//! - `GET`/`HEAD /download/manifest.json`: `200` JSON in the same shape, listing
//!   only the platforms whose file passed every check below. `Cache-Control:
//!   no-cache`, a strong `ETag` (the body's sha256) and `304` on a matching
//!   `If-None-Match`. No folder or no manifest: `{"version":null,"platforms":{}}`.
//! - `GET`/`HEAD /download/{mac|windows|linux}`: `302` to
//!   `/download/files/<file>` with `Cache-Control: no-cache`, or `404` when that
//!   platform is absent.
//! - `GET`/`HEAD /download/files/<file>`: the file, streamed from disk by
//!   `tower-http`'s `ServeFile`, with `Content-Disposition: attachment;
//!   filename="<file>"`, a strong `ETag` (the manifest's sha256), `If-None-Match`
//!   and `If-Range` handling, and [`FILE_CACHE`]. One byte range gets `206` (or
//!   `416` when it is past the end); any other `Range` is ignored and the whole
//!   file comes back ([`crate::range`]). Only names in the checked manifest are
//!   served; anything else under `/download/` is `404`.
//!
//! A listed file is served only when its name is a plain file name
//! ([`is_plain_file_name`]: letters, digits, `.`, `_`, `-`, `+`), its sha256 is 64
//! hex digits, it resolves (after symlinks) to a regular file inside the folder,
//! and its size on disk equals `bytes`. A platform that fails a check is left out
//! and logged once as a warning. Bad JSON gives the empty manifest and one
//! warning, never a `500`.
//!
//! A listed name that does not contain the manifest's version is still served,
//! with one warning: files are cached for a year, so a reused name would keep
//! old bytes in browsers.
//!
//! The file streams are bounded ([`StreamLimits`]): at most [`MAX_STREAMS`] at
//! once (`503` past it), at most [`MAX_STREAMS_PER_CLIENT`] per client network
//! (`429`), each client network may fetch about 1 GiB an hour (the
//! [`crate::RateLimitConfig::download`] bucket, in MiB; `429` past it), and a
//! stream whose client reads nothing for [`IDLE_TIMEOUT`] is closed. Every
//! refusal carries `Retry-After`.
//!
//! The manifest is read on request and kept for [`RELOAD_EVERY`], so a changed
//! folder shows up within seconds without a restart. Files are never compressed
//! (`ServeFile` without precompressed variants, and no compression layer).
//! The router's front door (Host check) and security headers apply as for every
//! other route.

use std::collections::{BTreeSet, HashMap};
use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::body::{Body, BodyDataStream, Bytes};
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::{Stream, StreamExt};
use parking_lot::Mutex;
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::config::{BucketConfig, RateLimitConfig};
use crate::error::text;
use crate::range::keep_single_byte_range;
use crate::rate_limit::{RateLimiter, RouteClass, UNKNOWN_CLIENT, client_key};
use crate::static_files::decode_uri_component;

pub mod bucket;
mod sigv4;

use bucket::BucketSource;
pub use bucket::{
    BucketCredentials, BucketOptions, BucketSettings, DownloadSource, bucket_from_lookup,
};

/// Environment variable that names the downloads folder.
pub const DOWNLOADS_DIR_ENV: &str = "EUCHRE_DOWNLOADS_DIR";
/// The downloads folder when [`DOWNLOADS_DIR_ENV`] is unset or empty, relative to
/// the server's working directory.
pub const DEFAULT_DOWNLOADS_DIR: &str = "downloads";
/// The manifest's file name inside the downloads folder.
pub const MANIFEST_FILE: &str = "manifest.json";
/// `Cache-Control` on a served file: names carry the version, so a file never changes.
pub const FILE_CACHE: &str = "public, max-age=31536000, immutable";
/// `Cache-Control` on the manifest and the per-platform redirects.
pub const NO_CACHE: &str = "no-cache";
/// How long a loaded manifest is reused before the folder is read again.
pub const RELOAD_EVERY: Duration = Duration::from_secs(5);
/// A larger `manifest.json` is refused (it holds three short entries).
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// Longest accepted file name, in bytes.
pub const MAX_FILE_NAME_BYTES: usize = 128;
/// File streams at once, all clients together. Each costs about 140 KB of
/// memory and two file handles while it runs.
pub const MAX_STREAMS: usize = 256;
/// File streams at once from one client network ([`client_key`]: an IPv4
/// address or an IPv6 `/64`).
pub const MAX_STREAMS_PER_CLIENT: usize = 8;
/// A stream whose client has read nothing for this long is closed. A client that
/// reads at least one 64 KiB chunk a minute (about 1 KB/s) is never cut.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// `Retry-After` on a `503` or a per-client `429`, in seconds.
pub const RETRY_AFTER_SECS: u64 = 30;
/// One MiB: the unit of the per-client byte budget.
pub const MIB: u64 = 1024 * 1024;

/// The downloads folder from [`DOWNLOADS_DIR_ENV`], else [`DEFAULT_DOWNLOADS_DIR`].
pub fn dir_from_env() -> PathBuf {
    match std::env::var_os(DOWNLOADS_DIR_ENV) {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(DEFAULT_DOWNLOADS_DIR),
    }
}

/// A desktop platform, in manifest order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Platform {
    /// `mac`
    Mac,
    /// `windows`
    Windows,
    /// `linux`
    Linux,
}

impl Platform {
    /// Every platform, in manifest order.
    pub const ALL: [Self; 3] = [Self::Mac, Self::Windows, Self::Linux];

    /// The manifest key and URL segment.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Mac => "mac",
            Self::Windows => "windows",
            Self::Linux => "linux",
        }
    }

    /// The platform for an exact manifest key or URL segment.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }
}

/// One checked release file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// Plain file name, as listed.
    pub file: String,
    /// Size in bytes, equal to the file on disk.
    pub bytes: u64,
    /// Lowercase hex sha256, as listed.
    pub sha256: String,
    /// The file after symlinks, inside the downloads folder (empty in bucket mode).
    path: PathBuf,
}

impl Asset {
    /// The strong entity tag sent as `ETag`.
    pub fn etag(&self) -> String {
        format!("\"{}\"", self.sha256)
    }
}

/// The checked manifest: only the platforms whose file passed every check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Release {
    /// The manifest's version, `None` when there is no usable manifest.
    pub version: Option<String>,
    /// Present platforms, in [`Platform::ALL`] order.
    pub platforms: Vec<(Platform, Asset)>,
}

impl Release {
    /// The asset for `platform`, when present.
    pub fn asset(&self, platform: Platform) -> Option<&Asset> {
        self.platforms
            .iter()
            .find(|(p, _)| *p == platform)
            .map(|(_, a)| a)
    }

    /// The asset whose file name is exactly `file`.
    pub fn by_file(&self, file: &str) -> Option<&Asset> {
        self.platforms
            .iter()
            .map(|(_, a)| a)
            .find(|a| a.file == file)
    }

    /// The `GET /download/manifest.json` body.
    pub fn to_json(&self) -> Value {
        let mut platforms = Map::new();
        for (platform, asset) in &self.platforms {
            platforms.insert(
                platform.key().to_owned(),
                json!({ "file": asset.file, "bytes": asset.bytes, "sha256": asset.sha256 }),
            );
        }
        json!({ "version": self.version, "platforms": platforms })
    }
}

/// A folder read: the checked release and every problem found, in plain words.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    /// What may be served.
    pub release: Release,
    /// One line per problem (each becomes one logged warning).
    pub warnings: Vec<String>,
}

impl Loaded {
    fn warn(mut self, message: String) -> Self {
        self.warnings.push(message);
        self
    }

    /// Add one platform's outcome: the asset (with a warning when its name lacks
    /// the version) or the warning that left it out.
    fn add(&mut self, platform: Platform, asset: Result<Asset, String>) {
        match asset {
            Ok(asset) => {
                if let Some(version) = &self.release.version
                    && !asset.file.contains(version.as_str())
                {
                    self.warnings.push(format!(
                        "{MANIFEST_FILE}: {} file {} does not contain the version {version}; \
                         files are cached for a year, so every release needs new file names",
                        platform.key(),
                        asset.file
                    ));
                }
                self.release.platforms.push((platform, asset));
            }
            Err(warning) => self.warnings.push(warning),
        }
    }
}

/// A single plain file name: 1 to [`MAX_FILE_NAME_BYTES`] bytes of ASCII
/// letters, digits, `.`, `_`, `-` and `+`, not starting with `.`, with no `..`,
/// and not [`MANIFEST_FILE`]. That rules out `/`, `\`, NUL, control characters,
/// quotes, spaces and `;`, so the name is safe in a path, in `Location` and in
/// `Content-Disposition`.
pub fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_FILE_NAME_BYTES
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+'))
        && !name.starts_with('.')
        && !name.contains("..")
        && name != MANIFEST_FILE
}

static VERSION: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$").ok()
});

/// A semver string such as `0.1.0`, `1.2.3-beta.1` or `1.2.3+build.5`, at most 64 bytes.
pub fn is_version(version: &str) -> bool {
    version.len() <= 64
        && VERSION
            .as_ref()
            .is_some_and(|re: &Regex| re.is_match(version))
}

fn is_sha256(hex: &str) -> bool {
    hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `Content-Type` for a release file, by extension (any letter case).
pub fn content_type(file: &str) -> &'static str {
    let lower = file.to_ascii_lowercase();
    if lower.ends_with(".dmg") {
        "application/x-apple-diskimage"
    } else if lower.ends_with(".zip") {
        "application/zip"
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        "application/gzip"
    } else if lower.ends_with(".exe") {
        "application/vnd.microsoft.portable-executable"
    } else {
        // `.AppImage` and anything else.
        "application/octet-stream"
    }
}

/// A short, quoted rendering of manifest text for a log line.
fn shown(raw: &str) -> String {
    let cut: String = raw.chars().take(64).collect();
    if cut.len() < raw.len() {
        format!("{cut:?}...")
    } else {
        format!("{cut:?}")
    }
}

enum Resolved {
    /// The file after symlinks, and its metadata.
    File(PathBuf, std::fs::Metadata),
    Missing,
    Outside,
    NotAFile,
    Unreadable(io::Error),
}

/// `root.join(name)` after symlinks, only when it lands inside `root`.
async fn resolve_inside(root: &Path, name: &str) -> Resolved {
    let path = match tokio::fs::canonicalize(root.join(name)).await {
        Ok(path) => path,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Resolved::Missing,
        Err(e) => return Resolved::Unreadable(e),
    };
    if path == root || !path.starts_with(root) {
        return Resolved::Outside;
    }
    match tokio::fs::metadata(&path).await {
        Ok(meta) if meta.is_file() => Resolved::File(path, meta),
        Ok(_) => Resolved::NotAFile,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Resolved::Missing,
        Err(e) => Resolved::Unreadable(e),
    }
}

/// Read and check the downloads folder `dir`. Never fails: every problem becomes
/// a warning and leaves the affected part out. No folder, or no manifest in it,
/// is the normal "nothing to download" state and gives no warning.
pub async fn load(dir: &Path) -> Loaded {
    let empty = Loaded::default();
    let root = match tokio::fs::canonicalize(dir).await {
        Ok(root) => root,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return empty,
        Err(e) => {
            return empty.warn(format!(
                "cannot open the downloads folder {}: {e}",
                dir.display()
            ));
        }
    };
    match tokio::fs::metadata(&root).await {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            return empty.warn(format!(
                "the downloads folder {} is not a directory",
                dir.display()
            ));
        }
        Err(e) => {
            return empty.warn(format!(
                "cannot open the downloads folder {}: {e}",
                dir.display()
            ));
        }
    }

    let (manifest_path, manifest_meta) = match resolve_inside(&root, MANIFEST_FILE).await {
        Resolved::File(path, meta) => (path, meta),
        Resolved::Missing => return empty,
        Resolved::Outside => {
            return empty.warn(format!(
                "{MANIFEST_FILE} points outside the downloads folder; ignoring it"
            ));
        }
        Resolved::NotAFile => {
            return empty.warn(format!("{MANIFEST_FILE} is not a regular file"));
        }
        Resolved::Unreadable(e) => return empty.warn(format!("cannot read {MANIFEST_FILE}: {e}")),
    };
    if manifest_meta.len() > MAX_MANIFEST_BYTES {
        return empty.warn(format!(
            "{MANIFEST_FILE} is {} bytes, over the {MAX_MANIFEST_BYTES}-byte limit",
            manifest_meta.len()
        ));
    }
    let raw = match read_capped(&manifest_path).await {
        Ok(Some(raw)) => raw,
        Ok(None) => {
            return empty.warn(format!(
                "{MANIFEST_FILE} grew past the {MAX_MANIFEST_BYTES}-byte limit while being read"
            ));
        }
        Err(e) => return empty.warn(format!("cannot read {MANIFEST_FILE}: {e}")),
    };
    let listing = match parse_listing(&raw) {
        Ok(listing) => listing,
        Err(warning) => return empty.warn(warning),
    };
    let mut loaded = listing.loaded();
    for (platform, entry) in listing.entries {
        let asset = match entry {
            Ok(entry) => on_disk(&root, platform, entry).await,
            Err(warning) => Err(warning),
        };
        loaded.add(platform, asset);
    }
    loaded
}

/// One platform entry that passed the manifest rules (storage not checked yet).
struct Entry {
    file: String,
    bytes: u64,
    sha256: String,
}

/// A manifest that passed the whole-file rules.
struct Listing {
    version: String,
    /// Unknown platform keys.
    warnings: Vec<String>,
    /// Each listed platform, in [`Platform::ALL`] order: its entry or why it is left out.
    entries: Vec<(Platform, Result<Entry, String>)>,
}

impl Listing {
    /// The [`Loaded`] to add the platforms to.
    fn loaded(&self) -> Loaded {
        Loaded {
            release: Release {
                version: Some(self.version.clone()),
                platforms: Vec::new(),
            },
            warnings: self.warnings.clone(),
        }
    }
}

/// The manifest rules on `raw` JSON. `Err` rejects the whole manifest.
fn parse_listing(raw: &[u8]) -> Result<Listing, String> {
    let value: Value = serde_json::from_slice(raw)
        .map_err(|e| format!("{MANIFEST_FILE} is not valid JSON: {e}"))?;
    let Some(top) = value.as_object() else {
        return Err(format!("{MANIFEST_FILE} is not a JSON object"));
    };
    let version = match top.get("version") {
        Some(Value::String(v)) if is_version(v) => v.clone(),
        Some(Value::String(v)) => {
            return Err(format!(
                "{MANIFEST_FILE}: version {} is not a semver like 0.1.0",
                shown(v)
            ));
        }
        _ => return Err(format!("{MANIFEST_FILE}: \"version\" must be a string")),
    };
    let Some(listed) = top.get("platforms").and_then(Value::as_object) else {
        return Err(format!("{MANIFEST_FILE}: \"platforms\" must be an object"));
    };
    let warnings = listed
        .keys()
        .filter(|key| Platform::from_key(key).is_none())
        .map(|key| {
            format!(
                "{MANIFEST_FILE}: unknown platform {} (use mac, windows or linux)",
                shown(key)
            )
        })
        .collect();
    let entries = Platform::ALL
        .into_iter()
        .filter_map(|platform| {
            listed
                .get(platform.key())
                .map(|entry| (platform, check_entry(platform, entry)))
        })
        .collect();
    Ok(Listing {
        version,
        warnings,
        entries,
    })
}

/// The manifest rules alone, for a manifest read from the bucket: every listed
/// platform that passes them is served. `Err` rejects the whole manifest
/// (bad JSON, version or `platforms`), with the warning.
pub(crate) fn check_manifest(raw: &[u8]) -> Result<Loaded, String> {
    if raw.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(format!(
            "{MANIFEST_FILE} is {} bytes, over the {MAX_MANIFEST_BYTES}-byte limit",
            raw.len()
        ));
    }
    let listing = parse_listing(raw)?;
    let mut loaded = listing.loaded();
    for (platform, entry) in listing.entries {
        loaded.add(
            platform,
            entry.map(|e| Asset {
                file: e.file,
                bytes: e.bytes,
                sha256: e.sha256,
                path: PathBuf::new(),
            }),
        );
    }
    Ok(loaded)
}

/// The whole file, or `None` when it is longer than [`MAX_MANIFEST_BYTES`].
async fn read_capped(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let file = tokio::fs::File::open(path).await?;
    let mut raw = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut raw)
        .await?;
    Ok((raw.len() as u64 <= MAX_MANIFEST_BYTES).then_some(raw))
}

fn left_out(platform: Platform, why: &str) -> String {
    format!("{MANIFEST_FILE}: {} left out: {why}", platform.key())
}

/// One platform entry, checked against the manifest rules.
fn check_entry(platform: Platform, entry: &Value) -> Result<Entry, String> {
    let fail = |why: String| left_out(platform, &why);
    let Some(entry) = entry.as_object() else {
        return Err(fail("the entry is not an object".to_owned()));
    };
    let Some(file) = entry.get("file").and_then(Value::as_str) else {
        return Err(fail("\"file\" must be a string".to_owned()));
    };
    if !is_plain_file_name(file) {
        return Err(fail(format!(
            "file {} is not a plain file name (letters, digits, '.', '_', '-', '+')",
            shown(file)
        )));
    }
    let Some(bytes) = entry.get("bytes").and_then(Value::as_u64) else {
        return Err(fail("\"bytes\" must be a whole number".to_owned()));
    };
    if bytes == 0 {
        return Err(fail("\"bytes\" is 0".to_owned()));
    }
    let Some(sha256) = entry.get("sha256").and_then(Value::as_str) else {
        return Err(fail("\"sha256\" must be a string".to_owned()));
    };
    if !is_sha256(sha256) {
        return Err(fail(format!(
            "sha256 {} is not 64 hex digits",
            shown(sha256)
        )));
    }
    Ok(Entry {
        file: file.to_owned(),
        bytes,
        sha256: sha256.to_ascii_lowercase(),
    })
}

/// One checked entry, checked against the folder.
async fn on_disk(root: &Path, platform: Platform, entry: Entry) -> Result<Asset, String> {
    let fail = |why: String| left_out(platform, &why);
    let file = entry.file.as_str();
    let (path, meta) = match resolve_inside(root, file).await {
        Resolved::File(path, meta) => (path, meta),
        Resolved::Missing => return Err(fail(format!("{file} is not in the downloads folder"))),
        Resolved::Outside => {
            return Err(fail(format!("{file} points outside the downloads folder")));
        }
        Resolved::NotAFile => return Err(fail(format!("{file} is not a regular file"))),
        Resolved::Unreadable(e) => return Err(fail(format!("cannot read {file}: {e}"))),
    };
    if meta.len() != entry.bytes {
        return Err(fail(format!(
            "{file} is {} bytes on disk but the manifest says {}",
            meta.len(),
            entry.bytes
        )));
    }
    Ok(Asset {
        file: entry.file,
        bytes: entry.bytes,
        sha256: entry.sha256,
        path,
    })
}

#[derive(Default)]
struct Cache {
    loaded: Option<(Instant, Arc<Release>)>,
    /// The warnings of the last read, each logged once when it first appeared.
    warned: BTreeSet<String>,
}

impl Cache {
    /// Store a fresh read; returns the warnings that are new since the last one.
    fn store(&mut self, loaded: Loaded, at: Instant) -> (Arc<Release>, Vec<String>) {
        let next: BTreeSet<String> = loaded.warnings.into_iter().collect();
        let new = next.difference(&self.warned).cloned().collect();
        self.warned = next;
        let release = Arc::new(loaded.release);
        self.loaded = Some((at, Arc::clone(&release)));
        (release, new)
    }
}

/// Bounds on the file streams (`GET /download/files/<file>`). `HEAD`, `304` and
/// `416` answers open no stream and are not counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamLimits {
    /// Streams at once, all clients together; past it, `503`.
    pub max_streams: usize,
    /// Streams at once per client network; past it, `429`.
    pub max_streams_per_client: usize,
    /// Close a stream whose client has read nothing for this long. `None` never
    /// closes one.
    pub idle_timeout: Option<Duration>,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            max_streams: MAX_STREAMS,
            max_streams_per_client: MAX_STREAMS_PER_CLIENT,
            idle_timeout: Some(IDLE_TIMEOUT),
        }
    }
}

/// How a [`Downloads`] handler is set up.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadOptions {
    /// The downloads folder; `None` serves the empty manifest and `404`s.
    pub dir: Option<PathBuf>,
    /// How long a read manifest is reused (`Duration::ZERO`: every request).
    pub reload_every: Duration,
    /// Per-client byte budget in MiB: a burst, refilled per minute. Each `200`
    /// or `206` costs its length rounded up to a whole MiB (at least 1).
    pub budget_mib: BucketConfig,
    /// Concurrency and idle bounds on the streams.
    pub streams: StreamLimits,
}

impl DownloadOptions {
    /// The production defaults for the folder `dir`.
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            reload_every: RELOAD_EVERY,
            budget_mib: RateLimitConfig::default().download,
            streams: StreamLimits::default(),
        }
    }
}

/// The `/download/**` handler. Cheap to clone; clones share the manifest cache
/// and the stream limits.
#[derive(Clone)]
pub struct Downloads {
    inner: Arc<Inner>,
}

struct Inner {
    /// Bucket mode when set; the disk fields below are then unused.
    bucket: Option<Arc<BucketSource>>,
    dir: Option<PathBuf>,
    reload_every: Duration,
    cache: tokio::sync::Mutex<Cache>,
    limits: StreamLimits,
    streams: Arc<Semaphore>,
    per_client: Arc<Mutex<HashMap<IpAddr, usize>>>,
    budget: RateLimiter,
}

impl std::fmt::Debug for Downloads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Downloads")
            .field("bucket", &self.inner.bucket)
            .field("dir", &self.inner.dir)
            .field("reload_every", &self.inner.reload_every)
            .field("limits", &self.inner.limits)
            .finish_non_exhaustive()
    }
}

impl Downloads {
    /// Serve the folder `dir` with the production limits; `None` serves the
    /// empty manifest and `404`s.
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self::with_options(DownloadOptions::new(dir))
    }

    /// [`Downloads::new`] with another reuse window (`Duration::ZERO` reads the
    /// folder on every request).
    pub fn with_reload_every(dir: Option<PathBuf>, reload_every: Duration) -> Self {
        Self::with_options(DownloadOptions {
            reload_every,
            ..DownloadOptions::new(dir)
        })
    }

    /// A handler set up by `options`.
    pub fn with_options(options: DownloadOptions) -> Self {
        Self::build(options, None)
    }

    /// Serve the bucket `settings` with the production timing.
    pub fn bucket(settings: BucketSettings) -> Self {
        Self::with_bucket(BucketOptions::new(settings))
    }

    /// A bucket-mode handler set up by `options`. The first manifest fetch starts
    /// in the background when a tokio runtime is running.
    pub fn with_bucket(options: BucketOptions) -> Self {
        Self::build(
            DownloadOptions::new(None),
            Some(BucketSource::start(options)),
        )
    }

    fn build(options: DownloadOptions, bucket: Option<Arc<BucketSource>>) -> Self {
        let permits = options.streams.max_streams.clamp(1, Semaphore::MAX_PERMITS);
        Self {
            inner: Arc::new(Inner {
                bucket,
                dir: options.dir,
                reload_every: options.reload_every,
                cache: tokio::sync::Mutex::new(Cache::default()),
                limits: options.streams,
                streams: Arc::new(Semaphore::new(permits)),
                per_client: Arc::new(Mutex::new(HashMap::new())),
                budget: RateLimiter::new(RateLimitConfig {
                    download: options.budget_mib,
                    ..RateLimitConfig::unlimited()
                }),
            }),
        }
    }

    /// The served folder (`None` in bucket mode).
    pub fn dir(&self) -> Option<&Path> {
        self.inner.dir.as_deref()
    }

    /// True in bucket mode.
    pub fn is_bucket(&self) -> bool {
        self.inner.bucket.is_some()
    }

    /// Bucket mode: fetch the manifest now, whatever its age.
    #[cfg(test)]
    pub(crate) async fn refresh_bucket(&self) {
        if let Some(bucket) = &self.inner.bucket {
            bucket.refresh(false).await;
        }
    }

    /// The checked manifest, read again once [`RELOAD_EVERY`] has passed. One
    /// read at a time; a new problem is logged once. In bucket mode, the last
    /// manifest fetched ([`bucket`]).
    pub async fn release(&self) -> Arc<Release> {
        if let Some(bucket) = &self.inner.bucket {
            return bucket.release().await;
        }
        let mut cache = self.inner.cache.lock().await;
        if let Some((at, release)) = &cache.loaded
            && at.elapsed() < self.inner.reload_every
        {
            return Arc::clone(release);
        }
        let loaded = match &self.inner.dir {
            Some(dir) => load(dir).await,
            None => Loaded::default(),
        };
        let previous = cache.loaded.as_ref().map(|(_, r)| Arc::clone(r));
        let (release, new_warnings) = cache.store(loaded, Instant::now());
        drop(cache);
        let dir = self
            .inner
            .dir
            .as_deref()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        for warning in new_warnings {
            tracing::warn!(dir = %dir, "desktop downloads: {warning}");
        }
        if previous.as_deref() != Some(&*release) {
            let platforms: Vec<&str> = release.platforms.iter().map(|(p, _)| p.key()).collect();
            tracing::info!(
                dir = %dir,
                version = release.version.as_deref().unwrap_or("none"),
                platforms = ?platforms,
                "desktop downloads loaded"
            );
        }
        release
    }

    /// Answer one `GET`/`HEAD` under `/download/` from an unknown client (all
    /// such requests share one client's limits).
    pub async fn serve(&self, req: Request) -> Response {
        self.serve_from(req, UNKNOWN_CLIENT).await
    }

    /// Answer one `GET`/`HEAD` under `/download/` from `client` (the trusted
    /// client IP; the router passes [`crate::api`]'s rate-limit key).
    pub async fn serve_from(&self, req: Request, client: IpAddr) -> Response {
        let Some(rest) = req.uri().path().strip_prefix("/download/") else {
            return not_found();
        };
        let Some(rest) = decode_uri_component(rest) else {
            return text(StatusCode::BAD_REQUEST, "Bad path");
        };
        if rest == MANIFEST_FILE {
            return manifest_response(&*self.release().await, req.headers());
        }
        if let Some(platform) = Platform::from_key(&rest) {
            let release = self.release().await;
            return match (release.asset(platform), &self.inner.bucket) {
                (Some(asset), Some(bucket)) => bucket.redirect(asset, req.method()),
                (Some(asset), None) => redirect_to(asset),
                (None, _) => not_found(),
            };
        }
        if let Some(file) = rest.strip_prefix("files/") {
            let release = self.release().await;
            return match (release.by_file(file), &self.inner.bucket) {
                (Some(asset), Some(bucket)) => bucket.redirect(asset, req.method()),
                (Some(asset), None) => self.serve_file(asset, req, client).await,
                (None, _) => not_found(),
            };
        }
        not_found()
    }

    /// A stream slot for `client`, or the status and message of the refusal.
    fn take_slot(
        &self,
        client: IpAddr,
    ) -> Result<(ClientSlot, OwnedSemaphorePermit), (StatusCode, &'static str)> {
        let key = client_key(client);
        let slot = {
            let mut counts = self.inner.per_client.lock();
            let count = counts.get(&key).copied().unwrap_or(0);
            if count >= self.inner.limits.max_streams_per_client {
                return Err((
                    StatusCode::TOO_MANY_REQUESTS,
                    "Too many downloads from your network at once. Try again in a minute.",
                ));
            }
            counts.insert(key, count + 1);
            ClientSlot {
                counts: Arc::clone(&self.inner.per_client),
                key,
            }
        };
        // On a refusal `slot` drops here and gives the client's slot back.
        match Arc::clone(&self.inner.streams).try_acquire_owned() {
            Ok(permit) => Ok((slot, permit)),
            Err(_) => Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Too many downloads are running. Try again in a minute.",
            )),
        }
    }

    async fn serve_file(&self, asset: &Asset, req: Request, client: IpAddr) -> Response {
        let etag = asset.etag();
        let is_get = req.method() == Method::GET;
        let (mut parts, _) = req.into_parts();
        if let Some(value) = parts.headers.get(header::IF_NONE_MATCH) {
            if none_match_hits(value, &etag) {
                return not_modified(&etag);
            }
            // RFC 9110 13.1.3: If-Modified-Since is ignored when If-None-Match is present.
            parts.headers.remove(header::IF_MODIFIED_SINCE);
        }
        if let Some(value) = parts.headers.get(header::IF_RANGE)
            && !if_range_holds(value, &etag)
        {
            parts.headers.remove(header::RANGE);
        }
        keep_single_byte_range(&mut parts.headers, Some(asset.bytes));
        let slot = if is_get {
            match self.take_slot(client) {
                Ok(slot) => Some(slot),
                Err((status, message)) => return refused(status, message, RETRY_AFTER_SECS),
            }
        } else {
            None
        };
        let req = Request::from_parts(parts, Body::empty());
        let mut res = match ServeFile::new(&asset.path).oneshot(req).await {
            Ok(res) => res.map(Body::new),
            Err(never) => match never {},
        };
        let status = res.status();
        let headers = res.headers_mut();
        match status {
            StatusCode::OK | StatusCode::PARTIAL_CONTENT => {
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(content_type(&asset.file)),
                );
                if let Ok(disposition) =
                    HeaderValue::from_str(&format!("attachment; filename=\"{}\"", asset.file))
                {
                    headers.insert(header::CONTENT_DISPOSITION, disposition);
                }
                headers.remove(header::CONTENT_ENCODING);
                file_headers(headers, &etag);
                if let Some((client_slot, permit)) = slot {
                    let len = headers
                        .get(header::CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(asset.bytes);
                    // Whole MiB, at least one; an f64 holds every such count exactly.
                    #[allow(clippy::cast_precision_loss)]
                    let cost = len.div_ceil(MIB).max(1) as f64;
                    if let Err(wait) = self.inner.budget.take(RouteClass::Download, client, cost) {
                        return refused(
                            StatusCode::TOO_MANY_REQUESTS,
                            "Your network has downloaded a lot in the last hour. Try again later.",
                            wait.as_secs().saturating_add(1),
                        );
                    }
                    let (parts, body) = res.into_parts();
                    let body = guard(body, client_slot, permit, self.inner.limits.idle_timeout);
                    return Response::from_parts(parts, body);
                }
            }
            StatusCode::NOT_MODIFIED => {
                file_headers(headers, &etag);
                *res.body_mut() = no_length_body();
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/plain;charset=UTF-8"),
                );
                headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(NO_CACHE));
            }
            StatusCode::NOT_FOUND => return not_found(),
            _ => {}
        }
        res
    }
}

/// One of a client's [`StreamLimits::max_streams_per_client`] slots, given back
/// when the response body is dropped (the stream ended or the connection closed).
struct ClientSlot {
    counts: Arc<Mutex<HashMap<IpAddr, usize>>>,
    key: IpAddr,
}

impl Drop for ClientSlot {
    fn drop(&mut self) {
        let mut counts = self.counts.lock();
        if let Some(count) = counts.get_mut(&self.key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                counts.remove(&self.key);
            }
        }
    }
}

/// What a running stream holds that the idle watchdog may take back.
struct Held {
    /// The file's bytes; `None` once the stream ended or timed out.
    data: Option<BodyDataStream>,
    /// The global stream slot.
    permit: Option<OwnedSemaphorePermit>,
    /// When the client last took a chunk.
    last_progress: Instant,
    /// The watchdog closed the stream; the next poll reports it.
    timed_out: bool,
}

/// A file body that holds its stream slots. The global slot and the file are
/// given back when the stream ends or its client stops reading for the idle
/// timeout; the client's own slot is held until the connection drops the body,
/// so one client network can never hold more than its share of connections.
struct Guarded {
    held: Arc<Mutex<Held>>,
    _client: ClientSlot,
}

impl Stream for Guarded {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut held = self.held.lock();
        if std::mem::take(&mut held.timed_out) {
            return Poll::Ready(Some(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the client stopped reading",
            ))));
        }
        let Some(data) = held.data.as_mut() else {
            return Poll::Ready(None);
        };
        match data.poll_next_unpin(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(bytes))) => {
                held.last_progress = Instant::now();
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(e))) => {
                held.data = None;
                held.permit = None;
                Poll::Ready(Some(Err(io::Error::other(e))))
            }
            Poll::Ready(None) => {
                held.data = None;
                held.permit = None;
                Poll::Ready(None)
            }
        }
    }
}

/// Wrap a file body in its slots, with an idle watchdog when `idle` is set.
fn guard(
    body: Body,
    client: ClientSlot,
    permit: OwnedSemaphorePermit,
    idle: Option<Duration>,
) -> Body {
    let held = Arc::new(Mutex::new(Held {
        data: Some(body.into_data_stream()),
        permit: Some(permit),
        last_progress: Instant::now(),
        timed_out: false,
    }));
    if let Some(idle) = idle {
        watch(Arc::downgrade(&held), idle);
    }
    Body::from_stream(Guarded {
        held,
        _client: client,
    })
}

/// Close the stream once its client has taken nothing for `idle`. A stalled
/// connection is not polled by hyper at all, so only a timer can notice it.
fn watch(held: Weak<Mutex<Held>>, idle: Duration) {
    let tick = (idle / 4).max(Duration::from_millis(5));
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tick).await;
            let Some(held) = held.upgrade() else {
                return;
            };
            let mut held = held.lock();
            if held.data.is_none() {
                return;
            }
            if held.last_progress.elapsed() >= idle {
                held.data = None;
                held.permit = None;
                held.timed_out = true;
                drop(held);
                tracing::debug!(
                    idle_secs = idle.as_secs(),
                    "desktop downloads: a client stopped reading; its stream was closed"
                );
                return;
            }
        }
    });
}

/// An empty body of unknown length. axum adds `Content-Length` from an exact
/// size hint, and a `304` must not carry `Content-Length: 0` (RFC 9110 8.6).
fn no_length_body() -> Body {
    Body::from_stream(futures::stream::empty::<io::Result<Bytes>>())
}

/// A refusal with `Retry-After` and `Cache-Control: no-store`.
fn refused(status: StatusCode, message: &'static str, retry_after_secs: u64) -> Response {
    let mut res = text(status, message);
    let headers = res.headers_mut();
    headers.insert(header::RETRY_AFTER, HeaderValue::from(retry_after_secs));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

fn not_modified(etag: &str) -> Response {
    let mut res = StatusCode::NOT_MODIFIED.into_response();
    file_headers(res.headers_mut(), etag);
    *res.body_mut() = no_length_body();
    res
}

fn not_found() -> Response {
    text(StatusCode::NOT_FOUND, "Not found")
}

/// The manifest JSON with `Cache-Control: no-cache` and a strong `ETag` (the
/// body's sha256); `304` when `If-None-Match` matches it.
fn manifest_response(release: &Release, request: &HeaderMap) -> Response {
    let body = release.to_json().to_string();
    let etag = format!("\"{:x}\"", Sha256::digest(body.as_bytes()));
    let mut res = match request.get(header::IF_NONE_MATCH) {
        Some(value) if none_match_hits(value, &etag) => {
            let mut res = StatusCode::NOT_MODIFIED.into_response();
            *res.body_mut() = no_length_body();
            res
        }
        _ => {
            let mut res = (StatusCode::OK, body).into_response();
            res.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            res
        }
    };
    let headers = res.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(NO_CACHE));
    if let Ok(etag) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, etag);
    }
    res
}

fn redirect_to(asset: &Asset) -> Response {
    // A plain file name is URL- and header-safe, so this cannot fail.
    let Ok(location) = HeaderValue::from_str(&format!("/download/files/{}", asset.file)) else {
        return not_found();
    };
    let mut res = StatusCode::FOUND.into_response();
    let headers = res.headers_mut();
    headers.insert(header::LOCATION, location);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(NO_CACHE));
    res
}

/// True when an `If-None-Match` value matches `etag` (weak comparison, or `*`).
fn none_match_hits(value: &HeaderValue, etag: &str) -> bool {
    let Ok(value) = value.to_str() else {
        return false;
    };
    value
        .split(',')
        .map(str::trim)
        .any(|tag| tag == "*" || tag.strip_prefix("W/").unwrap_or(tag) == etag)
}

/// True when an `If-Range` value is exactly `etag` (strong comparison). A date
/// never counts, so the client gets the whole file: always correct.
fn if_range_holds(value: &HeaderValue, etag: &str) -> bool {
    value.to_str().is_ok_and(|v| v.trim() == etag)
}

fn file_headers(headers: &mut HeaderMap, etag: &str) {
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(FILE_CACHE));
    if let Ok(etag) = HeaderValue::from_str(etag) {
        headers.insert(header::ETAG, etag);
    }
}

#[cfg(test)]
mod tests;
