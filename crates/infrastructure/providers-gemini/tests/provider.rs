use futures::StreamExt;
use providers_gemini::{GeminiProvider, GeminiProviderConfig};
use rook_core::{
    ApiFormat, CompletionRequest, FinishReason, HealthStatus, ModelId, ProviderPort, Role,
};
use shared_kernel::{ModelId as KModelId, ProviderId, RequestId};

// ---------------------------------------------------------------------------
// health_check tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn health_check_returns_healthy_on_2xx() {
    // Gemini's /v1beta/models endpoint accepts x-goog-api-key. A 200
    // response means credentials are valid and the model catalog is
    // available.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1beta/models"))
        .and(wiremock::matchers::header("x-goog-api-key", "test-key"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    { "name": "models/gemini-2.0-flash" }
                ]
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

    let status = provider.health_check().await;
    assert!(matches!(status, HealthStatus::Healthy { .. }));
}

#[tokio::test]
async fn health_check_returns_warning_on_429() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1beta/models"))
        .respond_with(wiremock::ResponseTemplate::new(429))
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

    let status = provider.health_check().await;
    match status {
        HealthStatus::Warning { reason, .. } => {
            assert!(
                reason.to_lowercase().contains("rate limit"),
                "expected reason to mention rate limit, got: {reason}"
            );
        }
        other => panic!("expected Warning, got {other:?}"),
    }
}

#[tokio::test]
async fn health_check_returns_unhealthy_on_401() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1beta/models"))
        .respond_with(wiremock::ResponseTemplate::new(401))
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
    match status {
        HealthStatus::Unhealthy { error, .. } => {
            assert!(
                error.contains("auth rejected") && error.contains("401"),
                "expected 'auth rejected' and '401' in error, got: {error}"
            );
        }
        other => panic!("expected Unhealthy, got {other:?}"),
    }
}

#[tokio::test]
async fn health_check_returns_unhealthy_on_500() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1beta/models"))
        .respond_with(wiremock::ResponseTemplate::new(500))
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

    let status = provider.health_check().await;
    match status {
        HealthStatus::Unhealthy { error, .. } => {
            assert!(
                error.contains("500"),
                "expected '500' in error, got: {error}"
            );
        }
        other => panic!("expected Unhealthy, got {other:?}"),
    }
}

#[tokio::test]
async fn health_check_returns_unhealthy_on_network_error() {
    // Port 1 is reserved and refuses connections — network error path.
    let provider = GeminiProvider::new(GeminiProviderConfig {
        id: ProviderId::new("gemini-test"),
        api_key: "test-key".to_string(),
        base_url: Some("http://127.0.0.1:1".to_string()),
        models: vec![ModelId::new("gemini-2.0-flash")],
        timeout_secs: 2,
    })
    .unwrap();

    let status = provider.health_check().await;
    match status {
        HealthStatus::Unhealthy { error, .. } => {
            assert!(
                error.contains("/v1beta/models") || error.contains("failed"),
                "expected error to mention the probe path or 'failed', got: {error}"
            );
        }
        other => panic!("expected Unhealthy, got {other:?}"),
    }
}

#[tokio::test]
async fn health_check_returns_warning_on_no_key() {
    // No API key configured — the probe must short-circuit before
    // touching the network.
    let server = wiremock::MockServer::start().await;
    // No wiremock routes mounted.
    let provider = GeminiProvider::new(GeminiProviderConfig {
        id: ProviderId::new("gemini-test"),
        api_key: String::new(),
        base_url: Some(server.uri()),
        models: vec![ModelId::new("gemini-2.0-flash")],
        timeout_secs: 10,
    })
    .unwrap();

    let status = provider.health_check().await;
    match status {
        HealthStatus::Warning { reason, .. } => {
            assert!(
                reason.to_lowercase().contains("no api key"),
                "expected reason to mention no API key, got: {reason}"
            );
        }
        other => panic!("expected Warning, got {other:?}"),
    }
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
            tool_calls: vec![],
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
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    };

    let result = provider.complete(&req).await;
    assert!(result.is_ok());
    let resp = result.unwrap();
    assert_eq!(resp.content, "Hello, world!");
    assert_eq!(resp.finish_reason, Some(FinishReason::Stop));
    // T5.3: Gemini parses promptTokenCount and candidatesTokenCount
    assert_eq!(resp.usage.prompt_tokens, 20);
    assert_eq!(resp.usage.completion_tokens, 12);
    // Cache/reasoning tokens not available in Gemini API
    assert_eq!(resp.usage.cache_read_tokens, None);
    assert_eq!(resp.usage.cache_creation_tokens, None);
    assert_eq!(resp.usage.reasoning_tokens, None);
}

