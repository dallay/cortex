// Startup helpers for the Rook binary.
//
// The bootstrap banner and the dotenv loader are startup behaviour of the
// composition root. They live in the library so that dedicated integration
// test targets can cover them, instead of an inline `#[cfg(test)]` module in
// the binary crate.

use std::path::{Path, PathBuf};

use crate::config;

/// Load .env from the same directory as the config file.
///
/// Tries locations in order:
/// 1. Same directory as the explicit config path (from CLI `--config` or `$ROOK_CONFIG`)
/// 2. `~/.cortex/rook/` (default config directory)
/// 3. Current working directory
///
/// Silently fails if no .env is found at any location.
/// If a found .env fails to load, logs a warning and continues.
pub fn load_dotenv(config_path: Option<&Path>) {
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
                Err(dotenvy::Error::LineParse(_, position)) => {
                    tracing::warn!(
                        env_file = %candidate.display(),
                        position,
                        "failed to parse .env file, continuing without it"
                    );
                }
                Err(dotenvy::Error::Io(error)) => {
                    tracing::warn!(
                        env_file = %candidate.display(),
                        error_kind = ?error.kind(),
                        "failed to read .env file, continuing without it"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        env_file = %candidate.display(),
                        "failed to load .env file, continuing without it"
                    );
                }
            }
            return;
        }
    }
}

/// Build a dynamically-sized ASCII box for the bootstrap banner.
///
/// Computes the box width from the longest content line so the border aligns
/// regardless of token length. The full token is displayed because this banner
/// goes to stderr (local console only), never to remote logs or the HTTP API.
pub fn build_bootstrap_banner(sanitized_token: &str) -> String {
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
    let width = (max_len + 1).max(62); // minimum 62 chars wide

    let border = format!("+{}+", "-".repeat(width));

    let center_pad = |text: &str| {
        let pad = width.saturating_sub(text.len());
        let left = pad / 2;
        format!("|{}{}{}|", " ".repeat(left), text, " ".repeat(pad - left))
    };

    let left_pad = |text: &str| format!("| {}{}|", text, " ".repeat(width - text.len() - 1));

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
