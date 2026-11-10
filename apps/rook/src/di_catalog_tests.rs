use super::{
    Arc, ConnectionId, CortexResult, DecryptedCredentials, DynamicProviderBuilder, ModelId,
    ProviderBuildInput, ProviderBuilderPort, ProviderId, StaticModelCatalog,
};
use async_trait::async_trait;
use rook_core::{CapabilityState, ModelDeclaration, OAuthTokenPort};

struct NoOAuth;

#[async_trait]
impl OAuthTokenPort for NoOAuth {
    async fn access_token(&self, _: &ConnectionId, _: bool) -> CortexResult<String> {
        panic!("constructing a provider must not request credentials upstream")
    }
}

fn input(
    kind: &'static rook_core::ProviderDescriptor,
    connection_id: ConnectionId,
) -> ProviderBuildInput {
    ProviderBuildInput {
        provider: kind,
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
        Arc::new(rook_usecases::PricingConfig {
            providers: Default::default(),
        }),
    );
    let first_provider = builder
        .build(input(
            rook_core::descriptor_by_id("openai").expect("openai descriptor"),
            first,
        ))
        .await
        .unwrap();
    let second_provider = builder
        .build(input(
            rook_core::descriptor_by_id("openai").expect("openai descriptor"),
            second,
        ))
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
        {"provider_kind":"gemini", "model_id":"gemini-test", "capabilities":{"tools":"supported", "streaming":"supported"}},
        {"provider_kind":"groq", "model_id":"groq-tools", "capabilities":{"tools":"supported"}}
    ])).unwrap();
    let builder = DynamicProviderBuilder::new(
        Arc::new(StaticModelCatalog::with_declarations(declarations).unwrap()),
        None,
        Arc::new(rook_usecases::PricingConfig {
            providers: Default::default(),
        }),
    );
    let mut compatible = input(
        rook_core::descriptor_by_id("openai-compatible").expect("openai-compatible descriptor"),
        ConnectionId::new(),
    );
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
        .build(input(
            rook_core::descriptor_by_id("gemini").expect("gemini descriptor"),
            ConnectionId::new(),
        ))
        .await
        .unwrap();
    let metadata = gemini.model_capabilities(&ModelId::new("gemini-test"));
    assert_eq!(metadata.tools.state, CapabilityState::Unsupported);
    assert_eq!(metadata.streaming.state, CapabilityState::Unsupported);
    let mut azure = input(
        rook_core::descriptor_by_id("azure-openai").expect("azure-openai descriptor"),
        ConnectionId::new(),
    );
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
    let groq = builder
        .build(input(
            rook_core::descriptor_by_id("groq").expect("groq descriptor"),
            ConnectionId::new(),
        ))
        .await
        .unwrap();
    assert_eq!(
        groq.model_capabilities(&ModelId::new("groq-tools"))
            .tools
            .state,
        CapabilityState::Supported
    );
}

#[tokio::test]
async fn openai_descriptor_requiring_base_url_is_rejected_without_override() {
    let mut descriptor = *rook_core::descriptor_by_id("openai-compatible").unwrap();
    descriptor.id = "custom-openai-with-required-url";
    descriptor.requires_base_url = true;
    descriptor.default_base_url = "";
    descriptor.endpoint_validation = rook_core::EndpointValidation::None;
    let descriptor = Box::leak(Box::new(descriptor));

    let result = super::build_provider_from_connection_with_policy(
        &ConnectionId::new(),
        descriptor,
        &DecryptedCredentials::ApiKey {
            api_key: "key".into(),
        },
        None,
        vec![],
        super::ProviderConnectionBuildOptions {
            quota_recorder: None,
            oauth_token_port: None,
            endpoint_policy: rook_core::EndpointPolicy::default(),
        },
    );
    assert!(
        result.is_err(),
        "required base URL must be enforced independently of endpoint-validation mode"
    );
}

#[tokio::test]
async fn custom_gemini_descriptor_default_host_is_used_for_completion_and_health() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1beta/models/gemini-2.0-flash:generateContent"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "candidates":[{"content":{"role":"model","parts":[{"text":"ok"}]},"finishReason":"STOP"}]
        })))
        .mount(&server).await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1beta/models"))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let mut descriptor = *rook_core::descriptor_by_id("gemini").unwrap();
    descriptor.default_base_url = Box::leak(server.uri().into_boxed_str());
    let descriptor = Box::leak(Box::new(descriptor));
    let provider = super::build_provider_from_connection_with_policy(
        &ConnectionId::new(),
        descriptor,
        &DecryptedCredentials::ApiKey {
            api_key: "key".into(),
        },
        None,
        vec![ModelId::new("gemini-2.0-flash")],
        super::ProviderConnectionBuildOptions {
            quota_recorder: None,
            oauth_token_port: None,
            endpoint_policy: rook_core::EndpointPolicy::default(),
        },
    )
    .unwrap();
    let request = rook_core::CompletionRequest {
        id: shared_kernel::RequestId::new(),
        model: ModelId::new("gemini-2.0-flash"),
        messages: vec![rook_core::Message {
            role: rook_core::Role::User,
            content: rook_core::MessageContent::Text("hello".into()),
            tool_calls: vec![],
        }],
        stream: false,
        max_tokens: Some(8),
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".into(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    };
    provider.complete(&request).await.unwrap();
    assert!(matches!(
        provider.health_check().await,
        rook_core::HealthStatus::Healthy { .. }
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);

    let override_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1beta/models"))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&override_server)
        .await;
    let overridden = super::build_provider_from_connection_with_policy(
        &ConnectionId::new(),
        descriptor,
        &DecryptedCredentials::ApiKey {
            api_key: "key".into(),
        },
        Some(override_server.uri()),
        vec![],
        super::ProviderConnectionBuildOptions {
            quota_recorder: None,
            oauth_token_port: None,
            endpoint_policy: rook_core::EndpointPolicy::default(),
        },
    )
    .unwrap();
    assert!(matches!(
        overridden.health_check().await,
        rook_core::HealthStatus::Healthy { .. }
    ));
    assert_eq!(override_server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn migrated_openai_descriptors_build_with_api_key_and_descriptor_models() {
    let descriptors = [
        "xai",
        "mistral",
        "deepseek",
        "together",
        "fireworks",
        "cerebras",
        "openrouter",
        "nan",
    ]
    .map(|id| rook_core::descriptor_by_id(id).expect("descriptor"));
    let declaration_values: Vec<serde_json::Value> = descriptors
        .iter()
        .flat_map(|descriptor| match descriptor.models {
            rook_core::ModelSource::Static(models) => models
                .iter()
                .map(|model| serde_json::json!({"provider_kind": descriptor.id, "model_id": model}))
                .collect::<Vec<_>>(),
            _ => panic!("expected migrated providers to declare static models"),
        })
        .collect::<Vec<_>>();
    let declarations: Vec<ModelDeclaration> =
        serde_json::from_value(serde_json::Value::Array(declaration_values)).unwrap();
    let builder = DynamicProviderBuilder::new(
        Arc::new(StaticModelCatalog::with_declarations(declarations).unwrap()),
        None,
        Arc::new(rook_usecases::PricingConfig {
            providers: Default::default(),
        }),
    );

    for descriptor in descriptors {
        let provider = builder
            .build(input(descriptor, ConnectionId::new()))
            .await
            .unwrap_or_else(|error| panic!("{} should build: {error}", descriptor.id));
        assert!(!provider.supported_models().is_empty(), "{}", descriptor.id);
        assert!(
            provider.is_available(),
            "{} should use the API key",
            descriptor.id
        );
    }
}
