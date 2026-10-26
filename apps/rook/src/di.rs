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

#[cfg(test)]
#[path = "di_catalog_tests.rs"]
mod catalog_tests;
use provider_sqlite::SqliteProviderRepository;
#[allow(unused_imports)]
use providers_ollama::OllamaProvider;
use rook_core::{
    ApiKeyRepositoryPort, AuditPort, CachePort, Combo, ComboAliasRepositoryPort,
    ComboRepositoryPort, ComboStep, ComboStrategy, ConnectionId, DecryptedCredentials,
    ModelAliasRepositoryPort, ModelCatalogPort, PasswordHasher, ProviderId, ProviderKind,
    ProviderPort, ProviderRegistryPort, ProviderRepositoryPort, QuotaObservation,
    QuotaObservationRecorderPort, RouterPort, SessionRepositoryPort, UsageRecorderPort,
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
    pub format_registry: Arc<transport_axum::format_registry::FormatRegistry>,
    /// Concrete usage repository — used for retention sweep task.
    pub usage_repository: Arc<SqliteUsageRepository>,
    /// Upstream quota observation recorder — kept distinct from local usage history.
    pub quota_observation_recorder: Arc<dyn QuotaObservationRecorderPort>,
    /// Usage config — retention_days and sweep_interval_hours for retention sweep.
    pub usage_config: crate::config::UsageConfig,
}

pub fn build_cache_port(config: &RookConfig) -> Arc<dyn CachePort> {
    if config.cache.enabled {
        Arc::new(InMemoryCache::new(
            config.cache.ttl(),
            config.cache.max_entries,
        ))
    } else {
        Arc::new(NoOpCache)
    }
}

pub fn build_api_key_auth(
    config: &RookConfig,
    api_key_repo: &Arc<dyn ApiKeyRepositoryPort>,
    registry: &Arc<dyn ProviderRegistryPort>,
) -> anyhow::Result<(
    Option<AuthenticateClientApi>,
    Option<rook_usecases::ManageApiKeys>,
)> {
    if !config.auth.api_keys.enabled {
        return Ok((None, None));
    }
    let hash_secret = resolve_api_key_secret(&config.database.db_path)?;
    Ok((
        Some(AuthenticateClientApi::new(
            api_key_repo.clone(),
            hash_secret.clone(),
        )),
        Some(rook_usecases::ManageApiKeys::new(
            api_key_repo.clone(),
            hash_secret,
            registry.clone(),
        )),
    ))
}

pub fn build_manage_connections(
    config: &RookConfig,
    provider_repo: &Arc<dyn ProviderRepositoryPort>,
    registry: &Arc<dyn ProviderRegistryPort>,
    model_catalog: &Arc<dyn ModelCatalogPort>,
    quota_observation_recorder: Option<Arc<dyn QuotaObservationRecorderPort>>,
) -> anyhow::Result<Option<ManageConnections>> {
    if !config.provider_crud.enabled {
        return Ok(None);
    }
    let passphrase = required_env("ENCRYPTION_PASSPHRASE", "provider_crud.enabled")?;
    let salt = required_env("ENCRYPTION_SALT", "provider_crud.enabled")?;
    let key_manager = Arc::new(
        AesGcmKeyManager::from_passphrase_and_salt(&passphrase, &salt)
            .map_err(|e| anyhow::anyhow!("invalid provider CRUD encryption config: {e}"))?,
    );
    let endpoint_policy = config.provider_endpoint_policy.clone();
    let builder: Arc<dyn ProviderBuilderPort> = Arc::new(DynamicProviderBuilder::new(
        model_catalog.clone(),
        quota_observation_recorder,
    ));
    Ok(Some(
        ManageConnections::new(
            provider_repo.clone(),
            registry.clone(),
            key_manager,
            builder,
        )
        .with_endpoint_policy(endpoint_policy),
    ))
}

fn build_rate_limit_store(
    config: &RookConfig,
) -> Option<transport_axum::handlers::rate_limits::RateLimitRuleStore> {
    if config.rate_limiting.enabled {
        Some(transport_axum::handlers::rate_limits::RateLimitRuleStore::new())
    } else {
        None
    }
}

