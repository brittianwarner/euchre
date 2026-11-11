//! The first-start engine download (review fix PX-1).
//!
//! rivetkit's own download holds the whole 58 MB body under one 60 s timeout, so a slow but
//! working connection fails every time. This one streams into `<cache path>.part` with a
//! connect timeout and a per-read idle timeout and no total cap, resumes a cut transfer with
//! `Range`, checks the release's SHA-256, then renames the file into rivetkit's cache (where
//! `resolve_engine_binary_path` finds it on every later start). It keeps going while bytes
//! arrive; it gives up only when nothing arrives for [`FetchTiming::give_up`], or when it
//! never reaches the release server at all.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where the engine comes from and where it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchPlan {
    /// `{releases}/rivet/{version}/engine/SHA256SUMS`.
    pub manifest_url: String,
    /// `{releases}/rivet/{version}/engine/{artifact}`.
    pub artifact_url: String,
    /// The artifact's name, as the manifest lists it.
    pub artifact: String,
    /// rivetkit's cache path for it.
    pub dest: PathBuf,
}

impl FetchPlan {
    /// The plan rivetkit-engine-process 3.1.0 would follow: `RIVETKIT_ENGINE_RELEASES_ENDPOINT`
    /// (default `https://releases.rivet.dev`), `RIVETKIT_ENGINE_VERSION` (default the pinned
    /// engine), cached at `$RIVETKIT_STORAGE_PATH/.rivetkit` (else `~/.rivetkit`)
    /// `/engine/v3/<version>/<artifact>`. `None` without a home dir.
    pub fn for_this_machine() -> Option<FetchPlan> {
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let releases = env("RIVETKIT_ENGINE_RELEASES_ENDPOINT")
            .unwrap_or_else(|| "https://releases.rivet.dev".to_owned());
        let version = env("RIVETKIT_ENGINE_VERSION")
            .unwrap_or_else(|| euchre_server_core::rivet::ENGINE_VERSION.to_owned());
        let root = match env("RIVETKIT_STORAGE_PATH") {
            Some(path) => PathBuf::from(path).join(".rivetkit"),
            None => PathBuf::from(env("HOME")?).join(".rivetkit"),
        };
        let artifact = artifact_name();
        let base = releases.trim_end_matches('/');
        Some(FetchPlan {
            manifest_url: format!("{base}/rivet/{version}/engine/SHA256SUMS"),
            artifact_url: format!("{base}/rivet/{version}/engine/{artifact}"),
            dest: root
                .join("engine")
                .join("v3")
                .join(&version)
                .join(&artifact),
            artifact,
        })
    }

    /// The partial file next to the destination.
    pub fn part_path(&self) -> PathBuf {
        let mut name = self
            .dest
            .file_name()
            .map(std::ffi::OsStr::to_os_string)
            .unwrap_or_default();
        name.push(".part");
        self.dest.with_file_name(name)
    }
}

/// The engine artifact's name on this platform (as rivetkit-engine-process names it).
pub fn artifact_name() -> String {
    let arch = std::env::consts::ARCH;
    let target = match std::env::consts::OS {
        "linux" => format!("{arch}-unknown-linux-musl"),
        "macos" => format!("{arch}-apple-darwin"),
        "windows" => format!("{arch}-pc-windows-gnu.exe"),
        other => format!("{arch}-{other}"),
    };
    format!("rivet-engine-{target}")
}

/// The download's clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FetchTiming {
    /// Reaching the server.
    pub connect: Duration,
    /// The longest wait for the next bytes (or the response head) before the connection is
    /// dropped and the transfer resumed on a new one.
    pub idle: Duration,
    /// No new bytes for this long and it gives up ([`FetchError::Stalled`]).
    pub give_up: Duration,
    /// Past this (or at the first retry) the download counts as slow.
    pub slow_after: Duration,
    /// Attempts that never reach the server before it gives up
    /// ([`FetchError::Unreachable`]).
    pub unreachable_tries: u32,
}

impl Default for FetchTiming {
    fn default() -> FetchTiming {
        FetchTiming {
            connect: Duration::from_secs(15),
            idle: Duration::from_secs(30),
            give_up: Duration::from_secs(180),
            slow_after: Duration::from_secs(45),
            unreachable_tries: 3,
        }
    }
}

