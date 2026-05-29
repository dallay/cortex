// di — dependency injection container
//
// This is the ONLY place where all crates are assembled.

use std::sync::Arc;
use std::time::Duration;

use audit_sqlite::SqliteAudit;
use cache_memory::InMemoryCache;
use encryption_inmemory::AesGcmKeyManager;
use provider_sqlite::SqliteProviderRepository;
use providers_anthropic::AnthropicProvider;
use providers_gemini::GeminiProvider;
use providers_groq::GroqProvider;
use providers_ollama::OllamaProvider;
use providers_openai::OpenAIProvider;
use rook_core::{
    AuditPort, CachePort, ProviderId, ProviderPort, ProviderRegistryPort, ProviderRepositoryPort,
    RouterPort,
};
use rook_usecases::{
    FallbackRouter, HealthCheck, ManageConnections, ManageProviders, RookUsecases, RouteRequest,
    RoutingStrategy,
};

use crate::config::{ProviderConfig, RookConfig};

pub struct RookContainer {
    pub usecases: Arc<RookUsecases>,
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
        let audit: Arc<dyn AuditPort> = Arc::new(SqliteAudit::new(&config.audit.db_path)?);

        // 4. Router and provider registry
        let strategy: RoutingStrategy = config.routing.strategy.into();
        let fallback_router: Arc<FallbackRouter> =
            Arc::new(FallbackRouter::new(providers, strategy));
        let router: Arc<dyn RouterPort> = fallback_router.clone();
        let registry: Arc<dyn ProviderRegistryPort> = fallback_router;

        let manage_connections = if config.provider_crud.enabled {
            let passphrase = required_env("ENCRYPTION_PASSPHRASE")?;
            let salt = required_env("ENCRYPTION_SALT")?;
            let key_manager = Arc::new(
                AesGcmKeyManager::from_passphrase_and_salt(&passphrase, &salt)
                    .map_err(|e| anyhow::anyhow!("invalid provider CRUD encryption config: {e}"))?,
            );
            let repo: Arc<dyn ProviderRepositoryPort> = Arc::new(SqliteProviderRepository::new(
                &config.provider_crud.db_path,
            )?);
            Some(ManageConnections::new(repo, registry.clone(), key_manager))
        } else {
            None
        };

        // 5. Use cases
        let usecases = Arc::new(RookUsecases {
            route_request: RouteRequest::new(router.clone(), cache.clone(), audit.clone()),
            manage_providers: ManageProviders::new(router.clone()),
            health_check: HealthCheck::new(registry),
            manage_connections,
        });

        Ok(Self { usecases })
    }
}

fn required_env(name: &str) -> anyhow::Result<String> {
    let value = std::env::var(name)
        .map_err(|_| anyhow::anyhow!("{name} is required when provider_crud.enabled=true"))?;
    if value.trim().is_empty() {
        anyhow::bail!("{name} must not be empty when provider_crud.enabled=true");
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
