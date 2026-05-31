// rook — AI proxy server entry point
//
// Boots the dependency injection container and starts the HTTP server.

mod config;
mod dashboard;
mod di;
mod server;

use anyhow::Context;
use clap::Parser;
use std::path::PathBuf;

use db_migration::MigrationRunner;
use observability::init_tracing;

#[derive(Parser)]
#[command(name = "rook")]
#[command(about = "AI proxy server", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Seed the admin user with a password (for initial setup or E2E testing)
    SeedAdmin {
        /// The password to set for the admin user
        password: String,
    },
    /// Database migration commands
    Db {
        #[command(subcommand)]
        command: DbCommands,
    },
}

#[derive(clap::Subcommand, Debug)]
enum DbCommands {
    /// Run pending database migrations
    Migrate,
    /// Show current migration status
    Status,
    /// Rollback: create a new migration to undo the last applied migration
    Rollback,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::SeedAdmin { password }) => return seed_admin(password).await,
        Some(Commands::Db { command }) => return run_db_command(command).await,
        None => {}
    }

    start_server().await
}

async fn seed_admin(password: String) -> anyhow::Result<()> {
    use rook_usecases::SetAdminPasswordInput;

    let config = load_config()?;
    let container = di::RookContainer::build(&config)
        .await
        .context("failed to build container")?;

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
    let config = load_config()?;

    // Run pending migrations (fail-fast — server doesn't start if migrations fail)
    di::run_startup_migrations(&config.database.db_path)?;

    let container = di::RookContainer::build(&config)
        .await
        .context("failed to build container")?;

    tracing::info!("container built successfully");
    server::run(container, config.server).await
}

async fn run_db_command(cmd: DbCommands) -> anyhow::Result<()> {
    let config = load_config()?;
    let runner = MigrationRunner::new(&config.database.db_path);

    match cmd {
        DbCommands::Migrate => {
            let count = runner.run()?;
            if count > 0 {
                println!("Applied {} migration(s)", count);
            } else {
                println!("No pending migrations");
            }
        }
        DbCommands::Status => {
            let status = runner.status()?;
            match status.current_version {
                Some(v) => println!("Current version: {}", v),
                None => println!("No migrations applied"),
            }
            if !status.applied.is_empty() {
                println!("\nApplied migrations:");
                for m in &status.applied {
                    println!("  V{}  {}", m.version, m.name);
                }
            }
        }
        DbCommands::Rollback => {
            println!("Refinery does not support automatic rollback.");
            println!("To rollback, create a new migration that undoes the last change:");
            println!("  1. Inspect the last applied migration in _migrations table");
            println!("  2. Run: rook db create-migration <name>");
            println!("  3. Write SQL in the new migration file to undo the change");
            println!("  4. Run: rook db migrate");
        }
    }

    Ok(())
}

fn load_config() -> anyhow::Result<config::RookConfig> {
    let config_path = std::env::var("ROOK_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("cortex")
                .join("rook.toml")
        });

    config::RookConfig::load(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))
}