fn resolve_setup_token_value(is_initialized: bool) -> Option<String> {
    if is_initialized {
        return None;
    }
    let token = std::env::var("ROOK_SETUP_TOKEN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(generate_setup_token);
    Some(token)
}

impl RookContainer {
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        self.usage_repository.shutdown().await?;
        Ok(())
    }

    pub async fn build(config: &RookConfig) -> anyhow::Result<Self> {
        let model_catalog: Arc<dyn ModelCatalogPort> = Arc::new(
            StaticModelCatalog::with_declarations(config.model_catalog.clone())
                .map_err(anyhow::Error::msg)?,
        );
        let default_combo = config
            .routing
            .default_combo
            .as_deref()
            .map(ComboId::parse_str)
            .transpose()
            .map_err(|error| {
                anyhow::anyhow!("invalid routing.default_combo: expected a combo UUID: {error}")
            })?;
        // Run startup migrations once before constructing any repositories
        run_startup_migrations(&config.database.db_path)?;

        // 1. Cache
        let cache: Arc<dyn CachePort> = build_cache_port(config);

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

        // Model catalog — shared across DynamicProviderBuilder and RookUsecases

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

        let (authenticate_client_api, manage_api_keys) =
            build_api_key_auth(config, &api_key_repo, &registry)?;

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

        // 7a. Usage repository — concrete SqliteUsageRepository stored on container for retention
        let sqlite_usage: Arc<SqliteUsageRepository> = Arc::new(
            SqliteUsageRepository::new(Path::new(&config.database.db_path))
                .map_err(|e| anyhow::anyhow!("failed to create usage repository: {e}"))?,
        );
        let usage_recorder: Option<Arc<dyn UsageRecorderPort>> = Some(sqlite_usage.clone());
        let quota_observation_recorder: Arc<dyn QuotaObservationRecorderPort> =
            sqlite_usage.clone();

        // 7b. ManageConnections (provider CRUD) — shares the recorder with provider adapters.
        let manage_connections = build_manage_connections(
            config,
            &provider_repo,
            &registry,
            &model_catalog,
            Some(quota_observation_recorder.clone()),
        )?;

        // 7c. Combo repository — SQLite-backed combo storage.
        // The same SQLite file backs combo aliases (V7 table); a second
        // connection serves the alias port (each repo owns its connection
        // with FK enforcement, same as provider/alias repos above).
        let combo_repo: Arc<dyn ComboRepositoryPort> =
            Arc::new(ComboSqliteRepository::new(&config.database.db_path)?);
        let combo_alias_repo: Arc<dyn ComboAliasRepositoryPort> =
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
                    created_at: shared_kernel::Utc::now(),
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
        if let Some(ref manage_connections) = manage_connections {
            manage_connections.start_oauth_refresh_task();
        }

        let admin =
            admin_result.map_err(|e| anyhow::anyhow!("failed to ensure admin user: {}", e))?;
        tracing::info!(admin_id = %admin.id, "admin user ready");

        // 8b. Seed combos from config into SQLite
        seed_combos_from_config(&combo_repo, &config.combos, default_combo).await?;
        if let Some(id) = default_combo {
            let combo = combo_repo
                .find(&id)
                .await
                .map_err(|error| anyhow::anyhow!("cannot validate routing.default_combo {id}: {error}"))?
                .ok_or_else(|| anyhow::anyhow!("routing.default_combo {id} does not exist; create the combo or remove routing.default_combo"))?;
            combo
                .validate()
                .map_err(|error| anyhow::anyhow!("invalid routing.default_combo {id}: {error}"))?;
        }

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

        // Create telemetry tracker and spawn cleanup task if enabled
        let telemetry_tracker = if config.telemetry.enabled {
            let telemetry_config: observability::TelemetryConfig = config.telemetry.clone().into();
            let tracker = Arc::new(observability::TelemetryTracker::new(telemetry_config));
            // Spawn background cleanup task
            let _cleanup_task = tracker.clone().start_cleanup_task();
            Some(tracker)
        } else {
            None
        };

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
                    telemetry_tracker, // telemetry tracker from config
                )
                .with_combo_alias_repository(combo_alias_repo.clone())
                .with_default_combo(default_combo)
                .with_quota_observation_reader(quota_observation_recorder.clone()),
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
                model_catalog.clone(),
                fallback_router.clone(),
            ));

        // Check initialization state to decide on setup token.
        // Run AFTER usecases is built so we can write the token into it.
        let bootstrap_state = bootstrap_status.execute().await?;
        let setup_token_value = resolve_setup_token_value(bootstrap_state.is_initialized);

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
        let rate_limit_store = build_rate_limit_store(config);

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
            quota_observation_recorder,
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
pub fn build_rate_limiter_config(
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::metadata(&secret_path)
                .map_err(|e| {
                    anyhow::anyhow!("failed to inspect api_key_secret.key permissions: {e}")
                })?
                .permissions();
            if permissions.mode() & 0o077 != 0 {
                std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| {
                        anyhow::anyhow!("failed to restrict api_key_secret.key permissions: {e}")
                    })?;
            }
        }
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
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&secret_path)
            .map_err(|e| anyhow::anyhow!("failed to write api_key_secret.key: {e}"))?;
        file.write_all(secret.as_bytes())
            .and_then(|()| file.set_permissions(std::fs::Permissions::from_mode(0o600)))
            .map_err(|e| anyhow::anyhow!("failed to secure api_key_secret.key: {e}"))?;
    }
    #[cfg(not(unix))]
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
/// Errors are logged without blocking startup, except for the configured default.
async fn seed_combos_from_config(
    combo_repo: &Arc<dyn ComboRepositoryPort>,
    combos: &[crate::config::ComboConfig],
    default_combo: Option<ComboId>,
) -> anyhow::Result<()> {
    if combos.is_empty() {
        return Ok(());
    }

    let mut seeded = 0;
    let mut failed = 0;

    for combo_cfg in combos {
        match process_combo_config(combo_repo, combo_cfg).await {
            Ok(()) => seeded += 1,
            Err(error) => {
                if ComboId::parse_str(&combo_cfg.id).ok() == default_combo
                    && default_combo.is_some()
                {
                    return Err(anyhow::anyhow!(
                        "invalid routing.default_combo {}: cannot load configured combo: {error}",
                        combo_cfg.id
                    ));
                }
                failed += 1;
            }
        }
    }

    if seeded > 0 || failed > 0 {
        tracing::info!(seeded, failed, "combo seeding from config completed");
    }
    Ok(())
}

