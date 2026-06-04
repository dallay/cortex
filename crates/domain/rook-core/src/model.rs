// Domain model — completely agnostic of any provider implementation.
//
// These types are the canonical internal representation.
// Translation to/from provider-specific wire formats happens in `infrastructure/transport-axum`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared_kernel::{CacheKey, ConnectionId, ModelId, ProviderId, RequestId};
use uuid::Uuid;

use crate::ApiKeyId;

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
/// API key restriction filters. Empty vecs mean "unrestricted" (all allowed).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiKeyRestrictions {
    /// If non-empty, only these model IDs are allowed.
    pub allowed_models: Vec<ModelId>,
    /// If non-empty, only these provider IDs are allowed.
    pub allowed_providers: Vec<ProviderId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub id: RequestId,
    pub model: ModelId,
    pub messages: Vec<Message>,
    pub stream: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub tools: Option<serde_json::Value>,
    pub tool_choice: Option<serde_json::Value>,
    pub metadata: RequestMetadata,
    /// API key restrictions — empty vecs mean unrestricted.
    #[serde(default)]
    pub restrictions: ApiKeyRestrictions,
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
    /// Authenticated client API key identifier, if the request used one.
    pub api_key_id: Option<ApiKeyId>,
    /// Requested service tier from the client request, if present.
    pub requested_tier: Option<String>,
}

/// The content of a message in the provider-agnostic domain model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: Vec<MessageContent>,
    },
}

impl MessageContent {
    /// Borrow text content when this is a text block.
    pub fn as_text(&self) -> &str {
        match self {
            Self::Text(s) => s.as_str(),
            Self::ToolUse { .. } | Self::ToolResult { .. } => "",
        }
    }

    /// Consume and return text content, or an empty string for non-text blocks.
    pub fn into_text(self) -> String {
        match self {
            Self::Text(s) => s,
            Self::ToolUse { .. } | Self::ToolResult { .. } => String::new(),
        }
    }
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

impl From<&str> for MessageContent {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}

impl std::fmt::Display for MessageContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_text())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
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
    pub content_blocks: Vec<MessageContent>,
    pub usage: TokenUsage,
    pub latency_ms: u64,
}

/// ---------------------------------------------------------------------------
/// Streaming
/// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub id: RequestId,
    pub model: ModelId,
    pub delta: String,
    pub finish_reason: Option<FinishReason>,
    /// Token usage is emitted on the final chunk only when the provider reports it.
    pub usage: Option<TokenUsage>,
}

/// The set of supported API wire formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApiFormat {
    OpenAI,
    Anthropic,
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
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEntry {
    pub request_id: RequestId,
    pub provider: ProviderId,
    pub model: ModelId,
    pub status: RequestStatus,
    pub requested_tier: Option<String>,
    pub api_key_id: Option<ApiKeyId>,
    pub connection_id: Option<ConnectionId>,
    pub tokens_prompt: Option<u64>,
    pub tokens_completion: Option<u64>,
    pub tokens_cache_read: Option<u64>,
    pub tokens_cache_creation: Option<u64>,
    pub tokens_reasoning: Option<u64>,
    pub ttft_ms: Option<u64>,
    pub latency_ms: u64,
    pub cost_usd: Option<f64>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageFilters {
    pub provider: Option<ProviderId>,
    pub model: Option<ModelId>,
    pub api_key_id: Option<ApiKeyId>,
    pub connection_id: Option<ConnectionId>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub status: Option<RequestStatus>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Pagination {
    pub offset: u64,
    pub limit: u64,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: Self::DEFAULT_LIMIT,
        }
    }
}

impl Pagination {
    pub const DEFAULT_LIMIT: u64 = 100;
    pub const MAX_LIMIT: u64 = 1000;

    pub fn clamped(self) -> Self {
        Self {
            offset: self.offset,
            limit: self.limit.min(Self::MAX_LIMIT),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSummary {
    pub total_requests: u64,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_reasoning_tokens: u64,
    pub avg_ttft_ms: Option<f64>,
    pub avg_latency_ms: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub total_cost_usd: f64,
    pub by_provider: HashMap<ProviderId, f64>,
    pub by_model: HashMap<ModelId, f64>,
    pub by_api_key: HashMap<ApiKeyId, f64>,
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

#[cfg(test)]
mod usage_domain_tests {
    use super::*;

    #[test]
    fn pagination_defaults_to_100_and_clamps_to_1000() {
        let default_pagination = Pagination::default();
        assert_eq!(default_pagination.offset, 0);
        assert_eq!(default_pagination.limit, 100);

        let clamped = Pagination {
            offset: 25,
            limit: 5000,
        }
        .clamped();
        assert_eq!(clamped.offset, 25);
        assert_eq!(clamped.limit, 1000);
    }

    #[test]
    fn usage_entry_serializes_nullable_dimensions_and_identifiers() {
        let entry = UsageEntry {
            request_id: RequestId::new(),
            provider: ProviderId::new("openai"),
            model: ModelId::new("gpt-4o"),
            status: RequestStatus::Success,
            requested_tier: Some("premium".to_string()),
            api_key_id: Some(ApiKeyId::new("key_abc123")),
            connection_id: Some(ConnectionId::new()),
            tokens_prompt: Some(10),
            tokens_completion: Some(20),
            tokens_cache_read: None,
            tokens_cache_creation: Some(5),
            tokens_reasoning: None,
            ttft_ms: Some(150),
            latency_ms: 250,
            cost_usd: Some(0.001),
            timestamp: Utc::now(),
        };

        let value = serde_json::to_value(&entry).expect("serialize usage entry");

        assert_eq!(value["provider"], "openai");
        assert_eq!(value["model"], "gpt-4o");
        assert_eq!(value["status"], "success");
        assert_eq!(value["api_key_id"], "key_abc123");
        assert!(value["tokens_cache_read"].is_null());
        assert_eq!(value["tokens_cache_creation"], 5);
        assert!(value["tokens_reasoning"].is_null());
    }
}

#[cfg(test)]
mod token_usage_tests {
    use super::*;

    #[test]
    fn token_usage_serializes_optional_cache_and_reasoning_dimensions() {
        let usage = TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: 30,
            cache_read_tokens: Some(4),
            cache_creation_tokens: None,
            reasoning_tokens: Some(6),
            estimated_cost_usd: Some(0.00042),
        };

        let value = serde_json::to_value(&usage).expect("serialize token usage");

        assert_eq!(value["cache_read_tokens"], 4);
        assert!(value["cache_creation_tokens"].is_null());
        assert_eq!(value["reasoning_tokens"], 6);
    }
}

#[cfg(test)]
mod message_content_tests {
    use super::*;

    #[test]
    fn as_text_returns_inner_string() {
        let content = MessageContent::Text("hello".to_string());
        assert_eq!(content.as_text(), "hello");
    }

    #[test]
    fn into_text_consumes_and_returns_string() {
        let content = MessageContent::Text("world".to_string());
        assert_eq!(content.into_text(), "world");
    }

    #[test]
    fn from_string_constructs_text_variant() {
        let content = MessageContent::from("test".to_string());
        assert_eq!(content, MessageContent::Text("test".to_string()));
    }

    #[test]
    fn serde_round_trips_message_as_plain_string() {
        // {"role":"user","content":"hi"} must round-trip to MessageContent::Text("hi")
        let json = r#"{"role":"user","content":"hi"}"#;
        let msg: Message = serde_json::from_str(json).expect("deserialize");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, MessageContent::Text("hi".to_string()));

        let serialized = serde_json::to_string(&msg).expect("serialize");
        assert!(serialized.contains(r#""content":"hi""#));
    }
}
