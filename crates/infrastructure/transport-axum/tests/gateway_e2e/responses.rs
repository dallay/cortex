use super::*;
use serde_json::json;

pub(super) fn initial_anthropic_tool_stream(input: serde_json::Value) -> String {
    [
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_initial","name":"lookup","input":input}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"input_tokens":3,"output_tokens":2}}),
        json!({"type":"message_stop"}),
    ].into_iter().map(|event| format!("data: {event}\n\n")).collect()
}

#[tokio::test]
async fn responses_anthropic_initial_input_without_deltas() {
    for input in [json!({}), json!({"key":"value"})] {
        for combo in [false, true] {
            let g = build_gateway_planned(
                vec![],
                UpstreamMock::Fail500,
                UpstreamMock::Sse(initial_anthropic_tool_stream(input.clone())),
            )
            .await;
            for _ in 0..3 {
                g.fallback
                    .on_failure(
                        &ProviderId::new("openai-e2e"),
                        &rook_core::CortexError::provider("fixture unavailable"),
                    )
                    .await;
            }
            let (_, _, body) = post_sse(g.app, "/v1/responses", json!({"model":if combo {"rook/e2e"} else {"claude-e2e"},"input":"hola","stream":true})).await;
            let events = sse_json(&body);
            assert_eq!(
                events.last().unwrap()["type"],
                "response.completed",
                "{body}"
            );
            assert_eq!(
                events.last().unwrap()["response"]["output"][0]["arguments"],
                input.to_string()
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e["type"] == "response.function_call_arguments.delta")
                    .count(),
                1
            );
            fixture(
                &format!(
                    "anthropic-initial-{}-{combo}.sse",
                    if input == json!({}) {
                        "empty"
                    } else {
                        "object"
                    }
                ),
                &body,
            );
        }
    }
}

#[tokio::test]
async fn responses_anthropic_initial_input_keeps_chat_and_messages_compatible() {
    for input in [json!({}), json!({"key":"value"})] {
        for messages in [false, true] {
            let g = build_gateway_planned(
                vec![],
                UpstreamMock::Fail500,
                UpstreamMock::Sse(initial_anthropic_tool_stream(input.clone())),
            )
            .await;
            let (uri, request) = if messages {
                ("/v1/messages", anthropic_body("claude-e2e", true))
            } else {
                ("/v1/chat/completions", openai_body("claude-e2e", true))
            };
            let (status, _, body) = post_sse(g.app, uri, request).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let events = sse_json(&body);
            assert!(
                !events.iter().any(|event| event.get("error").is_some()),
                "{body}"
            );
            let arguments: String = events
                .iter()
                .filter_map(|event| {
                    if messages {
                        event["delta"]["partial_json"].as_str()
                    } else {
                        event["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"]
                            .as_str()
                    }
                })
                .collect();
            assert_eq!(arguments, input.to_string(), "{body}");
            if messages {
                assert_eq!(
                    events.last().unwrap()["delta"]["stop_reason"],
                    "tool_use",
                    "{body}"
                );
            }
            assert!(body.ends_with("data: [DONE]\n\n"));
        }
    }
}

#[tokio::test]
async fn responses_empty_name_after_announcement_is_neutral() {
    let sse = openai_stream_frame(
        json!({"tool_calls":[{"index":0,"id":"call_empty_name","function":{"name":"lookup","arguments":"{"}}]}),
        None,
    ) + &openai_stream_frame(
        json!({"tool_calls":[{"index":0,"function":{"name":"","arguments":"}"}}]}),
        Some("tool_calls"),
    ) + "data: [DONE]\n\n";
    let g = build_gateway_planned(vec![], UpstreamMock::Sse(sse), UpstreamMock::Fail500).await;
    let (_, _, body) = post_sse(
        g.app,
        "/v1/responses",
        json!({"model":"gpt-4o","input":"hola","stream":true}),
    )
    .await;
    let events = sse_json(&body);
    assert_eq!(
        events.last().unwrap()["type"],
        "response.completed",
        "{body}"
    );
    assert_eq!(
        events.last().unwrap()["response"]["output"][0]["name"],
        "lookup"
    );
    fixture("empty-name.sse", &body);
}

