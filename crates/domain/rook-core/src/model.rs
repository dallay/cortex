// Domain model — completely agnostic of any provider implementation.
//
// These types are the canonical internal representation.
// Translation to/from provider-specific wire formats happens in `infrastructure/transport-axum`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared_kernel::{CacheKey, ComboId, ConnectionId, ModelId, ProviderId, RequestId};
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
    /// Uses SHA-256 hash of semantic fields: model, messages, max_tokens, temperature, tools, tool_choice.
    /// Excludes ephemeral fields: id, stream, metadata, restrictions.
    pub fn cache_key(&self) -> CacheKey {
        use sha2::{Digest, Sha256};
        use std::collections::BTreeMap;

        // Build canonical representation with sorted keys
        let mut canonical = BTreeMap::new();
        canonical.insert("model", serde_json::to_value(&self.model).unwrap());
        canonical.insert("messages", serde_json::to_value(&self.messages).unwrap());
        canonical.insert("max_tokens", serde_json::to_value(self.max_tokens).unwrap());
        canonical.insert(
            "temperature",
            serde_json::to_value(self.temperature).unwrap(),
        );
        canonical.insert("tools", serde_json::to_value(&self.tools).unwrap());
        canonical.insert(
            "tool_choice",
            serde_json::to_value(&self.tool_choice).unwrap(),
        );

        // Serialize to JSON bytes
        let json_bytes = serde_json::to_vec(&canonical).unwrap();

        // Compute SHA-256
        let mut hasher = Sha256::new();
        hasher.update(&json_bytes);
        let digest = hasher.finalize();

        // Hex-encode to 64-char string
        let signature = hex::encode(digest);

        CacheKey {
            request_id: self.id.clone(),
            signature,
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
    /// Combo identifier for multi-step fallback execution, if present.
    pub combo_id: Option<ComboId>,
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
    /// Combo identifier if this request was part of a combo execution
    pub combo_id: Option<ComboId>,
    /// Step index within the combo (0-based) if applicable
    pub combo_step_index: Option<usize>,
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

// ============================================================================
// Circuit Breaker State — read-only snapshot for observability
// ============================================================================

/// Read-only snapshot of circuit breaker state for a provider.
/// Serialization-safe (no Instant, all timestamps are `DateTime<Utc>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CircuitStateSnapshot {
    /// Number of consecutive failures recorded
    pub failures: u32,
    /// Whether the circuit is currently open (provider unavailable)
    pub is_open: bool,
    /// Last failure timestamp (UTC), or None if no failures
    pub last_failure: Option<DateTime<Utc>>,
    /// Cooldown expiry (UTC), or None if circuit is closed
    pub cooldown_until: Option<DateTime<Utc>>,
    /// Rate limit reset timestamp (Unix epoch seconds), or None if not rate-limited
    pub rate_limit_reset: Option<u64>,
}

// ============================================================================
// Combo — multi-step fallback chains
// ============================================================================

/// Strategy for executing combo steps (MVP: priority-based only)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComboStrategy {
    /// Execute steps in priority order (lower priority = attempted first)
    Priority,
}

/// A single step in a combo fallback chain
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComboStep {
    /// The provider to use for this step
    pub provider_id: ProviderId,
    /// The model to request from the provider
    pub model: ModelId,
    /// Optional connection ID for connection-specific routing
    pub connection_id: Option<ConnectionId>,
    /// Priority order (1-255, lower = attempted first)
    pub priority: u8,
}

/// Validation errors for combo creation/update
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComboValidationError {
    /// Combo name is empty
    EmptyName,
    /// Combo name exceeds 100 characters
    NameTooLong,
    /// Combo has no steps
    EmptySteps,
    /// Combo has more than 10 steps
    TooManySteps,
    /// Two or more steps have the same priority
    DuplicatePriority { priority: u8 },
    /// Priority is outside valid range (1-255)
    InvalidPriority { priority: u8 },
}

impl std::fmt::Display for ComboValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => write!(f, "combo name cannot be empty"),
            Self::NameTooLong => write!(f, "combo name cannot exceed 100 characters"),
            Self::EmptySteps => write!(f, "combo must have at least one step"),
            Self::TooManySteps => write!(f, "combo cannot have more than 10 steps"),
            Self::DuplicatePriority { priority } => {
                write!(f, "duplicate priority value: {priority}")
            }
            Self::InvalidPriority { priority } => {
                write!(f, "priority must be between 1 and 255, got: {priority}")
            }
        }
    }
}

impl std::error::Error for ComboValidationError {}

// ============================================================================
// ModelAlias — model alias mapping for stable model names
// ============================================================================

