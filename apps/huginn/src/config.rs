use anyhow::Context;
use huginn_presentation::settings::UiConfig;
use huginn_runtime::mcp::ServerConfig;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub provider: String,
    pub base_url: Option<String>,
    pub allow_loopback_http: bool,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub db: PathBuf,
    pub timeout_secs: u64,
    pub tool_timeout_secs: u64,
    pub max_iterations: usize,
    pub context_tokens: usize,
    pub max_output_tokens: u32,
    pub mcp: Vec<ServerConfig>,
    /// TUI preferences — populated from `[ui]` in `config.toml`. REQ-10,
    /// REQ-11: default-tolerant; inner structs do **not** carry
    /// `deny_unknown_fields`.
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(skip)]
    pub provider_explicit: bool,
}

/// Name of the per-user directory that holds Cortex configuration and state.
pub const CORTEX_HOME_DIR: &str = ".cortex";

/// Product directory for Huginn inside the Cortex home.
pub const HUGINN_HOME_DIR: &str = "huginn";

/// Resolve `~/.cortex`, the shared base for every Cortex product.
///
/// Uses the operating system home directory instead of the platform
/// configuration directory, so macOS, Linux and Windows agree on one layout.
pub fn cortex_home_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(CORTEX_HOME_DIR)
}

/// Map an optional explicit path to the file that readers and writers use.
pub fn resolve_with(explicit: Option<&Path>, primary: &Path) -> PathBuf {
    explicit.map_or_else(|| primary.to_path_buf(), Path::to_path_buf)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelReadiness {
    Ready,
    Unconfigured,
    Invalid,
}
impl Default for Config {
    fn default() -> Self {
        let root = cortex_home_dir().join(HUGINN_HOME_DIR);
        Self {
            provider: "openai".into(),
            base_url: None,
            allow_loopback_http: false,
            model: None,
            api_key_env: Some("HUGINN_API_KEY".into()),
            db: root.join("sessions.db"),
            timeout_secs: 120,
            tool_timeout_secs: 60,
            max_iterations: 20,
            context_tokens: 32768,
            max_output_tokens: 4096,
            mcp: vec![],
            ui: UiConfig::defaults(),
            provider_explicit: false,
        }
    }
}
impl Config {
    /// Fill any unset optional fields from the canonical `HUGINN_*`
    /// environment variables, falling back to the legacy `AGENT_*` names
    /// with a one-shot deprecation warning. Custom values explicitly
    /// configured in TOML or via CLI flags are preserved.
    pub fn model_readiness(&self) -> ModelReadiness {
        if self.provider == "mock" {
            return ModelReadiness::Ready;
        }
        if self.base_url.is_none() && self.model.is_none() && !self.provider_explicit {
            return ModelReadiness::Unconfigured;
        }
        if self.base_url.is_none() || self.model.is_none() {
            return ModelReadiness::Invalid;
        }
        if self.validate_model().is_ok() {
            ModelReadiness::Ready
        } else {
            ModelReadiness::Invalid
        }
    }

    pub fn require_model(&self) -> anyhow::Result<()> {
        match self.model_readiness() {
            ModelReadiness::Ready => Ok(()),
            ModelReadiness::Unconfigured => anyhow::bail!(
                "No model configured. Set HUGINN_BASE_URL and HUGINN_MODEL in your Huginn configuration."
            ),
            ModelReadiness::Invalid => self.validate_model(),
        }
    }

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

    pub fn primary_config_path() -> PathBuf {
        cortex_home_dir().join(HUGINN_HOME_DIR).join("config.toml")
    }

    /// Resolve the configuration file that `load` reads.
    ///
    /// An explicit `--config` wins; otherwise the file is
    /// `~/.cortex/huginn/config.toml`. Persistence must derive its target from
    /// this so a save never creates a file that `load` would not read.
    pub fn resolve_path(explicit: Option<&Path>) -> PathBuf {
        resolve_with(explicit, &Self::primary_config_path())
    }

    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        // Resolution order, all read-only:
        // 1. Explicit --config (highest priority, never falls back).
        // 2. ~/.cortex/huginn/config.toml.
        let resolved = Self::resolve_path(path);
        // The path comes from the explicit CLI option or a fixed home path;
        // keep it as a PathBuf so it cannot be confused with shell input.
        match std::fs::read_to_string(&resolved) {
            Ok(text) => Self::parse(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && path.is_none() => {
                Ok(Self::default())
            }
            Err(error) => Err(error)
                .with_context(|| format!("cannot read configuration {}", resolved.display())),
        }
    }

    fn parse(text: &str) -> anyhow::Result<Self> {
        let provider_explicit = toml::from_str::<toml::Value>(text)
            .ok()
            .and_then(|value| value.as_table().map(|table| table.contains_key("provider")))
            .unwrap_or(false);
        toml::from_str::<Self>(text)
            .map(|mut config| {
                config.provider_explicit = provider_explicit;
                config
            })
            .context("invalid Huginn TOML configuration")
    }
    pub fn create_model(&self) -> anyhow::Result<std::sync::Arc<dyn huginn_core::ModelProvider>> {
        if self.model_readiness() == ModelReadiness::Unconfigured {
            return Ok(std::sync::Arc::new(
                huginn_runtime::model::UnconfiguredModelProvider,
            ));
        }
        let factories = huginn_runtime::model::ProviderFactories::builtin();
        let key =
            if factories.uses_credentials(&self.provider)? {
                match self.api_key_env.as_deref().filter(|name| !name.is_empty()) {
                    Some(name) => Some(self.resolve_api_key()?.with_context(|| {
                        format!("credential environment variable {name} is unset")
                    })?),
                    None => None,
                }
            } else {
                None
            };
        Ok(factories.create(
            &self.provider,
            &huginn_runtime::model::ProviderConfig {
                base_url: self.base_url.clone(),
                model: self.model.clone(),
                key,
                timeout_secs: self.timeout_secs,
                allow_loopback_http: self.allow_loopback_http,
            },
        )?)
    }
    pub fn validate_common(&self) -> anyhow::Result<()> {
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

    fn validate_model(&self) -> anyhow::Result<()> {
        huginn_runtime::model::ProviderFactories::builtin().create(
            &self.provider,
            &huginn_runtime::model::ProviderConfig {
                base_url: self.base_url.clone(),
                model: self.model.clone(),
                key: None,
                timeout_secs: self.timeout_secs,
                allow_loopback_http: self.allow_loopback_http,
            },
        )?;
        Ok(())
    }
}
