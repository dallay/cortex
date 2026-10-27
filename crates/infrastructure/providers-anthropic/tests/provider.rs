use futures::StreamExt;
use providers_anthropic::{AnthropicProvider, AnthropicProviderConfig};
use rook_core::{
    ApiKeyRestrictions, CompletionRequest, FinishReason, HealthStatus, MessageContent, ModelId,
    ProviderPort, RequestMetadata, Role,
};
use shared_kernel::{ProviderId, RequestId};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn stream_merges_initial_input_and_cache_usage_with_terminal_output() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":0,\"cache_read_input_tokens\":8,\"cache_creation_input_tokens\":2}}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        ))).mount(&server).await;
    let provider = test_provider(&server);
    let mut req = make_test_request("claude-3-5-sonnet");
    req.stream = true;
    let chunks: Vec<_> = provider.stream(&req).await.unwrap().collect().await;
    let usage = chunks
        .last()
        .unwrap()
        .as_ref()
        .unwrap()
        .usage
        .as_ref()
        .unwrap();
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 7);
    assert_eq!(usage.total_tokens, 19);
    assert_eq!(usage.cache_read_tokens, Some(8));
    assert_eq!(usage.cache_creation_tokens, Some(2));
}

async fn mount_usage_stream(
    server: &MockServer,
    initial_message: Option<serde_json::Value>,
    terminal_usage: serde_json::Value,
) {
    let mut events = Vec::new();
    if let Some(message) = initial_message {
        events.push(serde_json::json!({"type": "message_start", "message": message}));
    }
    events.push(serde_json::json!({
        "type": "message_delta",
        "delta": {"stop_reason": "end_turn"},
        "usage": terminal_usage,
    }));
    events.push(serde_json::json!({"type": "message_stop"}));
    let body: String = events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect();
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .expect(1)
        .mount(server)
        .await;
}

async fn collect_stream_usage(provider: &AnthropicProvider) -> rook_core::TokenUsage {
    let mut req = make_test_request("claude-3-5-sonnet");
    req.stream = true;
    let chunks = provider
        .stream(&req)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        chunks.len(),
        1,
        "message_start is metadata, not a domain chunk"
    );
    assert_eq!(chunks[0].finish_reason, Some(FinishReason::Stop));
    chunks
        .into_iter()
        .next()
        .unwrap()
        .usage
        .expect("terminal usage")
}

#[tokio::test]
async fn stream_usage_explicit_zero_terminal_values_override_initial_counters() {
    let server = MockServer::start().await;
    mount_usage_stream(
        &server,
        Some(serde_json::json!({"usage": {
            "input_tokens": 12, "output_tokens": 2,
            "cache_read_input_tokens": 8, "cache_creation_input_tokens": 4,
        }})),
        serde_json::json!({
            "input_tokens": 0, "output_tokens": 0,
            "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0,
        }),
    )
    .await;
    let usage = collect_stream_usage(&test_provider(&server)).await;
    assert_eq!(usage.prompt_tokens, 0);
    assert_eq!(usage.completion_tokens, 0);
    assert_eq!(usage.total_tokens, 0);
    assert_eq!(usage.cache_read_tokens, Some(0));
    assert_eq!(usage.cache_creation_tokens, Some(0));
}

#[tokio::test]
async fn stream_usage_merges_each_missing_field_independently() {
    let server = MockServer::start().await;
    mount_usage_stream(
        &server,
        Some(serde_json::json!({"usage": {
            "input_tokens": 12, "cache_read_input_tokens": 8, "cache_creation_input_tokens": 4,
        }})),
        serde_json::json!({
            "input_tokens": 20, "output_tokens": 7,
            "cache_read_input_tokens": null, "cache_creation_input_tokens": 1,
        }),
    )
    .await;
    let usage = collect_stream_usage(&test_provider(&server)).await;
    assert_eq!(usage.prompt_tokens, 20);
    assert_eq!(usage.completion_tokens, 7);
    assert_eq!(usage.total_tokens, 27);
    assert_eq!(usage.cache_read_tokens, Some(8));
    assert_eq!(usage.cache_creation_tokens, Some(1));
}

