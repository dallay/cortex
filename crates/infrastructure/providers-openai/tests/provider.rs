use futures::StreamExt;
use providers_openai::provider::{OpenAIProvider, OpenAIProviderConfig};
use rook_core::{
    CompletionRequest, EndpointPolicy, FinishReason, HealthStatus, ModelId, ProviderPort, Role,
};
use shared_kernel::{ProviderId, RequestId};

fn stream_request() -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("gpt-4"),
        messages: vec![rook_core::Message {
            tool_calls: vec![],
            role: Role::User,
            content: rook_core::MessageContent::Text("test".to_string()),
        }],
        stream: true,
        max_tokens: None,
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
    }
}

async fn stream_results_from_body(
    body: String,
) -> Vec<Result<rook_core::StreamChunk, shared_kernel::CortexError>> {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(body))
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

    provider
        .stream(&stream_request())
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await
}

#[tokio::test]
async fn responses_omitted_tool_ids_reach_generation_in_both_modes() {
    use serde_json::json;
    for stream in [false, true] {
        for origin in ["responses", "openai", "anthropic"] {
            let server = wiremock::MockServer::start().await;
            let calls = json!([{"index":0,"function":{"name":"lookup","arguments":"{}"}}, {"index":1,"function":{"name":"search","arguments":"{}"}}]);
            let usage = json!({"prompt_tokens":3,"completion_tokens":2,"total_tokens":5});
            let body = if stream {
                format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"model":"gpt-4","choices":[{"delta":{"tool_calls":calls},"finish_reason":"tool_calls"}],"usage":usage})
                )
            } else {
                json!({"model":"gpt-4","choices":[{"message":{"tool_calls":calls},"finish_reason":"tool_calls"}],"usage":usage}).to_string()
            };
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(
                    body,
                    if stream {
                        "text/event-stream"
                    } else {
                        "application/json"
                    },
                ))
                .mount(&server)
                .await;
            let provider = OpenAIProvider::new(OpenAIProviderConfig {
                id: ProviderId::new("openai-test"),
                api_key: "test".into(),
                base_url: server.uri(),
                models: vec![ModelId::new("gpt-4")],
                timeout_secs: 10,
            })
            .unwrap();
            let mut req = stream_request();
            req.stream = stream;
            req.metadata.origin = origin.into();
            if stream {
                let chunks = provider
                    .stream(&req)
                    .await
                    .unwrap()
                    .collect::<Vec<_>>()
                    .await;
                if origin == "responses" {
                    let chunk = chunks[0]
                        .as_ref()
                        .expect("omitted IDs must reach Responses generation");
                    assert_eq!(chunk.tool_calls.len(), 2);
                    assert!(chunk.tool_calls.iter().all(|call| call.id.is_none()));
                    assert_eq!(chunk.usage.as_ref().unwrap().total_tokens, 5);
                    assert!(chunks.iter().all(Result::is_ok));
                } else {
                    assert!(chunks.iter().any(Result::is_err));
                }
            } else {
                let result = provider.complete(&req).await;
                if origin == "responses" {
                    let response = result.expect("omitted IDs must reach Responses generation");
                    assert_eq!(response.tool_calls.len(), 2);
                    assert!(response.tool_calls.iter().all(|call| call.id.is_none()));
                    assert_eq!(response.usage.total_tokens, 5);
                } else {
                    assert!(result.is_err());
                }
            }
        }
    }
}