#[tokio::test]
async fn responses_replay_interleaved_real_sse_output_is_one_turn() {
    let sse = openai_stream_frame(
        json!({"tool_calls":[{"index":0,"id":"a","function":{"name":"lookup","arguments":"{}"}}]}),
        None,
    ) + &openai_stream_frame(json!({"content":"Buscando"}), None)
        + &openai_stream_frame(
            json!({"tool_calls":[{"index":1,"id":"b","function":{"name":"lookup","arguments":"{}"}}]}),
            Some("tool_calls"),
        )
        + "data: [DONE]\n\n";
    let g = build_gateway_planned(vec![], UpstreamMock::Sse(sse), UpstreamMock::Fail500).await;
    let (_, _, body) = post_sse(
        g.app,
        "/v1/responses",
        json!({"model":"gpt-4o","input":"hola","stream":true}),
    )
    .await;
    let events = sse_json(&body);
    assert_eq!(
        events.last().unwrap()["type"],
        "response.completed",
        "{body}"
    );
    let mut input = events.last().unwrap()["response"]["output"]
        .as_array()
        .unwrap()
        .clone();
    fixture("interleaved-replay.sse", &body);
    assert_eq!(
        input
            .iter()
            .map(|item| item["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["function_call", "message", "function_call"]
    );
    input.extend([
        json!({"type":"function_call_output","call_id":"a","output":"uno"}),
        json!({"type":"function_call_output","call_id":"b","output":"dos"}),
        json!({"role":"user","content":"otro turno"}),
        json!({"type":"function_call","call_id":"c","name":"lookup","arguments":"{}"}),
        json!({"type":"function_call_output","call_id":"c","output":"tres"}),
    ]);
    let replay = build_gateway(vec![], false).await;
    let (status, body) = post_json(
        replay.app,
        "/v1/responses",
        json!({"model":"gpt-4o","input":input}),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sent: serde_json::Value =
        serde_json::from_slice(&replay.upstreams.openai.received_requests().await.unwrap()[0].body)
            .unwrap();
    let messages = sent["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 6, "{sent}");
    assert_eq!(messages[0]["content"], "Buscando");
    assert_eq!(messages[0]["tool_calls"][0]["id"], "a");
    assert_eq!(messages[0]["tool_calls"][1]["id"], "b");
    assert_eq!(messages[1]["tool_call_id"], "a");
    assert_eq!(messages[2]["tool_call_id"], "b");
    assert_eq!(messages[3]["role"], "user");
    assert_eq!(messages[4]["tool_calls"][0]["id"], "c");
}

#[test]
fn fixture_directory_must_be_under_system_temp() {
    let temp = std::env::temp_dir();
    assert!(fixture_directory_is_allowed(&temp, &temp).is_some());

    let outside = std::path::Path::new("/etc");
    assert!(fixture_directory_is_allowed(outside, &temp).is_none());

    let non_existing = temp.join("rook-responses-does-not-exist");
    assert!(fixture_directory_is_allowed(&non_existing, &temp).is_none());
}

fn fixture_directory_is_allowed(
    directory: &std::path::Path,
    temp_directory: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let canonical_directory = std::fs::canonicalize(directory).ok()?;
    let canonical_temp_directory = std::fs::canonicalize(temp_directory).ok()?;
    canonical_directory
        .starts_with(canonical_temp_directory)
        .then_some(canonical_directory)
}

pub(super) fn fixture(name: &str, body: &str) {
    if name.is_empty()
        || name.contains(['/', '\\'])
        || name == "."
        || name == ".."
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        panic!("fixture name must be a safe filename: {name:?}");
    }
    if let Some(directory) = std::env::var_os("ROOK_RESPONSES_FIXTURES_DIR") {
        let Some(directory) =
            fixture_directory_is_allowed(std::path::Path::new(&directory), &std::env::temp_dir())
        else {
            panic!("fixture directory must be under the system temp directory");
        };
        std::fs::write(directory.join(name), body).unwrap();
    }
}

#[tokio::test]
async fn responses_empty_arguments_with_separate_incomplete_finish() {
    for finish in ["length", "content_filter"] {
        for done in [false, true] {
            for combo in [false, true] {
                let first = json!({"model":"gpt-4o","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_empty","function":{"name":"lookup","arguments":""}}]},"finish_reason":null}]});
                let last = json!({"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":finish}]});
                let usage = json!({"model":"gpt-4o","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}});
                let sse = format!(
                    "data: {first}\n\ndata: {last}\n\ndata: {usage}\n\n{}",
                    if done { "data: [DONE]\n\n" } else { "" }
                );
                let g =
                    build_gateway_planned(vec![], UpstreamMock::Sse(sse), UpstreamMock::Fail500)
                        .await;
                let (_, _, body) = post_sse(g.app, "/v1/responses", json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hola","stream":true})).await;
                let events = sse_json(&body);
                let terminal = events.last().unwrap();
                assert_eq!(terminal["type"], "response.incomplete", "{body}");
                assert_eq!(terminal["response"]["output"][0]["arguments"], "");
                assert_eq!(terminal["response"]["output"][0]["name"], "lookup");
                assert_eq!(terminal["response"]["output"][0]["call_id"], "call_empty");
                assert_eq!(terminal["response"]["usage"]["total_tokens"], 5);
                assert_eq!(
                    events
                        .iter()
                        .filter(|e| e["type"] == "response.output_item.added")
                        .count(),
                    1
                );
                assert!(!body.contains("response.failed"));
                assert!(!body.contains("response.completed"));
                fixture(
                    &format!(
                        "empty-{finish}-{}-{}.sse",
                        if combo { "combo" } else { "standard" },
                        if done { "done" } else { "eof" }
                    ),
                    &body,
                );
            }
        }
    }
}

