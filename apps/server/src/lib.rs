//! `euchre-server`: the composition root. It starts (or connects to) the engine,
//! serves `euchreTable`, `aiSeat`, `playerProfile` and `health` on one registry,
//! and runs the HTTP API with the `/rivet` gateway in front.
//!
//! ```ignore
//! // Desktop embedded mode.
//! let server = euchre_server::start(ServerConfig::local(app_data_dir)).await?;
//! let base = server.url(); // http://127.0.0.1:<port>
//! // ... and when `server.closed()` resolves (the engine died), restart.
//! server.shutdown().await?;
//! ```
//!
//! The deployed binary (`main.rs`) calls [`run`]: [`ServerConfig::from_env`],
//! the web build in `build/client`, and a drain on SIGTERM.
//!
//! One server runs per process at a time: `euchreTable` and `playerProfile`
//! keep their config process-wide, so every start in one process must use the
//! same secrets (see [`BootError`]).

mod engine;
mod error;
pub mod health_actor;
mod process;
mod server;

pub use engine::MIN_FREE_DISK_BYTES;
pub use error::{BootError, ServerFailure};
pub use euchre_server_core::ServerConfig;
pub use server::{
    DEFAULT_STATIC_DIR, DRAINING_NEW_TABLE, RESTARTING, RunningServer, StartOptions, run, start,
    start_with,
};
