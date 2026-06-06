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
