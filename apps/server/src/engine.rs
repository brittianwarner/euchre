//! The spawned engine (`EngineMode::Spawn`): a port-record check, a disk check
//! and a port check before the start, and a boot error that names the cause
//! when the engine does not come up.
//!
//! A data dir whose engine ran on another port is refused before anything
//! starts ([`BootError::EnginePortChanged`]): engine 3 ties its token signing
//! ring to the port, so a moved engine would refuse every gateway token.
//!
//! A busy port fails the boot at once, unless this data dir records an engine
//! pid: [`LocalEngine::start`] first reclaims an engine orphaned by a killed
//! earlier run of the same data dir, so that orphan's port must not count as
//! taken. The start checks the ports again after the reclaim, and they are
//! checked once more after a failed start.

use std::path::{Path, PathBuf};

use euchre_server_core::config::Secret;
use euchre_server_core::rivet::{
    ENGINE_PID_FILE, ENGINE_VERSION, EnginePortMismatch, LocalEngine, LocalEngineOptions,
    engine_ports, loopback_port_in_use, recorded_engine_port,
};

use crate::BootError;

/// The engine refuses to run with less free disk than this (its log shard
/// reserves 4 GiB).
pub const MIN_FREE_DISK_BYTES: u64 = 4 << 30;

/// What [`spawn`] starts.
pub(crate) struct SpawnSpec {
    pub(crate) port: u16,
    pub(crate) storage_path: PathBuf,
    pub(crate) token: Secret,
    pub(crate) binary_path: Option<PathBuf>,
}

/// Start the engine: no download, the configured admin token, data under
/// `storage_path`. Waits for health and readiness (bounded by
/// [`LocalEngineOptions::ready_timeout`]).
pub(crate) async fn spawn(spec: SpawnSpec) -> anyhow::Result<LocalEngine> {
    ensure_same_port(&spec.storage_path, spec.port)?;
    ensure_free_disk(&spec.storage_path, MIN_FREE_DISK_BYTES, free_disk_bytes)?;
    if !orphan_record(&spec.storage_path).exists() {
        ensure_ports_free(spec.port)?;
    }
    let mut options =
        LocalEngineOptions::new(spec.port, &spec.storage_path).with_admin_token(spec.token);
    options.binary_path = spec.binary_path.or_else(|| {
        let rivetkit_finds_it = std::env::var_os("RIVET_ENGINE_BINARY_PATH").is_some()
            || std::env::var_os("RIVETKIT_STORAGE_PATH").is_none();
        home_cached_binary(
            rivetkit_finds_it,
            std::env::var_os("HOME").map(PathBuf::from),
        )
    });
    options.auto_download = false;
    match LocalEngine::start(options).await {
        Ok(engine) => Ok(engine),
        Err(err) => Err(diagnose(spec.port, &spec.storage_path, err)),
    }
}

/// [`BootError::EnginePortChanged`] when `storage`'s engine ran on another port.
fn ensure_same_port(storage: &Path, port: u16) -> Result<(), BootError> {
    match recorded_engine_port(storage) {
        Some(recorded) if recorded != port => Err(BootError::EnginePortChanged {
            recorded,
            requested: port,
        }),
        _ => Ok(()),
    }
}

/// The engine binary rivetkit caches under `$HOME/.rivetkit` (auto-downloads put
/// it there). With `RIVETKIT_STORAGE_PATH` set, rivetkit looks for its cache under
/// that path instead, and the server never downloads, so a server given its own
/// storage would not find a binary it already has. `None` when rivetkit finds the
/// binary by itself (`RIVET_ENGINE_BINARY_PATH` is set, or `RIVETKIT_STORAGE_PATH`
/// is not, so rivetkit's cache is this one) or nothing is cached there.
fn home_cached_binary(rivetkit_finds_it: bool, home: Option<PathBuf>) -> Option<PathBuf> {
    if rivetkit_finds_it {
        return None;
    }
    let path = home?
        .join(".rivetkit")
        .join("engine")
        .join("v3")
        .join(ENGINE_VERSION)
        .join(engine_artifact_name());
    path.is_file().then_some(path)
}

/// rivetkit's release artifact name for this platform
/// (`rivetkit-engine-process`'s private `engine_artifact_name`).
fn engine_artifact_name() -> String {
    let arch = std::env::consts::ARCH;
    let target = match std::env::consts::OS {
        "linux" => format!("{arch}-unknown-linux-musl"),
        "macos" => format!("{arch}-apple-darwin"),
        "windows" => format!("{arch}-pc-windows-gnu.exe"),
        other => format!("{arch}-{other}"),
    };
    format!("rivet-engine-{target}")
}

/// Where [`LocalEngine::start`] records the engine it spawned on `storage`
/// (removed again on a clean stop). Present after a crash: the start then
/// reclaims that orphan, ports included.
fn orphan_record(storage: &Path) -> PathBuf {
    storage
        .join(".rivetkit")
        .join("var")
        .join("engine")
        .join(ENGINE_PID_FILE)
}

/// [`BootError::EnginePortInUse`] for the first engine port something holds.
fn ensure_ports_free(port: u16) -> Result<(), BootError> {
    let ports = engine_ports(port);
    match ports.iter().copied().find(|p| port_in_use(*p)) {
        Some(busy) => Err(BootError::EnginePortInUse {
            port: busy,
            ports: ports.to_vec(),
        }),
        None => Ok(()),
    }
}