#[tokio::test]
async fn stream_usage_initial_output_tokens_are_not_added_to_terminal_output() {
    let server = MockServer::start().await;
    mount_usage_stream(
        &server,
        Some(serde_json::json!({"usage": {"input_tokens": 12, "output_tokens": 2}})),
        serde_json::json!({"output_tokens": 7}),
    )
    .await;
    let usage = collect_stream_usage(&test_provider(&server)).await;
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 7);
    assert_eq!(usage.total_tokens, 19);
    assert_eq!(usage.cache_read_tokens, None);
    assert_eq!(usage.cache_creation_tokens, None);
}

#[tokio::test]
async fn stream_usage_missing_terminal_output_defaults_to_zero() {
    let server = MockServer::start().await;
    mount_usage_stream(
        &server,
        Some(serde_json::json!({"usage": {"input_tokens": 12}})),
        serde_json::json!({}),
    )
    .await;
    let usage = collect_stream_usage(&test_provider(&server)).await;
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 0);
    assert_eq!(usage.total_tokens, 12);
}

#[tokio::test]
async fn stream_usage_does_not_leak_between_requests_on_the_same_provider() {
    let server = MockServer::start().await;
    let provider = test_provider(&server);
    mount_usage_stream(
        &server,
        Some(serde_json::json!({"usage": {
            "input_tokens": 12, "cache_read_input_tokens": 8, "cache_creation_input_tokens": 4,
        }})),
        serde_json::json!({"output_tokens": 7}),
    )
    .await;
    assert_eq!(collect_stream_usage(&provider).await.prompt_tokens, 12);
    server.verify().await;
    server.reset().await;

    mount_usage_stream(&server, None, serde_json::json!({"output_tokens": 3})).await;
    let usage = collect_stream_usage(&provider).await;
    assert_eq!(usage.prompt_tokens, 0);
    assert_eq!(usage.completion_tokens, 3);
    assert_eq!(usage.total_tokens, 3);
    assert_eq!(usage.cache_read_tokens, None);
    assert_eq!(usage.cache_creation_tokens, None);
}

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
async fn complete_does_not_follow_redirects() {
    let target = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&target)
        .await;
    let source = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/target", target.uri())),
        )
        .mount(&source)
        .await;
    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-redirect-test"),
        api_key: "sk-test".to_string(),
        base_url: source.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();
    let request = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("claude-3-5-sonnet"),
        messages: vec![rook_core::Message {
            tool_calls: vec![],
            role: Role::User,
            content: MessageContent::Text("hello".to_string()),
        }],
        stream: false,
        max_tokens: Some(10),
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".to_string(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions::default(),
    };

    let result = provider.complete(&request).await;

    assert!(result.is_err(), "a redirect response must fail the request");
    assert_eq!(source.received_requests().await.unwrap().len(), 1);
    assert!(target.received_requests().await.unwrap().is_empty());
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
            tool_calls: vec![],
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
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
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

#[tokio::test]
async fn complete_parses_cache_tokens() {
    // T5.2: Anthropic non-streaming response parses cache_creation_input_tokens
    // and cache_read_input_tokens.
    let mock_server = MockServer::start().await;

    let response_body = serde_json::json!({
        "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "Hi"}],
        "model": "claude-3-5-sonnet-20241022",
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {
            "input_tokens": 500,
            "output_tokens": 150,
            "cache_creation_input_tokens": 300,
            "cache_read_input_tokens": 200
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
            tool_calls: vec![],
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
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    };

    let result = provider.complete(&req).await;
    assert!(result.is_ok());
    let resp = result.unwrap();
    assert_eq!(resp.usage.cache_creation_tokens, Some(300));
    assert_eq!(resp.usage.cache_read_tokens, Some(200));
    assert_eq!(resp.usage.reasoning_tokens, None); // Not supported by Anthropic
}

// ---------------------------------------------------------------------------
// Error mapping tests — map_anthropic_http_error via public API
// ---------------------------------------------------------------------------

fn make_test_request(model: &str) -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new(model),
        messages: vec![rook_core::Message {
            tool_calls: vec![],
            role: Role::User,
            content: MessageContent::Text("Hi".to_string()),
        }],
        stream: false,
        max_tokens: Some(100),
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".to_string(),
            cacheable: true,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions::default(),
    }
}

