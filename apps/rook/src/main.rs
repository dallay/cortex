// rook — AI proxy server entry point
//
// Boots the dependency injection container and starts the HTTP server.

mod config;
mod dashboard;
mod di;
mod server;

use anyhow::Context;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use observability::init_tracing;

#[derive(Parser)]
#[command(name = "rook")]
#[command(about = "AI proxy server", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Seed the admin user with a password (for initial setup or E2E testing)
    SeedAdmin {
        /// The password to set for the admin user
        password: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Init tracing
    init_tracing();

    let cli = Cli::parse();

    // Handle CLI subcommands
    if let Some(Commands::SeedAdmin { password }) = cli.command {
        return seed_admin(password).await;
    }

    // Default: start the server
    start_server().await
}

async fn seed_admin(password: String) -> anyhow::Result<()> {
    use rook_usecases::SetAdminPasswordInput;

    // Load config
    let config_path = std::env::var("ROOK_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("cortex")
                .join("rook.toml")
        });

    let config = config::RookConfig::load(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    tracing::info!(config = ?config, "configuration loaded");

    // Build DI container
    let container = di::RookContainer::build(&config)
        .await
        .context("failed to build container")?;

    // Set admin password
    let input = SetAdminPasswordInput {
        new_password: password,
    };
    container
        .usecases
        .set_admin_password
        .execute(input)
        .await
        .context("failed to set admin password")?;

    tracing::info!("admin password set successfully");
    Ok(())
}

async fn start_server() -> anyhow::Result<()> {
    // 1. Load config
    let config_path = std::env::var("ROOK_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("cortex")
                .join("rook.toml")
        });

    let config = config::RookConfig::load(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    tracing::info!(config = ?config, "configuration loaded");

    // 2. Build DI container
    let container = di::RookContainer::build(&config)
        .await
        .context("failed to build container")?;

    tracing::info!("container built successfully");

    // 3. Start HTTP server
    server::run(container, config.server).await
}
