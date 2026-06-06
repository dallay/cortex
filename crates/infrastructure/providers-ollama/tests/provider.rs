use futures::TryStreamExt;
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

#[tokio::test]
async fn complete_returns_error_on_http_failure() {
    // Ollama returns error on non-2xx status
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .respond_with(
            wiremock::ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "error": "internal server error"
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
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    };

    let result = provider.complete(&req).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Ollama error 500"));
}

// ---------------------------------------------------------------------------
// stream() tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stream_returns_chunks_from_sse_lines() {
    // Ollama streams JSON objects one per line
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .respond_with(wiremock::ResponseTemplate::new(200)
            .set_body_bytes(r#"{"model":"llama3","message":{"role":"assistant","content":"Hello"},"done":false}
{"model":"llama3","message":{"role":"assistant","content":", world"},"done":false}
{"model":"llama3","message":{"role":"assistant","content":"!"},"done":true,"prompt_eval_count":10,"eval_count":5}
"#)
            .append_header("content-type", "application/json"))
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
        stream: true,
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

    let stream = provider.stream(&req).await.unwrap();
    let chunks: Vec<_> = stream.try_collect().await.unwrap();

    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].delta, "Hello");
    assert_eq!(chunks[1].delta, ", world");
    assert_eq!(chunks[2].delta, "!");
    assert!(chunks[2].finish_reason.is_some());
    // Last chunk has usage
    assert!(chunks[2].usage.is_some());
    assert_eq!(chunks[2].usage.as_ref().unwrap().prompt_tokens, 10);
    assert_eq!(chunks[2].usage.as_ref().unwrap().completion_tokens, 5);
}

#[tokio::test]
async fn stream_returns_error_on_http_failure() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .respond_with(
            wiremock::ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "error": "internal server error"
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
        stream: true,
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

    let result = provider.stream(&req).await;
    // Result is Err because the HTTP call failed
    assert!(result.is_err());
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected error"),
    };
    assert!(err.to_string().contains("Ollama error 500"));
}

#[tokio::test]
async fn stream_handles_single_chunk() {
    // Test that stream properly handles a single complete JSON line
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_bytes(
                    r#"{"model":"llama3","message":{"role":"assistant","content":"Hi"},"done":true}
"#,
                )
                .append_header("content-type", "application/json"),
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
        stream: true,
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

    let stream = provider.stream(&req).await.unwrap();
    let chunks: Vec<_> = stream.try_collect().await.unwrap();

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].delta, "Hi");
    assert!(chunks[0].finish_reason.is_some());
}
