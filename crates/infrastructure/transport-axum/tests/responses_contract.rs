//! Pure Responses contract tests: no HTTP server or upstream provider required.

use rook_core::{
    ApiFormat, CompletionRequest, CompletionResponse, FinishReason, MessageContent,
    MessageToolCall, Role, TokenUsage,
};
use serde_json::{json, Value};
use shared_kernel::{ModelId, ProviderId, RequestId};
use transport_axum::{
    format_registry::{DomainPivotTranslator, FormatRegistry},
    responses_adapter::{ResponseContext, ResponsesRequest},
};

fn request(body: Value) -> Result<CompletionRequest, String> {
    serde_json::from_value::<ResponsesRequest>(body)
        .map_err(|error| error.to_string())?
        .into_domain()
}

fn function(name: &str) -> Value {
    json!({"type":"function", "name":name, "parameters":{"type":"object"}})
}

fn call(id: &str) -> Value {
    json!({"type":"function_call", "call_id":id, "name":"lookup", "arguments":"{}"})
}

fn result(id: &str) -> Value {
    json!({"type":"function_call_output", "call_id":id, "output":"found"})
}

fn completion() -> CompletionResponse {
    CompletionResponse {
        id: RequestId::new(),
        provider: ProviderId::new("upstream"),
        model: ModelId::new("physical-model"),
        content: "answer".into(),
        content_blocks: vec![],
        thinking: None,
        tool_calls: vec![],
        finish_reason: Some(FinishReason::Stop),
        usage: TokenUsage {
            prompt_tokens: 12,
            completion_tokens: 7,
            total_tokens: 19,
            cache_read_tokens: Some(4),
            cache_creation_tokens: Some(3),
            reasoning_tokens: Some(2),
            estimated_cost_usd: Some(0.01),
        },
        latency_ms: 1,
        cache_hit: None,
    }
}

fn context() -> ResponseContext {
    ResponseContext::new(&request(json!({"model":"rook/coding", "input":"hi"})).unwrap())
}

#[test]
fn text_request_preserves_instructions_options_and_stateless_metadata() {
    let req = request(json!({
        "model":"rook/coding", "input":"hello", "instructions":"be precise",
        "stream":true, "max_output_tokens":1, "temperature":0, "store":false
    }))
    .unwrap();
    assert_eq!(req.model.as_str(), "rook/coding");
    assert!(req.stream);
    assert_eq!(req.max_tokens, Some(1));
    assert_eq!(req.temperature, Some(0.0));
    assert_eq!(req.messages.len(), 2);
    assert_eq!(req.messages[0].role, Role::System);
    assert_eq!(req.messages[0].content.as_text(), "be precise");
    assert_eq!(req.messages[1].role, Role::User);
    assert_eq!(req.messages[1].content.as_text(), "hello");
    assert_eq!(req.metadata.origin, "responses");
    assert!(!req.metadata.cacheable);
}

#[test]
fn numeric_and_model_boundaries_are_inclusive() {
    for temperature in [0.0, 2.0] {
        assert!(request(json!({
            "model":"m".repeat(256), "input":"", "temperature":temperature,
            "max_output_tokens":u32::MAX
        }))
        .is_ok());
    }
    for model in [String::new(), " \t\n".into(), "m".repeat(257)] {
        assert!(request(json!({"model":model, "input":"hi"})).is_err());
    }
    for temperature in [-0.01, 2.01] {
        assert!(request(json!({"model":"m", "input":"hi", "temperature":temperature})).is_err());
    }
}

#[test]
fn input_accepts_1024_items_and_rejects_the_next_item() {
    for count in [1, 1024, 1025] {
        let input = vec![json!({"role":"user", "content":"hi"}); count];
        let converted = request(json!({"model":"m", "input":input}));
        assert_eq!(converted.is_ok(), count <= 1024, "{count} items");
        if let Ok(converted) = converted {
            assert_eq!(converted.messages.len(), count);
        }
    }
}

