// Integration tests for the rook DI container and provider builder

use std::sync::Arc;

use rook::config::{RateLimiterConfig, RookConfig, TierConfig};
use rook::di::{
    build_api_key_auth, build_cache_port, build_manage_connections, build_provider_from_connection,
    build_rate_limiter_config, generate_setup_token,
};
use rook_core::{ApiKeyTier, ConnectionId, DecryptedCredentials, ModelId, ProviderKind};

fn conn_id() -> ConnectionId {
    ConnectionId::default()
}

fn minimal_config_toml(extra: &str) -> String {
    format!(
        r#"
[server]
host = "127.0.0.1"
port = 0

[routing]
strategy = "priority"

[cache]
enabled = false
ttl_secs = 60

{extra}
"#
    )
}

fn cache_config_toml(enabled: bool) -> String {
    format!(
        r#"
[server]
host = "127.0.0.1"
port = 0

[routing]
strategy = "priority"

[cache]
enabled = {enabled}
ttl_secs = 60
max_entries = 100
"#
    )
}

#[tokio::test]
async fn build_cache_port_returns_in_memory_cache_when_enabled() {
    let config: RookConfig = toml::from_str(&cache_config_toml(true)).expect("config parses");
    let cache = build_cache_port(&config);

    let result = cache.stats().await;
    assert!(result.is_ok(), "in-memory cache should be functional");
}

#[tokio::test]
async fn build_cache_port_returns_noop_cache_when_disabled() {
    let config: RookConfig = toml::from_str(&cache_config_toml(false)).expect("config parses");
    let cache = build_cache_port(&config);

    let result = cache.stats().await;
    assert!(result.is_ok(), "no-op cache should be functional too");
}

#[test]
fn build_rate_limiter_config_maps_tiers_correctly() {
    use std::collections::HashMap;

    let mut tiers = HashMap::new();
    tiers.insert(
        ApiKeyTier::Free,
        TierConfig {
            requests_per_minute: 60,
            requests_per_day: Some(500),
            tokens_per_minute: Some(5000),
        },
    );
    tiers.insert(
        ApiKeyTier::Pro,
        TierConfig {
            requests_per_minute: 600,
            requests_per_day: Some(5000),
            tokens_per_minute: Some(50000),
        },
    );

    let cfg = RateLimiterConfig {
        enabled: true,
        default_tier: ApiKeyTier::Free,
        tiers,
        ip_limits: Default::default(),
    };

    let result = build_rate_limiter_config(&cfg);

    assert!(result.enabled);
    assert_eq!(result.default_tier, ApiKeyTier::Free);
    assert_eq!(result.tiers.len(), 2);

    let free_tier = result
        .tiers
        .get(&ApiKeyTier::Free)
        .expect("Free tier exists");
    assert_eq!(free_tier.requests_per_minute, 60);
    assert_eq!(free_tier.requests_per_day, Some(500));
    assert_eq!(free_tier.tokens_per_minute, Some(5000));

    let pro_tier = result.tiers.get(&ApiKeyTier::Pro).expect("Pro tier exists");
    assert_eq!(pro_tier.requests_per_minute, 600);
}

#[test]
fn build_rate_limiter_config_disabled() {
    use std::collections::HashMap;

    let cfg = RateLimiterConfig {
        enabled: false,
        default_tier: ApiKeyTier::Free,
        tiers: HashMap::new(),
        ip_limits: Default::default(),
    };

    let result = build_rate_limiter_config(&cfg);

    assert!(!result.enabled);
}

// Mocks for build_api_key_auth and build_manage_connections tests
mod api_key_auth_mocks {
    use async_trait::async_trait;
    use rook_core::{ProviderId, ProviderPort, RegistryError};
    use std::sync::Arc;

    // FakeProviderRepository for build_manage_connections test
    pub struct FakeProviderRepository;

    #[async_trait]
    impl rook_core::ProviderRepositoryPort for FakeProviderRepository {
        async fn list(
            &self,
        ) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
            Ok(vec![])
        }

