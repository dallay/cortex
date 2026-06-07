// Integration tests for ManageConnections::test_credentials()

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::{
    ApiFormat, AuthType, BoxStream, CompletionRequest, CompletionResponse, ConnectionConfig,
    CortexResult, HealthStatus, ModelId, ProviderConnection, ProviderId, ProviderKind,
    ProviderPort, ProviderRegistryPort, ProviderRepositoryPort, QuotaWindowThresholds,
    RegistryError, RepositoryError, StreamChunk,
};
use rook_usecases::{
    manage_connections::{
        CredentialsInput, ManageConnectionsResult, ProviderBuilderPort, TestCredentialsRequest,
    },
    ManageConnections, ProviderBuildInput,
};
use shared_kernel::ConnectionId;

// ---------------------------------------------------------------------------
// Mock Provider
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MockProvider {
    provider_id: ProviderId,
    health_status: HealthStatus,
}

impl MockProvider {
    fn new(provider_id: ProviderId, health_status: HealthStatus) -> Self {
        Self {
            provider_id,
            health_status,
        }
    }
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
        unreachable!("not used in test_credentials tests")
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>> {
        unreachable!("not used in test_credentials tests")
    }
}

// ---------------------------------------------------------------------------
// NoopRegistry
// ---------------------------------------------------------------------------

struct NoopRegistry;

impl ProviderRegistryPort for NoopRegistry {
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

// ---------------------------------------------------------------------------
// NoopKeyManager (echo encryption: encrypt returns "encrypted:X", decrypt strips prefix)
// ---------------------------------------------------------------------------

struct NoopKeyManager;

impl rook_core::KeyManager for NoopKeyManager {
    fn encrypt(&self, plaintext: &str) -> Result<String, rook_core::CredentialEncryptionError> {
        Ok(format!("encrypted:{plaintext}"))
    }

    fn decrypt(&self, ciphertext: &str) -> Result<String, rook_core::CredentialEncryptionError> {
        Ok(ciphertext.to_string())
    }
}

// ---------------------------------------------------------------------------
// NoopProviderBuilder — builds and returns the provided MockProvider
// ---------------------------------------------------------------------------

struct NoopProviderBuilder {
    provider: Arc<dyn ProviderPort>,
}

impl NoopProviderBuilder {
    fn new(provider: Arc<dyn ProviderPort>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl ProviderBuilderPort for NoopProviderBuilder {
    async fn build(
        &self,
        _input: ProviderBuildInput,
    ) -> ManageConnectionsResult<Arc<dyn ProviderPort>> {
        Ok(self.provider.clone())
    }
}

// ---------------------------------------------------------------------------
// InMemoryProviderRepository
// ---------------------------------------------------------------------------

#[derive(Default)]
struct InMemoryProviderRepository {
    store: std::sync::Mutex<Vec<ProviderConnection>>,
}

#[async_trait]
impl ProviderRepositoryPort for InMemoryProviderRepository {
    async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
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

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn config() -> ConnectionConfig {
    ConnectionConfig {
        max_concurrent: 1,
        quota_window_thresholds: QuotaWindowThresholds {
            warning: 0.7,
            error: 0.9,
        },
        default_model: None,
        base_url: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_credentials_returns_healthy_for_valid_credentials() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let mock_provider = Arc::new(MockProvider::new(
                ProviderId::new("ollama-primary"),
                HealthStatus::Healthy {
                    provider: ProviderId::new("ollama-primary"),
                    latency_ms: 123,
                },
            ));

            let mc = ManageConnections::new(
                Arc::new(InMemoryProviderRepository::default()),
                Arc::new(NoopRegistry),
                Arc::new(NoopKeyManager),
                Arc::new(NoopProviderBuilder::new(mock_provider)),
            );

            let result = mc
                .test_credentials(TestCredentialsRequest {
                    provider_kind: ProviderKind::Ollama,
                    provider_runtime_id: ProviderId::new("ollama-primary"),
                    auth_type: AuthType::ApiKey,
                    credentials: CredentialsInput::ApiKey {
                        api_key: "valid-api-key".to_string(),
                    },
                    config: config(),
                })
                .await;

            assert!(result.is_ok(), "expected Ok, got {:?}", result);
            let res = result.unwrap();
            assert!(res.valid, "expected valid=true for healthy");
            assert_eq!(
                res.status, "ok",
                "expected status='ok', got '{}'",
                res.status
            );
            assert_eq!(
                res.latency_ms,
                Some(123),
                "expected latency_ms=Some(123), got {:?}",
                res.latency_ms
            );
            assert!(
                res.error.is_none(),
                "expected error=None, got {:?}",
                res.error
            );
            assert!(
                res.warning.is_none(),
                "expected warning=None for healthy, got {:?}",
                res.warning
            );
        });
}

#[test]
fn test_credentials_does_not_persist_to_database() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let mock_provider = Arc::new(MockProvider::new(
                ProviderId::new("ollama-primary"),
                HealthStatus::Healthy {
                    provider: ProviderId::new("ollama-primary"),
                    latency_ms: 99,
                },
            ));

            let repo = Arc::new(InMemoryProviderRepository::default());

            let mc = ManageConnections::new(
                repo.clone(),
                Arc::new(NoopRegistry),
                Arc::new(NoopKeyManager),
                Arc::new(NoopProviderBuilder::new(mock_provider)),
            );

            mc.test_credentials(TestCredentialsRequest {
                provider_kind: ProviderKind::Ollama,
                provider_runtime_id: ProviderId::new("ollama-primary"),
                auth_type: AuthType::ApiKey,
                credentials: CredentialsInput::ApiKey {
                    api_key: "test-key".to_string(),
                },
                config: config(),
            })
            .await
            .expect("test_credentials should succeed");

            // Verify nothing was persisted
            let connections = repo.list().await.expect("list should succeed");
            assert!(
                connections.is_empty(),
                "expected no connections persisted, but got {}",
                connections.len()
            );
        });
}

