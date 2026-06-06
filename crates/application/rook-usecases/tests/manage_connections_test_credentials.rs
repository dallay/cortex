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
            assert_eq!(res.ok, Some(true), "expected ok=Some(true)");
            assert_eq!(
                res.status, "active",
                "expected status='active', got '{}'",
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
