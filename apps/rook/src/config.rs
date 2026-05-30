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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // -----------------------------------------------------------------------
    // DatabaseConfig
    // -----------------------------------------------------------------------

    #[test]
    fn database_config_default_path_is_expected() {
        let db = DatabaseConfig::default();
        assert_eq!(db.db_path, "~/.local/share/cortex/rook/rook.db");
    }

    // -----------------------------------------------------------------------
    // ApiKeysAuthConfig
    // -----------------------------------------------------------------------

    #[test]
    fn api_keys_auth_config_default_disables_auth() {
        let cfg = ApiKeysAuthConfig::default();
        assert!(!cfg.enabled);
    }

    #[test]
    fn api_keys_auth_config_default_allows_env_fallback() {
        let cfg = ApiKeysAuthConfig::default();
        assert!(cfg.allow_env_fallback);
    }

    // -----------------------------------------------------------------------
    // ProviderCrudConfig
    // -----------------------------------------------------------------------

    #[test]
    fn provider_crud_config_default_is_disabled() {
        let cfg = ProviderCrudConfig::default();
        assert!(!cfg.enabled);
    }

    // -----------------------------------------------------------------------
    // CacheConfig::ttl
    // -----------------------------------------------------------------------

    #[test]
    fn cache_config_ttl_converts_secs_to_duration() {
        let cfg = CacheConfig {
            enabled: true,
            ttl_secs: 300,
        };
        assert_eq!(cfg.ttl(), Duration::from_secs(300));
    }

    #[test]
    fn cache_config_ttl_zero_secs_yields_zero_duration() {
        let cfg = CacheConfig {
            enabled: false,
            ttl_secs: 0,
        };
        assert_eq!(cfg.ttl(), Duration::ZERO);
    }

    #[test]
    fn cache_config_ttl_one_sec() {
        let cfg = CacheConfig {
            enabled: true,
            ttl_secs: 1,
        };
        assert_eq!(cfg.ttl(), Duration::from_secs(1));
    }

    // -----------------------------------------------------------------------
    // StrategyConfig → RoutingStrategy conversion
    // -----------------------------------------------------------------------

    #[test]
    fn strategy_config_priority_maps_to_routing_strategy() {
        let strategy: RoutingStrategy = StrategyConfig::Priority.into();
        assert!(matches!(strategy, RoutingStrategy::Priority));
    }

    #[test]
    fn strategy_config_round_robin_maps_to_routing_strategy() {
        let strategy: RoutingStrategy = StrategyConfig::RoundRobin.into();
        assert!(matches!(strategy, RoutingStrategy::RoundRobin));
    }

    #[test]
    fn strategy_config_model_based_maps_to_routing_strategy() {
        let strategy: RoutingStrategy = StrategyConfig::ModelBased.into();
        assert!(matches!(strategy, RoutingStrategy::ModelBased));
    }

    // -----------------------------------------------------------------------
    // StrategyConfig serde deserialization (kebab-case)
    // -----------------------------------------------------------------------

    #[test]
    fn strategy_config_deserializes_priority_from_kebab_case() {
        let toml_str = r#"strategy = "priority""#;
        #[derive(serde::Deserialize)]
        struct Wrapper {
            strategy: StrategyConfig,
        }
        let w: Wrapper = toml::from_str(toml_str).unwrap();
        assert!(matches!(w.strategy, StrategyConfig::Priority));
    }

    #[test]
    fn strategy_config_deserializes_round_robin_from_kebab_case() {
        let toml_str = r#"strategy = "round-robin""#;
        #[derive(serde::Deserialize)]
        struct Wrapper {
            strategy: StrategyConfig,
        }
        let w: Wrapper = toml::from_str(toml_str).unwrap();
        assert!(matches!(w.strategy, StrategyConfig::RoundRobin));
    }

    #[test]
    fn strategy_config_deserializes_model_based_from_kebab_case() {
        let toml_str = r#"strategy = "model-based""#;
        #[derive(serde::Deserialize)]
        struct Wrapper {
            strategy: StrategyConfig,
        }
        let w: Wrapper = toml::from_str(toml_str).unwrap();
        assert!(matches!(w.strategy, StrategyConfig::ModelBased));
    }

    // -----------------------------------------------------------------------
    // RookConfig::load
    // -----------------------------------------------------------------------

    fn minimal_config_toml() -> &'static str {
        r#"
[server]
host = "127.0.0.1"
port = 8080

[routing]
strategy = "priority"

[cache]
enabled = false
ttl_secs = 60