#[test]
fn test_credentials_validates_empty_api_key() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            // Repo and builder won't be reached because validation happens first
            let repo = Arc::new(InMemoryProviderRepository::default());
            let mock_provider = Arc::new(MockProvider::new(
                ProviderId::new("ollama-primary"),
                HealthStatus::Healthy {
                    provider: ProviderId::new("ollama-primary"),
                    latency_ms: 123,
                },
            ));

            let mc = ManageConnections::new(
                repo,
                Arc::new(NoopRegistry),
                Arc::new(NoopKeyManager),
                Arc::new(NoopProviderBuilder::new(mock_provider)),
            );

            let result = mc
                .test_credentials(TestCredentialsRequest {
                    provider_kind: ProviderKind::Ollama,
                    provider_runtime_id: ProviderId::new("ollama-primary"),
                    auth_type: AuthType::ApiKey,
                    credentials: CredentialsInput::ApiKey {
                        api_key: "".to_string(),
                    },
                    config: config(),
                })
                .await;

            assert!(
                result.is_err(),
                "expected Err for empty API key, got {:?}",
                result
            );
            let err = result.unwrap_err();
            let err_msg = err.to_string();
            assert!(
                err_msg.contains("empty") || err_msg.contains("Empty"),
                "expected error containing 'empty', got '{}'",
                err_msg
            );
        });
}

// ---------------------------------------------------------------------------
// from_health() mapper — covers the 4 HealthStatus variants.
//
// These tests exercise the `valid`/`status`/`warning`/`method` mapping
// end-to-end through the `test_credentials` use case. The mapper is
// private, so we drive it via the public API with a `MockProvider` that
// returns a canned HealthStatus. This is the only way to assert the
// Warning -> {valid: true, status: "warning", warning: Some(...)} path
// without exposing internals.
// ---------------------------------------------------------------------------

fn build_manage_connections(health: HealthStatus) -> ManageConnections {
    let mock_provider = Arc::new(MockProvider::new(ProviderId::new("ollama-primary"), health));
    ManageConnections::new(
        Arc::new(InMemoryProviderRepository::default()),
        Arc::new(NoopRegistry),
        Arc::new(NoopKeyManager),
        Arc::new(NoopProviderBuilder::new(mock_provider)),
    )
}

async fn run_test_credentials() -> rook_usecases::manage_connections::TestConnectionResult {
    let mc = build_manage_connections(HealthStatus::Healthy {
        provider: ProviderId::new("ollama-primary"),
        latency_ms: 0,
    });
    mc.test_credentials(TestCredentialsRequest {
        provider_kind: ProviderKind::Ollama,
        provider_runtime_id: ProviderId::new("ollama-primary"),
        auth_type: AuthType::ApiKey,
        credentials: CredentialsInput::ApiKey {
            api_key: "sk-test".to_string(),
        },
        config: config(),
    })
    .await
    .expect("test_credentials should succeed")
}