#[tokio::test]
async fn responses_tool_id_type_matrix_retains_usage_and_legacy_rejection() {
    use serde_json::json;
    for id in [
        json!(null),
        json!(""),
        json!("call_valid"),
        json!("bad\n"),
        json!("x".repeat(257)),
        json!(123),
        json!(-1),
        json!(1.5),
        json!(true),
        json!(false),
        json!([]),
        json!({"nested":[{}, []]}),
    ] {
        for stream in [false, true] {
            let server = wiremock::MockServer::start().await;
            let call = json!({"index":0,"id":id,"function":{"name":"lookup","arguments":"{}"}});
            let usage = json!({"prompt_tokens":3,"completion_tokens":2,"total_tokens":5});
            // A nonterminal frame exposes normalization without finish-time validation.
            let body = if stream {
                format!(
                    "data: {}\n\n",
                    json!({"model":"gpt-4","choices":[{"delta":{"tool_calls":[call]},"finish_reason":null}],"usage":usage})
                )
            } else {
                json!({"model":"gpt-4","choices":[{"message":{"tool_calls":[call]},"finish_reason":"tool_calls"}],"usage":usage}).to_string()
            };
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(
                    body,
                    if stream {
                        "text/event-stream"
                    } else {
                        "application/json"
                    },
                ))
                .mount(&server)
                .await;
            let provider = OpenAIProvider::new(OpenAIProviderConfig {
                id: ProviderId::new("openai-test"),
                api_key: "test".into(),
                base_url: server.uri(),
                models: vec![ModelId::new("gpt-4")],
                timeout_secs: 10,
            })
            .unwrap();
            for origin in ["responses", "openai", "anthropic"] {
                let mut req = stream_request();
                req.stream = stream;
                req.metadata.origin = origin.into();
                if stream {
                    let chunks = provider
                        .stream(&req)
                        .await
                        .unwrap()
                        .collect::<Vec<_>>()
                        .await;
                    if id.as_str().is_some_and(|id| id.len() > 256) {
                        assert!(chunks[0].is_err());
                        continue;
                    }
                    if origin != "responses" && !id.is_string() && !id.is_null() {
                        assert!(chunks[0].is_err());
                        continue;
                    }
                    let chunk = chunks[0].as_ref().unwrap();
                    assert_eq!(chunk.usage.as_ref().unwrap().total_tokens, 5);
                    assert_eq!(
                        chunk.tool_calls[0].id.as_deref(),
                        if id.is_null() {
                            None
                        } else {
                            Some(id.as_str().unwrap_or_default())
                        }
                    );
                } else {
                    let response = provider.complete(&req).await;
                    if origin != "responses" && !id.is_string() {
                        assert!(response.is_err());
                        continue;
                    }
                    let response = response.unwrap();
                    assert_eq!(
                        response.tool_calls[0].id.as_deref(),
                        Some(id.as_str().unwrap_or_default())
                    );
                    assert_eq!(response.usage.total_tokens, 5);
                }
            }
        }
    }
}

#[tokio::test]
async fn nullable_and_omitted_delta_ids_do_not_change_valid_tool_identity() {
    use serde_json::json;
    let server = wiremock::MockServer::start().await;
    let frames = [
        json!({"model":"gpt-4","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_valid","function":{"name":"lookup","arguments":"{"}}]},"finish_reason":null}]}),
        json!({"model":"gpt-4","choices":[{"delta":{"tool_calls":[{"index":0,"id":null,"function":{"name":null,"arguments":"\"key\":"}}]},"finish_reason":null}]}),
        json!({"model":"gpt-4","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"","arguments":"1}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}),
    ];
    let body = frames
        .iter()
        .map(|frame| format!("data: {frame}\n\n"))
        .collect::<String>()
        + "data: [DONE]\n\n";
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;
    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: "test".into(),
        base_url: server.uri(),
        models: vec![ModelId::new("gpt-4")],
        timeout_secs: 10,
    })
    .unwrap();
    for origin in ["responses", "openai", "anthropic"] {
        let mut req = stream_request();
        req.metadata.origin = origin.into();
        let chunks = provider
            .stream(&req)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(Result::is_ok), "{chunks:?}");
        assert_eq!(
            chunks[0].as_ref().unwrap().tool_calls[0].id.as_deref(),
            Some("call_valid")
        );
        assert!(chunks[1..]
            .iter()
            .all(|chunk| chunk.as_ref().unwrap().tool_calls[0].id.is_none()));
        assert_eq!(
            chunks[2].as_ref().unwrap().finish_reason,
            Some(FinishReason::ToolCalls)
        );
    }
}

#[tokio::test]
async fn legacy_id_validation_still_includes_unselected_choices() {
    use serde_json::json;
    for stream in [false, true] {
        let server = wiremock::MockServer::start().await;
        let invalid = json!({"index":0,"id":123,"function":{"name":"lookup","arguments":"{}"}});
        let body = if stream {
            format!(
                "data: {}\n\n",
                json!({"model":"gpt-4","choices":[
                    {"delta":{"content":"selected"},"finish_reason":"stop"},
                    {"delta":{"tool_calls":[invalid]},"finish_reason":"tool_calls"}
                ]})
            )
        } else {
            json!({"model":"gpt-4","choices":[
                {"message":{"content":"selected"},"finish_reason":"stop"},
                {"message":{"tool_calls":[invalid]},"finish_reason":"tool_calls"}
            ],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}})
            .to_string()
        };
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(
                body,
                if stream {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            ))
            .mount(&server)
            .await;
        let provider = OpenAIProvider::new(OpenAIProviderConfig {
            id: ProviderId::new("openai-test"),
            api_key: "test".into(),
            base_url: server.uri(),
            models: vec![ModelId::new("gpt-4")],
            timeout_secs: 10,
        })
        .unwrap();
        for origin in ["openai", "anthropic"] {
            let mut req = stream_request();
            req.stream = stream;
            req.metadata.origin = origin.into();
            if stream {
                let chunks = provider
                    .stream(&req)
                    .await
                    .unwrap()
                    .collect::<Vec<_>>()
                    .await;
                assert!(
                    chunks[0].is_err(),
                    "legacy parsing rejects invalid types in every choice"
                );
            } else {
                assert!(
                    provider.complete(&req).await.is_err(),
                    "legacy parsing rejects invalid types in every choice"
                );
            }
        }
    }
}

