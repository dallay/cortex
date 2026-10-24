use super::*;
use async_trait::async_trait;
use chrono::Utc;
use rook_core::{
    ApiFormat, BoxStream, CompletionRequest, CompletionResponse, CortexResult, ModelId,
    ProviderPort, RegistryError, StreamChunk,
};
use rook_core::{
    AuthType, ConnectionConfig, Credentials, EncryptedBlob, HealthStatus, ProviderConnection,
    ProviderKind, QuotaWindowThresholds, TestStatus, ValidationError,
};
use shared_kernel::{ConnectionId, ProviderId};
use std::sync::Arc;

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

    fn replace_all(&self, _providers: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError> {
        Ok(())
    }

    fn upsert(&self, _provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError> {
        Ok(())
    }

    fn remove(&self, _id: &ProviderId) -> Result<(), RegistryError> {
        Ok(())
    }
}

struct PlainKeyManager;

impl KeyManager for PlainKeyManager {
    fn encrypt(&self, plaintext: &str) -> Result<String, rook_core::CredentialEncryptionError> {
        Ok(format!("enc:v1:{plaintext}"))
    }

    fn decrypt(&self, ciphertext: &str) -> Result<String, rook_core::CredentialEncryptionError> {
        Ok(ciphertext.to_string())
    }
}

struct NoopProviderBuilder;

#[async_trait]
impl ProviderBuilderPort for NoopProviderBuilder {
    async fn build(
        &self,
        _input: ProviderBuildInput,
    ) -> ManageConnectionsResult<Arc<dyn ProviderPort>> {
        Err(ManageConnectionsError::RegistryUpdateFailed(
            "NoopProviderBuilder cannot build providers".to_string(),
        ))
    }
}

fn usecase() -> ManageConnections {
    ManageConnections::new(
        Arc::new(NoopRepository),
        Arc::new(EmptyRegistry),
        Arc::new(PlainKeyManager),
        Arc::new(NoopProviderBuilder),
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
        base_url: None,
        api_format: Some(rook_core::ApiFormat::OpenAI),
        supported_models: Vec::new(),
    }
}

#[test]
fn admission_state_is_reused_for_the_same_connection_id() {
    let manager = usecase();
    let connection_id = ConnectionId::new();

    let old_state = manager.admission_state_for_connection(connection_id, 2);
    let refreshed_state = manager.admission_state_for_connection(connection_id, 1);

    assert!(Arc::ptr_eq(&old_state, &refreshed_state));
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

    async fn find(&self, id: &ConnectionId) -> Result<Option<ProviderConnection>, RepositoryError> {
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
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
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
                Arc::new(NoopProviderBuilder),
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
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
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
                Arc::new(NoopProviderBuilder),
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

// 8.9 - create success with OAuth credentials
#[test]
fn create_success_with_oauth_credentials() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let result = usecase()
                .create(CreateConnectionRequest {
                    provider_kind: ProviderKind::Gemini,
                    provider_runtime_id: ProviderId::new("google-primary"),
                    auth_type: AuthType::OAuth,
                    name: "google-oauth".to_string(),
                    priority: 1,
                    is_active: true,
                    credentials: CredentialsInput::OAuth {
                        email: "user@example.com".to_string(),
                        // nosemgrep: generic.secrets.security.detected-google-oauth-access-token.detected-google-oauth-access-token, generic_secret
                        access_token: "fake_google_access_token_data".to_string(),
                        refresh_token: "1//refresh_token".to_string(),
                        expires_at: Utc::now().timestamp() + 3600,
                        scope: "https://www.googleapis.com/auth/bigquery".to_string(),
                        id_token: "id.token".to_string(),
                        project_id: "project-123".to_string(),
                        oauth_client_id: "google-client".to_string(),
                        oauth_client_secret: None,
                        oauth_tenant_id: None,
                    },
                    config: config(),
                })
                .await;
            assert!(result.is_ok());
            let conn = result.unwrap();
            assert_eq!(conn.name, "google-oauth");
            assert_eq!(conn.auth_type, AuthType::OAuth);
            assert!(matches!(conn.credentials, Credentials::OAuth { .. }));
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
                base_url: None,
                api_format: Some(rook_core::ApiFormat::OpenAI),
                supported_models: Vec::new(),
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
                base_url: None,
                api_format: Some(rook_core::ApiFormat::OpenAI),
                supported_models: Vec::new(),
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
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
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
                Arc::new(NoopProviderBuilder),
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

// 8.9 - update preserves credentials when not provided in request
#[test]
fn update_preserves_credentials_when_not_provided() {
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
                    api_key: EncryptedBlob("enc:v1:sk-original".to_string()),
                },
                config: ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
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
                Arc::new(NoopProviderBuilder),
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
            assert!(matches!(updated.credentials, Credentials::ApiKey { .. }));
        });
}

