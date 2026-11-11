//! What one process shares across every [`crate::start`]: the table's client
//! slot, the "one live server" lease, the remembered local HTTP port and the
//! envoy version counter.
//!
//! `euchreTable` and `playerProfile` keep their config process-wide (rivetkit
//! builds actors with no factory argument). A second start in the same process
//! must hand them an identical config, so:
//!
//! - every start passes the same [`TableClient`] slot ([`table_client`]);
//! - a local server that asked for port 0 gets the port the first one bound
//!   ([`bind_http`]), so its loopback origins, and with them the table's Origin
//!   allowlist, stay the same across restarts;
//! - only one server runs at a time ([`acquire`]): two would fight over the one
//!   client slot.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::LazyLock;

use anyhow::Context as _;
use euchre_table::TableClient;
use parking_lot::Mutex;
use tokio::net::TcpListener;

use crate::BootError;

static TABLE_CLIENT: LazyLock<TableClient> = LazyLock::new(TableClient::deferred);

struct State {
    running: bool,
    local_http: Option<SocketAddr>,
    last_version: u32,
}

static STATE: Mutex<State> = Mutex::new(State {
    running: false,
    local_http: None,
    last_version: 0,
});

/// Held by a [`crate::RunningServer`]; dropping it lets the next start run.
#[derive(Debug)]
pub(crate) struct Lease(());

impl Drop for Lease {
    fn drop(&mut self) {
        STATE.lock().running = false;
    }
}

/// The "one live server per process" lease.
pub(crate) fn acquire() -> Result<Lease, BootError> {
    let mut state = STATE.lock();
    if state.running {
        return Err(BootError::AlreadyRunning);
    }
    state.running = true;
    Ok(Lease(()))
}

/// The table's client slot. Clones share it, so `configure` sees the same slot
/// on every start.
pub(crate) fn table_client() -> TableClient {
    TABLE_CLIENT.clone()
}

/// Bind the HTTP listener. With `sticky` and port 0, reuse the port an earlier
/// sticky bind in this process got.
///
/// A local (`sticky`) wildcard bind on a known port is refused when something
/// already holds `127.0.0.1` on it: the OS lets `[::]:P` or `0.0.0.0:P` sit
/// beside a `127.0.0.1:P` listener, and loopback clients
/// ([`crate::RunningServer::url`]) would reach that one instead.
pub(crate) async fn bind_http(bind: SocketAddr, sticky: bool) -> anyhow::Result<TcpListener> {
    let remembered = if sticky && bind.port() == 0 {
        STATE
            .lock()
            .local_http
            .filter(|addr| addr.ip() == bind.ip())
    } else {
        None
    };
    let target = remembered.unwrap_or(bind);
    if sticky && target.ip().is_unspecified() && loopback_squatted(target.port()) {
        let held = SocketAddr::from((Ipv4Addr::LOCALHOST, target.port()));
        return Err(anyhow::anyhow!(
            "{held} is held by another listener; {target} would sit beside it"
        )
        .context(BootError::HttpPortInUse { addr: held }));
    }
    let listener = match TcpListener::bind(target).await {
        Ok(listener) => listener,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            return Err(anyhow::Error::new(e).context(BootError::HttpPortInUse { addr: target }));
        }
        Err(e) => {
            return Err(
                anyhow::Error::new(e).context(format!("bind the HTTP listener on {target}"))
            );
        }
    };
    if sticky && bind.port() == 0 {
        let addr = listener.local_addr().context("HTTP listener address")?;
        STATE.lock().local_http = Some(addr);
    }
    Ok(listener)
}

/// True when another listener holds `127.0.0.1:port` (port 0 never is).
fn loopback_squatted(port: u16) -> bool {
    port != 0 && crate::engine::port_in_use(port)
}

/// A fresh envoy version for a spawned engine: the unix time in seconds, and
/// always above the previous one in this process (a restart within the same
/// second still registers as a newer runner).
pub(crate) fn next_version(floor: u32) -> u32 {
    let now = u32::try_from(euchre_server_core::auth::now_secs()).unwrap_or(u32::MAX);
    let mut state = STATE.lock();
    let next = now.max(floor).max(state.last_version.saturating_add(1));
    state.last_version = next;
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wildcard bind beside a `127.0.0.1` listener on the same port is
    /// refused, so loopback clients never reach the other listener.
    #[tokio::test]
    async fn a_wildcard_bind_beside_a_loopback_squatter_is_refused() -> anyhow::Result<()> {
        let squatter = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = squatter.local_addr()?.port();
        let wildcard = SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, port));
        let err = bind_http(wildcard, true)
            .await
            .err()
            .context("bound beside the squatter")?;
        assert!(
            matches!(
                err.downcast_ref::<BootError>(),
                Some(BootError::HttpPortInUse { addr }) if addr.port() == port
            ),
            "{err:#}"
        );
        drop(squatter);
        let listener = bind_http(wildcard, true).await?;
        assert_eq!(listener.local_addr()?.port(), port);
        Ok(())
    }

    #[test]
    fn versions_only_go_up() {
        let a = next_version(0);
        let b = next_version(0);
        let c = next_version(u32::MAX - 1);
        assert!(b > a, "{a} then {b}");
        assert_eq!(c, u32::MAX - 1);
        assert_eq!(next_version(0), u32::MAX);
    }
}
