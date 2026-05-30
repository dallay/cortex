// di — dependency injection container
//
// This is the ONLY place where all crates are assembled.

use std::sync::Arc;
use std::time::Duration;

use audit_sqlite::SqliteAudit;
use auth_sqlite::{SqliteApiKeyRepository, SqliteSessionRepository, SqliteUserRepository};
use cache_memory::InMemoryCache;
use encryption_inmemory::{AesGcmKeyManager, Argon2idHasher};
use provider_sqlite::SqliteProviderRepository;
use providers_anthropic::AnthropicProvider;
use providers_gemini::GeminiProvider;
use providers_groq::GroqProvider;
use providers_ollama::OllamaProvider;
use providers_openai::OpenAIProvider;
use rook_core::{
    ApiKeyRepositoryPort, AuditPort, CachePort, PasswordHasher, ProviderId, ProviderPort,
    ProviderRegistryPort, ProviderRepositoryPort, RouterPort, SessionRepositoryPort,
};
use rook_usecases::{
    AuthenticateClientApi, EnsureAdminUser, FallbackRouter, HealthCheck, ManageConnections,
    ManageConnectionsError, ManageProviders, ProviderBuildInput, ProviderBuilderPort, RookUsecases,
    RouteRequest, RoutingStrategy, SetAdminPassword, ValidateSession,
};

use crate::config::{ProviderConfig, RookConfig};

pub struct RookContainer {
    pub usecases: Arc<RookUsecases>,
    pub authz_config: transport_axum::authz::AuthzConfig,
    pub login_rate_limiter: Arc<transport_axum::LoginRateLimiter>,
    pub api_key_rate_limiter: Arc<transport_axum::ApiKeyRateLimiter>,
    pub csrf_guard: Arc<transport_axum::CsrfGuard>,
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

        // 5. Auth — user/session repos and password hasher
        let user_repo: Arc<dyn rook_core::UserRepositoryPort> =
            Arc::new(SqliteUserRepository::new(&config.database.db_path)?);
        let session_repo: Arc<dyn SessionRepositoryPort> =
            Arc::new(SqliteSessionRepository::new(&config.database.db_path)?);
        let hasher: Arc<dyn PasswordHasher> = Arc::new(Argon2idHasher::new());

        // 6. Build ValidateSession for middleware
        let validate_session = Arc::new(ValidateSession::new(session_repo.clone(), user_repo.clone()));

        // 6. Auth use cases
        let ensure_admin_user = EnsureAdminUser::new(user_repo.clone());
        let set_admin_password = SetAdminPassword::new(user_repo.clone(), hasher.clone());
        let login = rook_usecases::Login::new(user_repo.clone(), session_repo.clone(), hasher.clone());
        let logout = rook_usecases::Logout::new(session_repo.clone());

        //7. Ensure admin exists on first boot (before HTTP server starts)
        // This guarantees the admin record exists with NULL password_hash.
        // Any login attempt before SetAdminPassword is called will fail with PASSWORD_NOT_SET.
        {
            let admin = tokio::runtime::Handle::current()
                .block_on(ensure_admin_user.execute())
                .map_err(|e| anyhow::anyhow!("failed to ensure admin user: {}", e))?;
            tracing::info!(admin_id = %admin.id, "admin user ready");
        }

        // 8. Use cases
        let usecases = Arc::new(RookUsecases {
            route_request: RouteRequest::new(router.clone(), cache.clone(), audit.clone()),
            manage_providers: ManageProviders::new(router.clone()),
            health_check: HealthCheck::new(registry),
            authenticate_client_api: authenticate_client_api.clone(),
            manage_connections,
            ensure_admin_user,
            set_admin_password,
            login,
            logout,
        });

        let authz_config = transport_axum::authz::AuthzConfig::from_env_with_client_auth(
            authenticate_client_api,
            config.auth.api_keys.allow_env_fallback,
        )
        .with_session_validator(validate_session);

        Ok(Self {
            usecases,
            authz_config,
            login_rate_limiter: Arc::new(transport_axum::LoginRateLimiter::new()),
            api_key_rate_limiter: Arc::new(transport_axum::ApiKeyRateLimiter::new()),
            csrf_guard: Arc::new(transport_axum::CsrfGuard::new()),
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

#[derive(Clone)]
struct DynamicProviderBuilder;

#[async_trait]
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
