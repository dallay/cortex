// di — dependency injection container
//
// This is the ONLY place where all crates are assembled.

use std::sync::Arc;
use std::time::Duration;

use audit_sqlite::SqliteAudit;
use auth_sqlite::{SqliteApiKeyRepository, SqliteSessionRepository, SqliteUserRepository};
use cache_memory::InMemoryCache;
use encryption_inmemory::{AesGcmKeyManager, Argon2idHasher};
use models_catalog::StaticModelCatalog;
use provider_sqlite::SqliteProviderRepository;
#[allow(unused_imports)]
use providers_ollama::OllamaProvider;
use rook_core::{
    ApiKeyRepositoryPort, AuditPort, CachePort, ConnectionId, DecryptedCredentials, PasswordHasher,
    ProviderId, ProviderKind, ProviderPort, ProviderRegistryPort, ProviderRepositoryPort,
    RouterPort, SessionRepositoryPort,
};
use rook_usecases::{
    AuthenticateClientApi, BootstrapStatus, EnsureAdminUser, FallbackRouter, HealthCheck,
    ManageConnections, ManageConnectionsError, ManageProviders, ProviderBuildInput,
    ProviderBuilderPort, RookUsecases, RouteRequest, RoutingStrategy, SetAdminPassword,
    ValidateSession,
};

use crate::config::RookConfig;

/// Generate a cryptographically random setup token with a recognisable prefix.
pub fn generate_setup_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..24).map(|_| rng.gen::<u8>()).collect();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("rk-setup-{hex}")
}

/// Run pending database migrations at startup.
///
/// Called BEFORE building the DI container to ensure the schema is up-to-date
/// before any repository uses the database. Fails fast if migrations fail.
pub fn run_startup_migrations(db_path: &str) -> anyhow::Result<usize> {
    db_migration::run_migrations(db_path)
}

pub struct RookContainer {
    pub usecases: Arc<RookUsecases>,
    pub authz_config: transport_axum::authz::AuthzConfig,
    pub login_rate_limiter: Arc<transport_axum::LoginRateLimiter>,
    pub api_key_rate_limiter: Arc<transport_axum::ApiKeyRateLimiter>,
    pub csrf_guard: Arc<transport_axum::CsrfGuard>,
    /// Format registry for provider wire-format lookup — used in Phase 2 routing.
    #[allow(dead_code)]
    pub format_registry: Arc<transport_axum::format_registry::FormatRegistry>,
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
        let bootstrap_status = BootstrapStatus::new(user_repo.clone());
        let ensure_admin_user = EnsureAdminUser::new(user_repo.clone());
        let set_admin_password = SetAdminPassword::new(user_repo.clone(), hasher.clone());
        let login =
            rook_usecases::Login::new(user_repo.clone(), session_repo.clone(), hasher.clone());
        let logout = rook_usecases::Logout::new(session_repo.clone());

        let api_key_repo: Arc<dyn ApiKeyRepositoryPort> =
            Arc::new(SqliteApiKeyRepository::new(&config.database.db_path)?);

