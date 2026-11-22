//! Regression coverage for the Ollama endpoint-grant workflow.
//!
//! An Ollama connection that targets a custom destination (another host, port,
//! or URL) must only reach the routing registry when the operator granted that
//! exact origin for that connection through `EndpointPolicyConfig`.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use rook_core::{
    ApiFormat, CompletionRequest, CompletionResponse, CortexError, CortexResult,
    CredentialEncryptionError, EndpointGrant, EndpointPolicyConfig, HealthStatus, KeyManager,
    ModelId, ProviderConnection, ProviderPort, ProviderRepositoryPort, RegistryError,
    RepositoryError, StreamChunk,
};
use rook_usecases::manage_connections::{
    CreateConnectionRequest, CredentialsInput, ManageConnections, ManageConnectionsError,
    ProviderBuildInput, ProviderBuilderPort, UpdateConnectionRequest,
};
use shared_kernel::{ConnectionId, ProviderId};

const CUSTOM_URL: &str = "http://10.0.0.1:11434";
const DEFAULT_URL: &str = "http://localhost:11434";

// ---------------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------------

#[derive(Default)]
struct InMemoryRepo {
    connections: Mutex<Vec<ProviderConnection>>,
}

#[async_trait]
impl ProviderRepositoryPort for InMemoryRepo {
    async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
        Ok(self.connections.lock().expect("repo lock").clone())
    }

    async fn list_routing_state(
        &self,
    ) -> Result<Vec<rook_core::ProviderConnectionRoutingState>, RepositoryError> {
        Ok(self
            .connections
            .lock()
            .expect("repo lock")
            .iter()
            .map(|connection| rook_core::ProviderConnectionRoutingState {
                id: connection.id,
                provider_runtime_id: connection.provider_runtime_id.clone(),
                adapter_revision: connection.adapter_revision,
                priority: connection.priority,
                created_at: connection.created_at,
                is_active: connection.is_active,
                auth_expired: matches!(
                    &connection.test_status,
                    rook_core::TestStatus::Expired { .. }
                ),
            })
            .collect())
    }

    async fn find(&self, id: &ConnectionId) -> Result<Option<ProviderConnection>, RepositoryError> {
        Ok(self
            .connections
            .lock()
            .expect("repo lock")
            .iter()
            .find(|c| &c.id == id)
            .cloned())
    }

    async fn find_connection_id_by_runtime(
        &self,
        provider: &ProviderId,
    ) -> Result<Option<ConnectionId>, RepositoryError> {
        Ok(self
            .connections
            .lock()
            .expect("repo lock")
            .iter()
            .find(|c| &c.provider_runtime_id == provider)
            .map(|c| c.id))
    }

    async fn create(&self, conn: &ProviderConnection) -> Result<(), RepositoryError> {
        self.connections
            .lock()
            .expect("repo lock")
            .push(conn.clone());
        Ok(())
    }

    async fn update(
        &self,
        conn: &ProviderConnection,
        expected_updated_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        let mut guard = self.connections.lock().expect("repo lock");
        let existing = guard
            .iter_mut()
            .find(|c| c.id == conn.id)
            .ok_or(RepositoryError::NotFound(conn.id))?;
        if existing.updated_at != expected_updated_at {
            return Err(RepositoryError::StaleUpdate);
        }
        *existing = conn.clone();
        Ok(())
    }

    async fn delete(&self, id: &ConnectionId) -> Result<(), RepositoryError> {
        let mut guard = self.connections.lock().expect("repo lock");
        let before = guard.len();
        guard.retain(|c| &c.id != id);
        if guard.len() == before {
            return Err(RepositoryError::NotFound(*id));
        }
        Ok(())
    }
}

#[derive(Default)]
struct RecordingRegistry {
    providers: Mutex<Vec<Arc<dyn ProviderPort>>>,
}

impl RecordingRegistry {
    fn ids(&self) -> Vec<ProviderId> {
        self.providers
            .lock()
            .expect("registry lock")
            .iter()
            .map(|p| p.id().clone())
            .collect()
    }
}

impl rook_core::ProviderRegistryPort for RecordingRegistry {
    fn providers(&self) -> Vec<ProviderId> {
        self.ids()
    }

    fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
        self.providers
            .lock()
            .expect("registry lock")
            .iter()
            .find(|p| p.id() == id)
            .cloned()
    }

    fn replace_all(&self, providers: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError> {
        *self.providers.lock().expect("registry lock") = providers;
        Ok(())
    }

    fn upsert(&self, provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError> {
        let mut guard = self.providers.lock().expect("registry lock");
        guard.retain(|p| p.id() != provider.id());
        guard.push(provider);
        Ok(())
    }

    fn remove(&self, id: &ProviderId) -> Result<(), RegistryError> {
        self.providers
            .lock()
            .expect("registry lock")
            .retain(|p| p.id() != id);
        Ok(())
    }
}

struct PlainKeyManager;

impl KeyManager for PlainKeyManager {
    fn encrypt(&self, plaintext: &str) -> Result<String, CredentialEncryptionError> {
        Ok(format!("enc:{plaintext}"))
    }

    fn decrypt(&self, ciphertext: &str) -> Result<String, CredentialEncryptionError> {
        Ok(ciphertext
            .strip_prefix("enc:")
            .unwrap_or(ciphertext)
            .to_string())
    }
}

struct StubProvider {
    id: ProviderId,
    models: Vec<ModelId>,
}

#[async_trait]
impl ProviderPort for StubProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn supported_models(&self) -> &[ModelId] {
        &self.models
    }

    fn api_format(&self) -> ApiFormat {
        ApiFormat::OpenAI
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy {
            provider: self.id.clone(),
            latency_ms: 1,
        }
    }

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.id.clone(),
            model: req.model.clone(),
            content: "stub".into(),
            content_blocks: vec![],
            thinking: None,
            tool_calls: vec![],
            finish_reason: None,
            usage: rook_core::TokenUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 0,
            cache_hit: None,
        })
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<Pin<Box<dyn Stream<Item = Result<StreamChunk, CortexError>> + Send>>> {
        Err(CortexError::provider("stub provider"))
    }
}

#[derive(Default)]
struct RecordingBuilder {
    inputs: Mutex<Vec<ProviderBuildInput>>,
}

impl RecordingBuilder {
    fn inputs(&self) -> Vec<ProviderBuildInput> {
        self.inputs.lock().expect("builder lock").clone()
    }
}

