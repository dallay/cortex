use agent_runtime::mcp::ServerConfig;
use anyhow::Context;
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
        let root = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("cortex/agent");
        Self {
            provider: "openai".into(),
            base_url: std::env::var("AGENT_BASE_URL").ok(),
            model: std::env::var("AGENT_MODEL").ok(),
            api_key_env: Some("AGENT_API_KEY".into()),
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
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let default = dirs::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("cortex/agent/config.toml");
        let explicit = path.is_some();
        let path = path.unwrap_or(&default);
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).context("invalid agent TOML configuration"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !explicit => {
                Ok(Self::default())
            }
            Err(error) => {
                Err(error).with_context(|| format!("cannot read configuration {}", path.display()))
            }
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
                "set base_url in config, --base-url, or AGENT_BASE_URL"
            );
            anyhow::ensure!(
                self.model.as_ref().is_some_and(|s| !s.trim().is_empty()),
                "set model in config, --model, or AGENT_MODEL"
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