#[tokio::test]
async fn responses_unknown_finish_rejects_frame_and_retains_usage() {
    use serde_json::json;
    for prior_finish in [false, true] {
        for reason in [
            None,
            Some(json!(null)),
            Some(json!("unknown")),
            Some(json!("")),
        ] {
            let server = wiremock::MockServer::start().await;
            let mut choice = json!({"delta":{}});
            if let Some(reason) = &reason {
                choice["finish_reason"] = reason.clone();
            }
            let last = json!({"model":"gpt-4","choices":[choice],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}});
            let first = json!({"model":"gpt-4","choices":[{"delta":{"content":"prior"},"finish_reason":"stop"}]});
            let sse = format!(
                "{}data: {last}\n\n",
                if prior_finish {
                    format!("data: {first}\n\n")
                } else {
                    String::new()
                }
            );
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/chat/completions"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(sse))
                .mount(&server)
                .await;
            let provider = OpenAIProvider::new(OpenAIProviderConfig {
                id: ProviderId::new("openai-test"),
                api_key: "test".into(),
                base_url: server.uri(),
                models: vec![ModelId::new("gpt-4")],
                timeout_secs: 10,
            })
            .unwrap();
            for origin in ["responses", "openai", "anthropic"] {
                let mut req = stream_request();
                req.metadata.origin = origin.into();
                let chunks = provider
                    .stream(&req)
                    .await
                    .unwrap()
                    .collect::<Vec<_>>()
                    .await;
                assert_eq!(chunks.len(), 1 + usize::from(prior_finish));
                let last = chunks.last().unwrap();
                if origin == "responses" && reason.as_ref().is_some_and(|r| !r.is_null()) {
                    let error = last
                        .as_ref()
                        .expect_err("explicit unknown finish must fail");
                    assert!(error.is_retryable());
                    let usage = rook_core::StreamValidationError::usage(error).unwrap();
                    assert_eq!(usage.total_tokens, 5);
                    assert_eq!(usage.prompt_tokens, 3);
                    assert_eq!(usage.completion_tokens, 2);
                } else {
                    let chunk = last.as_ref().unwrap();
                    assert!(chunk.finish_reason.is_none());
                    assert!(chunk.delta.is_empty());
                    assert_eq!(chunk.usage.as_ref().unwrap().total_tokens, 5);
                }
            }
        }
    }
}

#[tokio::test]
async fn rejected_frame_retains_usage_without_publishing_invalid_arguments() {
    for observe_error in [false, true] {
        let frame = serde_json::json!({"model":"gpt-4","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_bad","function":{"name":if observe_error {"x".repeat(4097)} else {"lookup".into()},"arguments":"{"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5,"prompt_tokens_details":{"cached_tokens":1},"completion_tokens_details":{"reasoning_tokens":1}}});
        let results = stream_results_from_body(format!("data: {frame}\n\ndata: [DONE]\n\n")).await;
        assert!(
            results.iter().all(Result::is_err),
            "ningún argumento inválido se publica como éxito"
        );
        let error = results[0].as_ref().unwrap_err();
        assert!(error.is_retryable());
        assert!(!error.is_invalid_request());
        let usage =
            rook_core::StreamValidationError::usage(error).expect("uso del frame rechazado");
        assert_eq!(usage.prompt_tokens, 3);
        assert_eq!(usage.completion_tokens, 2);
        assert_eq!(usage.total_tokens, 5);
        assert_eq!(usage.cache_read_tokens, Some(1));
        assert_eq!(usage.reasoning_tokens, Some(1));
        assert!(error.to_string().contains(if observe_error {
            "name limit"
        } else {
            "invalid tool-call arguments"
        }));
    }
}