struct NotFoundRepo;

// 8.9 - update replaces credentials when provided in request
#[test]
fn update_replaces_credentials() {
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
                    api_key: EncryptedBlob("enc:v1:sk-old".to_string()),
                },
                config: ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
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
                Arc::new(NoopProviderBuilder),
            );

            let expected_updated_at = conn.updated_at;
            let result = mc
                .update(
                    &conn.id,
                    UpdateConnectionRequest {
                        expected_updated_at,
                        credentials: Some(CredentialsInput::ApiKey {
                            api_key: "sk-new-replacement".to_string(),
                        }),
                        ..Default::default()
                    },
                )
                .await;

            assert!(result.is_ok());
            let updated = result.unwrap();
            assert!(matches!(updated.credentials, Credentials::ApiKey { .. }));
        });
}

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
                Arc::new(NoopProviderBuilder),
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
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
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
                Arc::new(NoopProviderBuilder),
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
                Arc::new(NoopProviderBuilder),
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

    fn api_format(&self) -> ApiFormat {
        ApiFormat::OpenAI
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn health_check(&self) -> HealthStatus {
        self.health_status.clone()
    }

    async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        unreachable!("not used in tests")
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>> {
        Ok(Box::pin(futures::stream::pending()))
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

    fn replace_all(&self, _providers: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError> {
        Ok(())
    }

    fn upsert(&self, _provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError> {
        Ok(())
    }

    fn remove(&self, _id: &ProviderId) -> Result<(), RegistryError> {
        Ok(())
    }
}
struct RepoWithConnections {
    connections: Vec<ProviderConnection>,
}

#[async_trait]
impl ProviderRepositoryPort for RepoWithConnections {
    async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
        Ok(self.connections.clone())
    }

    async fn find(&self, id: &ConnectionId) -> Result<Option<ProviderConnection>, RepositoryError> {
        Ok(self.connections.iter().find(|c| c.id == *id).cloned())
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

struct MockRegistry {
    providers: std::sync::Mutex<Vec<Arc<dyn ProviderPort>>>,
    reject_replacement: std::sync::atomic::AtomicBool,
}

impl MockRegistry {
    fn new() -> Self {
        Self {
            providers: std::sync::Mutex::new(Vec::new()),
            reject_replacement: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl Default for MockRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRegistryPort for MockRegistry {
    fn providers(&self) -> Vec<ProviderId> {
        self.providers
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.id().clone())
            .collect()
    }

    fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
        self.providers
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.id() == id)
            .cloned()
    }

    fn replace_all(&self, new_providers: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError> {
        if self
            .reject_replacement
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(RegistryError::RegistryLocked);
        }
        *self.providers.lock().unwrap() = new_providers;
        Ok(())
    }

    fn upsert(&self, _provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError> {
        Ok(())
    }

    fn remove(&self, _id: &ProviderId) -> Result<(), RegistryError> {
        Ok(())
    }
}

struct FailDecryptKeyManager;

impl KeyManager for FailDecryptKeyManager {
    fn encrypt(&self, _plaintext: &str) -> Result<String, rook_core::CredentialEncryptionError> {
        Ok("enc:v1:encrypted".to_string())
    }

    fn decrypt(&self, _ciphertext: &str) -> Result<String, rook_core::CredentialEncryptionError> {
        Err(rook_core::CredentialEncryptionError::Decrypt(
            "intentional test failure".to_string(),
        ))
    }
}

struct SuccessProviderBuilder {
    provider: Arc<dyn ProviderPort>,
    inputs: std::sync::Mutex<Vec<ProviderBuildInput>>,
}

#[async_trait]
impl ProviderBuilderPort for SuccessProviderBuilder {
    async fn build(
        &self,
        input: ProviderBuildInput,
    ) -> ManageConnectionsResult<Arc<dyn ProviderPort>> {
        self.inputs.lock().unwrap().push(input);
        Ok(self.provider.clone())
    }
}

use std::sync::Mutex;

struct RepoWithRefreshTracker {
    store: Arc<Mutex<Vec<ProviderConnection>>>,
    refresh_count: Arc<Mutex<usize>>,
}

impl Default for RepoWithRefreshTracker {
    fn default() -> Self {
        Self {
            store: Arc::new(Mutex::new(Vec::new())),
            refresh_count: Arc::new(Mutex::new(0)),
        }
    }
}

impl RepoWithRefreshTracker {
    fn refresh_count(&self) -> usize {
        *self.refresh_count.lock().unwrap()
    }
}

#[async_trait]
impl ProviderRepositoryPort for RepoWithRefreshTracker {
    async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
        *self.refresh_count.lock().unwrap() += 1;
        Ok(self.store.lock().unwrap().clone())
    }

    async fn find(&self, id: &ConnectionId) -> Result<Option<ProviderConnection>, RepositoryError> {
        Ok(self
            .store
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.id == *id)
            .cloned())
    }

    async fn create(&self, conn: &ProviderConnection) -> Result<(), RepositoryError> {
        self.store.lock().unwrap().push(conn.clone());
        Ok(())
    }

    async fn update(
        &self,
        conn: &ProviderConnection,
        _expected_updated_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        let mut store = self.store.lock().unwrap();
        if let Some(existing) = store.iter_mut().find(|c| c.id == conn.id) {
            *existing = conn.clone();
        }
        Ok(())
    }

    async fn delete(&self, id: &ConnectionId) -> Result<(), RepositoryError> {
        self.store.lock().unwrap().retain(|c| c.id != *id);
        Ok(())
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
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
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
                Arc::new(NoopProviderBuilder),
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
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
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
            let mc = ManageConnections::new(
                repo,
                registry,
                Arc::new(PlainKeyManager),
                Arc::new(NoopProviderBuilder),
            );

            let result = mc.test(&conn_id).await;
            assert!(result.is_ok());
            let res = result.unwrap();
            assert!(res.valid, "expected valid=true for healthy");
            assert_eq!(res.status, "ok");
            assert_eq!(res.latency_ms, Some(42));
        });
}

// 8.10 - test returns unhealthy when health check fails
#[test]
fn test_returns_unhealthy_when_health_check_fails() {
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
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let conn_id = conn.id;
            let repo = Arc::new(PopulatedRepo { conn });
            let mock_provider = Arc::new(MockProvider {
                provider_id: ProviderId::new("openai-primary"),
                health_status: HealthStatus::Unhealthy {
                    provider: ProviderId::new("openai-primary"),
                    latency_ms: None,
                    error: "connection refused".to_string(),
                },
            });
            let registry = Arc::new(MockRegistryWithProvider {
                provider_id: ProviderId::new("openai-primary"),
                provider: mock_provider,
            });
            let mc = ManageConnections::new(
                repo,
                registry,
                Arc::new(PlainKeyManager),
                Arc::new(NoopProviderBuilder),
            );

            let result = mc.test(&conn_id).await;
            assert!(result.is_ok());
            let res = result.unwrap();
            assert!(!res.valid, "expected valid=false for unhealthy");
            assert_eq!(res.status, "unhealthy");
        });
}

