// Domain model — completely agnostic of any provider implementation.
//
// These types are the canonical internal representation.
// Translation to/from provider-specific wire formats happens in `infrastructure/transport-axum`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared_kernel::{CacheKey, ModelId, ProviderId, RequestId};
use uuid::Uuid;

// ============================================================================
// User — admin user for MANAGEMENT routes
// ============================================================================

/// User identifier (UUID v4)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub Uuid);

impl UserId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse_str(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A user record — currently only admin exists
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub username: String,
    /// Argon2id hash, or None if password not set yet
    pub password_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a new user
#[derive(Debug, Clone)]
pub struct NewUser {
    pub username: String,
    pub password_hash: Option<String>,
}

/// Password hash wrapped as an opaque string (Argon2id format)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordHash(pub String);

impl PasswordHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for PasswordHash {
    fn from(s: String) -> Self {
        Self(s)
    }
}

// ============================================================================
// Session — session token for MANAGEMENT route auth
// ============================================================================

/// Session identifier (UUID v4)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse_str(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A session record — ties a user to a token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    /// SHA-256 hash of the raw token bytes
    pub token_hash: String,
    pub user_id: UserId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
}

/// Input for creating a new session
#[derive(Debug, Clone)]
pub struct NewSession {
    pub user_id: UserId,
    /// 32 raw random bytes (not hashed)
    pub token: Vec<u8>,
}

/// ---------------------------------------------------------------------------
/// Request / Response
/// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub id: RequestId,
    pub model: ModelId,
    pub messages: Vec<Message>,
    pub stream: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub metadata: RequestMetadata,
}

impl CompletionRequest {
    /// Derives the cache key from this request.
    /// Currently just the request ID; extend to include model + message hash
    /// for semantic (content-aware) caching.
    pub fn cache_key(&self) -> CacheKey {
        CacheKey {
            request_id: self.id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMetadata {
    /// Origin of the request (e.g. "corvus", "cerebro", "direct")
    pub origin: String,
    /// Whether the response may be cached
    pub cacheable: bool,
    /// Priority tier — lower = higher priority
    pub priority: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Developer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub id: RequestId,
    pub provider: ProviderId,
    pub model: ModelId,
    pub content: String,
    pub usage: TokenUsage,
    pub latency_ms: u64,
}

/// ---------------------------------------------------------------------------
/// Streaming
/// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub delta: String,
    pub finish_reason: Option<FinishReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
}

/// ---------------------------------------------------------------------------
/// Token usage & cost
/// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    /// Estimated cost in USD — calculated by the provider adapter
    pub estimated_cost_usd: Option<f64>,
}

/// ---------------------------------------------------------------------------
/// Health
/// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy {
        provider: ProviderId,
        latency_ms: u64,
    },
    Unhealthy {
        provider: ProviderId,
        latency_ms: Option<u64>,
        error: String,
    },
    Unknown {
        provider: ProviderId,
        reason: String,
    },
}

impl HealthStatus {
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy { .. })
    }

    pub fn provider_id(&self) -> &ProviderId {
        match self {
            Self::Healthy { provider, .. }
            | Self::Unhealthy { provider, .. }
            | Self::Unknown { provider, .. } => provider,
        }
    }

    pub fn latency_ms(&self) -> Option<u64> {
        match self {
            Self::Healthy { latency_ms, .. } => Some(*latency_ms),
            Self::Unhealthy { latency_ms, .. } => *latency_ms,
            Self::Unknown { .. } => None,
        }
    }

    pub fn last_error(&self) -> Option<&str> {
        match self {
            Self::Unhealthy { error, .. } => Some(error),
            Self::Unknown { reason, .. } => Some(reason),
            Self::Healthy { .. } => None,
        }
    }
}

/// ---------------------------------------------------------------------------
/// Audit
/// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Success,
    Failure,
    RateLimited,
    Timeout,
}

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub request_id: RequestId,
    pub provider: ProviderId,
    pub model: ModelId,
    pub status: RequestStatus,
    pub usage: Option<TokenUsage>,
    pub latency_ms: u64,
    pub timestamp: DateTime<Utc>,
}

impl AuditEntry {
    pub fn success(
        request_id: &RequestId,
        provider: &ProviderId,
        model: &ModelId,
        usage: Option<TokenUsage>,
        latency_ms: u64,
    ) -> Self {
        Self {
            request_id: request_id.clone(),
            provider: provider.clone(),
            model: model.clone(),
            status: RequestStatus::Success,
            usage,
            latency_ms,
            timestamp: Utc::now(),
        }
    }

    pub fn failure(
        request_id: &RequestId,
        provider: &ProviderId,
        model: &ModelId,
        status: RequestStatus,
        latency_ms: u64,
    ) -> Self {
        Self {
            request_id: request_id.clone(),
            provider: provider.clone(),
            model: model.clone(),
            status,
            usage: None,
            latency_ms,
            timestamp: Utc::now(),
        }
    }
}
