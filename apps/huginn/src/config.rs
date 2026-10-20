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

/// Shared resolution order: readers and writers must agree on one file.
fn resolve_with(explicit: Option<&Path>, primary: &Path, legacy: &Path) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    if primary.exists() {
        return primary.to_path_buf();
    }
    if legacy.exists() {
        return legacy.to_path_buf();
    }
    primary.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_prefers_explicit_then_primary_then_legacy() {
        let root = tempfile::tempdir().unwrap();
        let primary = root.path().join("huginn/config.toml");
        let legacy = root.path().join("agent/config.toml");
        std::fs::create_dir_all(primary.parent().unwrap()).unwrap();
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();

        // No file exists: writers create the primary path.
        assert_eq!(resolve_with(None, &primary, &legacy), primary);

        // Only the legacy file exists: readers and writers use it.
        std::fs::write(&legacy, "[provider]\n").unwrap();
        assert_eq!(resolve_with(None, &primary, &legacy), legacy);

        // The primary file wins once it exists.
        std::fs::write(&primary, "[ui]\n").unwrap();
        assert_eq!(resolve_with(None, &primary, &legacy), primary);

        // An explicit path never falls back.
        let explicit = root.path().join("elsewhere.toml");
        assert_eq!(resolve_with(Some(&explicit), &primary, &legacy), explicit);
    }

    #[test]
    fn legacy_config_seeds_the_primary_file_and_stays_untouched() {
        let root = tempfile::tempdir().unwrap();
        let primary = root.path().join("huginn/config.toml");
        let legacy = root.path().join("agent/config.toml");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(
            &legacy,
            "provider = \"openai\"\nmodel = \"gpt-4o\"\n\n[ui.theme]\nname = \"dracula\"\n",
        )
        .unwrap();
        let legacy_bytes = std::fs::read_to_string(&legacy).unwrap();

        // Mirrors `tui_chat`: read through the resolver, write to the primary,
        // seed from whatever the process actually loaded.
        let read_path = resolve_with(None, &primary, &legacy);
        assert_eq!(read_path, legacy);
        let loaded = Config::load(Some(&read_path)).unwrap();
        crate::ui_settings::persist_seeded(&loaded.ui, &primary, Some(&read_path)).unwrap();

        // The deprecated file is never modified.
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), legacy_bytes);
        // The new file keeps the whole configuration, not only `[ui]`.
        let reloaded = Config::load(Some(&primary)).unwrap();
        assert_eq!(reloaded.provider, "openai");
        assert_eq!(reloaded.model.as_deref(), Some("gpt-4o"));
        assert_eq!(reloaded.ui, loaded.ui);
        // The next start reads the primary path and stops warning.
        assert_eq!(resolve_with(None, &primary, &legacy), primary);
    }

    #[test]
    fn ui_defaults_fill_absent_sections_and_partial_nested_tables() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        for (text, expected) in [
            ("", UiConfig::defaults()),
            ("[ui]\n", UiConfig::defaults()),
            ("[ui.theme]\nname = 'dracula'\n", {
                let mut expected = UiConfig::defaults();
                expected.theme.name = "dracula".into();
                expected
            }),
            ("[ui.indicators]\nscrollbar = false\n", {
                let mut expected = UiConfig::defaults();
                expected.indicators.scrollbar = false;
                expected
            }),
            ("[ui.toasts]\nlevel = 'warn'\n", {
                let mut expected = UiConfig::defaults();
                expected.toasts.level = "warn".into();
                expected
            }),
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(
                Config::load(Some(&path)).unwrap().ui,
                expected,
                "input: {text:?}"
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelReadiness {
    Ready,
    Unconfigured,
    Invalid,
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
        dirs::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("cortex/huginn/config.toml")
    }

    pub fn legacy_config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("cortex/agent/config.toml")
    }

    /// Resolve the configuration file that `load` reads.
    ///
    /// Persistence must derive its target from this: patching a path `load`
    /// would not read creates a shadow file that hides an entire legacy
    /// configuration. When this resolves to the legacy path, seed the primary
    /// file from it instead of writing in place.
    pub fn resolve_path(explicit: Option<&Path>) -> PathBuf {
        resolve_with(
            explicit,
            &Self::primary_config_path(),
            &Self::legacy_config_path(),
        )
    }

    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        // Resolution order, all read-only:
        // 1. Explicit --config (highest priority, never falls back).
        // 2. Platform config dir / cortex/huginn/config.toml.
        // 3. Legacy platform config dir / cortex/agent/config.toml — only when
        //    the new path is absent; never overwritten or deleted.
        let resolved = Self::resolve_path(path);
        if path.is_none() && resolved != Self::primary_config_path() {
            eprintln!(
                "warning: using legacy configuration {} — move the file to {} to silence this notice",
                resolved.display(),
                Self::primary_config_path().display()
            );
        }
        // The path comes from the explicit CLI option or fixed platform paths;
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
