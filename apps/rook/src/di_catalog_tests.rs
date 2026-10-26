use super::*;
use rook_core::{CapabilityState, ModelDeclaration, OAuthTokenPort};

struct NoOAuth;

#[async_trait]
impl OAuthTokenPort for NoOAuth {
    async fn access_token(&self, _: &ConnectionId, _: bool) -> CortexResult<String> {
        panic!("constructing a provider must not request credentials upstream")
    }
}

fn input(kind: ProviderKind, connection_id: ConnectionId) -> ProviderBuildInput {
    ProviderBuildInput {
        provider_kind: kind,
        connection_id,
        provider_runtime_id: ProviderId::new("runtime"),
        decrypted_credentials: DecryptedCredentials::ApiKey {
            api_key: "synthetic-key".into(),
        },
        base_url: None,
        default_model: None,
        supported_models: vec![],
        api_format: None,
        endpoint_policy: rook_core::EndpointPolicy::default(),
        oauth_token_port: Arc::new(NoOAuth),
    }
}

#[tokio::test]
async fn builder_keeps_connection_scoped_models_and_metadata_on_the_exact_adapter() {
    let first = ConnectionId::new();
    let second = ConnectionId::new();
    let declarations: Vec<ModelDeclaration> = serde_json::from_value(serde_json::json!([
        {"provider_kind":"openai", "model_id":"custom", "capabilities":{"tools":"supported", "streaming":"supported"}},
        {"provider_kind":"openai", "model_id":"custom", "connection_id":first,"capabilities":{"tools":"unknown"}},
        {"provider_kind":"openai", "model_id":"private-model", "connection_id":first,"capabilities":{"streaming":"supported"}}
    ])).unwrap();
    let builder = DynamicProviderBuilder::new(
        Arc::new(StaticModelCatalog::with_declarations(declarations).unwrap()),
        None,
    );
    let first_provider = builder
        .build(input(ProviderKind::OpenAI, first))
        .await
        .unwrap();
    let second_provider = builder
        .build(input(ProviderKind::OpenAI, second))
        .await
        .unwrap();
    assert!(first_provider.supports_model(&ModelId::new("private-model")));
    assert!(!second_provider.supports_model(&ModelId::new("private-model")));
    let first_metadata = first_provider.model_capabilities(&ModelId::new("custom"));
    assert_eq!(first_metadata.tools.state, CapabilityState::Unknown);
    assert_eq!(first_metadata.tools.source, "connection_configuration");
    assert_eq!(first_metadata.streaming.state, CapabilityState::Supported);
    assert_eq!(
        second_provider
            .model_capabilities(&ModelId::new("custom"))
            .tools
            .state,
        CapabilityState::Supported
    );
}

#[tokio::test]
async fn declarations_preserve_azure_and_compatible_membership_and_adapter_limits() {
    let declarations = serde_json::from_value(serde_json::json!([
        {"provider_kind":"openai-compatible", "model_id":"undeclared"},
        {"provider_kind":"openai-compatible", "model_id":"physical", "capabilities":{"tools":"supported", "vision":"supported"}},
        {"provider_kind":"azure-openai", "model_id":"undeclared"},
        {"provider_kind":"azure-openai", "model_id":"deployment", "capabilities":{"tools":"supported"}},
        {"provider_kind":"gemini", "model_id":"gemini-test", "capabilities":{"tools":"supported", "streaming":"supported"}}
    ])).unwrap();
    let builder = DynamicProviderBuilder::new(
        Arc::new(StaticModelCatalog::with_declarations(declarations).unwrap()),
        None,
    );
    let mut compatible = input(ProviderKind::OpenAiCompatible, ConnectionId::new());
    compatible.base_url = Some("https://upstream.example/v1".into());
    compatible.supported_models = vec![ModelId::new("physical")];
    compatible.endpoint_policy = rook_core::EndpointPolicyConfig {
        grants: vec![rook_core::EndpointGrant {
            connection_id: compatible.connection_id.to_string(),
            allowed_origins: vec!["https://upstream.example:443".into()],
        }],
    }
    .for_connection(&compatible.connection_id.to_string());
    let provider = builder.build(compatible).await.unwrap();
    assert_eq!(provider.supported_models(), &[ModelId::new("physical")]);
    let metadata = provider.model_capabilities(&ModelId::new("physical"));
    assert_eq!(metadata.tools.state, CapabilityState::Supported);
    assert_eq!(metadata.vision.state, CapabilityState::Unsupported);
    assert_eq!(metadata.vision.source, "adapter");
    let gemini = builder
        .build(input(ProviderKind::Gemini, ConnectionId::new()))
        .await
        .unwrap();
    let metadata = gemini.model_capabilities(&ModelId::new("gemini-test"));
    assert_eq!(metadata.tools.state, CapabilityState::Unsupported);
    assert_eq!(metadata.streaming.state, CapabilityState::Unsupported);
    let mut azure = input(ProviderKind::AzureOpenAI, ConnectionId::new());
    azure.base_url = Some("https://example.openai.azure.com".into());
    azure.default_model = Some(ModelId::new("deployment"));
    azure.decrypted_credentials = DecryptedCredentials::OAuth {
        email: String::new(),
        access_token: String::new(),
        refresh_token: String::new(),
        expires_at: i64::MAX,
        scope: String::new(),
        id_token: String::new(),
        project_id: String::new(),
        oauth_client_id: "synthetic-client".into(),
        oauth_client_secret: None,
        oauth_tenant_id: None,
    };
    let azure = builder.build(azure).await.unwrap();
    assert_eq!(azure.supported_models(), &[ModelId::new("deployment")]);
    assert_eq!(
        azure
            .model_capabilities(&ModelId::new("deployment"))
            .tools
            .state,
        CapabilityState::Supported
    );
}