// ---------------------------------------------------------------------------
// health_check tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn health_check_returns_healthy_on_2xx() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/models"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer sk-test",
        ))
        .respond_with(wiremock::ResponseTemplate::new(200))
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

    let status = provider.health_check().await;
    assert!(matches!(status, HealthStatus::Healthy { .. }));
}

#[tokio::test]
async fn health_check_returns_warning_on_429() {
    // 429 means credentials are valid but the upstream is rate-limiting
    // the caller. The dashboard should show a yellow alert and keep
    // Save enabled (valid: true, status: "warning").
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/models"))
        .respond_with(wiremock::ResponseTemplate::new(429))
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
        .and(wiremock::matchers::path("/models"))
        .respond_with(wiremock::ResponseTemplate::new(401))
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
async fn health_check_returns_unhealthy_on_403() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/models"))
        .respond_with(wiremock::ResponseTemplate::new(403))
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

    let status = provider.health_check().await;
    match status {
        HealthStatus::Unhealthy { error, .. } => {
            assert!(
                error.contains("auth rejected") && error.contains("403"),
                "expected 'auth rejected' and '403' in error, got: {error}"
            );
        }
        other => panic!("expected Unhealthy, got {other:?}"),
    }
}

#[tokio::test]
async fn health_check_returns_unhealthy_on_500() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/models"))
        .respond_with(wiremock::ResponseTemplate::new(500))
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
    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: "sk-test".to_string(),
        base_url: "http://127.0.0.1:1".to_string(),
        models: vec![ModelId::new("gpt-4")],
        timeout_secs: 2,
    })
    .unwrap();

    let status = provider.health_check().await;
    match status {
        HealthStatus::Unhealthy { error, .. } => {
            assert!(
                error.contains("/v1/models") || error.contains("models"),
                "expected error to mention the probe path, got: {error}"
            );
        }
        other => panic!("expected Unhealthy, got {other:?}"),
    }
}

#[tokio::test]
async fn health_check_returns_warning_on_no_key() {
    // No API key configured — the user can add one later. Surface as a
    // yellow warning so the dashboard keeps Save enabled.
    let server = wiremock::MockServer::start().await;
    // No wiremock routes mounted — the probe must short-circuit before
    // touching the network.
    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: String::new(),
        base_url: server.uri(),
        models: vec![ModelId::new("gpt-4")],
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
async fn guarded_compatible_endpoint_blocks_untrusted_loopback_before_sending_credentials() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/models"))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let mut config = OpenAIProviderConfig {
        id: ProviderId::new("compatible-test"),
        api_key: "secret-marker".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("physical-model")],
        timeout_secs: 2,
    };

    let policy = EndpointPolicy::default();
    assert!(OpenAIProvider::new_with_endpoint_policy(config.clone(), policy).is_err());
    config.base_url = format!("http://localhost:{}", server.address().port());
    let provider = OpenAIProvider::new_with_endpoint_policy(
        config,
        EndpointPolicy {
            allowed_origins: vec![format!("http://localhost:{}", server.address().port())],
        },
    )
    .expect("explicit loopback opt-in is accepted");
    assert!(matches!(
        provider.health_check().await,
        HealthStatus::Healthy { .. } | HealthStatus::Warning { .. }
    ));
    assert_eq!(server.received_requests().await.expect("requests").len(), 1);
}

#[tokio::test]
async fn guarded_compatible_endpoint_does_not_follow_redirects_or_forward_credentials() {
    let server = wiremock::MockServer::start().await;
    let redirect_target = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&redirect_target)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/steal", redirect_target.uri())),
        )
        .mount(&server)
        .await;
    let provider = OpenAIProvider::new_with_endpoint_policy(
        OpenAIProviderConfig {
            id: ProviderId::new("compatible-redirect-test"),
            api_key: "secret-marker".to_string(),
            base_url: server.uri(),
            models: vec![ModelId::new("physical-model")],
            timeout_secs: 2,
        },
        EndpointPolicy {
            allowed_origins: vec![server.uri()],
        },
    )
    .expect("loopback endpoint configured explicitly");

    let error = provider
        .complete(&stream_request())
        .await
        .expect_err("redirect response is not followed");
    assert!(!error.to_string().contains("secret-marker"));
    assert_eq!(server.received_requests().await.expect("requests").len(), 1);
    assert!(redirect_target
        .received_requests()
        .await
        .expect("redirect target requests")
        .is_empty());
}

