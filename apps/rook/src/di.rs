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
#[allow(unused_imports)]
use providers_ollama::OllamaProvider;
use rook_core::{
    ApiKeyRepositoryPort, AuditPort, CachePort, ConnectionId, DecryptedCredentials, PasswordHasher,
    ProviderId, ProviderKind, ProviderPort, ProviderRegistryPort, ProviderRepositoryPort,
    RouterPort, SessionRepositoryPort,
};
use rook_usecases::{
    AuthenticateClientApi, EnsureAdminUser, FallbackRouter, HealthCheck, ManageConnections,
    ManageConnectionsError, ManageProviders, ProviderBuildInput, ProviderBuilderPort, RookUsecases,
    RouteRequest, RoutingStrategy, SetAdminPassword, ValidateSession,
};

use crate::config::RookConfig;

pub struct RookContainer {
    pub usecases: Arc<RookUsecases>,
    pub authz_config: transport_axum::authz::AuthzConfig,
    pub login_rate_limiter: Arc<transport_axum::LoginRateLimiter>,
    pub api_key_rate_limiter: Arc<transport_axum::ApiKeyRateLimiter>,
    pub csrf_guard: Arc<transport_axum::CsrfGuard>,
}

impl RookContainer {
    pub async fn build(config: &RookConfig) -> anyhow::Result<Self> {
        // 1. Cache
        let cache: Arc<dyn CachePort> = if config.cache.enabled {
            Arc::new(InMemoryCache::new(config.cache.ttl()))
        } else {
            Arc::new(NoOpCache)
        };

        // 2. Audit
        let audit: Arc<dyn AuditPort> = Arc::new(SqliteAudit::new(&config.database.db_path)?);

        // 3. Router and provider registry — starts empty, populated via refresh_registry
        let strategy: RoutingStrategy = config.routing.strategy.into();
        let fallback_router: Arc<FallbackRouter> = Arc::new(FallbackRouter::new_empty(strategy));
        let router: Arc<dyn RouterPort> = fallback_router.clone();
        let registry: Arc<dyn ProviderRegistryPort> = fallback_router;

        // 4. Auth — user/session repos and password hasher
        let user_repo: Arc<dyn rook_core::UserRepositoryPort> =
            Arc::new(SqliteUserRepository::new(&config.database.db_path)?);
        let session_repo: Arc<dyn SessionRepositoryPort> =
            Arc::new(SqliteSessionRepository::new(&config.database.db_path)?);
        let hasher: Arc<dyn PasswordHasher> = Arc::new(Argon2idHasher::new());

        // 5. Build ValidateSession for middleware
        let validate_session = Arc::new(ValidateSession::new(
            session_repo.clone(),
            user_repo.clone(),
        ));

        // 6. Auth use cases
        let ensure_admin_user = EnsureAdminUser::new(user_repo.clone());
        let set_admin_password = SetAdminPassword::new(user_repo.clone(), hasher.clone());
        let login =
            rook_usecases::Login::new(user_repo.clone(), session_repo.clone(), hasher.clone());
        let logout = rook_usecases::Logout::new(session_repo.clone());

        let authenticate_client_api = if config.auth.api_keys.enabled {
            let hash_secret = required_env("API_KEY_HASH_SECRET", "auth.api_keys.enabled")?;
            let repo: Arc<dyn ApiKeyRepositoryPort> =
                Arc::new(SqliteApiKeyRepository::new(&config.database.db_path)?);
            Some(AuthenticateClientApi::new(repo, hash_secret))
        } else {
            None
        };

        // 7. ManageConnections (provider CRUD) — built here so it can be used in join!
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

        // 8. Run async initialization tasks concurrently:
        // - registry refresh (if provider_crud enabled)
        // - ensure admin user exists
        //
        // Using tokio::join! to run both on the async runtime without blocking any thread.
        let (refresh_result, admin_result) = tokio::join!(
            async {
                if let Some(ref mc) = manage_connections {
                    mc.refresh_registry().await
                } else {
                    Ok(())
                }
            },
            ensure_admin_user.execute()
        );

        if let Err(e) = refresh_result {
            tracing::warn!(error = %e, "initial registry refresh failed, starting with empty registry");
        }

        let admin =
            admin_result.map_err(|e| anyhow::anyhow!("failed to ensure admin user: {}", e))?;
        tracing::info!(admin_id = %admin.id, "admin user ready");

        // 9. Use cases
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
// No-op cache (when cache is disabled)
// ---------------------------------------------------------------------------

use async_trait::async_trait;
use rook_core::CacheKey;
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
// Provider build errors
// ---------------------------------------------------------------------------

/// Errors that can occur when building a provider from connection data.
#[derive(Debug)]
pub enum ProviderBuildError {
    /// Ollama requires a base_url but none was provided.
    OllamaRequiresBaseUrl,
    /// Provider construction failed (e.g., invalid credentials, network error).
    ConstructionFailed(String),
}

impl std::fmt::Display for ProviderBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OllamaRequiresBaseUrl => {
                write!(
                    f,
                    "ollama provider requires a base_url but none was provided"
                )
            }
            Self::ConstructionFailed(msg) => write!(f, "provider construction failed: {msg}"),
        }
    }
}

