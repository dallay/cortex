//! Lifecycle coverage for generic OpenAI-compatible connections.
//!
//! Covers the acceptance criteria of DALLAY-682 that live above the wire:
//! independent connections, per-connection endpoint/credentials/models,
//! and registry refresh without restarting Rook.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures::Stream;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::{
    ApiFormat, CompletionRequest, CompletionResponse, CortexError, CortexResult,
    CredentialEncryptionError, EndpointGrant, EndpointPolicyConfig, HealthStatus, KeyManager,
    ModelId, ProviderConnection, ProviderKind, ProviderPort, ProviderRepositoryPort, RegistryError,
    RepositoryError, StreamChunk,
};
use rook_usecases::manage_connections::{
    CreateConnectionRequest, CredentialsInput, ManageConnections, ManageConnectionsError,
    ProviderBuildInput, ProviderBuilderPort, UpdateConnectionRequest,
};
use shared_kernel::{ConnectionId, ProviderId};

const RUNTIME_A: &str = "compatible-runtime-a";
const RUNTIME_B: &str = "compatible-runtime-b";

// ---------------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------------

#[derive(Default)]
struct InMemoryRepo {
    connections: Mutex<Vec<ProviderConnection>>,
}

#[async_trait]
impl rook_core::ProviderRepositoryPort for InMemoryRepo {
    async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
        Ok(self.connections.lock().expect("repo lock").clone())
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