        let (authenticate_client_api, manage_api_keys) = if config.auth.api_keys.enabled {
            let hash_secret = resolve_api_key_secret(&config.database.db_path)?;
            (
                Some(AuthenticateClientApi::new(
                    api_key_repo.clone(),
                    hash_secret.clone(),
                )),
                Some(rook_usecases::ManageApiKeys::new(
                    api_key_repo.clone(),
                    hash_secret,
                    registry.clone(),
                )),
            )
        } else {
            (None, None)
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

        // 9. Format registry and use cases
        let mut format_registry = transport_axum::format_registry::FormatRegistry::new();
        format_registry.register(
            rook_core::ApiFormat::OpenAI,
            rook_core::ApiFormat::Anthropic,
            transport_axum::format_registry::DomainPivotTranslator,
        );
        format_registry.register(
            rook_core::ApiFormat::Anthropic,
            rook_core::ApiFormat::OpenAI,
            transport_axum::format_registry::DomainPivotTranslator,
        );
        let format_registry = Arc::new(format_registry);

        let usecases: Arc<rook_usecases::RookUsecases> =
            Arc::new(rook_usecases::RookUsecases::new(
                RouteRequest::new(
                    router.clone(),
                    cache.clone(),
                    audit.clone(),
                    format_registry.clone(),
                ),
                ManageProviders::new(router.clone()),
                HealthCheck::new(registry),
                authenticate_client_api.clone(),
                manage_connections,
                manage_api_keys,
                bootstrap_status.clone(),
                ensure_admin_user,
                set_admin_password,
                login,
                logout,
                Arc::new(tokio::sync::RwLock::new(None)),
                session_repo.clone(),
                Arc::new(StaticModelCatalog::new()),
            ));

        // Check initialization state to decide on setup token.
        // Run AFTER usecases is built so we can write the token into it.
        let bootstrap_state = bootstrap_status.execute().await?;
        let setup_token_value = if bootstrap_state.is_initialized {
            None
        } else {
            let token = std::env::var("ROOK_SETUP_TOKEN")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(generate_setup_token);
            Some(token)
        };

        // Write token into usecases so HTTP handlers can access it
        {
            let mut guard = usecases.setup_token.write().await;
            *guard = setup_token_value;
        }

        let authz_config = transport_axum::authz::AuthzConfig::from_env_with_client_auth(
            authenticate_client_api,
            config.auth.api_keys.allow_env_fallback,
        )
        .with_session_validator(validate_session)
        .with_bootstrap_status(bootstrap_status.clone());

        Ok(Self {
            usecases,
            authz_config,
            login_rate_limiter: Arc::new(transport_axum::LoginRateLimiter::new()),
            api_key_rate_limiter: Arc::new(transport_axum::ApiKeyRateLimiter::new()),
            csrf_guard: Arc::new(transport_axum::CsrfGuard::new()),
            format_registry,
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

/// Resolve the API key hash secret.
///
/// Priority:
/// 1. `API_KEY_HASH_SECRET` env var (production / Docker).
/// 2. Persisted secret file next to the database (auto-created on first run).
///
/// The file-based fallback gives a zero-config experience for local usage
/// while keeping the secret stable across restarts.  Production deployments
/// should always set the env var so the secret is not stored on disk.
///
/// # Security note
/// The `db_path` parameter comes from internal configuration, not HTTP input.
/// This function validates the derived secret path to ensure it stays within
/// the database directory, preventing any possibility of path traversal even
/// in the unlikely event that config is manipulated.
//
// nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
// False positive — `db_path` is internal config, not HTTP request data.
// This rule targets Actix route handlers; this function is called at startup
// from DI code with values sourced from config files and environment variables.
fn is_in_memory_sqlite_target(path: &str) -> bool {
    path == ":memory:"
        || path.starts_with("file::memory:")
        || path.contains("mode=memory")
        || path.starts_with("mem:")
}

fn resolve_api_key_secret(db_path: &str) -> anyhow::Result<String> {
    // 1. Explicit env var wins.
    if let Ok(s) = std::env::var("API_KEY_HASH_SECRET") {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return Ok(s);
        }
    }

    // 2. In-memory SQLite targets cannot persist a secret file.
    // Generate a transient secret and return it — no file I/O.
    if is_in_memory_sqlite_target(db_path) {
        tracing::warn!(
            "API_KEY_HASH_SECRET not set and db_path is in-memory — \
             generating transient secret. Set API_KEY_HASH_SECRET env var for persistence."
        );
        return Ok(generate_setup_token());
    }

    // 3. Filesystem path — resolve and validate, then check/create the secret file.
    // Expand `~` so we land next to the real DB file.
    let expanded = if db_path.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        std::path::PathBuf::from(db_path.replacen('~', &home, 1)) // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    } else {
        std::path::PathBuf::from(db_path) // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    };

    // Resolve symlinks and normalize to an absolute path.
    let abs_db = expanded.canonicalize().unwrap_or_else(|_| expanded.clone());

    // Validate that the resolved path is a file (not a directory), preventing
    // traversal beyond the intended database file path.
    if abs_db.is_dir() {
        anyhow::bail!("database path must be a file, not a directory: {}", db_path);
    }

    let secret_path = abs_db
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("api_key_secret.key");

    // Defensive: ensure the secret path is actually a sibling, not a symlink escape.
    let secret_abs = secret_path
        .canonicalize()
        .unwrap_or_else(|_| secret_path.clone());
    let db_parent = abs_db.parent().unwrap_or(std::path::Path::new("."));
    let db_parent_abs = db_parent
        .canonicalize()
        .unwrap_or_else(|_| db_parent.to_path_buf());
    let secret_parent_abs = secret_abs.parent().unwrap_or(std::path::Path::new("."));
    if secret_parent_abs != db_parent_abs {
        anyhow::bail!(
            "secret path would escape database directory: {}",
            secret_path.display()
        );
    }

    if secret_path.exists() {
        let s = std::fs::read_to_string(&secret_path) // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
            .map_err(|e| anyhow::anyhow!("failed to read api_key_secret.key: {e}"))?;
        let s = s.trim().to_string();
        if !s.is_empty() {
            return Ok(s);
        }
    }

    // 3. Generate, persist, and warn.
    let secret = generate_setup_token(); // reuse the same random hex helper
    if let Some(parent) = secret_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            anyhow::anyhow!("failed to create data dir for api_key_secret.key: {e}")
        })?;
    }
    std::fs::write(&secret_path, &secret)
        .map_err(|e| anyhow::anyhow!("failed to write api_key_secret.key: {e}"))?;

    tracing::warn!(
        path = %secret_path.display(),
        "API_KEY_HASH_SECRET not set — generated and stored in file. \
         Set the env var for production deployments."
    );

    Ok(secret)
}

// ---------------------------------------------------------------------------
// No-op cache (when cache is disabled)
// ---------------------------------------------------------------------------

use async_trait::async_trait;
use rook_core::CacheKey;
use shared_kernel::CortexResult;

#[derive(Clone, Default)]
struct NoOpCache;

#[async_trait]
impl CachePort for NoOpCache {
    async fn get(&self, _: &CacheKey) -> CortexResult<Option<rook_core::CompletionResponse>> {
        Ok(None)
    }
    async fn set(
        &self,
        _: &CacheKey,
        _: &rook_core::CompletionResponse,
        _: Duration,
    ) -> CortexResult<()> {
        Ok(())
    }
    async fn delete(&self, _: &CacheKey) -> CortexResult<()> {
        Ok(())
    }
    async fn clear(&self) -> CortexResult<()> {
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
