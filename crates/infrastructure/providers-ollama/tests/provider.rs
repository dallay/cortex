use futures::TryStreamExt;
use providers_ollama::{OllamaProvider, OllamaProviderConfig};
use rook_core::{CompletionRequest, HealthStatus, ModelId, ProviderPort, Role};
use shared_kernel::{ProviderId, RequestId};

#[tokio::test]
async fn health_check_returns_healthy_on_local_ollama() {
    // Local Ollama: GET /api/tags returns 200, no api_key configured,
    // so the probe stops after step 1 and reports Healthy.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/tags"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
            r#"{"models":[{"name":"llama3","model":"llama3","modified_at":"","size":1,"digest":"","details":{}}]}"#,
        ))
        .mount(&server)
        .await;

    let provider = OllamaProvider::new(OllamaProviderConfig {
        id: ProviderId::new("ollama-test"),
        base_url: server.uri(),
        models: vec![ModelId::new("llama3")],
        timeout_secs: 10,
        api_key: None,
    })
    .unwrap();

    let status = provider.health_check().await;
    assert!(matches!(status, HealthStatus::Healthy { latency_ms, .. } if latency_ms < 5_000));
}

#[tokio::test]
async fn health_check_returns_unhealthy_on_5xx() {
    // Server returned 500 on /api/tags: server is reachable but the
    // probe is rejected. No api_key, so we never reach step 2.
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
        api_key: None,
    })
    .unwrap();

    let status = provider.health_check().await;
    assert!(
        matches!(status, HealthStatus::Unhealthy { ref error, .. } if error.contains("500")),
        "expected Unhealthy with 500, got {status:?}"
    );
}

#[tokio::test]
async fn health_check_ollama_cloud_healthy_with_valid_api_key() {
    // Ollama Cloud happy path: /api/tags is public (200), then a tiny
    // POST /api/chat with valid Bearer is accepted (200). The user
    // does NOT pay meaningful tokens — `stream: false` + a 2-token
    // prompt. Just a few input tokens.
    let server = wiremock::MockServer::start().await;
    // Step 1: /api/tags is public.
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/tags"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
            r#"{"models":[{"name":"gpt-oss:20b","model":"gpt-oss:20b","modified_at":"","size":1,"digest":"","details":{}}]}"#,
        ))
        .mount(&server)
        .await;
    // Step 2: /api/chat requires auth — valid Bearer returns 200.
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer valid-test-key",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "gpt-oss:20b",
                "message": { "role": "assistant", "content": "hi" },
                "done": true
            })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let provider = OllamaProvider::new(OllamaProviderConfig {
        id: ProviderId::new("ollama-cloud-test"),
        base_url: server.uri(),
        models: vec![ModelId::new("gpt-oss:20b")],
        timeout_secs: 10,
        api_key: Some("valid-test-key".to_string()),
    })
    .unwrap();

    let status = provider.health_check().await;
    assert!(
        matches!(status, HealthStatus::Healthy { latency_ms, .. } if latency_ms < 5_000),
        "expected Healthy, got {status:?}"
    );
}

#[tokio::test]
async fn health_check_ollama_cloud_unhealthy_with_invalid_api_key() {
    // Ollama Cloud rejects an invalid Bearer with 401 on /api/chat.
    // /api/tags still returns 200 (public), so we have to drill into
    // step 2 to catch the bad auth. The error message must surface
    // "auth rejected" so the dashboard can show a clear message
    // instead of the generic "Unhealthy: HTTP 401".
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/tags"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
            r#"{"models":[{"name":"gpt-oss:20b","model":"gpt-oss:20b","modified_at":"","size":1,"digest":"","details":{}}]}"#,
        ))
        .mount(&server)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer invalid-key",
        ))
        .respond_with(wiremock::ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let provider = OllamaProvider::new(OllamaProviderConfig {
        id: ProviderId::new("ollama-cloud-test"),
        base_url: server.uri(),
        models: vec![ModelId::new("gpt-oss:20b")],
        timeout_secs: 10,
        api_key: Some("invalid-key".to_string()),
    })
    .unwrap();

    let status = provider.health_check().await;
    assert!(
        matches!(status, HealthStatus::Unhealthy { ref error, .. } if error.contains("auth rejected")),
        "expected Unhealthy with 'auth rejected' message, got {status:?}"
    );
}

#[tokio::test]
async fn health_check_returns_unhealthy_on_connection_refused() {
    // Port 1 is reserved and refuses connections — network error path.
    let provider = OllamaProvider::new(OllamaProviderConfig {
        id: ProviderId::new("ollama-test"),
        base_url: "http://127.0.0.1:1".to_string(),
        models: vec![ModelId::new("llama3")],
        timeout_secs: 2,
        api_key: None,
    })
    .unwrap();

    let status = provider.health_check().await;
    assert!(
        matches!(status, HealthStatus::Unhealthy { ref error, .. } if error.contains("failed")),
        "expected Unhealthy with network error, got {status:?}"
    );
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
        api_key: None,
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
        api_key: None,
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
        api_key: None,
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
        api_key: None,
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
        api_key: None,
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
        api_key: None,
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

// ---------------------------------------------------------------------------
// Bearer auth tests (Ollama Cloud)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn complete_sends_bearer_header_when_api_key_configured() {
    // Ollama Cloud requires `Authorization: Bearer <key>`. The mock below
    // is configured to ONLY match when the header is present with the
    // expected value — if the header is missing or wrong, wiremock
    // returns a 404, which fails the request and the test.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer my-cloud-key",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "llama3",
                "message": { "role": "assistant", "content": "ok" },
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
        api_key: Some("my-cloud-key".to_string()),
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
    assert!(
        result.is_ok(),
        "expected ok with Bearer auth, got {result:?}"
    );
    // If the mock had returned 404 (no header match), the body parse would
    // also fail and complete() would return Err. The ok result above is
    // sufficient verification that the header was sent and matched.
}

#[tokio::test]
async fn complete_does_not_send_auth_header_when_api_key_is_none() {
    // Local Ollama: no Authorization header should be sent. We inspect
    // the received request after the call to assert the header is absent.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "llama3",
                "message": { "role": "assistant", "content": "ok" },
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
        api_key: None,
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
    assert!(result.is_ok(), "expected ok, got {result:?}");

    let received = server.received_requests().await.expect("received requests");
    assert_eq!(received.len(), 1);
    assert!(
        !received[0].headers.contains_key("Authorization"),
        "Authorization header must NOT be sent when api_key is None; got: {:?}",
        received[0].headers.get("Authorization")
    );
}

#[tokio::test]
async fn complete_does_not_send_auth_header_when_api_key_is_empty() {
    // Defensive: empty-string api_key is treated as no auth (frontend
    // form bugs can submit an uninitialized key).
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "llama3",
                "message": { "role": "assistant", "content": "ok" },
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
        api_key: Some(String::new()),
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
    assert!(result.is_ok(), "expected ok, got {result:?}");

    let received = server.received_requests().await.expect("received requests");
    assert_eq!(received.len(), 1);
    assert!(
        !received[0].headers.contains_key("Authorization"),
        "Authorization header must NOT be sent when api_key is empty; got: {:?}",
        received[0].headers.get("Authorization")
    );
}