async fn process_combo_config(
    combo_repo: &Arc<dyn ComboRepositoryPort>,
    combo_cfg: &crate::config::ComboConfig,
) -> anyhow::Result<()> {
    let combo_id = parse_combo_id(combo_cfg)?;
    let strategy = parse_combo_strategy(combo_cfg)?;
    let steps = build_combo_steps(combo_cfg);

    let mut combo = Combo::new(combo_cfg.name.clone(), strategy, steps);
    combo.id = combo_id;

    combo.validate().map_err(|e| {
        tracing::warn!(
            combo_name = %combo_cfg.name,
            error = %e,
            "combo validation failed - skipping"
        );
        anyhow::anyhow!("validation failed")
    })?;

    upsert_combo(combo_repo, &combo, combo_cfg).await
}

fn parse_combo_id(combo_cfg: &crate::config::ComboConfig) -> anyhow::Result<ComboId> {
    ComboId::parse_str(&combo_cfg.id).map_err(|e| {
        tracing::warn!(
            combo_name = %combo_cfg.name,
            combo_id = %combo_cfg.id,
            error = %e,
            "invalid combo ID format - skipping"
        );
        anyhow::anyhow!("invalid ID")
    })
}

fn parse_combo_strategy(combo_cfg: &crate::config::ComboConfig) -> anyhow::Result<ComboStrategy> {
    match combo_cfg.strategy.as_str() {
        "priority" => Ok(ComboStrategy::Priority),
        other => {
            tracing::warn!(
                combo_name = %combo_cfg.name,
                strategy = %other,
                "unsupported strategy - skipping"
            );
            Err(anyhow::anyhow!("unsupported strategy"))
        }
    }
}

