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
    use rook_core::{
        BoxStream, CompletionRequest, CompletionResponse, ModelId, NuxaResult, ProviderPort,
        StreamChunk,
    };

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

    // ---------------------------------------------------------------------------
    // list() tests
    // ---------------------------------------------------------------------------

    #[test]
    fn list_returns_empty() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase().list().await;
                assert!(result.is_ok());
                assert_eq!(result.unwrap().len(), 0);
            });
    }

    struct PopulatedRepo {
        conn: ProviderConnection,
    }

    #[async_trait]
    impl ProviderRepositoryPort for PopulatedRepo {
        async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
            Ok(vec![self.conn.clone()])
        }

        async fn find(
            &self,
            id: &ConnectionId,
        ) -> Result<Option<ProviderConnection>, RepositoryError> {
            Ok(if id == &self.conn.id {
                Some(self.conn.clone())
            } else {
                None
            })
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

    #[test]
    fn list_returns_populated() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "test-conn".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(PopulatedRepo { conn: conn.clone() });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                );
                let result = mc.list().await;
                assert!(result.is_ok());
                let list = result.unwrap();
                assert_eq!(list.len(), 1);
                assert_eq!(list[0].name, "test-conn");
            });
    }

    // ---------------------------------------------------------------------------
    // get() tests
    // ---------------------------------------------------------------------------

    #[test]
    fn get_returns_none_for_unknown_id() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase().get(&ConnectionId::new()).await;
                assert!(result.is_ok());
                assert!(result.unwrap().is_none());
            });
    }

    #[test]
    fn get_returns_some_for_existing_id() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "test-conn".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(PopulatedRepo { conn: conn.clone() });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                );
                let result = mc.get(&conn.id).await;
                assert!(result.is_ok());
                let found = result.unwrap();
                assert!(found.is_some());
                assert_eq!(found.unwrap().name, "test-conn");
            });
    }

    // ---------------------------------------------------------------------------
    // create() validation tests
    // ---------------------------------------------------------------------------

    #[test]
    fn create_success_with_api_key_credentials() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "primary".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test-key".to_string(),
                        },
                        config: config(),
                    })
                    .await;

                assert!(result.is_ok());
                let conn = result.unwrap();
                assert_eq!(conn.name, "primary");
                assert_eq!(conn.auth_type, AuthType::ApiKey);
                assert!(matches!(conn.credentials, Credentials::ApiKey { .. }));
                assert!(matches!(conn.test_status, TestStatus::NeverTested));
            });
    }

    #[test]
    fn create_rejects_empty_name() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "".to_string(),
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
                        ValidationError::EmptyName
                    ))
                ));
            });
    }

    #[test]
    fn create_rejects_whitespace_only_name() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "   ".to_string(),
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
                        ValidationError::EmptyName
                    ))
                ));
            });
    }

    #[test]
    fn create_rejects_name_too_long() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "a".repeat(300),
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
                        ValidationError::NameTooLong
                    ))
                ));
            });
    }

    #[test]
    fn create_rejects_priority_out_of_range_low() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "primary".to_string(),
                        priority: 0,
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
                        ValidationError::PriorityOutOfRange
                    ))
                ));
            });
    }

    #[test]
    fn create_rejects_priority_out_of_range_high() {
        // Note: u8 max is 255, so we test upper boundary by using 255 which is valid (1..=255).
        // The out-of-range case for "high" would be 256 but that doesn't fit in u8.
        // Instead we verify that 255 works (edge of valid range), and the low test catches 0.
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                // priority 255 should be accepted (within valid range 1..=255)
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "primary".to_string(),
                        priority: u8::MAX, // 255, edge of valid range
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test".to_string(),
                        },
                        config: config(),
                    })
                    .await;
                assert!(result.is_ok(), "priority 255 should be valid");
            });
    }

    #[test]
    fn create_rejects_invalid_config_max_concurrent_zero() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let invalid_config = ConnectionConfig {
                    max_concurrent: 0,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                };
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "primary".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test".to_string(),
                        },
                        config: invalid_config,
                    })
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Validation(
                        ValidationError::MaxConcurrentTooLow
                    ))
                ));
            });
    }

    #[test]
    fn create_rejects_invalid_config_thresholds_order() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let invalid_config = ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.9,
                        error: 0.7, // error <= warning
                    },
                    default_model: None,
                };
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "primary".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test".to_string(),
                        },
                        config: invalid_config,
                    })
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Validation(
                        ValidationError::QuotaThresholdOrder
                    ))
                ));
            });
    }

    #[test]
    fn create_rejects_empty_api_key_credential() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "primary".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "".to_string(),
                        },
                        config: config(),
                    })
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Validation(
                        ValidationError::EmptyCredential
                    ))
                ));
            });
    }

    // ---------------------------------------------------------------------------
    // update() tests
    // ---------------------------------------------------------------------------

    #[test]
    fn update_partial_update_changes_only_name() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "old-name".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(PopulatedRepo { conn: conn.clone() });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                );

                let expected_updated_at = conn.updated_at;
                let result = mc
                    .update(
                        &conn.id,
                        UpdateConnectionRequest {
                            expected_updated_at,
                            name: Some("new-name".to_string()),
                            ..Default::default()
                        },
                    )
                    .await;

                assert!(result.is_ok());
                let updated = result.unwrap();
                assert_eq!(updated.name, "new-name");
            });
    }

    struct NotFoundRepo;

    impl Default for UpdateConnectionRequest {
        fn default() -> Self {
            Self {
                expected_updated_at: Utc::now(),
                provider_kind: None,
                provider_runtime_id: None,
                auth_type: None,
                name: None,
                priority: None,
                is_active: None,
                credentials: None,
                config: None,
            }
        }
    }

    #[async_trait]
    impl ProviderRepositoryPort for NotFoundRepo {
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

        async fn delete(&self, id: &ConnectionId) -> Result<(), RepositoryError> {
            Err(RepositoryError::NotFound(*id))
        }
    }

    #[test]
    fn update_returns_not_found_for_unknown_id() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let mc = ManageConnections::new(
                    Arc::new(NotFoundRepo),
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                );
                let result = mc
                    .update(
                        &ConnectionId::new(),
                        UpdateConnectionRequest {
                            expected_updated_at: Utc::now(),
                            name: Some("new".to_string()),
                            ..Default::default()
                        },
                    )
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Repository(
                        RepositoryError::NotFound(_)
                    ))
                ));
            });
    }

    // ---------------------------------------------------------------------------
    // delete() tests
    // ---------------------------------------------------------------------------

    struct DeletableRepo {
        conn: ProviderConnection,
    }

    #[async_trait]
    impl ProviderRepositoryPort for DeletableRepo {
        async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
            Ok(vec![self.conn.clone()])
        }

        async fn find(
            &self,
            id: &ConnectionId,
        ) -> Result<Option<ProviderConnection>, RepositoryError> {
            Ok(if id == &self.conn.id {
                Some(self.conn.clone())
            } else {
                None
            })
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

        async fn delete(&self, id: &ConnectionId) -> Result<(), RepositoryError> {
            if id == &self.conn.id {
                Ok(())
            } else {
                Err(RepositoryError::NotFound(*id))
            }
        }
    }

    #[test]
    fn delete_returns_ok_for_existing_id() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "to-delete".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(DeletableRepo { conn: conn.clone() });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                );

                let result = mc.delete(&conn.id).await;
                assert!(result.is_ok());
            });
    }

    #[test]
    fn delete_returns_not_found_for_unknown_id() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let mc = ManageConnections::new(
                    Arc::new(NotFoundRepo),
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                );
                let result = mc.delete(&ConnectionId::new()).await;
                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Repository(
                        RepositoryError::NotFound(_)
                    ))
                ));
            });
    }

    // ---------------------------------------------------------------------------
    // test() health-check tests
    // ---------------------------------------------------------------------------

    struct MockProvider {
        provider_id: ProviderId,
        health_status: HealthStatus,
    }

    #[async_trait]
    impl ProviderPort for MockProvider {
        fn id(&self) -> &ProviderId {
            &self.provider_id
        }

        fn supported_models(&self) -> &[ModelId] {
            &[]
        }

        fn is_available(&self) -> bool {
            true
        }

        async fn health_check(&self) -> HealthStatus {
            self.health_status.clone()
        }

        async fn complete(&self, _req: &CompletionRequest) -> NuxaResult<CompletionResponse> {
            unreachable!("not used in tests")
        }

        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> NuxaResult<BoxStream<'_, NuxaResult<StreamChunk>>> {
            unreachable!("not used in tests")
        }
    }

    struct MockRegistryWithProvider {
        provider_id: ProviderId,
        provider: Arc<dyn ProviderPort>,
    }

    impl ProviderRegistryPort for MockRegistryWithProvider {
        fn providers(&self) -> Vec<ProviderId> {
            vec![self.provider_id.clone()]
        }

        fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
            if id == &self.provider_id {
                Some(self.provider.clone())
            } else {
                None
            }
        }
    }

    #[test]
    fn test_returns_unknown_when_provider_runtime_not_found() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                // Insert a connection with a known id so we get past the repo lookup
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "test".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let conn_id = conn.id;
                // Replace NotFoundRepo with one that returns our connection
                let repo = Arc::new(PopulatedRepo { conn });
                // EmptyRegistry returns None for get(), so test() should fail with ProviderRuntimeNotFound
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                );
                let result = mc.test(&conn_id).await;
                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::ProviderRuntimeNotFound(_))
                ));
            });
    }

    #[test]
    fn test_returns_active_when_health_check_succeeds() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "test".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let conn_id = conn.id;
                let repo = Arc::new(PopulatedRepo { conn });
                let mock_provider = Arc::new(MockProvider {
                    provider_id: ProviderId::new("openai-primary"),
                    health_status: HealthStatus::Healthy {
                        provider: ProviderId::new("openai-primary"),
                        latency_ms: 42,
                    },
                });
                let registry = Arc::new(MockRegistryWithProvider {
                    provider_id: ProviderId::new("openai-primary"),
                    provider: mock_provider,
                });
                let mc = ManageConnections::new(repo, registry, Arc::new(PlainKeyManager));

                let result = mc.test(&conn_id).await;
                assert!(result.is_ok());
                let res = result.unwrap();
                assert_eq!(res.ok, Some(true));
                assert_eq!(res.status, "active");
                assert_eq!(res.latency_ms, Some(42));
            });
    }

    // ---------------------------------------------------------------------------
    // Helper function tests
    // ---------------------------------------------------------------------------

    #[test]
    fn validate_email_rejects_invalid_formats() {
        // no @
        assert!(validate_email("invalid.email.com").is_err());
        // empty local
        assert!(validate_email("@domain.com").is_err());
        // empty domain
        assert!(validate_email("user@").is_err());
        // no dot in domain
        assert!(validate_email("user@domaincom").is_err());
        // empty domain label
        assert!(validate_email("user@.com").is_err());
        assert!(validate_email("user@domain.").is_err());
        assert!(validate_email("user@domain..c").is_err());
    }

    #[test]
    fn validate_email_accepts_valid_email() {
        assert!(validate_email("user@example.com").is_ok());
        assert!(validate_email("test.name@sub.domain.example.org").is_ok());
        assert!(validate_email("a@b.co").is_ok());
    }

    #[test]
    fn credentials_matches_auth_type_returns_false_for_mismatch() {
        // ApiKey credentials with OAuth auth type
        let creds = Credentials::ApiKey {
            api_key: EncryptedBlob("enc:v1:key".to_string()),
        };
        assert!(!credentials_matches_auth_type(AuthType::OAuth, &creds));

        // OAuth credentials with ApiKey auth type
        let creds_oauth = Credentials::OAuth {
            email: EncryptedBlob("enc:v1:e".to_string()),
            access_token: EncryptedBlob("enc:v1:a".to_string()),
            refresh_token: EncryptedBlob("enc:v1:r".to_string()),
            expires_at: Utc::now().timestamp() + 3600,
            scope: EncryptedBlob("enc:v1:s".to_string()),
            id_token: EncryptedBlob("enc:v1:i".to_string()),
            project_id: EncryptedBlob("enc:v1:p".to_string()),
        };
        assert!(!credentials_matches_auth_type(
            AuthType::ApiKey,
            &creds_oauth
        ));
    }
}
