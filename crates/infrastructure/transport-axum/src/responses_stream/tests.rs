use super::*;
use crate::responses_adapter::ResponsesRequest;
use rook_core::{ModelId, ToolCallDelta, ToolCallFunctionDelta};
use shared_kernel::RequestId;

fn encoder() -> ResponsesStream {
    let request: ResponsesRequest =
        serde_json::from_value(json!({"model":"test","input":"hola"})).unwrap();
    ResponsesStream::new(ResponseContext::new(&request.into_domain().unwrap()))
}

#[test]
fn rejected_terminal_chunk_does_not_mutate_public_snapshot() {
    let mut encoder = encoder();
    encoder.start();
    encoder
        .push(chunk(
            "previo",
            vec![tool(0, Some("a"), Some("lookup"), Some("{"))],
            None,
        ))
        .unwrap();
    assert!(encoder
        .push(chunk(
            "rechazado",
            vec![
                tool(0, None, None, Some("}")),
                tool(1, Some("b"), Some("lookup"), Some("[]")),
            ],
            Some(FinishReason::ToolCalls)
        ))
        .is_err());
    let failed = encoder.fail("error");
    let output = &failed.last().unwrap()["response"]["output"];
    assert_eq!(output.as_array().unwrap().len(), 2);
    assert_eq!(output[0]["content"][0]["text"], "previo");
    assert_eq!(output[1]["arguments"], "{");
    assert_eq!(failed[0]["sequence_number"], 7);
}

#[test]
fn stop_with_complete_functions_fails_before_done_events() {
    let mut encoder = encoder();
    encoder.start();
    encoder
        .push(chunk(
            "",
            vec![tool(0, Some("call_a"), Some("lookup"), Some("{}"))],
            None,
        ))
        .unwrap();
    let message = encoder
        .push(chunk(
            "",
            vec![tool(0, Some("call_a"), Some("lookup"), None)],
            Some(FinishReason::Stop),
        ))
        .expect_err("stop con funciones debe fallar");
    let events = encoder.fail(message);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["type"], "error");
    assert_eq!(events[1]["type"], "response.failed");
    assert_eq!(events[1]["response"]["output"][0]["status"], "in_progress");
}

#[test]
fn equal_nonterminal_name_fragments_are_not_summaries() {
    let mut encoder = encoder();
    for name in ["foo", "foo"] {
        encoder
            .push(chunk(
                "",
                vec![tool(0, Some("call_a"), Some(name), None)],
                None,
            ))
            .unwrap();
    }
    encoder
        .push(chunk(
            "",
            vec![tool(0, None, None, Some("{}"))],
            Some(FinishReason::ToolCalls),
        ))
        .unwrap();
    assert_eq!(
        encoder.finish().unwrap().last().unwrap()["response"]["output"][0]["name"],
        "foofoo"
    );
}

#[test]
fn terminal_identity_summary_requires_the_existing_call_id() {
    for id in [Some("call_a"), None, Some("call_b")] {
        let mut encoder = encoder();
        encoder
            .push(chunk(
                "",
                vec![tool(0, Some("call_a"), Some("foo"), Some("{}"))],
                None,
            ))
            .unwrap();
        let result = encoder.push(chunk(
            "",
            vec![tool(0, id, Some("foo"), None)],
            Some(FinishReason::ToolCalls),
        ));
        assert_eq!(result.is_ok(), id == Some("call_a"));
    }
}

fn chunk(text: &str, calls: Vec<ToolCallDelta>, finish: Option<FinishReason>) -> StreamChunk {
    StreamChunk {
        id: RequestId::new(),
        model: ModelId::new("test"),
        delta: text.into(),
        thinking: None,
        tool_calls: calls,
        finish_reason: finish,
        usage: None,
    }
}

fn tool(
    index: u32,
    id: Option<&str>,
    name: Option<&str>,
    arguments: Option<&str>,
) -> ToolCallDelta {
    ToolCallDelta {
        index,
        id: id.map(str::to_owned),
        function: ToolCallFunctionDelta {
            name: name.map(str::to_owned),
            arguments: arguments.map(str::to_owned),
        },
    }
}