#[tokio::test]
async fn responses_text_uses_existing_gateway() {
    let g = build_gateway(vec![], false).await;
    let (status, body) = post_json(
        g.app,
        "/v1/responses",
        json!({"model":"gpt-4o", "input":"hola", "store":false}),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["object"], "response");
    assert!(body["id"].as_str().unwrap().starts_with("resp_"));
    assert_eq!(body["model"], "gpt-4o");
    assert_eq!(body["status"], "completed");
    assert_eq!(body["output"][0]["content"][0]["text"], "mock reply");
    assert_eq!(body["usage"]["total_tokens"], 5);
    fixture("text.json", &body.to_string());
}

#[tokio::test]
async fn responses_function_round_trip_preserves_calls_and_native_tool_declarations() {
    for anthropic in [false, true] {
        let mut openai = openai_ok_body();
        openai["choices"][0]["message"] = json!({"role":"assistant", "content":"Buscando",
            "tool_calls":[{"id":"call_real", "type":"function",
                "function":{"name":"lookup", "arguments":"{\"key\":\"value\"}"}}]});
        openai["choices"][0]["finish_reason"] = json!("tool_calls");
        let g = build_gateway_planned(vec![], UpstreamMock::Json(openai),
            UpstreamMock::Json(json!({"id":"msg_real", "model":"claude-e2e", "type":"message",
                "role":"assistant", "content":[{"type":"text", "text":"Buscando"},
                    {"type":"tool_use", "id":"call_real", "name":"lookup", "input":{"key":"value"}}],
                "stop_reason":"tool_use", "usage":{"input_tokens":3,"output_tokens":4}}))).await;
        let model = if anthropic { "claude-e2e" } else { "gpt-4o" };
        let (status, body) = post_json(g.app, "/v1/responses", json!({
            "model":model, "instructions":"Ayuda", "max_output_tokens":64, "temperature":0.2,
            "input":[{"role":"user","content":[{"type":"input_text","text":"busca"}]},
                {"type":"function_call","id":"fc_prior","call_id":"call_prior", "name":"lookup", "arguments":"{}"},
                {"type":"function_call_output", "call_id":"call_prior", "output":"resultado"}],
            "tools":[{"type":"function", "name":"lookup", "description":"Busca", "parameters":{"type":"object"},"strict":false}],
            "tool_choice":{"type":"function","name":"lookup"}
        }), &[]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let output = body["output"].as_array().unwrap();
        assert_eq!(output.len(), 2, "no duplicar tool_calls y content_blocks");
        let call = output
            .iter()
            .find(|item| item["type"] == "function_call")
            .unwrap();
        assert_eq!(call["call_id"], "call_real");
        assert_eq!(call["name"], "lookup");
        assert_eq!(call["arguments"], "{\"key\":\"value\"}");
        assert_eq!(call["status"], "completed");
        assert_eq!(body["tools"][0]["name"], "lookup");
        assert_eq!(
            body["tool_choice"],
            json!({"type":"function","name":"lookup"})
        );
        fixture(
            if anthropic {
                "anthropic-tool.json"
            } else {
                "openai-tool.json"
            },
            &body.to_string(),
        );
        let server = if anthropic {
            &g.upstreams.anthropic
        } else {
            &g.upstreams.openai
        };
        let sent: serde_json::Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        if anthropic {
            assert_eq!(sent["tools"][0]["name"], "lookup");
            assert_eq!(sent["tools"][0]["input_schema"]["type"], "object");
            assert_eq!(sent["tool_choice"], json!({"type":"tool","name":"lookup"}));
            assert_eq!(sent["messages"][1]["content"][0]["id"], "call_prior");
            assert_eq!(
                sent["messages"][2]["content"][0]["tool_use_id"],
                "call_prior"
            );
        } else {
            assert_eq!(sent["tools"][0]["function"]["name"], "lookup");
            assert_eq!(sent["messages"][2]["tool_calls"][0]["id"], "call_prior");
            assert_eq!(sent["messages"][3]["tool_call_id"], "call_prior");
        }
    }
}

#[tokio::test]
async fn responses_stream_preserves_lifecycle_tools_alias_and_trailing_usage() {
    for anthropic in [false, true] {
        let g = build_gateway_planned(
            vec![],
            if anthropic {
                UpstreamMock::Fail500
            } else {
                UpstreamMock::Sse(fragmented_combo_sse())
            },
            UpstreamMock::Sse(many_anthropic_tool_stream(1)),
        )
        .await;
        let (status, content_type, body) = post_sse(
            g.app,
            "/v1/responses",
            json!({"model":"rook/e2e","input":"hola","stream":true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(content_type.starts_with("text/event-stream"));
        assert!(!body.contains("[DONE]"));
        let events = sse_json(&body);
        let kinds: Vec<_> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            vec![
                "response.created",
                "response.in_progress",
                "response.output_item.added",
                "response.function_call_arguments.delta",
                if anthropic {
                    "response.function_call_arguments.done"
                } else {
                    "response.function_call_arguments.delta"
                },
                if anthropic {
                    "response.output_item.done"
                } else {
                    "response.function_call_arguments.done"
                },
                if anthropic {
                    "response.completed"
                } else {
                    "response.output_item.done"
                }
            ]
            .into_iter()
            .chain((!anthropic).then_some("response.completed"))
            .collect::<Vec<_>>(),
            "{body}"
        );
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event["sequence_number"], index);
            assert!(body.contains(&format!("event: {}", event["type"].as_str().unwrap())));
        }
        let final_response = &events.last().unwrap()["response"];
        assert_eq!(final_response["model"], "rook/e2e");
        assert_eq!(final_response["id"], events[0]["response"]["id"]);
        assert_eq!(
            final_response["output"][0]["call_id"],
            if anthropic { "tool-0" } else { "call_real" }
        );
        assert_eq!(
            final_response["output"][0]["arguments"],
            if anthropic {
                "{}"
            } else {
                "{\"key\":\"value\"}"
            }
        );
        assert_eq!(final_response["usage"]["output_tokens"], 7);
        fixture(
            if anthropic {
                "anthropic-tool.sse"
            } else {
                "openai-tool.sse"
            },
            &body,
        );
    }
}

#[tokio::test]
async fn responses_text_stream_is_incremental_and_length_is_incomplete() {
    for finish in ["stop", "length", "content_filter"] {
        let upstream = openai_stream_frame(json!({"content":"Ho"}), None)
            + &openai_stream_frame(json!({"content":"la"}), Some(finish))
            + "data: [DONE]\n\n";
        let g =
            build_gateway_planned(vec![], UpstreamMock::Sse(upstream), UpstreamMock::Fail500).await;
        let (status, _, body) = post_sse(
            g.app,
            "/v1/responses",
            json!({"model":"gpt-4o","input":"hola","stream":true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let events = sse_json(&body);
        assert_eq!(
            events
                .iter()
                .filter(|e| e["type"] == "response.output_text.delta")
                .map(|e| e["delta"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["Ho", "la"]
        );
        let terminal = events.last().unwrap();
        assert_eq!(
            terminal["response"]["output"][0]["content"][0]["text"],
            "Hola"
        );
        assert_eq!(
            terminal["type"],
            if finish == "stop" {
                "response.completed"
            } else {
                "response.incomplete"
            }
        );
        if finish != "stop" {
            assert_eq!(
                terminal["response"]["incomplete_details"]["reason"],
                if finish == "length" {
                    "max_output_tokens"
                } else {
                    "content_filter"
                }
            );
        }
        fixture(&format!("text-{finish}.sse"), &body);
    }
}

#[tokio::test]
async fn responses_partial_tool_finish_preserves_incomplete_and_trailing_usage() {
    for finish in ["length", "content_filter"] {
        for done in [false, true] {
            let g = build_gateway_planned(
                vec![],
                UpstreamMock::Sse(partial_tool_finish_stream(finish, done)),
                UpstreamMock::Fail500,
            )
            .await;
            let (status, _, body) = post_sse(
                g.app,
                "/v1/responses",
                json!({"model":"gpt-4o","input":"hola","stream":true}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let events = sse_json(&body);
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e["type"] == "response.incomplete")
                    .count(),
                1,
                "finish={finish}, done={done}: {body}"
            );
            assert!(
                !events.iter().any(|e| matches!(
                    e["type"].as_str(),
                    Some("response.failed" | "response.completed" | "error")
                )),
                "{body}"
            );
            let terminal = events.last().unwrap();
            assert_eq!(terminal["type"], "response.incomplete");
            let response = &terminal["response"];
            assert_eq!(response["status"], "incomplete");
            assert_eq!(
                response["incomplete_details"]["reason"],
                if finish == "length" {
                    "max_output_tokens"
                } else {
                    "content_filter"
                }
            );
            assert_eq!(response["output"].as_array().unwrap().len(), 1);
            assert_eq!(response["output"][0]["call_id"], "call_partial");
            assert_eq!(response["output"][0]["name"], "lookup");
            assert_eq!(response["output"][0]["arguments"], "{\"key\":\"val");
            assert_eq!(response["output"][0]["status"], "incomplete");
            assert_eq!(response["usage"]["input_tokens"], 3);
            assert_eq!(response["usage"]["output_tokens"], 7);
            assert_eq!(response["usage"]["total_tokens"], 10);
            let deltas: String = events
                .iter()
                .filter(|e| e["type"] == "response.function_call_arguments.delta")
                .map(|e| e["delta"].as_str().unwrap())
                .collect();
            assert_eq!(deltas, "{\"key\":\"val");
            fixture(
                &format!(
                    "partial-tool-{finish}-{}.sse",
                    if done { "done" } else { "eof" }
                ),
                &body,
            );
        }
    }
}

fn partial_tool_finish_stream(finish: &str, done: bool) -> String {
    openai_stream_frame(
        json!({"tool_calls":[{"index":0,"id":"call_partial", "function":{"name":"lookup","arguments":"{\"key\":"}}]}),
        None,
    ) + &openai_stream_frame(
        json!({"tool_calls":[{"index":0,"function":{"arguments":"\"val"}}]}),
        Some(finish),
    ) + "data: {\"model\":\"gpt-4o\",\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":7,\"total_tokens\":10}}\n\n"
        + if done { "data: [DONE]\n\n" } else { "" }
}

#[tokio::test]
async fn chat_and_messages_partial_tool_finish_keep_terminal_error() {
    for finish in ["length", "content_filter"] {
        for done in [false, true] {
            for messages in [false, true] {
                let g = build_gateway_planned(
                    vec![],
                    UpstreamMock::Sse(partial_tool_finish_stream(finish, done)),
                    UpstreamMock::Fail500,
                )
                .await;
                let (uri, request) = if messages {
                    ("/v1/messages", anthropic_body("rook/e2e", true))
                } else {
                    ("/v1/chat/completions", openai_body("gpt-4o", true))
                };
                let (status, _, body) = post_sse(g.app, uri, request).await;
                assert_eq!(status, StatusCode::OK, "{body}");
                let events = sse_json(&body);
                assert_eq!(
                    events.iter().filter(|e| e.get("error").is_some()).count(),
                    1,
                    "finish={finish}, done={done}, messages={messages}: {body}"
                );
                assert!(events.last().unwrap().get("error").is_some(), "{body}");
                assert!(body.ends_with("data: [DONE]\n\n"));
                assert!(!events.iter().any(|e| e["type"] == "message_stop"));
            }
        }
    }
}

#[tokio::test]
async fn responses_partial_tool_other_finishes_still_fail() {
    for finish in ["stop", "tool_calls"] {
        for done in [false, true] {
            let g = build_gateway_planned(
                vec![],
                UpstreamMock::Sse(partial_tool_finish_stream(finish, done)),
                UpstreamMock::Fail500,
            )
            .await;
            let (status, _, body) = post_sse(
                g.app,
                "/v1/responses",
                json!({"model":"gpt-4o","input":"hola","stream":true}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let events = sse_json(&body);
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e["type"] == "response.failed")
                    .count(),
                1,
                "{body}"
            );
            assert_eq!(events.last().unwrap()["type"], "response.failed");
            assert!(
                !events.iter().any(|e| matches!(
                    e["type"].as_str(),
                    Some("response.completed" | "response.incomplete")
                )),
                "{body}"
            );
        }
    }
}

#[tokio::test]
async fn responses_stream_errors_and_missing_finish_never_complete_or_fallback() {
    for (label, suffix) in [
        (
            "error",
            "data: {\"error\":{\"message\":\"private upstream error\"}}\n\n",
        ),
        ("done", "data: [DONE]\n\n"),
        ("eof", ""),
    ] {
        let upstream = openai_stream_frame(
            json!({"tool_calls":[{"index":0,"id":"call_partial", "function":{"name":"lookup","arguments":"{\"key\":"}}]}),
            None,
        ) + suffix;
        let g = build_gateway_planned(
            vec![],
            UpstreamMock::Sse(upstream),
            UpstreamMock::Json(fallback_reply_body()),
        )
        .await;
        let (_, _, body) = post_sse(
            g.app,
            "/v1/responses",
            json!({"model":"rook/e2e","input":"hola","stream":true}),
        )
        .await;
        let events = sse_json(&body);
        assert_eq!(
            events
                .iter()
                .filter(|e| e["type"] == "response.failed")
                .count(),
            1,
            "{body}"
        );
        assert!(!events.iter().any(|e| e["type"] == "response.completed"));
        assert_eq!(
            events.last().unwrap()["response"]["output"][0]["call_id"],
            "call_partial"
        );
        assert_eq!(
            events.last().unwrap()["response"]["output"][0]["arguments"],
            "{\"key\":"
        );
        assert!(!body.contains("private upstream error"));
        assert!(g
            .upstreams
            .anthropic
            .received_requests()
            .await
            .unwrap()
            .is_empty());
        fixture(&format!("failed-{label}.sse"), &body);
    }
}

#[tokio::test]
async fn responses_unsupported_and_malformed_input_fail_before_upstream() {
    let invalid = vec![
        json!({"model":"gpt-4o","input":"hi","tools":[{"type":"web_search"}]}),
        json!({"model":"gpt-4o","input":"hi","previous_response_id":"resp_old"}),
        json!({"model":"gpt-4o","input":"hi","background":true}),
        json!({"model":"gpt-4o","input":"hi","store":true}),
        json!({"model":"gpt-4o","input":[{"role":"other","content":"hi"}]}),
        json!({"model":"gpt-4o","input":[{"role":"user","content":[{"type":"input_image","image_url":"https://example.com"}]}]}),
        json!({"model":"gpt-4o","input":[{"type":"function_call_output","call_id":"orphan","output":"hi"}]}),
        json!({"model":"gpt-4o","input":[{"type":"function_call","call_id":"call","name":"f","arguments":"invalid"}]}),
        json!({"model":"gpt-4o","input":"hi","tools":[{"type":"function","name":"f","parameters":{"type":"object"},"strict":true}]}),
        json!({"model":"gpt-4o","input":"hi","tools":[{"type":"function","name":"f","parameters":{"type":"object"},"extra":1}]}),
        json!({"model":"gpt-4o","input":"hi","tool_choice":{"type":"function","name":"missing"}}),
        json!({"model":"gpt-4o","input":[],"max_output_tokens":0}),
        json!({"model":"gpt-4o","input":"hi","temperature":3}),
    ];
    for body in invalid {
        let g = build_gateway(vec![], false).await;
        let (status, response) = post_json(g.app, "/v1/responses", body.clone(), &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {response}");
        assert_eq!(response["error"]["code"], "invalid_request_error");
        assert!(g
            .upstreams
            .openai
            .received_requests()
            .await
            .unwrap()
            .is_empty());
        assert!(g
            .upstreams
            .anthropic
            .received_requests()
            .await
            .unwrap()
            .is_empty());
    }
}

#[tokio::test]
async fn responses_authentication_scopes_and_restrictions_fail_before_upstream() {
    for stream in [false, true] {
        for mode in ["revoked", "read_only", "model", "provider", "combo"] {
            let g = build_gateway(
                if mode == "model" {
                    vec!["other"]
                } else {
                    vec![]
                },
                false,
            )
            .await;
            if mode == "revoked" {
                *g.key_repo.revoked.lock().unwrap() = true;
            }
            if mode == "read_only" {
                *g.key_repo.read_only.lock().unwrap() = true;
            }
            if mode == "provider" || mode == "combo" {
                *g.key_repo.allowed_providers.lock().unwrap() = vec![ProviderId::new("other")];
            }
            let (status, body) = post_json(g.app, "/v1/responses", json!({"model":if mode == "combo" {"rook/e2e"} else {"gpt-4o"},"input":"hola","stream":stream}),
                &[("x-authz-auth-scopes","admin"),("x-authz-allowed-models","[]"),("x-authz-allowed-providers","[]")]).await;
            assert_eq!(
                status,
                if mode == "revoked" {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::FORBIDDEN
                },
                "{mode}: {body}"
            );
            assert!(g
                .upstreams
                .openai
                .received_requests()
                .await
                .unwrap()
                .is_empty());
            assert!(g
                .upstreams
                .anthropic
                .received_requests()
                .await
                .unwrap()
                .is_empty());
            // wiremock verifica en Drop con block_on; evitar que agote el
            // presupuesto cooperativo de Tokio tras muchos casos consecutivos.
            tokio::task::yield_now().await;
        }
    }
}

#[tokio::test]
async fn responses_multiple_function_inputs_form_one_assistant_turn() {
    let g = build_gateway(vec![], false).await;
    let (status, body) = post_json(
        g.app,
        "/v1/responses",
        json!({"model":"gpt-4o","input":[
            {"type":"function_call","call_id":"a","name":"lookup","arguments":"{}"},
            {"type":"function_call","call_id":"b","name":"lookup","arguments":"{}"},
            {"type":"function_call_output","call_id":"a","output":"uno"},
            {"type":"function_call_output","call_id":"b","output":"dos"}
        ]}),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sent: serde_json::Value =
        serde_json::from_slice(&g.upstreams.openai.received_requests().await.unwrap()[0].body)
            .unwrap();
    assert_eq!(
        sent["messages"][0]["tool_calls"].as_array().unwrap().len(),
        2
    );
    assert_eq!(sent["messages"][1]["tool_call_id"], "a");
    assert_eq!(sent["messages"][2]["tool_call_id"], "b");
}

#[tokio::test]
async fn responses_nonstream_finish_state_is_not_reported_as_completed() {
    let mut upstream = openai_ok_body();
    upstream["choices"][0]["finish_reason"] = json!("length");
    let g =
        build_gateway_planned(vec![], UpstreamMock::Json(upstream), UpstreamMock::Fail500).await;
    let (status, body) = post_json(
        g.app,
        "/v1/responses",
        json!({"model":"gpt-4o","input":"hola"}),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "incomplete");
    assert_eq!(body["incomplete_details"]["reason"], "max_output_tokens");
    fixture("incomplete.json", &body.to_string());
}

#[tokio::test]
async fn responses_missing_key_and_invalid_json_never_reach_upstream() {
    for (key, content_type, body, expected) in [
        (None, "application/json", "{}", StatusCode::UNAUTHORIZED),
        (
            Some("Bearer e2e-key-1"),
            "application/json",
            "{",
            StatusCode::BAD_REQUEST,
        ),
        (
            Some("Bearer e2e-key-1"),
            "text/plain",
            "{}",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
    ] {
        let g = build_gateway(vec![], false).await;
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri("/v1/responses")
            .header("content-type", content_type);
        if let Some(key) = key {
            builder = builder.header("authorization", key);
        }
        let response = g
            .app
            .oneshot(builder.body(axum::body::Body::from(body)).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert!(g
            .upstreams
            .openai
            .received_requests()
            .await
            .unwrap()
            .is_empty());
        assert!(g
            .upstreams
            .anthropic
            .received_requests()
            .await
            .unwrap()
            .is_empty());
    }
}