// 8.10 - test returns expired OAuth when token is expired
#[test]
fn test_returns_expired_oauth() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let expired_at = Utc::now().timestamp() - 3600;
            let conn = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::Gemini,
                provider_runtime_id: ProviderId::new("google-primary"),
                name: "test".to_string(),
                priority: 1,
                is_active: true,
                auth_type: AuthType::OAuth,
                credentials: Credentials::OAuth {
                    email: EncryptedBlob("enc:v1:user@example.com".to_string()),
                    // nosemgrep: generic.secrets.security.detected-google-oauth-access-token.detected-google-oauth-access-token
                    access_token: EncryptedBlob("enc:v1:fake-google-token-data".to_string()),
                    refresh_token: EncryptedBlob("enc:v1:refresh".to_string()),
                    expires_at: expired_at,
                    scope: EncryptedBlob("enc:v1:scope".to_string()),
                    id_token: EncryptedBlob("enc:v1:id".to_string()),
                    project_id: EncryptedBlob("enc:v1:project".to_string()),
                    oauth_client_id: "google-client".to_string(),
                    oauth_client_secret: None,
                    oauth_tenant_id: None,
                },
                config: ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let conn_id = conn.id;
            let repo = Arc::new(PopulatedRepo { conn });
            let mc = ManageConnections::new(
                repo,
                Arc::new(EmptyRegistry),
                Arc::new(PlainKeyManager),
                Arc::new(NoopProviderBuilder),
            );

            let result = mc.test(&conn_id).await;
            assert!(result.is_ok());
            let res = result.unwrap();
            assert!(!res.valid, "expected valid=false for expired");
            assert_eq!(res.status, "expired");
            assert!(res.error.is_some());
            assert_eq!(res.method.as_deref(), Some("oauth_expired"));
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
        oauth_client_id: "client".to_string(),
        oauth_client_secret: None,
        oauth_tenant_id: None,
    };
    assert!(!credentials_matches_auth_type(
        AuthType::ApiKey,
        &creds_oauth
    ));
}

// ---------------------------------------------------------------------------
// refresh_registry tests (5.6-5.12)
// ---------------------------------------------------------------------------

struct PausingBuilder {
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}

#[async_trait]
impl ProviderBuilderPort for PausingBuilder {
    async fn build(
        &self,
        input: ProviderBuildInput,
    ) -> ManageConnectionsResult<Arc<dyn ProviderPort>> {
        self.entered.notify_one();
        self.resume.notified().await;
        Ok(Arc::new(MockProvider {
            provider_id: ProviderId::new(input.default_model.unwrap().as_str()),
            health_status: HealthStatus::Unknown {
                provider: ProviderId::new("dynamic"),
                reason: "test".into(),
            },
        }))
    }
}

