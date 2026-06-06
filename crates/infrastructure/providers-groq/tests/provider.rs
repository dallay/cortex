use providers_groq::{GroqProvider, GroqProviderConfig};
use rook_core::{CompletionRequest, HealthStatus, ModelId, ProviderPort, Role};
use shared_kernel::{ProviderId, RequestId};

#[tokio::test]
async fn health_check_returns_unknown_when_not_implemented() {
    let provider = GroqProvider::new(GroqProviderConfig {
        id: ProviderId::new("groq-test"),
        api_key: "gsk-test".to_string(),
        base_url: None,
        models: vec![ModelId::new("llama-3.3-70b")],
        timeout_secs: 10,
    })
    .unwrap();

    let status = provider.health_check().await;
    let reason = match &status {
        HealthStatus::Unknown { reason, .. } => reason.clone(),
        _ => panic!("expected Unknown status"),
    };
    assert_eq!(reason, "health_check_not_supported");
    assert!(matches!(status, HealthStatus::Unknown { .. }));
}

#[tokio::test]
async fn complete_returns_response_with_token_counts() {
    // T5.3: Groq complete() parses usage (OpenAI-compatible format).
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-groq-123",
                "model": "llama-3.3-70b-versatile",
                "choices": [{
                    "message": { "role": "assistant", "content": "Hello, world!" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 25,
                    "completion_tokens": 13,
                    "total_tokens": 38
                }
            })),
        )
        .mount(&server)
        .await;

    let provider = GroqProvider::new(GroqProviderConfig {
        id: ProviderId::new("groq-test"),
        api_key: "gsk-test".to_string(),
        base_url: Some(server.uri()),
        models: vec![ModelId::new("llama-3.3-70b")],
        timeout_secs: 10,
    })
    .unwrap();

    let req = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("llama-3.3-70b"),
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
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
            cache_control_header: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    };

    let result = provider.complete(&req).await;
    if let Err(ref e) = result {
        eprintln!("GROQ ERROR: {:?}", e);
    }
    assert!(result.is_ok());
    let resp = result.unwrap();
    assert_eq!(resp.content, "Hello, world!");
    // T5.3: Groq parses prompt_tokens and completion_tokens (OpenAI-compatible)
    assert_eq!(resp.usage.prompt_tokens, 25);
    assert_eq!(resp.usage.completion_tokens, 13);
    // Cache/reasoning tokens not available in Groq API
    assert_eq!(resp.usage.cache_read_tokens, None);
    assert_eq!(resp.usage.cache_creation_tokens, None);
    assert_eq!(resp.usage.reasoning_tokens, None);
}
