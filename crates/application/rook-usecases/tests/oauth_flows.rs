use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::{
    ConnectionConfig, CredentialEncryptionError, KeyManager, ProviderConnection, ProviderId,
    ProviderPort, ProviderRegistryPort, ProviderRepositoryPort, QuotaWindowThresholds,
    RegistryError, RepositoryError,
};
use rook_usecases::{
    manage_connections::{OAuthFlowType, StartOAuthFlowRequest},
    ManageConnections, ProviderBuildInput, ProviderBuilderPort,
};
use shared_kernel::ConnectionId;

struct EmptyRepository;

#[async_trait]
impl ProviderRepositoryPort for EmptyRepository {
    async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
        Ok(Vec::new())
    }
    async fn find(&self, _: &ConnectionId) -> Result<Option<ProviderConnection>, RepositoryError> {
        Ok(None)
    }
    async fn create(&self, _: &ProviderConnection) -> Result<(), RepositoryError> {
        Ok(())
    }
    async fn update(
        &self,
        _: &ProviderConnection,
        _: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }
    async fn delete(&self, _: &ConnectionId) -> Result<(), RepositoryError> {
        Ok(())
    }
}

struct EmptyRegistry;

impl ProviderRegistryPort for EmptyRegistry {
    fn providers(&self) -> Vec<ProviderId> {
        Vec::new()
    }
    fn get(&self, _: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
        None
    }
    fn replace_all(&self, _: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError> {
        Ok(())
    }
    fn upsert(&self, _: Arc<dyn ProviderPort>) -> Result<(), RegistryError> {
        Ok(())
    }
    fn remove(&self, _: &ProviderId) -> Result<(), RegistryError> {
        Ok(())
    }
}

struct PassThroughKeyManager;

impl KeyManager for PassThroughKeyManager {
    fn encrypt(&self, plaintext: &str) -> Result<String, CredentialEncryptionError> {
        Ok(plaintext.to_string())
    }
    fn decrypt(&self, ciphertext: &str) -> Result<String, CredentialEncryptionError> {
        Ok(ciphertext.to_string())
    }
}

struct UnusedBuilder;

#[async_trait]
impl ProviderBuilderPort for UnusedBuilder {
    async fn build(
        &self,
        _: ProviderBuildInput,
    ) -> Result<Arc<dyn ProviderPort>, rook_usecases::ManageConnectionsError> {
        Err(rook_usecases::ManageConnectionsError::OAuth(
            "builder must not be called by OAuth flow start".to_string(),
        ))
    }
}

fn manager() -> ManageConnections {
    ManageConnections::new(
        Arc::new(EmptyRepository),
        Arc::new(EmptyRegistry),
        Arc::new(PassThroughKeyManager),
        Arc::new(UnusedBuilder),
    )
}

fn request(flow_type: OAuthFlowType) -> StartOAuthFlowRequest {
    StartOAuthFlowRequest {
        flow_type,
        connection_id: None,
        provider_runtime_id: ProviderId::new("azure-prod"),
        name: "Azure production".to_string(),
        priority: 10,
        is_active: true,
        config: ConnectionConfig {
            max_concurrent: 10,
            quota_window_thresholds: QuotaWindowThresholds {
                warning: 0.8,
                error: 0.95,
            },
            default_model: Some(shared_kernel::ModelId::new("gpt-prod")),
            base_url: Some("https://resource.openai.azure.com".to_string()),
        },
        client_id: "client-id".to_string(),
        client_secret: Some("must-not-appear-in-url".to_string()),
        tenant_id: Some("tenant-guid".to_string()),
        redirect_uri: "https://rook.example.com/api/oauth/callback".to_string(),
        project_id: None,
    }
}

#[tokio::test]
async fn azure_authorization_code_uses_pkce_and_keeps_secret_out_of_url() {
    let manager = manager();
    let started = manager
        .start_oauth_flow(
            rook_core::ProviderKind::AzureOpenAI,
            request(OAuthFlowType::AuthorizationCode),
        )
        .await
        .expect("flow starts without contacting identity provider");
    let url = reqwest::Url::parse(started.authorization_url.as_deref().unwrap()).unwrap();
    let query: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();

    assert_eq!(url.host_str(), Some("login.microsoftonline.com"));
    assert_eq!(
        query.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert_eq!(
        query.get("redirect_uri").map(String::as_str),
        Some("https://rook.example.com/api/oauth/callback")
    );
    assert!(!url.as_str().contains("must-not-appear-in-url"));
    assert!(!query.get("code_challenge").unwrap().is_empty());
}

#[tokio::test]
async fn gemini_authorization_requests_its_documented_scope_and_offline_access() {
    let manager = manager();
    let mut request = request(OAuthFlowType::AuthorizationCode);
    request.project_id = Some("google-project".to_string());
    request.config.base_url = None;
    let started = manager
        .start_oauth_flow(rook_core::ProviderKind::Gemini, request)
        .await
        .expect("Gemini authorization flow starts");
    let url = reqwest::Url::parse(started.authorization_url.as_deref().unwrap()).unwrap();
    let query: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();

    assert_eq!(url.host_str(), Some("accounts.google.com"));
    assert_eq!(
        query.get("access_type").map(String::as_str),
        Some("offline")
    );
    assert!(query
        .get("scope")
        .is_some_and(|scope| scope.contains("generative-language.retriever")));
    assert_eq!(
        query.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
}

#[tokio::test]
async fn authorization_rejects_redirects_outside_the_rook_callback_path() {
    let manager = manager();
    let mut request = request(OAuthFlowType::AuthorizationCode);
    request.redirect_uri = "https://attacker.example/collect".to_string();

    assert!(manager
        .start_oauth_flow(rook_core::ProviderKind::AzureOpenAI, request)
        .await
        .is_err());
}

#[tokio::test]
async fn gemini_does_not_advertise_device_code_flow() {
    let manager = manager();
    let mut request = request(OAuthFlowType::DeviceCode);
    request.project_id = Some("google-project".to_string());

    assert!(manager
        .start_oauth_flow(rook_core::ProviderKind::Gemini, request)
        .await
        .is_err());
}

#[tokio::test]
async fn azure_requires_a_deployment_and_azure_resource_endpoint() {
    let manager = manager();
    let mut request = request(OAuthFlowType::AuthorizationCode);
    request.config.base_url = Some("https://attacker.example".to_string());

    assert!(manager
        .start_oauth_flow(rook_core::ProviderKind::AzureOpenAI, request)
        .await
        .is_err());
}

#[tokio::test]
async fn unsupported_provider_is_rejected() {
    let manager = manager();
    let err = manager
        .start_oauth_flow(
            rook_core::ProviderKind::OpenAI,
            request(OAuthFlowType::AuthorizationCode),
        )
        .await
        .expect_err("OpenAI has no OAuth");
    assert!(err.to_string().contains("not supported"));
}

#[tokio::test]
async fn missing_client_id_is_rejected() {
    let manager = manager();
    let mut req = request(OAuthFlowType::AuthorizationCode);
    req.client_id = "   ".to_string();
    let err = manager
        .start_oauth_flow(rook_core::ProviderKind::AzureOpenAI, req)
        .await
        .expect_err("clientId required");
    assert!(err.to_string().contains("clientId"));
}

#[tokio::test]
async fn empty_name_is_rejected() {
    let manager = manager();
    let mut req = request(OAuthFlowType::AuthorizationCode);
    req.name = "  ".to_string();
    assert!(manager
        .start_oauth_flow(rook_core::ProviderKind::AzureOpenAI, req)
        .await
        .is_err());
}

#[tokio::test]
async fn zero_priority_is_rejected() {
    let manager = manager();
    let mut req = request(OAuthFlowType::AuthorizationCode);
    req.priority = 0;
    assert!(manager
        .start_oauth_flow(rook_core::ProviderKind::AzureOpenAI, req)
        .await
        .is_err());
}

#[tokio::test]
async fn invalid_threshold_order_is_rejected() {
    let manager = manager();
    let mut req = request(OAuthFlowType::AuthorizationCode);
    req.config.quota_window_thresholds.warning = 0.9;
    req.config.quota_window_thresholds.error = 0.8;
    assert!(manager
        .start_oauth_flow(rook_core::ProviderKind::AzureOpenAI, req)
        .await
        .is_err());
}

#[tokio::test]
async fn gemini_requires_project_id() {
    let manager = manager();
    let mut req = request(OAuthFlowType::AuthorizationCode);
    req.project_id = None;
    let err = manager
        .start_oauth_flow(rook_core::ProviderKind::Gemini, req)
        .await
        .expect_err("project required");
    assert!(err.to_string().contains("projectId"));
}

#[tokio::test]
async fn tenant_with_invalid_characters_is_rejected() {
    let manager = manager();
    let mut req = request(OAuthFlowType::AuthorizationCode);
    req.tenant_id = Some("bad tenant!".to_string());
    assert!(manager
        .start_oauth_flow(rook_core::ProviderKind::AzureOpenAI, req)
        .await
        .is_err());
}

#[tokio::test]
async fn unknown_connection_id_is_rejected() {
    let manager = manager();
    let mut req = request(OAuthFlowType::AuthorizationCode);
    req.connection_id = Some(ConnectionId::new());
    assert!(manager
        .start_oauth_flow(rook_core::ProviderKind::AzureOpenAI, req)
        .await
        .is_err());
}

#[tokio::test]
async fn flow_status_unknown_id_is_rejected() {
    let manager = manager();
    assert!(manager.oauth_flow_status("nope").await.is_err());
}

#[tokio::test]
async fn complete_with_invalid_state_is_rejected() {
    let manager = manager();
    assert!(manager
        .complete_oauth_authorization("bad-state", "code")
        .await
        .is_err());
}

#[tokio::test]
async fn cancel_with_unknown_state_is_a_noop() {
    let manager = manager();
    manager.cancel_oauth_authorization("unknown-state").await;
    assert!(manager.oauth_flow_status("unknown-flow").await.is_err());
}

#[tokio::test]
async fn refresh_unknown_connection_is_rejected() {
    let manager = manager();
    assert!(manager
        .refresh_oauth_connection(&ConnectionId::new())
        .await
        .is_err());
}

#[tokio::test]
async fn disconnect_unknown_connection_is_rejected() {
    let manager = manager();
    assert!(manager
        .disconnect_oauth_connection(&ConnectionId::new())
        .await
        .is_err());
}

#[tokio::test]
async fn authorization_flow_status_is_pending_then_cancellable() {
    let manager = manager();
    let started = manager
        .start_oauth_flow(
            rook_core::ProviderKind::AzureOpenAI,
            request(OAuthFlowType::AuthorizationCode),
        )
        .await
        .expect("flow starts");
    let status = manager
        .oauth_flow_status(&started.flow_id)
        .await
        .expect("status");
    assert_eq!(status.status, "pending");
    assert!(status.connection_id.is_none());
}

#[tokio::test]
async fn device_code_start_uses_mock_endpoint() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_code": "device-123",
                "user_code": "USER-CODE",
                "verification_uri": "https://verify.example",
                "expires_in": 600,
                "interval": 1,
                "message": "enter code",
            })),
        )
        .mount(&server)
        .await;
    let manager = ManageConnections::with_oauth_endpoint_overrides(
        Arc::new(EmptyRepository),
        Arc::new(EmptyRegistry),
        Arc::new(PassThroughKeyManager),
        Arc::new(UnusedBuilder),
        rook_usecases::manage_connections::OAuthEndpointOverrides {
            azure_token_url: None,
            gemini_token_url: None,
            azure_devicecode_url: Some(server.uri()),
        },
    );
    let started = manager
        .start_oauth_flow(
            rook_core::ProviderKind::AzureOpenAI,
            request(OAuthFlowType::DeviceCode),
        )
        .await
        .expect("device flow starts");
    assert_eq!(started.user_code.as_deref(), Some("USER-CODE"));
    let status = manager
        .oauth_flow_status(&started.flow_id)
        .await
        .expect("status");
    assert_eq!(status.status, "pending");
}