#[tokio::test]
async fn concurrent_refreshes_publish_matching_snapshots_and_admission_limits() {
    let connection = ProviderConnection {
        id: ConnectionId::new(),
        provider_kind: ProviderKind::OpenAI,
        provider_runtime_id: ProviderId::new("dynamic"),
        name: "dynamic".into(),
        priority: 1,
        is_active: true,
        auth_type: AuthType::ApiKey,
        credentials: Credentials::ApiKey {
            api_key: EncryptedBlob("enc:v1:test".into()),
        },
        config: ConnectionConfig {
            default_model: Some(ModelId::new("first")),
            ..config()
        },
        test_status: TestStatus::NeverTested,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let repo = Arc::new(RepoWithRefreshTracker::default());
    repo.create(&connection).await.unwrap();
    let builder = Arc::new(PausingBuilder {
        entered: Default::default(),
        resume: Default::default(),
    });
    let registry = Arc::new(MockRegistry::new());
    let manager = ManageConnections::new(
        repo.clone(),
        registry.clone(),
        Arc::new(PlainKeyManager),
        builder.clone(),
    );
    let first = manager.refresh_registry();
    tokio::pin!(first);
    assert!(futures::poll!(&mut first).is_pending());
    builder.entered.notified().await;

    // A second refresh must wait before reading a snapshot or building it.
    let mut updated = connection.clone();
    updated.config.max_concurrent = 2;
    updated.config.default_model = Some(ModelId::new("second"));
    repo.update(&updated, connection.updated_at).await.unwrap();
    let second = manager.refresh_registry();
    tokio::pin!(second);
    assert!(futures::poll!(&mut second).is_pending());
    assert_eq!(repo.refresh_count(), 1);

    builder.resume.notify_one();
    first.await.unwrap();
    let old_provider = registry.get(&ProviderId::new("first")).unwrap();
    let request = CompletionRequest {
        id: shared_kernel::RequestId::new(),
        model: ModelId::new("test"),
        messages: vec![],
        stream: true,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".into(),
            cacheable: false,
            priority: 1,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: Default::default(),
    };
    let first_stream = old_provider.stream(&request).await.unwrap();
    assert!(old_provider
        .stream(&request)
        .await
        .err()
        .unwrap()
        .is_local_backpressure());

    assert!(futures::poll!(&mut second).is_pending());
    builder.entered.notified().await;
    // The in-progress build must leave the active snapshot's cap at one.
    assert!(old_provider
        .stream(&request)
        .await
        .err()
        .unwrap()
        .is_local_backpressure());
    builder.resume.notify_one();
    second.await.unwrap();
    assert_eq!(repo.refresh_count(), 2);
    assert_eq!(registry.providers(), vec![ProviderId::new("second")]);
    let current = registry.get(&ProviderId::new("second")).unwrap();
    let second_stream = current.stream(&request).await.unwrap();
    assert!(current
        .stream(&request)
        .await
        .err()
        .unwrap()
        .is_local_backpressure());
    // Both snapshots still share capacity after the limit is raised.
    assert!(old_provider
        .stream(&request)
        .await
        .err()
        .unwrap()
        .is_local_backpressure());
    drop(first_stream);
    assert!(current.stream(&request).await.is_ok());

    // A failed publication must leave both the snapshot and cap intact.
    updated.config.max_concurrent = 1;
    repo.update(&updated, connection.updated_at).await.unwrap();
    registry
        .reject_replacement
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let failed = manager.refresh_registry();
    tokio::pin!(failed);
    assert!(futures::poll!(&mut failed).is_pending());
    builder.entered.notified().await;
    builder.resume.notify_one();
    assert!(failed.await.is_err());
    assert!(Arc::ptr_eq(
        &current,
        &registry.get(&ProviderId::new("second")).unwrap()
    ));
    assert!(current.stream(&request).await.is_ok());
    drop(second_stream);
}

// 5.6 - refresh_registry skips inactive connections
#[test]
fn refresh_registry_skips_inactive_connections() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let active_conn = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::OpenAI,
                provider_runtime_id: ProviderId::new("openai-primary"),
                name: "active".to_string(),
                priority: 1,
                is_active: true,
                auth_type: AuthType::ApiKey,
                credentials: Credentials::ApiKey {
                    api_key: EncryptedBlob("enc:v1:sk-active".to_string()),
                },
                config: ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let inactive_conn = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::Anthropic,
                provider_runtime_id: ProviderId::new("anthropic-backup"),
                name: "inactive".to_string(),
                priority: 2,
                is_active: false,
                auth_type: AuthType::ApiKey,
                credentials: Credentials::ApiKey {
                    api_key: EncryptedBlob("enc:v1:sk-inactive".to_string()),
                },
                config: ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let expected_connection_id = active_conn.id;
            let expected_runtime_id = active_conn.provider_runtime_id.clone();
            let repo = Arc::new(RepoWithConnections {
                connections: vec![active_conn, inactive_conn],
            });
            let mock_provider = Arc::new(MockProvider {
                provider_id: ProviderId::new("openai-primary"),
                health_status: HealthStatus::Healthy {
                    provider: ProviderId::new("openai-primary"),
                    latency_ms: 1,
                },
            });
            let builder = Arc::new(SuccessProviderBuilder {
                provider: mock_provider,
                inputs: Mutex::default(),
            });
            let mc = ManageConnections::new(
                repo,
                Arc::new(MockRegistry::new()),
                Arc::new(PlainKeyManager),
                builder.clone(),
            );
            mc.refresh_registry()
                .await
                .expect("refresh_registry should succeed");
            let providers = mc.registry.providers();
            assert_eq!(providers.len(), 1);
            assert_eq!(providers[0].as_str(), "openai-primary");
            let inputs = builder.inputs.lock().unwrap();
            assert_eq!(inputs.len(), 1);
            assert_eq!(inputs[0].connection_id, expected_connection_id);
            assert_eq!(inputs[0].provider_runtime_id, expected_runtime_id);
        });
}

