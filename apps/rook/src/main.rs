// rook — AI proxy server entry point
//
// Boots the dependency injection container and starts the HTTP server.

mod config;
mod dashboard;
mod di;
mod server;

use anyhow::Context;
use clap::Parser;
use std::io::Write;
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
    /// Admin setup and user management commands
    Admin {
        #[command(subcommand)]
        command: AdminCommands,
    },
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
enum AdminCommands {
    /// Prompt for and set the initial admin password
    Bootstrap,
    /// Create an additional admin user (placeholder; only one admin is currently supported)
    CreateUser {
        /// Email/username for the user to create
        #[arg(long)]
        email: String,
    },
    /// List admin users (placeholder; only bootstrap status is currently supported)
    ListUsers,
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
        Some(Commands::Admin { command }) => return run_admin_command(command).await,
        Some(Commands::SeedAdmin { password }) => return seed_admin(password).await,
        Some(Commands::Db { command }) => return run_db_command(command).await,
        None => {}
    }

    start_server().await
}

async fn run_admin_command(command: AdminCommands) -> anyhow::Result<()> {
    match command {
        AdminCommands::Bootstrap => {
            let password = prompt_password()?;
            seed_admin(password).await
        }
        AdminCommands::CreateUser { email } => {
            anyhow::bail!(
                "creating additional admin users is not supported yet (requested: {email})"
            )
        }
        AdminCommands::ListUsers => {
            let config = load_config()?;
            let container = di::RookContainer::build(&config)
                .await
                .context("failed to build container")?;
            let state = container
                .usecases
                .bootstrap_status
                .execute(std::env::var("ROOK_SETUP_TOKEN").ok())
                .await?;
            println!("initialized: {}", state.is_initialized);
            println!("admin_user_exists: {}", state.admin_user_exists);
            Ok(())
        }
    }
}

fn prompt_password() -> anyhow::Result<String> {
    print!("Admin password: ");
    std::io::stdout().flush()?;
    let mut password = String::new();
    std::io::stdin().read_line(&mut password)?;
    let password = password.trim_end_matches(['\r', '\n']).to_string();
    if password.is_empty() {
        anyhow::bail!("password must not be empty");
    }
    Ok(password)
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

    announce_bootstrap_if_needed(&container).await?;

    tracing::info!("container built successfully");
    server::run(container, config.server).await
}

async fn announce_bootstrap_if_needed(container: &di::RookContainer) -> anyhow::Result<()> {
    let setup_token = std::env::var("ROOK_SETUP_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty());
    let state = container
        .usecases
        .bootstrap_status
        .execute(setup_token.clone())
        .await?;

    if !state.is_initialized {
        match setup_token {
            Some(token) => {
                // Sanitize: replace control/non-printable chars to prevent log injection
                let sanitized: String = token.chars().map(|c| {
                    if c.is_ascii_control() || c == '"' || c == '\\' || c == '\n' || c == '\r' || c == '\t' {
                        '?'
                    } else {
                        c
                    }
                }).collect();
                let preview = if sanitized.len() > 8 {
                    format!("{}…", &sanitized[..8])
                } else {
                    sanitized.clone()
                };
                tracing::warn!(setup_token_preview = %preview, setup_token_len = token.len(), "rook is in bootstrap mode; set the admin password before using the server");
                // Only print full token to interactive TTY; otherwise show preview only
                if atty::is(atty::Stream::Stderr) {
                    eprintln!("rook bootstrap mode: use setup token {token} to set the admin password");
                } else {
                    eprintln!("rook bootstrap mode: use setup token {preview}… (len={}) to set the admin password", token.len());
                }
            }
            None => {
                tracing::warn!("rook is in bootstrap mode; run `rook admin bootstrap` or set ROOK_SETUP_TOKEN and POST /api/bootstrap/setup");
                eprintln!(
                    "rook bootstrap mode: run `rook admin bootstrap` to set the admin password"
                );
            }
        }
    }

    Ok(())
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
