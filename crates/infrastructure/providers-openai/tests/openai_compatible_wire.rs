//! Wire-level coverage for generic OpenAI-compatible connections (DALLAY-682).
//!
//! These tests drive the OpenAI adapter against a mock HTTP upstream, using
//! the endpoint policy that a compatible connection receives at runtime.

use futures::StreamExt;
use providers_openai::provider::{OpenAIProvider, OpenAIProviderConfig};
use rook_core::{
    CompletionRequest, EndpointPolicy, Message, MessageContent, ModelId, ProviderPort, Role,
};
use serde_json::json;
use shared_kernel::{ProviderId, RequestId};

fn loopback_policy(base_url: &str) -> EndpointPolicy {
    let url = reqwest::Url::parse(base_url).expect("mock server URL");
    EndpointPolicy {
        allowed_origins: vec![format!(
            "{}://{}:{}",
            url.scheme(),
            url.host_str().unwrap(),
            url.port_or_known_default().unwrap()
        )],
    }
}

fn request(model: &str) -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new(model),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("hola".to_string()),
            tool_calls: vec![],
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
    }
}

fn provider(id: &str, api_key: &str, base_url: String, models: &[&str]) -> OpenAIProvider {
    OpenAIProvider::new_with_endpoint_policy(
        OpenAIProviderConfig {
            id: ProviderId::new(id),
            api_key: api_key.to_string(),
            base_url: base_url.clone(),
            models: models.iter().map(|m| ModelId::new(*m)).collect(),
            timeout_secs: 5,
        },
        loopback_policy(&base_url),
    )
    .expect("compatible provider is built for an explicitly trusted loopback endpoint")
}

#[tokio::test]
async fn chat_request_reaches_the_declared_physical_model_with_its_own_credential() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer key-connection-a",
        ))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-1",
            "model": "physical-model-a",
            "choices": [{
                "message": { "role": "assistant", "content": "respuesta" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let provider = provider(
        "compatible-a",
        "key-connection-a",
        server.uri(),
        &["physical-model-a"],
    );

    let response = provider
        .complete(&request("physical-model-a"))
        .await
        .expect("compatible chat request succeeds");

    assert_eq!(response.content, "respuesta");
    assert_eq!(response.model, ModelId::new("physical-model-a"));

    let requests = server.received_requests().await.expect("recorded requests");
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).expect("json body");
    assert_eq!(body["model"], "physical-model-a");
    assert_eq!(body["stream"], json!(false));
}

#[tokio::test]
async fn tool_definitions_and_tool_choice_reach_the_upstream() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-2",
            "model": "physical-model-a",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\":\"Habana\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 9, "completion_tokens": 4, "total_tokens": 13 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let provider = provider(
        "compatible-a",
        "key-connection-a",
        server.uri(),
        &["physical-model-a"],
    );

    let mut req = request("physical-model-a");
    req.tools = Some(json!([{
        "type": "function",
        "function": {
            "name": "get_weather",
            "parameters": { "type": "object", "properties": { "city": { "type": "string" } } }
        }
    }]));
    req.tool_choice = Some(json!("auto"));

    let response = provider
        .complete(&req)
        .await
        .expect("compatible tool request succeeds");

    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "get_weather");
    assert_eq!(response.tool_calls[0].arguments["city"], "Habana");

    let requests = server.received_requests().await.expect("recorded requests");
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).expect("json body");
    assert_eq!(body["tools"][0]["function"]["name"], "get_weather");
    assert_eq!(body["tool_choice"], "auto");
}

#[tokio::test]
async fn streaming_reaches_the_upstream_and_returns_chunks() {
    let server = wiremock::MockServer::start().await;
    let sse = concat!(
        "data: {\"id\":\"chatcmpl-3\",\"model\":\"physical-model-a\",\"choices\":[{\"delta\":{\"content\":\"ho\"},\"index\":0}]}\n\n",
        "data: {\"id\":\"chatcmpl-3\",\"model\":\"physical-model-a\",\"choices\":[{\"delta\":{\"content\":\"la\"},\"index\":0}]}\n\n",
        "data: {\"id\":\"chatcmpl-3\",\"model\":\"physical-model-a\",\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
        "data: [DONE]\n\n"
    );
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .expect(1)
        .mount(&server)
        .await;

    let provider = provider(
        "compatible-a",
        "key-connection-a",
        server.uri(),
        &["physical-model-a"],
    );

    let mut req = request("physical-model-a");
    req.stream = true;
    let mut stream = provider
        .stream(&req)
        .await
        .expect("compatible stream starts");
    let mut text = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("stream chunk is valid");
        text.push_str(&chunk.delta);
    }
    assert_eq!(text, "hola");

    let requests = server.received_requests().await.expect("recorded requests");
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).expect("json body");
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["model"], "physical-model-a");
}