#[test]
fn mapper_healthy_returns_valid_true_status_ok_no_warning() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let res = run_test_credentials().await;
            assert!(res.valid, "expected valid=true for Healthy");
            assert_eq!(res.status, "ok");
            assert!(res.warning.is_none(), "warning should be None for Healthy");
            assert!(res.error.is_none());
            assert!(
                res.method.is_some(),
                "method should be Some for a probed Healthy response"
            );
        });
}

#[test]
fn mapper_warning_returns_valid_true_status_warning_with_warning_text() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            // We can't use run_test_credentials() because it bakes in
            // a Healthy status. Build the usecase with a Warning
            // status directly and run test_credentials through it.
            let mc = build_manage_connections(HealthStatus::Warning {
                provider: ProviderId::new("ollama-primary"),
                latency_ms: 17,
                reason: "Rate limited, but credentials are valid".to_string(),
            });
            let res = mc
                .test_credentials(TestCredentialsRequest {
                    provider_kind: ProviderKind::Ollama,
                    provider_runtime_id: ProviderId::new("ollama-primary"),
                    auth_type: AuthType::ApiKey,
                    credentials: CredentialsInput::ApiKey {
                        api_key: "sk-test".to_string(),
                    },
                    config: config(),
                })
                .await
                .expect("test_credentials should succeed");

            assert!(res.valid, "expected valid=true for Warning");
            assert_eq!(res.status, "warning");
            assert_eq!(res.latency_ms, Some(17));
            assert_eq!(
                res.warning.as_deref(),
                Some("Rate limited, but credentials are valid")
            );
            assert!(res.error.is_none(), "error should be None for Warning");
        });
}

#[test]
fn mapper_unhealthy_returns_valid_false_status_unhealthy_with_error() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let mc = build_manage_connections(HealthStatus::Unhealthy {
                provider: ProviderId::new("ollama-primary"),
                latency_ms: Some(30),
                error: "auth rejected: HTTP 401 — invalid key".to_string(),
            });
            let res = mc
                .test_credentials(TestCredentialsRequest {
                    provider_kind: ProviderKind::Ollama,
                    provider_runtime_id: ProviderId::new("ollama-primary"),
                    auth_type: AuthType::ApiKey,
                    credentials: CredentialsInput::ApiKey {
                        api_key: "sk-test".to_string(),
                    },
                    config: config(),
                })
                .await
                .expect("test_credentials should succeed");

            assert!(!res.valid, "expected valid=false for Unhealthy");
            assert_eq!(res.status, "unhealthy");
            assert_eq!(res.latency_ms, Some(30));
            assert_eq!(
                res.error.as_deref(),
                Some("auth rejected: HTTP 401 — invalid key")
            );
            assert!(
                res.warning.is_none(),
                "warning should be None for Unhealthy"
            );
        });
}

#[test]
fn mapper_unknown_returns_valid_true_status_unknown_with_reason_as_warning() {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(async {
            let mc = build_manage_connections(HealthStatus::Unknown {
                provider: ProviderId::new("anthropic-primary"),
                reason: "health_check_not_supported".to_string(),
            });
            let res = mc
                .test_credentials(TestCredentialsRequest {
                    provider_kind: ProviderKind::Anthropic,
                    provider_runtime_id: ProviderId::new("anthropic-primary"),
                    auth_type: AuthType::ApiKey,
                    credentials: CredentialsInput::ApiKey {
                        api_key: "sk-test".to_string(),
                    },
                    config: config(),
                })
                .await
                .expect("test_credentials should succeed");

            assert!(res.valid, "expected valid=true for Unknown (Save enabled)");
            assert_eq!(res.status, "unknown");
            // Unknown carries the reason text as a `warning` so the
            // dashboard can optionally display "no probe available",
            // but it's still a valid connection.
            assert_eq!(res.warning.as_deref(), Some("health_check_not_supported"));
            assert!(res.error.is_none(), "error should be None for Unknown");
            assert_eq!(
                res.method.as_deref(),
                Some("not_supported"),
                "method should be 'not_supported' for Unknown"
            );
        });
}
