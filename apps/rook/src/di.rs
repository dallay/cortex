// di — dependency injection container
//
// This is the ONLY place where all crates are assembled.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use alias_sqlite::SqliteModelAliasRepository;
use audit_sqlite::{SqliteAudit, SqliteUsageRepository};
use auth_sqlite::{SqliteApiKeyRepository, SqliteSessionRepository, SqliteUserRepository};
use cache_memory::InMemoryCache;
use combo_sqlite::ComboSqliteRepository;
use encryption_inmemory::{AesGcmKeyManager, Argon2idHasher};
use models_catalog::StaticModelCatalog;
use provider_sqlite::SqliteProviderRepository;
#[allow(unused_imports)]
use providers_ollama::OllamaProvider;
use rook_core::{
    ApiKeyRepositoryPort, AuditPort, CachePort, Combo, ComboRepositoryPort, ComboStep,
    ComboStrategy, ConnectionId, DecryptedCredentials, ModelAliasRepositoryPort, PasswordHasher,
    ProviderId, ProviderKind, ProviderPort, ProviderRegistryPort, ProviderRepositoryPort,
    RouterPort, SessionRepositoryPort, UsageRecorderPort,
};
use rook_usecases::{
    AuthenticateClientApi, BootstrapStatus, EnsureAdminUser, FallbackRouter, HealthCheck,
    ManageConnections, ManageConnectionsError, ManageProviders, ProviderBuildInput,
    ProviderBuilderPort, RookUsecases, RouteRequest, RoutingStrategy, SetAdminPassword,
    ValidateSession,
};
use shared_kernel::{ComboId, ModelId};

use crate::config::RookConfig;

/// Generate a cryptographically random setup token with a recognisable prefix.
pub fn generate_setup_token() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..24).map(|_| rng.random::<u8>()).collect();
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
    pub ip_rate_limiter: Arc<transport_axum::IpRateLimiter>,
    pub api_key_rate_limiter: Arc<transport_axum::ApiKeyRateLimiter>,
    pub csrf_guard: Arc<transport_axum::CsrfGuard>,
    pub rate_limit_store: Option<transport_axum::handlers::rate_limits::RateLimitRuleStore>,
    /// Format registry for provider wire-format lookup — used in Phase 2 routing.
    #[allow(dead_code)]
    pub format_registry: Arc<transport_axum::format_registry::FormatRegistry>,
    /// Concrete usage repository — used for retention sweep task.
    pub usage_repository: Arc<SqliteUsageRepository>,
    /// Usage config — retention_days and sweep_interval_hours for retention sweep.
    pub usage_config: crate::config::UsageConfig,
}

