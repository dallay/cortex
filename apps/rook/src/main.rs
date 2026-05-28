// rook — AI proxy server entry point
//
// Boots the dependency injection container and starts the HTTP server.

mod config;
mod di;
mod server;

use anyhow::Context;
use std::path::PathBuf;

use observability::init_tracing;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Init tracing
    init_tracing();

    // 2. Load config
    let config_path = std::env::var("ROOK_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("nuxa")
                .join("rook.toml")
        });

    let config = config::RookConfig::load(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    tracing::info!(config = ?config, "configuration loaded");

    // 3. Build DI container
    let container = di::RookContainer::build(&config).context("failed to build container")?;

    tracing::info!("container built successfully");

    // 4. Start HTTP server
    server::run(container, config.server).await
}
