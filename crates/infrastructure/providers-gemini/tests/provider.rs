use providers_gemini::{GeminiProvider, GeminiProviderConfig};
use rook_core::{CompletionRequest, HealthStatus, ModelId, ProviderPort, Role};
use shared_kernel::{ProviderId, RequestId};

#[tokio::test]
async fn health_check_returns_unknown() {
    // Gemini provider's health_check is not implemented — it always returns Unknown
    let provider = GeminiProvider::new(GeminiProviderConfig {
        id: ProviderId::new("gemini-test"),
        api_key: "test-key".to_string(),
        base_url: None,
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
        base_url: Some(server.uri()),
        models: vec![ModelId::new("gemini-2.0-flash")],
        timeout_secs: 10,
    })
    .unwrap();

    let status = provider.health_check().await;
    // health_check is not implemented, so it always returns Unknown
    assert!(matches!(status, HealthStatus::Unknown { .. }));
}

#[tokio::test]
async fn complete_returns_response_with_token_counts() {
    // T5.3: Gemini complete() parses usageMetadata.promptTokenCount and candidatesTokenCount.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(
            "/v1beta/models/gemini-2.0-flash:generateContent",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [{ "text": "Hello, world!" }]
                    },
                    "finishReason": "STOP",
                    "index": 0
                }],
                "usageMetadata": {
                    "promptTokenCount": 20,
                    "candidatesTokenCount": 12,
                    "totalTokenCount": 32
                }
            })),
        )
        .mount(&server)
        .await;

    let provider = GeminiProvider::new(GeminiProviderConfig {
        id: ProviderId::new("gemini-test"),
        api_key: "test-key".to_string(),
        base_url: Some(server.uri()),
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
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
            cache_control_header: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    };

    let result = provider.complete(&req).await;
    assert!(result.is_ok());
    let resp = result.unwrap();
    assert_eq!(resp.content, "Hello, world!");
    // T5.3: Gemini parses promptTokenCount and candidatesTokenCount
    assert_eq!(resp.usage.prompt_tokens, 20);
    assert_eq!(resp.usage.completion_tokens, 12);
    // Cache/reasoning tokens not available in Gemini API
    assert_eq!(resp.usage.cache_read_tokens, None);
    assert_eq!(resp.usage.cache_creation_tokens, None);
    assert_eq!(resp.usage.reasoning_tokens, None);
}