#[test]
fn fragmented_names_sparse_indices_and_interleaved_text_preserve_identity() {
    let mut encoder = encoder();
    encoder.start();
    assert!(encoder
        .push(chunk(
            "",
            vec![tool(99, Some("call_a"), Some("look"), None)],
            None
        ))
        .unwrap()
        .is_empty());
    encoder
        .push(chunk(
            "Hola",
            vec![
                tool(99, None, Some("up"), Some("{\"a\":")),
                tool(3, Some("call_b"), Some("lookup"), Some("{}")),
            ],
            None,
        ))
        .unwrap();
    encoder
        .push(chunk(
            " mundo",
            vec![tool(99, None, None, Some("1}"))],
            Some(FinishReason::ToolCalls),
        ))
        .unwrap();
    let events = encoder.finish().unwrap();
    let response = &events.last().unwrap()["response"];
    assert_eq!(response["output"][0]["content"][0]["text"], "Hola mundo");
    assert_eq!(response["output"][1]["call_id"], "call_a");
    assert_eq!(response["output"][1]["name"], "lookup");
    assert_eq!(response["output"][1]["arguments"], "{\"a\":1}");
    assert_eq!(response["output"][2]["call_id"], "call_b");
}

#[test]
fn tools_are_bounded_and_65th_call_fails() {
    let mut encoder = encoder();
    for index in 0..64 {
        encoder
            .push(chunk(
                "",
                vec![tool(index, None, Some("lookup"), Some("{}"))],
                None,
            ))
            .unwrap();
    }
    assert!(encoder
        .push(chunk(
            "",
            vec![tool(100, None, Some("lookup"), Some("{}"))],
            None
        ))
        .is_err());
}

#[test]
fn text_and_argument_accumulation_are_bounded() {
    let mut text = encoder();
    assert!(text
        .push(chunk(&"x".repeat(MAX_OUTPUT_BYTES + 1), vec![], None))
        .is_err());
    let mut args = encoder();
    assert!(args
        .push(chunk(
            "",
            vec![tool(
                0,
                None,
                Some("lookup"),
                Some(&"x".repeat(MAX_ARGUMENT_BYTES + 1))
            )],
            None
        ))
        .is_err());
}

#[test]
fn invalid_upstream_calls_and_late_content_fail_closed() {
    let mut renamed = encoder();
    renamed
        .push(chunk(
            "",
            vec![tool(0, Some("a"), Some("lookup"), Some("{}"))],
            None,
        ))
        .unwrap();
    assert!(renamed
        .push(chunk("", vec![tool(0, Some("b"), None, None)], None))
        .is_err());
    let mut duplicate = encoder();
    assert!(duplicate
        .push(chunk(
            "",
            vec![
                tool(0, Some("a"), Some("f"), Some("{}")),
                tool(1, Some("a"), Some("g"), Some("{}"))
            ],
            None
        ))
        .is_err());
    let mut malformed = encoder();
    assert!(malformed
        .push(chunk(
            "",
            vec![tool(0, None, Some("f"), Some("{"))],
            Some(FinishReason::ToolCalls),
        ))
        .is_err());
    assert!(malformed.finish().is_err());
    let mut late = encoder();
    late.push(chunk("Hola", vec![], Some(FinishReason::Stop)))
        .unwrap();
    assert!(late.push(chunk("contenido tardío", vec![], None)).is_err());
    assert!(encoder().finish().is_err());
}

#[test]
fn only_empty_names_are_neutral_after_announcement() {
    for name in ["", "lookup", "changed"] {
        let mut encoder = encoder();
        encoder
            .push(chunk(
                "",
                vec![tool(0, Some("a"), Some("lookup"), Some("{}"))],
                None,
            ))
            .unwrap();
        assert_eq!(
            encoder
                .push(chunk("", vec![tool(0, None, Some(name), None)], None))
                .is_ok(),
            name.is_empty()
        );
    }
}

#[test]
fn incomplete_tool_arguments_are_preserved_without_claiming_valid_json() {
    let mut encoder = encoder();
    encoder
        .push(chunk(
            "",
            vec![tool(0, Some("a"), Some("f"), Some("{"))],
            Some(FinishReason::Length),
        ))
        .unwrap();
    let events = encoder.finish().unwrap();
    assert_eq!(events.last().unwrap()["type"], "response.incomplete");
    assert_eq!(
        events.last().unwrap()["response"]["output"][0]["arguments"],
        "{"
    );
}