/// Why the download ended without an engine.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The release server never answered (offline, DNS, refused).
    #[error("the release server could not be reached")]
    Unreachable,
    /// It started, then nothing arrived for [`FetchTiming::give_up`].
    #[error("the download stalled")]
    Stalled,
    /// The server answered something that is not the engine (a 404, a bad manifest, a
    /// checksum that never matches).
    #[error("the release is not usable: {0}")]
    Bad(String),
    /// The cache could not be written.
    #[error("the engine could not be saved: {0}")]
    Disk(String),
}

/// What one attempt ended with.
enum Attempt {
    /// The whole body is in the part file.
    Complete,
    /// The connection dropped or went idle; try again (`bool`: it reached the server).
    Retry { reached: bool },
}

/// Downloads the engine for `plan`, calling `slow` once when it turns slow. Returns the
/// cached path.
pub async fn fetch_engine(
    plan: &FetchPlan,
    timing: FetchTiming,
    slow: &(dyn Fn() + Send + Sync),
) -> Result<PathBuf, FetchError> {
    let client = reqwest::Client::builder()
        .connect_timeout(timing.connect)
        .build()
        .map_err(|err| FetchError::Bad(format!("client: {err}")))?;
    if let Some(dir) = plan.dest.parent() {
        std::fs::create_dir_all(dir).map_err(|err| FetchError::Disk(err.kind().to_string()))?;
    }
    let began = Instant::now();
    let mut said_slow = false;
    let mut mark_slow = || {
        if !said_slow {
            said_slow = true;
            tracing::info!(
                after_ms = began.elapsed().as_millis(),
                "the engine download is slow; it keeps going"
            );
            slow();
        }
    };
    let mut last_progress = Instant::now();
    let mut unreachable = 0_u32;
    let mut reached_ever = false;
    let mut checksum_misses = 0_u32;
    let mut expected: Option<String> = None;
    let mut retry = 0_u32;
    loop {
        if began.elapsed() >= timing.slow_after {
            mark_slow();
        }
        let outcome = match &expected {
            None => match manifest(&client, plan, timing.idle).await {
                Ok(sum) => {
                    reached_ever = true;
                    last_progress = Instant::now();
                    expected = Some(sum);
                    continue;
                }
                Err(Some(bad)) => return Err(bad),
                Err(None) => Attempt::Retry { reached: false },
            },
            Some(_) => {
                body(
                    &client,
                    plan,
                    timing,
                    &mut last_progress,
                    began,
                    &mut mark_slow,
                )
                .await?
            }
        };
        match outcome {
            Attempt::Complete => {
                let sum = expected.clone().unwrap_or_default();
                let part = plan.part_path();
                let got = tokio::task::spawn_blocking(move || sha256_file(&part))
                    .await
                    .map_err(|err| FetchError::Disk(err.to_string()))?
                    .map_err(|err| FetchError::Disk(err.kind().to_string()))?;
                if got.eq_ignore_ascii_case(&sum) {
                    return install(plan);
                }
                let _ = std::fs::remove_file(plan.part_path());
                checksum_misses += 1;
                tracing::warn!(
                    checksum_misses,
                    "the engine download did not match its checksum"
                );
                if checksum_misses >= 2 {
                    return Err(FetchError::Bad("checksum mismatch".to_owned()));
                }
                last_progress = Instant::now();
            }
            Attempt::Retry { reached } => {
                if reached {
                    reached_ever = true;
                } else {
                    unreachable += 1;
                }
                if !reached_ever && unreachable >= timing.unreachable_tries {
                    return Err(FetchError::Unreachable);
                }
                if reached_ever && last_progress.elapsed() >= timing.give_up {
                    return Err(FetchError::Stalled);
                }
                if reached_ever {
                    mark_slow();
                }
                retry += 1;
                let backoff = Duration::from_millis(500 * u64::from(retry.min(8)));
                tokio::time::sleep(backoff.min(timing.idle)).await;
            }
        }
    }
}

