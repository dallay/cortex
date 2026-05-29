use std::sync::Arc;

use chrono::{DateTime, Utc};
use rook_core::{
    AuthType, ConnectionConfig, CredentialEncryptionError, Credentials, EncryptedBlob,
    HealthStatus, KeyManager, ProviderConnection, ProviderKind, ProviderRegistryPort,
    ProviderRepositoryPort, QuotaWindowThresholds, RepositoryError, TestStatus, ValidationError,
};
use shared_kernel::{ConnectionId, ProviderId};

#[derive(Debug, thiserror::Error)]
pub enum ManageConnectionsError {
    #[error("validation error: {0}")]
    Validation(#[from] ValidationError),
    #[error("repository error: {0}")]
    Repository(#[from] RepositoryError),
    #[error("encryption error: {0}")]
    Encryption(#[from] CredentialEncryptionError),
    #[error("provider runtime not found: {0}")]
    ProviderRuntimeNotFound(ProviderId),
}

pub type ManageConnectionsResult<T> = Result<T, ManageConnectionsError>;

pub struct ManageConnections {
    repo: Arc<dyn ProviderRepositoryPort>,
    registry: Arc<dyn ProviderRegistryPort>,
    key_manager: Arc<dyn KeyManager>,
}

impl ManageConnections {
    pub fn new(
        repo: Arc<dyn ProviderRepositoryPort>,
        registry: Arc<dyn ProviderRegistryPort>,
        key_manager: Arc<dyn KeyManager>,
    ) -> Self {
        Self {
            repo,
            registry,
            key_manager,
        }
    }

    pub async fn list(&self) -> ManageConnectionsResult<Vec<ProviderConnection>> {
        self.repo.list().await.map_err(Into::into)
    }

    pub async fn get(
        &self,
        id: &ConnectionId,
    ) -> ManageConnectionsResult<Option<ProviderConnection>> {
        self.repo.find(id).await.map_err(Into::into)
    }

    pub async fn create(
        &self,
        request: CreateConnectionRequest,
    ) -> ManageConnectionsResult<ProviderConnection> {
        validate_base_fields(
            &request.provider_runtime_id,
            &request.name,
            request.priority,
            &request.config,
        )?;

        let credentials = self.encrypt_credentials(&request.credentials)?;
        validate_credentials_not_expired(&credentials)?;
        if !credentials_matches_auth_type(request.auth_type, &credentials) {
            return Err(ValidationError::AuthTypeCredentialMismatch.into());
        }

        let now = Utc::now();
        let conn = ProviderConnection {
            id: ConnectionId::new(),
            provider_kind: request.provider_kind,
            provider_runtime_id: request.provider_runtime_id,
            name: request.name.trim().to_string(),
            priority: request.priority,
            is_active: request.is_active,
            auth_type: request.auth_type,
            credentials,
            config: request.config,
            test_status: TestStatus::NeverTested,
            created_at: now,
            updated_at: now,
        };

        self.repo.create(&conn).await?;
        Ok(conn)
    }

    pub async fn update(
        &self,
        id: &ConnectionId,
        request: UpdateConnectionRequest,
    ) -> ManageConnectionsResult<ProviderConnection> {
        let existing = self
            .repo
            .find(id)
            .await?
            .ok_or(RepositoryError::NotFound(*id))?;

        let auth_type = request.auth_type.unwrap_or(existing.auth_type);
        let credentials = match request.credentials {
            Some(credentials) => {
                let encrypted = self.encrypt_credentials(&credentials)?;
                validate_credentials_not_expired(&encrypted)?;
                encrypted
            }
            None => existing.credentials.clone(),
        };
        if !credentials_matches_auth_type(auth_type, &credentials) {
            return Err(ValidationError::AuthTypeCredentialMismatch.into());
        }

        let provider_runtime_id = request
            .provider_runtime_id
            .unwrap_or_else(|| existing.provider_runtime_id.clone());
        let name = request.name.unwrap_or_else(|| existing.name.clone());
        let priority = request.priority.unwrap_or(existing.priority);
        let config = request.config.unwrap_or_else(|| existing.config.clone());
        validate_base_fields(&provider_runtime_id, &name, priority, &config)?;

        let updated = ProviderConnection {
            id: existing.id,
            provider_kind: request.provider_kind.unwrap_or(existing.provider_kind),
            provider_runtime_id,
            name: name.trim().to_string(),
            priority,
            is_active: request.is_active.unwrap_or(existing.is_active),
            auth_type,
            credentials,
            config,
            test_status: existing.test_status,
            created_at: existing.created_at,
            updated_at: Utc::now(),
        };

        self.repo
            .update(&updated, request.expected_updated_at)
            .await?;
        Ok(updated)
    }

    pub async fn delete(&self, id: &ConnectionId) -> ManageConnectionsResult<()> {
        self.repo.delete(id).await.map_err(Into::into)
    }

    pub async fn test(&self, id: &ConnectionId) -> ManageConnectionsResult<TestConnectionResult> {
        let mut conn = self
            .repo
            .find(id)
            .await?
            .ok_or(RepositoryError::NotFound(*id))?;

        if let Credentials::OAuth { expires_at, .. } = conn.credentials {
            if expires_at <= Utc::now().timestamp() {
                let expected = conn.updated_at;
                conn.test_status = TestStatus::Expired {
                    last_test_at: Utc::now(),
                    expires_at,
                };
                conn.updated_at = Utc::now();
                self.repo.update(&conn, expected).await?;
                return Ok(TestConnectionResult {
                    ok: Some(false),
                    status: "expired".to_string(),
                    latency_ms: None,
                    error: Some(format!("OAuth token expired at {expires_at}")),
                });
            }
        }

        let provider = self
            .registry
            .get(&conn.provider_runtime_id)
            .ok_or_else(|| {
                ManageConnectionsError::ProviderRuntimeNotFound(conn.provider_runtime_id.clone())
            })?;
        let health = provider.health_check().await;
        let result = TestConnectionResult::from_health(&health);
        let expected = conn.updated_at;
        conn.test_status = test_status_from_health(health);
        conn.updated_at = Utc::now();
        self.repo.update(&conn, expected).await?;
        Ok(result)
    }

    fn encrypt_credentials(
        &self,
        input: &CredentialsInput,
    ) -> ManageConnectionsResult<Credentials> {
        match input {
            CredentialsInput::ApiKey { api_key } => {
                validate_non_empty(api_key, ValidationError::EmptyCredential)?;
                Ok(Credentials::ApiKey {
                    api_key: self.encrypt_blob(api_key)?,
                })
            }
            CredentialsInput::OAuth {
                email,
                access_token,
                refresh_token,
                expires_at,
                scope,
                id_token,
                project_id,
            } => {
                validate_email(email)?;
                validate_non_empty(
                    access_token,
                    ValidationError::OAuthFieldMissing("accessToken"),
                )?;
                validate_non_empty(
                    refresh_token,
                    ValidationError::OAuthFieldMissing("refreshToken"),
                )?;
                validate_non_empty(scope, ValidationError::OAuthFieldMissing("scope"))?;
                validate_non_empty(id_token, ValidationError::OAuthFieldMissing("idToken"))?;
                validate_non_empty(project_id, ValidationError::OAuthFieldMissing("projectId"))?;
                if *expires_at <= Utc::now().timestamp() {
                    return Err(ValidationError::OAuthExpiresAtPast.into());
                }

                Ok(Credentials::OAuth {
                    email: self.encrypt_blob(email)?,
                    access_token: self.encrypt_blob(access_token)?,
                    refresh_token: self.encrypt_blob(refresh_token)?,
                    expires_at: *expires_at,
                    scope: self.encrypt_blob(scope)?,
                    id_token: self.encrypt_blob(id_token)?,
                    project_id: self.encrypt_blob(project_id)?,
                })
            }
        }
    }

    fn encrypt_blob(&self, plaintext: &str) -> ManageConnectionsResult<EncryptedBlob> {
        Ok(EncryptedBlob(self.key_manager.encrypt(plaintext.trim())?))
    }
}

#[derive(Debug, Clone)]
pub struct CreateConnectionRequest {
    pub provider_kind: ProviderKind,
    pub provider_runtime_id: ProviderId,
    pub auth_type: AuthType,
    pub name: String,
    pub priority: u8,
    pub is_active: bool,
    pub credentials: CredentialsInput,
    pub config: ConnectionConfig,
}

#[derive(Debug, Clone)]
pub struct UpdateConnectionRequest {
    pub expected_updated_at: DateTime<Utc>,
    pub provider_kind: Option<ProviderKind>,
    pub provider_runtime_id: Option<ProviderId>,
    pub auth_type: Option<AuthType>,
    pub name: Option<String>,
    pub priority: Option<u8>,
    pub is_active: Option<bool>,
    pub credentials: Option<CredentialsInput>,
    pub config: Option<ConnectionConfig>,
}

#[derive(Debug, Clone)]
pub enum CredentialsInput {
    ApiKey {
        api_key: String,
    },
    OAuth {
        email: String,
        access_token: String,
        refresh_token: String,
        expires_at: i64,
        scope: String,
        id_token: String,
        project_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestConnectionResult {
    pub ok: Option<bool>,
    pub status: String,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

impl TestConnectionResult {
    fn from_health(health: &HealthStatus) -> Self {
        match health {
            HealthStatus::Healthy { latency_ms, .. } => Self {
                ok: Some(true),
                status: "active".to_string(),
                latency_ms: Some(*latency_ms),
                error: None,
            },
            HealthStatus::Unhealthy {
                latency_ms, error, ..
            } => Self {
                ok: Some(false),
                status: "unhealthy".to_string(),
                latency_ms: *latency_ms,
                error: Some(error.clone()),
            },
            HealthStatus::Unknown { reason, .. } => Self {
                ok: None,
                status: "unknown".to_string(),
                latency_ms: None,
                error: Some(reason.clone()),
            },
        }
    }
}

fn test_status_from_health(health: HealthStatus) -> TestStatus {
    let last_test_at = Utc::now();
    match health {
        HealthStatus::Healthy { latency_ms, .. } => TestStatus::Active {
            last_test_at,
            latency_ms,
        },
        HealthStatus::Unhealthy { error, .. } => TestStatus::Unhealthy {
            last_test_at,
            error,
        },
        HealthStatus::Unknown { reason, .. } => TestStatus::Unknown {
            last_test_at,
            reason,
        },
    }
}

fn validate_base_fields(
    provider_runtime_id: &ProviderId,
    name: &str,
    priority: u8,
    config: &ConnectionConfig,
) -> Result<(), ValidationError> {
    if provider_runtime_id.as_str().trim().is_empty() {
        return Err(ValidationError::EmptyRuntimeId);
    }
    if name.trim().is_empty() {
        return Err(ValidationError::EmptyName);
    }
    if name.chars().count() > 256 {
        return Err(ValidationError::NameTooLong);
    }
    if !(1..=255).contains(&priority) {
        return Err(ValidationError::PriorityOutOfRange);
    }
    validate_config(config)
}

fn validate_config(config: &ConnectionConfig) -> Result<(), ValidationError> {
    if config.max_concurrent < 1 {
        return Err(ValidationError::MaxConcurrentTooLow);
    }
    let QuotaWindowThresholds { warning, error } = config.quota_window_thresholds;
    if !warning.is_finite()
        || !error.is_finite()
        || !(0.0..=1.0).contains(&warning)
        || !(0.0..=1.0).contains(&error)
    {
        return Err(ValidationError::QuotaThresholdOutOfRange);
    }
    if error <= warning {
        return Err(ValidationError::QuotaThresholdOrder);
    }
    Ok(())
}

fn validate_non_empty(value: &str, error: ValidationError) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        Err(error)
    } else {
        Ok(())
    }
}

fn validate_email(email: &str) -> Result<(), ValidationError> {
    validate_non_empty(email, ValidationError::OAuthFieldMissing("email"))?;
    let email = email.trim();
    let mut parts = email.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    if parts.next().is_some() || local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return Err(ValidationError::OAuthEmailInvalid);
    }
    // Reject empty domain labels (e.g., "a@.com", "a@domain.", "a@b..c")
    for label in domain.split('.') {
        if label.is_empty() {
            return Err(ValidationError::OAuthEmailInvalid);
        }
    }
    Ok(())
}

fn validate_credentials_not_expired(credentials: &Credentials) -> Result<(), ValidationError> {
    if let Credentials::OAuth { expires_at, .. } = credentials {
        if *expires_at <= Utc::now().timestamp() {
            return Err(ValidationError::OAuthExpiresAtPast);
        }
    }
    Ok(())
}

fn credentials_matches_auth_type(auth_type: AuthType, credentials: &Credentials) -> bool {
    matches!(
        (auth_type, credentials),
        (AuthType::ApiKey, Credentials::ApiKey { .. })
            | (AuthType::OAuth, Credentials::OAuth { .. })
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rook_core::ProviderPort;

    struct NoopRepository;

    #[async_trait]
    impl ProviderRepositoryPort for NoopRepository {
        async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
            Ok(Vec::new())
        }

        async fn find(
            &self,
            _id: &ConnectionId,
        ) -> Result<Option<ProviderConnection>, RepositoryError> {
            Ok(None)
        }

        async fn create(&self, _conn: &ProviderConnection) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn update(
            &self,
            _conn: &ProviderConnection,
            _expected_updated_at: DateTime<Utc>,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn delete(&self, _id: &ConnectionId) -> Result<(), RepositoryError> {
            Ok(())
        }
    }

    struct EmptyRegistry;

    impl ProviderRegistryPort for EmptyRegistry {
        fn providers(&self) -> Vec<ProviderId> {
            Vec::new()
        }

        fn get(&self, _id: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
            None
        }
    }

    struct PlainKeyManager;

    impl KeyManager for PlainKeyManager {
        fn encrypt(&self, plaintext: &str) -> Result<String, rook_core::CredentialEncryptionError> {
            Ok(format!("enc:v1:{plaintext}"))
        }

        fn decrypt(
            &self,
            ciphertext: &str,
        ) -> Result<String, rook_core::CredentialEncryptionError> {
            Ok(ciphertext.to_string())
        }
    }

    fn usecase() -> ManageConnections {
        ManageConnections::new(
            Arc::new(NoopRepository),
            Arc::new(EmptyRegistry),
            Arc::new(PlainKeyManager),
        )
    }

    fn config() -> ConnectionConfig {
        ConnectionConfig {
            max_concurrent: 1,
            quota_window_thresholds: QuotaWindowThresholds {
                warning: 0.7,
                error: 0.9,
            },
            default_model: None,
        }
    }

    #[test]
    fn create_rejects_credentials_that_do_not_match_auth_type() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::OAuth,
                        name: "primary".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test".to_string(),
                        },
                        config: config(),
                    })
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Validation(
                        ValidationError::AuthTypeCredentialMismatch
                    ))
                ));
            });
    }
}
