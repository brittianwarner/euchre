//! `euchre-server` binary: configuration from the environment, logs to stderr
//! (`RUST_LOG`, default [`DEFAULT_LOG`]), then run until SIGTERM or ctrl-c.

use tracing_subscriber::EnvFilter;

/// `info`, with rivetkit's per-action, per-request and per-message logs held at
/// `warn`; the envoy's connection and start/stop logs stay (`RUST_LOG=info`
/// brings everything back).
const DEFAULT_LOG: &str = "info,rivetkit_core::actor=warn,rivetkit_core::registry=warn,\
rivet_envoy_client=warn,rivet_envoy_client::connection=info,rivet_envoy_client::envoy=info";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG)),
        )
        .with_writer(std::io::stderr)
        .init();
    euchre_server::run(euchre_server::ServerConfig::from_env()?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_log_filter_parses() {
        let filter = EnvFilter::try_new(DEFAULT_LOG).map(|f| f.to_string());
        assert!(filter.is_ok(), "{filter:?}");
        assert!(!DEFAULT_LOG.contains(char::is_whitespace), "{DEFAULT_LOG}");
    }
}