/// The artifact's SHA-256 from the manifest. `Err(None)`: try again; `Err(Some)`: give up.
async fn manifest(
    client: &reqwest::Client,
    plan: &FetchPlan,
    idle: Duration,
) -> Result<String, Option<FetchError>> {
    let sent = tokio::time::timeout(idle, client.get(&plan.manifest_url).send()).await;
    let response = match sent {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            tracing::info!(kind = %request_kind(&err), "the engine manifest request failed");
            return Err(None);
        }
        Err(_) => return Err(None),
    };
    let status = response.status();
    if status.is_server_error() || status.as_u16() == 429 || status.as_u16() == 408 {
        return Err(None);
    }
    if !status.is_success() {
        return Err(Some(FetchError::Bad(format!("manifest status {status}"))));
    }
    let Ok(Ok(text)) = tokio::time::timeout(idle, response.text()).await else {
        return Err(None);
    };
    checksum_for(&text, &plan.artifact)
        .ok_or_else(|| Some(FetchError::Bad("the manifest has no entry".to_owned())))
}

/// One attempt at the body, resuming from the part file.
async fn body(
    client: &reqwest::Client,
    plan: &FetchPlan,
    timing: FetchTiming,
    last_progress: &mut Instant,
    began: Instant,
    mark_slow: &mut (dyn FnMut() + Send),
) -> Result<Attempt, FetchError> {
    let part = plan.part_path();
    let have = std::fs::metadata(&part).map_or(0, |m| m.len());
    let mut request = client.get(&plan.artifact_url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut response = match tokio::time::timeout(timing.idle, request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            tracing::info!(kind = %request_kind(&err), "the engine request failed");
            return Ok(Attempt::Retry {
                reached: !err.is_connect(),
            });
        }
        Err(_) => return Ok(Attempt::Retry { reached: false }),
    };
    let status = response.status().as_u16();
    let append = match status {
        206 if have > 0 => true,
        200 => false,
        // The part already holds every byte.
        416 if have > 0 => return Ok(Attempt::Complete),
        408 | 429 | 500..=599 => return Ok(Attempt::Retry { reached: true }),
        other => return Err(FetchError::Bad(format!("engine status {other}"))),
    };
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true);
    if append {
        options.append(true);
    } else {
        options.truncate(true);
    }
    let mut file = options
        .open(&part)
        .map_err(|err| FetchError::Disk(err.kind().to_string()))?;
    loop {
        match tokio::time::timeout(timing.idle, response.chunk()).await {
            Ok(Ok(Some(bytes))) => {
                file.write_all(&bytes)
                    .map_err(|err| FetchError::Disk(err.kind().to_string()))?;
                *last_progress = Instant::now();
                if began.elapsed() >= timing.slow_after {
                    mark_slow();
                }
            }
            Ok(Ok(None)) => {
                file.flush()
                    .map_err(|err| FetchError::Disk(err.kind().to_string()))?;
                return Ok(Attempt::Complete);
            }
            Ok(Err(_)) | Err(_) => {
                let _ = file.flush();
                return Ok(Attempt::Retry { reached: true });
            }
        }
    }
}

/// Moves the verified part file into the cache, executable.
fn install(plan: &FetchPlan) -> Result<PathBuf, FetchError> {
    let disk = |err: std::io::Error| FetchError::Disk(err.kind().to_string());
    std::fs::rename(plan.part_path(), &plan.dest).map_err(disk)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&plan.dest, std::fs::Permissions::from_mode(0o755))
            .map_err(disk)?;
    }
    Ok(plan.dest.clone())
}

/// A request error's kind for the log (never its text, which can carry a URL).
fn request_kind(err: &reqwest::Error) -> &'static str {
    if err.is_connect() {
        "connect"
    } else if err.is_timeout() {
        "timeout"
    } else if err.is_body() || err.is_decode() {
        "body"
    } else {
        "other"
    }
}

/// The checksum the manifest lists for `artifact` (`<64 hex> [*]<name>` lines).
pub fn checksum_for(manifest: &str, artifact: &str) -> Option<String> {
    manifest.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let sum = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit()) && name == artifact)
            .then(|| sum.to_ascii_lowercase())
    })
}

/// A file's SHA-256, lowercase hex.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finish()))
}

/// Lowercase hex.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(64), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// SHA-256 (FIPS 180-4), streaming. The workspace lock carries no hash crate for the
/// desktop, and this is the only digest it needs.
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    filled: usize,
    length: u64,
}

const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