#[tokio::test]
async fn error_body_redacts_non_empty_secrets_shorter_than_eight_characters() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(500).set_body_string("invalid key: k7"))
        .mount(&server)
        .await;
    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: "k7".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("gpt-4")],
        timeout_secs: 10,
    })
    .unwrap();

    let error = provider
        .complete(&stream_request())
        .await
        .expect_err("the upstream error response is returned as an error");

    assert!(!error.to_string().contains("k7"));
    assert!(error.to_string().contains("invalid key"));
}

#[tokio::test]
async fn trailing_slashes_in_base_url_are_removed_for_chat_and_health_requests() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-123",
                "model": "gpt-4",
                "choices": [{
                    "message": { "role": "assistant", "content": "Hello" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
            })),
        )
        .mount(&server)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/models"))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: "sk-test".to_string(),
        base_url: format!("{}///", server.uri()),
        models: vec![ModelId::new("gpt-4")],
        timeout_secs: 10,
    })
    .unwrap();

    provider
        .complete(&stream_request())
        .await
        .expect("chat request uses the normalized base URL");
    assert!(matches!(
        provider.health_check().await,
        HealthStatus::Healthy { .. }
    ));

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .any(|request| request.method == "POST" && request.url.path() == "/chat/completions"));
    assert!(requests
        .iter()
        .any(|request| request.method == "GET" && request.url.path() == "/models"));
}

#[tokio::test]
async fn complete_returns_response_on_success() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-123",
                "model": "gpt-4",
                "choices": [{
                    "message": { "role": "assistant", "content": "Hello, world!" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13 }
            })),
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
}

#[tokio::test]
async fn stream_returns_chunks_on_openai_sse_success() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_string(
                "data: {\"id\":\"chatcmpl-123\",\"model\":\"gpt-4\",\"choices\":[{\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n\
                 data: {\"id\":\"chatcmpl-123\",\"model\":\"gpt-4\",\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,\"total_tokens\":12}}\n\n\
                 data: [DONE]\n\n",
            ),
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
            tool_calls: vec![],
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

    let chunks = provider
        .stream(&req)
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("chunks parse");

    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.delta.as_str())
            .collect::<String>(),
        "Hello"
    );
    assert_eq!(
        chunks.last().unwrap().finish_reason,
        Some(FinishReason::Stop)
    );
    assert_eq!(
        chunks.last().unwrap().usage.as_ref().unwrap().total_tokens,
        12
    );
}

#[tokio::test]
async fn stream_preserves_fragmented_tool_call_deltas_and_interleaved_text() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(concat!(
            r#"data: {"id":"chatcmpl-123","model":"gpt-4","choices":[{"delta":{"content":"checking ","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{\"pa"}},{"index":1,"id":"call-2","type":"function","function":{"name":"count","arguments":"{\"n\":"}}]},"finish_reason":null}]}"#,
            "\n\n",
            r#"data: {"id":"chatcmpl-123","model":"gpt-4","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"x\"}"}},{"index":1,"function":{"arguments":"1}"}}]},"finish_reason":null}]}"#,
            "\n\n",
            r#"data: {"id":"chatcmpl-123","model":"gpt-4","choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "\n\n",
            "data: [DONE]\n\n"
        )))
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
            tool_calls: vec![],
            role: Role::User,
            content: rook_core::MessageContent::Text("read and count".to_string()),
        }],
        stream: true,
        max_tokens: None,
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

    let chunks = provider
        .stream(&req)
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("tool-call deltas parse");

    assert_eq!(chunks[0].delta, "checking ");
    assert_eq!(chunks[0].tool_calls.len(), 2);
    assert_eq!(chunks[0].tool_calls[0].index, 0);
    assert_eq!(chunks[0].tool_calls[0].id.as_deref(), Some("call-1"));
    assert_eq!(
        chunks[0].tool_calls[0].function.name.as_deref(),
        Some("read_file")
    );
    assert_eq!(
        chunks[0].tool_calls[0].function.arguments.as_deref(),
        Some("{\"pa")
    );
    assert_eq!(chunks[0].tool_calls[1].index, 1);
    assert_eq!(
        chunks[1].tool_calls[0].function.arguments.as_deref(),
        Some("th\":\"x\"}")
    );
    assert_eq!(
        chunks[1].tool_calls[1].function.arguments.as_deref(),
        Some("1}")
    );
    assert_eq!(chunks[2].finish_reason, Some(FinishReason::ToolCalls));
}