/// Model alias mapping — resolves friendly alias names to canonical model IDs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAlias {
    /// The alias name (e.g., "gpt-4o-latest")
    pub alias: ModelId,
    /// The canonical model ID (e.g., "gpt-4o-2024-05-13")
    pub canonical: ModelId,
    /// Optional provider scope (null = global)
    pub provider_id: Option<ProviderId>,
    /// Creation timestamp (ISO 8601)
    pub created_at: String,
}

/// A multi-step fallback chain aggregate
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Combo {
    /// Unique combo identifier
    pub id: ComboId,
    /// Human-readable name (unique, 1-100 chars)
    pub name: String,
    /// Execution strategy
    pub strategy: ComboStrategy,
    /// Ordered steps to try
    pub steps: Vec<ComboStep>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl Combo {
    /// Create a new combo with the given parameters.
    /// Note: Call `validate()` to ensure the combo is valid before persisting.
    pub fn new(name: String, strategy: ComboStrategy, steps: Vec<ComboStep>) -> Self {
        let now = Utc::now();
        Self {
            id: ComboId::new(),
            name,
            strategy,
            steps,
            created_at: now,
            updated_at: now,
        }
    }

    /// Validate the combo according to business rules.
    pub fn validate(&self) -> Result<(), ComboValidationError> {
        // Name: 1-100 characters, non-empty
        if self.name.is_empty() {
            return Err(ComboValidationError::EmptyName);
        }
        if self.name.len() > 100 {
            return Err(ComboValidationError::NameTooLong);
        }

        // Steps: 1-10 items
        if self.steps.is_empty() {
            return Err(ComboValidationError::EmptySteps);
        }
        if self.steps.len() > 10 {
            return Err(ComboValidationError::TooManySteps);
        }

        // Priority: unique within combo, range 1-255
        let mut seen_priorities = std::collections::HashSet::new();
        for step in &self.steps {
            if step.priority == 0 {
                return Err(ComboValidationError::InvalidPriority { priority: 0 });
            }
            // Note: u8 max is 255, so no need to check upper bound
            if !seen_priorities.insert(step.priority) {
                return Err(ComboValidationError::DuplicatePriority {
                    priority: step.priority,
                });
            }
        }

        Ok(())
    }

    /// Returns steps sorted by priority ascending (lower priority = attempted first).
    pub fn sorted_steps(&self) -> Vec<&ComboStep> {
        let mut sorted: Vec<&ComboStep> = self.steps.iter().collect();
        sorted.sort_by_key(|s| s.priority);
        sorted
    }
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
            combo_id: None,
            combo_step_index: None,
        }
    }

    pub fn success_with_combo(
        request_id: &RequestId,
        provider: &ProviderId,
        model: &ModelId,
        usage: Option<TokenUsage>,
        latency_ms: u64,
        combo_id: Option<ComboId>,
        combo_step_index: Option<usize>,
    ) -> Self {
        Self {
            request_id: request_id.clone(),
            provider: provider.clone(),
            model: model.clone(),
            status: RequestStatus::Success,
            usage,
            latency_ms,
            timestamp: Utc::now(),
            combo_id,
            combo_step_index,
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
            combo_id: None,
            combo_step_index: None,
        }
    }

    pub fn failure_with_combo(
        request_id: &RequestId,
        provider: &ProviderId,
        model: &ModelId,
        status: RequestStatus,
        latency_ms: u64,
        combo_id: Option<ComboId>,
        combo_step_index: Option<usize>,
    ) -> Self {
        Self {
            request_id: request_id.clone(),
            provider: provider.clone(),
            model: model.clone(),
            status,
            usage: None,
            latency_ms,
            timestamp: Utc::now(),
            combo_id,
            combo_step_index,
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

// ============================================================================
// Cache Statistics
// ============================================================================

/// Statistics snapshot for cache operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: u64,
    pub max_entries: u64,
}

impl CacheStats {
    /// Calculate hit rate as hits / (hits + misses)
    /// Returns 0.0 if no requests have been made
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Calculate cache utilization as entries / max_entries
    /// Returns None if max_entries is 0 (unlimited)
    pub fn utilization(&self) -> Option<f64> {
        if self.max_entries == 0 {
            None
        } else {
            Some(self.entries as f64 / self.max_entries as f64)
        }
    }
}

#[cfg(test)]
mod combo_tests {
    use super::*;

    #[test]
    fn combo_new_generates_id_and_timestamps() {
        let combo = Combo::new(
            "test-combo".to_string(),
            ComboStrategy::Priority,
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );
        assert_eq!(combo.name, "test-combo");
        assert_eq!(combo.strategy, ComboStrategy::Priority);
        assert_eq!(combo.steps.len(), 1);
        assert!(combo.created_at <= Utc::now());
        assert!(combo.updated_at <= Utc::now());
    }

    #[test]
    fn combo_validate_rejects_empty_name() {
        let combo = Combo::new(
            String::new(),
            ComboStrategy::Priority,
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );
        assert_eq!(combo.validate(), Err(ComboValidationError::EmptyName));
    }

    #[test]
    fn combo_validate_rejects_name_too_long() {
        let long_name = "a".repeat(101);
        let combo = Combo::new(
            long_name,
            ComboStrategy::Priority,
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );
        assert_eq!(combo.validate(), Err(ComboValidationError::NameTooLong));
    }

    #[test]
    fn combo_validate_accepts_name_at_max_length() {
        let max_name = "a".repeat(100);
        let combo = Combo::new(
            max_name,
            ComboStrategy::Priority,
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );
        assert!(combo.validate().is_ok());
    }

    #[test]
    fn combo_validate_rejects_empty_steps() {
        let combo = Combo::new("test".to_string(), ComboStrategy::Priority, vec![]);
        assert_eq!(combo.validate(), Err(ComboValidationError::EmptySteps));
    }

    #[test]
    fn combo_validate_rejects_too_many_steps() {
        let steps: Vec<ComboStep> = (1..=11)
            .map(|i| ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: i as u8,
            })
            .collect();
        let combo = Combo::new("test".to_string(), ComboStrategy::Priority, steps);
        assert_eq!(combo.validate(), Err(ComboValidationError::TooManySteps));
    }

    #[test]
    fn combo_validate_accepts_10_steps() {
        let steps: Vec<ComboStep> = (1..=10)
            .map(|i| ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: i as u8,
            })
            .collect();
        let combo = Combo::new("test".to_string(), ComboStrategy::Priority, steps);
        assert!(combo.validate().is_ok());
    }

    #[test]
    fn combo_validate_rejects_duplicate_priority() {
        let steps = vec![
            ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            },
            ComboStep {
                provider_id: ProviderId::new("anthropic"),
                model: ModelId::new("claude-opus-4"),
                connection_id: None,
                priority: 1,
            },
        ];
        let combo = Combo::new("test".to_string(), ComboStrategy::Priority, steps);
        assert_eq!(
            combo.validate(),
            Err(ComboValidationError::DuplicatePriority { priority: 1 })
        );
    }

    #[test]
    fn combo_validate_rejects_priority_zero() {
        let combo = Combo::new(
            "test".to_string(),
            ComboStrategy::Priority,
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 0,
            }],
        );
        assert_eq!(
            combo.validate(),
            Err(ComboValidationError::InvalidPriority { priority: 0 })
        );
    }

    #[test]
    fn combo_validate_accepts_priority_255() {
        let combo = Combo::new(
            "test".to_string(),
            ComboStrategy::Priority,
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 255,
            }],
        );
        assert!(combo.validate().is_ok());
    }

    #[test]
    fn combo_sorted_steps_returns_ascending_priority() {
        let combo = Combo::new(
            "test".to_string(),
            ComboStrategy::Priority,
            vec![
                ComboStep {
                    provider_id: ProviderId::new("ollama"),
                    model: ModelId::new("llama3.1:70b"),
                    connection_id: None,
                    priority: 3,
                },
                ComboStep {
                    provider_id: ProviderId::new("openai"),
                    model: ModelId::new("gpt-4o"),
                    connection_id: None,
                    priority: 1,
                },
                ComboStep {
                    provider_id: ProviderId::new("anthropic"),
                    model: ModelId::new("claude-opus-4"),
                    connection_id: None,
                    priority: 2,
                },
            ],
        );

        let sorted = combo.sorted_steps();
        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].priority, 1);
        assert_eq!(sorted[0].provider_id, ProviderId::new("openai"));
        assert_eq!(sorted[1].priority, 2);
        assert_eq!(sorted[1].provider_id, ProviderId::new("anthropic"));
        assert_eq!(sorted[2].priority, 3);
        assert_eq!(sorted[2].provider_id, ProviderId::new("ollama"));
    }

    #[test]
    fn combo_validation_error_display() {
        assert_eq!(
            ComboValidationError::EmptyName.to_string(),
            "combo name cannot be empty"
        );
        assert_eq!(
            ComboValidationError::NameTooLong.to_string(),
            "combo name cannot exceed 100 characters"
        );
        assert_eq!(
            ComboValidationError::EmptySteps.to_string(),
            "combo must have at least one step"
        );
        assert_eq!(
            ComboValidationError::TooManySteps.to_string(),
            "combo cannot have more than 10 steps"
        );
        assert_eq!(
            ComboValidationError::DuplicatePriority { priority: 5 }.to_string(),
            "duplicate priority value: 5"
        );
        assert_eq!(
            ComboValidationError::InvalidPriority { priority: 0 }.to_string(),
            "priority must be between 1 and 255, got: 0"
        );
    }

    #[test]
    fn combo_strategy_serializes_as_lowercase() {
        let json = serde_json::to_string(&ComboStrategy::Priority).expect("serialize");
        assert_eq!(json, r#""priority""#);
    }

    #[test]
    fn combo_strategy_deserializes_from_lowercase() {
        let strategy: ComboStrategy = serde_json::from_str(r#""priority""#).expect("deserialize");
        assert_eq!(strategy, ComboStrategy::Priority);
    }
}

#[cfg(test)]
mod cache_key_tests {
    use super::*;

    fn make_request() -> CompletionRequest {
        CompletionRequest {
            id: RequestId::new(),
            model: ModelId::new("gpt-4o"),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hello".to_string()),
            }],
            stream: false,
            max_tokens: Some(100),
            temperature: Some(0.7),
            tools: None,
            tool_choice: None,
            metadata: RequestMetadata {
                origin: "test".to_string(),
                cacheable: true,
                priority: 1,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: ApiKeyRestrictions::default(),
        }
    }

    #[test]
    fn cache_key_determinism() {
        let req = make_request();
        let signatures: Vec<String> = (0..100).map(|_| req.cache_key().signature).collect();

        // All signatures should be identical
        let first = &signatures[0];
        assert!(signatures.iter().all(|s| s == first));
    }

    #[test]
    fn cache_key_excludes_ephemeral_fields() {
        let mut req1 = make_request();
        let mut req2 = make_request();

        // Changing id, stream, metadata should NOT change signature
        req1.id = RequestId::new();
        req2.id = RequestId::new();
        req1.stream = false;
        req2.stream = true;
        req1.metadata.origin = "origin1".to_string();
        req2.metadata.origin = "origin2".to_string();

        assert_eq!(req1.cache_key().signature, req2.cache_key().signature);
    }

    #[test]
    fn cache_key_includes_semantic_fields() {
        let req1 = make_request();
        let mut req2 = make_request();

        // Changing model should change signature
        req2.model = ModelId::new("claude-opus-4");
        assert_ne!(req1.cache_key().signature, req2.cache_key().signature);

        // Changing messages should change signature
        let mut req3 = make_request();
        req3.messages = vec![Message {
            role: Role::User,
            content: MessageContent::Text("goodbye".to_string()),
        }];
        assert_ne!(req1.cache_key().signature, req3.cache_key().signature);
    }

    #[test]
    fn cache_key_signature_is_64_hex_chars() {
        let req = make_request();
        let key = req.cache_key();

        assert_eq!(key.signature.len(), 64);
        assert!(key.signature.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn cache_key_display_shows_first_8_chars() {
        let key = CacheKey::test_key(
            RequestId::new(),
            "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".to_string(),
        );
        let display = format!("{}", key);
        assert!(display.contains("abcdef12"));
    }
}

#[cfg(test)]
mod cache_stats_tests {
    use super::*;

    #[test]
    fn hit_rate_with_zero_requests() {
        let stats = CacheStats {
            hits: 0,
            misses: 0,
            evictions: 0,
            entries: 0,
            max_entries: 100,
        };
        assert_eq!(stats.hit_rate(), 0.0);
    }

    #[test]
    fn hit_rate_with_only_hits() {
        let stats = CacheStats {
            hits: 10,
            misses: 0,
            evictions: 0,
            entries: 5,
            max_entries: 100,
        };
        assert_eq!(stats.hit_rate(), 1.0);
    }

    #[test]
    fn hit_rate_with_mixed_hits_and_misses() {
        let stats = CacheStats {
            hits: 7,
            misses: 3,
            evictions: 0,
            entries: 5,
            max_entries: 100,
        };
        assert_eq!(stats.hit_rate(), 0.7);
    }

    #[test]
    fn utilization_with_no_limit() {
        let stats = CacheStats {
            hits: 10,
            misses: 5,
            evictions: 0,
            entries: 50,
            max_entries: 0,
        };
        assert_eq!(stats.utilization(), None);
    }

    #[test]
    fn utilization_with_limit() {
        let stats = CacheStats {
            hits: 10,
            misses: 5,
            evictions: 0,
            entries: 50,
            max_entries: 100,
        };
        assert_eq!(stats.utilization(), Some(0.5));
    }

    #[test]
    fn utilization_at_capacity() {
        let stats = CacheStats {
            hits: 10,
            misses: 5,
            evictions: 2,
            entries: 100,
            max_entries: 100,
        };
        assert_eq!(stats.utilization(), Some(1.0));
    }
}
