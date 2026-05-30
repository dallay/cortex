// di — dependency injection container
//
// This is the ONLY place where all crates are assembled.

use std::sync::Arc;
use std::time::Duration;

use audit_sqlite::SqliteAudit;
use auth_sqlite::SqliteApiKeyRepository;
use cache_memory::InMemoryCache;
use encryption_inmemory::AesGcmKeyManager;
use provider_sqlite::SqliteProviderRepository;
use providers_anthropic::AnthropicProvider;
use providers_gemini::GeminiProvider;
use providers_groq::GroqProvider;
use providers_ollama::OllamaProvider;
use providers_openai::OpenAIProvider;
use rook_core::{
    ApiKeyRepositoryPort, AuditPort, CachePort, ProviderId, ProviderPort, ProviderRegistryPort,
    ProviderRepositoryPort, RouterPort,
};
use rook_usecases::{
    AuthenticateClientApi, FallbackRouter, HealthCheck, ManageConnections, ManageConnectionsError,
    ManageProviders, ProviderBuildInput, ProviderBuilderPort, RookUsecases, RouteRequest,
    RoutingStrategy,
};

use crate::config::{ProviderConfig, RookConfig};

pub struct RookContainer {
    pub usecases: Arc<RookUsecases>,
    pub authz_config: transport_axum::authz::AuthzConfig,
}

impl RookContainer {
    pub fn build(config: &RookConfig) -> anyhow::Result<Self> {
        // 1. Build all providers
        let providers: Vec<Arc<dyn ProviderPort>> = config
            .providers
            .iter()
            .filter_map(|pc| build_provider(pc))
            .collect();

        if providers.is_empty() {
            anyhow::bail!("no providers configured");
        }

        tracing::info!(count = providers.len(), "providers initialized");

        // 2. Cache
        let cache: Arc<dyn CachePort> = if config.cache.enabled {
            Arc::new(InMemoryCache::new(config.cache.ttl()))
        } else {
            Arc::new(NoOpCache)
        };

        // 3. Audit
        let audit: Arc<dyn AuditPort> = Arc::new(SqliteAudit::new(&config.database.db_path)?);

        // 4. Router and provider registry
        let strategy: RoutingStrategy = config.routing.strategy.into();
        let fallback_router: Arc<FallbackRouter> =
            Arc::new(FallbackRouter::new(providers, strategy));
        let router: Arc<dyn RouterPort> = fallback_router.clone();
        let registry: Arc<dyn ProviderRegistryPort> = fallback_router;

        let manage_connections = if config.provider_crud.enabled {
            let passphrase = required_env("ENCRYPTION_PASSPHRASE", "provider_crud.enabled")?;
            let salt = required_env("ENCRYPTION_SALT", "provider_crud.enabled")?;
            let key_manager = Arc::new(
                AesGcmKeyManager::from_passphrase_and_salt(&passphrase, &salt)
                    .map_err(|e| anyhow::anyhow!("invalid provider CRUD encryption config: {e}"))?,
            );
            let repo: Arc<dyn ProviderRepositoryPort> =
                Arc::new(SqliteProviderRepository::new(&config.database.db_path)?);
            let builder: Arc<dyn ProviderBuilderPort> = Arc::new(DynamicProviderBuilder);
            Some(ManageConnections::new(
                repo,
                registry.clone(),
                key_manager,
                builder,
            ))
        } else {
            None
        };

        let authenticate_client_api = if config.auth.api_keys.enabled {
            let hash_secret = required_env("API_KEY_HASH_SECRET", "auth.api_keys.enabled")?;
            let repo: Arc<dyn ApiKeyRepositoryPort> =
                Arc::new(SqliteApiKeyRepository::new(&config.database.db_path)?);
            Some(AuthenticateClientApi::new(repo, hash_secret))
        } else {
            None
        };

        // 5. Use cases
        let usecases = Arc::new(RookUsecases {
            route_request: RouteRequest::new(router.clone(), cache.clone(), audit.clone()),
            manage_providers: ManageProviders::new(router.clone()),
            health_check: HealthCheck::new(registry),
            authenticate_client_api: authenticate_client_api.clone(),
            manage_connections,
        });

        let authz_config = transport_axum::authz::AuthzConfig::from_env_with_client_auth(
            authenticate_client_api,
            config.auth.api_keys.allow_env_fallback,
        );

        Ok(Self {
            usecases,
            authz_config,
        })
    }
}

