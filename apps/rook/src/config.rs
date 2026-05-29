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
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub provider_crud: ProviderCrudConfig,
    pub providers: Vec<ProviderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    #[serde(rename = "db_path")]
    pub db_path: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            db_path: "~/.local/share/cortex/rook/rook.db".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthConfig {
    #[serde(default)]
    pub api_keys: ApiKeysAuthConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiKeysAuthConfig {
    pub enabled: bool,
    #[serde(default = "default_allow_env_fallback")]
    pub allow_env_fallback: bool,
}

impl Default for ApiKeysAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_env_fallback: true,
        }
    }
}

fn default_allow_env_fallback() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProviderCrudConfig {
    pub enabled: bool,
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

        if config.database.db_path.starts_with('~') {
            let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
            config.database.db_path = config
                .database
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
