// config — load and validate RookConfig from TOML

use rook_usecases::RoutingStrategy;
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

/// Root configuration for rook
#[derive(Debug, Clone, Deserialize)]
pub struct RookConfig {
    pub server: ServerConfig,
    pub routing: RoutingConfig,
    pub cache: CacheConfig,
    pub audit: AuditConfig,
    #[serde(default)]
    pub provider_crud: ProviderCrudConfig,
    pub providers: Vec<ProviderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderCrudConfig {
    pub enabled: bool,
    #[serde(rename = "db_path")]
    pub db_path: String,
}

impl Default for ProviderCrudConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            db_path: "~/.local/share/cortex/rook/providers.db".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoutingConfig {
    pub strategy: StrategyConfig,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StrategyConfig {
    Priority,
    RoundRobin,
    ModelBased,
}

impl From<StrategyConfig> for RoutingStrategy {
    fn from(s: StrategyConfig) -> Self {
        match s {
            StrategyConfig::Priority => RoutingStrategy::Priority,
            StrategyConfig::RoundRobin => RoutingStrategy::RoundRobin,
            StrategyConfig::ModelBased => RoutingStrategy::ModelBased,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    pub enabled: bool,
    #[serde(rename = "ttl_secs")]
    pub ttl_secs: u64,
}

impl CacheConfig {
    pub fn ttl(&self) -> Duration {
        Duration::from_secs(self.ttl_secs)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuditConfig {
    #[serde(rename = "db_path")]
    pub db_path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: String,
    pub api_key: Option<String>,
    #[serde(rename = "base_url")]
    pub base_url: Option<String>,
    pub models: Vec<String>,
    #[serde(rename = "timeout_secs")]
    pub timeout_secs: Option<u64>,
}

impl RookConfig {
    /// Load config from a TOML file, expanding ~ in paths.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: RookConfig = toml::from_str(&content)?;

        // Expand ~ in audit db path
        if config.audit.db_path.starts_with('~') {
            let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
            config.audit.db_path = config
                .audit
                .db_path
                .replace('~', home.to_str().unwrap_or(""));
        }

        if config.provider_crud.db_path.starts_with('~') {
            let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
            config.provider_crud.db_path = config
                .provider_crud
                .db_path
                .replace('~', home.to_str().unwrap_or(""));
        }

        // Expand env vars in api_key values (${VAR} syntax)
        for provider in &mut config.providers {
            if let Some(ref key) = provider.api_key {
                if key.starts_with("${") && key.ends_with('}') {
                    let var = &key[2..key.len() - 1];
                    provider.api_key = Some(std::env::var(var).unwrap_or_default());
                }
            }
        }

        Ok(config)
    }
}