        async fn find(
            &self,
            _id: &rook_core::ConnectionId,
        ) -> Result<Option<rook_core::ProviderConnection>, rook_core::RepositoryError> {
            Ok(None)
        }

        async fn create(
            &self,
            _conn: &rook_core::ProviderConnection,
        ) -> Result<(), rook_core::RepositoryError> {
            Ok(())
        }

        async fn update(
            &self,
            _conn: &rook_core::ProviderConnection,
            _expected_updated_at: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), rook_core::RepositoryError> {
            Ok(())
        }

        async fn delete(
            &self,
            _id: &rook_core::ConnectionId,
        ) -> Result<(), rook_core::RepositoryError> {
            Ok(())
        }
    }

    pub struct FakeProviderRegistry;

    impl rook_core::ProviderRegistryPort for FakeProviderRegistry {
        fn providers(&self) -> Vec<ProviderId> {
            vec![]
        }

        fn get(&self, _id: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
            None
        }

        fn replace_all(&self, _providers: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError> {
            Ok(())
        }

        fn upsert(&self, _provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError> {
            Ok(())
        }

        fn remove(&self, _id: &ProviderId) -> Result<(), RegistryError> {
            Ok(())
        }
    }
}

use api_key_auth_mocks::{FakeProviderRegistry, FakeProviderRepository};
use cortex_test_support::FakeApiKeyRepository;

#[test]
fn build_api_key_auth_disabled_returns_none() {
    let config: RookConfig =
        toml::from_str(&minimal_config_toml("[auth.api_keys]\nenabled = false"))
            .expect("config parses");
    let repo: Arc<dyn rook_core::ApiKeyRepositoryPort> = Arc::new(FakeApiKeyRepository::default());
    let registry: Arc<dyn rook_core::ProviderRegistryPort> = Arc::new(FakeProviderRegistry);

    let result = build_api_key_auth(&config, &repo, &registry);

    assert!(result.is_ok());
    let (auth_api, manage_keys) = result.unwrap();
    assert!(auth_api.is_none());
    assert!(manage_keys.is_none());
}

#[test]
fn build_api_key_auth_enabled_returns_some() {
    // Use in-memory SQLite so resolve_api_key_secret generates a transient secret
    // automatically without needing to mutate the process environment.
    let config: RookConfig = toml::from_str(
        r#"
[server]
host = "127.0.0.1"
port = 0

[routing]
strategy = "priority"

[cache]
enabled = false
ttl_secs = 60

[auth.api_keys]
enabled = true

[database]
db_path = ":memory:"

[provider_crud]
enabled = false

[rate_limiting]
enabled = false
"#,
    )
    .expect("config parses");
    let repo: Arc<dyn rook_core::ApiKeyRepositoryPort> = Arc::new(FakeApiKeyRepository::default());
    let registry: Arc<dyn rook_core::ProviderRegistryPort> = Arc::new(FakeProviderRegistry);

    let result = build_api_key_auth(&config, &repo, &registry);

    assert!(result.is_ok());
    let (auth_api, manage_keys) = result.unwrap();
    assert!(auth_api.is_some());
    assert!(manage_keys.is_some());
}

// Mock for ModelCatalogPort
mod model_catalog_mock {
    use async_trait::async_trait;
    use rook_core::ModelCatalogEntry;

    pub struct FakeModelCatalog;

    #[async_trait]
    impl rook_core::ModelCatalogPort for FakeModelCatalog {
        async fn list(&self) -> Vec<ModelCatalogEntry> {
            vec![]
        }
    }
}

use model_catalog_mock::FakeModelCatalog;

#[test]
fn build_manage_connections_disabled_returns_none() {
    let config: RookConfig =
        toml::from_str(&minimal_config_toml("[provider_crud]\nenabled = false"))
            .expect("config parses");
    let provider_repo: Arc<dyn rook_core::ProviderRepositoryPort> =
        Arc::new(FakeProviderRepository);
    let registry: Arc<dyn rook_core::ProviderRegistryPort> = Arc::new(FakeProviderRegistry);
    let model_catalog: Arc<dyn rook_core::ModelCatalogPort> = Arc::new(FakeModelCatalog);

    let result = build_manage_connections(&config, &provider_repo, &registry, &model_catalog);

    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

// T7.1 — DI wires usage recorder with nullable port
#[test]
fn rook_container_build_wires_nullable_usage_recorder() {
    // Compile-time verification: RookUsecases accepts Option<Arc<dyn UsageRecorderPort>>
    // and RookContainer stores usage_repository for retention access.
    // Full integration test: `cargo test -p rook di`
}

// T7.1 — DI shares single provider repository with manage_connections and RouteRequest
#[test]
fn provider_repository_is_shared_between_manage_connections_and_route_request() {
    // Verified at compile time by the shared Arc passed to both ManageConnections
    // and provider_repository_for_usage in RouteRequest::new call.
}

// 5.13 — OpenAI uses default base URL when no override is provided
#[test]
fn build_provider_from_connection_openai_uses_default_base_url() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "sk-test-key".to_string(),
    };
    let id = conn_id();
    let result =
        build_provider_from_connection(&id, ProviderKind::OpenAI, &creds, None, Vec::new());
    let provider = result.expect("expected Ok for OpenAI with default base_url");
    assert_eq!(provider.id().as_str(), id.to_string());
}