#[async_trait]
impl ProviderBuilderPort for RecordingBuilder {
    async fn build(
        &self,
        input: ProviderBuildInput,
    ) -> Result<Arc<dyn ProviderPort>, ManageConnectionsError> {
        self.inputs
            .lock()
            .expect("builder lock")
            .push(input.clone());
        Ok(Arc::new(StubProvider {
            id: ProviderId::new(input.connection_id.to_string()),
            models: input.supported_models.clone(),
        }))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ollama_config(base_url: &str) -> rook_core::ConnectionConfig {
    rook_core::ConnectionConfig {
        max_concurrent: 1,
        quota_window_thresholds: rook_core::QuotaWindowThresholds {
            warning: 0.7,
            error: 0.9,
        },
        default_model: None,
        base_url: Some(base_url.to_string()),
        api_format: None,
        supported_models: Vec::new(),
    }
}

fn create_request(base_url: &str, is_active: bool) -> CreateConnectionRequest {
    CreateConnectionRequest {
        provider: rook_core::descriptor_by_id("ollama").expect("ollama descriptor"),
        provider_runtime_id: ProviderId::new("ollama-local"),
        auth_type: rook_core::AuthType::ApiKey,
        name: "ollama-local".to_string(),
        priority: 10,
        is_active,
        credentials: CredentialsInput::ApiKey {
            api_key: "local-key".to_string(),
        },
        config: ollama_config(base_url),
    }
}

fn usecase(
    policy: EndpointPolicyConfig,
    repo: Arc<InMemoryRepo>,
    registry: Arc<RecordingRegistry>,
    builder: Arc<RecordingBuilder>,
) -> ManageConnections {
    ManageConnections::new(repo, registry, Arc::new(PlainKeyManager), builder)
        .with_endpoint_policy(policy)
}

fn grant_for(connection_id: &str, origin: &str) -> EndpointPolicyConfig {
    EndpointPolicyConfig {
        grants: vec![EndpointGrant {
            connection_id: connection_id.to_string(),
            allowed_origins: vec![origin.to_string()],
        }],
    }
}

fn activation(connection: &ProviderConnection, base_url: &str) -> UpdateConnectionRequest {
    UpdateConnectionRequest {
        expected_updated_at: connection.updated_at,
        provider: None,
        provider_runtime_id: None,
        auth_type: None,
        name: None,
        priority: None,
        is_active: Some(true),
        credentials: None,
        config: Some(ollama_config(base_url)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn custom_ollama_origin_activates_only_with_the_exact_connection_grant() {
    let repo = Arc::new(InMemoryRepo::default());
    let registry = Arc::new(RecordingRegistry::default());
    let builder = Arc::new(RecordingBuilder::default());
    let mc = usecase(
        EndpointPolicyConfig::default(),
        repo.clone(),
        registry.clone(),
        builder.clone(),
    );

    let connection = mc
        .create(create_request(DEFAULT_URL, false))
        .await
        .expect("a built-in Ollama origin needs no grant");
    assert!(!connection.is_active);

    // Without a grant the custom destination is rejected and the connection
    // stays inactive and unpublished.
    let error = mc
        .update(&connection.id, activation(&connection, CUSTOM_URL))
        .await
        .expect_err("an ungranted custom Ollama origin must be rejected");
    assert!(
        matches!(
            error,
            ManageConnectionsError::Validation(rook_core::ValidationError::InvalidBaseUrl)
        ),
        "{error:?}"
    );
    assert!(registry.ids().is_empty());
    assert!(builder.inputs().is_empty());

    // The operator grant for this exact connection unlocks the custom origin.
    let granted = usecase(
        grant_for(&connection.id.to_string(), CUSTOM_URL),
        repo.clone(),
        registry.clone(),
        builder.clone(),
    );
    let activated = granted
        .update(&connection.id, activation(&connection, CUSTOM_URL))
        .await
        .expect("the exact connection grant permits the custom Ollama origin");
    assert!(activated.is_active);
    assert_eq!(activated.config.base_url.as_deref(), Some(CUSTOM_URL));
    assert_eq!(
        registry.ids(),
        vec![ProviderId::new(connection.id.to_string())],
        "the granted connection must reach the routing registry"
    );
    let inputs = builder.inputs();
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0].endpoint_policy.allowed_origins,
        vec![CUSTOM_URL.to_string()],
        "the builder receives the connection-specific grant"
    );
}

#[tokio::test]
async fn a_grant_for_another_connection_does_not_unlock_a_custom_ollama_origin() {
    let repo = Arc::new(InMemoryRepo::default());
    let registry = Arc::new(RecordingRegistry::default());
    let builder = Arc::new(RecordingBuilder::default());
    let mc = usecase(
        EndpointPolicyConfig::default(),
        repo.clone(),
        registry.clone(),
        builder.clone(),
    );

    let connection = mc
        .create(create_request(DEFAULT_URL, false))
        .await
        .expect("a built-in Ollama origin needs no grant");

    // The grant targets another connection: it must not unlock this one.
    let other_connection_id = "6ba7b810-9dad-41d1-80b4-00c04fd430c8";
    let granted = usecase(
        grant_for(other_connection_id, CUSTOM_URL),
        repo.clone(),
        registry.clone(),
        builder.clone(),
    );
    let error = granted
        .update(&connection.id, activation(&connection, CUSTOM_URL))
        .await
        .expect_err("a foreign connection grant must be rejected");
    assert!(
        matches!(
            error,
            ManageConnectionsError::Validation(rook_core::ValidationError::InvalidBaseUrl)
        ),
        "{error:?}"
    );
    assert!(registry.ids().is_empty());
    assert!(builder.inputs().is_empty());
    assert!(
        !repo
            .find(&connection.id)
            .await
            .expect("stored connection")
            .expect("connection exists")
            .is_active
    );
}

#[tokio::test]
async fn startup_refresh_skips_an_ungranted_custom_ollama_connection() {
    let repo = Arc::new(InMemoryRepo::default());
    let registry = Arc::new(RecordingRegistry::default());
    let builder = Arc::new(RecordingBuilder::default());
    let mc = usecase(
        EndpointPolicyConfig::default(),
        repo.clone(),
        registry.clone(),
        builder.clone(),
    );

    let connection = mc
        .create(create_request(DEFAULT_URL, false))
        .await
        .expect("a built-in Ollama origin needs no grant");

    // Seed the persisted state of a connection that already points at a custom
    // Ollama destination and is active, as an earlier deployment stored it.
    let mut stored = connection.clone();
    stored.config.base_url = Some(CUSTOM_URL.to_string());
    stored.is_active = true;
    repo.update(&stored, connection.updated_at)
        .await
        .expect("seed the stored connection");

    let ungranted = usecase(
        EndpointPolicyConfig::default(),
        repo.clone(),
        registry.clone(),
        builder.clone(),
    );
    ungranted
        .refresh_registry()
        .await
        .expect("an ungranted connection is skipped, not fatal");
    assert!(
        registry.ids().is_empty(),
        "an ungranted custom Ollama connection must not be published"
    );
    assert!(builder.inputs().is_empty());

    let granted = usecase(
        grant_for(&connection.id.to_string(), CUSTOM_URL),
        repo.clone(),
        registry.clone(),
        builder.clone(),
    );
    granted
        .refresh_registry()
        .await
        .expect("the granted connection is published");
    assert_eq!(
        registry.ids(),
        vec![ProviderId::new(connection.id.to_string())]
    );
    let inputs = builder.inputs();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].base_url.as_deref(), Some(CUSTOM_URL));
    assert_eq!(
        inputs[0].endpoint_policy.allowed_origins,
        vec![CUSTOM_URL.to_string()]
    );
}

#[tokio::test]
async fn built_in_ollama_origins_activate_without_any_grant() {
    let repo = Arc::new(InMemoryRepo::default());
    let registry = Arc::new(RecordingRegistry::default());
    let builder = Arc::new(RecordingBuilder::default());
    let mc = usecase(
        EndpointPolicyConfig::default(),
        repo.clone(),
        registry.clone(),
        builder.clone(),
    );

    for origin in [
        "http://localhost:11434",
        "http://127.0.0.1:11434",
        "http://[::1]:11434",
        "https://localhost:11434",
        "https://127.0.0.1:11434",
        "https://[::1]:11434",
        "https://ollama.com",
    ] {
        let connection = mc
            .create(create_request(origin, false))
            .await
            .expect("a built-in Ollama origin needs no grant");
        let activated = mc
            .update(&connection.id, activation(&connection, origin))
            .await
            .unwrap_or_else(|error| panic!("{origin} must activate without a grant: {error:?}"));
        assert!(activated.is_active, "{origin}");
    }
}