fn build_combo_steps(combo_cfg: &crate::config::ComboConfig) -> Vec<ComboStep> {
    combo_cfg
        .steps
        .iter()
        .map(|s| ComboStep {
            provider_id: ProviderId::new(s.provider_id.clone()),
            model: ModelId::new(s.model.clone()),
            connection_id: None,
            priority: s.priority,
        })
        .collect()
}

async fn upsert_combo(
    combo_repo: &Arc<dyn ComboRepositoryPort>,
    combo: &Combo,
    combo_cfg: &crate::config::ComboConfig,
) -> anyhow::Result<()> {
    match combo_repo.find(&combo.id).await {
        Ok(Some(existing)) => update_existing_combo(combo_repo, combo, &existing, combo_cfg).await,
        Ok(None) => create_new_combo(combo_repo, combo, combo_cfg).await,
        Err(e) => {
            tracing::warn!(
                combo_name = %combo_cfg.name,
                error = %e,
                "failed to check existing combo - skipping"
            );
            Err(anyhow::anyhow!("find failed"))
        }
    }
}

async fn update_existing_combo(
    combo_repo: &Arc<dyn ComboRepositoryPort>,
    combo: &Combo,
    existing: &Combo,
    combo_cfg: &crate::config::ComboConfig,
) -> anyhow::Result<()> {
    let mut updated = combo.clone();
    updated.created_at = existing.created_at;

    combo_repo.update(&updated).await.map_err(|e| {
        tracing::warn!(
            combo_name = %combo_cfg.name,
            error = %e,
            "failed to update combo - skipping"
        );
        anyhow::anyhow!("update failed")
    })?;

    tracing::debug!(
        combo_id = %combo.id,
        combo_name = %combo_cfg.name,
        "updated combo from config"
    );
    Ok(())
}

async fn create_new_combo(
    combo_repo: &Arc<dyn ComboRepositoryPort>,
    combo: &Combo,
    combo_cfg: &crate::config::ComboConfig,
) -> anyhow::Result<()> {
    combo_repo.create(combo).await.map_err(|e| {
        tracing::warn!(
            combo_name = %combo_cfg.name,
            error = %e,
            "failed to create combo - skipping"
        );
        anyhow::anyhow!("create failed")
    })?;

    tracing::debug!(
        combo_id = %combo.id,
        combo_name = %combo_cfg.name,
        "created combo from config"
    );
    Ok(())
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
    async fn stats(&self) -> CortexResult<rook_core::CacheStats> {
        Ok(rook_core::CacheStats {
            hits: 0,
            misses: 0,
            evictions: 0,
            entries: 0,
            max_entries: 0,
            token_cache: rook_core::TokenCacheStats::default(),
        })
    }
    async fn delete_by_signature(&self, _: &str) -> CortexResult<usize> {
        Ok(0)
    }
    async fn list_signatures(&self) -> CortexResult<Vec<rook_core::SignatureEntry>> {
        Ok(Vec::new())
    }
    async fn get_by_signature(
        &self,
        _: &str,
    ) -> CortexResult<Option<rook_core::CompletionResponse>> {
        Ok(None)
    }
    async fn increment_token_cache_hit(&self, _: u64, _: f64) -> CortexResult<()> {
        Ok(())
    }
    async fn increment_token_cache_miss(&self) -> CortexResult<()> {
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
    AzureOpenAIRequiresValidEndpoint,
    AzureOpenAIRequiresOAuth,
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
            Self::AzureOpenAIRequiresValidEndpoint => {
                write!(f, "Azure OpenAI requires a valid HTTPS resource endpoint")
            }
            Self::AzureOpenAIRequiresOAuth => {
                write!(
                    f,
                    "Azure OpenAI provider connections require OAuth credentials"
                )
            }
            Self::ConstructionFailed(msg) => write!(f, "provider construction failed: {msg}"),
        }
    }
}