#[tokio::test]
async fn complete_returns_auth_error_on_401() {
    // T-05: Error handling - 401 returns auth error
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "type": "error",
            "error": {"type": "authentication_error", "message": "invalid api key"}
        })))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-invalid".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let result = provider
        .complete(&make_test_request("claude-3-5-sonnet"))
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("authentication"),
        "Expected auth error, got: {}",
        err
    );
}

#[tokio::test]
async fn complete_returns_rate_limit_error_on_429() {
    // T-05: Error handling - 429 returns rate limit error with retry info
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "60")
                .insert_header("x-ratelimit-reset", "120")
                .set_body_json(serde_json::json!({
                    "type": "error",
                    "error": {"type": "rate_limit_error", "message": "slow down"}
                })),
        )
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let result = provider
        .complete(&make_test_request("claude-3-5-sonnet"))
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("rate limited") || err_msg.contains("429"),
        "Expected rate limit error, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn complete_returns_invalid_request_on_400() {
    // T-05: Error handling - 400 returns invalid request error
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "type": "error",
            "error": {"type": "invalid_request_error", "message": "bad request"}
        })))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let result = provider
        .complete(&make_test_request("claude-3-5-sonnet"))
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("400") || err.to_string().contains("bad request"),
        "Expected 400 error, got: {}",
        err
    );
}

#[tokio::test]
async fn complete_sanitizes_error_body_sensitive_data() {
    // Verify that sensitive data (api_key, token, etc.) is redacted in error messages
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"type":"error","error":{"type":"invalid_request","message":"bad request"},"api_key":"sk-ant-api12345secret","token":"abc123"}"#,
        ))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let result = provider
        .complete(&make_test_request("claude-3-5-sonnet"))
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_msg = err.to_string();
    // Sensitive data should be redacted
    assert!(
        !err_msg.contains("sk-ant-api12345secret"),
        "api_key should be redacted: {}",
        err_msg
    );
    assert!(
        !err_msg.contains("abc123"),
        "token value should be redacted: {}",
        err_msg
    );
    // But the error type should be preserved
    assert!(
        err_msg.contains("invalid_request") || err_msg.contains("400"),
        "Error type should be visible: {}",
        err_msg
    );
}

// ---------------------------------------------------------------------------
// Streaming tests — stream() SSE parsing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stream_returns_chunks_on_anthropic_sse_success() {
    // BIG GAP: Anthropic streaming was not tested. This tests the full SSE parsing.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\n\
             data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n\
             data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n\
             data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":5,\"input_tokens\":10}}\n\n\
             data: {\"type\":\"message_stop\"}\n\n",
        ))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let mut req = make_test_request("claude-3-5-sonnet");
    req.stream = true;

    let chunks = provider
        .stream(&req)
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("chunks parse");

    // Collect the delta text
    let delta_text: String = chunks
        .iter()
        .filter(|chunk| !chunk.delta.is_empty())
        .map(|chunk| chunk.delta.as_str())
        .collect();

    assert_eq!(delta_text, "Hello");

    // Check the final chunk has finish reason and usage
    let final_chunk = chunks.last().unwrap();
    assert_eq!(final_chunk.finish_reason, Some(FinishReason::Stop));
    assert!(final_chunk.usage.is_some());
    assert_eq!(final_chunk.usage.as_ref().unwrap().completion_tokens, 5);
}

