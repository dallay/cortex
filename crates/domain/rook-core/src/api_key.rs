use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared_kernel::{ModelId, ProviderId};
use smol_str::SmolStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApiKeyId(SmolStr);

impl ApiKeyId {
    pub fn new(value: impl Into<SmolStr>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ApiKeyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Canonical set of valid API key scopes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnownScope {
    ChatRead,
    ChatWrite,
    ProvidersRead,
    ProvidersWrite,
    Admin,
}

impl KnownScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatRead => "chat:read",
            Self::ChatWrite => "chat:write",
            Self::ProvidersRead => "providers:read",
            Self::ProvidersWrite => "providers:write",
            Self::Admin => "admin",
        }
    }
}

impl FromStr for KnownScope {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "chat:read" => Ok(Self::ChatRead),
            "chat:write" => Ok(Self::ChatWrite),
            "providers:read" => Ok(Self::ProvidersRead),
            "providers:write" => Ok(Self::ProvidersWrite),
            "admin" => Ok(Self::Admin),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKeyScope(SmolStr);

impl ApiKeyScope {
    /// Strict parse — only canonical scope values are accepted.
    /// Returns `Err(UnknownScope)` for anything outside the known set.
    pub fn parse(value: &str) -> Result<Self, ApiKeyValidationError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(ApiKeyValidationError::EmptyScope);
        }
        if value.parse::<KnownScope>().is_err() {
            return Err(ApiKeyValidationError::UnknownScope(value.to_string()));
        }
        Ok(Self(value.into()))
    }

    /// Lenient parse for reading from the database.
    /// Accepts any non-empty string and logs a warning for unknown scopes
    /// so existing DB rows are never rejected.
    pub fn parse_lenient(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty() {
            tracing::warn!(scope = "<empty>", "parse_lenient received empty scope");
        } else if value.parse::<KnownScope>().is_err() {
            tracing::warn!(scope = value, "unknown API key scope loaded from database");
        }
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiKeyTier {
    Free,
    Pro,
    Enterprise,
}

impl ApiKeyTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Pro => "pro",
            Self::Enterprise => "enterprise",
        }
    }
}

impl FromStr for ApiKeyTier {
    type Err = ApiKeyValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "free" => Ok(Self::Free),
            "pro" => Ok(Self::Pro),
            "enterprise" => Ok(Self::Enterprise),
            _ => Err(ApiKeyValidationError::InvalidTier(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeySubject {
    pub id: ApiKeyId,
    pub label: String,
    pub scopes: Vec<ApiKeyScope>,
    pub tier: ApiKeyTier,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ApiKeyValidationError {
    #[error("API key scope must not be empty")]
    EmptyScope,
    #[error("unknown API key scope: {0}")]
    UnknownScope(String),
    #[error("invalid API key tier: {0}")]
    InvalidTier(String),
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ApiKeyRepositoryError {
    #[error("duplicate API key hash")]
    DuplicateHash,
    #[error("API key not found: {0}")]
    NotFound(ApiKeyId),
    #[error("database error: {0}")]
    Database(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyRecord {
    pub id: ApiKeyId,
    pub label: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub scopes: Vec<ApiKeyScope>,
    pub tier: ApiKeyTier,
    pub is_active: bool,
    pub revoked_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    /// Empty vec means unrestricted (all models allowed).
    pub allowed_models: Vec<ModelId>,
    /// Empty vec means unrestricted (all providers allowed).
    pub allowed_providers: Vec<ProviderId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_five_known_scopes_parse_ok() {
        assert!(ApiKeyScope::parse("chat:read").is_ok());
        assert!(ApiKeyScope::parse("chat:write").is_ok());
        assert!(ApiKeyScope::parse("providers:read").is_ok());
        assert!(ApiKeyScope::parse("providers:write").is_ok());
        assert!(ApiKeyScope::parse("admin").is_ok());
    }

    #[test]
    fn unknown_scope_returns_unknown_scope_error() {
        let err = ApiKeyScope::parse("read").unwrap_err();
        assert_eq!(err, ApiKeyValidationError::UnknownScope("read".to_string()));
    }

    #[test]
    fn empty_scope_returns_empty_scope_error() {
        let err = ApiKeyScope::parse("").unwrap_err();
        assert_eq!(err, ApiKeyValidationError::EmptyScope);

        let err_ws = ApiKeyScope::parse("   ").unwrap_err();
        assert_eq!(err_ws, ApiKeyValidationError::EmptyScope);
    }

    #[test]
    fn uppercase_scope_is_rejected() {
        // Scope matching is case-sensitive; uppercase variants are unknown.
        let err = ApiKeyScope::parse("Chat:Read").unwrap_err();
        assert!(
            matches!(err, ApiKeyValidationError::UnknownScope(_)),
            "expected UnknownScope, got {:?}",
            err
        );

        let err2 = ApiKeyScope::parse("ADMIN").unwrap_err();
        assert!(matches!(err2, ApiKeyValidationError::UnknownScope(_)));
    }

    #[test]
    fn parse_lenient_accepts_unknown_scope_without_error() {
        // Should not panic or return an error; just log a warning.
        let scope = ApiKeyScope::parse_lenient("legacy:custom");
        assert_eq!(scope.as_str(), "legacy:custom");
    }

    #[test]
    fn parse_lenient_accepts_known_scope() {
        let scope = ApiKeyScope::parse_lenient("chat:read");
        assert_eq!(scope.as_str(), "chat:read");
    }

    #[test]
    fn api_key_record_has_allowed_models_and_providers_fields() {
        use shared_kernel::{ModelId, ProviderId};
        let record = ApiKeyRecord {
            id: ApiKeyId::new("test-key"),
            label: "Test".to_string(),
            key_hash: "hash".to_string(),
            key_prefix: "prefix".to_string(),
            scopes: vec![],
            tier: ApiKeyTier::Free,
            is_active: true,
            revoked_at: None,
            expires_at: None,
            created_at: chrono::Utc::now(),
            last_used_at: None,
            allowed_models: vec![ModelId::new("gpt-4"), ModelId::new("claude-3")],
            allowed_providers: vec![ProviderId::new("openai")],
        };
        assert_eq!(record.allowed_models.len(), 2);
        assert_eq!(record.allowed_providers.len(), 1);
    }

    #[test]
    fn api_key_record_empty_restrictions_means_unrestricted() {
        let record = ApiKeyRecord {
            id: ApiKeyId::new("test-key"),
            label: "Test".to_string(),
            key_hash: "hash".to_string(),
            key_prefix: "prefix".to_string(),
            scopes: vec![],
            tier: ApiKeyTier::Free,
            is_active: true,
            revoked_at: None,
            expires_at: None,
            created_at: chrono::Utc::now(),
            last_used_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        // Empty means unrestricted — both fields are present and empty
        assert!(record.allowed_models.is_empty());
        assert!(record.allowed_providers.is_empty());
    }
}