fn validate_azure_openai_endpoint(value: &str) -> Option<String> {
    let endpoint = reqwest::Url::parse(value).ok()?;
    let host = endpoint.host_str()?;
    let azure_host =
        host.ends_with(".openai.azure.com") || host.ends_with(".services.ai.azure.com");
    if endpoint.scheme() != "https"
        || !azure_host
        || endpoint.username() != ""
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !matches!(endpoint.path(), "" | "/")
    {
        return None;
    }
    Some(endpoint.as_str().trim_end_matches('/').to_string())
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
    models: Vec<ModelId>,
) -> Result<Arc<dyn ProviderPort>, ProviderBuildError> {
    build_provider_from_connection_with_recorder(
        connection_id,
        kind,
        credentials,
        base_url_override,
        models,
        None,
        None,
    )
}

/// T1 — P1 canonical URLs: decide whether upstream quota capture stays enabled.
///
/// `None` (backend default) always retains the recorder. An explicit override
/// retains it only when it normalizes to the provider's canonical endpoint:
/// - OpenAI: `https://api.openai.com` with or without `/v1`
/// - Anthropic: `https://api.anthropic.com`
/// - Groq: `https://api.groq.com/openai/v1`
///
/// Anything else (custom proxy / mock / non-https) disables capture so
/// third-party rate-limit headers are never attributed to the canonical
/// provider.
///
/// Normalization: trim surrounding whitespace, case-insensitive scheme and
/// host, ignore trailing slashes, tolerate `/v1` for OpenAI.
pub fn is_canonical_base_url(kind: ProviderKind, base_url_override: Option<&str>) -> bool {
    let Some(raw) = base_url_override else {
        return true;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    // Canonical URLs never carry userinfo, query, or fragment.
    if rest.contains(['@', '?', '#']) {
        return false;
    }
    let (host, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };
    if host.is_empty() || host.contains(':') {
        // Empty host or explicit port suffix => custom.
        return false;
    }
    let host = host.to_ascii_lowercase();
    // Strip trailing slashes: "/" -> "", "/v1/" -> "/v1".
    let mut norm_path = path;
    while norm_path.len() > 1 && norm_path.ends_with('/') {
        norm_path = &norm_path[..norm_path.len() - 1];
    }
    if norm_path == "/" {
        norm_path = "";
    }
    let norm_path = norm_path.to_ascii_lowercase();
    match kind {
        ProviderKind::OpenAI => {
            host == "api.openai.com" && (norm_path.is_empty() || norm_path == "/v1")
        }
        ProviderKind::Anthropic => host == "api.anthropic.com" && norm_path.is_empty(),
        ProviderKind::Groq => host == "api.groq.com" && norm_path == "/openai/v1",
        // Ollama / OllamaCloud / Gemini never carry quota capture; no explicit
        // URL is canonical (the recorder is ignored downstream anyway).
        _ => false,
    }
}

pub struct ProviderConnectionBuildOptions {
    pub quota_recorder: Option<Arc<dyn QuotaObservationRecorderPort>>,
    pub oauth_token_port: Option<Arc<dyn rook_core::OAuthTokenPort>>,
    pub endpoint_policy: rook_core::EndpointPolicy,
}

pub fn build_provider_from_connection_with_recorder(
    connection_id: &ConnectionId,
    kind: ProviderKind,
    credentials: &DecryptedCredentials,
    base_url_override: Option<String>,
    models: Vec<ModelId>,
    quota_recorder: Option<Arc<dyn QuotaObservationRecorderPort>>,
    oauth_token_port: Option<Arc<dyn rook_core::OAuthTokenPort>>,
) -> Result<Arc<dyn ProviderPort>, ProviderBuildError> {
    build_provider_from_connection_with_policy(
        connection_id,
        kind,
        credentials,
        base_url_override,
        models,
        ProviderConnectionBuildOptions {
            quota_recorder,
            oauth_token_port,
            endpoint_policy: rook_core::EndpointPolicy::default(),
        },
    )
}

pub fn build_provider_from_connection_with_policy(
    connection_id: &ConnectionId,
    kind: ProviderKind,
    credentials: &DecryptedCredentials,
    base_url_override: Option<String>,
    models: Vec<ModelId>,
    options: ProviderConnectionBuildOptions,
) -> Result<Arc<dyn ProviderPort>, ProviderBuildError> {
    let ProviderConnectionBuildOptions {
        quota_recorder,
        oauth_token_port,
        endpoint_policy,
    } = options;
    let quota_recorder =
        quota_recorder.filter(|_| is_canonical_base_url(kind, base_url_override.as_deref()));
    // Extract credentials — use access_token for OAuth where the provider supports it
    let api_key = match credentials {
        DecryptedCredentials::ApiKey { api_key } => api_key.clone(),
        DecryptedCredentials::OAuth { access_token, .. } if oauth_token_port.is_none() => {
            // OAuth access tokens can be used as API keys for providers that accept them.
            // For providers that don't support OAuth natively, this returns an error below.
            access_token.clone()
        }
        DecryptedCredentials::OAuth { .. } => String::new(),
    };

    let provider = match kind {
        ProviderKind::OpenAI => {
            let config = providers_openai::OpenAIProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                api_key,
                base_url: base_url_override.unwrap_or_else(|| "https://api.openai.com".to_string()),
                models: models.clone(),
                timeout_secs: 60,
            };
            let provider = match (&oauth_token_port, credentials) {
                (Some(token_port), DecryptedCredentials::OAuth { .. }) => {
                    providers_openai::OpenAIProvider::new_with_oauth_and_quota_recorder(
                        config,
                        *connection_id,
                        token_port.clone(),
                        quota_recorder.clone(),
                    )
                }
                _ => providers_openai::OpenAIProvider::new_with_quota_recorder(
                    config,
                    quota_recorder.clone(),
                ),
            }
            .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?;
            Arc::new(provider) as Arc<dyn ProviderPort>
        }
        ProviderKind::OpenAiCompatible => {
            let base_url = base_url_override.ok_or_else(|| {
                ProviderBuildError::ConstructionFailed(
                    "OpenAI-compatible provider requires base_url".to_string(),
                )
            })?;
            if matches!(credentials, DecryptedCredentials::OAuth { .. }) {
                return Err(ProviderBuildError::ConstructionFailed(
                    "OpenAI-compatible providers require API key authentication".to_string(),
                ));
            }
            let config = providers_openai::OpenAIProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                api_key,
                base_url,
                models: models.clone(),
                timeout_secs: 60,
            };
            Arc::new(
                providers_openai::OpenAIProvider::new_with_endpoint_policy(config, endpoint_policy)
                    .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?,
            ) as Arc<dyn ProviderPort>
        }
        ProviderKind::Anthropic => {
            let config = providers_anthropic::AnthropicProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                api_key,
                base_url: base_url_override
                    .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
                models: models.clone(),
                timeout_secs: 60,
            };
            providers_anthropic::AnthropicProvider::new_with_quota_recorder(
                config,
                quota_recorder.clone(),
            )
            .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?
        }
        ProviderKind::Ollama => {
            let base_url = base_url_override.ok_or(ProviderBuildError::OllamaRequiresBaseUrl)?;
            let config = providers_ollama::OllamaProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                base_url,
                models: models.clone(),
                timeout_secs: 300,
                api_key: None,
            };
            providers_ollama::OllamaProvider::new(config)
                .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?
        }
        ProviderKind::OllamaCloud => {
            // Ollama Cloud reuses the same `OllamaProvider` implementation —
            // it differs only in the base URL (defaults to https://ollama.com)
            // and the required Bearer auth token. The API surface
            // (`/api/chat`) is identical to local Ollama.
            let base_url = base_url_override.unwrap_or_else(|| "https://ollama.com".to_string());
            let config = providers_ollama::OllamaProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                base_url,
                models: models.clone(),
                timeout_secs: 300,
                api_key: if api_key.is_empty() {
                    None
                } else {
                    Some(api_key)
                },
            };
            providers_ollama::OllamaProvider::new(config)
                .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?
        }
        ProviderKind::Gemini => {
            let config = providers_gemini::GeminiProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                api_key,
                base_url: base_url_override,
                models: models.clone(),
                timeout_secs: 60,
            };
            match (oauth_token_port, credentials) {
                (Some(token_port), DecryptedCredentials::OAuth { project_id, .. }) => {
                    providers_gemini::GeminiProvider::new_with_oauth(
                        config,
                        *connection_id,
                        project_id.clone(),
                        token_port,
                    )
                }
                _ => providers_gemini::GeminiProvider::new(config),
            }
            .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?
        }
        ProviderKind::Groq => {
            let config = providers_groq::GroqProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                api_key,
                base_url: None,
                models: models.clone(),
                timeout_secs: 60,
            };
            providers_groq::GroqProvider::new_with_quota_recorder(config, quota_recorder.clone())
                .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?
        }
        ProviderKind::AzureOpenAI => {
            if !matches!(credentials, DecryptedCredentials::OAuth { .. }) {
                return Err(ProviderBuildError::AzureOpenAIRequiresOAuth);
            }
            let endpoint = base_url_override
                .as_deref()
                .and_then(validate_azure_openai_endpoint)
                .ok_or(ProviderBuildError::AzureOpenAIRequiresValidEndpoint)?;
            let config = providers_openai::OpenAIProviderConfig {
                id: ProviderId::new(connection_id.to_string()),
                api_key,
                base_url: format!("{endpoint}/openai/v1"),
                models: models.clone(),
                timeout_secs: 60,
            };
            let provider = match oauth_token_port {
                Some(token_port) if matches!(credentials, DecryptedCredentials::OAuth { .. }) => {
                    providers_openai::OpenAIProvider::new_with_oauth(
                        config,
                        *connection_id,
                        token_port,
                    )
                }
                _ => providers_openai::OpenAIProvider::new(config),
            }
            .map_err(|e| ProviderBuildError::ConstructionFailed(e.to_string()))?;
            Arc::new(provider) as Arc<dyn ProviderPort>
        }
    };

    Ok(provider)
}

