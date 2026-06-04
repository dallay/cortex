// rook-core — domain model and ports for the rook proxy
//
// Ports (traits) live here. Implementations live in `infrastructure/` and `application/`.

pub mod api_key;
pub mod decrypted_credentials;
pub mod model;
pub mod ports;
pub mod provider_connection;

pub use api_key::*;
pub use decrypted_credentials::*;
pub use model::*;
pub use ports::*;
pub use provider_connection::*;

// Re-export shared_kernel types that are used across the domain
pub use shared_kernel::{
    CacheKey, ComboId, ConnectionId, CortexError, CortexResult, Instant, ModelId, ProviderId,
    RequestId,
};

#[cfg(test)]
mod api_key_tests {
    use std::str::FromStr;

    use super::{ApiKeyScope, ApiKeyTier};

    #[test]
    fn api_key_tier_parses_supported_values() {
        assert_eq!(
            ApiKeyTier::from_str("free").expect("free"),
            ApiKeyTier::Free
        );
        assert_eq!(ApiKeyTier::from_str("pro").expect("pro"), ApiKeyTier::Pro);
        assert_eq!(
            ApiKeyTier::from_str("enterprise").expect("enterprise"),
            ApiKeyTier::Enterprise
        );
        assert!(ApiKeyTier::from_str("unknown").is_err());
    }

    #[test]
    fn api_key_scope_trims_and_rejects_empty_values() {
        // Only canonical scope values are accepted by parse().
        let scope = ApiKeyScope::parse(" chat:read ").expect("scope");
        assert_eq!(scope.as_str(), "chat:read");
        assert!(ApiKeyScope::parse(" ").is_err());
        assert!(ApiKeyScope::parse("read").is_err()); // non-canonical → UnknownScope
    }
}
