// config — load and validate RookConfig from TOML

use rook_core::ApiKeyTier;
use rook_usecases::{PricingConfig, RoutingStrategy};
use serde::Deserialize;
use std::collections::HashMap;
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
    #[serde(default)]
    pub rate_limiting: RateLimiterConfig,
    #[serde(default)]
    #[allow(dead_code)] // TODO: Phase 7 - wire usage recorder and retention sweep
    pub usage: UsageConfig,
    #[serde(default)]
    #[allow(dead_code)] // TODO: Phase 7 - wire pricing through DI
    pub pricing: PricingConfig,
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
            enabled: true,
            allow_env_fallback: true,
        }
    }
}

fn default_allow_env_fallback() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderCrudConfig {
    #[serde(default = "default_provider_crud_enabled")]
    pub enabled: bool,
}

impl Default for ProviderCrudConfig {
    fn default() -> Self {
        Self {
            enabled: default_provider_crud_enabled(),
        }
    }
}

fn default_provider_crud_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct UsageConfig {
    #[serde(default = "default_usage_retention_days")]
    #[allow(dead_code)] // TODO: Phase 7 - wire retention sweep
    pub retention_days: u32,
    #[serde(default = "default_usage_sweep_interval_hours")]
    #[allow(dead_code)] // TODO: Phase 7 - wire retention sweep
    pub sweep_interval_hours: u32,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            retention_days: default_usage_retention_days(),
            sweep_interval_hours: default_usage_sweep_interval_hours(),
        }
    }
}

fn default_usage_retention_days() -> u32 {
    90
}

fn default_usage_sweep_interval_hours() -> u32 {
    6
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// Background health check interval in seconds (default: 30)
    #[serde(default = "default_health_check_interval_secs")]
    pub health_check_interval_secs: u64,
}

fn default_health_check_interval_secs() -> u64 {
    30
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
pub struct RateLimiterConfig {
    #[serde(default = "default_rate_limiting_enabled")]
    pub enabled: bool,
    #[serde(default = "default_tier")]
    pub default_tier: ApiKeyTier,
    #[serde(default)]
    pub tiers: HashMap<ApiKeyTier, TierConfig>,
    #[serde(default)]
    pub ip_limits: IpRateLimitConfig,
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        let mut tiers = HashMap::new();
        // Default values match the original hardcoded tier_params for backward compatibility
        tiers.insert(
            ApiKeyTier::Free,
            TierConfig {
                requests_per_minute: 100, // 100 capacity, ~1.67/s refill (original: 100 cap, 10/s)
                requests_per_day: Some(1000),
                tokens_per_minute: Some(10000),
            },
        );
        tiers.insert(
            ApiKeyTier::Pro,
            TierConfig {
                requests_per_minute: 1000, // 1000 capacity, ~16.67/s refill (original: 1000 cap, 100/s)
                requests_per_day: Some(100000),
                tokens_per_minute: Some(100000),
            },
        );
        tiers.insert(
            ApiKeyTier::Enterprise,
            TierConfig {
                requests_per_minute: 10000, // 10000 capacity, ~166.67/s refill (original: 10000 cap, 1000/s)
                requests_per_day: Some(10000000),
                tokens_per_minute: Some(1000000),
            },
        );
        Self {
            enabled: false,
            default_tier: ApiKeyTier::Free,
            tiers,
            ip_limits: IpRateLimitConfig::default(),
        }
    }
}

fn default_rate_limiting_enabled() -> bool {
    false
}

fn default_tier() -> ApiKeyTier {
    ApiKeyTier::Free
}

#[derive(Debug, Clone, Deserialize)]
pub struct TierConfig {
    pub requests_per_minute: u32,
    pub requests_per_day: Option<u32>,
    pub tokens_per_minute: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct IpRateLimitConfig {
    #[serde(default = "default_ip_requests_per_minute")]
    pub requests_per_minute: u32,
}

fn default_ip_requests_per_minute() -> u32 {
    30
}

impl RookConfig {
    /// Load config from a TOML file, expanding ~ in paths.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?; // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
        let mut config: RookConfig = toml::from_str(&content)?;

        if config.database.db_path.starts_with('~') {
            let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
            config.database.db_path = config
                .database
                .db_path
                .replace('~', home.to_str().unwrap_or(""));
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_check_interval_defaults_to_30() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = false
            ttl_secs = 300
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        assert_eq!(config.server.health_check_interval_secs, 30);
    }

    #[test]
    fn test_health_check_interval_can_be_overridden() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000
            health_check_interval_secs = 10

            [routing]
            strategy = "priority"

            [cache]
            enabled = false
            ttl_secs = 300
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        assert_eq!(config.server.health_check_interval_secs, 10);
    }
}
