// Ports (traits) for rook-core.
//
// Each port is a capability that the domain needs but cannot implement itself.
// Implementations live in `infrastructure/` crates.
//
// Naming convention: `{Capability}Port`

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use shared_kernel::{CacheKey, ModelId, NuxaResult, ProviderId};

use super::{AuditEntry, CompletionRequest, CompletionResponse, HealthStatus, StreamChunk};

/// ---------------------------------------------------------------------------
/// ProviderPort — the primary port for LLM providers
/// ---------------------------------------------------------------------------
/// Main port for LLM providers (OpenAI, Anthropic, Ollama, etc.).
/// Every provider implementation must implement this.
#[async_trait]
pub trait ProviderPort: Send + Sync + 'static {
    fn id(&self) -> &ProviderId;
    fn supported_models(&self) -> &[ModelId];

    /// Check if this provider can handle the given model
    fn supports_model(&self, model: &ModelId) -> bool {
        self.supported_models().contains(model)
    }

    /// Synchronous health check — fast, no network call
    fn is_available(&self) -> bool;

    /// Full health check with latency measurement
    async fn health_check(&self) -> HealthStatus;

    /// Execute a completion request
    async fn complete(&self, req: &CompletionRequest) -> NuxaResult<CompletionResponse>;

    /// Stream a completion response
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> NuxaResult<BoxStream<'_, NuxaResult<StreamChunk>>>;
}

// ---------------------------------------------------------------------------
// RouterPort — provider selection and fallback
// ---------------------------------------------------------------------------

/// RouterPort decides which provider to use for a given request.
/// Implementations carry the fallback/routing strategy.
#[async_trait]
pub trait RouterPort: Send + Sync {
    /// Select the best provider for this request.
    /// Returns the selected provider, never an error if at least one provider is available.
    async fn select(&self, req: &CompletionRequest) -> NuxaResult<Arc<dyn ProviderPort>>;

    /// Called when a provider call fails — allows the router to update
    /// internal state (circuit breaker, weights, etc.)
    async fn on_failure(&self, provider: &ProviderId, error: &shared_kernel::NuxaError);

    /// Get the list of all registered providers
    fn providers(&self) -> Vec<ProviderId>;
}

// ---------------------------------------------------------------------------
// CachePort — response caching
// ---------------------------------------------------------------------------

#[async_trait]
pub trait CachePort: Send + Sync {
    async fn get(&self, key: &CacheKey) -> NuxaResult<Option<CompletionResponse>>;
    async fn set(
        &self,
        key: &CacheKey,
        value: &CompletionResponse,
        ttl: Duration,
    ) -> NuxaResult<()>;
    async fn delete(&self, key: &CacheKey) -> NuxaResult<()>;
    async fn clear(&self) -> NuxaResult<()>;
}

// ---------------------------------------------------------------------------
// AuditPort — audit logging
// ---------------------------------------------------------------------------

#[async_trait]
pub trait AuditPort: Send + Sync {
    async fn record(&self, entry: AuditEntry) -> NuxaResult<()>;
}

// ---------------------------------------------------------------------------
// HealthPort — aggregated health checks
// ---------------------------------------------------------------------------

#[async_trait]
pub trait HealthPort: Send + Sync {
    async fn health(&self) -> Vec<HealthStatus>;
}

// ---------------------------------------------------------------------------
// BoxStream re-export for convenience
// ---------------------------------------------------------------------------

pub type BoxStream<'a, T> = futures::stream::BoxStream<'a, T>;