// 5.7 - refresh_registry decrypts and builds provider successfully
#[test]
fn refresh_registry_decrypts_and_builds_provider() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let conn = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::OpenAI,
                provider_runtime_id: ProviderId::new("openai-primary"),
                name: "primary".to_string(),
                priority: 1,
                is_active: true,
                auth_type: AuthType::ApiKey,
                credentials: Credentials::ApiKey {
                    api_key: EncryptedBlob("enc:v1:sk-test-key".to_string()),
                },
                config: ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let repo = Arc::new(PopulatedRepo { conn });
            let mock_provider = Arc::new(MockProvider {
                provider_id: ProviderId::new("openai-primary"),
                health_status: HealthStatus::Healthy {
                    provider: ProviderId::new("openai-primary"),
                    latency_ms: 1,
                },
            });
            let registry = Arc::new(MockRegistryWithProvider {
                provider_id: ProviderId::new("openai-primary"),
                provider: mock_provider.clone(),
            });
            let mc = ManageConnections::new(
                repo,
                registry.clone(),
                Arc::new(PlainKeyManager),
                Arc::new(SuccessProviderBuilder {
                    provider: mock_provider,
                    inputs: Mutex::default(),
                }),
            );
            mc.refresh_registry()
                .await
                .expect("refresh_registry should succeed");
            let loaded = registry.get(&ProviderId::new("openai-primary"));
            assert!(loaded.is_some());
            assert_eq!(loaded.unwrap().id().as_str(), "openai-primary");
        });
}

// 5.8 - refresh_registry partial failure: one decrypt fails, other still added
#[test]
fn refresh_registry_partial_failure_keeps_valid_providers() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let conn_ok = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::OpenAI,
                provider_runtime_id: ProviderId::new("openai-primary"),
                name: "ok".to_string(),
                priority: 1,
                is_active: true,
                auth_type: AuthType::ApiKey,
                credentials: Credentials::ApiKey {
                    api_key: EncryptedBlob("enc:v1:sk-ok".to_string()),
                },
                config: ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let conn_bad = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::Anthropic,
                provider_runtime_id: ProviderId::new("anthropic-bad"),
                name: "bad".to_string(),
                priority: 2,
                is_active: true,
                auth_type: AuthType::ApiKey,
                credentials: Credentials::ApiKey {
                    api_key: EncryptedBlob("enc:v1:sk-bad".to_string()),
                },
                config: ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let repo = Arc::new(RepoWithConnections {
                connections: vec![conn_ok.clone(), conn_bad.clone()],
            });
            let mock_provider = Arc::new(MockProvider {
                provider_id: ProviderId::new("openai-primary"),
                health_status: HealthStatus::Healthy {
                    provider: ProviderId::new("openai-primary"),
                    latency_ms: 1,
                },
            });
            let registry = Arc::new(MockRegistryWithProvider {
                provider_id: ProviderId::new("openai-primary"),
                provider: mock_provider.clone(),
            });
            let mc = ManageConnections::new(
                repo,
                registry.clone(),
                Arc::new(FailDecryptKeyManager),
                Arc::new(SuccessProviderBuilder {
                    provider: mock_provider,
                    inputs: Mutex::default(),
                }),
            );
            mc.refresh_registry()
                .await
                .expect("refresh_registry should succeed (partial failure is warn)");
            let providers = registry.providers();
            assert_eq!(providers.len(), 1);
            assert_eq!(providers[0].as_str(), "openai-primary");
        });
}

