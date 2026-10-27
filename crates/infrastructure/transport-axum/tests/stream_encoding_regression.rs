use rook_core::{ModelId, StreamChunk, ToolCallDelta, ToolCallFunctionDelta};
use shared_kernel::RequestId;
use transport_axum::openai_adapter::OpenAIChatCompletionChunk;

fn wire_tools(tools: Vec<ToolCallDelta>, request_id: &RequestId) -> Vec<serde_json::Value> {
    let chunk = StreamChunk {
        id: request_id.clone(),
        model: ModelId::new("rook/alias"),
        delta: String::new(),
        thinking: None,
        tool_calls: tools,
        finish_reason: None,
        usage: None,
    };
    let wire = serde_json::to_value(OpenAIChatCompletionChunk::from(&chunk)).unwrap();
    assert_eq!(wire["model"], "rook/alias");
    wire["choices"][0]["delta"]["tool_calls"]
        .as_array()
        .unwrap()
        .clone()
}

fn tool(index: u32, id: Option<&str>, name: Option<&str>, args: Option<&str>) -> ToolCallDelta {
    ToolCallDelta {
        index,
        id: id.map(str::to_owned),
        function: ToolCallFunctionDelta {
            name: name.map(str::to_owned),
            arguments: args.map(str::to_owned),
        },
    }
}

#[test]
fn openai_tool_continuations_omit_identity_for_empty_and_nonempty_fragments() {
    for args in [None, Some(""), Some("{\"key\":"), Some("\"value\"}")] {
        let tools = wire_tools(vec![tool(7, None, None, args)], &RequestId::new());
        assert_eq!(tools[0]["index"], 7);
        assert!(
            tools[0].get("id").is_none(),
            "continuations must not invent an ID"
        );
        assert!(tools[0]["function"].get("name").is_none());
        match args {
            Some(args) => assert_eq!(tools[0]["function"]["arguments"], args),
            None => assert!(tools[0]["function"].get("arguments").is_none()),
        }
    }
}

#[test]
fn openai_explicit_tool_ids_survive_even_without_a_function_name() {
    for name in [None, Some("lookup")] {
        let tools = wire_tools(
            vec![tool(3, Some("upstream-call"), name, Some("{}"))],
            &RequestId::new(),
        );
        assert_eq!(tools[0]["id"], "upstream-call");
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["arguments"], "{}");
    }
}

#[test]
fn openai_generated_start_ids_are_stable_distinct_by_index_and_absent_on_continuations() {
    let id = RequestId::new();
    let starts = vec![
        tool(2, None, Some("lookup"), Some("{\"key\":")),
        tool(9, None, Some("lookup"), Some("{}")),
    ];
    let first = wire_tools(starts.clone(), &id);
    let repeated = wire_tools(starts, &id);
    for index in 0..2 {
        assert_eq!(first[index]["id"], repeated[index]["id"]);
        assert!(!first[index]["id"].as_str().unwrap().is_empty());
    }
    assert_ne!(first[0]["id"], first[1]["id"]);
    let continuations = wire_tools(vec![tool(2, None, None, Some("\"value\"}"))], &id);
    assert_eq!(continuations[0]["index"], first[0]["index"]);
    assert!(continuations[0].get("id").is_none());
}