[[providers]]
id = "openai-test"
kind = "openai"
api_key = "sk-test"
models = ["gpt-4"]
"#
    }

    fn write_temp_config(content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "rook_test_config_{}.toml",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));
        std::fs::write(&path, content).expect("write temp config");
        path
    }

    #[test]
    fn load_returns_error_for_missing_file() {
        let result = RookConfig::load(std::path::Path::new("/nonexistent/path/config.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn load_parses_minimal_valid_config() {
        let path = write_temp_config(minimal_config_toml());
        let result = RookConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        let config = result.expect("should parse valid config");

        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 8080);
        assert!(!config.cache.enabled);
        assert_eq!(config.cache.ttl_secs, 60);
        assert_eq!(config.providers.len(), 1);
        assert_eq!(config.providers[0].id, "openai-test");
        assert_eq!(config.providers[0].kind, "openai");
    }

    #[test]
    fn load_applies_database_config_default_when_absent() {
        let path = write_temp_config(minimal_config_toml());
        let result = RookConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        let config = result.expect("should parse valid config");

        // database section not in TOML → defaults apply
        assert!(config.database.db_path.contains("rook.db"));
    }

    #[test]
    fn load_applies_auth_config_default_when_absent() {
        let path = write_temp_config(minimal_config_toml());
        let result = RookConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        let config = result.expect("should parse valid config");

        assert!(!config.auth.api_keys.enabled);
        assert!(config.auth.api_keys.allow_env_fallback);
    }

    #[test]
    fn load_applies_provider_crud_default_when_absent() {
        let path = write_temp_config(minimal_config_toml());
        let result = RookConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        let config = result.expect("should parse valid config");

        assert!(!config.provider_crud.enabled);
    }

    #[test]
    fn load_expands_env_var_in_api_key() {
        // Use unique env var name to avoid test pollution
        let var_name = "ROOK_TEST_API_KEY_EXPANSION";
        std::env::set_var(var_name, "sk-expanded-secret");

        let toml = format!(
            r#"
[server]
host = "127.0.0.1"
port = 8080

[routing]
strategy = "priority"

[cache]
enabled = false
ttl_secs = 60

[[providers]]
id = "openai-test"
kind = "openai"
api_key = "${{{}}}"
models = ["gpt-4"]
"#,
            var_name
        );

        let path = write_temp_config(&toml);
        let result = RookConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        std::env::remove_var(var_name);

        let config = result.expect("should parse config");
        assert_eq!(
            config.providers[0].api_key,
            Some("sk-expanded-secret".to_string())
        );
    }

    #[test]
    fn load_env_var_expansion_yields_empty_when_var_unset() {
        let var_name = "ROOK_TEST_UNSET_VAR_FOR_CONFIG";
        std::env::remove_var(var_name);

        let toml = format!(
            r#"
[server]
host = "127.0.0.1"
port = 8080

[routing]
strategy = "priority"

[cache]
enabled = false
ttl_secs = 60

[[providers]]
id = "openai-test"
kind = "openai"
api_key = "${{{}}}"
models = ["gpt-4"]
"#,
            var_name
        );

        let path = write_temp_config(&toml);
        let result = RookConfig::load(&path);
        let _ = std::fs::remove_file(&path);

        let config = result.expect("should parse config");
        assert_eq!(config.providers[0].api_key, Some(String::new()));
    }

    #[test]
    fn load_leaves_literal_api_key_unchanged() {
        let path = write_temp_config(minimal_config_toml());
        let result = RookConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        let config = result.expect("should parse config");

        // Literal "sk-test" doesn't match ${...} pattern, should be unchanged
        assert_eq!(
            config.providers[0].api_key,
            Some("sk-test".to_string())
        );
    }

    #[test]
    fn load_provider_config_optional_fields_default_to_none() {
        let toml = r#"
[server]
host = "127.0.0.1"
port = 9090

[routing]
strategy = "round-robin"

[cache]
enabled = true
ttl_secs = 120

[[providers]]
id = "ollama-local"
kind = "ollama"
models = ["llama2"]
"#;
        let path = write_temp_config(toml);
        let result = RookConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        let config = result.expect("should parse config");

        assert!(config.providers[0].api_key.is_none());
        assert!(config.providers[0].base_url.is_none());
        assert!(config.providers[0].timeout_secs.is_none());
    }

    #[test]
    fn load_returns_error_for_invalid_toml() {
        let path = write_temp_config("this is not valid toml ][[[");
        let result = RookConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn load_tilde_in_db_path_is_expanded() {
        // Only run if we can determine a home directory
        if dirs::home_dir().is_none() {
            return;
        }

        let toml = r#"
[server]
host = "127.0.0.1"
port = 8080

[routing]
strategy = "priority"

[cache]
enabled = false
ttl_secs = 60

[database]
db_path = "~/mydata/rook.db"

[[providers]]
id = "openai-test"
kind = "openai"
api_key = "sk-test"
models = ["gpt-4"]
"#;
        let path = write_temp_config(toml);
        let result = RookConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        let config = result.expect("should parse config");

        // After expansion, tilde should be replaced with home dir path
        assert!(!config.database.db_path.starts_with('~'));
        assert!(config.database.db_path.ends_with("mydata/rook.db"));
    }

    #[test]
    fn load_literal_db_path_without_tilde_is_unchanged() {
        let toml = r#"
[server]
host = "127.0.0.1"
port = 8080

[routing]
strategy = "priority"

[cache]
enabled = false
ttl_secs = 60

[database]
db_path = "/absolute/path/rook.db"

[[providers]]
id = "openai-test"
kind = "openai"
api_key = "sk-test"
models = ["gpt-4"]
"#;
        let path = write_temp_config(toml);
        let result = RookConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        let config = result.expect("should parse config");

        assert_eq!(config.database.db_path, "/absolute/path/rook.db");
    }

    #[test]
    fn provider_config_with_all_optional_fields() {
        let toml = r#"
[server]
host = "0.0.0.0"
port = 3000

[routing]
strategy = "model-based"

[cache]
enabled = true
ttl_secs = 3600

[[providers]]
id = "openai-custom"
kind = "openai"
api_key = "sk-custom"
base_url = "https://custom.openai.example.com"
models = ["gpt-4", "gpt-3.5-turbo"]
timeout_secs = 120
"#;
        let path = write_temp_config(toml);
        let result = RookConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        let config = result.expect("should parse config");

        let p = &config.providers[0];
        assert_eq!(p.id, "openai-custom");
        assert_eq!(p.base_url, Some("https://custom.openai.example.com".to_string()));
        assert_eq!(p.models, vec!["gpt-4", "gpt-3.5-turbo"]);
        assert_eq!(p.timeout_secs, Some(120));
    }
}
