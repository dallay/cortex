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
        oauth_client_id: String,
        oauth_client_secret: Option<EncryptedBlob>,
        oauth_tenant_id: Option<String>,
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
    #[serde(default)]
    pub supported_models: Vec<ModelId>,
    #[serde(default)]
    pub api_format: Option<crate::ApiFormat>,
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
    #[serde(rename = "openai-compatible")]
    OpenAiCompatible,
    Anthropic,
    Ollama,
    OllamaCloud,
    Gemini,
    Groq,
    #[serde(alias = "azure-openai")]
    AzureOpenAI,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAI => "openai",
            Self::OpenAiCompatible => "openai-compatible",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
            Self::OllamaCloud => "ollama-cloud",
            Self::Gemini => "gemini",
            Self::Groq => "groq",
            Self::AzureOpenAI => "azure-openai",
        }
    }
}

impl TryFrom<&str> for ProviderKind {
    type Error = ValidationError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.to_ascii_lowercase().as_str() {
            "openai" => Ok(Self::OpenAI),
            "openai-compatible" => Ok(Self::OpenAiCompatible),
            "anthropic" => Ok(Self::Anthropic),
            "ollama" => Ok(Self::Ollama),
            "ollama-cloud" => Ok(Self::OllamaCloud),
            "gemini" => Ok(Self::Gemini),
            "groq" => Ok(Self::Groq),
            "azure-openai" => Ok(Self::AzureOpenAI),
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
    CloudMetadataEndpointNotAllowed,
    InvalidBaseUrl,
    EmptySupportedModels,
    UnsupportedApiFormat,
    AzureOpenAIRequiresEndpoint,
    AzureOpenAIRequiresDeployment,
    AzureOpenAIRequiresOAuth,
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
            Self::CloudMetadataEndpointNotAllowed => {
                write!(
                    f,
                    "provider endpoint must not target cloud metadata services"
                )
            }
            Self::InvalidBaseUrl => write!(
                f,
                "provider endpoint is invalid or blocked by server policy"
            ),
            Self::EmptySupportedModels => write!(
                f,
                "OpenAI-compatible providers require at least one supported model"
            ),
            Self::UnsupportedApiFormat => write!(
                f,
                "OpenAI-compatible providers currently support only OpenAI Chat Completions"
            ),
            Self::AzureOpenAIRequiresEndpoint => {
                write!(f, "Azure OpenAI requires a valid HTTPS resource endpoint")
            }
            Self::AzureOpenAIRequiresDeployment => {
                write!(
                    f,
                    "Azure OpenAI requires a deployment name as default_model"
                )
            }
            Self::AzureOpenAIRequiresOAuth => {
                write!(f, "Azure OpenAI connections require OAuth authentication")
            }
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
        for kind in [
            "openai",
            "anthropic",
            "ollama",
            "ollama-cloud",
            "gemini",
            "groq",
        ] {
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
        assert_eq!(
            ProviderKind::try_from("OLLAMA-CLOUD"),
            Ok(ProviderKind::OllamaCloud)
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

    #[test]
    fn provider_kind_as_str_roundtrip() {
        for kind in [
            ProviderKind::OpenAI,
            ProviderKind::Anthropic,
            ProviderKind::Ollama,
            ProviderKind::OllamaCloud,
            ProviderKind::Gemini,
            ProviderKind::Groq,
        ] {
            assert_eq!(ProviderKind::try_from(kind.as_str()), Ok(kind));
        }
    }

    #[test]
    fn cloud_metadata_validation_error_has_specific_display_message() {
        assert_eq!(
            ValidationError::CloudMetadataEndpointNotAllowed.to_string(),
            "provider endpoint must not target cloud metadata services"
        );
    }

    #[test]
    fn test_status_accessors_expose_only_the_matching_outcome_data() {
        let timestamp = Utc::now();
        let active = TestStatus::Active {
            last_test_at: timestamp,
            latency_ms: 42,
        };
        assert_eq!(active.latency_ms(), Some(42));
        assert_eq!(active.last_test_at(), Some(timestamp));
        assert_eq!(active.error_msg(), None);
        assert_eq!(active.expires_at(), None);

        let unhealthy = TestStatus::Unhealthy {
            last_test_at: timestamp,
            error: "unauthorized".into(),
        };
        assert_eq!(unhealthy.error_msg().as_deref(), Some("unauthorized"));
        assert_eq!(unhealthy.latency_ms(), None);
        assert_eq!(unhealthy.expires_at(), None);
        assert_eq!(unhealthy.last_test_at(), Some(timestamp));

        let expired = TestStatus::Expired {
            last_test_at: timestamp,
            expires_at: 1_800_000_000,
        };
        assert_eq!(expired.expires_at(), Some(1_800_000_000));
        assert_eq!(expired.last_test_at(), Some(timestamp));
        assert_eq!(expired.error_msg(), None);

        let unknown = TestStatus::Unknown {
            last_test_at: timestamp,
            reason: "provider returned an unknown state".into(),
        };
        assert_eq!(
            unknown.error_msg().as_deref(),
            Some("provider returned an unknown state")
        );
        assert_eq!(unknown.last_test_at(), Some(timestamp));

        let never_tested = TestStatus::NeverTested;
        assert_eq!(never_tested.last_test_at(), None);
        assert_eq!(never_tested.latency_ms(), None);
        assert_eq!(never_tested.error_msg(), None);
        assert_eq!(never_tested.expires_at(), None);
    }
}