#[test]
fn replay_concatenates_assistant_text_and_calls_until_a_result_delimits_the_turn() {
    let req = request(json!({"model":"m", "input":[
        {"role":"developer", "content":"policy"},
        {"type":"message", "role":"assistant", "id":"msg_old", "status":"completed",
         "content":[{"type":"output_text", "text":"Searching", "annotations":[], "logprobs":[]}]},
        call("a"),
        {"role":"assistant", "content":[{"type":"input_text", "text":" now"}]},
        call("b"), result("a"), result("b"), call("c")
    ]}))
    .unwrap();
    assert_eq!(req.messages.len(), 5);
    assert_eq!(req.messages[0].role, Role::Developer);
    assert_eq!(req.messages[1].role, Role::Assistant);
    assert_eq!(req.messages[1].content.as_text(), "Searching now");
    assert_eq!(req.messages[1].tool_calls.len(), 2);
    assert_eq!(req.messages[1].tool_calls[0].id.as_deref(), Some("a"));
    assert_eq!(req.messages[1].tool_calls[1].id.as_deref(), Some("b"));
    for (message, expected_id) in req.messages[2..4].iter().zip(["a", "b"]) {
        let MessageContent::ToolResult {
            tool_use_id,
            content,
        } = &message.content
        else {
            panic!("result must retain its call identity");
        };
        assert_eq!(tool_use_id, expected_id);
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].as_text(), "found");
    }
    assert_eq!(req.messages[4].role, Role::Assistant);
    assert_eq!(req.messages[4].tool_calls[0].id.as_deref(), Some("c"));
}

#[test]
fn duplicate_calls_results_and_results_before_calls_are_rejected() {
    for input in [
        json!([call("a"), call("a")]),
        json!([call("a"), result("a"), result("a")]),
        json!([result("a"), call("a")]),
        json!([call("a"), result("b")]),
    ] {
        assert!(
            request(json!({"model":"m", "input":input})).is_err(),
            "{input}"
        );
    }
}

#[test]
fn function_replay_requires_object_arguments() {
    for arguments in ["null", "[]", "42", "true", "\"text\"", "{"] {
        let mut item = call("a");
        item["arguments"] = json!(arguments);
        assert!(
            request(json!({"model":"m", "input":[item]})).is_err(),
            "{arguments}"
        );
    }
}

#[test]
fn unknown_fields_are_rejected_at_every_input_boundary() {
    let cases = [
        json!({"model":"m", "input":"hi", "previous_response_id":"resp_old"}),
        json!({"model":"m", "input":[{"role":"user", "content":"hi", "extra":true}]}),
        json!({"model":"m", "input":[{"role":"user", "content":[
            {"type":"input_text", "text":"hi", "extra":true}]}]}),
        json!({"model":"m", "input":[{
            "type":"function_call", "call_id":"a", "name":"f", "arguments":"{}", "extra":true}]}),
        json!({"model":"m", "input":[{
            "type":"function_call_output", "call_id":"a", "output":"hi", "extra":true}]}),
        json!({"model":"m", "input":"hi", "tools":[{
            "type":"function", "name":"f", "parameters":{"type":"object"}, "extra":true}]}),
    ];
    for body in cases {
        assert!(
            serde_json::from_value::<ResponsesRequest>(body.clone()).is_err(),
            "{body}"
        );
    }
}

#[test]
fn output_text_and_nonempty_annotations_are_rejected_for_unsupported_replay() {
    for part in [
        json!({"type":"output_text", "text":"hi"}),
        json!({"type":"input_text", "text":"hi", "annotations":[{}]}),
        json!({"type":"input_text", "text":"hi", "logprobs":[{}]}),
        json!({"type":"input_image", "text":"hi"}),
    ] {
        assert!(
            request(json!({"model":"m", "input":[{"role":"user", "content":[part]}]})).is_err()
        );
    }
}

#[test]
fn replay_id_and_status_validation_applies_to_messages_calls_and_results() {
    for (field, value) in [("id", "bad\u{0}id"), ("status", "in_progress")] {
        for mut item in [
            json!({"role":"assistant", "content":"hi"}),
            call("b"),
            result("a"),
        ] {
            item[field] = json!(value);
            assert!(request(json!({"model":"m", "input":[call("a"), item]})).is_err());
        }
    }
}

#[test]
fn declared_function_limit_allows_64_unique_names() {
    let mut tools: Vec<_> = (0..64).map(|i| function(&format!("f_{i}"))).collect();
    let req = request(json!({"model":"m", "input":"hi", "tools":tools, "tool_choice":"required"}))
        .unwrap();
    assert_eq!(req.tools.unwrap().as_array().unwrap().len(), 64);
    tools.push(function("f_64"));
    assert!(request(json!({"model":"m", "input":"hi", "tools":tools})).is_err());
}

