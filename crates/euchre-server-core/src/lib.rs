//! euchre-server-core: the shared server foundation (lane SERVER-FOUNDATION).
//!
//! Configuration and secrets ([`config`]), the token families ([`auth`]),
//! rivetkit helpers shared by every actor and client ([`rivet`]), and the
//! local-engine test harness ([`testing`]). Every server crate builds on it;
//! see `docs/port/ARCHITECTURE.md` (D29).

pub mod auth;
pub mod config;
pub mod rivet;
pub mod testing;

pub use config::ServerConfig;