#[tokio::test]
async fn stream_returns_error_for_malformed_upstream_json_event() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_string("data: {not-valid-json}\n\n"),
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

    let results = provider
        .stream(&stream_request())
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await;

    assert_eq!(results.len(), 1);
    assert!(results[0].as_ref().is_err());
}

#[tokio::test]
async fn stream_returns_error_when_tool_arguments_are_incomplete_at_finish() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(concat!(
            r#"data: {"id":"chatcmpl-123","model":"gpt-4","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"read_file","arguments":"{\"path\":"}}]},"finish_reason":null}]}"#,
            "\n\n",
            r#"data: {"id":"chatcmpl-123","model":"gpt-4","choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "\n\n",
            "data: [DONE]\n\n"
        )))
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

    let results = provider
        .stream(&stream_request())
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await;

    assert_eq!(results.len(), 2);
    assert!(results[0].is_ok());
    let error = results[1].as_ref().expect_err("incomplete arguments fail");
    assert!(error.to_string().contains("invalid tool-call arguments"));
}

#[tokio::test]
async fn stream_returns_error_when_connection_ends_before_tool_finish_reason() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(concat!(
            r#"data: {"id":"chatcmpl-123","model":"gpt-4","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"read_file","arguments":"{}"}}]},"finish_reason":null}]}"#,
            "\n\n"
        )))
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

    let results = provider
        .stream(&stream_request())
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await;

    assert_eq!(results.len(), 2);
    assert!(results[0].is_ok());
    assert!(results[1]
        .as_ref()
        .expect_err("missing finish reason fails")
        .to_string()
        .contains("before the tool_calls finish reason"));
}

#[tokio::test]
async fn stream_returns_error_for_eof_inside_sse_event() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
            r#"data: {"id":"chatcmpl-123","model":"gpt-4","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":null}]}"#,
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

    let results = provider
        .stream(&stream_request())
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await;

    assert_eq!(results.len(), 1);
    assert!(results[0]
        .as_ref()
        .expect_err("unterminated SSE event must fail")
        .to_string()
        .contains("incomplete SSE event"));
}

#[tokio::test]
async fn stream_joins_multiple_data_lines_in_one_sse_event() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_string(concat!(
                r#"data: {"id":"chatcmpl-123","model":"gpt-4","#,
                "\n",
                r#"data: "choices":[{"delta":{"content":"Hi"},"finish_reason":"stop"}]}"#,
                "\n\n",
                "data: [DONE]\n\n"
            )),
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

    let chunks = provider
        .stream(&stream_request())
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("multiline data event parses");

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].delta, "Hi");
    assert_eq!(chunks[0].finish_reason, Some(FinishReason::Stop));
}

#[tokio::test]
async fn stream_rejects_more_than_sixty_four_tool_calls() {
    let tool_calls: Vec<_> = (0..65)
        .map(|index| {
            serde_json::json!({
                "index": index,
                "id": format!("call-{index}"),
                "type": "function",
                "function": { "name": "lookup", "arguments": "{}" }
            })
        })
        .collect();
    let event = serde_json::json!({
        "id": "chatcmpl-123",
        "model": "gpt-4",
        "choices": [{
            "delta": { "tool_calls": tool_calls },
            "finish_reason": null
        }]
    });
    let results = stream_results_from_body(format!("data: {event}\n\n")).await;

    let error = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("too many tool calls must fail");
    assert!(error.to_string().contains("tool-call limit"));
}

#[tokio::test]
async fn stream_rejects_tool_arguments_over_one_megabyte() {
    let event = serde_json::json!({
        "id": "chatcmpl-123",
        "model": "gpt-4",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call-1",
                    "function": {
                        "name": "lookup",
                        "arguments": "x".repeat(1_048_577)
                    }
                }]
            },
            "finish_reason": null
        }]
    });
    let results = stream_results_from_body(format!("data: {event}\n\n")).await;

    let error = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("oversized tool arguments must fail");
    assert!(error.to_string().contains("tool-call argument limit"));
}