#[test]
fn duplicate_strict_and_nonobject_function_definitions_are_rejected() {
    for tools in [
        json!([function("f"), function("f")]),
        json!([{"type":"function", "name":"f", "parameters":{"type":"object"}, "strict":true}]),
        json!([{"type":"function", "name":"f", "parameters":{"type":"array"}}]),
        json!([{"type":"function", "name":"f", "parameters":[]}]),
    ] {
        assert!(
            request(json!({"model":"m", "input":"hi", "tools":tools})).is_err(),
            "{tools}"
        );
    }
}

#[test]
fn named_tool_choice_and_schema_round_trip_without_losing_description() {
    let tool = json!({"type":"function", "name":"lookup", "description":"Find a file",
        "parameters":{"type":"object", "properties":{"path":{"type":"string"}}, "required":["path"]},
        "strict":false});
    let choice = json!({"type":"function", "name":"lookup"});
    let req =
        request(json!({"model":"m", "input":"hi", "tools":[tool], "tool_choice":choice})).unwrap();
    assert_eq!(
        req.tool_choice,
        Some(json!({"type":"function", "function":{"name":"lookup"}}))
    );
    let envelope = ResponseContext::new(&req)
        .response_body(&completion())
        .unwrap();
    assert_eq!(envelope["tools"], json!([tool]));
    assert_eq!(envelope["tool_choice"], choice);
}

#[test]
fn named_tool_choice_rejects_extra_fields_and_wrong_types() {
    for choice in [
        json!({"type":"function", "name":"lookup", "extra":true}),
        json!({"type":"function", "name":1}),
        json!("unsupported"),
        json!([]),
    ] {
        assert!(request(
            json!({"model":"m", "input":"hi", "tools":[function("lookup")], "tool_choice":choice})
        )
        .is_err());
    }
}

#[test]
fn response_envelope_uses_requested_identity_and_preserves_usage_details() {
    let context = context();
    let mut response = completion();
    response.thinking = Some("private reasoning".into());
    let body = context.response_body(&response).unwrap();
    assert_eq!(body["id"], context.id);
    assert_eq!(body["model"], "rook/coding");
    assert_eq!(body["store"], false);
    assert_eq!(body["status"], "completed");
    assert_eq!(body["output"][0]["content"][0]["text"], "answer");
    assert_eq!(
        body["usage"],
        json!({"input_tokens":12, "output_tokens":7, "total_tokens":19,
        "input_tokens_details":{"cached_tokens":4}, "output_tokens_details":{"reasoning_tokens":2}})
    );
    assert!(!body.to_string().contains("private reasoning"));
    assert_eq!(body["incomplete_details"], Value::Null);
}

#[test]
fn anthropic_tool_blocks_are_used_once_and_canonical_calls_take_precedence() {
    let context = context();
    let mut response = completion();
    response.finish_reason = Some(FinishReason::ToolCalls);
    response.content_blocks = vec![MessageContent::ToolUse {
        id: "call_a".into(),
        name: "lookup".into(),
        input: json!({"path":"README"}),
    }];
    let body = context.response_body(&response).unwrap();
    assert_eq!(body["output"].as_array().unwrap().len(), 2);
    assert_eq!(body["output"][1]["call_id"], "call_a");
    assert_eq!(
        serde_json::from_str::<Value>(body["output"][1]["arguments"].as_str().unwrap()).unwrap(),
        json!({"path":"README"})
    );
    response.tool_calls.push(MessageToolCall {
        id: Some("call_a".into()),
        name: "lookup".into(),
        arguments: json!({"path":"canonical"}),
    });
    let body = context.response_body(&response).unwrap();
    assert_eq!(body["output"].as_array().unwrap().len(), 2);
    assert_eq!(
        serde_json::from_str::<Value>(body["output"][1]["arguments"].as_str().unwrap()).unwrap(),
        json!({"path":"canonical"})
    );
}

#[test]
fn omitted_call_ids_are_stable_distinct_and_replayable() {
    let context = context();
    let mut response = completion();
    response.content.clear();
    response.finish_reason = Some(FinishReason::ToolCalls);
    response.tool_calls = vec![
        MessageToolCall {
            id: None,
            name: "lookup".into(),
            arguments: json!({})
        };
        2
    ];
    let body = context.response_body(&response).unwrap();
    assert_eq!(body, context.response_body(&response).unwrap());
    let output = body["output"].as_array().unwrap();
    assert_ne!(output[0]["call_id"], output[1]["call_id"]);
    let mut input = output.clone();
    input.extend(
        output
            .iter()
            .map(|item| result(item["call_id"].as_str().unwrap())),
    );
    let replay = request(json!({"model":"m", "input":input})).unwrap();
    assert_eq!(replay.messages[0].tool_calls.len(), 2);
    assert_eq!(replay.messages.len(), 3);
}

