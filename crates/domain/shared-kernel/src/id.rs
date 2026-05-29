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
}
