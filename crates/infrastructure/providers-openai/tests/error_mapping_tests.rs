use providers_openai::{OpenAIProvider, OpenAIProviderConfig};
use rook_core::{
    ApiKeyRestrictions, CompletionRequest, ModelId, ProviderPort, RequestMetadata, Role,
};
use shared_kernel::{ProviderId, RequestId};

// ---------------------------------------------------------------------------
// Test helpers — extracted to reduce duplication and drift
// ---------------------------------------------------------------------------

fn test_provider(base_url: String) -> OpenAIProvider {
    test_provider_with_key(base_url, "sk-invalid".to_string())
}

fn test_provider_with_key(base_url: String, api_key: String) -> OpenAIProvider {
    OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key,
        base_url,
        models: vec![ModelId::new("gpt-4")],
        timeout_secs: 10,
    })
    .unwrap()
}

fn base_request(stream: bool) -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("gpt-4"),
        messages: vec![rook_core::Message {
            role: Role::User,
            content: rook_core::MessageContent::Text("Hi".to_string()),
        }],
        stream,
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

// ---------------------------------------------------------------------------
// Error mapping tests — map_openai_http_error via public API
// ---------------------------------------------------------------------------

#[tokio::test]
async fn complete_returns_auth_error_on_401() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(401)
                .set_body_json(serde_json::json!({"error": "invalid_api_key"})),
        )
        .mount(&server)
        .await;

    let provider = test_provider(server.uri());
    let req = base_request(false);

    let result = provider.complete(&req).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("authentication"),
        "Expected auth error, got: {}",
        err
    );
}

#[tokio::test]
async fn complete_returns_rate_limit_error_on_429() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(429)
                .insert_header("retry-after", "60")
                .insert_header("x-ratelimit-reset", "120")
                .set_body_json(serde_json::json!({"error": "rate_limit_exceeded"})),
        )
        .mount(&server)
        .await;

    let provider = test_provider_with_key(server.uri(), "sk-test".to_string());
    let req = base_request(false);

    let result = provider.complete(&req).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_msg = err.to_string();
    // Error message contains "rate limited" or "429"
    assert!(
        err_msg.contains("rate limited") || err_msg.contains("429"),
        "Expected rate limit error, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn complete_returns_invalid_request_on_400() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": {"message": "bad request", "type": "invalid_request_error"}
            })),
        )
        .mount(&server)
        .await;

    let provider = test_provider_with_key(server.uri(), "sk-test".to_string());
    let req = base_request(false);

    let result = provider.complete(&req).await;
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
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(400).set_body_string(
            r#"{"error": "invalid_request", "api_key": "sk-12345secret", "token": "abc123"}"#,
        ))
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
            content: rook_core::MessageContent::Text("Hi".to_string()),
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
    };

    let result = provider.complete(&req).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_msg = err.to_string();
    // Sensitive data should be redacted
    assert!(
        !err_msg.contains("sk-12345secret"),
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

#[tokio::test]
async fn stream_returns_error_on_429() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(429)
                .insert_header("retry-after", "30")
                .set_body_json(serde_json::json!({"error": "rate_limit_exceeded"})),
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
            content: rook_core::MessageContent::Text("Hi".to_string()),
        }],
        stream: true,
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
    };

    let result = provider.stream(&req).await;
    // Use match to avoid Debug requirement on Ok type (BoxStream doesn't implement Debug)
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