impl Default for Sha256 {
    fn default() -> Sha256 {
        Sha256::new()
    }
}

impl Sha256 {
    /// A fresh hash.
    pub fn new() -> Sha256 {
        Sha256 {
            state: [
                0x6a09_e667,
                0xbb67_ae85,
                0x3c6e_f372,
                0xa54f_f53a,
                0x510e_527f,
                0x9b05_688c,
                0x1f83_d9ab,
                0x5be0_cd19,
            ],
            block: [0; 64],
            filled: 0,
            length: 0,
        }
    }

    /// Adds `data`.
    pub fn update(&mut self, mut data: &[u8]) {
        self.length = self.length.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let take = (64 - self.filled).min(data.len());
            self.block[self.filled..self.filled + take].copy_from_slice(&data[..take]);
            self.filled += take;
            data = &data[take..];
            if self.filled == 64 {
                let block = self.block;
                self.compress(&block);
                self.filled = 0;
            }
        }
    }

    /// The digest.
    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.length.wrapping_mul(8);
        self.update(&[0x80]);
        while self.filled != 56 {
            self.update(&[0]);
        }
        let block = {
            let mut block = self.block;
            block[56..].copy_from_slice(&bits.to_be_bytes());
            block
        };
        self.compress(&block);
        let mut out = [0_u8; 32];
        for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *chunk = word.to_be_bytes();
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0_u32; 64];
        for (i, chunk) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*chunk);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, add) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(add);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    fn digest(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        hex(&h.finish())
    }

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(
            digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // A million 'a', fed in uneven pieces.
        let mut h = Sha256::new();
        let piece = [b'a'; 997];
        let mut left = 1_000_000;
        while left > 0 {
            let n = left.min(piece.len());
            h.update(&piece[..n]);
            left -= n;
        }
        assert_eq!(
            hex(&h.finish()),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn the_manifest_entry_is_found() {
        let sum = "a".repeat(64);
        let manifest = format!(
            "{}  rivet-engine-x86_64-apple-darwin\n{} *rivet-engine-aarch64-apple-darwin\n",
            "b".repeat(64),
            sum.to_uppercase()
        );
        assert_eq!(
            checksum_for(&manifest, "rivet-engine-aarch64-apple-darwin"),
            Some(sum)
        );
        assert_eq!(checksum_for(&manifest, "rivet-engine-other"), None);
        assert_eq!(
            checksum_for("xyz rivet-engine-other", "rivet-engine-other"),
            None
        );
    }

    /// How the test release server sends the body.
    #[derive(Clone, Copy)]
    struct Pace {
        /// Bytes per write.
        piece: usize,
        /// Pause between writes.
        gap: Duration,
        /// The first full-body request drops the connection after this many bytes.
        cut_at: Option<usize>,
        /// Headers only, then silence.
        stall: bool,
    }

    /// A one-file release server on loopback: `SHA256SUMS` and the artifact, with `Range`.
    /// Returns its base URL and the `Range` headers it saw.
    async fn release_server(
        artifact: String,
        body: Arc<Vec<u8>>,
        pace: Pace,
    ) -> (String, Arc<parking_lot::Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let ranges = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let seen = Arc::clone(&ranges);
        let cuts = Arc::new(AtomicUsize::new(0));
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let body = Arc::clone(&body);
                let artifact = artifact.clone();
                let seen = Arc::clone(&seen);
                let cuts = Arc::clone(&cuts);
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0_u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if sock.read(&mut byte).await.unwrap_or(0) == 0 {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).to_string();
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                    if path.ends_with("/SHA256SUMS") {
                        let text = format!("{}  {artifact}\n", digest(&body));
                        let reply = format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                            text.len()
                        );
                        let _ = sock.write_all(reply.as_bytes()).await;
                        return;
                    }
                    let from = head
                        .lines()
                        .find_map(|l| {
                            let l = l.to_ascii_lowercase();
                            l.strip_prefix("range: bytes=")
                                .map(|r| r.trim_end_matches('-').trim().to_owned())
                        })
                        .map(|r| {
                            seen.lock().push(r.clone());
                            r.parse::<usize>().unwrap_or(0)
                        });
                    let start = from.unwrap_or(0);
                    let rest = &body[start.min(body.len())..];
                    let head = if from.is_some() {
                        format!(
                            "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {start}-{}/{}\r\nconnection: close\r\n\r\n",
                            rest.len(),
                            body.len() - 1,
                            body.len()
                        )
                    } else {
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            rest.len()
                        )
                    };
                    if sock.write_all(head.as_bytes()).await.is_err() {
                        return;
                    }
                    if pace.stall {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        return;
                    }
                    let cut = (from.is_none() && cuts.fetch_add(1, Ordering::SeqCst) == 0)
                        .then_some(pace.cut_at)
                        .flatten();
                    let mut sent = 0;
                    for piece in rest.chunks(pace.piece) {
                        if cut.is_some_and(|c| sent >= c) {
                            return;
                        }
                        if sock.write_all(piece).await.is_err() {
                            return;
                        }
                        sent += piece.len();
                        tokio::time::sleep(pace.gap).await;
                    }
                });
            }
        });
        (format!("http://{addr}"), ranges)
    }

    fn plan(base: &str, dir: &Path) -> FetchPlan {
        let artifact = "rivet-engine-test".to_owned();
        FetchPlan {
            manifest_url: format!("{base}/rivet/9.9.9/engine/SHA256SUMS"),
            artifact_url: format!("{base}/rivet/9.9.9/engine/{artifact}"),
            dest: dir.join("engine/v3/9.9.9").join(&artifact),
            artifact,
        }
    }

    /// A slow body that takes longer than any total cap would allow, cut once mid-way: it
    /// resumes with `Range`, verifies, installs and says it was slow.
    #[tokio::test]
    async fn a_slow_cut_download_resumes_and_finishes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let body: Arc<Vec<u8>> = Arc::new((0..48_000_u32).map(|i| (i % 251) as u8).collect());
        let pace = Pace {
            piece: 4_000,
            gap: Duration::from_millis(120),
            cut_at: Some(20_000),
            stall: false,
        };
        let (base, ranges) =
            release_server("rivet-engine-test".to_owned(), Arc::clone(&body), pace).await;
        let plan = plan(&base, dir.path());
        let timing = FetchTiming {
            connect: Duration::from_secs(2),
            idle: Duration::from_millis(800),
            give_up: Duration::from_secs(5),
            slow_after: Duration::from_millis(600),
            unreachable_tries: 3,
        };
        let slow = Arc::new(AtomicUsize::new(0));
        let said = Arc::clone(&slow);
        let began = Instant::now();
        let path = fetch_engine(&plan, timing, &move || {
            said.fetch_add(1, Ordering::SeqCst);
        })
        .await
        .expect("downloaded");
        assert!(
            began.elapsed() > Duration::from_millis(1_200),
            "longer than a short total cap would allow"
        );
        assert_eq!(path, plan.dest);
        assert_eq!(std::fs::read(&path).expect("engine"), *body);
        assert!(
            !plan.part_path().exists(),
            "the part file became the engine"
        );
        assert_eq!(
            *ranges.lock(),
            vec!["20000".to_owned()],
            "resumed where it was cut"
        );
        assert_eq!(slow.load(Ordering::SeqCst), 1, "slow, said once");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[tokio::test]
    async fn a_stalled_download_gives_up_and_nothing_reached_is_unreachable() {
        let dir = tempfile::tempdir().expect("temp dir");
        let body = Arc::new(vec![7_u8; 1_000]);
        let pace = Pace {
            piece: 100,
            gap: Duration::ZERO,
            cut_at: None,
            stall: true,
        };
        let (base, _) = release_server("rivet-engine-test".to_owned(), body, pace).await;
        let timing = FetchTiming {
            connect: Duration::from_millis(500),
            idle: Duration::from_millis(200),
            give_up: Duration::from_millis(900),
            slow_after: Duration::from_secs(60),
            unreachable_tries: 2,
        };
        let got = fetch_engine(&plan(&base, dir.path()), timing, &|| {}).await;
        assert_eq!(got, Err(FetchError::Stalled));

        // Nothing listens there.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", closed.local_addr().expect("addr"));
        drop(closed);
        let got = fetch_engine(&plan(&base, dir.path()), timing, &|| {}).await;
        assert_eq!(got, Err(FetchError::Unreachable));
    }
}
