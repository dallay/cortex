// Domain model — completely agnostic of any provider implementation.
//
// These types are the canonical internal representation.
// Translation to/from provider-specific wire formats happens in `infrastructure/transport-axum`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared_kernel::{CacheKey, ModelId, ProviderId, RequestId};

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
