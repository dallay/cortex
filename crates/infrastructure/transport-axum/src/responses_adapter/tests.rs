use super::*;

fn completion() -> CompletionResponse {
    CompletionResponse {
        id: shared_kernel::RequestId::new(),
        provider: shared_kernel::ProviderId::new("test"),
        model: shared_kernel::ModelId::new("test"),
        content: String::new(),
        content_blocks: vec![],
        thinking: None,
        tool_calls: vec![],
        finish_reason: Some(FinishReason::Stop),
        usage: TokenUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            reasoning_tokens: None,
            estimated_cost_usd: None,
        },
        latency_ms: 0,
        cache_hit: None,
    }
}

fn context() -> ResponseContext {
    let request: ResponsesRequest =
        serde_json::from_value(json!({"model":"test","input":"hola"})).unwrap();
    ResponseContext::new(&request.into_domain().unwrap())
}

#[test]
fn nonstream_requires_coherent_terminal_state() {
    let mut response = completion();
    response.finish_reason = None;
    assert!(context().response_body(&response).is_err());
    response.finish_reason = Some(FinishReason::ToolCalls);
    assert!(context().response_body(&response).is_err());
}

#[test]
fn nonstream_response_has_same_resource_limits_as_streaming() {
    let mut response = completion();
    response.tool_calls = (0..65)
        .map(|i| rook_core::MessageToolCall {
            id: Some(format!("call_{i}")),
            name: "lookup".into(),
            arguments: json!({}),
        })
        .collect();
    assert!(context().response_body(&response).is_err());
    response.tool_calls.clear();
    response.content = "x".repeat(4 * 1024 * 1024 + 1);
    assert!(context().response_body(&response).is_err());
}

#[test]
fn nonstream_partial_tool_arguments_preserve_incomplete_state() {
    let mut response = completion();
    response.finish_reason = Some(FinishReason::Length);
    response.tool_calls.push(rook_core::MessageToolCall {
        id: Some("call_a".into()),
        name: "lookup".into(),
        arguments: json!("{\"a\":"),
    });
    let body = context().response_body(&response).unwrap();
    assert_eq!(body["status"], "incomplete");
    assert_eq!(body["output"][0]["arguments"], "{\"a\":");
}
