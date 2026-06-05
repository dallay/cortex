// shared-kernel — common types shared across all cortex crates
//
// Design principles:
// - No external service dependencies (no DB, no HTTP, no async runtime)
// - All types are Send + Sync where possible
// - Newtypes for IDs to prevent mixing up ProviderId vs ModelId at the type level

pub mod error;
pub mod id;
pub mod rate_limit;
pub mod time_;

pub use error::{CortexError, CortexResult, RestrictionViolation};
pub use id::{ComboId, ConnectionId, ModelId, ProviderId, RequestId};
pub use rate_limit::{ProviderRateLimit, RateLimitRule, RateLimitScope, RateLimitStatus};
pub use time_::Instant;

// Re-export chrono for convenience in downstream crates
pub use chrono::{DateTime, Utc};

// ---------------------------------------------------------------------------
// Blanket re-exports — most used items live here
// ---------------------------------------------------------------------------

/// A cache key derived from a request.
/// Includes both request ID and a content signature (SHA-256 hash of semantic fields).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub request_id: RequestId,
    /// SHA-256 signature of semantic fields (model, messages, parameters)
    pub signature: String,
}

impl From<&RequestId> for CacheKey {
    fn from(request_id: &RequestId) -> Self {
        Self {
            request_id: request_id.clone(),
            signature: String::new(),
        }
    }
}

impl CacheKey {
    /// Test helper for constructing cache keys with explicit signature
    pub fn test_key(request_id: RequestId, signature: String) -> Self {
        Self {
            request_id,
            signature,
        }
    }
}

impl std::fmt::Display for CacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sig_preview = if self.signature.len() >= 8 {
            &self.signature[..8]
        } else {
            &self.signature
        };
        write!(f, "{}:{}", self.request_id, sig_preview)
    }
}