#[tokio::test]
async fn stream_returns_error_on_http_429() {
    // Verify that stream() properly handles HTTP errors via map_anthropic_http_error
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "30")
                .set_body_json(serde_json::json!({
                    "type": "error",
                    "error": {"type": "rate_limit_error", "message": "slow down"}
                })),
        )
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let mut req = make_test_request("claude-3-5-sonnet");
    req.stream = true;

    let result = provider.stream(&req).await;
    let err_msg = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("Expected error on 429 response"),
    };
    assert!(
        err_msg.contains("rate limited") || err_msg.contains("429"),
        "Expected rate limit error, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn stream_returns_error_on_http_401() {
    // Verify that stream() properly handles 401 auth errors
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "type": "error",
            "error": {"type": "authentication_error", "message": "invalid api key"}
        })))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-invalid".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let mut req = make_test_request("claude-3-5-sonnet");
    req.stream = true;

    let result = provider.stream(&req).await;
    let err_msg = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("Expected error on 401 response"),
    };
    assert!(
        err_msg.to_lowercase().contains("authentication"),
        "Expected auth error, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn stream_parses_tool_use_finish_reason() {
    // Test that streaming correctly parses tool_use finish reason
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\"}}\n\n\
             data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"tool\\\":\\\"get_weather\\\"}\"}}\n\n\
             data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":20,\"input_tokens\":15}}\n\n\
             data: {\"type\":\"message_stop\"}\n\n",
        ))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let mut req = make_test_request("claude-3-5-sonnet");
    req.stream = true;

    let chunks = provider
        .stream(&req)
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("chunks parse");

    let final_chunk = chunks.last().unwrap();
    assert_eq!(final_chunk.finish_reason, Some(FinishReason::ToolCalls));
}

#[tokio::test]
async fn stream_includes_system_message_in_request() {
    // Test that system messages are properly extracted and sent in the system field
    let server = MockServer::start().await;
    // Capture the request and verify it contains system field
    let captured_request = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured_clone = captured_request.clone();

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(move |req: &wiremock::Request| {
            let body_str = std::str::from_utf8(&req.body).unwrap();
            *captured_clone.lock().unwrap() = Some(body_str.to_string());
            ResponseTemplate::new(200).set_body_string(
                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
                 data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2,\"input_tokens\":5}}\n\n\
                 data: {\"type\":\"message_stop\"}\n\n",
            )
        })
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let mut req = make_test_request("claude-3-5-sonnet");
    req.stream = true;
    // Add system message
    req.messages.insert(
        0,
        rook_core::Message {
            tool_calls: vec![],
            role: Role::System,
            content: MessageContent::Text("You are a helpful assistant.".to_string()),
        },
    );

    let _ = provider.stream(&req).await;

    let captured = captured_request.lock().unwrap();
    let body = captured.as_ref().expect("request was made");
    // Verify the request contains system field with the system prompt
    assert!(
        body.contains("\"system\""),
        "Request should contain system field: {}",
        body
    );
    assert!(
        body.contains("You are a helpful assistant"),
        "Request should contain system prompt: {}",
        body
    );
}

#[tokio::test]
async fn complete_maps_pause_turn_to_stop_finish_reason() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg_pause",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "paused"}],
            "model": "claude-3-5-sonnet-20241022",
            "stop_reason": "pause_turn",
            "stop_sequence": null,
            "usage": {"input_tokens": 3, "output_tokens": 2}
        })))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let resp = provider
        .complete(&make_test_request("claude-3-5-sonnet"))
        .await
        .expect("complete succeeds");
    assert_eq!(resp.finish_reason, Some(FinishReason::Stop));
}

#[tokio::test]
async fn stream_maps_pause_turn_to_stop_finish_reason() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
             data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"pause_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":2}}\n\n\
             data: {\"type\":\"message_stop\"}\n\n",
        ))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let mut req = make_test_request("claude-3-5-sonnet");
    req.stream = true;
    let chunks = provider
        .stream(&req)
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("chunks parse");

    let final_chunk = chunks.last().unwrap();
    assert_eq!(final_chunk.finish_reason, Some(FinishReason::Stop));
}

