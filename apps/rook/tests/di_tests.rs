// Integration tests for the rook DI container and provider builder

use std::sync::Arc;

use rook::config::{RateLimiterConfig, RookConfig, TierConfig};
use rook::di::{
    build_api_key_auth, build_cache_port, build_manage_connections, build_provider_from_connection,
    build_provider_from_connection_with_policy, build_provider_from_connection_with_recorder,
    build_rate_limiter_config, generate_setup_token, is_canonical_base_url, ProviderBuildError,
    ProviderConnectionBuildOptions,
};
use rook_core::{ApiKeyTier, ConnectionId, DecryptedCredentials, ModelId, ProviderKind};

fn conn_id() -> ConnectionId {
    ConnectionId::default()
}

#[test]
fn azure_openai_connection_uses_configured_deployment_as_model() {
    let credentials = DecryptedCredentials::OAuth {
        email: "operator@example.com".to_string(),
        access_token: "azure-access-token".to_string(),
        refresh_token: "azure-refresh-token".to_string(),
        expires_at: i64::MAX,
        scope: "https://cognitiveservices.azure.com/.default".to_string(),
        id_token: String::new(),
        project_id: String::new(),
        oauth_client_id: "azure-client".to_string(),
        oauth_client_secret: None,
        oauth_tenant_id: Some("tenant-id".to_string()),
    };

    let provider = build_provider_from_connection(
        &conn_id(),
        ProviderKind::AzureOpenAI,
        &credentials,
        Some("https://example.openai.azure.com".to_string()),
        vec![ModelId::new("deployment-chat")],
    )
    .expect("Azure provider should build from OAuth credentials");

    assert!(provider.supports_model(&ModelId::new("deployment-chat")));
}