#[tokio::test]
async fn two_compatible_connections_keep_separate_upstreams_and_credentials() {
    let first = wiremock::MockServer::start().await;
    let second = wiremock::MockServer::start().await;

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .and(wiremock::matchers::header("authorization", "Bearer key-a"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-a",
            "model": "model-a",
            "choices": [{
                "message": { "role": "assistant", "content": "from-a" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })))
        .expect(1)
        .mount(&first)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .and(wiremock::matchers::header("authorization", "Bearer key-b"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-b",
            "model": "model-b",
            "choices": [{
                "message": { "role": "assistant", "content": "from-b" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })))
        .expect(1)
        .mount(&second)
        .await;

    let provider_a = provider("compatible-a", "key-a", first.uri(), &["model-a"]);
    let provider_b = provider("compatible-b", "key-b", second.uri(), &["model-b"]);

    assert_eq!(
        provider_a
            .complete(&request("model-a"))
            .await
            .expect("first connection completes")
            .content,
        "from-a"
    );
    assert_eq!(
        provider_b
            .complete(&request("model-b"))
            .await
            .expect("second connection completes")
            .content,
        "from-b"
    );

    let body_a: serde_json::Value =
        serde_json::from_slice(&first.received_requests().await.expect("a")[0].body)
            .expect("json body");
    let body_b: serde_json::Value =
        serde_json::from_slice(&second.received_requests().await.expect("b")[0].body)
            .expect("json body");
    assert_eq!(body_a["model"], "model-a");
    assert_eq!(body_b["model"], "model-b");
}

#[tokio::test]
async fn upstream_failures_produce_bounded_errors_without_leaking_the_credential() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(400)
                .set_body_string("invalid request for key-connection-a"),
        )
        .mount(&server)
        .await;

    let provider = provider(
        "compatible-a",
        "key-connection-a",
        server.uri(),
        &["physical-model-a"],
    );

    let error = provider
        .complete(&request("physical-model-a"))
        .await
        .expect_err("upstream failure is surfaced");
    let message = error.to_string();
    assert!(
        !message.contains("key-connection-a"),
        "error must not leak the credential: {message}"
    );
    assert!(message.len() < 500, "error must stay bounded: {message}");
}

#[tokio::test]
async fn guarded_health_probe_preserves_the_base_path_and_sanitizes_upstream_errors() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/gateway/v1/models"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer compatible-key",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(503).set_body_string("failure for compatible-key"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider = provider(
        "compatible",
        "compatible-key",
        format!("{}/gateway/v1", server.uri()),
        &["model"],
    );
    match provider.health_check().await {
        rook_core::HealthStatus::Unhealthy {
            provider, error, ..
        } => {
            assert_eq!(provider, ProviderId::new("compatible"));
            assert!(error.contains("503"));
            assert!(!error.contains("compatible-key"));
        }
        other => panic!("expected unhealthy, got {other:?}"),
    }
}

#[tokio::test]
async fn guarded_stream_and_health_probe_never_follow_redirects() {
    let target = wiremock::MockServer::start().await;
    for status in [301, 302, 303, 307, 308] {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(
                wiremock::ResponseTemplate::new(status)
                    .insert_header("Location", format!("{}/capture", target.uri())),
            )
            .expect(2)
            .mount(&server)
            .await;
        let provider = provider("compatible", "redirect-secret", server.uri(), &["model"]);

        let mut req = request("model");
        req.stream = true;
        let error = match provider.stream(&req).await {
            Err(error) => error,
            Ok(_) => panic!("HTTP {status} must not start a stream"),
        };
        assert!(!error.to_string().contains("redirect-secret"));
        assert!(
            matches!(
                provider.health_check().await,
                rook_core::HealthStatus::Unhealthy { .. }
            ),
            "HTTP {status}"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
    assert!(
        target.received_requests().await.unwrap().is_empty(),
        "redirects must not forward credentials to a different origin"
    );
}

#[tokio::test]
async fn guarded_resolver_rejects_an_unexpected_hostname_before_dns_lookup() {
    use reqwest::dns::Resolve;

    let resolver = providers_openai::endpoint_resolver::GuardedResolver::new(
        EndpointPolicy {
            allowed_origins: vec!["https://upstream.invalid".into()],
        },
        "https",
        "upstream.invalid",
        443,
    );
    let error = match resolver.resolve("other.invalid".parse().unwrap()).await {
        Err(error) => error,
        Ok(_) => panic!("resolver must not resolve a host other than its configured upstream"),
    };
    let io_error = error
        .downcast_ref::<std::io::Error>()
        .expect("policy rejection is an I/O error");
    assert_eq!(io_error.kind(), std::io::ErrorKind::PermissionDenied);
}