#[tokio::test]
async fn stream_keeps_finish_reason_and_usage_when_stop_sequence_is_a_string() {
    // Anthropic reports the matched stop sequence as a string and may send a
    // non-empty `content` array in `message_start`; neither may drop events.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"\"}],\"model\":\"claude-3-5-sonnet\"}}\n\n\
             data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
             data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"stop_sequence\",\"stop_sequence\":\"END\"},\"usage\":{\"output_tokens\":7}}\n\n\
             data: {\"type\":\"message_stop\"}\n\n",
        ))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap();

    let mut req = make_test_request("claude-3-5-sonnet");
    req.stream = true;
    let chunks = provider
        .stream(&req)
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("chunks parse");

    let final_chunk = chunks.last().unwrap();
    assert_eq!(final_chunk.finish_reason, Some(FinishReason::Stop));
    assert_eq!(
        final_chunk.usage.as_ref().map(|u| u.completion_tokens),
        Some(7)
    );
}

// ---------------------------------------------------------------------------
// R3: structured tool-calling (RED first)
// ---------------------------------------------------------------------------

fn tool_test_request() -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("claude-3-5-sonnet"),
        messages: vec![rook_core::Message {
            tool_calls: vec![],
            role: Role::User,
            content: MessageContent::Text("Weather in Paris?".to_string()),
        }],
        stream: false,
        max_tokens: Some(100),
        temperature: None,
        tools: Some(serde_json::json!([{
            "name": "get_weather",
            "description": "Get weather",
            "input_schema": {"type": "object"}
        }])),
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".to_string(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions::default(),
    }
}

fn test_provider(server: &MockServer) -> std::sync::Arc<AnthropicProvider> {
    AnthropicProvider::new(AnthropicProviderConfig {
        id: ProviderId::new("anthropic-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("claude-3-5-sonnet")],
        timeout_secs: 10,
    })
    .unwrap()
}

#[tokio::test]
async fn complete_preserves_single_tool_use() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg_tool1",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "text", "text": "Let me check."},
                {"type": "tool_use", "id": "toolu_123", "name": "get_weather",
                 "input": {"city": "Paris"}}
            ],
            "model": "claude-3-5-sonnet-20241022",
            "stop_reason": "tool_use",
            "stop_sequence": null,
            "usage": {"input_tokens": 20, "output_tokens": 15}
        })))
        .mount(&server)
        .await;

    let resp = test_provider(&server)
        .complete(&tool_test_request())
        .await
        .expect("complete with tool_use succeeds");
    assert_eq!(resp.finish_reason, Some(FinishReason::ToolCalls));
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].id.as_deref(), Some("toolu_123"));
    assert_eq!(resp.tool_calls[0].name, "get_weather");
    assert_eq!(
        resp.tool_calls[0].arguments,
        serde_json::json!({"city": "Paris"})
    );
    assert!(resp.content_blocks.iter().any(|b| matches!(
        b, MessageContent::ToolUse { id, .. } if id == "toolu_123"
    )));
}

#[tokio::test]
async fn complete_preserves_multiple_tool_use() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg_toolN",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "tool_use", "id": "toolu_1", "name": "get_weather",
                 "input": {"city": "Paris"}},
                {"type": "tool_use", "id": "toolu_2", "name": "get_time",
                 "input": {"zone": "CET"}}
            ],
            "model": "claude-3-5-sonnet-20241022",
            "stop_reason": "tool_use",
            "stop_sequence": null,
            "usage": {"input_tokens": 30, "output_tokens": 20}
        })))
        .mount(&server)
        .await;

    let resp = test_provider(&server)
        .complete(&tool_test_request())
        .await
        .expect("complete with N tool_use succeeds");
    assert_eq!(resp.finish_reason, Some(FinishReason::ToolCalls));
    assert_eq!(resp.tool_calls.len(), 2);
    assert_eq!(resp.tool_calls[0].name, "get_weather");
    assert_eq!(resp.tool_calls[1].name, "get_time");
    assert_eq!(resp.tool_calls[1].id.as_deref(), Some("toolu_2"));
}