fn required_env(name: &str, context: &str) -> anyhow::Result<String> {
    let value =
        std::env::var(name).map_err(|_| anyhow::anyhow!("{name} is required when {context}"))?;
    if value.trim().is_empty() {
        anyhow::bail!("{name} must not be empty when {context}");
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
// Provider builder
// ---------------------------------------------------------------------------

fn build_provider(config: &ProviderConfig) -> Option<Arc<dyn ProviderPort>> {
    match config.kind.as_str() {
        "openai" => OpenAIProvider::new(providers_openai::OpenAIProviderConfig {
            id: ProviderId::new(&config.id),
            api_key: config.api_key.clone().unwrap_or_default(),
            base_url: config
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com".to_string()),
            models: config.models.iter().map(|s| s.as_str().into()).collect(),
            timeout_secs: config.timeout_secs.unwrap_or(60),
        })
        .ok()
        .map(|p| Arc::new(p) as Arc<dyn ProviderPort>),
        "anthropic" => AnthropicProvider::new(providers_anthropic::AnthropicProviderConfig {
            id: ProviderId::new(&config.id),
            api_key: config.api_key.clone().unwrap_or_default(),
            base_url: config
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
            models: config.models.iter().map(|s| s.as_str().into()).collect(),
            timeout_secs: config.timeout_secs.unwrap_or(60),
        })
        .ok()
        .map(|p| p as Arc<dyn ProviderPort>),
        "ollama" => OllamaProvider::new(providers_ollama::OllamaProviderConfig {
            id: ProviderId::new(&config.id),
            base_url: config
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string()),
            models: config.models.iter().map(|s| s.as_str().into()).collect(),
            timeout_secs: config.timeout_secs.unwrap_or(300),
        })
        .ok()
        .map(|p| p as Arc<dyn ProviderPort>),
        "gemini" => GeminiProvider::new(providers_gemini::GeminiProviderConfig {
            id: ProviderId::new(&config.id),
            api_key: config.api_key.clone().unwrap_or_default(),
            models: config.models.iter().map(|s| s.as_str().into()).collect(),
            timeout_secs: config.timeout_secs.unwrap_or(60),
        })
        .ok()
        .map(|p| p as Arc<dyn ProviderPort>),
        "groq" => GroqProvider::new(providers_groq::GroqProviderConfig {
            id: ProviderId::new(&config.id),
            api_key: config.api_key.clone().unwrap_or_default(),
            models: config.models.iter().map(|s| s.as_str().into()).collect(),
            timeout_secs: config.timeout_secs.unwrap_or(60),
        })
        .ok()
        .map(|p| p as Arc<dyn ProviderPort>),
        unknown => {
            tracing::warn!(kind = unknown, "unknown provider kind, skipping");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// No-op cache (when cache is disabled)
// ---------------------------------------------------------------------------

use async_trait::async_trait;
use rook_core::{CacheKey, DecryptedCredentials};
use shared_kernel::NuxaResult;

#[derive(Clone, Default)]
struct NoOpCache;

#[async_trait]
impl CachePort for NoOpCache {
    async fn get(&self, _: &CacheKey) -> NuxaResult<Option<rook_core::CompletionResponse>> {
        Ok(None)
    }
    async fn set(
        &self,
        _: &CacheKey,
        _: &rook_core::CompletionResponse,
        _: Duration,
    ) -> NuxaResult<()> {
        Ok(())
    }
    async fn delete(&self, _: &CacheKey) -> NuxaResult<()> {
        Ok(())
    }
    async fn clear(&self) -> NuxaResult<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Dynamic provider builder (for provider CRUD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{build_provider, required_env, NoOpCache};
    use rook_core::CachePort;
    use std::time::Duration;

    // -----------------------------------------------------------------------
    // required_env()
    // -----------------------------------------------------------------------

    #[test]
    fn required_env_returns_value_when_set() {
        let var_name = "ROOK_DI_TEST_VAR_PRESENT";
        std::env::set_var(var_name, "my-secret");
        let result = required_env(var_name, "some feature");
        std::env::remove_var(var_name);

        assert_eq!(result.unwrap(), "my-secret");
    }

    #[test]
    fn required_env_errors_when_var_not_set() {
        let var_name = "ROOK_DI_TEST_VAR_DEFINITELY_ABSENT_12345";
        std::env::remove_var(var_name);
        let result = required_env(var_name, "test context");

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains(var_name), "error should mention the var name");
        assert!(msg.contains("test context"), "error should mention context");
    }

    #[test]
    fn required_env_errors_when_var_is_empty_string() {
        let var_name = "ROOK_DI_TEST_VAR_EMPTY";
        std::env::set_var(var_name, "");
        let result = required_env(var_name, "empty check context");
        std::env::remove_var(var_name);

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains(var_name), "error should mention the var name");
    }

    #[test]
    fn required_env_errors_when_var_is_whitespace_only() {
        let var_name = "ROOK_DI_TEST_VAR_WHITESPACE";
        std::env::set_var(var_name, "   ");
        let result = required_env(var_name, "whitespace check");
        std::env::remove_var(var_name);

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("must not be empty"),
            "error should indicate empty value"
        );
    }

    #[test]
    fn required_env_accepts_value_with_surrounding_whitespace() {
        // Only trims for empty check, but preserves value as-is
        let var_name = "ROOK_DI_TEST_VAR_PADDED";
        std::env::set_var(var_name, "  actual-value  ");
        let result = required_env(var_name, "padded value");
        std::env::remove_var(var_name);

        // trim().is_empty() is false, so should succeed with original value
        assert_eq!(result.unwrap(), "  actual-value  ");
    }

    // -----------------------------------------------------------------------
    // NoOpCache
    // -----------------------------------------------------------------------

    fn make_cache_key() -> rook_core::CacheKey {
        use shared_kernel::RequestId;
        rook_core::CacheKey {
            request_id: RequestId::new(),
        }
    }

    fn make_completion_response() -> rook_core::CompletionResponse {
        use shared_kernel::{ModelId, ProviderId, RequestId};
        rook_core::CompletionResponse {
            id: RequestId::new(),
            provider: ProviderId::new("openai"),
            model: ModelId::new("gpt-4"),
            content: "hello".to_string(),
            usage: rook_core::TokenUsage {
                prompt_tokens: 5,
                completion_tokens: 1,
                total_tokens: 6,
                estimated_cost_usd: None,
            },
            latency_ms: 42,
        }
    }

    #[tokio::test]
    async fn no_op_cache_get_always_returns_none() {
        let cache = NoOpCache;
        let key = make_cache_key();
        let result = cache.get(&key).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn no_op_cache_set_always_returns_ok() {
        let cache = NoOpCache;
        let key = make_cache_key();
        let response = make_completion_response();
        let result = cache.set(&key, &response, Duration::from_secs(60)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn no_op_cache_delete_always_returns_ok() {
        let cache = NoOpCache;
        let key = make_cache_key();
        let result = cache.delete(&key).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn no_op_cache_clear_always_returns_ok() {
        let cache = NoOpCache;
        let result = cache.clear().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn no_op_cache_get_returns_none_for_distinct_keys() {
        use shared_kernel::RequestId;
        let cache = NoOpCache;
        // Verify no key is ever "cached" — each distinct key returns None
        let key1 = rook_core::CacheKey {
            request_id: RequestId::new(),
        };
        let key2 = rook_core::CacheKey {
            request_id: RequestId::new(),
        };

        assert!(cache.get(&key1).await.unwrap().is_none());
        assert!(cache.get(&key2).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn no_op_cache_set_then_get_still_returns_none() {
        // Verifies that set() does not actually store anything
        let cache = NoOpCache;
        let key = make_cache_key();
        let response = make_completion_response();

        cache
            .set(&key, &response, Duration::from_secs(60))
            .await
            .unwrap();
        let result = cache.get(&key).await.unwrap();
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // build_provider — unknown kind
    // -----------------------------------------------------------------------

    #[test]
    fn build_provider_returns_none_for_unknown_kind() {
        let config = crate::config::ProviderConfig {
            id: "test-id".to_string(),
            kind: "nonexistent-provider-xyz".to_string(),
            api_key: None,
            base_url: None,
            models: vec!["model-a".to_string()],
            timeout_secs: None,
        };
        let result = build_provider(&config);
        assert!(result.is_none());
    }

    #[test]
    fn build_provider_returns_none_for_empty_kind() {
        let config = crate::config::ProviderConfig {
            id: "test-id".to_string(),
            kind: "".to_string(),
            api_key: None,
            base_url: None,
            models: vec![],
            timeout_secs: None,
        };
        let result = build_provider(&config);
        assert!(result.is_none());
    }

    #[test]
    fn build_provider_returns_some_for_ollama_kind() {
        // Ollama doesn't require an API key and uses localhost by default
        let config = crate::config::ProviderConfig {
            id: "ollama-test".to_string(),
            kind: "ollama".to_string(),
            api_key: None,
            base_url: Some("http://localhost:11434".to_string()),
            models: vec!["llama2".to_string()],
            timeout_secs: Some(60),
        };
        let result = build_provider(&config);
        assert!(result.is_some());
    }

    #[test]
    fn build_provider_uses_default_ollama_url_when_base_url_absent() {
        let config = crate::config::ProviderConfig {
            id: "ollama-no-url".to_string(),
            kind: "ollama".to_string(),
            api_key: None,
            base_url: None,
            models: vec!["llama2".to_string()],
            timeout_secs: None,
        };
        // Should not panic — should use the fallback URL
        let result = build_provider(&config);
        assert!(result.is_some());
    }
}

#[derive(Clone)]
struct DynamicProviderBuilder;


impl ProviderBuilderPort for DynamicProviderBuilder {
    async fn build(
        &self,
        input: ProviderBuildInput,
    ) -> Result<Arc<dyn ProviderPort>, ManageConnectionsError> {
        let api_key = match &input.decrypted_credentials {
            DecryptedCredentials::ApiKey { api_key } => api_key.clone(),
            DecryptedCredentials::OAuth { .. } => {
                return Err(ManageConnectionsError::RegistryUpdateFailed(
                    "OAuth provider build not yet implemented".to_string(),
                ));
            }
        };

        // Build the appropriate provider based on kind
        let provider = match input.provider_kind.as_str() {
            "openai" => {
                let config = providers_openai::OpenAIProviderConfig {
                    id: ProviderId::new(input.connection_id.to_string()),
                    api_key,
                    base_url: input
                        .base_url
                        .unwrap_or_else(|| "https://api.openai.com".to_string()),
                    models: input.default_model.map(|m| vec![m]).unwrap_or_default(),
                    timeout_secs: 60,
                };
                Arc::new(providers_openai::OpenAIProvider::new(config).map_err(|e| {
                    ManageConnectionsError::RegistryUpdateFailed(format!(
                        "OpenAI provider build failed: {e}"
                    ))
                })?) as Arc<dyn ProviderPort>
            }
            "anthropic" => {
                let config = providers_anthropic::AnthropicProviderConfig {
                    id: ProviderId::new(input.connection_id.to_string()),
                    api_key,
                    base_url: input
                        .base_url
                        .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
                    models: input.default_model.map(|m| vec![m]).unwrap_or_default(),
                    timeout_secs: 60,
                };
                providers_anthropic::AnthropicProvider::new(config).map_err(|e| {
                    ManageConnectionsError::RegistryUpdateFailed(format!(
                        "Anthropic provider build failed: {e}"
                    ))
                })?
            }
            "ollama" => {
                let config = providers_ollama::OllamaProviderConfig {
                    id: ProviderId::new(input.connection_id.to_string()),
                    base_url: input
                        .base_url
                        .unwrap_or_else(|| "http://localhost:11434".to_string()),
                    models: input.default_model.map(|m| vec![m]).unwrap_or_default(),
                    timeout_secs: 300,
                };
                providers_ollama::OllamaProvider::new(config).map_err(|e| {
                    ManageConnectionsError::RegistryUpdateFailed(format!(
                        "Ollama provider build failed: {e}"
                    ))
                })?
            }
            "gemini" => {
                let config = providers_gemini::GeminiProviderConfig {
                    id: ProviderId::new(input.connection_id.to_string()),
                    api_key,
                    models: input.default_model.map(|m| vec![m]).unwrap_or_default(),
                    timeout_secs: 60,
                };
                providers_gemini::GeminiProvider::new(config).map_err(|e| {
                    ManageConnectionsError::RegistryUpdateFailed(format!(
                        "Gemini provider build failed: {e}"
                    ))
                })?
            }
            "groq" => {
                let config = providers_groq::GroqProviderConfig {
                    id: ProviderId::new(input.connection_id.to_string()),
                    api_key,
                    models: input.default_model.map(|m| vec![m]).unwrap_or_default(),
                    timeout_secs: 60,
                };
                providers_groq::GroqProvider::new(config).map_err(|e| {
                    ManageConnectionsError::RegistryUpdateFailed(format!(
                        "Groq provider build failed: {e}"
                    ))
                })?
            }
            unknown => {
                return Err(ManageConnectionsError::RegistryUpdateFailed(format!(
                    "unknown provider kind: {unknown}"
                )));
            }
        };

        Ok(provider)
    }
}