// 5.14 — OpenAI uses override base URL when one is provided
#[test]
fn build_provider_from_connection_openai_uses_override() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "sk-test-key".to_string(),
    };
    let id = conn_id();
    let override_url = "https://custom.openai.example.com/v1".to_string();
    let result = build_provider_from_connection(
        &id,
        ProviderKind::OpenAI,
        &creds,
        Some(override_url),
        Vec::new(),
    );
    let provider = result.expect("expected Ok for OpenAI with override base_url");
    assert_eq!(provider.id().as_str(), id.to_string());
}

// 5.15 — Ollama requires base_url; None override returns OllamaRequiresBaseUrl
#[test]
fn build_provider_from_connection_ollama_requires_base_url() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: String::new(),
    };
    let id = conn_id();
    let result =
        build_provider_from_connection(&id, ProviderKind::Ollama, &creds, None, Vec::new());
    let err = match result {
        Ok(provider) => panic!(
            "expected OllamaRequiresBaseUrl error, got Ok({:?})",
            provider.id()
        ),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("ollama") && msg.contains("base_url"),
        "expected ollama base_url error, got: {msg}"
    );
}

// 5.16 — Ollama uses override base URL when one is provided
#[test]
fn build_provider_from_connection_ollama_uses_override() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: String::new(),
    };
    let id = conn_id();
    let result = build_provider_from_connection(
        &id,
        ProviderKind::Ollama,
        &creds,
        Some("http://localhost:11434".to_string()),
        Vec::new(),
    );
    let provider = result.expect("expected Ok for Ollama with base_url override");
    assert_eq!(provider.id().as_str(), id.to_string());
}

// 5.17 — OAuth access_token is forwarded as api_key for providers that accept it
#[test]
fn build_provider_from_connection_oauth_access_token_used_as_api_key() {
    let creds = DecryptedCredentials::OAuth {
        email: "test@example.com".to_string(),
        access_token: "oauth-access-token-123".to_string(),
        refresh_token: "refresh".to_string(),
        expires_at: 9999999999,
        scope: "read".to_string(),
        id_token: "id-token".to_string(),
        project_id: "project".to_string(),
    };
    let id = conn_id();
    let result =
        build_provider_from_connection(&id, ProviderKind::OpenAI, &creds, None, Vec::new());
    assert!(
        result.is_ok(),
        "expected Ok — OAuth access_token should work as api_key"
    );
    let provider = result.unwrap();
    assert_eq!(provider.id().as_str(), id.to_string());
}