#[test]
fn test_observe_usage_and_fail_and_invalid_tool_name_emission() {
    let mut encoder = encoder();
    encoder.observe_usage(TokenUsage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        cache_read_tokens: None,
        cache_creation_tokens: None,
        reasoning_tokens: None,
        estimated_cost_usd: None,
    });

    let failed = encoder.fail("custom_error_message");
    assert_eq!(failed[0]["type"], "error");
    assert_eq!(failed[0]["message"], "custom_error_message");
    assert_eq!(failed[1]["type"], "response.failed");
    assert_eq!(failed[1]["response"]["usage"]["total_tokens"], 15);

    let mut invalid_name_enc = self::encoder();
    assert!(invalid_name_enc
        .push(chunk(
            "",
            vec![tool(0, Some("call_a"), Some("invalid name!"), Some("{}"))],
            None,
        ))
        .is_err());
}

fn usage(prompt_tokens: u32, completion_tokens: u32) -> TokenUsage {
    TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens + completion_tokens,
        cache_read_tokens: Some(2),
        cache_creation_tokens: None,
        reasoning_tokens: Some(1),
        estimated_cost_usd: None,
    }
}

#[test]
fn text_lifecycle_has_contiguous_sequences_stable_identity_and_late_usage() {
    let mut encoder = encoder();
    let mut events = encoder.start();
    events.extend(encoder.push(chunk("Hello", vec![], None)).unwrap());
    events.extend(
        encoder
            .push(chunk(" world", vec![], Some(FinishReason::Stop)))
            .unwrap(),
    );
    let mut usage_only = chunk("", vec![], None);
    usage_only.usage = Some(usage(8, 3));
    assert!(encoder.push(usage_only).unwrap().is_empty());
    assert!(!events
        .iter()
        .any(|event| event["type"].as_str().unwrap().ends_with(".done")));
    events.extend(encoder.finish().unwrap());

    let kinds: Vec<_> = events
        .iter()
        .map(|event| event["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        vec![
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
            "response.output_text.delta",
            "response.output_text.delta",
            "response.output_text.done",
            "response.content_part.done",
            "response.output_item.done",
            "response.completed",
        ]
    );
    for (sequence, event) in events.iter().enumerate() {
        assert_eq!(event["sequence_number"], sequence);
    }
    let initial = &events[0]["response"];
    let final_response = &events.last().unwrap()["response"];
    assert_eq!(initial["id"], final_response["id"]);
    assert_eq!(initial["created_at"], final_response["created_at"]);
    assert_eq!(
        final_response["output"][0]["content"][0]["text"],
        "Hello world"
    );
    assert_eq!(final_response["usage"]["total_tokens"], 11);
    assert_eq!(
        final_response["usage"]["input_tokens_details"]["cached_tokens"],
        2
    );
    let item_id = &events[2]["item"]["id"];
    for event in &events[3..8] {
        assert_eq!(&event["item_id"], item_id);
        assert_eq!(event["output_index"], 0);
    }
    assert_eq!(&final_response["output"][0]["id"], item_id);
}

#[test]
fn rejected_frame_rolls_back_unicode_content_and_sequences_but_keeps_its_usage() {
    let mut encoder = encoder();
    let mut accepted = encoder.start();
    accepted.extend(
        encoder
            .push(chunk(
                "pré",
                vec![tool(7, Some("a"), Some("lookup"), Some("{"))],
                None,
            ))
            .unwrap(),
    );
    let mut rejected = chunk(
        "秘密",
        vec![
            tool(7, None, None, Some("}")),
            tool(9, Some("b"), Some("lookup"), Some("[]")),
        ],
        Some(FinishReason::ToolCalls),
    );
    rejected.usage = Some(usage(20, 4));
    let error = encoder.push(rejected).unwrap_err();
    let failed = encoder.fail(error);
    assert_eq!(failed[0]["sequence_number"], accepted.len());
    assert_eq!(failed[1]["sequence_number"], accepted.len() + 1);
    let response = &failed[1]["response"];
    assert_eq!(response["status"], "failed");
    assert_eq!(response["output"].as_array().unwrap().len(), 2);
    assert_eq!(response["output"][0]["content"][0]["text"], "pré");
    assert_eq!(response["output"][1]["arguments"], "{");
    assert_eq!(response["usage"]["total_tokens"], 24);
    assert_eq!(response["error"]["message"], error);
}

