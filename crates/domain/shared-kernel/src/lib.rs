// shared-kernel — common types shared across all cortex crates
//
// Design principles:
// - No external service dependencies (no DB, no HTTP, no async runtime)
// - All types are Send + Sync where possible
// - Newtypes for IDs to prevent mixing up ProviderId vs ModelId at the type level

pub mod error;
pub mod id;
pub mod time_;

pub use error::{CortexError, CortexResult, RestrictionViolation};
pub use id::{ConnectionId, ModelId, ProviderId, RequestId};
pub use time_::Instant;

// Re-export chrono for convenience in downstream crates
pub use chrono::{DateTime, Utc};

// ---------------------------------------------------------------------------
// Blanket re-exports — most used items live here
// ---------------------------------------------------------------------------

/// A cache key derived from a request — currently just the request ID.
/// TODO: extend to include model + message hash for semantic caching.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub request_id: RequestId,
}

impl From<&RequestId> for CacheKey {
    fn from(request_id: &RequestId) -> Self {
        Self {
            request_id: request_id.clone(),
        }
    }
}
