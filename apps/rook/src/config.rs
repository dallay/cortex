// config — load and validate RookConfig from TOML

use rook_core::ApiKeyTier;
use rook_usecases::{PricingConfig, RoutingStrategy};
use serde::{Deserialize, Serialize};
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
    /// Combo (multi-step fallback chain) definitions
    #[serde(default)]
    pub combos: Vec<ComboConfig>,
    /// Model aliases configuration
    #[serde(default)]
    pub model_aliases: ModelAliasesConfig,
    /// Telemetry configuration
    #[serde(default)]
    pub telemetry: TelemetryConfig,
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
pub struct ModelAliasesConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub auto_seed: bool,
}

impl Default for ModelAliasesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_seed: true,
        }
    }
}

impl From<ModelAliasesConfig> for rook_usecases::route_request::ModelAliasesConfig {
    fn from(cfg: ModelAliasesConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            auto_seed: cfg.auto_seed,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelemetryConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_telemetry_max_observations")]
    pub max_observations: usize,
    #[serde(default = "default_telemetry_max_age_seconds")]
    pub max_age_seconds: u64,
    #[serde(default = "default_telemetry_cleanup_interval_seconds")]
    pub cleanup_interval_seconds: u64,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_observations: default_telemetry_max_observations(),
            max_age_seconds: default_telemetry_max_age_seconds(),
            cleanup_interval_seconds: default_telemetry_cleanup_interval_seconds(),
        }
    }
}

fn default_telemetry_max_observations() -> usize {
    1000
}

fn default_telemetry_max_age_seconds() -> u64 {
    3600
}

fn default_telemetry_cleanup_interval_seconds() -> u64 {
    60
}

impl From<TelemetryConfig> for observability::TelemetryConfig {
    fn from(cfg: TelemetryConfig) -> Self {
        use std::time::Duration;
        Self {
            max_observations: cfg.max_observations,
            max_age: Duration::from_secs(cfg.max_age_seconds),
            cleanup_interval: Duration::from_secs(cfg.cleanup_interval_seconds),
        }
    }
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

/// Configuration for a single step in a combo fallback chain
#[derive(Debug, Clone, Deserialize)]
pub struct ComboStepConfig {
    /// Provider ID to use for this step
    pub provider_id: String,
    /// Model to request from the provider
    pub model: String,
    /// Priority order (lower = attempted first, 1-255)
    pub priority: u8,
}

/// Configuration for a combo (multi-step fallback chain)
#[derive(Debug, Clone, Deserialize)]
pub struct ComboConfig {
    /// Unique combo ID (used for X-Rook-Combo header reference)
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Execution strategy (currently only "priority" supported)
    #[serde(default = "default_combo_strategy")]
    pub strategy: String,
    /// Ordered steps to try in fallback order
    pub steps: Vec<ComboStepConfig>,
}

fn default_combo_strategy() -> String {
    "priority".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoutingConfig {
    pub strategy: StrategyConfig,
    /// Optional default combo ID to use when no X-Rook-Combo header is present
    #[serde(default)]
    #[allow(dead_code)] // Used in DI for wiring combo execution
    pub default_combo: Option<String>,
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
    #[serde(default)]
    pub max_entries: Option<usize>,
    /// Layer 1: signature-based request deduplication.
    #[serde(default)]
    pub signature_cache: SignatureCacheConfig,
    /// Layer 2: provider-side token caching.
    /// Fields will be used in WU-2 (Phase 4-5: provider integration).
    #[serde(default)]
    #[allow(dead_code)]
    pub token_cache: TokenCacheConfig,
}

/// Configuration for Layer 1 signature cache (request deduplication).
///
/// Signature cache uses SHA-256 hashing to deduplicate identical requests
/// within the TTL window. When enabled, Rook computes a signature from the
/// normalized request (model, messages, parameters) and checks the cache
/// before forwarding to providers.
///
/// **Inspection endpoints** (`/api/cache/signatures`, `/api/cache/signature/:sig`)
/// allow operators to list cached signatures and retrieve responses by signature
/// for debugging and cache validation.
#[derive(Debug, Clone, Deserialize)]
pub struct SignatureCacheConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Enable GET /api/cache/signatures and GET /api/cache/signature/:sig endpoints.
    #[serde(default = "default_true")]
    pub inspection_endpoints: bool,
}

/// Configuration for Layer 2 token cache (provider-side token caching).
///
/// Token cache leverages provider-side caching (e.g., Anthropic prompt caching)
/// by injecting `cache-control: max-stale=3600` headers into outbound requests.
/// Providers return `x-cache: hit` or `x-cache: miss` headers, which Rook parses
/// to track cache hits and estimate cost savings.
///
/// **Configuration:**
/// - `mode = "auto"`: Enable only for known providers (Anthropic, DeepSeek, Qwen, ZAI)
///   or providers explicitly listed in the `providers` array.
/// - `mode = "always"`: Force enable for ALL providers (use with caution).
/// - `mode = "never"`: Disable provider-side token caching (default).
/// - `providers = []`: Empty list uses default known providers when mode=auto.
///   Non-empty list overrides defaults (prefix matching: "anthropic" matches "anthropic-v2").
///
/// **Cost Savings:**
/// Rook tracks tokens saved via provider-side caching and estimates cost savings
/// using average provider pricing. Metrics appear in `GET /api/cache/stats`.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct TokenCacheConfig {
    /// Cache mode: auto (known providers only), always (all providers), never (disabled).
    #[serde(default = "default_cache_mode")]
    pub mode: CacheMode,
    /// List of provider IDs that support token caching. Empty defaults to known supporting providers.
    /// Supports prefix matching: "anthropic" matches "anthropic", "anthropic-v2", etc.
    #[serde(default)]
    pub providers: Vec<String>,
}