/// Put the cause on a failed start: a moved port, a busy port, low disk, a
/// missing binary.
fn diagnose(port: u16, storage: &Path, err: anyhow::Error) -> anyhow::Error {
    if let Some(moved) = err.downcast_ref::<EnginePortMismatch>() {
        let moved = BootError::EnginePortChanged {
            recorded: moved.recorded,
            requested: moved.requested,
        };
        return err.context(moved);
    }
    if let Err(busy) = ensure_ports_free(port) {
        return err.context(busy);
    }
    if let Err(low) = ensure_free_disk(storage, MIN_FREE_DISK_BYTES, free_disk_bytes) {
        return err.context(low);
    }
    if format!("{err:#}").contains("resolve rivet-engine binary") {
        return err.context(BootError::EngineBinaryMissing);
    }
    err.context("the local engine did not start")
}

/// True when something holds `127.0.0.1:port`: it cannot be bound, or it
/// accepts a connection.
pub(crate) fn port_in_use(port: u16) -> bool {
    loopback_port_in_use(port)
}

/// Refuse to start when `probe(path)` reports less than `min` bytes free. An
/// unknown amount (no probe on this platform) passes; the engine still checks.
pub(crate) fn ensure_free_disk(
    path: &Path,
    min: u64,
    probe: impl Fn(&Path) -> Option<u64>,
) -> Result<(), BootError> {
    match probe(path) {
        Some(free) if free < min => Err(BootError::LowDisk {
            path: path.to_owned(),
            free_bytes: free,
        }),
        _ => Ok(()),
    }
}

/// Free bytes available to this user on the filesystem holding `path` (or its
/// nearest existing ancestor).
pub(crate) fn free_disk_bytes(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|p| p.exists())?;
    statvfs_available(existing)
}

#[cfg(unix)]
fn statvfs_available(path: &Path) -> Option<u64> {
    let stat = rustix::fs::statvfs(path).ok()?;
    Some(stat.f_bavail.saturating_mul(stat.f_frsize))
}

#[cfg(not(unix))]
fn statvfs_available(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;
    use std::net::{Ipv4Addr, TcpListener};
    use std::time::Duration;

    #[test]
    fn low_disk_names_the_cause() {
        let path = Path::new("/data/engine");
        let err = ensure_free_disk(path, MIN_FREE_DISK_BYTES, |_| Some(1 << 30))
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("not enough free disk space"), "{err}");
        assert!(err.contains("1.0 GiB free"), "{err}");
        assert!(err.contains("/data/engine"), "{err}");
        assert!(err.contains("4 GiB"), "{err}");
        assert!(ensure_free_disk(path, MIN_FREE_DISK_BYTES, |_| Some(5 << 30)).is_ok());
        assert!(ensure_free_disk(path, MIN_FREE_DISK_BYTES, |_| None).is_ok());
    }

    #[test]
    fn free_disk_reads_the_nearest_existing_dir() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let missing = dir.path().join("not").join("yet");
        if cfg!(unix) {
            let free = free_disk_bytes(&missing).unwrap_or(0);
            assert!(free > 0, "no free space read for {}", missing.display());
        }
        Ok(())
    }

    #[test]
    fn a_moved_port_is_refused_before_anything_starts() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = dir.path().join("engine");
        assert!(ensure_same_port(&storage, 6910).is_ok(), "fresh storage");
        let var = storage.join(".rivetkit").join("var").join("engine");
        std::fs::create_dir_all(&var)?;
        std::fs::write(var.join("euchre-engine.port"), "6910\n")?;
        assert!(ensure_same_port(&storage, 6910).is_ok());
        assert!(matches!(
            ensure_same_port(&storage, 6912),
            Err(BootError::EnginePortChanged {
                recorded: 6910,
                requested: 6912
            })
        ));
        Ok(())
    }

    #[test]
    fn the_home_cache_is_used_when_nothing_else_names_a_binary() -> anyhow::Result<()> {
        let home = tempfile::tempdir()?;
        assert_eq!(
            home_cached_binary(false, Some(home.path().to_owned())),
            None
        );
        let cached = home
            .path()
            .join(".rivetkit/engine/v3")
            .join(ENGINE_VERSION)
            .join(engine_artifact_name());
        std::fs::create_dir_all(cached.parent().context("no parent")?)?;
        std::fs::write(&cached, b"")?;
        assert_eq!(
            home_cached_binary(false, Some(home.path().to_owned())),
            Some(cached)
        );
        assert_eq!(home_cached_binary(true, Some(home.path().to_owned())), None);
        assert_eq!(home_cached_binary(false, None), None);
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_eq!(engine_artifact_name(), "rivet-engine-aarch64-apple-darwin");
        }
        Ok(())
    }

    #[test]
    fn a_held_port_reads_as_in_use() -> anyhow::Result<()> {
        let held = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = held.local_addr()?.port();
        assert!(port_in_use(port));
        drop(held);
        assert!(!port_in_use(port));
        Ok(())
    }

    /// A port held by another process fails the boot at once, before any engine
    /// starts (no orphan record on this storage to reclaim).
    #[tokio::test]
    async fn a_busy_port_fails_fast_without_an_orphan_record() -> anyhow::Result<()> {
        let held = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = held.local_addr()?.port();
        let dir = tempfile::tempdir()?;
        let started = std::time::Instant::now();
        let err = spawn(SpawnSpec {
            port,
            storage_path: dir.path().join("engine"),
            token: Secret::new("t".repeat(40)),
            binary_path: None,
        })
        .await
        .err()
        .context("spawned on a held port")?;
        assert!(
            matches!(
                err.downcast_ref::<BootError>(),
                Some(BootError::EnginePortInUse { port: busy, .. }) if *busy == port
            ),
            "{err:#}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!dir.path().join("engine").exists(), "storage was touched");
        Ok(())
    }

    #[test]
    fn the_engine_binds_guard_peer_and_metrics() {
        assert_eq!(engine_ports(6860), [6860, 6861, 6865]);
    }
}
