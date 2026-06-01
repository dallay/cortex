use providers_gemini::{GeminiProvider, GeminiProviderConfig};
use rook_core::{CompletionRequest, HealthStatus, ModelId, ProviderPort, Role};
use shared_kernel::{ProviderId, RequestId};

#[tokio::test]
async fn health_check_returns_unknown() {
    // Gemini provider's health_check is not implemented — it always returns Unknown
    let provider = GeminiProvider::new(GeminiProviderConfig {
        id: ProviderId::new("gemini-test"),
        api_key: "test-key".to_string(),
        models: vec![ModelId::new("gemini-2.0-flash")],
        timeout_secs: 10,
    })
    .unwrap();

    let status = provider.health_check().await;
    assert!(matches!(status, HealthStatus::Unknown { .. }));
}

#[tokio::test]
async fn health_check_is_unhealthy_on_error() {
    // Even with a bad response, health_check returns Unknown (not implemented)
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1beta/models"))
        .respond_with(wiremock::ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let provider = GeminiProvider::new(GeminiProviderConfig {
        id: ProviderId::new("gemini-test"),
        api_key: "bad-key".to_string(),
        models: vec![ModelId::new("gemini-2.0-flash")],
        timeout_secs: 10,
    })
    .unwrap();

    let status = provider.health_check().await;
    // health_check is not implemented, so it always returns Unknown
    assert!(matches!(status, HealthStatus::Unknown { .. }));
}

#[tokio::test]
async fn complete_returns_error_not_implemented() {
    let provider = GeminiProvider::new(GeminiProviderConfig {
        id: ProviderId::new("gemini-test"),
        api_key: "test-key".to_string(),
        models: vec![ModelId::new("gemini-2.0-flash")],
        timeout_secs: 10,
    })
    .unwrap();

    let req = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("gemini-2.0-flash"),
        messages: vec![rook_core::Message {
            role: Role::User,
            content: rook_core::MessageContent::Text("Hi".to_string()),
        }],
        stream: false,
        max_tokens: Some(100),
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".to_string(),
            cacheable: true,
            priority: 0,
        },
    };

    let result = provider.complete(&req).await;
    // Gemini provider not yet implemented — should return an error
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("not yet implemented"));
}
