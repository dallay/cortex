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
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
            cache_control_header: None,
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
            cache_control_header: None,
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

#[tokio::test]
async fn complete_parses_x_cache_hit_header() {
    // Phase 5: Task 5.1-5.2 - Parse x-cache: hit header and set cache_hit: Some(true)
    let mock_server = MockServer::start().await;

    let response_body = serde_json::json!({
        "id": "msg_cached",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "Cached response"}],
        "model": "claude-3-5-sonnet-20241022",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_read_input_tokens": 100
        }
    });

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(response_body)
                .insert_header("x-cache", "hit"),
        )
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
            content: MessageContent::Text("Test".to_string()),
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
    assert_eq!(
        resp.cache_hit,
        Some(true),
        "x-cache: hit should set cache_hit to Some(true)"
    );
}

#[tokio::test]
async fn complete_parses_x_cache_miss_header() {
    // Phase 5: Task 5.3 - Parse x-cache: miss header and set cache_hit: Some(false)
    let mock_server = MockServer::start().await;

    let response_body = serde_json::json!({
        "id": "msg_uncached",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "Fresh response"}],
        "model": "claude-3-5-sonnet-20241022",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50
        }
    });

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(response_body)
                .insert_header("x-cache", "miss"),
        )
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
            content: MessageContent::Text("Test".to_string()),
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
    assert_eq!(
        resp.cache_hit,
        Some(false),
        "x-cache: miss should set cache_hit to Some(false)"
    );
}

#[tokio::test]
async fn complete_handles_missing_x_cache_header() {
    // Phase 5: Task 5.7 - Missing x-cache header defaults to None
    let mock_server = MockServer::start().await;

    let response_body = serde_json::json!({
        "id": "msg_no_cache_header",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "Response without cache header"}],
        "model": "claude-3-5-sonnet-20241022",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50
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
            content: MessageContent::Text("Test".to_string()),
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
    assert_eq!(
        resp.cache_hit, None,
        "missing x-cache header should leave cache_hit as None"
    );
}

#[tokio::test]
async fn complete_handles_malformed_x_cache_header() {
    // Phase 5: Task 5.7 - Malformed x-cache header defaults to None with warning
    let mock_server = MockServer::start().await;

    let response_body = serde_json::json!({
        "id": "msg_malformed",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "Response with malformed cache header"}],
        "model": "claude-3-5-sonnet-20241022",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50
        }
    });

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(response_body)
                .insert_header("x-cache", "invalid-value"),
        )
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
            content: MessageContent::Text("Test".to_string()),
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
    assert_eq!(
        resp.cache_hit, None,
        "malformed x-cache header should leave cache_hit as None"
    );
}
