use chrono::{DateTime, Utc};
use rook_core::{
    AuthType, ConnectionConfig, Credentials, ProviderConnection, ProviderKind, TestStatus,
};
use rook_usecases::manage_connections::{
    CreateConnectionRequest, CredentialsInput as UsecaseCredentialsInput, TestConnectionResult,
    UpdateConnectionRequest as UsecaseUpdateConnectionRequest,
};
use serde::{Deserialize, Serialize};
use shared_kernel::{ConnectionId, ModelId, ProviderId};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateProviderRequest {
    pub provider_kind: String,
    pub provider_runtime_id: ProviderId,
    pub auth_type: String,
    pub name: String,
    pub priority: u8,
    pub is_active: bool,
    pub credentials: CredentialsInput,
    pub config: ConnectionConfigDto,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateProviderRequest {
    pub expected_updated_at: String,
    pub provider_kind: Option<String>,
    pub provider_runtime_id: Option<ProviderId>,
    pub auth_type: Option<String>,
    pub name: Option<String>,
    pub priority: Option<u8>,
    pub is_active: Option<bool>,
    pub credentials: Option<CredentialsInput>,
    pub config: Option<ConnectionConfigDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCredentialsRequest {
    pub provider_kind: String,
    pub provider_runtime_id: String,
    pub auth_type: String,
    pub credentials: CredentialsInput,
    pub config: ConnectionConfigDto,
}

impl TestCredentialsRequest {
    pub fn try_into_domain(
        &self,
    ) -> Result<rook_usecases::manage_connections::TestCredentialsRequest, String> {
        use rook_core::ProviderKind;
        use shared_kernel::ProviderId;

        let provider_kind = ProviderKind::try_from(self.provider_kind.as_str())
            .map_err(|e| format!("invalid provider kind: {}", e))?;

        let auth_type = parse_auth_type(&self.auth_type)?;

        let credentials = credentials_to_usecase(&self.credentials);
        let config = config_to_domain(&self.config);

        Ok(rook_usecases::manage_connections::TestCredentialsRequest {
            provider_kind,
            provider_runtime_id: ProviderId::new(&self.provider_runtime_id),
            auth_type,
            credentials,
            config,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum CredentialsInput {
    ApiKey(ApiKeyCredentialsInput),
    OAuth(OAuthCredentialsInput),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiKeyCredentialsInput {
    pub api_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OAuthCredentialsInput {
    pub email: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub scope: String,
    pub id_token: String,
    pub project_id: String,
    #[serde(default)]
    pub oauth_client_id: Option<String>,
    #[serde(default)]
    pub oauth_client_secret: Option<String>,
    #[serde(default)]
    pub oauth_tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionConfigDto {
    pub max_concurrent: u32,
    #[serde(rename = "quotaWindowThresholds")]
    pub quota_window_thresholds: QuotaWindowThresholdsDto,
    pub default_model: Option<String>,
    pub base_url: Option<String>,
    #[serde(default)]
    pub supported_models: Vec<String>,
    #[serde(default)]
    pub api_format: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaWindowThresholdsDto {
    pub warning: f32,
    pub error: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionResponse {
    pub id: ConnectionId,
    pub provider_kind: String,
    pub provider_runtime_id: ProviderId,
    pub auth_type: String,
    pub name: String,
    pub priority: u8,
    pub is_active: bool,
    pub credentials: EmptyCredentials,
    pub config: ConnectionConfigResponse,
    pub test_status: TestStatusResponse,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_tenant_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EmptyCredentials {}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfigResponse {
    pub max_concurrent: u32,
    #[serde(rename = "quotaWindowThresholds")]
    pub quota_window_thresholds: QuotaWindowThresholdsResponse,
    pub default_model: Option<String>,
    pub base_url: Option<String>,
    pub supported_models: Vec<String>,
    pub api_format: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindowThresholdsResponse {
    pub warning: f32,
    pub error: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestStatusResponse {
    pub status: String,
    pub last_test_at: Option<DateTime<Utc>>,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestConnectionResponse {
    /// Whether the credentials are usable. `true` for `Healthy`,
    /// `Warning`, and `Unknown`; `false` for `Unhealthy` and `Expired`.
    /// The dashboard's Save button is enabled iff `valid === true`,
    /// regardless of `status` or `warning`.
    pub valid: bool,
    /// One of `"ok" | "warning" | "unhealthy" | "unknown" | "expired"`.
    pub status: String,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    /// Soft signal surfaced to the dashboard as a yellow alert. Set
    /// when credentials are valid but the probe saw a non-fatal
    /// condition (HTTP 429, no API key configured, etc.).
    pub warning: Option<String>,
    /// Probe method used to derive this result. Free-form string.
    pub method: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersQuotaResponse {
    pub generated_at: DateTime<Utc>,
    pub items: Vec<ProviderQuotaSummaryResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderQuotaSummaryResponse {
    pub provider_kind: String,
    pub connection_count: u32,
    pub active_connection_count: u32,
    pub coverage: &'static str,
    pub usage_availability: &'static str,
    pub account_quota: ProviderAccountQuotaResponse,
    pub local_thresholds: Vec<ProviderQuotaThresholdResponse>,
    pub rate_limit_observation: ProviderRateLimitObservationResponse,
    pub local_warning_level: &'static str,
    pub all_time: ProviderQuotaWindowResponse,
    pub last_24h: ProviderQuotaWindowResponse,
    pub last_7d: ProviderQuotaWindowResponse,
    pub trend: Vec<ProviderQuotaTrendPointResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountQuotaResponse {
    pub availability: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderQuotaThresholdResponse {
    pub warning: f32,
    pub error: f32,
    pub connection_count: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRateLimitObservationResponse {
    pub source: &'static str,
    pub availability: &'static str,
    pub window: &'static str,
    pub rate_limited_requests: Option<u64>,
    pub requests: Option<u64>,
    pub ratio: Option<f32>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderQuotaWindowResponse {
    pub availability: &'static str,
    pub rate_limit_availability: &'static str,
    pub requests: Option<u64>,
    pub rate_limited_requests: Option<u64>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub cost_availability: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderQuotaTrendPointResponse {
    pub date: String,
    pub usage_availability: &'static str,
    pub requests: Option<u64>,
    pub rate_limited_requests: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub cost_availability: &'static str,
}

impl TryFrom<&CreateProviderRequest> for CreateConnectionRequest {
    type Error = String;

    fn try_from(req: &CreateProviderRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            provider_kind: ProviderKind::try_from(req.provider_kind.as_str())
                .map_err(|e| e.to_string())?,
            provider_runtime_id: req.provider_runtime_id.clone(),
            auth_type: parse_auth_type(&req.auth_type)?,
            name: req.name.clone(),
            priority: req.priority,
            is_active: req.is_active,
            credentials: credentials_to_usecase(&req.credentials),
            config: config_to_domain(&req.config),
        })
    }
}

impl TryFrom<&UpdateProviderRequest> for UsecaseUpdateConnectionRequest {
    type Error = String;

    fn try_from(req: &UpdateProviderRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            expected_updated_at: DateTime::parse_from_rfc3339(&req.expected_updated_at)
                .map_err(|e| format!("invalid expectedUpdatedAt: {e}"))?
                .with_timezone(&Utc),
            provider_kind: req
                .provider_kind
                .as_ref()
                .map(|kind| ProviderKind::try_from(kind.as_str()).map_err(|e| e.to_string()))
                .transpose()?,
            provider_runtime_id: req.provider_runtime_id.clone(),
            auth_type: req
                .auth_type
                .as_ref()
                .map(|auth| parse_auth_type(auth))
                .transpose()?,
            name: req.name.clone(),
            priority: req.priority,
            is_active: req.is_active,
            credentials: req.credentials.as_ref().map(credentials_to_usecase),
            config: req.config.as_ref().map(config_to_domain),
        })
    }
}

impl From<&ProviderConnection> for ProviderConnectionResponse {
    fn from(conn: &ProviderConnection) -> Self {
        Self {
            id: conn.id,
            provider_kind: conn.provider_kind.as_str().to_string(),
            provider_runtime_id: conn.provider_runtime_id.clone(),
            auth_type: auth_type_str(conn.auth_type).to_string(),
            name: conn.name.clone(),
            priority: conn.priority,
            is_active: conn.is_active,
            credentials: EmptyCredentials {},
            config: ConnectionConfigResponse::from(&conn.config),
            test_status: TestStatusResponse::from(&conn.test_status),
            created_at: conn.created_at,
            updated_at: conn.updated_at,
            oauth_status: oauth_status(conn),
            oauth_expires_at: match &conn.credentials {
                Credentials::OAuth { expires_at, .. } => Some(*expires_at),
                Credentials::ApiKey { .. } => None,
            },
            oauth_client_id: match &conn.credentials {
                Credentials::OAuth {
                    oauth_client_id, ..
                } if !oauth_client_id.is_empty() => Some(oauth_client_id.clone()),
                _ => None,
            },
            oauth_tenant_id: match &conn.credentials {
                Credentials::OAuth {
                    oauth_tenant_id, ..
                } => oauth_tenant_id.clone(),
                Credentials::ApiKey { .. } => None,
            },
        }
    }
}

pub(crate) fn oauth_status(conn: &ProviderConnection) -> Option<String> {
    let Credentials::OAuth { expires_at, .. } = &conn.credentials else {
        return None;
    };
    // A failed refresh is checked first: once Rook knows the refresh token
    // no longer works, reauthorization is required even if (and when) the
    // access token is also past its expiry.
    let status = if matches!(
        &conn.test_status,
        TestStatus::Unhealthy { error, .. } if error.contains("OAuth token refresh failed")
    ) {
        "reauthentication_required"
    } else if *expires_at <= Utc::now().timestamp() {
        "expired"
    } else {
        "connected"
    };
    Some(status.to_string())
}

impl From<&ConnectionConfig> for ConnectionConfigResponse {
    fn from(config: &ConnectionConfig) -> Self {
        Self {
            max_concurrent: config.max_concurrent,
            quota_window_thresholds: QuotaWindowThresholdsResponse {
                warning: config.quota_window_thresholds.warning,
                error: config.quota_window_thresholds.error,
            },
            default_model: config.default_model.as_ref().map(ToString::to_string),
            base_url: config.base_url.clone(),
            supported_models: config
                .supported_models
                .iter()
                .map(ToString::to_string)
                .collect(),
            api_format: match config.api_format.unwrap_or(rook_core::ApiFormat::OpenAI) {
                rook_core::ApiFormat::OpenAI => "openai".to_string(),
                rook_core::ApiFormat::Anthropic => "anthropic".to_string(),
            },
        }
    }
}

impl From<&TestStatus> for TestStatusResponse {
    fn from(status: &TestStatus) -> Self {
        match status {
            TestStatus::NeverTested => Self {
                status: "neverTested".to_string(),
                last_test_at: None,
                latency_ms: None,
                error: None,
            },
            TestStatus::Active {
                last_test_at,
                latency_ms,
            } => Self {
                status: "active".to_string(),
                last_test_at: Some(*last_test_at),
                latency_ms: Some(*latency_ms),
                error: None,
            },
            TestStatus::Unhealthy {
                last_test_at,
                error,
            } => Self {
                status: "unhealthy".to_string(),
                last_test_at: Some(*last_test_at),
                latency_ms: None,
                error: Some(error.clone()),
            },
            TestStatus::Expired {
                last_test_at,
                expires_at,
            } => Self {
                status: "expired".to_string(),
                last_test_at: Some(*last_test_at),
                latency_ms: None,
                error: Some(format!("token expired at {expires_at}")),
            },
            TestStatus::Unknown {
                last_test_at,
                reason,
            } => Self {
                status: "unknown".to_string(),
                last_test_at: Some(*last_test_at),
                latency_ms: None,
                error: Some(reason.clone()),
            },
        }
    }
}

impl From<&TestConnectionResult> for TestConnectionResponse {
    fn from(result: &TestConnectionResult) -> Self {
        Self {
            valid: result.valid,
            status: result.status.clone(),
            latency_ms: result.latency_ms,
            error: result.error.clone(),
            warning: result.warning.clone(),
            method: result.method.clone(),
        }
    }
}

fn credentials_to_usecase(credentials: &CredentialsInput) -> UsecaseCredentialsInput {
    match credentials {
        CredentialsInput::ApiKey(credentials) => UsecaseCredentialsInput::ApiKey {
            api_key: credentials.api_key.clone(),
        },
        CredentialsInput::OAuth(credentials) => UsecaseCredentialsInput::OAuth {
            email: credentials.email.clone(),
            access_token: credentials.access_token.clone(),
            refresh_token: credentials.refresh_token.clone(),
            expires_at: credentials.expires_at,
            scope: credentials.scope.clone(),
            id_token: credentials.id_token.clone(),
            project_id: credentials.project_id.clone(),
            oauth_client_id: credentials.oauth_client_id.clone().unwrap_or_default(),
            oauth_client_secret: credentials.oauth_client_secret.clone(),
            oauth_tenant_id: credentials.oauth_tenant_id.clone(),
        },
    }
}
fn config_to_domain(config: &ConnectionConfigDto) -> ConnectionConfig {
    ConnectionConfig {
        max_concurrent: config.max_concurrent,
        quota_window_thresholds: rook_core::QuotaWindowThresholds {
            warning: config.quota_window_thresholds.warning,
            error: config.quota_window_thresholds.error,
        },
        default_model: config.default_model.as_ref().map(ModelId::new),
        base_url: config.base_url.clone(),
        supported_models: config.supported_models.iter().map(ModelId::new).collect(),
        api_format: config
            .api_format
            .as_deref()
            .and_then(|format| match format {
                "openai" => Some(rook_core::ApiFormat::OpenAI),
                "anthropic" => Some(rook_core::ApiFormat::Anthropic),
                _ => None,
            }),
    }
}

fn parse_auth_type(value: &str) -> Result<AuthType, String> {
    let lower = value.to_lowercase();
    match lower.as_str() {
        "apikey" | "api_key" | "api-key" => Ok(AuthType::ApiKey),
        "oauth" => Ok(AuthType::OAuth),
        _ => Err(format!("invalid authType: {value}")),
    }
}

fn auth_type_str(auth_type: AuthType) -> &'static str {
    match auth_type {
        AuthType::ApiKey => "apiKey",
        AuthType::OAuth => "oauth",
    }
}