// Failed validation and provider construction retain the prior runtime for active connections.
#[test]
fn refresh_registry_preserves_previous_provider_when_active_connection_fails_validation() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let mut connection = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::AzureOpenAI,
                provider_runtime_id: ProviderId::new("azure-primary"),
                name: "azure-primary".into(),
                priority: 1,
                is_active: true,
                auth_type: AuthType::OAuth,
                credentials: Credentials::OAuth {
                    email: EncryptedBlob("enc:v1:user@example.com".into()),
                    access_token: EncryptedBlob("enc:v1:token".into()),
                    refresh_token: EncryptedBlob("enc:v1:refresh".into()),
                    expires_at: Utc::now().timestamp() + 3600,
                    scope: EncryptedBlob("enc:v1:scope".into()),
                    id_token: EncryptedBlob("enc:v1:id".into()),
                    project_id: EncryptedBlob("enc:v1:project".into()),
                    oauth_client_id: "azure-client".into(),
                    oauth_client_secret: None,
                    oauth_tenant_id: None,
                },
                config: ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            connection.config.default_model = Some(ModelId::new("deployment"));
            let provider_id = ProviderId::new(connection.id.to_string());
            let registry = Arc::new(MockRegistry::new());
            registry
                .replace_all(vec![Arc::new(MockProvider {
                    provider_id: provider_id.clone(),
                    health_status: HealthStatus::Unknown {
                        provider: provider_id.clone(),
                        reason: "previous".into(),
                    },
                })])
                .unwrap();
            let manager = ManageConnections::new(
                Arc::new(RepoWithConnections {
                    connections: vec![connection],
                }),
                registry.clone(),
                Arc::new(PlainKeyManager),
                Arc::new(NoopProviderBuilder),
            );

            manager.refresh_registry().await.unwrap();

            assert_eq!(registry.providers(), vec![provider_id.clone()]);
            assert!(registry.get(&provider_id).is_some());
        });
}

#[test]
fn refresh_registry_preserves_previous_provider_when_active_connection_build_fails() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let connection = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::OpenAI,
                provider_runtime_id: ProviderId::new("openai-primary"),
                name: "openai-primary".into(),
                priority: 1,
                is_active: true,
                auth_type: AuthType::ApiKey,
                credentials: Credentials::ApiKey {
                    api_key: EncryptedBlob("enc:v1:token".into()),
                },
                config: ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let provider_id = ProviderId::new(connection.id.to_string());
            let registry = Arc::new(MockRegistry::new());
            registry
                .replace_all(vec![Arc::new(MockProvider {
                    provider_id: provider_id.clone(),
                    health_status: HealthStatus::Unknown {
                        provider: provider_id.clone(),
                        reason: "previous".into(),
                    },
                })])
                .unwrap();
            let manager = ManageConnections::new(
                Arc::new(RepoWithConnections {
                    connections: vec![connection],
                }),
                registry.clone(),
                Arc::new(PlainKeyManager),
                Arc::new(NoopProviderBuilder),
            );

            manager.refresh_registry().await.unwrap();

            assert_eq!(registry.providers(), vec![provider_id.clone()]);
            assert!(registry.get(&provider_id).is_some());
        });
}

// A failed rebuild of an active connection keeps its previously published provider.
#[test]
fn refresh_registry_preserves_previous_provider_when_active_connection_fails_to_decrypt() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let connection = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::OpenAI,
                provider_runtime_id: ProviderId::new("openai-primary"),
                name: "primary".into(),
                priority: 1,
                is_active: true,
                auth_type: AuthType::ApiKey,
                credentials: Credentials::ApiKey {
                    api_key: EncryptedBlob("enc:v1:sk-test".into()),
                },
                config: ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let provider_id = ProviderId::new(connection.id.to_string());
            let previous_provider = Arc::new(MockProvider {
                provider_id: provider_id.clone(),
                health_status: HealthStatus::Healthy {
                    provider: provider_id.clone(),
                    latency_ms: 1,
                },
            });
            let registry = Arc::new(MockRegistry::new());
            registry.replace_all(vec![previous_provider]).unwrap();
            let manager = ManageConnections::new(
                Arc::new(RepoWithConnections {
                    connections: vec![connection],
                }),
                registry.clone(),
                Arc::new(FailDecryptKeyManager),
                Arc::new(NoopProviderBuilder),
            );

            manager.refresh_registry().await.unwrap();

            assert_eq!(registry.providers(), vec![provider_id.clone()]);
            assert!(registry.get(&provider_id).is_some());
        });
}

// 5.9 - refresh_registry all failures: results in empty registry
#[test]
fn refresh_registry_all_failures_results_in_empty_registry() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let conn = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::OpenAI,
                provider_runtime_id: ProviderId::new("openai-primary"),
                name: "primary".to_string(),
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
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let repo = Arc::new(PopulatedRepo { conn });
            let mc = ManageConnections::new(
                repo,
                Arc::new(EmptyRegistry),
                Arc::new(FailDecryptKeyManager),
                Arc::new(NoopProviderBuilder),
            );
            mc.refresh_registry()
                .await
                .expect("refresh_registry should succeed even when all fail");
            let providers = mc.registry.providers();
            assert!(providers.is_empty());
        });
}

