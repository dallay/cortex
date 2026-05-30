use providers_ollama::{OllamaProvider, OllamaProviderConfig};
use rook_core::{CompletionRequest, HealthStatus, ModelId, ProviderPort, Role};
use shared_kernel::{ProviderId, RequestId};

#[tokio::test]
async fn health_check_returns_unknown_when_not_implemented() {
    // Ollama provider health_check is not yet implemented — always returns Unknown
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/tags"))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let provider = OllamaProvider::new(OllamaProviderConfig {
        id: ProviderId::new("ollama-test"),
        base_url: server.uri(),
        models: vec![ModelId::new("llama3")],
        timeout_secs: 10,
    })
    .unwrap();

    let status = provider.health_check().await;
    // Current implementation returns Unknown with reason "health_check_not_supported"
    assert!(matches!(status, HealthStatus::Unknown { .. }));
}

#[tokio::test]
async fn health_check_returns_unknown_on_any_response() {
    // Even on 500, the unimplemented health_check returns Unknown (not Unhealthy)
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/tags"))
        .respond_with(wiremock::ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let provider = OllamaProvider::new(OllamaProviderConfig {
        id: ProviderId::new("ollama-test"),
        base_url: server.uri(),
        models: vec![ModelId::new("llama3")],
        timeout_secs: 10,
    })
    .unwrap();

    let status = provider.health_check().await;
    // Current implementation doesn't actually call the server — returns Unknown
    assert!(matches!(status, HealthStatus::Unknown { .. }));
}

#[tokio::test]
async fn complete_returns_error_when_not_implemented() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "llama3",
                "message": { "role": "assistant", "content": "Hello, world!" },
                "done": true
            })),
        )
        .mount(&server)
        .await;

    let provider = OllamaProvider::new(OllamaProviderConfig {
        id: ProviderId::new("ollama-test"),
        base_url: server.uri(),
        models: vec![ModelId::new("llama3")],
        timeout_secs: 10,
    })
    .unwrap();

    let req = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("llama3"),
        messages: vec![rook_core::Message {
            role: Role::User,
            content: "Hi".to_string(),
        }],
        stream: false,
        max_tokens: Some(100),
        temperature: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".to_string(),
            cacheable: true,
            priority: 0,
        },
    };

    // Complete is not yet implemented — returns provider error
    let result = provider.complete(&req).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("not yet implemented"));
}