/// Token cache mode — controls when cache-control headers are injected.
///
/// **Mode Behavior:**
/// - `Auto`: Inject `cache-control` only for known providers (Anthropic, DeepSeek, Qwen, ZAI)
///   or providers explicitly listed in `TokenCacheConfig.providers`.
/// - `Always`: Always inject `cache-control` header regardless of provider (experimental).
/// - `Never`: Disable provider-side token caching (default, safest).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheMode {
    /// Inject cache-control only for known supporting providers (Anthropic, DeepSeek, Qwen, ZAI).
    Auto,
    /// Always inject cache-control header regardless of provider.
    Always,
    /// Never inject cache-control header (token caching disabled).
    Never,
}

impl Default for SignatureCacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            inspection_endpoints: true,
        }
    }
}

impl Default for TokenCacheConfig {
    fn default() -> Self {
        Self {
            mode: CacheMode::Never,
            providers: Vec::new(),
        }
    }
}

fn default_cache_mode() -> CacheMode {
    CacheMode::Never
}

impl From<TokenCacheConfig> for rook_usecases::TokenCacheConfig {
    fn from(cfg: TokenCacheConfig) -> Self {
        Self {
            mode: cfg.mode.into(),
            providers: cfg.providers,
        }
    }
}

impl From<CacheMode> for rook_usecases::CacheMode {
    fn from(mode: CacheMode) -> Self {
        match mode {
            CacheMode::Auto => rook_usecases::CacheMode::Auto,
            CacheMode::Always => rook_usecases::CacheMode::Always,
            CacheMode::Never => rook_usecases::CacheMode::Never,
        }
    }
}

impl CacheConfig {
    pub fn ttl(&self) -> Duration {
        Duration::from_secs(self.ttl_secs)
    }

    /// Validate cache configuration at startup
    pub fn validate(&self) -> Result<(), String> {
        // Reject TTL > 24 hours (86400 seconds)
        if self.ttl_secs > 86400 {
            return Err(format!(
                "cache.ttl_secs ({}) exceeds 24h maximum (86400)",
                self.ttl_secs
            ));
        }

        // Reject max_entries = Some(0)
        if let Some(0) = self.max_entries {
            return Err(
                "cache.max_entries must be greater than 0 or None for unlimited".to_string(),
            );
        }

        // Validate signature_cache (currently just structural checks)
        if !self.signature_cache.enabled && self.signature_cache.inspection_endpoints {
            tracing::warn!(
                "cache.signature_cache.inspection_endpoints=true but enabled=false; \
                 inspection endpoints will not return any data"
            );
        }

        Ok(())
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

        // Validate combo configurations
        Self::validate_combos(&config.combos);

        // Validate cache configuration
        config
            .cache
            .validate()
            .map_err(|e| anyhow::anyhow!("invalid cache config: {}", e))?;

        Ok(config)
    }

