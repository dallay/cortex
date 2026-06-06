// provider-utils — shared helpers for provider adapters and transport layer
//
// Provides zero-deps utilities (only std) plus lightweight helpers that are
// duplicated across provider implementations and rate limiters.

pub mod role;
pub mod error;
pub mod token_bucket;

// Re-export for convenience
pub use role::role_to_string;
pub use error::{map_http_error, sanitize_error_body};
pub use token_bucket::{TokenBucket, RateLimitExceeded};