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
use shared_kernel::{CacheKey, ConnectionId, CortexResult, ModelId, ProviderId};

use super::{
    ApiFormat, AuditEntry, CompletionRequest, CompletionResponse, CostBreakdown, HealthStatus,
    Pagination, StreamChunk, UsageEntry, UsageFilters, UsageSummary,
};
use super::{
    ApiKeyId, ApiKeyRecord, ApiKeyRepositoryError, ApiKeySubject, NewSession, NewUser,
    PasswordHash, ProviderConnection, ProviderKind, RepositoryError, Session, SessionId, User,
    UserId,
};

/// ---------------------------------------------------------------------------
/// ProviderPort — the primary port for LLM providers
/// ---------------------------------------------------------------------------
/// Main port for LLM providers (OpenAI, Anthropic, Ollama, etc.).
/// Every provider implementation must implement this.
#[async_trait]
pub trait ProviderPort: Send + Sync + 'static {
    fn id(&self) -> &ProviderId;
    fn supported_models(&self) -> &[ModelId];

    /// Wire format expected by this provider implementation.
    fn api_format(&self) -> ApiFormat;

    /// Check if this provider can handle the given model
    fn supports_model(&self, model: &ModelId) -> bool {
        self.supported_models().contains(model)
    }

    /// Synchronous health check — fast, no network call
    fn is_available(&self) -> bool;

    /// Full health check with latency measurement
    async fn health_check(&self) -> HealthStatus;

    /// Execute a completion request
    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse>;

    /// Stream a completion response
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>>;
}

// ---------------------------------------------------------------------------
// FormatTranslatorPort — explicit domain-pivot format routing
// ---------------------------------------------------------------------------

pub trait FormatTranslatorPort: Send + Sync {
    fn translate_request(
        &self,
        from: ApiFormat,
        to: ApiFormat,
        req: CompletionRequest,
    ) -> CortexResult<CompletionRequest>;

    fn translate_response(
        &self,
        from: ApiFormat,
        to: ApiFormat,
        resp: CompletionResponse,
    ) -> CortexResult<CompletionResponse>;
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
    async fn select(&self, req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>>;

    /// Called when a provider call fails — allows the router to update
    /// internal state (circuit breaker, weights, etc.)
    async fn on_failure(&self, provider: &ProviderId, error: &shared_kernel::CortexError);

    /// Get the list of all registered providers
    fn providers(&self) -> Vec<ProviderId>;
}

// ---------------------------------------------------------------------------
// CachePort — response caching
// ---------------------------------------------------------------------------

#[async_trait]
pub trait CachePort: Send + Sync {
    async fn get(&self, key: &CacheKey) -> CortexResult<Option<CompletionResponse>>;
    async fn set(
        &self,
        key: &CacheKey,
        value: &CompletionResponse,
        ttl: Duration,
    ) -> CortexResult<()>;
    async fn delete(&self, key: &CacheKey) -> CortexResult<()>;
    async fn clear(&self) -> CortexResult<()>;
}

// ---------------------------------------------------------------------------
// AuditPort — audit logging
// ---------------------------------------------------------------------------

#[async_trait]
pub trait AuditPort: Send + Sync {
    async fn record(&self, entry: AuditEntry) -> CortexResult<()>;
}

#[async_trait]
pub trait UsageRecorderPort: Send + Sync {
    async fn record(&self, entry: UsageEntry) -> CortexResult<()>;

    async fn list(
        &self,
        filters: UsageFilters,
        pagination: Pagination,
    ) -> CortexResult<Vec<UsageEntry>>;

    async fn count(&self, filters: UsageFilters) -> CortexResult<u64>;

    async fn summary(&self, filters: UsageFilters) -> CortexResult<UsageSummary>;

    async fn cost_breakdown(&self, filters: UsageFilters) -> CortexResult<CostBreakdown>;
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
    ProviderBuildFailed {
        provider_id: ProviderId,
        reason: String,
    },
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
    async fn find_connection_id_by_runtime(
        &self,
        _provider: &ProviderId,
    ) -> Result<Option<ConnectionId>, RepositoryError> {
        Ok(None)
    }
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

    async fn list(&self) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError>;