    /// Validate combo configurations and emit warnings for issues
    fn validate_combos(combos: &[ComboConfig]) {
        use std::collections::{HashMap, HashSet};

        let mut seen_names: HashSet<String> = HashSet::new();
        let mut seen_ids: HashSet<String> = HashSet::new();

        for combo in combos {
            // Check duplicate combo names
            if !seen_names.insert(combo.name.clone()) {
                tracing::warn!(
                    combo_name = %combo.name,
                    "duplicate combo name found in config - later definition will override"
                );
            }

            // Check duplicate combo IDs
            if !seen_ids.insert(combo.id.clone()) {
                tracing::warn!(
                    combo_id = %combo.id,
                    "duplicate combo ID found in config - later definition will override"
                );
            }

            // Validate strategy
            if combo.strategy != "priority" {
                tracing::warn!(
                    combo_name = %combo.name,
                    strategy = %combo.strategy,
                    "unsupported combo strategy - only 'priority' is supported in this version"
                );
            }

            // Check for duplicate priorities within combo
            let mut priorities: HashMap<u8, usize> = HashMap::new();
            for (idx, step) in combo.steps.iter().enumerate() {
                if let Some(prev_idx) = priorities.insert(step.priority, idx) {
                    tracing::warn!(
                        combo_name = %combo.name,
                        priority = step.priority,
                        step_indices = format!("{prev_idx}, {idx}"),
                        "duplicate priority within combo - execution order may be unpredictable"
                    );
                }

                // Validate priority range
                if step.priority == 0 {
                    tracing::warn!(
                        combo_name = %combo.name,
                        step_index = idx,
                        "priority must be between 1 and 255, got 0 - this combo may fail at runtime"
                    );
                }
            }

            // Check step count
            if combo.steps.is_empty() {
                tracing::warn!(
                    combo_name = %combo.name,
                    "combo has no steps - it will fail at runtime"
                );
            }
            if combo.steps.len() > 10 {
                tracing::warn!(
                    combo_name = %combo.name,
                    step_count = combo.steps.len(),
                    "combo has more than 10 steps - maximum is 10, this may fail at runtime"
                );
            }
        }
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

    #[test]
    fn test_combo_config_parses_correctly() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"
            default_combo = "main-chain"

            [cache]
            enabled = false
            ttl_secs = 300

            [[combos]]
            id = "main-chain"
            name = "OpenAI → Anthropic → Ollama"
            strategy = "priority"

              [[combos.steps]]
              provider_id = "openai-primary"
              model = "gpt-4o"
              priority = 1

              [[combos.steps]]
              provider_id = "anthropic-primary"
              model = "claude-opus-4"
              priority = 2

              [[combos.steps]]
              provider_id = "ollama-local"
              model = "llama3"
              priority = 3
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        assert_eq!(config.routing.default_combo, Some("main-chain".to_string()));
        assert_eq!(config.combos.len(), 1);

        let combo = &config.combos[0];
        assert_eq!(combo.id, "main-chain");
        assert_eq!(combo.name, "OpenAI → Anthropic → Ollama");
        assert_eq!(combo.strategy, "priority");
        assert_eq!(combo.steps.len(), 3);

        assert_eq!(combo.steps[0].provider_id, "openai-primary");
        assert_eq!(combo.steps[0].model, "gpt-4o");
        assert_eq!(combo.steps[0].priority, 1);

        assert_eq!(combo.steps[2].provider_id, "ollama-local");
        assert_eq!(combo.steps[2].model, "llama3");
        assert_eq!(combo.steps[2].priority, 3);
    }

    #[test]
    fn test_combo_config_defaults_to_empty() {
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
        assert_eq!(config.combos.len(), 0);
        assert_eq!(config.routing.default_combo, None);
    }

