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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKeyScope(SmolStr);

impl ApiKeyScope {
    pub fn parse(value: &str) -> Result<Self, ApiKeyValidationError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(ApiKeyValidationError::EmptyScope);
        }
        Ok(Self(value.into()))
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
