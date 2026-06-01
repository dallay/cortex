use providers_anthropic::{AnthropicProvider, AnthropicProviderConfig};
use rook_core::{CompletionRequest, HealthStatus, MessageContent, ModelId, ProviderPort, Role};
use shared_kernel::{ProviderId, RequestId};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn health_check_returns_unknown_when_not_supported() {
    // Anthropic provider returns HealthStatus::Unknown because
    // health_check is not supported via the Anthropic API.
    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: "http://localhost".to_string(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let status = provider.health_check().await;
    assert!(matches!(
        status,
        HealthStatus::Unknown {
            reason,
            ..
        } if reason == "health_check_not_supported"
    ));
}

#[tokio::test]
async fn complete_returns_valid_response_from_mock_server() {
    // T-05 AC: complete() returns Ok(CompletionResponse) with matching content and token counts.
    let mock_server = MockServer::start().await;

    let response_body = serde_json::json!({
        "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "Hello, world!"}],
        "model": "claude-3-5-sonnet-20241022",
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {
            "input_tokens": 10,
            "output_tokens": 5
        }
    });

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: mock_server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet-20241022")],
        timeout_secs: 10,
    })
    .unwrap();

    let req = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("claude-3-5-sonnet-20241022"),
        messages: vec![rook_core::Message {
            role: Role::User,
            content: MessageContent::Text("Hi".to_string()),
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
    assert!(
        result.is_ok(),
        "complete() should succeed: {:?}",
        result.err()
    );

    let resp = result.unwrap();
    assert_eq!(resp.content, "Hello, world!");
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 5);
    assert_eq!(resp.usage.total_tokens, 15);
}