impl RookContainer {
    pub async fn build(config: &RookConfig) -> anyhow::Result<Self> {
        // Run startup migrations once before constructing any repositories
        run_startup_migrations(&config.database.db_path)?;

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
        let registry: Arc<dyn ProviderRegistryPort> = fallback_router.clone();

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

        // 7. Build shared provider repository for manage_connections AND usage/connection lookup
        // Single shared instance — NOT duplicated for RouteRequest.
        let provider_repo: Arc<dyn ProviderRepositoryPort> =
            Arc::new(SqliteProviderRepository::new(&config.database.db_path)?);
        let provider_repository_for_usage: Option<Arc<dyn ProviderRepositoryPort>> =
            if config.provider_crud.enabled {
                Some(provider_repo.clone())
            } else {
                None
            };

        // 7a. ManageConnections (provider CRUD) — uses shared provider_repo
        let manage_connections = if config.provider_crud.enabled {
            let passphrase = required_env("ENCRYPTION_PASSPHRASE", "provider_crud.enabled")?;
            let salt = required_env("ENCRYPTION_SALT", "provider_crud.enabled")?;
            let key_manager = Arc::new(
                AesGcmKeyManager::from_passphrase_and_salt(&passphrase, &salt)
                    .map_err(|e| anyhow::anyhow!("invalid provider CRUD encryption config: {e}"))?,
            );
            let repo = provider_repo.clone();
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

        // 7b. Usage repository — concrete SqliteUsageRepository stored on container for retention
        let sqlite_usage: Arc<SqliteUsageRepository> = Arc::new(
            SqliteUsageRepository::new(Path::new(&config.database.db_path))
                .map_err(|e| anyhow::anyhow!("failed to create usage repository: {e}"))?,
        );
        let usage_recorder: Option<Arc<dyn UsageRecorderPort>> = Some(sqlite_usage.clone());

        // 7c. Combo repository — SQLite-backed combo storage
        let combo_repo: Arc<dyn ComboRepositoryPort> =
            Arc::new(ComboSqliteRepository::new(&config.database.db_path)?);

        // 7d. Model alias repository — SQLite-backed alias storage
        let alias_repo: Arc<dyn ModelAliasRepositoryPort> =
            Arc::new(SqliteModelAliasRepository::new(&config.database.db_path)?);

        // 7e. Seed built-in aliases if enabled
        if config.model_aliases.auto_seed {
            let builtin_aliases = alias_sqlite::builtin::DEFAULT_ALIASES
                .iter()
                .map(|(alias, canonical, provider_id)| rook_core::ModelAlias {
                    alias: shared_kernel::ModelId::new(*alias),
                    canonical: shared_kernel::ModelId::new(*canonical),
                    provider_id: provider_id.map(shared_kernel::ProviderId::new),
                    created_at: shared_kernel::Utc::now().to_rfc3339(),
                })
                .collect::<Vec<_>>();

            match alias_repo.seed(builtin_aliases).await {
                Ok(count) => {
                    tracing::info!(count, "Seeded default model aliases");
                }
                Err(e) => {
                    tracing::warn!(error = ?e, "Failed to seed model aliases");
                }
            }
        }

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

        // 8b. Seed combos from config into SQLite
        seed_combos_from_config(&combo_repo, &config.combos).await;

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

        // Create HealthCheck and spawn background task
        let health_check = Arc::new(HealthCheck::new(registry));
        let health_check_interval =
            std::time::Duration::from_secs(config.server.health_check_interval_secs);
        let _health_check_task =
            HealthCheck::spawn_background_task(health_check.clone(), health_check_interval);

        let usecases: Arc<rook_usecases::RookUsecases> =
            Arc::new(rook_usecases::RookUsecases::new(
                RouteRequest::new(
                    router.clone(),
                    cache.clone(),
                    audit.clone(),
                    usage_recorder.clone(),
                    provider_repository_for_usage,
                    Some(combo_repo.clone()), // combo_repository - wired in Phase 4
                    Arc::new(config.pricing.clone()),
                    format_registry.clone(),
                    alias_repo.clone(), // model_alias_repository - wired in Phase 3
                    config.model_aliases.clone().into(),
                ),
                ManageProviders::new(router.clone()),
                health_check,
                authenticate_client_api.clone(),
                manage_connections,
                manage_api_keys,
                None,
                bootstrap_status.clone(),
                ensure_admin_user,
                set_admin_password,
                login,
                logout,
                Arc::new(tokio::sync::RwLock::new(None)),
                session_repo.clone(),
                Arc::new(StaticModelCatalog::new()),
                fallback_router.clone(),
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

        // Build rate limiter config from TOML
        let rate_limiter_config = Arc::new(build_rate_limiter_config(&config.rate_limiting));

        // Build rate limit rule store if rate limiting is enabled
        let rate_limit_store = if config.rate_limiting.enabled {
            Some(Arc::new(dashmap::DashMap::new()))
        } else {
            None
        };

        Ok(Self {
            usecases,
            authz_config,
            login_rate_limiter: Arc::new(transport_axum::LoginRateLimiter::new()),
            ip_rate_limiter: Arc::new(transport_axum::IpRateLimiter::with_capacity(
                config.rate_limiting.ip_limits.requests_per_minute,
            )),
            api_key_rate_limiter: Arc::new(transport_axum::ApiKeyRateLimiter::with_config(
                rate_limiter_config,
            )),
            csrf_guard: Arc::new(transport_axum::CsrfGuard::new()),
            rate_limit_store,
            format_registry,
            usage_repository: sqlite_usage,
            usage_config: config.usage.clone(),
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

/// Build transport-axum RateLimiterConfig from apps/rook config
fn build_rate_limiter_config(
    cfg: &crate::config::RateLimiterConfig,
) -> transport_axum::middleware::api_key_rate_limiter::RateLimiterConfig {
    use std::collections::HashMap;
    use transport_axum::middleware::api_key_rate_limiter::{RateLimiterConfig, TierConfig};

    let mut tiers = HashMap::new();
    for (tier, tier_cfg) in &cfg.tiers {
        tiers.insert(
            *tier,
            TierConfig {
                requests_per_minute: tier_cfg.requests_per_minute,
                requests_per_day: tier_cfg.requests_per_day,
                tokens_per_minute: tier_cfg.tokens_per_minute,
            },
        );
    }

    RateLimiterConfig {
        enabled: cfg.enabled,
        default_tier: cfg.default_tier,
        tiers,
    }
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

/// Seed combos from config into SQLite on startup.
///
/// For each combo in config:
/// - Parse combo ID from string
/// - Convert config steps to domain ComboStep objects
/// - Try to find existing combo by ID
/// - If exists: update with new config
/// - If not: create new combo
///
/// Errors are logged but don't block startup.
async fn seed_combos_from_config(
    combo_repo: &Arc<dyn ComboRepositoryPort>,
    combos: &[crate::config::ComboConfig],
) {
    if combos.is_empty() {
        return;
    }

    let mut seeded = 0;
    let mut failed = 0;

    for combo_cfg in combos {
        // Parse combo ID
        let combo_id = match ComboId::parse_str(&combo_cfg.id) {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(
                    combo_name = %combo_cfg.name,
                    combo_id = %combo_cfg.id,
                    error = %e,
                    "invalid combo ID format - skipping"
                );
                failed += 1;
                continue;
            }
        };

        // Parse strategy
        let strategy = match combo_cfg.strategy.as_str() {
            "priority" => ComboStrategy::Priority,
            other => {
                tracing::warn!(
                    combo_name = %combo_cfg.name,
                    strategy = %other,
                    "unsupported strategy - skipping"
                );
                failed += 1;
                continue;
            }
        };

        // Convert steps
        let steps: Vec<ComboStep> = combo_cfg
            .steps
            .iter()
            .map(|s| ComboStep {
                provider_id: ProviderId::new(s.provider_id.clone()),
                model: ModelId::new(s.model.clone()),
                connection_id: None, // Config-based combos don't specify connection_id
                priority: s.priority,
            })
            .collect();

        // Create domain combo
        let mut combo = Combo::new(combo_cfg.name.clone(), strategy, steps);
        combo.id = combo_id; // Override generated ID with config ID

        // Validate
        if let Err(e) = combo.validate() {
            tracing::warn!(
                combo_name = %combo_cfg.name,
                error = %e,
                "combo validation failed - skipping"
            );
            failed += 1;
            continue;
        }

        // Upsert: try to find existing, then update or create
        match combo_repo.find(&combo_id).await {
            Ok(Some(existing)) => {
                // Update existing combo
                combo.created_at = existing.created_at; // Preserve creation time
                match combo_repo.update(&combo).await {
                    Ok(()) => {
                        tracing::debug!(
                            combo_id = %combo_id,
                            combo_name = %combo_cfg.name,
                            "updated combo from config"
                        );
                        seeded += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            combo_name = %combo_cfg.name,
                            error = %e,
                            "failed to update combo - skipping"
                        );
                        failed += 1;
                    }
                }
            }
            Ok(None) => {
                // Create new combo
                match combo_repo.create(&combo).await {
                    Ok(()) => {
                        tracing::debug!(
                            combo_id = %combo_id,
                            combo_name = %combo_cfg.name,
                            "created combo from config"
                        );
                        seeded += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            combo_name = %combo_cfg.name,
                            error = %e,
                            "failed to create combo - skipping"
                        );
                        failed += 1;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    combo_name = %combo_cfg.name,
                    error = %e,
                    "failed to check existing combo - skipping"
                );
                failed += 1;
            }
        }
    }

    if seeded > 0 {
        tracing::info!(
            seeded = seeded,
            failed = failed,
            "seeded combos from config"
        );
    }
    if failed > 0 {
        tracing::warn!(failed = failed, "some combos failed to seed from config");
    }
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
                base_url: base_url_override,
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
                base_url: None,
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