/// Restores managed identity independently of the connection-scoped registry ID.
struct ConnectionQuotaRecorder {
    recorder: Arc<dyn QuotaObservationRecorderPort>,
    provider_runtime_id: ProviderId,
    connection_id: ConnectionId,
}

/// Test/CRUD seam for the canonical-URL quota gate plus identity wrapping.
///
/// Kept `pub` so integration tests exercise the production canonical-URL
/// decision without reaching into private state.
pub fn quota_recorder_for_connection(
    kind: ProviderKind,
    base_url_override: Option<&str>,
    connection_id: ConnectionId,
    provider_runtime_id: ProviderId,
    recorder: Option<Arc<dyn QuotaObservationRecorderPort>>,
) -> Option<Arc<dyn QuotaObservationRecorderPort>> {
    if !is_canonical_base_url(kind, base_url_override) {
        return None;
    }
    recorder.map(|recorder| {
        Arc::new(ConnectionQuotaRecorder {
            recorder,
            provider_runtime_id,
            connection_id,
        }) as Arc<dyn QuotaObservationRecorderPort>
    })
}

#[async_trait]
impl QuotaObservationRecorderPort for ConnectionQuotaRecorder {
    async fn record_observation(
        &self,
        observation: QuotaObservation,
    ) -> shared_kernel::CortexResult<()> {
        self.record_observations(vec![observation]).await
    }