    async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        Err(CortexError::provider("stub provider"))
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

fn compatible_config(base_url: &str, models: &[&str]) -> rook_core::ConnectionConfig {
    rook_core::ConnectionConfig {
        max_concurrent: 4,
        quota_window_thresholds: rook_core::QuotaWindowThresholds {
            warning: 0.8,
            error: 0.95,
        },
        default_model: None,
        base_url: Some(base_url.to_string()),
        supported_models: models.iter().map(|m| ModelId::new(*m)).collect(),
        api_format: Some(ApiFormat::OpenAI),
    }
}

fn create_request(
    runtime: &str,
    name: &str,
    base_url: &str,
    api_key: &str,
    models: &[&str],
) -> CreateConnectionRequest {
    CreateConnectionRequest {
        provider_kind: ProviderKind::OpenAiCompatible,
        provider_runtime_id: ProviderId::new(runtime),
        auth_type: rook_core::AuthType::ApiKey,
        name: name.to_string(),
        priority: 10,
        is_active: true,
        credentials: CredentialsInput::ApiKey {
            api_key: api_key.to_string(),
        },
        config: compatible_config(base_url, models),
    }
}

fn usecase(
    policy: EndpointPolicyConfig,
) -> (
    ManageConnections,
    Arc<InMemoryRepo>,
    Arc<RecordingRegistry>,
    Arc<RecordingBuilder>,
) {
    let repo = Arc::new(InMemoryRepo::default());
    let registry = Arc::new(RecordingRegistry::default());
    let builder = Arc::new(RecordingBuilder::default());
    let mc = ManageConnections::new(
        repo.clone(),
        registry.clone(),
        Arc::new(PlainKeyManager),
        builder.clone(),
    )
    .with_endpoint_policy(policy);
    (mc, repo, registry, builder)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_compatible_connections_keep_independent_endpoints_credentials_and_models() {
    let (mc, repo, registry, builder) = usecase(EndpointPolicyConfig::default());

    let mut first_request = create_request(
        RUNTIME_A,
        "compat-a",
        "https://upstream-a.example/v1",
        "key-a",
        &["model-a"],
    );
    first_request.is_active = false;
    let first = mc
        .create(first_request)
        .await
        .expect("first connection is stored disabled");
    let mut second_request = create_request(
        RUNTIME_B,
        "compat-b",
        "https://upstream-b.example/v1",
        "key-b",
        &["model-b", "model-b-mini"],
    );
    second_request.is_active = false;
    let second = mc
        .create(second_request)
        .await
        .expect("second connection is stored disabled");
    let mc = ManageConnections::new(
        repo.clone(),
        registry.clone(),
        Arc::new(PlainKeyManager),
        builder.clone(),
    )
    .with_endpoint_policy(EndpointPolicyConfig {
        grants: vec![
            EndpointGrant {
                connection_id: first.id.to_string(),
                allowed_origins: vec!["https://upstream-a.example".into()],
            },
            EndpointGrant {
                connection_id: second.id.to_string(),
                allowed_origins: vec!["https://upstream-b.example".into()],
            },
        ],
    });
    let first = mc
        .update(
            &first.id,
            UpdateConnectionRequest {
                expected_updated_at: first.updated_at,
                provider_kind: None,
                provider_runtime_id: None,
                auth_type: None,
                name: None,
                priority: None,
                is_active: Some(true),
                credentials: None,
                config: None,
            },
        )
        .await
        .expect("exact grant activates first connection");
    let second = mc
        .update(
            &second.id,
            UpdateConnectionRequest {
                expected_updated_at: second.updated_at,
                provider_kind: None,
                provider_runtime_id: None,
                auth_type: None,
                name: None,
                priority: None,
                is_active: Some(true),
                credentials: None,
                config: None,
            },
        )
        .await
        .expect("exact grant activates second connection");

    assert_ne!(first.id, second.id, "connections must be independent");
    assert_ne!(first.provider_runtime_id, second.provider_runtime_id);

    let stored = repo.list().await.expect("stored connections");
    assert_eq!(stored.len(), 2);
    let stored_a = stored.iter().find(|c| c.id == first.id).expect("a");
    let stored_b = stored.iter().find(|c| c.id == second.id).expect("b");
    assert_eq!(
        stored_a.config.base_url.as_deref(),
        Some("https://upstream-a.example/v1")
    );
    assert_eq!(
        stored_b.config.base_url.as_deref(),
        Some("https://upstream-b.example/v1")
    );
    assert_eq!(
        stored_a.config.supported_models,
        vec![ModelId::new("model-a")]
    );
    assert_eq!(
        stored_b.config.supported_models,
        vec![ModelId::new("model-b"), ModelId::new("model-b-mini")]
    );

    let inputs = builder.inputs();
    let input_a = inputs
        .iter()
        .rev()
        .find(|i| i.connection_id == first.id)
        .expect("a");
    let input_b = inputs
        .iter()
        .rev()
        .find(|i| i.connection_id == second.id)
        .expect("b");
    assert_eq!(
        input_a.base_url.as_deref(),
        Some("https://upstream-a.example/v1")
    );
    assert_eq!(
        input_b.base_url.as_deref(),
        Some("https://upstream-b.example/v1")
    );
    assert_ne!(input_a.decrypted_credentials, input_b.decrypted_credentials);

    let registry_ids = registry.ids();
    assert_eq!(registry_ids.len(), 2, "both connections must be registered");
    assert!(registry_ids.contains(&ProviderId::new(first.id.to_string())));
    assert!(registry_ids.contains(&ProviderId::new(second.id.to_string())));
}

#[tokio::test]
async fn disabling_and_updating_a_compatible_connection_changes_routing_without_restart() {
    let (initial, repo, registry, builder) = usecase(EndpointPolicyConfig::default());
    let mut first_request = create_request(
        RUNTIME_A,
        "compat-a",
        "https://upstream-a.example/v1",
        "key-a",
        &["model-a"],
    );
    first_request.is_active = false;
    let first = initial
        .create(first_request)
        .await
        .expect("first connection stored disabled");
    let mut second_request = create_request(
        RUNTIME_B,
        "compat-b",
        "https://upstream-b.example/v1",
        "key-b",
        &["model-b"],
    );
    second_request.is_active = false;
    let second = initial
        .create(second_request)
        .await
        .expect("second connection stored disabled");
    let mc = ManageConnections::new(
        repo,
        registry.clone(),
        Arc::new(PlainKeyManager),
        builder.clone(),
    )
    .with_endpoint_policy(EndpointPolicyConfig {
        grants: vec![
            EndpointGrant {
                connection_id: first.id.to_string(),
                allowed_origins: vec![
                    "https://upstream-a.example".into(),
                    "https://upstream-a2.example".into(),
                ],
            },
            EndpointGrant {
                connection_id: second.id.to_string(),
                allowed_origins: vec!["https://upstream-b.example".into()],
            },
        ],
    });
    let first = mc
        .update(
            &first.id,
            UpdateConnectionRequest {
                expected_updated_at: first.updated_at,
                provider_kind: None,
                provider_runtime_id: None,
                auth_type: None,
                name: None,
                priority: None,
                is_active: Some(true),
                credentials: None,
                config: None,
            },
        )
        .await
        .expect("exact grant activates first connection");
    let second = mc
        .update(
            &second.id,
            UpdateConnectionRequest {
                expected_updated_at: second.updated_at,
                provider_kind: None,
                provider_runtime_id: None,
                auth_type: None,
                name: None,
                priority: None,
                is_active: Some(true),
                credentials: None,
                config: None,
            },
        )
        .await
        .expect("exact grant activates second connection");

    let registry_ids = registry.ids();
    assert_eq!(registry_ids.len(), 2);

    // Reject a new origin that is not included in the operator grant.
    let first_current = mc
        .get(&first.id)
        .await
        .expect("connection lookup")
        .expect("first connection exists");
    mc.update(
        &first.id,
        UpdateConnectionRequest {
            expected_updated_at: first_current.updated_at,
            provider_kind: None,
            provider_runtime_id: None,
            auth_type: None,
            name: None,
            priority: None,
            is_active: None,
            credentials: None,
            config: Some(compatible_config(
                "https://ungranted.example/v1",
                &["model-x"],
            )),
        },
    )
    .await
    .expect_err("endpoint origin must remain within the connection grant");

    // Update the endpoint to another origin explicitly included in the grant.
    let updated = mc
        .update(
            &first.id,
            UpdateConnectionRequest {
                expected_updated_at: first_current.updated_at,
                provider_kind: None,
                provider_runtime_id: None,
                auth_type: None,
                name: None,
                priority: None,
                is_active: None,
                credentials: None,
                config: Some(compatible_config(
                    "https://upstream-a2.example/v1",
                    &["model-a2"],
                )),
            },
        )
        .await
        .expect("update applies");

    assert_eq!(
        updated.config.base_url.as_deref(),
        Some("https://upstream-a2.example/v1")
    );
    assert_eq!(
        updated.config.supported_models,
        vec![ModelId::new("model-a2")]
    );

    let inputs = builder.inputs();
    let last_for_first = inputs
        .iter()
        .rev()
        .find(|i| i.connection_id == first.id)
        .expect("rebuild for first connection");
    assert_eq!(
        last_for_first.base_url.as_deref(),
        Some("https://upstream-a2.example/v1")
    );
    assert_eq!(
        last_for_first.supported_models,
        vec![ModelId::new("model-a2")]
    );

    // Registry keeps the same runtime ids: no restart is required.
    let registry_ids = registry.ids();
    assert_eq!(registry_ids.len(), 2);
    assert!(registry_ids.contains(&ProviderId::new(first.id.to_string())));
    assert!(registry_ids.contains(&ProviderId::new(second.id.to_string())));

    // Deactivate the second connection: it must leave the active registry.
    let deactivated = mc
        .update(
            &second.id,
            UpdateConnectionRequest {
                expected_updated_at: second.updated_at,
                provider_kind: None,
                provider_runtime_id: None,
                auth_type: None,
                name: None,
                priority: None,
                is_active: Some(false),
                credentials: None,
                config: None,
            },
        )
        .await
        .expect("deactivation applies");
    assert!(!deactivated.is_active);

    let registry_ids = registry.ids();
    assert_eq!(registry_ids.len(), 1);
    assert!(registry_ids.contains(&ProviderId::new(first.id.to_string())));
    assert!(!registry_ids.contains(&ProviderId::new(second.id.to_string())));
}

#[tokio::test]
async fn unsafe_compatible_endpoints_are_rejected_without_exposing_the_api_key() {
    let (mc, repo, _registry, builder) = usecase(EndpointPolicyConfig::default());

    let error = mc
        .create(create_request(
            RUNTIME_A,
            "compat-a",
            "http://upstream-a.example/v1",
            "super-secret-key",
            &["model-a"],
        ))
        .await
        .expect_err("public HTTP is rejected by default");
    assert!(
        !error.to_string().contains("super-secret-key"),
        "error must not leak credentials: {error}"
    );
    assert!(repo.list().await.expect("repo").is_empty());
    assert!(builder.inputs().is_empty());
}

#[tokio::test]
async fn disabled_compatible_connection_can_store_local_endpoint_without_upstream_access() {
    let (mc, repo, registry, builder) = usecase(EndpointPolicyConfig::default());
    let mut request = create_request(
        RUNTIME_A,
        "compat-local",
        "http://127.0.0.1:11434/v1",
        "local-key",
        &["local-model"],
    );
    request.is_active = false;

    let connection = mc
        .create(request)
        .await
        .expect("disabled connection stores its local endpoint without requiring a grant");

    assert!(!connection.is_active);
    assert_eq!(repo.list().await.expect("repo").len(), 1);
    assert!(
        registry.ids().is_empty(),
        "inactive connection is not published"
    );
    assert!(
        builder.inputs().is_empty(),
        "inactive connection causes no upstream build or request"
    );
}

#[tokio::test]
async fn compatible_connection_test_cannot_bypass_the_connection_grant() {
    let (mc, _, _, _) = usecase(EndpointPolicyConfig::default());
    let error = mc
        .test_credentials(rook_usecases::manage_connections::TestCredentialsRequest {
            provider_kind: ProviderKind::OpenAiCompatible,
            provider_runtime_id: ProviderId::new(RUNTIME_A),
            auth_type: rook_core::AuthType::ApiKey,
            credentials: CredentialsInput::ApiKey {
                api_key: "secret".into(),
            },
            config: compatible_config("https://upstream.example/v1", &["model-a"]),
        })
        .await
        .expect_err("temporary credentials test cannot bypass administrator grants");
    assert!(!error.to_string().contains("secret"));
}

#[tokio::test]
async fn loopback_compatible_endpoint_requires_exact_connection_grant_to_activate() {
    let (mc, repo, registry, builder) = usecase(EndpointPolicyConfig::default());
    let mut request = create_request(
        RUNTIME_A,
        "compat-a",
        "http://127.0.0.1:11434/v1",
        "key",
        &["model-a"],
    );
    request.is_active = false;
    let connection = mc
        .create(request)
        .await
        .expect("disabled endpoint is stored");

    let activation = || UpdateConnectionRequest {
        expected_updated_at: connection.updated_at,
        provider_kind: None,
        provider_runtime_id: None,
        auth_type: None,
        name: None,
        priority: None,
        is_active: Some(true),
        credentials: None,
        config: None,
    };
    mc.update(&connection.id, activation())
        .await
        .expect_err("activation without a grant is denied");
    assert!(registry.ids().is_empty());
    assert!(builder.inputs().is_empty());

    let granted =
        ManageConnections::new(repo, registry.clone(), Arc::new(PlainKeyManager), builder)
            .with_endpoint_policy(EndpointPolicyConfig {
                grants: vec![EndpointGrant {
                    connection_id: connection.id.to_string(),
                    allowed_origins: vec!["http://127.0.0.1:11434".to_string()],
                }],
            });
    let activated = granted
        .update(&connection.id, activation())
        .await
        .expect("the exact connection grant permits activation");
    assert!(activated.is_active);
    assert!(!registry.ids().is_empty());
}

#[tokio::test]
async fn compatible_connections_require_declared_models_and_openai_api_format() {
    let (mc, _repo, _registry, _builder) = usecase(EndpointPolicyConfig::default());

    let mut request = create_request(
        RUNTIME_A,
        "compat-a",
        "https://upstream-a.example/v1",
        "key",
        &["model-a"],
    );
    request.config.supported_models.clear();
    mc.create(request)
        .await
        .expect_err("a compatible connection without declared models is rejected");

    let mut request = create_request(
        RUNTIME_A,
        "compat-a",
        "https://upstream-a.example/v1",
        "key",
        &["model-a"],
    );
    request.config.api_format = Some(ApiFormat::Anthropic);
    mc.create(request)
        .await
        .expect_err("a non OpenAI api format is rejected for compatible connections");
}

#[tokio::test]
async fn compatible_connections_reject_oauth_and_missing_base_url() {
    let (mc, _repo, _registry, _builder) = usecase(EndpointPolicyConfig::default());

    let mut request = create_request(
        RUNTIME_A,
        "compat-a",
        "https://upstream-a.example/v1",
        "key",
        &["model-a"],
    );
    request.auth_type = rook_core::AuthType::OAuth;
    request.credentials = CredentialsInput::OAuth {
        email: "operator@example.com".into(),
        access_token: "access-token".into(),
        refresh_token: "refresh-token".into(),
        expires_at: i64::MAX,
        scope: "scope".into(),
        id_token: String::new(),
        project_id: String::new(),
        oauth_client_id: String::new(),
        oauth_client_secret: None,
        oauth_tenant_id: None,
    };
    mc.create(request)
        .await
        .expect_err("OAuth authentication is rejected for compatible connections");

    let mut request = create_request(
        RUNTIME_A,
        "compat-a",
        "https://upstream-a.example/v1",
        "key",
        &["model-a"],
    );
    request.config.base_url = None;
    mc.create(request)
        .await
        .expect_err("a compatible connection without base_url is rejected");
}