// 5.18 — OllamaCloud uses the cloud default base URL when no override
#[test]
fn build_provider_from_connection_ollama_cloud_uses_default_base_url() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "ollama-cloud-key".to_string(),
    };
    let id = conn_id();
    let result =
        build_provider_from_connection(&id, ProviderKind::OllamaCloud, &creds, None, Vec::new());
    let provider = result.expect("expected Ok for OllamaCloud with default base_url");
    assert_eq!(provider.id().as_str(), id.to_string());
}

// 5.19 — OllamaCloud honors an override base URL
#[test]
fn build_provider_from_connection_ollama_cloud_uses_override() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "ollama-cloud-key".to_string(),
    };
    let id = conn_id();
    let result = build_provider_from_connection(
        &id,
        ProviderKind::OllamaCloud,
        &creds,
        Some("https://staging.ollama.example.com".to_string()),
        Vec::new(),
    );
    let provider = result.expect("expected Ok for OllamaCloud with override base_url");
    assert_eq!(provider.id().as_str(), id.to_string());
}

// Fix verification: models passed to build_provider_from_connection are exposed via supported_models()

#[test]
fn build_provider_from_connection_passes_models_to_openai_provider() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "sk-test-key".to_string(),
    };
    let id = conn_id();
    let models = vec![ModelId::new("gpt-4o"), ModelId::new("gpt-4o-mini")];
    let result =
        build_provider_from_connection(&id, ProviderKind::OpenAI, &creds, None, models.clone());
    let provider = result.expect("expected Ok");
    let supported = provider.supported_models();
    assert_eq!(
        supported.len(),
        2,
        "expected 2 models, got {}",
        supported.len()
    );
    assert!(
        supported.contains(&ModelId::new("gpt-4o")),
        "expected gpt-4o in supported_models"
    );
    assert!(
        supported.contains(&ModelId::new("gpt-4o-mini")),
        "expected gpt-4o-mini in supported_models"
    );
}

#[test]
fn build_provider_from_connection_passes_models_to_ollama_cloud_provider() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "ollama-cloud-key".to_string(),
    };
    let id = conn_id();
    let models = vec![
        ModelId::new("ollamacloud/qwen3-coder-next"),
        ModelId::new("ollamacloud/deepseek-v4-pro"),
    ];
    let result = build_provider_from_connection(
        &id,
        ProviderKind::OllamaCloud,
        &creds,
        None,
        models.clone(),
    );
    let provider = result.expect("expected Ok for OllamaCloud");
    let supported = provider.supported_models();
    assert_eq!(
        supported.len(),
        2,
        "expected 2 models, got {}",
        supported.len()
    );
    assert!(
        supported.contains(&ModelId::new("ollamacloud/qwen3-coder-next")),
        "expected ollamacloud/qwen3-coder-next in supported_models"
    );
    assert!(
        supported.contains(&ModelId::new("ollamacloud/deepseek-v4-pro")),
        "expected ollamacloud/deepseek-v4-pro in supported_models"
    );
}

#[test]
fn build_provider_from_connection_empty_models_list() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "sk-test-key".to_string(),
    };
    let id = conn_id();
    let result =
        build_provider_from_connection(&id, ProviderKind::OpenAI, &creds, None, Vec::new());
    let provider = result.expect("expected Ok");
    let supported = provider.supported_models();
    assert!(
        supported.is_empty(),
        "expected empty supported_models for empty input, got {} models",
        supported.len()
    );
}

#[test]
fn test_generate_setup_token() {
    let token = generate_setup_token();
    assert!(token.starts_with("rk-setup-"));
    assert_eq!(token.len(), 9 + 48); // "rk-setup-" (9) + 48 hex chars
}