    async fn record_observations(
        &self,
        mut observations: Vec<QuotaObservation>,
    ) -> shared_kernel::CortexResult<()> {
        for observation in &mut observations {
            observation.provider = self.provider_runtime_id.clone();
            observation.connection_id = Some(self.connection_id);
        }
        self.recorder.record_observations(observations).await
    }

    async fn list_observations(
        &self,
        provider: Option<ProviderId>,
    ) -> shared_kernel::CortexResult<Vec<QuotaObservation>> {
        self.recorder.list_observations(provider).await
    }

    /// Forward bounded reads to the shared store.
    ///
    /// The stored observations already carry the connection identity, so the
    /// read needs no rewriting here.
    async fn latest_observations(
        &self,
        query: &rook_core::QuotaObservationQuery,
    ) -> shared_kernel::CortexResult<Vec<QuotaObservation>> {
        self.recorder.latest_observations(query).await
    }
}

#[derive(Clone)]
struct DynamicProviderBuilder {
    catalog: Arc<dyn ModelCatalogPort>,
    quota_observation_recorder: Option<Arc<dyn QuotaObservationRecorderPort>>,
}

impl DynamicProviderBuilder {
    fn new(
        catalog: Arc<dyn ModelCatalogPort>,
        quota_observation_recorder: Option<Arc<dyn QuotaObservationRecorderPort>>,
    ) -> Self {
        Self {
            catalog,
            quota_observation_recorder,
        }
    }

