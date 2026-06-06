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
async fn complete_returns_response_with_token_counts() {
    // T5.3: Ollama complete() parses prompt_eval_count and eval_count.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "llama3",
                "message": { "role": "assistant", "content": "Hello, world!" },
                "done": true,
                "prompt_eval_count": 50,
                "eval_count": 25
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
    // T5.3: Ollama returns prompt_eval_count and eval_count
    assert_eq!(resp.usage.prompt_tokens, 50);
    assert_eq!(resp.usage.completion_tokens, 25);
    // Cache/reasoning tokens are not available for Ollama
    assert_eq!(resp.usage.cache_read_tokens, None);
    assert_eq!(resp.usage.cache_creation_tokens, None);
    assert_eq!(resp.usage.reasoning_tokens, None);
}

#[tokio::test]
async fn complete_handles_missing_eval_counts() {
    // T5.3: Ollama with no token count fields returns zeros
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "llama3",
                "message": { "role": "assistant", "content": "Hi" },
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
    // Missing eval counts default to 0
    assert_eq!(resp.usage.prompt_tokens, 0);
    assert_eq!(resp.usage.completion_tokens, 0);
}
