use providers_openai::provider::{OpenAIProvider, OpenAIProviderConfig};
use rook_core::{CompletionRequest, HealthStatus, ModelId, ProviderPort, Role};
use shared_kernel::{ProviderId, RequestId};

#[tokio::test]
async fn health_check_returns_healthy_on_200() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/models"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer sk-test",
        ))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("gpt-4")],
        timeout_secs: 10,
    })
    .unwrap();

    let status = provider.health_check().await;
    assert!(matches!(status, HealthStatus::Healthy { .. }));
}

#[tokio::test]
async fn health_check_returns_unhealthy_on_401() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/models"))
        .respond_with(wiremock::ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("gpt-4")],
        timeout_secs: 10,
    })
    .unwrap();

    let status = provider.health_check().await;
    assert!(matches!(status, HealthStatus::Unhealthy { .. }));
}

#[tokio::test]
async fn complete_returns_response_on_success() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-123",
                "model": "gpt-4",
                "choices": [{
                    "message": { "role": "assistant", "content": "Hello, world!" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13 }
            })),
        )
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("gpt-4")],
        timeout_secs: 10,
    })
    .unwrap();

    let req = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("gpt-4"),
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

    let result = provider.complete(&req).await;
    assert!(result.is_ok());
    let resp = result.unwrap();
    assert_eq!(resp.content, "Hello, world!");
}
