use transport_axum::openai_adapter::OpenAIChatRequest;

#[test]
fn image_requests_are_rejected_instead_of_silently_becoming_text() {
    for stream in [false, true] {
        let request = serde_json::json!({"model":"custom", "stream":stream,"messages":[
            {"role":"user", "content":[{"type":"text","text":"describe"},
                {"type":"image_url","image_url":{"url":"https://image.example/test.png"}}]}
        ]});
        assert!(serde_json::from_value::<OpenAIChatRequest>(request).is_err());
    }
}

#[test]
fn valid_text_parts_and_null_tool_messages_remain_compatible() {
    let request: OpenAIChatRequest =
        serde_json::from_value(serde_json::json!({"model":"custom","messages":[
            {"role":"user","content":[{"type":"text","text":"hello"}]},
            {"role":"assistant","content":null,"tool_calls":[]}
        ]}))
        .unwrap();
    let domain = rook_core::CompletionRequest::from(request);
    assert_eq!(domain.messages[0].content.as_text(), "hello");
}

#[test]
fn anthropic_explicit_none_does_not_require_tools_for_plain_text() {
    let request: transport_axum::anthropic_adapter::AnthropicMessagesRequest = serde_json::from_value(serde_json::json!({
        "model":"custom", "messages":[{"role":"user","content":"hello"}], "tool_choice":{"type":"none"}
    })).unwrap();
    let request = rook_core::CompletionRequest::from(request);
    assert!(!rook_core::RequiredCapabilities::from_request(&request).tools);
}

#[test]
fn anthropic_image_blocks_including_nested_tool_results_are_rejected() {
    for content in [
        serde_json::json!([{"type":"image","source":{"type":"url","url":"https://image.example/test.png"}}]),
        serde_json::json!([{"type":"tool_result","tool_use_id":"call-1","content":[
            {"type":"image","source":{"type":"url","url":"https://image.example/test.png"}}
        ]}]),
    ] {
        assert!(serde_json::from_value::<
            transport_axum::anthropic_adapter::AnthropicMessagesRequest,
        >(serde_json::json!({
            "model":"custom","messages":[{"role":"user","content":content}]
        }))
        .is_err());
    }
}
