//! Host configuration and UI preference persistence.
pub mod config;
pub mod ui_settings;

use std::path::PathBuf;

pub fn resolve_workspace(explicit: Option<PathBuf>) -> std::io::Result<PathBuf> {
    explicit.map_or_else(std::env::current_dir, Ok)
}
