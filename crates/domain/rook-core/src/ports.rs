// Ports (traits) for rook-core.
//
// Each port is a capability that the domain needs but cannot implement itself.
// Implementations live in `infrastructure/` crates.
//
// Naming convention: `{Capability}Port`

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use shared_kernel::{CacheKey, ConnectionId, ModelId, NuxaResult, ProviderId};

use super::{ApiKeyId, ApiKeyRepositoryError, ApiKeySubject, ProviderConnection, RepositoryError};
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
// ProviderRegistryPort — lookup for runtime providers
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum RegistryError {
    #[error("provider build failed for '{provider_id}': {reason}")]
    ProviderBuildFailed { provider_id: ProviderId, reason: String },
    #[error("registry locked")]
    RegistryLocked,
}

pub trait ProviderRegistryPort: Send + Sync {
    fn providers(&self) -> Vec<ProviderId>;
    fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>>;
    fn replace_all(&self, providers: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError>;
    fn upsert(&self, provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError>;
    fn remove(&self, id: &ProviderId) -> Result<(), RegistryError>;
}

// ---------------------------------------------------------------------------
// ProviderRepositoryPort — persistence for provider connections
// ---------------------------------------------------------------------------

#[async_trait]
pub trait ProviderRepositoryPort: Send + Sync {
    async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError>;
    async fn find(&self, id: &ConnectionId) -> Result<Option<ProviderConnection>, RepositoryError>;
    async fn create(&self, conn: &ProviderConnection) -> Result<(), RepositoryError>;
    async fn update(
        &self,
        conn: &ProviderConnection,
        expected_updated_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError>;
    async fn delete(&self, id: &ConnectionId) -> Result<(), RepositoryError>;
}

// ---------------------------------------------------------------------------
// ApiKeyRepositoryPort — persistence for client API key subjects
// ---------------------------------------------------------------------------

#[async_trait]
pub trait ApiKeyRepositoryPort: Send + Sync {
    async fn find_active_by_hash(
        &self,
        hash: &str,
    ) -> Result<Option<ApiKeySubject>, ApiKeyRepositoryError>;

    async fn record_last_used(
        &self,
        id: &ApiKeyId,
        used_at: DateTime<Utc>,
    ) -> Result<(), ApiKeyRepositoryError>;
}

// ---------------------------------------------------------------------------
// KeyManager — encryption boundary for provider credentials
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialEncryptionError {
    Encrypt(String),
    Decrypt(String),
}

impl std::fmt::Display for CredentialEncryptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encrypt(message) => write!(f, "credential encryption failed: {message}"),
            Self::Decrypt(message) => write!(f, "credential decryption failed: {message}"),
        }
    }
}

impl std::error::Error for CredentialEncryptionError {}

pub trait KeyManager: Send + Sync {
    fn encrypt(&self, plaintext: &str) -> Result<String, CredentialEncryptionError>;
    fn decrypt(&self, ciphertext: &str) -> Result<String, CredentialEncryptionError>;
}

// ---------------------------------------------------------------------------
// BoxStream re-export for convenience
// ---------------------------------------------------------------------------

pub type BoxStream<'a, T> = futures::stream::BoxStream<'a, T>;