impl std::error::Error for ProviderBuildError {}

// ---------------------------------------------------------------------------
// Dynamic provider builder (for provider CRUD)
// ---------------------------------------------------------------------------

/// Builds a provider from connection data.
///
/// This is the single function that handles all 5 providers (openai, anthropic,
/// ollama, gemini, groq) for both ApiKey and OAuth credentials.
pub fn build_provider_from_connection(
    connection_id: &ConnectionId,
    kind: ProviderKind,
    credentials: &DecryptedCredentials,
    base_url_override: Option<String>,
) -> Result<Arc<dyn ProviderPort>, ProviderBuildError> {
    // Extract credentials — use access_token for OAuth where the provider supports it
    let api_key = match credentials {
        DecryptedCredentials::ApiKey { api_key } => api_key.clone(),
        DecryptedCredentials::OAuth { access_token, .. } => {
            // OAuth access tokens can be used as API keys for providers that accept them.
            // For providers that don't support OAuth natively, this returns an error below.
            access_token.clone()
        }
    };

    let provider = match kind {
        ProviderKind::OpenAI => {
            let config = providers_openai::OpenAIProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                api_key,
                base_url: base_url_override.unwrap_or_else(|| "https://api.openai.com".to_string()),
                models: Vec::new(),
                timeout_secs: 60,
            };
            Arc::new(
                providers_openai::OpenAIProvider::new(config)
                    .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?,
            ) as Arc<dyn ProviderPort>
        }
        ProviderKind::Anthropic => {
            let config = providers_anthropic::AnthropicProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                api_key,
                base_url: base_url_override
                    .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
                models: Vec::new(),
                timeout_secs: 60,
            };
            providers_anthropic::AnthropicProvider::new(config)
                .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?
        }
        ProviderKind::Ollama => {
            let base_url = base_url_override.ok_or(ProviderBuildError::OllamaRequiresBaseUrl)?;
            let config = providers_ollama::OllamaProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                base_url,
                models: Vec::new(),
                timeout_secs: 300,
            };
            providers_ollama::OllamaProvider::new(config)
                .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?
        }
        ProviderKind::Gemini => {
            let config = providers_gemini::GeminiProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                api_key,
                models: Vec::new(),
                timeout_secs: 60,
            };
            providers_gemini::GeminiProvider::new(config)
                .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?
        }
        ProviderKind::Groq => {
            let config = providers_groq::GroqProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                api_key,
                models: Vec::new(),
                timeout_secs: 60,
            };
            providers_groq::GroqProvider::new(config)
                .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?
        }
    };

    Ok(provider)
}

#[derive(Clone)]
struct DynamicProviderBuilder;

#[async_trait]
impl ProviderBuilderPort for DynamicProviderBuilder {
    async fn build(
        &self,
        input: ProviderBuildInput,
    ) -> Result<Arc<dyn ProviderPort>, ManageConnectionsError> {
        build_provider_from_connection(
            &input.connection_id,
            input.provider_kind,
            &input.decrypted_credentials,
            input.base_url,
        )
        .map_err(|e| ManageConnectionsError::RegistryUpdateFailed(e.to_string()))
    }
}