#[tokio::test]
async fn authorization_code_exchange_completes_connection() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "exchanged-access",
                "expires_in": 3600,
                "refresh_token": "exchanged-refresh",
                "scope": "scope-x",
                "id_token": "id-x",
            })),
        )
        .mount(&server)
        .await;
    let manager = ManageConnections::with_oauth_endpoint_overrides(
        Arc::new(EmptyRepository),
        Arc::new(EmptyRegistry),
        Arc::new(PassThroughKeyManager),
        Arc::new(UnusedBuilder),
        rook_usecases::manage_connections::OAuthEndpointOverrides {
            azure_token_url: Some(server.uri()),
            gemini_token_url: Some(server.uri()),
            azure_devicecode_url: Some(server.uri()),
        },
    );
    let started = manager
        .start_oauth_flow(
            rook_core::ProviderKind::AzureOpenAI,
            request(OAuthFlowType::AuthorizationCode),
        )
        .await
        .expect("flow starts");
    let url = reqwest::Url::parse(started.authorization_url.as_deref().unwrap()).unwrap();
    let query: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    let state = query.get("state").cloned().unwrap();
    let status = manager
        .complete_oauth_authorization(&state, "auth-code")
        .await
        .expect("exchange completes");
    assert_eq!(status.status, "connected");
    assert!(status.connection_id.is_some());
    // State is single-use.
    assert!(manager
        .complete_oauth_authorization(&state, "auth-code")
        .await
        .is_err());
}