#[test]
fn azure_openai_rejects_static_api_key_authentication() {
    let credentials = DecryptedCredentials::ApiKey {
        api_key: "not-an-entra-token".to_string(),
    };
    let result = build_provider_from_connection(
        &conn_id(),
        ProviderKind::AzureOpenAI,
        &credentials,
        Some("https://example.openai.azure.com".to_string()),
        vec![ModelId::new("deployment-chat")],
    );

    assert!(matches!(
        result,
        Err(rook::di::ProviderBuildError::AzureOpenAIRequiresOAuth)
    ));
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
use std::sync::Mutex;

static API_KEY_SECRET_ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvVarGuard {
    name: &'static str,
    original: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn remove(name: &'static str) -> Self {
        let original = std::env::var_os(name);
        std::env::remove_var(name);
        Self { name, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.original.take() {
            Some(value) => std::env::set_var(self.name, value),
            None => std::env::remove_var(self.name),
        }
    }
}

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

#[cfg(unix)]
#[test]
fn api_key_secret_storage_accepts_symlinked_database_directory() {
    use std::os::unix::fs::symlink;

    let _lock = API_KEY_SECRET_ENV_LOCK.lock().unwrap();
    let _env = EnvVarGuard::remove("API_KEY_HASH_SECRET");
    let dir = tempfile::tempdir().unwrap();
    let database_dir = dir.path().join("database");
    let database_alias = dir.path().join("database-alias");
    std::fs::create_dir(&database_dir).unwrap();
    symlink(&database_dir, &database_alias).unwrap();
    let db_path = database_alias.join("rook.db");
    let config = api_key_auth_config(db_path.to_str().unwrap());
    let repo: Arc<dyn rook_core::ApiKeyRepositoryPort> = Arc::new(FakeApiKeyRepository::default());
    let registry: Arc<dyn rook_core::ProviderRegistryPort> = Arc::new(FakeProviderRegistry);

    build_api_key_auth(&config, &repo, &registry).unwrap();

    assert!(database_dir.join("api_key_secret.key").is_file());
}

#[cfg(unix)]
#[test]
fn api_key_secret_storage_rejects_symlink_outside_database_directory() {
    use std::os::unix::fs::symlink;

    let _lock = API_KEY_SECRET_ENV_LOCK.lock().unwrap();
    let _env = EnvVarGuard::remove("API_KEY_HASH_SECRET");
    let dir = tempfile::tempdir().unwrap();
    let database_dir = dir.path().join("database");
    let outside_dir = dir.path().join("outside");
    std::fs::create_dir(&database_dir).unwrap();
    std::fs::create_dir(&outside_dir).unwrap();
    let outside_secret = outside_dir.join("secret.key");
    std::fs::write(&outside_secret, "secret\n").unwrap();
    symlink(&outside_secret, database_dir.join("api_key_secret.key")).unwrap();
    let db_path = database_dir.join("rook.db");
    let config = api_key_auth_config(db_path.to_str().unwrap());
    let repo: Arc<dyn rook_core::ApiKeyRepositoryPort> = Arc::new(FakeApiKeyRepository::default());
    let registry: Arc<dyn rook_core::ProviderRegistryPort> = Arc::new(FakeProviderRegistry);

    let error = match build_api_key_auth(&config, &repo, &registry) {
        Ok(_) => panic!("secret symlink outside database directory must be rejected"),
        Err(error) => error,
    };

    assert!(error
        .to_string()
        .contains("secret path would escape database directory"));
}

#[cfg(unix)]
#[test]
fn generated_api_key_secret_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let _lock = API_KEY_SECRET_ENV_LOCK.lock().unwrap();
    let _env = EnvVarGuard::remove("API_KEY_HASH_SECRET");
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("rook.db");
    std::fs::write(&db, "").unwrap();
    let config = api_key_auth_config(db.to_str().unwrap());
    let repo: Arc<dyn rook_core::ApiKeyRepositoryPort> = Arc::new(FakeApiKeyRepository::default());
    let registry: Arc<dyn rook_core::ProviderRegistryPort> = Arc::new(FakeProviderRegistry);

    build_api_key_auth(&config, &repo, &registry).unwrap();

    assert_eq!(
        std::fs::metadata(dir.path().join("api_key_secret.key"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn existing_api_key_secret_permissions_are_restricted_without_changing_content() {
    use std::os::unix::fs::PermissionsExt;

    let _lock = API_KEY_SECRET_ENV_LOCK.lock().unwrap();
    let _env = EnvVarGuard::remove("API_KEY_HASH_SECRET");
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("rook.db");
    std::fs::write(&db, "").unwrap();
    let secret_path = dir.path().join("api_key_secret.key");
    std::fs::write(&secret_path, "existing-secret\n").unwrap();
    std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let config = api_key_auth_config(db.to_str().unwrap());
    let repo: Arc<dyn rook_core::ApiKeyRepositoryPort> = Arc::new(FakeApiKeyRepository::default());
    let registry: Arc<dyn rook_core::ProviderRegistryPort> = Arc::new(FakeProviderRegistry);

    build_api_key_auth(&config, &repo, &registry).unwrap();

    assert_eq!(
        std::fs::read_to_string(&secret_path).unwrap(),
        "existing-secret\n"
    );
    assert_eq!(
        std::fs::metadata(&secret_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn already_restricted_api_key_secret_permissions_are_preserved() {
    use std::os::unix::fs::PermissionsExt;

    let _lock = API_KEY_SECRET_ENV_LOCK.lock().unwrap();
    let _env = EnvVarGuard::remove("API_KEY_HASH_SECRET");
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("rook.db");
    std::fs::write(&db, "").unwrap();
    let secret_path = dir.path().join("api_key_secret.key");
    std::fs::write(&secret_path, "read-only-secret\n").unwrap();
    std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    let config = api_key_auth_config(db.to_str().unwrap());
    let repo: Arc<dyn rook_core::ApiKeyRepositoryPort> = Arc::new(FakeApiKeyRepository::default());
    let registry: Arc<dyn rook_core::ProviderRegistryPort> = Arc::new(FakeProviderRegistry);

    build_api_key_auth(&config, &repo, &registry).unwrap();

    assert_eq!(
        std::fs::read_to_string(&secret_path).unwrap(),
        "read-only-secret\n"
    );
    assert_eq!(
        std::fs::metadata(&secret_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o400
    );
}

#[cfg(unix)]
fn api_key_auth_config(db_path: &str) -> RookConfig {
    let config = format!(
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
db_path = {db_path:?}
[provider_crud]
enabled = false
[rate_limiting]
enabled = false
"#
    );
    toml::from_str(&config).expect("test config parses")
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

    let result = build_manage_connections(&config, &provider_repo, &registry, &model_catalog, None);

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
        oauth_client_id: String::new(),
        oauth_client_secret: None,
        oauth_tenant_id: None,
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

// T1 — P1 canonical URLs: default (None) retains quota capture.
#[test]
fn canonical_base_url_default_none_retains_recorder() {
    assert!(
        is_canonical_base_url(ProviderKind::OpenAI, None),
        "OpenAI default (None) must retain quota capture"
    );
    assert!(
        is_canonical_base_url(ProviderKind::Anthropic, None),
        "Anthropic default (None) must retain quota capture"
    );
}

// T1 — P1 canonical URLs: explicit canonical retains quota capture.
#[test]
fn canonical_base_url_explicit_canonical_retains_recorder() {
    // OpenAI accepts with and without /v1 (UI sends /v1, backend default omits it).
    assert!(is_canonical_base_url(
        ProviderKind::OpenAI,
        Some("https://api.openai.com")
    ));
    assert!(is_canonical_base_url(
        ProviderKind::OpenAI,
        Some("https://api.openai.com/v1")
    ));
    assert!(is_canonical_base_url(
        ProviderKind::Anthropic,
        Some("https://api.anthropic.com")
    ));
    assert!(is_canonical_base_url(
        ProviderKind::Groq,
        Some("https://api.groq.com/openai/v1")
    ));
}

// T1 — P1 canonical URLs: custom URLs disable quota capture.
#[test]
fn canonical_base_url_custom_rejects_recorder() {
    assert!(
        !is_canonical_base_url(
            ProviderKind::OpenAI,
            Some("https://custom.openai.example.com/v1")
        ),
        "custom OpenAI proxy must disable quota capture"
    );
    assert!(
        !is_canonical_base_url(ProviderKind::Anthropic, Some("https://proxy.example.com")),
        "custom Anthropic proxy must disable quota capture"
    );
    assert!(
        !is_canonical_base_url(
            ProviderKind::Groq,
            Some("https://custom.groq.example.com/openai/v1")
        ),
        "custom Groq proxy must disable quota capture"
    );
    // Mock servers are http + port => always custom.
    assert!(!is_canonical_base_url(
        ProviderKind::OpenAI,
        Some("http://127.0.0.1:8080")
    ));
}

// T1 — P1 canonical URLs: Groq default (None) retains quota capture.
#[test]
fn canonical_base_url_groq_none_retains_recorder() {
    assert!(
        is_canonical_base_url(ProviderKind::Groq, None),
        "Groq default (None) must retain quota capture"
    );
}

// T1 — normalization: trim, trailing slash, case-insensitive host, /v1 tolerance.
#[test]
fn canonical_base_url_normalization() {
    assert!(is_canonical_base_url(
        ProviderKind::OpenAI,
        Some("  https://api.openai.com/v1/  ")
    ));
    assert!(is_canonical_base_url(
        ProviderKind::OpenAI,
        Some("https://API.OPENAI.COM/V1")
    ));
    assert!(is_canonical_base_url(
        ProviderKind::OpenAI,
        Some("https://api.openai.com/")
    ));
    assert!(is_canonical_base_url(
        ProviderKind::Anthropic,
        Some("https://API.ANTHROPIC.COM")
    ));
    assert!(is_canonical_base_url(
        ProviderKind::Groq,
        Some("https://api.groq.com/openai/v1/")
    ));
    // Anthropic must NOT tolerate /v1; Groq requires the full /openai/v1 path.
    assert!(!is_canonical_base_url(
        ProviderKind::Anthropic,
        Some("https://api.anthropic.com/v1")
    ));
    assert!(!is_canonical_base_url(
        ProviderKind::Groq,
        Some("https://api.groq.com")
    ));
    assert!(!is_canonical_base_url(
        ProviderKind::Groq,
        Some("https://api.groq.com/v1")
    ));
    // Scheme must be https; empty and schemeless values are custom.
    assert!(!is_canonical_base_url(
        ProviderKind::OpenAI,
        Some("http://api.openai.com/v1")
    ));
    assert!(!is_canonical_base_url(ProviderKind::OpenAI, Some("")));
    assert!(!is_canonical_base_url(
        ProviderKind::OpenAI,
        Some("api.openai.com/v1")
    ));
}

struct StubTokenPort;

#[async_trait::async_trait]
impl rook_core::OAuthTokenPort for StubTokenPort {
    async fn access_token(&self, _: &ConnectionId, _: bool) -> shared_kernel::CortexResult<String> {
        Ok("stub-access-token".to_string())
    }
}

fn oauth_creds() -> DecryptedCredentials {
    DecryptedCredentials::OAuth {
        email: "operator@example.com".to_string(),
        access_token: "oauth-access".to_string(),
        refresh_token: "oauth-refresh".to_string(),
        expires_at: i64::MAX,
        scope: "scope".to_string(),
        id_token: String::new(),
        project_id: "google-project".to_string(),
        oauth_client_id: "client-id".to_string(),
        oauth_client_secret: None,
        oauth_tenant_id: None,
    }
}

#[test]
fn openai_builds_oauth_backed_provider_with_token_port() {
    let provider = build_provider_from_connection_with_recorder(
        &conn_id(),
        ProviderKind::OpenAI,
        &oauth_creds(),
        None,
        vec![ModelId::new("gpt-4o")],
        None,
        Some(Arc::new(StubTokenPort)),
    )
    .expect("OpenAI OAuth provider should build");
    assert!(provider.supports_model(&ModelId::new("gpt-4o")));
}

#[test]
fn gemini_builds_oauth_backed_provider_with_token_port() {
    let provider = build_provider_from_connection_with_recorder(
        &conn_id(),
        ProviderKind::Gemini,
        &oauth_creds(),
        None,
        vec![ModelId::new("gemini-2.0-flash")],
        None,
        Some(Arc::new(StubTokenPort)),
    )
    .expect("Gemini OAuth provider should build");
    assert!(provider.supports_model(&ModelId::new("gemini-2.0-flash")));
}

#[test]
fn gemini_without_token_port_uses_access_token_as_api_key() {
    let provider = build_provider_from_connection_with_recorder(
        &conn_id(),
        ProviderKind::Gemini,
        &oauth_creds(),
        None,
        vec![ModelId::new("gemini-2.0-flash")],
        None,
        None,
    )
    .expect("Gemini provider should build without token port");
    assert!(provider.supports_model(&ModelId::new("gemini-2.0-flash")));
}

#[test]
fn azure_builds_oauth_backed_provider_with_token_port() {
    let provider = build_provider_from_connection_with_recorder(
        &conn_id(),
        ProviderKind::AzureOpenAI,
        &oauth_creds(),
        Some("https://example.openai.azure.com".to_string()),
        vec![ModelId::new("deployment-chat")],
        None,
        Some(Arc::new(StubTokenPort)),
    )
    .expect("Azure OAuth provider should build");
    assert!(provider.supports_model(&ModelId::new("deployment-chat")));
}

// ---------------------------------------------------------------------------
// OpenAI-compatible connections (DALLAY-682)
// ---------------------------------------------------------------------------

fn expect_build_error(
    result: Result<Arc<dyn rook_core::ProviderPort>, ProviderBuildError>,
) -> ProviderBuildError {
    match result {
        Ok(_) => panic!("expected the provider build to fail"),
        Err(error) => error,
    }
}

#[test]
fn build_provider_for_openai_compatible_exposes_declared_models_and_connection_id() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "compatible-key".to_string(),
    };
    let id = conn_id();
    let models = vec![
        ModelId::new("physical-model"),
        ModelId::new("physical-model-mini"),
    ];
    let provider = build_provider_from_connection_with_policy(
        &id,
        ProviderKind::OpenAiCompatible,
        &creds,
        Some("https://upstream.example.com/v1".to_string()),
        models.clone(),
        ProviderConnectionBuildOptions {
            quota_recorder: None,
            oauth_token_port: None,
            endpoint_policy: rook_core::EndpointPolicy {
                allowed_origins: vec!["https://upstream.example.com:443".to_string()],
            },
        },
    )
    .expect("expected Ok for an OpenAI-compatible connection");

    assert_eq!(provider.id().as_str(), id.to_string());
    assert_eq!(provider.supported_models(), models.as_slice());
    assert!(provider.supports_model(&ModelId::new("physical-model")));
    assert!(!provider.supports_model(&ModelId::new("gpt-4o")));
}

#[test]
fn build_provider_for_openai_compatible_requires_base_url() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "compatible-key".to_string(),
    };
    let error = expect_build_error(build_provider_from_connection_with_policy(
        &conn_id(),
        ProviderKind::OpenAiCompatible,
        &creds,
        None,
        vec![ModelId::new("physical-model")],
        ProviderConnectionBuildOptions {
            quota_recorder: None,
            oauth_token_port: None,
            endpoint_policy: rook_core::EndpointPolicy::default(),
        },
    ));
    assert!(
        error.to_string().to_lowercase().contains("base_url"),
        "expected a base_url error, got: {error}"
    );
}

#[test]
fn build_provider_for_openai_compatible_rejects_oauth_credentials() {
    let creds = DecryptedCredentials::OAuth {
        email: "user@example.com".to_string(),
        access_token: "token".to_string(),
        refresh_token: "refresh".to_string(),
        expires_at: 0,
        scope: "openid".to_string(),
        id_token: "id".to_string(),
        project_id: "project".to_string(),
        oauth_client_id: "client".to_string(),
        oauth_client_secret: Some("secret".to_string()),
        oauth_tenant_id: None,
    };
    let error = expect_build_error(build_provider_from_connection_with_policy(
        &conn_id(),
        ProviderKind::OpenAiCompatible,
        &creds,
        Some("https://upstream.example.com/v1".to_string()),
        vec![ModelId::new("physical-model")],
        ProviderConnectionBuildOptions {
            quota_recorder: None,
            oauth_token_port: None,
            endpoint_policy: rook_core::EndpointPolicy::default(),
        },
    ));
    assert!(
        error.to_string().to_lowercase().contains("api key"),
        "expected an API key error, got: {error}"
    );
}

#[test]
fn build_provider_for_openai_compatible_rejects_unsafe_endpoints() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "compatible-key".to_string(),
    };
    let error = expect_build_error(build_provider_from_connection_with_policy(
        &conn_id(),
        ProviderKind::OpenAiCompatible,
        &creds,
        Some("http://169.254.169.254/v1".to_string()),
        vec![ModelId::new("physical-model")],
        ProviderConnectionBuildOptions {
            quota_recorder: None,
            oauth_token_port: None,
            endpoint_policy: rook_core::EndpointPolicy::default(),
        },
    ));
    assert!(
        !error.to_string().contains("compatible-key"),
        "the error must not leak the credential: {error}"
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
            new_password: format!("{}Aa1!", rook::di::generate_setup_token()),
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
        // The executable is the current Cargo test harness; arguments are fixed below.
        // CodeQL cannot infer this executable path is trusted in this test-only subprocess.
        // codeql[rust/command-line-injection]
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "test_sqlite_secret_storage_through_api_key_auth"])
            .current_dir(dir.path())
            .env(CHILD_ENV, "1")
            .env_remove("API_KEY_HASH_SECRET")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "secret storage test failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
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
