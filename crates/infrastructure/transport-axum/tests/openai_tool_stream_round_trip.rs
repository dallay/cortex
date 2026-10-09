use futures::StreamExt;
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
}