    #[test]
    fn test_combo_strategy_defaults_to_priority() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = false
            ttl_secs = 300

            [[combos]]
            id = "test-combo"
            name = "Test Combo"

              [[combos.steps]]
              provider_id = "openai"
              model = "gpt-4"
              priority = 1
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        assert_eq!(config.combos[0].strategy, "priority");
    }

    // -------------------------------------------------------------------------
    // Dual-Layer Cache Config Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_cache_signature_cache_defaults_to_enabled() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = true
            ttl_secs = 300
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        assert!(config.cache.signature_cache.enabled);
        assert!(config.cache.signature_cache.inspection_endpoints);
    }

    #[test]
    fn test_cache_token_cache_defaults_to_never() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = true
            ttl_secs = 300
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        assert_eq!(config.cache.token_cache.mode, CacheMode::Never);
        assert!(config.cache.token_cache.providers.is_empty());
    }

    #[test]
    fn test_cache_token_cache_mode_parses_auto() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = true
            ttl_secs = 300

            [cache.token_cache]
            mode = "auto"
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        assert_eq!(config.cache.token_cache.mode, CacheMode::Auto);
    }

    #[test]
    fn test_cache_token_cache_mode_parses_always() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = true
            ttl_secs = 300

            [cache.token_cache]
            mode = "always"
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        assert_eq!(config.cache.token_cache.mode, CacheMode::Always);
    }

    #[test]
    fn test_cache_token_cache_mode_parses_never() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = true
            ttl_secs = 300

            [cache.token_cache]
            mode = "never"
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        assert_eq!(config.cache.token_cache.mode, CacheMode::Never);
    }

    #[test]
    fn test_cache_token_cache_providers_parses_list() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = true
            ttl_secs = 300

            [cache.token_cache]
            mode = "auto"
            providers = ["anthropic", "deepseek", "qwen"]
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        assert_eq!(
            config.cache.token_cache.providers,
            vec!["anthropic", "deepseek", "qwen"]
        );
    }

    #[test]
    fn test_cache_signature_cache_can_be_disabled() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = true
            ttl_secs = 300

            [cache.signature_cache]
            enabled = false
            inspection_endpoints = false
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        assert!(!config.cache.signature_cache.enabled);
        assert!(!config.cache.signature_cache.inspection_endpoints);
    }

    #[test]
    fn test_cache_validate_accepts_valid_config() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = true
            ttl_secs = 300
            max_entries = 1000

            [cache.signature_cache]
            enabled = true
            inspection_endpoints = true

            [cache.token_cache]
            mode = "auto"
            providers = ["anthropic"]
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        assert!(config.cache.validate().is_ok());
    }

    #[test]
    fn test_cache_validate_rejects_ttl_over_24h() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = true
            ttl_secs = 86401
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        let err = config.cache.validate().unwrap_err();
        assert!(err.contains("86401"));
        assert!(err.contains("24h"));
    }

    #[test]
    fn test_cache_validate_rejects_zero_max_entries() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = true
            ttl_secs = 300
            max_entries = 0
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        let err = config.cache.validate().unwrap_err();
        assert!(err.contains("max_entries"));
    }

    #[test]
    fn test_cache_token_cache_invalid_mode_rejected_by_toml_parser() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = true
            ttl_secs = 300

            [cache.token_cache]
            mode = "invalid_mode"
        "#;

        let result: Result<RookConfig, _> = toml::from_str(toml);
        assert!(
            result.is_err(),
            "Expected TOML parsing to reject invalid CacheMode"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("unknown variant") || err_msg.contains("invalid_mode"));
    }

    #[test]
    fn test_cache_token_cache_empty_defaults() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            port = 3000

            [routing]
            strategy = "priority"

            [cache]
            enabled = true
            ttl_secs = 300
        "#;

        let config: RookConfig = toml::from_str(toml).expect("parse config");
        // When not specified, token_cache defaults to CacheMode::Never and empty providers list
        assert_eq!(config.cache.token_cache.mode, CacheMode::Never);
        assert!(config.cache.token_cache.providers.is_empty());
    }
}
