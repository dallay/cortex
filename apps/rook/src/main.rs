// rook — AI proxy server entry point
//
// Boots the dependency injection container and starts the HTTP server.

use rook::{config, di, server};

use anyhow::Context;
use clap::Parser;
use std::io::Write;
use std::path::{Path, PathBuf};

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

/// Load .env from the same directory as the config file.
///
/// Tries locations in order:
/// 1. Same directory as the explicit config path (from CLI `--config` or `$ROOK_CONFIG`)
/// 2. `~/.cortex/rook/` (default config directory)
/// 3. Current working directory
///
/// Silently fails if no .env is found at any location.
/// If a found .env fails to load, logs a warning and continues.
fn load_dotenv(config_path: Option<&Path>) {
    let candidates: Vec<PathBuf> = {
        // 1. Same directory as the explicit config path
        let from_config = config_path
            .and_then(|p| p.parent())
            .map(|dir| dir.join(".env"));

        // Fallback: same directory as $ROOK_CONFIG (if no explicit --config was passed)
        let from_env_config = std::env::var("ROOK_CONFIG")
            .ok()
            .and_then(|p| PathBuf::from(p).parent().map(|dir| dir.join(".env")));

        // 2. Default config directory (~/.cortex/rook/)
        let from_default = config::cortex_home_dir()
            .join(config::ROOK_HOME_DIR)
            .join(".env");

        // 3. Current working directory
        let from_cwd = std::env::current_dir().ok().map(|cwd| cwd.join(".env"));

        [from_config, from_env_config, Some(from_default), from_cwd]
            .into_iter()
            .flatten()
            .collect()
    };

    for candidate in &candidates {
        if candidate.exists() {
            match dotenvy::from_path(candidate) {
                Ok(()) => {
                    tracing::debug!(env_file = %candidate.display(), "loaded environment variables");
                }
                Err(e) => {
                    tracing::warn!(
                        env_file = %candidate.display(),
                        error = %e,
                        "failed to load .env file, continuing without it"
                    );
                }
            }
            return;
        }
    }
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

/// Return a truncated preview of a token: first 8 chars + asterisks + last 4 chars.
/// For short tokens (≤ 12 chars), show the first 4 chars masked with asterisks.
#[allow(dead_code)]
fn truncate_token(token: &str) -> String {
    let len = token.chars().count();
    if len <= 12 {
        let visible: String = token.chars().take(4).collect();
        format!("{}****", visible)
    } else {
        let start: String = token.chars().take(8).collect();
        let end: String = token.chars().skip(len.saturating_sub(4)).collect();
        format!("{}****{}", start, end)
    }
}

/// Build a dynamically-sized ASCII box for the bootstrap banner.
///
/// Computes the box width from the longest content line so the border aligns
/// regardless of token length. The full token is displayed because this banner
/// goes to stderr (local console only), never to remote logs or the HTTP API.
fn build_bootstrap_banner(sanitized_token: &str) -> String {
    let header = "ROOK -- BOOTSTRAP MODE ACTIVE";
    let dash_url = "http://localhost:4747";
    let instruction = "To complete setup, open the dashboard at";
    let token_label = "Setup token:";
    let token_display = sanitized_token;

    // Lines to measure — compute max visible width
    let lines: &[&str] = &[
        header,
        instruction,
        dash_url,
        &format!("{} {}", token_label, token_display),
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

    format!(
        "{0}\n{1}\n{0}\n{2}\n{3}\n{4}\n{5}\n{6}\n{7}\n{0}",
        border,
        center_pad(header),
        left_pad(&format!("{} {}", token_label, token_display)),
        left_pad(""),
        left_pad("Paste it into the dashboard to complete setup."),
        left_pad(instruction),
        left_pad(dash_url),
        left_pad("along with your desired admin password."),
    )
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

    #[test]
    fn truncate_token_shows_full_preview_for_long_tokens() {
        // nosemgrep: rust.generic-secret.generic-secret
        let token = "test_token_abcdefghijklmnopqrstuvwxyz0123456789";
        let result = truncate_token(token);
        assert!(result.starts_with("test_tok"));
        assert!(result.ends_with("6789"));
        assert!(result.contains("****"));
        assert!(!result.contains("ijklmnop")); // middle is hidden
    }

    #[test]
    fn truncate_token_masks_short_tokens() {
        // nosemgrep: rust.generic-secret.generic-secret
        let token = "short";
        let result = truncate_token(token);
        assert_eq!(result, "shor****");
    }

    #[test]
    fn truncate_token_exactly_12_chars_is_short() {
        // nosemgrep: rust.generic-secret.generic-secret
        let token = "abcdefghij01";
        let result = truncate_token(token);
        assert_eq!(result, "abcd****");
    }

    #[test]
    fn truncate_token_13_chars_is_long() {
        // nosemgrep: rust.generic-secret.generic-secret
        let token = "abcdefghijklm";
        let result = truncate_token(token);
        assert!(result.starts_with("abcdefgh"));
        assert!(result.ends_with("klm"));
    }

    #[test]
    fn truncate_token_empty_string() {
        let result = truncate_token("");
        assert_eq!(result, "****");
    }
}
