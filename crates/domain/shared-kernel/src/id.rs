// ID newtypes — never use raw String for identifiers

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// Provider identifier, e.g. "openai-primary", "anthropic-fs"
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderId(pub SmolStr);

impl ProviderId {
    pub fn new(s: impl Into<SmolStr>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ProviderId {
    fn from(s: String) -> Self {
        Self(s.into())
    }
}

impl From<&str> for ProviderId {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Model identifier, e.g. "gpt-4o", "claude-opus-4-5"
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelId(pub SmolStr);

impl ModelId {
    pub fn new(s: impl Into<SmolStr>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ModelId {
    fn from(s: String) -> Self {
        Self(s.into())
    }
}

impl From<&str> for ModelId {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for ModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique request identifier (UUID v4)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(pub uuid::Uuid);

impl RequestId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Connection identifier for stored provider connections (UUID v4).
/// Distinct from ProviderId, which identifies runtime providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConnectionId(pub uuid::Uuid);

impl ConnectionId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub fn parse_str(s: &str) -> Result<Self, uuid::Error> {
        uuid::Uuid::parse_str(s).map(Self)
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_id_new_is_unique() {
        assert_ne!(ConnectionId::new(), ConnectionId::new());
    }

    #[test]
    fn connection_id_default_generates_uuid_v4() {
        let id = ConnectionId::default();
        assert_eq!(id.0.get_version_num(), 4);
    }

    #[test]
    fn connection_id_display_is_uuid_string() {
        let rendered = ConnectionId::new().to_string();
        assert_eq!(rendered.len(), 36);
        assert!(rendered.contains('-'));
    }

    // =============================================================================
    // RequestId tests
    // =============================================================================

    #[test]
    fn request_id_new_generates_uuid_v4() {
        let id = RequestId::new();
        assert_eq!(id.0.get_version_num(), 4);
    }

    #[test]
    fn request_id_default_generates_uuid_v4() {
        let id = RequestId::default();
        assert_eq!(id.0.get_version_num(), 4);
    }

    #[test]
    fn request_id_display_is_uuid_string() {
        let rendered = RequestId::new().to_string();
        assert_eq!(rendered.len(), 36);
        assert!(rendered.contains('-'));
    }

    #[test]
    fn request_id_new_is_unique() {
        assert_ne!(RequestId::new(), RequestId::new());
    }

    // =============================================================================
    // ConnectionId parse_str tests
    // =============================================================================

    #[test]
    fn connection_id_parse_str_valid_uuid() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let result = ConnectionId::parse_str(uuid_str);
        assert!(result.is_ok());
        let id = result.unwrap();
        assert_eq!(id.0.get_version_num(), 4);
    }

    #[test]
    fn connection_id_parse_str_different_version() {
        // UUID v1 (time-based) should still parse, just not be v4
        let uuid_str = "550e8400-e29b-11d4-a716-446655440000";
        let result = ConnectionId::parse_str(uuid_str);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().0.get_version_num(), 1);
    }

    #[test]
    fn connection_id_parse_str_invalid_format_error() {
        let invalid = "not-a-valid-uuid";
        let result = ConnectionId::parse_str(invalid);
        assert!(result.is_err());
    }

    #[test]
    fn connection_id_parse_str_empty_string_error() {
        let result = ConnectionId::parse_str("");
        assert!(result.is_err());
    }

    #[test]
    fn connection_id_parse_str_malformed_uuid_error() {
        // Missing hyphen in wrong position
        let result = ConnectionId::parse_str("550e8400-e29b-41d4-a716446655440000");
        assert!(result.is_err());
    }

    #[test]
    fn connection_id_display_matches_parsed_uuid() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let id = ConnectionId::parse_str(uuid_str).unwrap();
        assert_eq!(id.to_string(), uuid_str);
    }

    // =============================================================================
    // ProviderId tests
    // =============================================================================

    #[test]
    fn provider_id_new_from_string() {
        let id = ProviderId::new(String::from("openai-primary"));
        assert_eq!(id.as_str(), "openai-primary");
    }

    #[test]
    fn provider_id_new_from_static_str() {
        let id = ProviderId::new("anthropic-fs");
        assert_eq!(id.as_str(), "anthropic-fs");
    }

    #[test]
    fn provider_id_display() {
        let id = ProviderId::new("test-provider");
        assert_eq!(id.to_string(), "test-provider");
    }

    #[test]
    fn provider_id_from_string_trait() {
        let id: ProviderId = String::from("from-string").into();
        assert_eq!(id.as_str(), "from-string");
    }

    #[test]
    fn provider_id_from_str_trait() {
        let id: ProviderId = "from-str".into();
        assert_eq!(id.as_str(), "from-str");
    }

    #[test]
    fn provider_id_clone_equals() {
        let id1 = ProviderId::new("original");
        let id2 = id1.clone();
        assert_eq!(id1, id2);
    }

    #[test]
    fn provider_id_hash() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let id = ProviderId::new("hashed");
        let mut h1 = DefaultHasher::new();
        let mut h2 = DefaultHasher::new();
        id.hash(&mut h1);
        id.hash(&mut h2);
        assert_eq!(h1.finish(), h2.finish());
    }

    // =============================================================================
    // ModelId tests
    // =============================================================================

    #[test]
    fn model_id_new_from_string() {
        let id = ModelId::new(String::from("gpt-4o"));
        assert_eq!(id.as_str(), "gpt-4o");
    }

    #[test]
    fn model_id_new_from_static_str() {
        let id = ModelId::new("claude-opus-4-5");
        assert_eq!(id.as_str(), "claude-opus-4-5");
    }

    #[test]
    fn model_id_display() {
        let id = ModelId::new("test-model");
        assert_eq!(id.to_string(), "test-model");
    }

    #[test]
    fn model_id_from_string_trait() {
        let id: ModelId = String::from("from-string").into();
        assert_eq!(id.as_str(), "from-string");
    }

    #[test]
    fn model_id_from_str_trait() {
        let id: ModelId = "from-str".into();
        assert_eq!(id.as_str(), "from-str");
    }

    #[test]
    fn model_id_clone_equals() {
        let id1 = ModelId::new("original");
        let id2 = id1.clone();
        assert_eq!(id1, id2);
    }

    #[test]
    fn model_id_hash() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let id = ModelId::new("hashed");
        let mut h1 = DefaultHasher::new();
        let mut h2 = DefaultHasher::new();
        id.hash(&mut h1);
        id.hash(&mut h2);
        assert_eq!(h1.finish(), h2.finish());
    }

    // =============================================================================
    // Integration-style tests (cross-type behavior)
    // =============================================================================

    #[test]
    fn request_id_debug_format() {
        let id = RequestId::new();
        let debug = format!("{:?}", id);
        // Should contain the UUID string representation
        assert!(debug.contains("RequestId"));
    }

    #[test]
    fn connection_id_debug_format() {
        let id = ConnectionId::new();
        let debug = format!("{:?}", id);
        assert!(debug.contains("ConnectionId"));
    }

    #[test]
    fn provider_id_debug_format() {
        let id = ProviderId::new("debug-test");
        let debug = format!("{:?}", id);
        assert!(debug.contains("ProviderId"));
    }

    #[test]
    fn model_id_debug_format() {
        let id = ModelId::new("debug-test");
        let debug = format!("{:?}", id);
        assert!(debug.contains("ModelId"));
    }
}
