// rook — AI proxy server entry point
//
// Boots the dependency injection container and starts the HTTP server.

use rook::{config, di, server};

use anyhow::Context;
use clap::Parser;
use rook::startup::{build_bootstrap_banner, load_dotenv};
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
    /// Password is read from $ROOK_SEED_PASSWORD, or prompted from stdin.
    SeedAdmin {
        /// Path to the config file (defaults to $ROOK_CONFIG or ~/.cortex/rook/config.toml)
        #[arg(short, long)]
        config: Option<PathBuf>,
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

    // Extract the explicit config path (if any) so dotenv loads from the right directory
    let explicit_config_path: Option<PathBuf> = match &cli.command {
        Some(Commands::SeedAdmin { config }) => config.clone(),
        _ => None,
    };

    // Load .env from the same directory as the config file (silent failure if missing)
    load_dotenv(explicit_config_path.as_deref());

    match cli.command {
        Some(Commands::Admin { command }) => return run_admin_command(command).await,
        Some(Commands::SeedAdmin { config }) => return seed_admin(config).await,
        Some(Commands::Db { command }) => return run_db_command(command).await,
        None => {}
    }

    start_server().await
}

async fn run_admin_command(command: AdminCommands) -> anyhow::Result<()> {
    match command {
        AdminCommands::Bootstrap => seed_admin(None).await,
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
            let result = container.usecases.bootstrap_status.execute().await;
            let shutdown_result = container.shutdown().await;
            let state = result?;
            shutdown_result?;
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

async fn seed_admin(config_path: Option<PathBuf>) -> anyhow::Result<()> {
    use rook_usecases::SetAdminPasswordInput;

    // Get password from environment variable or prompt from stdin
    let password = std::env::var("ROOK_SEED_PASSWORD").or_else(|_| {
        prompt_password().map_err(|e| {
            anyhow::anyhow!("password required: set ROOK_SEED_PASSWORD or provide via stdin ({e})")
        })
    })?;

    let config = load_config_with_path(config_path)?;
    let container = di::RookContainer::build(&config)
        .await
        .context("failed to build container")?;

    let result = async {
        let input = SetAdminPasswordInput {
            new_password: password,
        };
        container
            .usecases
            .set_admin_password
            .execute(input)
            .await
            .context("failed to set admin password")
    }
    .await;
    let shutdown_result = container.shutdown().await;
    result?;
    shutdown_result?;

    tracing::info!("admin password set successfully");
    Ok(())
}

async fn start_server() -> anyhow::Result<()> {
    rook::banner::print_banner();
    let config = load_config()?;

    // Run pending migrations (fail-fast — server doesn't start if migrations fail)
    di::run_startup_migrations(&config.database.db_path)?;

    let container = di::RookContainer::build(&config)
        .await
        .context("failed to build container")?;

    if let Err(error) = announce_bootstrap_if_needed(&container).await {
        let shutdown_result = container.shutdown().await;
        shutdown_result?;
        return Err(error);
    }

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

fn load_config() -> anyhow::Result<config::RookConfig> {
    let config_path =
        std::env::var("ROOK_CONFIG").map_or_else(|_| config::default_config_path(), PathBuf::from);

    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    config::RookConfig::load(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))
}

fn load_config_with_path(config_path: Option<PathBuf>) -> anyhow::Result<config::RookConfig> {
    let config_path = config_path
        .or_else(|| std::env::var("ROOK_CONFIG").map(PathBuf::from).ok())
        .unwrap_or_else(config::default_config_path);

    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    config::RookConfig::load(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn load_dotenv_reads_from_the_explicit_config_directory() {
        let _guard = ENV_LOCK.lock().expect("environment lock");
        let directory = tempfile::tempdir().expect("temporary directory");
        let config_path = directory.path().join("config.toml");
        let dotenv_path = directory.path().join(".env");
        let variable = format!("ROOK_DOTENV_TEST_{}", std::process::id());
        std::env::remove_var(&variable);
        std::fs::write(
            &dotenv_path,
            format!("{variable}=loaded-from-explicit-config\n"),
        )
        .expect("write controlled dotenv file");

        load_dotenv(Some(&config_path));

        assert_eq!(
            std::env::var(&variable).as_deref(),
            Ok("loaded-from-explicit-config")
        );
        std::env::remove_var(&variable);
    }

    #[test]
    fn load_dotenv_keeps_valid_entries_when_a_line_is_malformed() {
        let _guard = ENV_LOCK.lock().expect("environment lock");
        let directory = tempfile::tempdir().expect("temporary directory");
        let config_path = directory.path().join("config.toml");
        let dotenv_path = directory.path().join(".env");
        let variable = format!("ROOK_DOTENV_INVALID_TEST_{}", std::process::id());
        std::env::remove_var(&variable);
        std::fs::write(
            &dotenv_path,
            format!("{variable}=loaded\nthis line is invalid dotenv syntax\n"),
        )
        .expect("write controlled dotenv file");

        load_dotenv(Some(&config_path));

        assert_eq!(std::env::var(&variable).as_deref(), Ok("loaded"));
        std::env::remove_var(&variable);
    }
}
