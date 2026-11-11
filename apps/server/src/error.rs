//! Boot failures a caller can act on. [`crate::start`] returns them inside its
//! `anyhow::Error` (`err.downcast_ref::<BootError>()`), with the underlying
//! cause in the chain; the message names the cause and what to change.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// Why the server did not start.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    /// Another server started by this process is still running.
    #[error(
        "a euchre server is already running in this process; shut it down before starting another"
    )]
    AlreadyRunning,
    /// A port the spawned engine binds is taken by another process.
    #[error(
        "engine port {port} on 127.0.0.1 is already in use (the engine binds {list}); stop what holds it, or give a fresh data dir other ports with ServerConfig::with_engine_port or RIVET_RUN_ENGINE_PORT",
        list = join_ports(.ports)
    )]
    EnginePortInUse {
        /// The busy port.
        port: u16,
        /// Every port the engine binds (guard, peer, metrics).
        ports: Vec<u16>,
    },
    /// The data dir's engine ran on another port. Engine 3 ties its token signing
    /// ring to that port, so on a new one every table session would fail.
    #[error(
        "this data dir's engine runs on port {recorded}, not {requested}: start it on port {recorded} again (ServerConfig::local picks it by itself), or use a fresh data dir for port {requested}"
    )]
    EnginePortChanged {
        /// The port the data dir's engine first ran on.
        recorded: u16,
        /// The port this start asked for.
        requested: u16,
    },
    /// The HTTP listen address is taken.
    #[error("HTTP address {addr} is already in use")]
    HttpPortInUse {
        /// The address.
        addr: SocketAddr,
    },
    /// Less than 4 GiB free where the engine keeps its data.
    #[error(
        "not enough free disk space for the engine: {free} free at {path}, at least 4 GiB needed",
        free = gib(*.free_bytes),
        path = .path.display()
    )]
    LowDisk {
        /// The engine storage path.
        path: PathBuf,
        /// Bytes free there.
        free_bytes: u64,
    },
    /// No engine binary, and downloading is off.
    #[error(
        "the rivet-engine 3.1.0 binary was not found (downloads are off); set RIVET_ENGINE_BINARY_PATH"
    )]
    EngineBinaryMissing,
    /// `playerProfile` already runs with other keys in this process.
    #[error(
        "playerProfile already runs with different keys in this process: every server in one process must share one secrets set (the same secrets.json or environment)"
    )]
    KeysConflict,
    /// `euchreTable` already runs with another config in this process.
    #[error(
        "euchreTable already runs with a different config in this process: keep the secrets, allowed origins, fast tempo and HTTP port the same across starts"
    )]
    TableConfigConflict,
    /// The actor runner did not serve the health probe in time.
    #[error("the actor runner did not start serving within {0:?}")]
    NotReady(Duration),
}

/// Why a started server stopped serving by itself ([`crate::RunningServer::closed`]).
/// `/health` already answers `503` when this is reported; restart the server.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServerFailure {
    /// The spawned engine exited.
    #[error("the local engine exited ({status}); restart the server")]
    EngineExited {
        /// Its exit status.
        status: String,
    },
    /// The actor runner (`Registry::serve`) stopped.
    #[error("the actor runner stopped ({reason}); restart the server")]
    RunnerStopped {
        /// Its error, or that it returned.
        reason: String,
    },
}

fn join_ports(ports: &[u16]) -> String {
    ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn gib(bytes: u64) -> String {
    // Display only: precision loss above 2^53 bytes does not matter here.
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64 / f64::from(1u32 << 30);
    format!("{value:.1} GiB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_cause() {
        let port = BootError::EnginePortInUse {
            port: 6865,
            ports: vec![6860, 6861, 6865],
        }
        .to_string();
        assert!(port.contains("engine port 6865"), "{port}");
        assert!(port.contains("6860, 6861, 6865"), "{port}");
        let disk = BootError::LowDisk {
            path: PathBuf::from("/x"),
            free_bytes: 3 << 29,
        }
        .to_string();
        assert!(disk.contains("1.5 GiB free at /x"), "{disk}");
        let moved = BootError::EnginePortChanged {
            recorded: 6910,
            requested: 6912,
        }
        .to_string();
        assert!(moved.contains("runs on port 6910, not 6912"), "{moved}");
    }
}
