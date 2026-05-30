use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared_kernel::{ConnectionId, ModelId, ProviderId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthType {
    ApiKey,
    OAuth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedBlob(pub String);

impl EncryptedBlob {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credentials {
    ApiKey {
        api_key: EncryptedBlob,
    },
    OAuth {
        email: EncryptedBlob,
        access_token: EncryptedBlob,
        refresh_token: EncryptedBlob,
        expires_at: i64,
        scope: EncryptedBlob,
        id_token: EncryptedBlob,
        project_id: EncryptedBlob,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaWindowThresholds {
    pub warning: f32,
    pub error: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub max_concurrent: u32,
    pub quota_window_thresholds: QuotaWindowThresholds,
    pub default_model: Option<ModelId>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum TestStatus {
    #[default]
    NeverTested,
    Active {
        last_test_at: DateTime<Utc>,
        latency_ms: u64,
    },
    Unhealthy {
        last_test_at: DateTime<Utc>,
        error: String,
    },
    Expired {
        last_test_at: DateTime<Utc>,
        expires_at: i64,
    },
    Unknown {
        last_test_at: DateTime<Utc>,
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenAI,
    Anthropic,
    Ollama,
    Gemini,
    Groq,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
            Self::Gemini => "gemini",
            Self::Groq => "groq",
        }
    }
}

impl TryFrom<&str> for ProviderKind {
    type Error = ValidationError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.to_ascii_lowercase().as_str() {
            "openai" => Ok(Self::OpenAI),
            "anthropic" => Ok(Self::Anthropic),
            "ollama" => Ok(Self::Ollama),
            "gemini" => Ok(Self::Gemini),
            "groq" => Ok(Self::Groq),
            _ => Err(ValidationError::InvalidProviderKind),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderConnection {
    pub id: ConnectionId,
    pub provider_kind: ProviderKind,
    pub provider_runtime_id: ProviderId,
    pub name: String,
    pub priority: u8,
    pub is_active: bool,
    pub auth_type: AuthType,
    pub credentials: Credentials,
    pub config: ConnectionConfig,
    pub test_status: TestStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    InvalidProviderKind,
    EmptyRuntimeId,
    EmptyName,
    NameTooLong,
    PriorityOutOfRange,
    MaxConcurrentTooLow,
    QuotaThresholdOutOfRange,
    QuotaThresholdOrder,
    EmptyCredential,
    AuthTypeCredentialMismatch,
    OAuthFieldMissing(&'static str),
    OAuthEmailInvalid,
    OAuthExpiresAtPast,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidProviderKind => write!(f, "invalid provider kind"),
            Self::EmptyRuntimeId => write!(f, "provider runtime id must not be empty"),
            Self::EmptyName => write!(f, "name must not be empty"),
            Self::NameTooLong => write!(f, "name must be at most 256 Unicode scalar values"),
            Self::PriorityOutOfRange => write!(f, "priority must be between 1 and 255"),
            Self::MaxConcurrentTooLow => write!(f, "max_concurrent must be at least 1"),
            Self::QuotaThresholdOutOfRange => write!(f, "quota thresholds must be in [0.0, 1.0]"),
            Self::QuotaThresholdOrder => {
                write!(f, "quota threshold error must be greater than warning")
            }
            Self::EmptyCredential => write!(f, "credential value must not be empty"),
            Self::AuthTypeCredentialMismatch => {
                write!(f, "credentials do not match auth type")
            }
            Self::OAuthFieldMissing(field) => write!(f, "OAuth {field} must not be empty"),
            Self::OAuthEmailInvalid => write!(f, "OAuth email format is invalid"),
            Self::OAuthExpiresAtPast => write!(f, "OAuth expires_at must be in the future"),
        }
    }
}

impl std::error::Error for ValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryError {
    NotFound(ConnectionId),
    DuplicateId(ConnectionId),
    DuplicateConnection(String),
    StaleUpdate,
    Encryption(String),
    Database(String),
}

impl std::fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "connection not found: {id}"),
            Self::DuplicateId(id) => write!(f, "duplicate connection id: {id}"),
            Self::DuplicateConnection(name) => write!(f, "duplicate connection: {name}"),
            Self::StaleUpdate => write!(f, "connection was modified since last read"),
            Self::Encryption(message) => write!(f, "encryption error: {message}"),
            Self::Database(message) => write!(f, "database error: {message}"),
        }
    }
}

impl std::error::Error for RepositoryError {}

impl TestStatus {
    pub fn latency_ms(&self) -> Option<u64> {
        match self {
            Self::Active { latency_ms, .. } => Some(*latency_ms),
            _ => None,
        }
    }

    pub fn error_msg(&self) -> Option<String> {
        match self {
            Self::Unhealthy { error, .. } => Some(error.clone()),
            Self::Unknown { reason, .. } => Some(reason.clone()),
            _ => None,
        }
    }

    pub fn expires_at(&self) -> Option<i64> {
        match self {
            Self::Expired { expires_at, .. } => Some(*expires_at),
            _ => None,
        }
    }

    pub fn last_test_at(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Active { last_test_at, .. }
            | Self::Unhealthy { last_test_at, .. }
            | Self::Expired { last_test_at, .. }
            | Self::Unknown { last_test_at, .. } => Some(*last_test_at),
            Self::NeverTested => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_valid_cases() {
        for kind in ["openai", "anthropic", "ollama", "gemini", "groq"] {
            assert!(ProviderKind::try_from(kind).is_ok());
        }
    }

    #[test]
    fn provider_kind_case_insensitive() {
        assert_eq!(ProviderKind::try_from("OPENAI"), Ok(ProviderKind::OpenAI));
        assert_eq!(
            ProviderKind::try_from("Anthropic"),
            Ok(ProviderKind::Anthropic)
        );
    }

    #[test]
    fn provider_kind_invalid_cases() {
        for kind in ["unknown", "azure", "openrouter", "", "openai "] {
            assert_eq!(
                ProviderKind::try_from(kind),
                Err(ValidationError::InvalidProviderKind)
            );
        }
    }
}