#[tokio::test]
async fn remote_cleartext_devicecode_override_is_rejected() {
    let manager = ManageConnections::with_oauth_endpoint_overrides(
        Arc::new(EmptyRepository),
        Arc::new(EmptyRegistry),
        Arc::new(PassThroughKeyManager),
        Arc::new(UnusedBuilder),
        rook_usecases::manage_connections::OAuthEndpointOverrides {
            azure_token_url: None,
            gemini_token_url: None,
            azure_devicecode_url: Some("http://evil.example/devicecode".to_string()),
        },
    );
    let err = manager
        .start_oauth_flow(
            rook_core::ProviderKind::AzureOpenAI,
            request(OAuthFlowType::DeviceCode),
        )
        .await
        .expect_err("cleartext override must be rejected");
    assert!(err.to_string().contains("HTTPS or loopback"));
}

#[tokio::test]
async fn remote_cleartext_token_override_rejects_exchange() {
    let manager = ManageConnections::with_oauth_endpoint_overrides(
        Arc::new(EmptyRepository),
        Arc::new(EmptyRegistry),
        Arc::new(PassThroughKeyManager),
        Arc::new(UnusedBuilder),
        rook_usecases::manage_connections::OAuthEndpointOverrides {
            azure_token_url: Some("http://evil.example/token".to_string()),
            gemini_token_url: None,
            azure_devicecode_url: None,
        },
    );
    let started = manager
        .start_oauth_flow(
            rook_core::ProviderKind::AzureOpenAI,
            request(OAuthFlowType::AuthorizationCode),
        )
        .await
        .expect("flow starts");
    let url = reqwest::Url::parse(started.authorization_url.as_deref().unwrap()).unwrap();
    let query: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    let state = query.get("state").cloned().unwrap();
    let err = manager
        .complete_oauth_authorization(&state, "auth-code")
        .await
        .expect_err("cleartext override must be rejected");
    assert!(err.to_string().contains("HTTPS or loopback"));
}