    fn quota_recorder(
        &self,
        input: &ProviderBuildInput,
    ) -> Option<Arc<dyn QuotaObservationRecorderPort>> {
        // Default endpoints and explicit canonical URLs both qualify; custom
        // endpoints must not have third-party headers attributed to the provider.
        quota_recorder_for_connection(
            input.provider_kind,
            input.base_url.as_deref(),
            input.connection_id,
            input.provider_runtime_id.clone(),
            self.quota_observation_recorder.clone(),
        )
    }
}

#[async_trait]
impl ProviderBuilderPort for DynamicProviderBuilder {
    async fn build(
        &self,
        input: ProviderBuildInput,
    ) -> Result<Arc<dyn ProviderPort>, ManageConnectionsError> {
        // Query model catalog for models matching this provider kind
        let catalog_entries = self
            .catalog
            .list_for_connection(input.provider_kind, &input.connection_id)
            .await;
        let catalog_models: Vec<ModelId> = catalog_entries
            .into_iter()
            .filter(|e| e.provider_kind == input.provider_kind)
            .map(|e| ModelId::new(e.model_id))
            .collect();
        let models = match input.provider_kind {
            ProviderKind::AzureOpenAI => input.default_model.clone().into_iter().collect(),
            ProviderKind::OpenAiCompatible => input.supported_models.clone(),
            _ => catalog_models,
        };

        let quota_recorder = self.quota_recorder(&input);
        let mut metadata = std::collections::HashMap::new();
        for model in &models {
            metadata.insert(
                model.clone(),
                self.catalog
                    .resolve(
                        input.provider_kind,
                        Some(&input.connection_id),
                        model.as_str(),
                    )
                    .await,
            );
        }
        let ceiling = adapter_capability_ceiling(input.provider_kind);
        let provider = build_provider_from_connection_with_policy(
            &input.connection_id,
            input.provider_kind,
            &input.decrypted_credentials,
            input.base_url,
            models,
            ProviderConnectionBuildOptions {
                quota_recorder,
                oauth_token_port: Some(input.oauth_token_port),
                endpoint_policy: input.endpoint_policy,
            },
        )
        .map_err(|e| ManageConnectionsError::RegistryUpdateFailed(e.to_string()))?;
        Ok(Arc::new(
            rook_usecases::catalogued_provider::CataloguedProvider::new(
                provider, metadata, ceiling,
            ),
        ))
    }
}

/// Verified implementation limits, not claims about any upstream model.
fn adapter_capability_ceiling(kind: ProviderKind) -> rook_core::ModelCapabilities {
    use rook_core::{
        CapabilityOverrides,
        CapabilityState::{Supported, Unsupported},
        ModelCapabilities,
    };
    let mut out = ModelCapabilities::default();
    out.apply(
        &CapabilityOverrides {
            tools: Some(
                if matches!(
                    kind,
                    ProviderKind::OpenAI
                        | ProviderKind::OpenAiCompatible
                        | ProviderKind::AzureOpenAI
                        | ProviderKind::Anthropic
                ) {
                    Supported
                } else {
                    Unsupported
                },
            ),
            vision: Some(Unsupported),
            reasoning: Some(Unsupported),
            streaming: Some(if kind == ProviderKind::Gemini {
                Unsupported
            } else {
                Supported
            }),
            context_tokens: None,
        },
        "adapter",
    );
    out
}
