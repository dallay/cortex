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
        /// Path to the config file (defaults to $ROOK_CONFIG or ~/.config/cortex/rook.toml)
        #[arg(short, long)]
        config: Option<PathBuf>,

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
    // Load .env file from repo root (silent failure if missing)
    let dotenv_path = std::path::Path::new("/Users/acosta/Dev/dallay/cortex/.env");
    if dotenv_path.exists() {
        let _ = dotenvy::from_path(dotenv_path);
    }

    init_tracing();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Admin { command }) => return run_admin_command(command).await,
        Some(Commands::SeedAdmin { config, password }) => {
            return seed_admin(config, password).await
        }
        Some(Commands::Db { command }) => return run_db_command(command).await,
        None => {}
    }

    start_server().await
}

async fn run_admin_command(command: AdminCommands) -> anyhow::Result<()> {
    match command {
        AdminCommands::Bootstrap => {
            let password = prompt_password()?;
            seed_admin(None, password).await
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
            let state = container.usecases.bootstrap_status.execute().await?;
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

async fn seed_admin(config_path: Option<PathBuf>, password: String) -> anyhow::Result<()> {
    use rook_usecases::SetAdminPasswordInput;

    let config = load_config_with_path(config_path)?;
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
    let state = container.usecases.bootstrap_status.execute().await?;

    if !state.is_initialized {
        // Read the generated token from the in-memory RwLock (set during container build)
        let setup_token_guard = container.usecases.setup_token.read().await;
        match setup_token_guard.as_ref() {
            Some(token) => {
                // Sanitize: replace control/non-printable chars to prevent log injection
                let sanitized: String = token
                    .chars()
                    .map(|c| {
                        if c.is_ascii_control()
                            || c == '"'
                            || c == '\\'
                            || c == '\n'
                            || c == '\r'
                            || c == '\t'
                        {
                            '?'
                        } else {
                            c
                        }
                    })
                    .collect();
                let preview = if sanitized.chars().count() > 8 {
                    format!("{}…", sanitized.chars().take(8).collect::<String>())
                } else {
                    sanitized.clone()
                };
                tracing::warn!(setup_token_preview = %preview, setup_token_len = token.len(), "rook is in bootstrap mode; set the admin password before using the server");
                let banner = build_bootstrap_banner(&sanitized);
                eprintln!("{banner}");
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

/// Build a dynamically-sized ASCII box for the bootstrap banner.
///
/// Computes the box width from the longest content line so the border aligns
/// regardless of token length.
fn build_bootstrap_banner(sanitized_token: &str) -> String {
    let header = "ROOK -- BOOTSTRAP MODE ACTIVE";
    let dash_url = "http://localhost:5173";
    let instruction = "To complete setup, open the dashboard at";
    let token_label = "Setup token:";

    // Lines to measure — compute max visible width
    let lines: &[&str] = &[
        header,
        instruction,
        dash_url,
        &format!("{} {}", token_label, sanitized_token),
        "along with your desired admin password.",
    ];

    let max_len = lines.iter().map(|l| l.len()).max().unwrap_or(40);
    let width = max_len.max(62); // minimum 62 chars wide

    let border = format!("+{}+", "-".repeat(width));

    let center_pad = |text: &str| {
        let pad = width.saturating_sub(text.len());
        let left = pad / 2;
        format!("|{}{}{}|", " ".repeat(left), text, " ".repeat(pad - left))
    };

    let left_pad = |text: &str| {
        let pad = width.saturating_sub(text.len());
        format!(
            "| {}{}{}|",
            text,
            " ".repeat(pad.saturating_sub(1)),
            " ".repeat(pad.saturating_sub(1))
        )
    };

    let banner = format!(
        "{0}\n{1}\n{0}\n{2}\n{3}\n{4}\n{5}\n{6}\n{0}",
        border,
        center_pad(header),
        left_pad(&format!("{} {}", token_label, sanitized_token)),
        left_pad(""),
        left_pad(instruction),
        left_pad(dash_url),
        left_pad("along with your desired admin password."),
    );
    banner
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

    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    config::RookConfig::load(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))
}

fn load_config_with_path(config_path: Option<PathBuf>) -> anyhow::Result<config::RookConfig> {
    let config_path = config_path
        .or_else(|| std::env::var("ROOK_CONFIG").map(PathBuf::from).ok())
        .unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("cortex")
                .join("rook.toml")
        });

    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    config::RookConfig::load(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))
}