// 5.10 - create calls refresh_registry after write
#[test]
fn create_calls_refresh_after_write() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let repo = Arc::new(RepoWithRefreshTracker::default());
            let mc = ManageConnections::new(
                repo.clone(),
                Arc::new(EmptyRegistry),
                Arc::new(PlainKeyManager),
                Arc::new(NoopProviderBuilder),
            );
            mc.create(CreateConnectionRequest {
                provider_kind: ProviderKind::OpenAI,
                provider_runtime_id: ProviderId::new("openai-primary"),
                auth_type: AuthType::ApiKey,
                name: "primary".to_string(),
                priority: 1,
                is_active: true,
                credentials: CredentialsInput::ApiKey {
                    api_key: "sk-test".to_string(),
                },
                config: config(),
            })
            .await
            .expect("create should succeed");
            assert_eq!(
                repo.refresh_count(),
                1,
                "refresh_registry should be called once after create"
            );
        });
}

// 5.11 - update calls refresh_registry after write
#[test]
fn update_calls_refresh_after_write() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let existing_conn = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::OpenAI,
                provider_runtime_id: ProviderId::new("openai-primary"),
                name: "primary".to_string(),
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
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let repo = Arc::new(RepoWithRefreshTracker::default());
            repo.store
                .clone()
                .lock()
                .unwrap()
                .push(existing_conn.clone());
            let mc = ManageConnections::new(
                repo.clone(),
                Arc::new(EmptyRegistry),
                Arc::new(PlainKeyManager),
                Arc::new(NoopProviderBuilder),
            );
            mc.update(
                &existing_conn.id,
                UpdateConnectionRequest {
                    expected_updated_at: existing_conn.updated_at,
                    provider_kind: None,
                    provider_runtime_id: None,
                    auth_type: None,
                    name: Some("updated-name".to_string()),
                    priority: None,
                    is_active: None,
                    credentials: None,
                    config: None,
                },
            )
            .await
            .expect("update should succeed");
            assert_eq!(
                repo.refresh_count(),
                1,
                "refresh_registry should be called once after update"
            );
        });
}

// 5.12 - delete calls refresh_registry after write
#[test]
fn delete_calls_refresh_after_write() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let existing_conn = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::OpenAI,
                provider_runtime_id: ProviderId::new("openai-primary"),
                name: "primary".to_string(),
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
                    base_url: None,
                    api_format: Some(rook_core::ApiFormat::OpenAI),
                    supported_models: Vec::new(),
                },
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let repo = Arc::new(RepoWithRefreshTracker::default());
            repo.store
                .clone()
                .lock()
                .unwrap()
                .push(existing_conn.clone());
            let mc = ManageConnections::new(
                repo.clone(),
                Arc::new(EmptyRegistry),
                Arc::new(PlainKeyManager),
                Arc::new(NoopProviderBuilder),
            );
            mc.delete(&existing_conn.id)
                .await
                .expect("delete should succeed");
            assert_eq!(
                repo.refresh_count(),
                1,
                "refresh_registry should be called once after delete"
            );
        });
}

fn connection_config(base_url: &str) -> ConnectionConfig {
    ConnectionConfig {
        max_concurrent: 1,
        quota_window_thresholds: rook_core::QuotaWindowThresholds {
            warning: 0.7,
            error: 0.9,
        },
        default_model: None,
        base_url: Some(base_url.to_string()),
        api_format: Some(rook_core::ApiFormat::OpenAI),
        supported_models: Vec::new(),
    }
}

#[test]
fn provider_config_rejects_normalized_cloud_metadata_hosts() {
    for url in [
        "http://169.254.169.254/latest/meta-data/",
        "http://100.100.100.200/",
        "http://[::ffff:169.254.169.254]/",
        "http://[::ffff:100.100.100.200]/",
        "http://metadata.google.internal./",
        "http://METADATA.GOOGLE.INTERNAL/",
        "http://[fd00:ec2::254]/",
    ] {
        assert!(
            matches!(
                validate_provider_config(
                    ProviderKind::OpenAI,
                    &connection_config(url),
                    &rook_core::EndpointPolicy::default(),
                    false
                ),
                Err(ValidationError::CloudMetadataEndpointNotAllowed)
            ),
            "expected cloud metadata validation error for {url}"
        );
    }
}