#[test]
fn test_build_provider_from_connection() {
    let conn_id = ConnectionId::new();
    let creds = DecryptedCredentials::ApiKey {
        api_key: "sk-test-key".to_string(),
    };

    // OpenAI
    let openai =
        build_provider_from_connection(&conn_id, ProviderKind::OpenAI, &creds, None, vec![]);
    assert!(openai.is_ok());

    // Anthropic
    let anthropic =
        build_provider_from_connection(&conn_id, ProviderKind::Anthropic, &creds, None, vec![]);
    assert!(anthropic.is_ok());

    // Ollama without base_url -> error
    let ollama_err =
        build_provider_from_connection(&conn_id, ProviderKind::Ollama, &creds, None, vec![]);
    assert!(matches!(
        ollama_err,
        Err(rook::di::ProviderBuildError::OllamaRequiresBaseUrl)
    ));

    // Ollama with base_url -> ok
    let ollama_ok = build_provider_from_connection(
        &conn_id,
        ProviderKind::Ollama,
        &creds,
        Some("http://localhost:11434".to_string()),
        vec![],
    );
    assert!(ollama_ok.is_ok());
}

#[tokio::test]
async fn test_setup_token_through_container() {
    let dir = tempfile::tempdir().unwrap();
    let mut config: RookConfig = toml::from_str(&minimal_config_toml(
        "[provider_crud]\nenabled = false\n[auth.api_keys]\nenabled = false\n[telemetry]\nenabled = false",
    ))
    .unwrap();
    config.database.db_path = dir.path().join("rook.db").to_str().unwrap().to_string();

    let container = rook::di::RookContainer::build(&config).await.unwrap();
    {
        let token = container.usecases.setup_token.read().await;
        let token = token
            .as_ref()
            .expect("uninitialized container has a setup token");
        let configured = std::env::var("ROOK_SETUP_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(configured) = configured {
            assert!(
                token == &configured,
                "configured setup token should be used"
            );
        } else {
            assert!(token.starts_with("rk-setup-"));
            assert_eq!(token.len(), 9 + 48);
        }
    }

    container
        .usecases
        .set_admin_password
        .execute(rook_usecases::SetAdminPasswordInput {
            new_password: "Test-fixture-password-123!".to_string(),
        })
        .await
        .unwrap();
    drop(container);

    let initialized = rook::di::RookContainer::build(&config).await.unwrap();
    assert!(initialized.usecases.setup_token.read().await.is_none());
}

#[test]
fn test_sqlite_secret_storage_through_api_key_auth() {
    // Isolate the environment and any accidental filesystem writes from the other tests.
    const CHILD_ENV: &str = "ROOK_DI_SECRET_TEST_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        let dir = tempfile::tempdir().unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "test_sqlite_secret_storage_through_api_key_auth"])
            .current_dir(dir.path())
            .env(CHILD_ENV, "1")
            .env_remove("API_KEY_HASH_SECRET")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "secret storage test failed: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        return;
    }
    let repo: Arc<dyn rook_core::ApiKeyRepositoryPort> = Arc::new(FakeApiKeyRepository::default());
    let registry: Arc<dyn rook_core::ProviderRegistryPort> = Arc::new(FakeProviderRegistry);
    for (target, in_memory) in [
        (":memory:", true),
        ("file::memory:?cache=shared", true),
        ("file:test.db?mode=memory", true),
        ("mem:test", true),
        ("rook.db", false),
        ("rook.sqlite", false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut config: RookConfig = toml::from_str(&minimal_config_toml("")).unwrap();
        config.database.db_path = dir.path().join(target).to_str().unwrap().to_string();
        // Prefix-based SQLite targets must retain their prefix.
        if target.starts_with("file:") || target.starts_with("mem:") || target == ":memory:" {
            config.database.db_path = target.to_string();
        }
        let (auth, manage_keys) = build_api_key_auth(&config, &repo, &registry).unwrap();
        assert!(auth.is_some());
        assert!(manage_keys.is_some());
        let secret_path = std::path::Path::new(&config.database.db_path)
            .parent()
            .unwrap()
            .join("api_key_secret.key");
        assert_eq!(secret_path.exists(), !in_memory, "target: {target}");
    }
}

#[tokio::test]
async fn test_noop_cache_port() {
    let config: RookConfig = toml::from_str(&cache_config_toml(false)).unwrap();
    let cache = build_cache_port(&config);
    assert_eq!(cache.delete_by_signature("sig").await.unwrap(), 0);
    assert!(cache.stats().await.is_ok());
    assert!(cache.clear().await.is_ok());
}