#[test]
fn incomplete_finish_materializes_empty_arguments_and_keeps_late_zero_usage() {
    for (finish, reason) in [
        (FinishReason::Length, "max_output_tokens"),
        (FinishReason::ContentFilter, "content_filter"),
    ] {
        let mut encoder = encoder();
        encoder.start();
        let mut first = chunk("", vec![tool(42, None, Some("lookup"), None)], None);
        first.usage = Some(usage(10, 5));
        assert!(encoder.push(first).unwrap().is_empty());
        let added = encoder.push(chunk("", vec![], Some(finish))).unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0]["type"], "response.output_item.added");
        assert_eq!(added[0]["output_index"], 0);
        let mut zero = chunk("", vec![], None);
        zero.usage = Some(usage(0, 0));
        assert!(encoder.push(zero).unwrap().is_empty());
        let events = encoder.finish().unwrap();
        assert_eq!(events.last().unwrap()["type"], "response.incomplete");
        let response = &events.last().unwrap()["response"];
        assert_eq!(response["incomplete_details"]["reason"], reason);
        assert_eq!(response["output"][0]["arguments"], "");
        assert_eq!(
            response["output"][0]["call_id"],
            added[0]["item"]["call_id"]
        );
        assert_eq!(response["usage"]["total_tokens"], 0);
    }
}

#[test]
fn exact_argument_budget_is_shared_across_tools_and_next_byte_is_rejected() {
    let mut encoder = encoder();
    for (index, id) in [(1, "a"), (99, "b")] {
        encoder
            .push(chunk(
                "",
                vec![tool(
                    index,
                    Some(id),
                    Some("lookup"),
                    Some(&"é".repeat(MAX_ARGUMENT_BYTES / 4)),
                )],
                None,
            ))
            .unwrap();
    }
    assert!(encoder
        .push(chunk("", vec![tool(99, None, None, Some("x"))], None))
        .is_err());
    let events = encoder.fail("limit");
    let output = events.last().unwrap()["response"]["output"]
        .as_array()
        .unwrap();
    assert_eq!(output.len(), 2);
    assert_eq!(
        output
            .iter()
            .map(|item| item["arguments"].as_str().unwrap().len())
            .sum::<usize>(),
        MAX_ARGUMENT_BYTES
    );
}

#[test]
fn aggregate_output_budget_accepts_exact_limit_and_rejects_next_text_byte() {
    let mut encoder = encoder();
    encoder
        .push(chunk(
            &"x".repeat(MAX_OUTPUT_BYTES - 2),
            vec![tool(0, Some("a"), Some("lookup"), Some("{}"))],
            None,
        ))
        .unwrap();
    assert!(encoder.push(chunk("x", vec![], None)).is_err());
    encoder
        .push(chunk("", vec![], Some(FinishReason::ToolCalls)))
        .unwrap();
    let events = encoder.finish().unwrap();
    let output = &events.last().unwrap()["response"]["output"];
    assert_eq!(
        output[0]["content"][0]["text"].as_str().unwrap().len(),
        MAX_OUTPUT_BYTES - 2
    );
    assert_eq!(output[1]["arguments"], "{}");
}

#[test]
fn empty_finished_stream_has_one_terminal_event_and_no_invented_message() {
    let mut encoder = encoder();
    encoder.start();
    assert!(encoder
        .push(chunk("", vec![], Some(FinishReason::Stop)))
        .unwrap()
        .is_empty());
    let events = encoder.finish().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "response.completed");
    assert_eq!(events[0]["response"]["output"], json!([]));
    assert_eq!(events[0]["response"]["usage"], Value::Null);
}

#[test]
fn late_finish_or_function_content_fails_without_overwriting_delivered_output() {
    for late in [
        chunk("", vec![], Some(FinishReason::Stop)),
        chunk(
            "",
            vec![tool(0, Some("a"), Some("lookup"), Some("{}"))],
            None,
        ),
    ] {
        let mut encoder = encoder();
        encoder.start();
        encoder
            .push(chunk("accepted", vec![], Some(FinishReason::Stop)))
            .unwrap();
        assert!(encoder.push(late).is_err());
        let failed = encoder.fail("late content");
        assert_eq!(failed[0]["type"], "error");
        assert_eq!(failed[1]["type"], "response.failed");
        assert_eq!(failed[1]["response"]["output"].as_array().unwrap().len(), 1);
        assert_eq!(
            failed[1]["response"]["output"][0]["content"][0]["text"],
            "accepted"
        );
    }
}