#[tokio::test]
async fn stream_limits_aggregate_arguments_across_tool_calls() {
    let tool_calls: Vec<_> = (0..2)
        .map(|index| {
            serde_json::json!({
                "index": index,
                "id": format!("call-{index}"),
                "function": {
                    "name": "lookup",
                    "arguments": "x".repeat(600_000)
                }
            })
        })
        .collect();
    let event = serde_json::json!({
        "id": "chatcmpl-123",
        "model": "gpt-4",
        "choices": [{ "delta": { "tool_calls": tool_calls }, "finish_reason": null }]
    });
    let results = stream_results_from_body(format!("data: {event}\n\n")).await;

    let error = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("aggregate argument limit must fail");
    assert!(error.to_string().contains("total tool-call argument limit"));
}

#[tokio::test]
async fn stream_rejects_an_sse_event_over_two_megabytes() {
    let body = format!("data: {}", "x".repeat(2_097_153));
    let results = stream_results_from_body(body).await;

    let error = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("oversized SSE event must fail");
    assert!(error.to_string().contains("SSE event exceeds"));
}

#[tokio::test]
async fn complete_parses_cached_tokens_and_reasoning_tokens() {
    // T5.1: OpenAI extended usage parses prompt_tokens_details.cached_tokens
    // and completion_tokens_details.reasoning_tokens.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-123",
                "model": "gpt-4o",
                "choices": [{
                    "message": { "role": "assistant", "content": "Hi" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 500,
                    "completion_tokens": 150,
                    "total_tokens": 650,
                    "prompt_tokens_details": {
                        "cached_tokens": 400
                    },
                    "completion_tokens_details": {
                        "reasoning_tokens": 120
                    }
                }
            })),
        )
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: "sk-test".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("gpt-4o")],
        timeout_secs: 10,
    })
    .unwrap();

    let req = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("gpt-4o"),
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
    // Verify cache_read_tokens (cached prompt) is parsed
    assert_eq!(resp.usage.cache_read_tokens, Some(400));
    // Verify reasoning_tokens is parsed
    assert_eq!(resp.usage.reasoning_tokens, Some(120));
    // cache_creation_tokens is not available from OpenAI response
    assert_eq!(resp.usage.cache_creation_tokens, None);
}

#[tokio::test]
async fn stream_request_includes_include_usage_option() {
    // T5.1: Streaming requests set stream_options.include_usage: true
    // so the final chunk carries usage data.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_string(
                "data: {\"id\":\"chatcmpl-123\",\"model\":\"gpt-4\",\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"total_tokens\":7}}\n\ndata: [DONE]\n\n",
            ),
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
            tool_calls: vec![],
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

    let chunks = provider
        .stream(&req)
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("chunks parse");

    // Verify the final chunk has usage from the SSE event
    let final_usage = chunks.last().unwrap().usage.as_ref();
    assert!(final_usage.is_some());
    assert_eq!(final_usage.unwrap().prompt_tokens, 5);
    assert_eq!(final_usage.unwrap().completion_tokens, 2);
}

#[tokio::test]
async fn provider_metadata_and_availability() {
    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: "sk-test".to_string(),
        base_url: "http://localhost".to_string(),
        models: vec![ModelId::new("gpt-4")],
        timeout_secs: 10,
    })
    .unwrap();

    assert_eq!(provider.id().as_str(), "openai-test");
    assert_eq!(provider.supported_models(), &[ModelId::new("gpt-4")]);
    assert_eq!(provider.api_format(), rook_core::ApiFormat::OpenAI);
    assert!(provider.is_available());

    let empty_key_provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: String::new(),
        base_url: "http://localhost".to_string(),
        models: vec![ModelId::new("gpt-4")],
        timeout_secs: 10,
    })
    .unwrap();

    assert!(!empty_key_provider.is_available());
}

#[tokio::test]
async fn complete_supports_all_roles() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-roles",
                "model": "gpt-4",
                "choices": [{
                    "message": { "role": "assistant", "content": "Done" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 10, "completion_tokens": 1, "total_tokens": 11 }
            })),
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
        messages: vec![
            rook_core::Message {
                role: Role::System,
                content: rook_core::MessageContent::Text("sys".to_string()),
                tool_calls: vec![],
            },
            rook_core::Message {
                role: Role::User,
                content: rook_core::MessageContent::Text("user".to_string()),
                tool_calls: vec![],
            },
            rook_core::Message {
                role: Role::Assistant,
                content: rook_core::MessageContent::Text("asst".to_string()),
                tool_calls: vec![],
            },
            rook_core::Message {
                role: Role::Developer,
                content: rook_core::MessageContent::Text("dev".to_string()),
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
            cacheable: true,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    };

    let res = provider.complete(&req).await;
    assert!(res.is_ok());

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value =
        serde_json::from_slice(&requests[0].body).expect("completion request is JSON");
    let roles: Vec<_> = body["messages"]
        .as_array()
        .expect("messages is an array")
        .iter()
        .map(|message| message["role"].as_str().expect("role is a string"))
        .collect();
    assert_eq!(roles, ["system", "user", "assistant", "developer"]);
}