#[test]
fn provider_config_allows_explicit_private_provider_endpoints() {
    for (kind, url) in [
        (ProviderKind::OpenAI, "http://10.0.0.12:8080/v1"),
        (ProviderKind::Ollama, "http://127.0.0.1:11434"),
    ] {
        assert!(
            validate_provider_config(
                kind,
                &connection_config(url),
                &rook_core::EndpointPolicy::default(),
                false
            )
            .is_ok(),
            "explicit self-hosted endpoint should remain supported: {url}"
        );
    }
}

#[test]
fn refresh_registry_skips_stored_metadata_endpoint_before_decrypting() {
    struct PanicOnDecrypt;

    impl KeyManager for PanicOnDecrypt {
        fn encrypt(&self, plaintext: &str) -> Result<String, rook_core::CredentialEncryptionError> {
            Ok(plaintext.to_string())
        }

        fn decrypt(
            &self,
            _ciphertext: &str,
        ) -> Result<String, rook_core::CredentialEncryptionError> {
            panic!("invalid provider config must be checked before decrypting")
        }
    }

    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let conn = ProviderConnection {
                id: ConnectionId::new(),
                provider_kind: ProviderKind::OpenAI,
                provider_runtime_id: ProviderId::new("legacy-metadata-provider"),
                name: "legacy metadata endpoint".to_string(),
                priority: 1,
                is_active: true,
                auth_type: AuthType::ApiKey,
                credentials: Credentials::ApiKey {
                    api_key: EncryptedBlob("encrypted-key".to_string()),
                },
                config: connection_config("http://169.254.169.254/latest/meta-data/"),
                test_status: TestStatus::NeverTested,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let repo = Arc::new(RepoWithConnections {
                connections: vec![conn],
            });
            let registry = Arc::new(MockRegistry::new());
            let usecases = ManageConnections::new(
                repo,
                registry.clone(),
                Arc::new(PanicOnDecrypt),
                Arc::new(NoopProviderBuilder),
            );

            usecases
                .refresh_registry()
                .await
                .expect("invalid stored config is skipped, not fatal");

            assert!(registry.providers().is_empty());
        });
}

// ---------------------------------------------------------------------------
// OAuth redirect URI host allowlist (A01) regression tests
// ---------------------------------------------------------------------------

#[test]
fn redirect_uri_rejects_unlisted_https_host_with_oauth_error() {
    let origins = parse_allowed_oauth_redirect_origins("https://rook.example.com");
    let err = validate_oauth_redirect_uri_with_origins(
        "https://evil.example/api/oauth/callback",
        &origins,
    )
    .expect_err("unlisted host must be rejected");
    assert!(
        matches!(err, ManageConnectionsError::OAuth(_)),
        "must be an OAuth error, got: {err}"
    );
    assert!(err.to_string().contains("origin is not allowed"));
}

#[test]
fn redirect_uri_accepts_allowlisted_https_host() {
    let origins = parse_allowed_oauth_redirect_origins("https://rook.example.com");
    validate_oauth_redirect_uri_with_origins(
        "https://rook.example.com/api/oauth/callback",
        &origins,
    )
    .expect("allowlisted dashboard host must pass");
}

#[test]
fn redirect_uri_rejects_unlisted_port_on_allowlisted_host() {
    let origins = parse_allowed_oauth_redirect_origins("https://rook.example.com");
    let err = validate_oauth_redirect_uri_with_origins(
        "https://rook.example.com:8443/api/oauth/callback",
        &origins,
    )
    .expect_err("an unlisted port must not inherit the allowed host");
    assert!(matches!(err, ManageConnectionsError::OAuth(_)));
}

#[test]
fn redirect_uri_rejects_wrong_scheme_for_allowlisted_host() {
    let origins = parse_allowed_oauth_redirect_origins("https://rook.example.com");
    let err = validate_oauth_redirect_uri_with_origins(
        "http://rook.example.com/api/oauth/callback",
        &origins,
    )
    .expect_err("non-loopback HTTP must not inherit an HTTPS allowlist entry");
    assert!(matches!(err, ManageConnectionsError::OAuth(_)));
}

#[test]
fn redirect_uri_accepts_loopback_http_for_dev() {
    let origins = parse_allowed_oauth_redirect_origins("https://rook.example.com");
    validate_oauth_redirect_uri_with_origins("http://localhost:3000/api/oauth/callback", &origins)
        .expect("loopback HTTP must keep working for dev");
    validate_oauth_redirect_uri_with_origins("http://127.0.0.1:4747/api/oauth/callback", &origins)
        .expect("loopback HTTP must keep working for dev");
}

#[test]
fn redirect_uri_wildcard_origin_does_not_authorize_arbitrary_host() {
    let origins = parse_allowed_oauth_redirect_origins("*");
    let result = validate_oauth_redirect_uri_with_origins(
        "https://evil.example/api/oauth/callback",
        &origins,
    );
    let err = result.expect_err("wildcard must not authorize arbitrary hosts");
    assert!(matches!(err, ManageConnectionsError::OAuth(_)));
}