#[tokio::test]
async fn complete_sends_structured_tool_history() {
    let server = MockServer::start().await;
    let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured_clone = captured.clone();
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(move |req: &wiremock::Request| {
            *captured_clone.lock().unwrap() =
                Some(std::str::from_utf8(&req.body).unwrap().to_string());
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg_ok", "type": "message", "role": "assistant",
                "content": [{"type": "text", "text": "Sunny."}],
                "model": "claude-3-5-sonnet", "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": {"input_tokens": 40, "output_tokens": 5}
            }))
        })
        .mount(&server)
        .await;

    let mut req = tool_test_request();
    req.messages.push(rook_core::Message {
        tool_calls: vec![rook_core::MessageToolCall {
            id: Some("toolu_123".to_string()),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "Paris"}),
        }],
        role: Role::Assistant,
        content: MessageContent::Text(String::new()),
    });
    req.messages.push(rook_core::Message {
        tool_calls: vec![],
        role: Role::User,
        content: MessageContent::ToolResult {
            tool_use_id: "toolu_123".to_string(),
            content: vec![MessageContent::Text("sunny".to_string())],
        },
    });

    test_provider(&server)
        .complete(&req)
        .await
        .expect("complete sends tools");
    let body = captured.lock().unwrap().clone().expect("request captured");
    let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
    let msgs = v["messages"].as_array().expect("messages array");
    let asst = msgs
        .iter()
        .find(|m| m["role"] == "assistant")
        .expect("assistant msg");
    let blocks = asst["content"]
        .as_array()
        .expect("structured content blocks");
    assert!(
        blocks.iter().any(|b| b["type"] == "tool_use"
            && b["id"] == "toolu_123"
            && b["name"] == "get_weather"),
        "assistant tool_calls must serialize as tool_use blocks: {body}"
    );
    let user = msgs
        .iter()
        .find(|m| {
            m["role"] == "user"
                && m["content"]
                    .as_array()
                    .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_result"))
        })
        .expect("user msg with tool_result");
    let user_blocks = user["content"].as_array().expect("user blocks");
    assert!(
        user_blocks
            .iter()
            .any(|b| b["type"] == "tool_result" && b["tool_use_id"] == "toolu_123"),
        "tool results must serialize as tool_result blocks: {body}"
    );
}

#[tokio::test]
async fn stream_emits_tool_call_fragments() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_s\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n\
             data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_123\",\"name\":\"get_weather\"}}\n\n\
             data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\"}}\n\n\
             data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\" \\\"Paris\\\"}\"}}\n\n\
             data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
             data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":20,\"input_tokens\":15}}\n\n\
             data: {\"type\":\"message_stop\"}\n\n",
        ))
        .mount(&server)
        .await;

    let mut req = tool_test_request();
    req.stream = true;
    let chunks = test_provider(&server)
        .stream(&req)
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("chunks parse");

    let start = chunks
        .iter()
        .find(|c| {
            c.tool_calls
                .iter()
                .any(|t| t.id.as_deref() == Some("toolu_123"))
        })
        .expect("start chunk carries tool id");
    assert_eq!(
        start.tool_calls[0].function.name.as_deref(),
        Some("get_weather")
    );
    let partials: String = chunks
        .iter()
        .flat_map(|c| c.tool_calls.iter())
        .filter_map(|t| t.function.arguments.clone())
        .collect();
    assert!(
        partials.contains("Paris"),
        "input_json_delta fragments must flow through, got: {partials}"
    );
    let final_chunk = chunks.last().unwrap();
    assert_eq!(final_chunk.finish_reason, Some(FinishReason::ToolCalls));
    assert!(
        final_chunk
            .tool_calls
            .iter()
            .any(|t| t.id.as_deref() == Some("toolu_123")),
        "final chunk carries tool summary for SSE stops"
    );
}
