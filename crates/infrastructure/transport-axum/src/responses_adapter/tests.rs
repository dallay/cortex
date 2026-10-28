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

#[test]
fn test_valid_name_and_valid_id() {
    assert!(valid_name("tool_name-123"));
    assert!(!valid_name(""));
    assert!(!valid_name(&"a".repeat(65)));
    assert!(!valid_name("invalid name!"));

    assert!(valid_id("id-123"));
    assert!(!valid_id(""));
    assert!(!valid_id(&"a".repeat(257)));
    assert!(!valid_id("id\nwith\ncontrol"));
}

#[test]
fn test_validate_replay_cases() {
    assert!(validate_replay(&Some("valid_id".into()), &Some("completed".into())).is_ok());
    assert!(validate_replay(&Some("valid_id".into()), &Some("incomplete".into())).is_ok());
    assert!(validate_replay(&Some("".into()), &Some("completed".into())).is_err());
    assert!(validate_replay(&Some("valid_id".into()), &Some("other".into())).is_err());
}

#[test]
fn test_responses_request_validation_errors() {
    let req: ResponsesRequest = serde_json::from_value(json!({
        "model": "m", "input": "hi", "max_output_tokens": 0
    }))
    .unwrap();
    assert!(req.into_domain().is_err());

    let req: ResponsesRequest = serde_json::from_value(json!({
        "model": "m", "input": "hi", "temperature": -1.0
    }))
    .unwrap();
    assert!(req.into_domain().is_err());

    let req: ResponsesRequest = serde_json::from_value(json!({
        "model": "m", "input": "hi", "store": true
    }))
    .unwrap();
    assert!(req.into_domain().is_err());

    let req: ResponsesRequest = serde_json::from_value(json!({
        "model": "m", "input": []
    }))
    .unwrap();
    assert!(req.into_domain().is_err());

    let req: ResponsesRequest = serde_json::from_value(json!({
        "model": "m", "input": [{"role": "unknown", "content": "hi"}]
    }))
    .unwrap();
    assert!(req.into_domain().is_err());

    let req: ResponsesRequest = serde_json::from_value(json!({
        "model": "m", "input": [{"type": "function_call", "call_id": "c", "name": "f", "arguments": "invalid_json"}]
    })).unwrap();
    assert!(req.into_domain().is_err());

    let req: ResponsesRequest = serde_json::from_value(json!({
        "model": "m", "input": [{"type": "function_call_output", "call_id": "unmatched", "output": "o"}]
    })).unwrap();
    assert!(req.into_domain().is_err());
}

#[test]
fn test_responses_request_tool_choice_validation() {
    let req: ResponsesRequest = serde_json::from_value(json!({
        "model": "m", "input": "hi", "tool_choice": "required"
    }))
    .unwrap();
    assert!(req.into_domain().is_err());

    let req: ResponsesRequest = serde_json::from_value(json!({
        "model": "m", "input": "hi", "tool_choice": {"type": "function", "name": "undeclared"}
    }))
    .unwrap();
    assert!(req.into_domain().is_err());
}

#[test]
fn test_response_context_envelope_content_filter() {
    let ctx = context();
    let env = ctx.envelope("completed", vec![], None, Some(FinishReason::ContentFilter));
    assert_eq!(env["incomplete_details"]["reason"], "content_filter");
}