#[test]
fn invalid_or_duplicate_explicit_output_ids_never_receive_replacements() {
    for ids in [vec![""], vec!["bad\nid"], vec!["duplicate", "duplicate"]] {
        let mut response = completion();
        response.finish_reason = Some(FinishReason::ToolCalls);
        response.tool_calls = ids
            .iter()
            .map(|id| MessageToolCall {
                id: Some((*id).into()),
                name: "lookup".into(),
                arguments: json!({}),
            })
            .collect();
        assert!(context().response_body(&response).is_err(), "{ids:?}");
    }
}

#[test]
fn incomplete_finish_preserves_raw_arguments_and_reports_the_correct_reason() {
    for (finish, reason) in [
        (FinishReason::Length, "max_output_tokens"),
        (FinishReason::ContentFilter, "content_filter"),
    ] {
        let mut response = completion();
        response.finish_reason = Some(finish);
        response.tool_calls.push(MessageToolCall {
            id: Some("a".into()),
            name: "lookup".into(),
            arguments: json!("{\"path\":"),
        });
        let body = context().response_body(&response).unwrap();
        assert_eq!(body["status"], "incomplete");
        assert_eq!(body["incomplete_details"]["reason"], reason);
        assert_eq!(body["output"][0]["status"], "incomplete");
        assert_eq!(body["output"][1]["status"], "incomplete");
        assert_eq!(body["output"][1]["arguments"], "{\"path\":");
        response.finish_reason = Some(FinishReason::ToolCalls);
        assert!(context().response_body(&response).is_err());
    }
}

#[test]
fn response_argument_budget_is_aggregate_and_inclusive() {
    let mut response = completion();
    response.content.clear();
    response.finish_reason = Some(FinishReason::Length);
    response.tool_calls = ["a", "b"]
        .into_iter()
        .map(|id| MessageToolCall {
            id: Some(id.into()),
            name: "lookup".into(),
            arguments: json!("x".repeat(512 * 1024)),
        })
        .collect();
    assert!(context().response_body(&response).is_ok());
    response.tool_calls[1].arguments = json!("x".repeat(512 * 1024 + 1));
    assert!(context().response_body(&response).is_err());
}

#[test]
fn response_budget_counts_utf8_bytes_and_text_plus_arguments() {
    let mut response = completion();
    response.content = "é".repeat(2 * 1024 * 1024);
    assert!(context().response_body(&response).is_ok());
    response.content.push('a');
    assert!(context().response_body(&response).is_err());
    response.content = "x".repeat(4 * 1024 * 1024 - 2);
    response.finish_reason = Some(FinishReason::ToolCalls);
    response.tool_calls.push(MessageToolCall {
        id: None,
        name: "lookup".into(),
        arguments: json!({}),
    });
    assert!(context().response_body(&response).is_ok());
    response.content.push('a');
    assert!(context().response_body(&response).is_err());
}

#[test]
fn format_translation_is_limited_to_responses_openai_to_anthropic() {
    let mut registry = FormatRegistry::new();
    registry.register(
        ApiFormat::OpenAI,
        ApiFormat::Anthropic,
        DomainPivotTranslator,
    );
    for (origin, target, converted) in [
        ("responses", ApiFormat::Anthropic, true),
        ("openai", ApiFormat::Anthropic, false),
        ("responses", ApiFormat::OpenAI, false),
    ] {
        let mut req = request(
            json!({"model":"m", "input":"hi", "tools":[function("lookup")], "tool_choice":"auto"}),
        )
        .unwrap();
        req.metadata.origin = origin.into();
        let original_tools = req.tools.clone();
        let translated = registry
            .translate_request(ApiFormat::OpenAI, target, req)
            .unwrap();
        if converted {
            assert_eq!(
                translated.tools,
                Some(json!([{"name":"lookup", "input_schema":{"type":"object"}}]))
            );
            assert_eq!(translated.tool_choice, Some(json!({"type":"auto"})));
        } else {
            assert_eq!(translated.tools, original_tools);
            assert_eq!(translated.tool_choice, Some(json!("auto")));
        }
    }
}
