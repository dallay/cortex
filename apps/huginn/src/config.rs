use anyhow::Context;
use huginn_runtime::mcp::ServerConfig;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub provider: String,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub db: PathBuf,
    pub timeout_secs: u64,
    pub tool_timeout_secs: u64,
    pub max_iterations: usize,
    pub context_tokens: usize,
    pub max_output_tokens: u32,
    pub mcp: Vec<ServerConfig>,
}
impl Default for Config {
    fn default() -> Self {
        let data_root = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
        let canonical_data = data_root.join("cortex/huginn");
        let legacy_data = data_root.join("cortex/agent");
        // Keep using an existing legacy SQLite database in place. Never copy
        // a live database: WAL/SHM files and SQLite's own locks must remain
        // beside the original database.
        let root = if !canonical_data.join("sessions.db").exists()
            && legacy_data.join("sessions.db").exists()
        {
            legacy_data
        } else {
            canonical_data
        };
        Self {
            provider: "openai".into(),
            base_url: None,
            model: None,
            api_key_env: Some("HUGINN_API_KEY".into()),
            db: root.join("sessions.db"),
            timeout_secs: 120,
            tool_timeout_secs: 60,
            max_iterations: 20,
            context_tokens: 32768,
            max_output_tokens: 4096,
            mcp: vec![],
        }
    }
}
impl Config {
    /// Fill any unset optional fields from the canonical `HUGINN_*`
    /// environment variables, falling back to the legacy `AGENT_*` names
    /// with a one-shot deprecation warning. Custom values explicitly
    /// configured in TOML or via CLI flags are preserved.
    pub fn fill_from_env(&mut self) {
        if self.base_url.is_none() {
            self.base_url = Self::env_with_legacy_fallback("HUGINN_BASE_URL", "AGENT_BASE_URL");
        }
        if self.model.is_none() {
            self.model = Self::env_with_legacy_fallback("HUGINN_MODEL", "AGENT_MODEL");
        }
    }

    /// Resolve the API key from the configured env var, falling back to
    /// the legacy `AGENT_API_KEY` only when the canonical name was left
    /// at its default. Custom env names are not migrated.
    pub fn resolve_api_key(&self) -> anyhow::Result<Option<String>> {
        let Some(name) = self.api_key_env.as_deref().filter(|name| !name.is_empty()) else {
            return Ok(None);
        };
        if let Ok(value) = std::env::var(name) {
            return Ok(Some(value));
        }
        if name == "HUGINN_API_KEY" {
            if let Ok(value) = std::env::var("AGENT_API_KEY") {
                eprintln!("warning: AGENT_API_KEY is deprecated; use HUGINN_API_KEY");
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    fn env_with_legacy_fallback(canonical: &str, legacy: &str) -> Option<String> {
        if let Ok(value) = std::env::var(canonical) {
            return Some(value);
        }
        if let Ok(value) = std::env::var(legacy) {
            eprintln!("warning: {legacy} is deprecated; use {canonical}");
            return Some(value);
        }
        None
    }

    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        // Resolution order, all read-only:
        // 1. Explicit --config (highest priority, never falls back).
        // 2. Platform config dir / cortex/huginn/config.toml.
        // 3. Legacy platform config dir / cortex/agent/config.toml — only when
        //    the new path is absent; never overwritten or deleted.
        let primary_default = dirs::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("cortex/huginn/config.toml");
        let legacy_default = dirs::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("cortex/agent/config.toml");
        let explicit = path.is_some();
        let primary_path = path.unwrap_or(&primary_default);
        match std::fs::read_to_string(primary_path) {
            Ok(text) => toml::from_str(&text).context("invalid Huginn TOML configuration"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !explicit => {
                if legacy_default != primary_default && legacy_default.exists() {
                    eprintln!(
                        "warning: using legacy configuration {} — move the file to {} to silence this notice",
                        legacy_default.display(),
                        primary_default.display()
                    );
                    let text = std::fs::read_to_string(&legacy_default).with_context(|| {
                        format!(
                            "cannot read legacy configuration {}",
                            legacy_default.display()
                        )
                    })?;
                    toml::from_str(&text).context("invalid Huginn TOML configuration")
                } else {
                    Ok(Self::default())
                }
            }
            Err(error) => Err(error)
                .with_context(|| format!("cannot read configuration {}", primary_path.display())),
        }
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(self.provider.as_str(), "openai" | "mock"),
            "provider must be openai or mock"
        );
        if self.provider == "openai" {
            anyhow::ensure!(
                self.base_url.as_ref().is_some_and(|s| !s.trim().is_empty()),
                "set base_url in config, --base-url, HUGINN_BASE_URL, or AGENT_BASE_URL"
            );
            anyhow::ensure!(
                self.model.as_ref().is_some_and(|s| !s.trim().is_empty()),
                "set model in config, --model, HUGINN_MODEL, or AGENT_MODEL"
            );
        }
        anyhow::ensure!(
            (1..=3600).contains(&self.timeout_secs) && (1..=3600).contains(&self.tool_timeout_secs),
            "timeouts must be between 1 and 3600 seconds"
        );
        anyhow::ensure!(
            (1..=100).contains(&self.max_iterations),
            "max_iterations must be between 1 and 100"
        );
        anyhow::ensure!(
            (1024..=1_000_000).contains(&self.context_tokens)
                && self.max_output_tokens > 0
                && (self.max_output_tokens as usize) < self.context_tokens / 2,
            "invalid context/output token limits"
        );
        Ok(())
    }
}
