use futures::StreamExt;
use providers_ollama::{OllamaProvider, OllamaProviderConfig};
use providers_openai::{OpenAIProvider, OpenAIProviderConfig};
use rook_core::{CompletionRequest, ModelId, ProviderPort, Role};
use shared_kernel::{ProviderId, RequestId};
use transport_axum::openai_adapter::OpenAIChatCompletionChunk;

#[tokio::test]
async fn upstream_tool_call_fragments_survive_provider_domain_and_openai_wire() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(concat!(
            r#"data: {"id":"chatcmpl-test","model":"gpt-4o","choices":[{"delta":{"content":"Looking up.","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"lookup","arguments":"{\"key\":"}}]},"finish_reason":null}]}"#,
            "\n\n",
            r#"data: {"id":"chatcmpl-test","model":"gpt-4o","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"value\"}"}}]},"finish_reason":null}]}"#,
            "\n\n",
            r#"data: {"id":"chatcmpl-test","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "\n\n",
            "data: [DONE]\n\n"
        )))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-test"),
        api_key: "test-key".to_string(),
        base_url: server.uri(),
        models: vec![ModelId::new("gpt-4o")],
        timeout_secs: 10,
    })
    .expect("provider constructs");
    let request = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("gpt-4o"),
        messages: vec![rook_core::Message {
            tool_calls: vec![rook_core::MessageToolCall {
                id: Some("call-prior".to_string()),
                name: "previous_lookup".to_string(),
                arguments: serde_json::json!({"key": "prior"}),
            }],
            role: Role::Assistant,
            content: String::new().into(),
        }],
        stream: true,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".to_string(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    };

    let domain_chunks = provider
        .stream(&request)
        .await
        .expect("upstream stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("provider preserves valid tool-call stream");
    let wire_chunks: Vec<serde_json::Value> = domain_chunks
        .iter()
        .map(|chunk| {
            serde_json::to_value(OpenAIChatCompletionChunk::from(chunk))
                .expect("chunk serializes to OpenAI JSON")
        })
        .collect();

    assert_eq!(
        wire_chunks[0]["choices"][0]["delta"]["content"],
        "Looking up."
    );
    assert_eq!(
        wire_chunks[0]["choices"][0]["delta"]["tool_calls"][0]["index"],
        0
    );
    assert_eq!(
        wire_chunks[0]["choices"][0]["delta"]["tool_calls"][0]["id"],
        "call-1"
    );
    assert_eq!(
        wire_chunks[0]["choices"][0]["delta"]["tool_calls"][0]["function"]["name"],
        "lookup"
    );
    assert_eq!(
        wire_chunks[0]["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
        "{\"key\":"
    );
    assert_eq!(
        wire_chunks[1]["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
        "\"value\"}"
    );
    assert_eq!(wire_chunks[2]["choices"][0]["finish_reason"], "tool_calls");

    let requests = server.received_requests().await.expect("request captured");
    let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(sent["messages"][0]["tool_calls"][0]["id"], "call-prior");
    assert_eq!(
        sent["messages"][0]["tool_calls"][0]["function"]["arguments"],
        "{\"key\":\"prior\"}"
    );
}

#[tokio::test]
async fn ollama_structured_tool_call_survives_domain_and_openai_wire() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(concat!(
            r#"{"model":"qwen3","message":{"role":"assistant","content":"","thinking":"checking the key","tool_calls":[{"function":{"name":"lookup","arguments":{"key":"value"}}}]},"done":true,"done_reason":"tool_calls"}"#,
            "\n"
        )))
        .mount(&server)
        .await;

    let provider = OllamaProvider::new(OllamaProviderConfig {
        id: ProviderId::new("ollama-test"),
        base_url: server.uri(),
        models: vec![ModelId::new("qwen3")],
        timeout_secs: 10,
        api_key: None,
    })
    .expect("provider constructs");
    let request = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("qwen3"),
        messages: vec![rook_core::Message {
            tool_calls: vec![],
            role: Role::User,
            content: "Look up the key".into(),
        }],
        stream: true,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".to_string(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    };

    let chunks = provider
        .stream(&request)
        .await
        .expect("upstream stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("valid Ollama stream");
    let wire_chunks: Vec<serde_json::Value> = chunks
        .iter()
        .map(|chunk| {
            serde_json::to_value(OpenAIChatCompletionChunk::from(chunk))
                .expect("chunk serializes to OpenAI JSON")
        })
        .collect();

    let tool_call = &wire_chunks[0]["choices"][0]["delta"]["tool_calls"][0];
    assert_eq!(tool_call["index"], 0);
    assert!(
        tool_call["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("call_")),
        "missing Ollama IDs must become valid non-empty OpenAI IDs"
    );
    assert_eq!(tool_call["type"], "function");
    assert_eq!(tool_call["function"]["name"], "lookup");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            tool_call["function"]["arguments"].as_str().unwrap()
        )
        .unwrap(),
        serde_json::json!({"key": "value"})
    );
    assert_eq!(wire_chunks[0]["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(
        wire_chunks[0]["choices"][0]["delta"]["reasoning_content"],
        "checking the key"
    );
}

#[tokio::test]
async fn ollama_json_text_remains_content_in_openai_wire() {
    let text = r#"{"tool":"lookup","arguments":{"key":"value"}}"#;
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/chat"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "qwen3",
                "message": {"role": "assistant", "content": text},
                "done": true,
                "done_reason": "stop"
            })),
        )
        .mount(&server)
        .await;

    let provider = OllamaProvider::new(OllamaProviderConfig {
        id: ProviderId::new("ollama-test"),
        base_url: server.uri(),
        models: vec![ModelId::new("qwen3")],
        timeout_secs: 10,
        api_key: None,
    })
    .expect("provider constructs");
    let request = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("qwen3"),
        messages: vec![rook_core::Message {
            role: Role::User,
            content: "Return a JSON string".into(),
            tool_calls: vec![],
        }],
        stream: true,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".to_string(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    };

    let chunks = provider
        .stream(&request)
        .await
        .expect("upstream stream starts")
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("valid Ollama stream");
    let wire = serde_json::to_value(OpenAIChatCompletionChunk::from(&chunks[0])).unwrap();

    assert_eq!(wire["choices"][0]["delta"]["content"], text);
    assert!(wire["choices"][0]["delta"]["tool_calls"].is_null());
    assert_eq!(wire["choices"][0]["finish_reason"], "stop");
}