    async fn find(&self, id: &ApiKeyId) -> Result<Option<ApiKeyRecord>, ApiKeyRepositoryError>;

    async fn create(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError>;

    async fn update(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError>;

    async fn delete(&self, id: &ApiKeyId) -> Result<(), ApiKeyRepositoryError>;

    async fn revoke(
        &self,
        id: &ApiKeyId,
        revoked_at: DateTime<Utc>,
    ) -> Result<(), ApiKeyRepositoryError>;

    /// Atomically replace the `key_hash` and `key_prefix` for the given id.
    /// Used by key rotation: the new hash invalidates the old one because
    /// `find_active_by_hash` looks up rows by `key_hash`. The new prefix
    /// keeps the displayed `rk-XXXXXXX…` snippet in sync with the secret.
    /// Returns `NotFound` if the row does not exist.
    async fn rotate_hash(
        &self,
        id: &ApiKeyId,
        new_hash: &str,
        new_prefix: &str,
    ) -> Result<(), ApiKeyRepositoryError>;

    async fn list_paginated(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError>;

    async fn count(&self) -> Result<i64, ApiKeyRepositoryError>;
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
// UserRepositoryPort — persistence for admin user
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum UserRepositoryError {
    #[error("user not found: {0}")]
    NotFound(UserId),
    #[error("duplicate username")]
    DuplicateUsername,
    #[error("database error: {0}")]
    Database(String),
}

#[async_trait]
pub trait UserRepositoryPort: Send + Sync {
    async fn find_by_username(&self, username: &str) -> Result<Option<User>, UserRepositoryError>;
    async fn find_by_id(&self, user_id: &UserId) -> Result<Option<User>, UserRepositoryError>;
    async fn has_any_user(&self) -> Result<bool, UserRepositoryError>;
    async fn create(&self, user: &NewUser) -> Result<User, UserRepositoryError>;
    async fn update_password_hash(
        &self,
        user_id: &UserId,
        hash: &PasswordHash,
    ) -> Result<(), UserRepositoryError>;
}

// ---------------------------------------------------------------------------
// SessionRepositoryPort — session token persistence
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SessionRepositoryError {
    #[error("session not found: {0}")]
    NotFound(SessionId),
    #[error("database error: {0}")]
    Database(String),
}

#[async_trait]
pub trait SessionRepositoryPort: Send + Sync {
    /// Create a new session. `token_hash` is the SHA-256 of the raw token bytes.
    async fn create(
        &self,
        session: &NewSession,
        token_hash: &str,
    ) -> Result<Session, SessionRepositoryError>;

    /// Find a valid (non-revoked, non-expired) session by token hash.
    async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<Session>, SessionRepositoryError>;

    /// Revoke a session by ID.
    async fn revoke(&self, session_id: &SessionId) -> Result<(), SessionRepositoryError>;

    /// Delete all expired sessions. Returns the count of deleted rows.
    async fn delete_expired(&self) -> Result<u64, SessionRepositoryError>;
}

// ---------------------------------------------------------------------------
// PasswordHasher — Argon2id password hashing
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PasswordHashError {
    #[error("hash generation failed")]
    HashGeneration,
    #[error("hash verification failed")]
    Verification,
}

#[async_trait]
pub trait PasswordHasher: Send + Sync {
    fn hash_password(&self, password: &str) -> Result<PasswordHash, PasswordHashError>;
    fn verify_password(
        &self,
        password: &str,
        hash: &PasswordHash,
    ) -> Result<bool, PasswordHashError>;
}

// ---------------------------------------------------------------------------
// BoxStream re-export for convenience
// ---------------------------------------------------------------------------

pub type BoxStream<'a, T> = futures::stream::BoxStream<'a, T>;

#[cfg(test)]
mod usage_port_tests {
    use super::*;
    use crate::{CostBreakdown, Pagination, RequestStatus, UsageEntry, UsageFilters, UsageSummary};
    use shared_kernel::RequestId;

    struct FakeUsageRecorder;

    #[async_trait]
    impl UsageRecorderPort for FakeUsageRecorder {
        async fn record(&self, _entry: UsageEntry) -> CortexResult<()> {
            Ok(())
        }

        async fn list(
            &self,
            _filters: UsageFilters,
            pagination: Pagination,
        ) -> CortexResult<Vec<UsageEntry>> {
            assert_eq!(pagination.limit, Pagination::DEFAULT_LIMIT);
            Ok(Vec::new())
        }

        async fn count(&self, _filters: UsageFilters) -> CortexResult<u64> {
            Ok(0)
        }

        async fn summary(&self, _filters: UsageFilters) -> CortexResult<UsageSummary> {
            Ok(UsageSummary::default())
        }

        async fn cost_breakdown(&self, _filters: UsageFilters) -> CortexResult<CostBreakdown> {
            Ok(CostBreakdown::default())
        }
    }

    struct FakeProviderRepository {
        runtime_id: ProviderId,
        connection_id: ConnectionId,
    }

    #[async_trait]
    impl ProviderRepositoryPort for FakeProviderRepository {
        async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
            Ok(Vec::new())
        }

        async fn find(
            &self,
            _id: &ConnectionId,
        ) -> Result<Option<ProviderConnection>, RepositoryError> {
            Ok(None)
        }

        async fn find_connection_id_by_runtime(
            &self,
            provider: &ProviderId,
        ) -> Result<Option<ConnectionId>, RepositoryError> {
            Ok((provider == &self.runtime_id).then_some(self.connection_id))
        }

        async fn create(&self, _conn: &ProviderConnection) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn update(
            &self,
            _conn: &ProviderConnection,
            _expected_updated_at: DateTime<Utc>,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn delete(&self, _id: &ConnectionId) -> Result<(), RepositoryError> {
            Ok(())
        }
    }

    #[test]
    fn usage_recorder_port_exposes_write_and_read_methods() {
        let port = FakeUsageRecorder;
        let entry = UsageEntry {
            request_id: RequestId::new(),
            provider: ProviderId::new("openai"),
            model: ModelId::new("gpt-4o"),
            status: RequestStatus::Success,
            requested_tier: None,
            api_key_id: None,
            connection_id: None,
            tokens_prompt: Some(10),
            tokens_completion: Some(20),
            tokens_cache_read: None,
            tokens_cache_creation: None,
            tokens_reasoning: None,
            ttft_ms: Some(100),
            latency_ms: 200,
            cost_usd: None,
            timestamp: Utc::now(),
        };

        futures::executor::block_on(async {
            port.record(entry).await.expect("record");
            let filters = UsageFilters::default();
            port.list(filters.clone(), Pagination::default())
                .await
                .expect("list");
            assert_eq!(port.count(filters.clone()).await.expect("count"), 0);
            port.summary(filters.clone()).await.expect("summary");
            port.cost_breakdown(filters).await.expect("cost breakdown");
        });
    }

    #[test]
    fn provider_repository_port_resolves_connection_id_by_runtime_provider() {
        let connection_id = ConnectionId::new();
        let repo = FakeProviderRepository {
            runtime_id: ProviderId::new("openai-primary"),
            connection_id,
        };

        let found = futures::executor::block_on(
            repo.find_connection_id_by_runtime(&ProviderId::new("openai-primary")),
        )
        .expect("lookup");
        let missing = futures::executor::block_on(
            repo.find_connection_id_by_runtime(&ProviderId::new("ollama-local")),
        )
        .expect("lookup");

        assert_eq!(found, Some(connection_id));
        assert_eq!(missing, None);
    }
}

// ---------------------------------------------------------------------------
// ModelCatalogPort — source of truth for "which models can an API key be
// restricted to?"
// ---------------------------------------------------------------------------
//
// Implementations live in `infrastructure/` crates. Today the catalog is
// static (hardcoded by provider kind); it may become dynamic in the future
// (per-provider fetch, real `/v1/models` aggregation, etc.). The wire format
// and the consuming UI must NOT depend on the implementation details.

/// One entry in the model catalog: a model id known to be supported by a
/// given provider kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCatalogEntry {
    pub model_id: String,
    pub provider_kind: ProviderKind,
}

#[async_trait]
pub trait ModelCatalogPort: Send + Sync {
    /// Returns the full list of model catalog entries known to the proxy.
    async fn list(&self) -> Vec<ModelCatalogEntry>;
}