#[tokio::test]
async fn test_gemini_provider_basic_methods() {
    let config = GeminiProviderConfig {
        id: ProviderId::new("gemini-1"),
        api_key: "".to_string(),
        base_url: None,
        models: vec![KModelId::new("gemini-1.5-pro")],
        timeout_secs: 5,
    };

    let provider = GeminiProvider::new(config).unwrap();

    assert_eq!(provider.id().as_str(), "gemini-1");
    assert_eq!(provider.supported_models().len(), 1);
    assert_eq!(provider.api_format(), ApiFormat::OpenAI);
    assert!(!provider.is_available());

    // health check with empty key
    let health = provider.health_check().await;
    match health {
        HealthStatus::Warning { reason, .. } => {
            assert!(reason.contains("No API key configured"));
        }
        _ => panic!("expected HealthStatus::Warning"),
    }

    // stream returns unimplemented error
    let req = CompletionRequest {
        id: shared_kernel::RequestId::new(),
        model: ModelId::new("gemini-1.5-pro"),
        messages: vec![rook_core::Message {
            role: rook_core::Role::User,
            content: rook_core::MessageContent::Text("Hello".to_string()),
            tool_calls: vec![],
        }],
        stream: false,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".to_string(),
            cacheable: false,
            priority: 1,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: Default::default(),
    };

    let stream_res = provider.stream(&req).await;
    assert!(stream_res.is_ok());
    let mut st = stream_res.unwrap();
    let chunk = st.next().await;
    assert!(chunk.unwrap().is_err());
}

#[tokio::test]
async fn test_gemini_provider_with_wiremock() {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    let config = GeminiProviderConfig {
        id: ProviderId::new("gemini-test"),
        api_key: "test-key-123".to_string(),
        base_url: Some(mock_server.uri()),
        models: vec![KModelId::new("gemini-1.5-pro")],
        timeout_secs: 5,
    };

    let provider = GeminiProvider::new(config).unwrap();
    assert!(provider.is_available());

    // Mock health check (GET /v1beta/models)
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .and(header("x-goog-api-key", "test-key-123"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let health = provider.health_check().await;
    match health {
        HealthStatus::Healthy { provider, .. } => {
            assert_eq!(provider.as_str(), "gemini-test");
        }
        _ => panic!("expected HealthStatus::Healthy"),
    }

    for (reason, expected) in [
        ("STOP", Some(FinishReason::Stop)),
        ("MAX_TOKENS", Some(FinishReason::Length)),
        ("SAFETY", Some(FinishReason::ContentFilter)),
        ("RECITATION", Some(FinishReason::ContentFilter)),
        ("OTHER", None),
        ("UNKNOWN", None),
    ] {
        // Mock generateContent
        let response_body = serde_json::json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{ "text": "Hello from Gemini!" }]
                },
                "finishReason": reason,
                "index": 0
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 5,
                "totalTokenCount": 15
            }
        });

        let _completion_mock = Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-1.5-pro:generateContent"))
            .and(header("x-goog-api-key", "test-key-123"))
            .and(body_partial_json(serde_json::json!({
                "systemInstruction": { "parts": [{ "text": "You are a helpful assistant." }] },
                "contents": [{ "role": "user", "parts": [{ "text": "Hi" }] }]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
            .expect(1)
            .mount_as_scoped(&mock_server)
            .await;

        let req = CompletionRequest {
            id: shared_kernel::RequestId::new(),
            model: ModelId::new("gemini-1.5-pro"),
            messages: vec![
                rook_core::Message {
                    role: rook_core::Role::System,
                    content: rook_core::MessageContent::Text(
                        "You are a helpful assistant.".to_string(),
                    ),
                    tool_calls: vec![],
                },
                rook_core::Message {
                    role: rook_core::Role::User,
                    content: rook_core::MessageContent::Text("Hi".to_string()),
                    tool_calls: vec![],
                },
            ],
            stream: false,
            max_tokens: None,
            temperature: None,
            tools: None,
            tool_choice: None,
            metadata: rook_core::RequestMetadata {
                origin: "test".to_string(),
                cacheable: false,
                priority: 1,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: Default::default(),
        };

        let resp = provider.complete(&req).await.unwrap();
        assert_eq!(resp.content, "Hello from Gemini!");
        assert_eq!(resp.usage.prompt_tokens, 10);
        assert_eq!(resp.usage.completion_tokens, 5);
        assert_eq!(resp.usage.total_tokens, 15);
        assert_eq!(resp.finish_reason, expected, "finish reason: {reason}");
    }
}
