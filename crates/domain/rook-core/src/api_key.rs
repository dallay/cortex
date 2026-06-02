use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
        if value.parse::<KnownScope>().is_err() && !value.is_empty() {
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
}