#[tokio::test]
async fn complete_returns_error_on_empty_choices() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-empty",
                "model": "gpt-4",
                "choices": [],
                "usage": { "prompt_tokens": 10, "completion_tokens": 0, "total_tokens": 10 }
            })),
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

    let res = provider.complete(&stream_request()).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("no choices"));
}

#[tokio::test]
async fn complete_returns_error_on_invalid_json() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("invalid json"))
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

    let res = provider.complete(&stream_request()).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("json parse failed"));
}

#[tokio::test]
async fn complete_returns_error_on_network_failure() {
    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: "sk-test".to_string(),
        base_url: "http://127.0.0.1:1".to_string(),
        models: vec![ModelId::new("gpt-4")],
        timeout_secs: 2,
    })
    .unwrap();

    let res = provider.complete(&stream_request()).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("request failed"));
}

#[tokio::test]
async fn stream_returns_error_on_network_failure() {
    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: "sk-test".to_string(),
        base_url: "http://127.0.0.1:1".to_string(),
        models: vec![ModelId::new("gpt-4")],
        timeout_secs: 2,
    })
    .unwrap();

    let res = provider.stream(&stream_request()).await;
    match res {
        Err(e) => assert!(e.to_string().contains("request failed")),
        Ok(_) => panic!("Expected error on stream network failure"),
    }
}

#[tokio::test]
async fn stream_returns_error_when_tool_call_identity_changes() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(concat!(
            r#"data: {"id":"chatcmpl-123","model":"gpt-4","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"read_file"}}]}}]}"#,
            "\n\n",
            r#"data: {"id":"chatcmpl-123","model":"gpt-4","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-2","function":{"name":"read_file"}}]}}]}"#,
            "\n\n"
        )))
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

    let results = provider
        .stream(&stream_request())
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await;

    assert_eq!(results.len(), 2);
    assert!(results[0].is_ok());
    let err = results[1].as_ref().expect_err("changed identity fails");
    assert!(err.to_string().contains("changed tool-call identity"));
}

#[tokio::test]
async fn stream_parses_length_and_content_filter_finish_reasons() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(concat!(
            r#"data: {"id":"chatcmpl-123","model":"gpt-4","choices":[{"delta":{"content":"a"},"finish_reason":"length"}]}"#,
            "\n\n",
            r#"data: {"id":"chatcmpl-123","model":"gpt-4","choices":[{"delta":{"content":"b"},"finish_reason":"content_filter"}]}"#,
            "\n\n",
            "data: [DONE]\n\n"
        )))
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

    let chunks = provider
        .stream(&stream_request())
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("chunks parse");

    assert_eq!(chunks[0].finish_reason, Some(FinishReason::Length));
    assert_eq!(chunks[1].finish_reason, Some(FinishReason::ContentFilter));
}

#[tokio::test]
async fn stream_returns_error_when_tool_calls_empty_at_finish() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(concat!(
            r#"data: {"id":"chatcmpl-123","model":"gpt-4","choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "\n\n",
            "data: [DONE]\n\n"
        )))
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

    let results = provider
        .stream(&stream_request())
        .await
        .expect("stream starts")
        .collect::<Vec<_>>()
        .await;

    assert_eq!(results.len(), 1);
    let err = results[0].as_ref().expect_err("no tool deltas fail");
    assert!(err.to_string().contains("no tool-call deltas"));
}

#[tokio::test]
async fn complete_parses_null_content_with_tool_calls() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "chatcmpl-null-content",
                "model": "gpt-4",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {"name": "lookup", "arguments": "{\"key\":\"value\"}"}
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 7, "total_tokens": 12}
            })),
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
            tool_calls: vec![],
            role: Role::User,
            content: rook_core::MessageContent::Text("Hi".to_string()),
        }],
        stream: false,
        max_tokens: None,
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

    let resp = provider.complete(&req).await.expect("complete succeeds");
    assert_eq!(resp.content, "");
    assert_eq!(
        resp.content_blocks,
        vec![rook_core::MessageContent::Text(String::new())]
    );
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "lookup");
    assert_eq!(resp.finish_reason, Some(FinishReason::ToolCalls));
}
