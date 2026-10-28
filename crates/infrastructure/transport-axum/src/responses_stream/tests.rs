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
