use providers_anthropic::{AnthropicProvider, AnthropicProviderConfig};
use rook_core::{CompletionRequest, HealthStatus, ModelId, ProviderPort, Role};
use shared_kernel::{ProviderId, RequestId};

#[tokio::test]
async fn health_check_returns_unknown_when_not_supported() {
    // Note: Anthropic provider returns HealthStatus::Unknown because
    // health_check is not yet implemented — no HTTP call is made.
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
async fn complete_returns_error_not_yet_implemented() {
    // Note: Anthropic provider complete() is not yet implemented —
    // it returns an error immediately without making HTTP calls.
    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: "http://localhost".to_string(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let req = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("claude-3-5-sonnet"),
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
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("not yet implemented"));
}
