use super::*;
use rook_core::{
    CostBreakdown, Pagination, RequestStatus, UsageEntry, UsageFilters, UsageRecorderPort,
    UsageSummary,
};
use serde_json::json;

fn anthropic_initial_usage() -> String {
    format!(
        "data: {}\n\n",
        json!({"type":"message_start","message":{"usage":{"input_tokens":12,"output_tokens":0,"cache_read_input_tokens":8,"cache_creation_input_tokens":4}}})
    )
}

#[tokio::test]
async fn responses_openai_omitted_ids_generate_unique_stable_replayable_calls() {
    for stream in [false, true] {
        for combo in [false, true] {
            let calls = json!([
                {"index":0,"function":{"name":"lookup","arguments":"{\"key\":\"one\"}"}},
                {"index":1,"function":{"name":"search","arguments":"{\"key\":\"two\"}"}}
            ]);
            let usage = json!({"prompt_tokens":3,"completion_tokens":2,"total_tokens":5});
            let upstream = if stream {
                let first = json!({"model":"gpt-4o","choices":[{"delta":{"tool_calls":calls},"finish_reason":null}]});
                // Nullable IDs and neutral names in continuations must not change identity.
                let last = json!({"model":"gpt-4o","choices":[{"delta":{"tool_calls":[
                    {"index":0,"id":null,"function":{"name":"","arguments":""}},
                    {"index":1,"id":null,"function":{"name":null,"arguments":""}}
                ]},"finish_reason":"tool_calls"}],"usage":usage});
                UpstreamMock::Sse(format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n"))
            } else {
                UpstreamMock::Json(
                    json!({"model":"gpt-4o","choices":[{"message":{"tool_calls":calls},"finish_reason":"tool_calls"}],"usage":usage}),
                )
            };
            let g = build_gateway_planned(vec![], upstream, UpstreamMock::Fail500).await;
            let model = if combo { "rook/e2e" } else { "gpt-4o" };
            let request = json!({"model":model,"input":"hi","stream":stream});
            let body = if stream {
                let (status, _, sse) = post_sse(g.app.clone(), "/v1/responses", request).await;
                assert_eq!(status, StatusCode::OK, "{sse}");
                let events = sse_json(&sse);
                let terminal = events.last().unwrap();
                assert_eq!(terminal["type"], "response.completed", "{sse}");
                let output = terminal["response"]["output"].as_array().unwrap();
                for event in &events {
                    if let Some(index) = event["output_index"].as_u64() {
                        let call = &output[index as usize];
                        if event.get("item").is_some() {
                            assert_eq!(event["item"]["id"], call["id"]);
                            assert_eq!(event["item"]["call_id"], call["call_id"]);
                        }
                        if event.get("item_id").is_some() {
                            assert_eq!(event["item_id"], call["id"]);
                        }
                    }
                }
                super::responses::fixture(&format!("openai-generated-ids-{combo}.sse"), &sse);
                terminal["response"].clone()
            } else {
                let (status, body) = post_json(g.app.clone(), "/v1/responses", request, &[]).await;
                assert_eq!(status, StatusCode::OK, "{body}");
                super::responses::fixture(
                    &format!("openai-generated-ids-{combo}.json"),
                    &body.to_string(),
                );
                body
            };
            assert_eq!(body["status"], "completed");
            assert_eq!(body["usage"]["total_tokens"], 5);
            let calls = body["output"].as_array().unwrap();
            assert_eq!(calls.len(), 2);
            assert_ne!(calls[0]["id"], calls[1]["id"]);
            assert_ne!(calls[0]["call_id"], calls[1]["call_id"]);
            for (call, name, key) in [(&calls[0], "lookup", "one"), (&calls[1], "search", "two")] {
                assert!(call["call_id"].as_str().unwrap().starts_with("call_"));
                assert_eq!(call["name"], name);
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(call["arguments"].as_str().unwrap())
                        .unwrap(),
                    json!({"key":key})
                );
            }
            {
                let audit = g.accounting.audit.lock().unwrap();
                let usage = g.accounting.usage.lock().unwrap();
                assert_eq!(audit.len(), 1);
                assert_eq!(usage.len(), 1);
                assert_eq!(audit[0].status, RequestStatus::Success);
                assert_eq!(usage[0].status, RequestStatus::Success);
                assert_eq!(audit[0].combo_id.is_some(), combo);
                assert_eq!(audit[0].request_id, usage[0].request_id);
                assert_eq!(audit[0].usage.as_ref().unwrap().total_tokens, 5);
                assert!((usage[0].cost_usd.unwrap() - 7.0 / 1_000_000.0).abs() < 1e-12);
            }
            let mut input = calls.clone();
            input.extend(calls.iter().map(|call| json!({"type":"function_call_output","call_id":call["call_id"],"output":"found"})));
            // Replay through the same provider and mode so generated IDs cross the wire.
            if stream {
                let (_, _, replay) = post_sse(
                    g.app,
                    "/v1/responses",
                    json!({"model":model,"input":input,"stream":true}),
                )
                .await;
                assert_eq!(
                    sse_json(&replay).last().unwrap()["type"],
                    "response.completed",
                    "{replay}"
                );
            } else {
                let (status, replay) = post_json(
                    g.app,
                    "/v1/responses",
                    json!({"model":model,"input":input}),
                    &[],
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{replay}");
            }
            let requests = g.upstreams.openai.received_requests().await.unwrap();
            assert_eq!(requests.len(), 2);
            let sent: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
            for (index, call) in calls.iter().enumerate() {
                assert_eq!(
                    sent["messages"][0]["tool_calls"][index]["id"],
                    call["call_id"]
                );
                assert_eq!(
                    sent["messages"][0]["tool_calls"][index]["function"]["name"],
                    call["name"]
                );
                assert_eq!(sent["messages"][index + 1]["tool_call_id"], call["call_id"]);
            }
            assert!(g
                .upstreams
                .anthropic
                .received_requests()
                .await
                .unwrap()
                .is_empty());
        }
    }
}

#[tokio::test]
async fn responses_anthropic_invalid_id_types_retain_accounting_without_fallback() {
    for (label, id) in [
        ("number", json!(123)),
        ("negative", json!(-1)),
        ("float", json!(1.5)),
        ("boolean", json!(true)),
        ("array", json!([{"large":"discard"}])),
        ("object", json!({"nested":[1,2]})),
    ] {
        for stream in [false, true] {
            for combo in [false, true] {
                let upstream = if stream {
                    UpstreamMock::Sse(
                        format!(
                            "data: {}\n\n",
                            json!({"type":"message_start","message":{"usage":{"input_tokens":3,"output_tokens":2}}})
                        ) + &super::responses::initial_anthropic_tool_stream(json!({}))
                            .replace("\"id\":\"call_initial\"", &format!("\"id\":{id}")),
                    )
                } else {
                    UpstreamMock::Json(
                        json!({"model":"claude-e2e","content":[{"type":"tool_use","id":id,"name":"lookup","input":{}}],"stop_reason":"tool_use","usage":{"input_tokens":3,"output_tokens":2}}),
                    )
                };
                let g =
                    build_gateway_planned(vec![], UpstreamMock::Json(openai_ok_body()), upstream)
                        .await;
                let request = json!({"model":if combo {"rook/anthropic-first"} else {"claude-e2e"},"input":"hi","stream":stream});
                if stream {
                    let (_, _, body) = post_sse(g.app, "/v1/responses", request).await;
                    assert_failed_terminal_usage(&body, &g.accounting);
                    assert!(!body.contains("response.output_item.added"), "{body}");
                    super::responses::fixture(
                        &format!("anthropic-invalid-id-{label}-{combo}.sse"),
                        &body,
                    );
                } else {
                    let (status, body) = post_json(g.app, "/v1/responses", request, &[]).await;
                    assert_eq!(status, StatusCode::BAD_GATEWAY, "{label}: {body}");
                    super::responses::fixture(
                        &format!("anthropic-invalid-id-{label}-{combo}-http-error.json"),
                        &body.to_string(),
                    );
                }
                assert_anthropic_accounting(&g.accounting, combo, RequestStatus::Failure, true);
                assert!(
                    (g.accounting.usage.lock().unwrap()[0].cost_usd.unwrap() - 7.0 / 1_000_000.0)
                        .abs()
                        < 1e-12
                );
                assert!(g
                    .upstreams
                    .openai
                    .received_requests()
                    .await
                    .unwrap()
                    .is_empty());
                assert_eq!(
                    g.upstreams
                        .anthropic
                        .received_requests()
                        .await
                        .unwrap()
                        .len(),
                    1
                );
                assert!(g
                    .fallback
                    .circuit_states()
                    .iter()
                    .all(|(_, state)| state.failures == 0));
            }
        }
    }
}

#[tokio::test]
async fn responses_openai_invalid_ids_retain_same_frame_usage() {
    for (label, id) in [
        ("null", json!(null)),
        ("empty", json!("")),
        ("control", json!("bad\n")),
        ("long", json!("x".repeat(257))),
        ("number", json!(123)),
        ("boolean", json!(true)),
        ("array", json!([])),
        ("object", json!({})),
    ] {
        for stream in [false, true] {
            if stream && id.is_null() {
                continue;
            } // OpenAI delta null is neutral.
            for combo in [false, true] {
                let usage = json!({"prompt_tokens":3,"completion_tokens":2,"total_tokens":5});
                let call = json!({"index":0,"id":id,"function":{"name":"lookup","arguments":"{}"}});
                let upstream = if stream {
                    let prior = json!({"model":"gpt-4o","choices":[{"delta":{"content":"prior"},"finish_reason":null}]});
                    let invalid = json!({"model":"gpt-4o","choices":[{"delta":{"tool_calls":[call]},"finish_reason":"tool_calls"}],"usage":usage});
                    UpstreamMock::Sse(format!(
                        "data: {prior}\n\ndata: {invalid}\n\ndata: [DONE]\n\n"
                    ))
                } else {
                    UpstreamMock::Json(
                        json!({"model":"gpt-4o","choices":[{"message":{"tool_calls":[call]},"finish_reason":"tool_calls"}],"usage":usage}),
                    )
                };
                let g = build_gateway_planned(vec![], upstream, UpstreamMock::Fail500).await;
                let request = json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hi","stream":stream});
                if stream {
                    let (_, _, body) = post_sse(g.app, "/v1/responses", request).await;
                    assert_failed_terminal_usage(&body, &g.accounting);
                    assert!(!body.contains("response.function_call_arguments.delta"));
                    super::responses::fixture(
                        &format!("openai-invalid-id-{label}-{combo}.sse"),
                        &body,
                    );
                } else {
                    let (status, body) = post_json(g.app, "/v1/responses", request, &[]).await;
                    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
                    super::responses::fixture(
                        &format!("openai-invalid-id-{label}-{combo}-http-error.json"),
                        &body.to_string(),
                    );
                }
                assert_failure(&g.accounting, combo);
                assert!(g
                    .upstreams
                    .anthropic
                    .received_requests()
                    .await
                    .unwrap()
                    .is_empty());
            }
        }
    }
}

fn assert_anthropic_snapshot(records: &Records, combo: bool, status: RequestStatus, output: u32) {
    assert_anthropic_accounting(records, combo, status, false);
    let audit = records.audit.lock().unwrap();
    let usage = records.usage.lock().unwrap();
    let tokens = audit[0].usage.as_ref().unwrap();
    assert_eq!(tokens.prompt_tokens, 12);
    assert_eq!(tokens.completion_tokens, output);
    assert_eq!(tokens.total_tokens, 12 + output);
    assert_eq!(tokens.cache_read_tokens, Some(8));
    assert_eq!(tokens.cache_creation_tokens, Some(4));
    assert_eq!(usage[0].tokens_prompt, Some(12));
    assert_eq!(usage[0].tokens_completion, Some(u64::from(output)));
    assert_eq!(usage[0].tokens_cache_read, Some(8));
    assert_eq!(usage[0].tokens_cache_creation, Some(4));
    let expected = (22.0 + 2.0 * f64::from(output)) / 1_000_000.0;
    assert!((usage[0].cost_usd.unwrap() - expected).abs() < 1e-12);
}

#[tokio::test]
async fn responses_anthropic_nonstream_missing_ids_generate_unique_replayable_calls() {
    for combo in [false, true] {
        let upstream = json!({"model":"claude-e2e","content":[
            {"type":"tool_use","name":"lookup","input":{"key":"one"}},
            {"type":"tool_use","name":"search","input":{"key":"two"}}
        ],"stop_reason":"tool_use","usage":{"input_tokens":3,"output_tokens":2}});
        let g = build_gateway_planned(vec![], UpstreamMock::Fail500, UpstreamMock::Json(upstream))
            .await;
        let model = if combo {
            "rook/anthropic-first"
        } else {
            "claude-e2e"
        };
        let (status, body) = post_json(
            g.app.clone(),
            "/v1/responses",
            json!({"model":model,"input":"hi"}),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "completed");
        let calls = body["output"].as_array().unwrap();
        assert_eq!(calls.len(), 2);
        assert_ne!(calls[0]["call_id"], calls[1]["call_id"]);
        assert_ne!(calls[0]["id"], calls[1]["id"]);
        for (call, name, key) in [(&calls[0], "lookup", "one"), (&calls[1], "search", "two")] {
            assert_eq!(call["type"], "function_call");
            assert!(call["call_id"].as_str().unwrap().starts_with("call_"));
            assert_eq!(call["name"], name);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(call["arguments"].as_str().unwrap())
                    .unwrap(),
                json!({"key":key})
            );
        }
        assert_anthropic_accounting(&g.accounting, combo, RequestStatus::Success, true);
        super::responses::fixture(
            &format!("anthropic-nonstream-generated-ids-{combo}.json"),
            &body.to_string(),
        );
        let mut input = calls.clone();
        input.extend(calls.iter().map(|call| json!({"type":"function_call_output","call_id":call["call_id"],"output":"found"})));
        let (status, replay) = post_json(
            g.app,
            "/v1/responses",
            json!({"model":model,"input":input}),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replay}");
        let requests = g.upstreams.anthropic.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        let sent: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert_eq!(sent["messages"][0]["role"], "assistant");
        for (index, call) in calls.iter().enumerate() {
            let block = &sent["messages"][0]["content"][index];
            assert_eq!(block["id"], call["call_id"]);
            assert_eq!(block["name"], call["name"]);
            assert_eq!(
                block["input"],
                serde_json::from_str::<serde_json::Value>(call["arguments"].as_str().unwrap())
                    .unwrap()
            );
            assert_eq!(
                sent["messages"][index + 1]["content"][0]["tool_use_id"],
                call["call_id"]
            );
        }
        assert!(g
            .upstreams
            .openai
            .received_requests()
            .await
            .unwrap()
            .is_empty());
    }
}

#[tokio::test]
async fn responses_anthropic_nonstream_explicit_invalid_ids_are_not_generated() {
    for id in [String::new(), "bad\n".into(), "x".repeat(257)] {
        for combo in [false, true] {
            let upstream = json!({"model":"claude-e2e","content":[{"type":"tool_use","id":id,"name":"lookup","input":{}}],"stop_reason":"tool_use","usage":{"input_tokens":3,"output_tokens":2}});
            let g =
                build_gateway_planned(vec![], UpstreamMock::Fail500, UpstreamMock::Json(upstream))
                    .await;
            let (status, body) = post_json(
                g.app,
                "/v1/responses",
                json!({"model":if combo {"rook/anthropic-first"} else {"claude-e2e"},"input":"hi"}),
                &[],
            )
            .await;
            assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
            assert_anthropic_accounting(&g.accounting, combo, RequestStatus::Failure, true);
            assert!(g
                .upstreams
                .openai
                .received_requests()
                .await
                .unwrap()
                .is_empty());
            assert!(g
                .fallback
                .circuit_states()
                .iter()
                .all(|(_, state)| state.failures == 0));
        }
    }
}

async fn assert_anthropic_explicit_null_id_rejected(stream: bool, combo: bool) {
    let upstream = if stream {
        let usage = json!({"type":"message_start","message":{"usage":{"input_tokens":3,"output_tokens":2}}});
        UpstreamMock::Sse(
            format!("data: {usage}\n\n")
                + &super::responses::initial_anthropic_tool_stream(json!({}))
                    .replace("\"id\":\"call_initial\"", "\"id\":null"),
        )
    } else {
        UpstreamMock::Json(
            json!({"model":"claude-e2e","content":[{"type":"tool_use","id":null,"name":"lookup","input":{}}],"stop_reason":"tool_use","usage":{"input_tokens":3,"output_tokens":2}}),
        )
    };
    let g = build_gateway_planned(vec![], UpstreamMock::Fail500, upstream).await;
    let request = json!({"model":if combo {"rook/anthropic-first"} else {"claude-e2e"},"input":"hi","stream":stream});
    if stream {
        let (status, _, body) = post_sse(g.app, "/v1/responses", request).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_failed_terminal_usage(&body, &g.accounting);
        assert!(!body.contains("response.completed"), "{body}");
        assert!(!body.contains("response.output_item.added"), "{body}");
        super::responses::fixture(&format!("anthropic-null-id-{combo}.sse"), &body);
    } else {
        let (status, body) = post_json(g.app, "/v1/responses", request, &[]).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
        super::responses::fixture(
            &format!("anthropic-null-id-{combo}-http-error.json"),
            &body.to_string(),
        );
    }
    assert_anthropic_accounting(&g.accounting, combo, RequestStatus::Failure, true);
    assert!(g
        .upstreams
        .openai
        .received_requests()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn responses_anthropic_explicit_null_id_nonstream_standard() {
    assert_anthropic_explicit_null_id_rejected(false, false).await;
}

#[tokio::test]
async fn responses_anthropic_explicit_null_id_nonstream_combo() {
    assert_anthropic_explicit_null_id_rejected(false, true).await;
}

#[tokio::test]
async fn responses_anthropic_explicit_null_id_stream_standard() {
    assert_anthropic_explicit_null_id_rejected(true, false).await;
}

#[tokio::test]
async fn responses_anthropic_explicit_null_id_stream_combo() {
    assert_anthropic_explicit_null_id_rejected(true, true).await;
}

#[tokio::test]
async fn responses_openai_unknown_finish_cannot_be_neutral_late_usage() {
    for prior_finish in [true, false] {
        for reason in [None, Some(json!(null)), Some(json!("unknown"))] {
            for combo in [false, true] {
                let first = json!({"model":"gpt-4o","choices":[{"delta":{"content":"prior"},"finish_reason":if prior_finish {json!("stop")} else {json!(null)}}]});
                let mut choice = json!({"delta":{}});
                if let Some(reason) = &reason {
                    choice["finish_reason"] = reason.clone();
                }
                let last = json!({"model":"gpt-4o","choices":[choice],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}});
                let g = build_gateway_planned(
                    vec![],
                    UpstreamMock::Sse(format!("data: {first}\n\ndata: {last}\n\n")),
                    UpstreamMock::Fail500,
                )
                .await;
                let (_, _, body) = post_sse(g.app, "/v1/responses", json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hi","stream":true})).await;
                let unknown = reason.as_ref().is_some_and(|r| !r.is_null());
                if unknown || !prior_finish {
                    assert_failed_terminal_usage(&body, &g.accounting);
                    assert_failure(&g.accounting, combo);
                    assert!(!body.contains("response.completed"));
                    assert!(!sse_json(&body)
                        .iter()
                        .any(|e| e["type"].as_str().unwrap().ends_with(".done")));
                } else {
                    let events = sse_json(&body);
                    assert_eq!(
                        events.last().unwrap()["type"],
                        "response.completed",
                        "{body}"
                    );
                    assert_eq!(
                        events.last().unwrap()["response"]["usage"]["total_tokens"],
                        5
                    );
                    assert_eq!(
                        g.accounting.audit.lock().unwrap()[0].status,
                        RequestStatus::Success
                    );
                }
                assert_eq!(
                    g.accounting.audit.lock().unwrap()[0]
                        .usage
                        .as_ref()
                        .unwrap()
                        .total_tokens,
                    5
                );
                assert_eq!(
                    g.accounting.usage.lock().unwrap()[0].tokens_completion,
                    Some(2)
                );
                assert!(
                    (g.accounting.usage.lock().unwrap()[0].cost_usd.unwrap() - 7.0 / 1_000_000.0)
                        .abs()
                        < 1e-12
                );
                assert!(body.contains("prior"));
                assert!(g
                    .upstreams
                    .anthropic
                    .received_requests()
                    .await
                    .unwrap()
                    .is_empty());
                for (id, state) in g.fallback.circuit_states() {
                    assert_eq!(
                        state.failures,
                        u32::from(unknown && id.as_str() == "openai-e2e")
                    );
                }
                super::responses::fixture(
                    &format!(
                        "openai-finish-usage-{prior_finish}-{}-{combo}.sse",
                        if unknown {
                            "unknown"
                        } else if reason.is_none() {
                            "absent"
                        } else {
                            "null"
                        }
                    ),
                    &body,
                );
            }
        }
    }
}

#[tokio::test]
async fn responses_openai_initial_unknown_finish_retains_precommit_fallback() {
    for combo in [false, true] {
        let frame = json!({"model":"gpt-4o","choices":[{"delta":{"content":"rejected"},"finish_reason":"unknown"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}});
        let g = build_gateway_planned(
            vec![],
            UpstreamMock::Sse(format!("data: {frame}\n\n")),
            UpstreamMock::Sse(anthropic_text_stream(json!({"stop_reason":"end_turn"}))),
        )
        .await;
        let (_, _, body) = post_sse(
            g.app,
            "/v1/responses",
            json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hi","stream":true}),
        )
        .await;
        assert!(!body.contains("rejected"));
        if combo {
            assert_eq!(
                sse_json(&body).last().unwrap()["type"],
                "response.completed",
                "{body}"
            );
            assert!(body.contains("previo"));
        } else {
            assert_failed_terminal_usage(&body, &g.accounting);
            assert_failure(&g.accounting, false);
        }
        {
            let audit = g.accounting.audit.lock().unwrap();
            let usage = g.accounting.usage.lock().unwrap();
            assert_eq!(audit.len(), if combo { 2 } else { 1 });
            assert_eq!(usage.len(), audit.len());
            let failed = audit
                .iter()
                .find(|e| e.provider.as_str() == "openai-e2e")
                .unwrap();
            assert_eq!(failed.status, RequestStatus::Failure);
            assert_eq!(failed.usage.as_ref().unwrap().total_tokens, 5);
            let failed_usage = usage
                .iter()
                .find(|e| e.request_id == failed.request_id && e.provider == failed.provider)
                .unwrap();
            assert_eq!(failed_usage.tokens_prompt, Some(3));
            assert_eq!(failed_usage.tokens_completion, Some(2));
        }
        super::responses::fixture(&format!("openai-initial-unknown-{combo}.sse"), &body);
    }
}

#[tokio::test]
async fn responses_anthropic_usage_omissions_retain_updated_counters_and_zeros() {
    for delta in [json!({}), json!({"stop_reason":null})] {
        for zero in [false, true] {
            for combo in [false, true] {
                let mut sse = anthropic_initial_usage()
                    + &format!(
                        "data: {}\n\ndata: {}\n\n",
                        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}),
                        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":14,"cache_read_input_tokens":10,"cache_creation_input_tokens":5,"output_tokens":2}})
                    );
                if zero {
                    sse += &format!(
                        "data: {}\n\n",
                        json!({"type":"message_delta","delta":delta,"usage":{"input_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":3}})
                    );
                }
                sse += &format!(
                    "data: {}\n\n",
                    json!({"type":"message_delta","delta":delta,"usage":{"output_tokens":7}})
                );
                let g =
                    build_gateway_planned(vec![], UpstreamMock::Fail500, UpstreamMock::Sse(sse))
                        .await;
                let (_, _, body) = post_sse(g.app, "/v1/responses", json!({"model":if combo {"rook/anthropic-first"} else {"claude-e2e"},"input":"hi","stream":true})).await;
                let events = sse_json(&body);
                let terminal = events.last().unwrap();
                assert_eq!(terminal["type"], "response.completed", "{body}");
                let (input, read, creation) = if zero { (0, 0, 0) } else { (14, 10, 5) };
                assert_eq!(terminal["response"]["usage"]["input_tokens"], input);
                assert_eq!(terminal["response"]["usage"]["output_tokens"], 7);
                assert_eq!(terminal["response"]["usage"]["total_tokens"], input + 7);
                assert_eq!(
                    terminal["response"]["usage"]["input_tokens_details"]["cached_tokens"],
                    read
                );
                assert_anthropic_accounting(&g.accounting, combo, RequestStatus::Success, false);
                {
                    let audit = g.accounting.audit.lock().unwrap();
                    let usage = g.accounting.usage.lock().unwrap();
                    let tokens = audit[0].usage.as_ref().unwrap();
                    assert_eq!(tokens.prompt_tokens, input);
                    assert_eq!(tokens.completion_tokens, 7);
                    assert_eq!(tokens.total_tokens, input + 7);
                    assert_eq!(tokens.cache_read_tokens, Some(read));
                    assert_eq!(tokens.cache_creation_tokens, Some(creation));
                    assert_eq!(usage[0].tokens_prompt, Some(u64::from(input)));
                    assert_eq!(usage[0].tokens_completion, Some(7));
                    assert_eq!(usage[0].tokens_cache_read, Some(read));
                    assert_eq!(usage[0].tokens_cache_creation, Some(creation));
                    let expected = if zero { 14.0 } else { 40.5 } / 1_000_000.0;
                    assert!((usage[0].cost_usd.unwrap() - expected).abs() < 1e-12);
                }
                assert!(g
                    .upstreams
                    .openai
                    .received_requests()
                    .await
                    .unwrap()
                    .is_empty());
                super::responses::fixture(
                    &format!(
                        "anthropic-updated-usage-{zero}-{}-{combo}.sse",
                        delta.get("stop_reason").is_some()
                    ),
                    &body,
                );
            }
        }
    }
}

#[tokio::test]
async fn responses_anthropic_unknown_finish_after_recognized_finish_still_fails() {
    for tool in [false, true] {
        for combo in [false, true] {
            let finished = if tool {
                super::responses::initial_anthropic_tool_stream(json!({}))
                    .replace("\"input_tokens\":3,", "")
            } else {
                anthropic_text_stream(json!({"stop_reason":"end_turn"}))
                    .replace("\"input_tokens\":3,", "")
            };
            let sse = anthropic_initial_usage()
                + &finished
                + &format!(
                    "data: {}\n\n",
                    json!({"type":"message_delta","delta":{"stop_reason":"unknown"},"usage":{"output_tokens":7}})
                );
            let g =
                build_gateway_planned(vec![], UpstreamMock::Fail500, UpstreamMock::Sse(sse)).await;
            let (_, _, body) = post_sse(g.app, "/v1/responses", json!({"model":if combo {"rook/anthropic-first"} else {"claude-e2e"},"input":"hi","stream":true})).await;
            assert_failed_terminal_usage(&body, &g.accounting);
            assert_anthropic_snapshot(&g.accounting, combo, RequestStatus::Failure, 7);
            assert!(!body.contains("response.completed"));
            assert!(g
                .upstreams
                .openai
                .received_requests()
                .await
                .unwrap()
                .is_empty());
            super::responses::fixture(
                &format!("anthropic-unknown-after-finish-{tool}-{combo}.sse"),
                &body,
            );
        }
    }
}

#[tokio::test]
async fn responses_anthropic_late_usage_completes_with_latest_snapshot() {
    for id in [false, true] {
        for delta in [json!({}), json!({"stop_reason":null})] {
            for combo in [false, true] {
                let mut tool = super::responses::initial_anthropic_tool_stream(json!({}))
                    .replace("\"input_tokens\":3,", "");
                if !id {
                    tool = tool.replace("\"id\":\"call_initial\",", "");
                }
                let sse = anthropic_initial_usage()
                    + &tool
                    + &format!(
                        "data: {}\n\n",
                        json!({"type":"message_delta","delta":delta,"usage":{"output_tokens":7}})
                    );
                let g =
                    build_gateway_planned(vec![], UpstreamMock::Fail500, UpstreamMock::Sse(sse))
                        .await;
                let (_, _, body) = post_sse(g.app, "/v1/responses", json!({"model":if combo {"rook/anthropic-first"} else {"claude-e2e"},"input":"hi","stream":true})).await;
                let events = sse_json(&body);
                let terminal = events.last().unwrap();
                assert_eq!(terminal["type"], "response.completed", "{body}");
                assert_eq!(terminal["response"]["usage"]["input_tokens"], 12);
                assert_eq!(terminal["response"]["usage"]["output_tokens"], 7);
                assert_eq!(terminal["response"]["usage"]["total_tokens"], 19);
                assert_eq!(
                    terminal["response"]["usage"]["input_tokens_details"]["cached_tokens"],
                    8
                );
                assert_eq!(terminal["response"]["output"].as_array().unwrap().len(), 1);
                assert_eq!(terminal["response"]["output"][0]["name"], "lookup");
                assert_eq!(terminal["response"]["output"][0]["arguments"], "{}");
                assert_eq!(
                    events
                        .iter()
                        .filter(|e| e["type"] == "response.function_call_arguments.delta")
                        .count(),
                    1
                );
                assert_anthropic_snapshot(&g.accounting, combo, RequestStatus::Success, 7);
                assert!(g
                    .upstreams
                    .openai
                    .received_requests()
                    .await
                    .unwrap()
                    .is_empty());
                super::responses::fixture(
                    &format!(
                        "anthropic-late-usage-{id}-{}-{combo}.sse",
                        delta.get("stop_reason").is_some()
                    ),
                    &body,
                );
            }
        }
    }
}

#[tokio::test]
async fn responses_anthropic_initial_usage_survives_error_eof_and_commits_selection() {
    for suffix in [
        "",
        "data: {\"type\":\"error\",\"error\":{\"type\":\"api_error\",\"message\":\"failed\"}}\n\n",
    ] {
        for text in [false, true] {
            for combo in [false, true] {
                let mut sse = anthropic_initial_usage();
                if text {
                    sse += &format!(
                        "data: {}\n\n",
                        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"observed"}})
                    );
                }
                sse += suffix;
                let g = build_gateway_planned(
                    vec![],
                    UpstreamMock::Sse(openai_stream_frame(
                        json!({"content":"fallback"}),
                        Some("stop"),
                    )),
                    UpstreamMock::Sse(sse),
                )
                .await;
                let (_, _, body) = post_sse(g.app, "/v1/responses", json!({"model":if combo {"rook/anthropic-first"} else {"claude-e2e"},"input":"hi","stream":true})).await;
                assert_failed_terminal_usage(&body, &g.accounting);
                assert_anthropic_snapshot(&g.accounting, combo, RequestStatus::Failure, 0);
                let events = sse_json(&body);
                assert_eq!(
                    events
                        .iter()
                        .filter(|e| e["type"] == "response.failed")
                        .count(),
                    1
                );
                assert_eq!(
                    events.last().unwrap()["response"]["usage"]["input_tokens_details"]
                        ["cached_tokens"],
                    8
                );
                assert_eq!(body.contains("observed"), text);
                assert!(!body.contains("fallback"));
                assert!(g
                    .upstreams
                    .openai
                    .received_requests()
                    .await
                    .unwrap()
                    .is_empty());
                super::responses::fixture(
                    &format!(
                        "anthropic-initial-failure-{}-{text}-{combo}.sse",
                        !suffix.is_empty()
                    ),
                    &body,
                );
            }
        }
    }
}

#[tokio::test]
async fn responses_anthropic_initial_usage_survives_body_cancellation() {
    use futures::StreamExt;
    for combo in [false, true] {
        let sse =
            anthropic_initial_usage() + &anthropic_text_stream(json!({"stop_reason":"end_turn"}));
        let g = build_gateway_planned(vec![], UpstreamMock::Fail500, UpstreamMock::Sse(sse)).await;
        let response = g.app.oneshot(Request::builder().method(Method::POST).uri("/v1/responses")
            .header("content-type", "application/json").header("authorization", "Bearer e2e-key-1")
            .body(axum::body::Body::from(json!({"model":if combo {"rook/anthropic-first"} else {"claude-e2e"},"input":"hi","stream":true}).to_string())).unwrap()).await.unwrap();
        let mut body = response.into_body().into_data_stream();
        loop {
            let frame = body.next().await.unwrap().unwrap();
            if String::from_utf8_lossy(&frame).contains("response.output_text.delta") {
                break;
            }
        }
        // Stop before polling message_delta, irrespective of upstream HTTP frame batching.
        drop(body);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while g.accounting.usage.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_anthropic_snapshot(&g.accounting, combo, RequestStatus::Failure, 0);
        assert!(g
            .fallback
            .circuit_states()
            .iter()
            .all(|(_, state)| state.failures == 0));
        assert!(g
            .upstreams
            .openai
            .received_requests()
            .await
            .unwrap()
            .is_empty());
    }
}

fn anthropic_text_stream(delta: serde_json::Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\n",
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"previo"}}),
        json!({"type":"message_delta","delta":delta,"usage":{"input_tokens":3,"output_tokens":2}})
    )
}

fn assert_anthropic_accounting(
    records: &Records,
    combo: bool,
    status: RequestStatus,
    tokens: bool,
) {
    let audit = records.audit.lock().unwrap();
    let usage = records.usage.lock().unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(usage.len(), 1);
    assert_eq!(audit[0].status, status);
    assert_eq!(usage[0].status, status);
    assert_eq!(audit[0].provider.as_str(), "anthropic-e2e");
    assert_eq!(audit[0].model.as_str(), "claude-e2e");
    assert_eq!(audit[0].combo_id.is_some(), combo);
    assert_eq!(audit[0].request_id, usage[0].request_id);
    assert_eq!(audit[0].provider, usage[0].provider);
    assert_eq!(audit[0].model, usage[0].model);
    assert!(usage[0].api_key_id.is_some());
    if tokens {
        assert_eq!(audit[0].usage.as_ref().unwrap().total_tokens, 5);
        assert_eq!(usage[0].tokens_prompt, Some(3));
        assert_eq!(usage[0].tokens_completion, Some(2));
    }
}

#[tokio::test]
async fn responses_anthropic_missing_or_unknown_finish_never_succeeds() {
    for delta in [json!({}), json!({"stop_reason":"unknown"})] {
        for stream in [false, true] {
            for combo in [false, true] {
                let mut upstream = json!({"model":"claude-e2e","content":[{"type":"text","text":"previo"}],"usage":{"input_tokens":3,"output_tokens":2}});
                if let Some(reason) = delta.get("stop_reason") {
                    upstream["stop_reason"] = reason.clone();
                }
                let g = build_gateway_planned(
                    vec![],
                    UpstreamMock::Sse(openai_stream_frame(
                        json!({"content":"fallback"}),
                        Some("stop"),
                    )),
                    if stream {
                        UpstreamMock::Sse(anthropic_text_stream(delta.clone()))
                    } else {
                        UpstreamMock::Json(upstream)
                    },
                )
                .await;
                let request = json!({"model":if combo {"rook/anthropic-first"} else {"claude-e2e"},"input":"hola","stream":stream});
                if stream {
                    let (_, _, body) = post_sse(g.app, "/v1/responses", request).await;
                    assert_failed_terminal_usage(&body, &g.accounting);
                    assert!(body.contains("previo"));
                    assert!(!body.contains("response.completed"));
                    super::responses::fixture(
                        &format!(
                            "anthropic-missing-finish-{}-{combo}.sse",
                            delta.get("stop_reason").is_some()
                        ),
                        &body,
                    );
                } else {
                    let (status, body) = post_json(g.app, "/v1/responses", request, &[]).await;
                    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
                }
                assert_anthropic_accounting(&g.accounting, combo, RequestStatus::Failure, true);
                assert!(g
                    .upstreams
                    .openai
                    .received_requests()
                    .await
                    .unwrap()
                    .is_empty());
                for (id, state) in g.fallback.circuit_states() {
                    // Explicit unknown reasons are provider validation errors, not
                    // missing-finish errors detected by the transport at EOF.
                    let expected = u32::from(
                        stream
                            && delta.get("stop_reason").is_some()
                            && id.as_str() == "anthropic-e2e",
                    );
                    assert_eq!(state.failures, expected);
                }
            }
        }
    }
}

#[tokio::test]
async fn responses_anthropic_malformed_and_pending_sse_preserve_prior_usage() {
    for (label, suffix) in [
        ("malformed", "data: {bad}\n\n"),
        (
            "truncated",
            "data: {\"type\":\"error\",\"error\":{\"usage\":{\"input_tokens\":999}",
        ),
    ] {
        for prior in [false, true] {
            for combo in [false, true] {
                let prefix = if prior {
                    anthropic_text_stream(json!({"stop_reason":"end_turn"}))
                } else {
                    String::new()
                };
                let g = build_gateway_planned(
                    vec![],
                    UpstreamMock::Sse(openai_stream_frame(
                        json!({"content":"fallback"}),
                        Some("stop"),
                    )),
                    UpstreamMock::Sse(prefix + suffix),
                )
                .await;
                let (_, _, body) = post_sse(g.app, "/v1/responses", json!({"model":if combo {"rook/anthropic-first"} else {"claude-e2e"},"input":"hola","stream":true})).await;
                if !prior && combo {
                    assert!(body.contains("response.completed"), "{body}");
                    assert!(body.contains("fallback"));
                    let audit = g.accounting.audit.lock().unwrap();
                    let usage = g.accounting.usage.lock().unwrap();
                    assert_eq!(audit.len(), 2);
                    assert_eq!(usage.len(), 2);
                    assert_eq!(
                        audit
                            .iter()
                            .filter(|entry| entry.status == RequestStatus::Failure
                                && entry.provider.as_str() == "anthropic-e2e")
                            .count(),
                        1
                    );
                    assert_eq!(
                        audit
                            .iter()
                            .filter(|entry| entry.status == RequestStatus::Success
                                && entry.provider.as_str() == "openai-e2e")
                            .count(),
                        1
                    );
                } else {
                    let events = sse_json(&body);
                    assert_eq!(events.last().unwrap()["type"], "response.failed", "{body}");
                    assert!(!body.contains("response.completed"));
                    assert_eq!(body.contains("previo"), prior);
                    assert!(!body.contains("\"input_tokens\":999"));
                    assert_anthropic_accounting(
                        &g.accounting,
                        combo,
                        RequestStatus::Failure,
                        prior,
                    );
                    if prior {
                        assert_failed_terminal_usage(&body, &g.accounting);
                    }
                    assert!(g
                        .upstreams
                        .openai
                        .received_requests()
                        .await
                        .unwrap()
                        .is_empty());
                }
                assert_eq!(
                    g.upstreams
                        .anthropic
                        .received_requests()
                        .await
                        .unwrap()
                        .len(),
                    1
                );
                assert_eq!(
                    g.fallback
                        .circuit_states()
                        .iter()
                        .find(|(id, _)| id.as_str() == "anthropic-e2e")
                        .unwrap()
                        .1
                        .failures,
                    1
                );
                super::responses::fixture(&format!("anthropic-{label}-{prior}-{combo}.sse"), &body);
            }
        }
    }
}

#[tokio::test]
async fn responses_anthropic_no_upstream_id_publishes_one_stable_call() {
    for combo in [false, true] {
        let sse = super::responses::initial_anthropic_tool_stream(json!({}))
            .replace("\"id\":\"call_initial\",", "");
        let g = build_gateway_planned(vec![], UpstreamMock::Fail500, UpstreamMock::Sse(sse)).await;
        let (_, _, body) = post_sse(g.app, "/v1/responses", json!({"model":if combo {"rook/anthropic-first"} else {"claude-e2e"},"input":"hola","stream":true})).await;
        let events = sse_json(&body);
        let terminal = events.last().unwrap();
        assert_eq!(terminal["type"], "response.completed", "{body}");
        let output = &terminal["response"]["output"][0];
        assert!(output["call_id"].as_str().unwrap().starts_with("call_"));
        assert_eq!(output["name"], "lookup");
        assert_eq!(output["arguments"], "{}");
        for event in &events {
            if event.get("item").is_some() {
                assert_eq!(event["item"]["call_id"], output["call_id"]);
                assert_eq!(event["item"]["id"], output["id"]);
            }
            if event.get("item_id").is_some() {
                assert_eq!(event["item_id"], output["id"]);
            }
        }
        assert_eq!(
            events
                .iter()
                .filter(|e| e["type"] == "response.function_call_arguments.delta")
                .count(),
            1
        );
        assert_eq!(terminal["response"]["usage"]["total_tokens"], 5);
        assert_anthropic_accounting(&g.accounting, combo, RequestStatus::Success, true);
        assert!(g
            .upstreams
            .openai
            .received_requests()
            .await
            .unwrap()
            .is_empty());
        super::responses::fixture(&format!("anthropic-generated-id-{combo}.sse"), &body);
    }
}

#[tokio::test]
async fn responses_anthropic_clean_eof_ignores_ping_and_comments() {
    for combo in [false, true] {
        let sse = ": keepalive\n\ndata: {\"type\":\"ping\"}\n\n".to_string()
            + &anthropic_text_stream(json!({"stop_reason":"end_turn"}))
            + ": after finish\n\ndata: {\"type\":\"ping\"}\n\n";
        let g = build_gateway_planned(vec![], UpstreamMock::Fail500, UpstreamMock::Sse(sse)).await;
        let (_, _, body) = post_sse(g.app, "/v1/responses", json!({"model":if combo {"rook/anthropic-first"} else {"claude-e2e"},"input":"hola","stream":true})).await;
        assert_eq!(
            sse_json(&body).last().unwrap()["type"],
            "response.completed",
            "{body}"
        );
        assert_anthropic_accounting(&g.accounting, combo, RequestStatus::Success, true);
    }
}

#[tokio::test]
async fn responses_anthropic_fixes_keep_chat_and_messages_legacy_contracts() {
    for messages in [false, true] {
        for stream in [false, true] {
            for case in ["missing", "unknown", "malformed", "pending", "no_id"] {
                let mut delta = json!({"stop_reason":"end_turn"});
                if case == "missing" {
                    delta = json!({});
                }
                if case == "unknown" {
                    delta = json!({"stop_reason":"unknown"});
                }
                let mut upstream = json!({"model":"claude-e2e","content":[{"type":"text","text":"previo"}],"usage":{"input_tokens":3,"output_tokens":2}});
                if let Some(reason) = delta.get("stop_reason") {
                    upstream["stop_reason"] = reason.clone();
                }
                let sse = if case == "no_id" {
                    super::responses::initial_anthropic_tool_stream(json!({}))
                        .replace("\"id\":\"call_initial\",", "")
                } else {
                    anthropic_text_stream(delta)
                        + match case {
                            "malformed" => "data: {bad}\n\n",
                            "pending" => "data: {\"type\":\"error\",\"error\":{",
                            _ => "",
                        }
                };
                let g = build_gateway_planned(
                    vec![],
                    UpstreamMock::Fail500,
                    if stream {
                        UpstreamMock::Sse(sse)
                    } else {
                        UpstreamMock::Json(upstream)
                    },
                )
                .await;
                let (uri, request) = if messages {
                    ("/v1/messages", anthropic_body("claude-e2e", stream))
                } else {
                    ("/v1/chat/completions", openai_body("claude-e2e", stream))
                };
                if stream {
                    let (status, _, body) = post_sse(g.app, uri, request).await;
                    assert_eq!(
                        status,
                        StatusCode::OK,
                        "case={case}, messages={messages}: {body}"
                    );
                    assert!(
                        sse_json(&body).iter().all(|e| e.get("error").is_none()),
                        "case={case}, messages={messages}: {body}"
                    );
                    assert!(body.ends_with("data: [DONE]\n\n"));
                } else {
                    let (status, body) = post_json(g.app, uri, request, &[]).await;
                    assert_eq!(status, StatusCode::OK, "{body}");
                    if messages {
                        assert_eq!(body["stop_reason"], "end_turn");
                    } else {
                        assert_eq!(body["choices"][0]["finish_reason"], "stop");
                    }
                }
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Records {
    audit: Mutex<Vec<AuditEntry>>,
    usage: Mutex<Vec<UsageEntry>>,
}

#[async_trait]
impl AuditPort for Records {
    async fn record(&self, entry: AuditEntry) -> CortexResult<()> {
        self.audit.lock().unwrap().push(entry);
        Ok(())
    }
}

#[async_trait]
impl UsageRecorderPort for Records {
    async fn record(&self, entry: UsageEntry) -> CortexResult<()> {
        self.usage.lock().unwrap().push(entry);
        Ok(())
    }
    async fn list(&self, _: UsageFilters, _: Pagination) -> CortexResult<Vec<UsageEntry>> {
        Ok(self.usage.lock().unwrap().clone())
    }
    async fn count(&self, _: UsageFilters) -> CortexResult<u64> {
        Ok(self.usage.lock().unwrap().len() as u64)
    }
    async fn summary(&self, _: UsageFilters) -> CortexResult<UsageSummary> {
        unimplemented!()
    }
    async fn cost_breakdown(&self, _: UsageFilters) -> CortexResult<CostBreakdown> {
        unimplemented!()
    }
}

fn assert_failure(records: &Records, combo: bool) {
    assert_failure_tokens(records, combo, 3, 2);
}

fn assert_failure_tokens(records: &Records, combo: bool, prompt: u64, completion: u64) {
    let audit = records.audit.lock().unwrap();
    let usage = records.usage.lock().unwrap();
    assert_eq!(audit.len(), 1, "un solo resultado definitivo");
    assert_eq!(usage.len(), 1, "un solo uso definitivo");
    assert_eq!(audit[0].status, RequestStatus::Failure);
    assert_eq!(usage[0].status, RequestStatus::Failure);
    assert_eq!(
        audit[0].usage.as_ref().unwrap().prompt_tokens,
        prompt as u32
    );
    assert_eq!(usage[0].tokens_prompt, Some(prompt));
    assert_eq!(usage[0].tokens_completion, Some(completion));
    assert!(usage[0].cost_usd.unwrap() > 0.0);
    assert_eq!(audit[0].provider.as_str(), "openai-e2e");
    assert_eq!(audit[0].model.as_str(), "gpt-4o");
    assert_eq!(usage[0].provider, audit[0].provider);
    assert_eq!(usage[0].request_id, audit[0].request_id);
    assert_eq!(audit[0].combo_id.is_some(), combo);
    assert!(usage[0].api_key_id.is_some());
}

fn assert_failed_terminal_usage(body: &str, records: &Records) {
    let events = sse_json(body);
    let terminal = events.last().unwrap();
    assert_eq!(terminal["type"], "response.failed", "{body}");
    let audit = records.audit.lock().unwrap();
    let usage = records.usage.lock().unwrap();
    let observed = audit[0].usage.as_ref().unwrap();
    let published = &terminal["response"]["usage"];
    assert_eq!(published["input_tokens"], observed.prompt_tokens, "{body}");
    assert_eq!(
        published["output_tokens"], observed.completion_tokens,
        "{body}"
    );
    assert_eq!(published["total_tokens"], observed.total_tokens, "{body}");
    assert_eq!(published["input_tokens"].as_u64(), usage[0].tokens_prompt);
    assert_eq!(
        published["output_tokens"].as_u64(),
        usage[0].tokens_completion
    );
}

#[tokio::test]
async fn responses_terminal_nonobject_frame_is_rejected_before_delivery() {
    for combo in [false, true] {
        for prior in [false, true] {
            let first = json!({"model":"gpt-4o","choices":[{"delta":{"content":"previo"},"finish_reason":null}]});
            let bad = json!({"model":"gpt-4o","choices":[{"delta":{"content":"rechazado","tool_calls":[{"index":0,"id":"call_rejected","function":{"name":"lookup","arguments":"[]"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}});
            let sse = format!(
                "{}data: {bad}\n\ndata: [DONE]\n\n",
                if prior {
                    format!("data: {first}\n\n")
                } else {
                    String::new()
                }
            );
            let g =
                build_gateway_planned(vec![], UpstreamMock::Sse(sse), UpstreamMock::Fail500).await;
            let (_, _, body) = post_sse(
                g.app,
                "/v1/responses",
                json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hola","stream":true}),
            )
            .await;
            assert!(body.contains("response.failed"), "{body}");
            assert!(!body.contains("rechazado"), "{body}");
            assert!(!body.contains("call_rejected"), "{body}");
            assert!(
                !body.contains("response.function_call_arguments.delta"),
                "{body}"
            );
            assert_eq!(body.contains("previo"), prior);
            assert_failure(&g.accounting, combo);
            assert_failed_terminal_usage(&body, &g.accounting);
            assert_eq!(
                sse_json(&body)
                    .iter()
                    .filter(|e| e["type"] == "response.failed")
                    .count(),
                1
            );
            assert!(g
                .upstreams
                .anthropic
                .received_requests()
                .await
                .unwrap()
                .is_empty());
            assert!(g
                .fallback
                .circuit_states()
                .iter()
                .all(|(_, state)| state.failures == 0));
            assert!(!sse_json(&body)
                .iter()
                .any(|event| event["type"].as_str().unwrap().ends_with(".done")));
            super::responses::fixture(&format!("terminal-nonobject-{combo}-{prior}.sse"), &body);
        }
    }
}

#[tokio::test]
async fn anthropic_stop_with_functions_records_one_failure_before_done() {
    for combo in [false, true] {
        let sse = many_anthropic_tool_stream(1)
            .replace(
                "\"stop_reason\":\"tool_use\"",
                "\"stop_reason\":\"end_turn\"",
            )
            .replace(
                "\"output_tokens\":7",
                "\"input_tokens\":3,\"output_tokens\":2",
            );
        let g = build_gateway_planned(vec![], UpstreamMock::Fail500, UpstreamMock::Sse(sse)).await;
        // Skip the first combo step without creating an unrelated failed attempt.
        for _ in 0..3 {
            g.fallback
                .on_failure(
                    &ProviderId::new("openai-e2e"),
                    &rook_core::CortexError::provider("fixture unavailable"),
                )
                .await;
        }
        let (status, _, body) = post_sse(
            g.app,
            "/v1/responses",
            json!({"model":if combo {"rook/e2e"} else {"claude-e2e"},"input":"hola","stream":true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let events = sse_json(&body);
        assert_eq!(events.last().unwrap()["type"], "response.failed", "{body}");
        assert_eq!(events.iter().filter(|e| e["type"] == "error").count(), 1);
        assert_eq!(
            events
                .iter()
                .filter(|e| e["type"] == "response.failed")
                .count(),
            1
        );
        assert!(!body.contains("response.completed"));
        assert!(!events
            .iter()
            .any(|e| e["type"].as_str().unwrap().ends_with(".done")));
        {
            let audit = g.accounting.audit.lock().unwrap();
            let usage = g.accounting.usage.lock().unwrap();
            assert_eq!(audit.len(), 1);
            assert_eq!(usage.len(), 1);
            assert_eq!(audit[0].status, RequestStatus::Failure);
            assert_eq!(usage[0].status, RequestStatus::Failure);
            assert_eq!(audit[0].provider.as_str(), "anthropic-e2e");
            assert_eq!(audit[0].combo_id.is_some(), combo);
            assert_eq!(audit[0].request_id, usage[0].request_id);
        }
        assert_failed_terminal_usage(&body, &g.accounting);
        assert!(g
            .upstreams
            .openai
            .received_requests()
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            g.upstreams
                .anthropic
                .received_requests()
                .await
                .unwrap()
                .len(),
            1
        );
        super::responses::fixture(&format!("anthropic-stop-functions-{combo}.sse"), &body);
    }
}

#[tokio::test]
async fn provider_validation_error_keeps_usage_from_the_rejected_frame() {
    for observe_error in [false, true] {
        for combo in [false, true] {
            let first = json!({"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"hola"},"finish_reason":null}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}});
            let invalid = json!({"model":"gpt-4o","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_bad","function":{"name":if observe_error {"x".repeat(4097)} else {"lookup".into()},"arguments":"{"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}});
            let sse = format!("data: {first}\n\ndata: {invalid}\n\ndata: [DONE]\n\n");
            let g =
                build_gateway_planned(vec![], UpstreamMock::Sse(sse), UpstreamMock::Fail500).await;
            let (_, _, body) = post_sse(
                g.app,
                "/v1/responses",
                json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hola","stream":true}),
            )
            .await;
            assert!(body.contains("response.failed"), "{body}");
            assert!(!body.contains("response.completed"));
            assert!(!body.contains("response.function_call_arguments.delta"));
            assert!(!body.contains("call_bad"));
            assert!(!sse_json(&body)
                .iter()
                .any(|event| event["type"].as_str().unwrap().ends_with(".done")));
            assert_failure(&g.accounting, combo);
            assert_failed_terminal_usage(&body, &g.accounting);
            super::responses::fixture(
                &format!("validation-late-{observe_error}-{combo}.sse"),
                &body,
            );
            assert!(g
                .upstreams
                .anthropic
                .received_requests()
                .await
                .unwrap()
                .is_empty());
        }
    }
}

#[tokio::test]
async fn first_frame_validation_error_preserves_usage_and_combo_fallback() {
    for combo in [false, true] {
        let frame = json!({"model":"gpt-4o","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_bad","function":{"name":"lookup","arguments":"{"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}});
        let g = build_gateway_planned(
            vec![],
            UpstreamMock::Sse(format!("data: {frame}\n\ndata: [DONE]\n\n")),
            UpstreamMock::Sse(many_anthropic_tool_stream(1)),
        )
        .await;
        let (_, _, body) = post_sse(
            g.app,
            "/v1/responses",
            json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hola","stream":true}),
        )
        .await;
        if combo {
            assert!(body.contains("response.completed"), "{body}");
            assert!(!body.contains("response.failed"));
            let audit = g.accounting.audit.lock().unwrap();
            let usage = g.accounting.usage.lock().unwrap();
            assert_eq!(audit.len(), 2);
            assert_eq!(usage.len(), 2);
            assert_eq!(
                audit
                    .iter()
                    .filter(|e| e.status == RequestStatus::Failure)
                    .count(),
                1
            );
            let failure = audit
                .iter()
                .find(|e| e.status == RequestStatus::Failure)
                .unwrap();
            assert_eq!(failure.provider.as_str(), "openai-e2e");
            assert_eq!(failure.model.as_str(), "gpt-4o");
            assert_eq!(failure.usage.as_ref().unwrap().total_tokens, 5);
            let failure_usage = usage
                .iter()
                .find(|e| e.status == RequestStatus::Failure)
                .unwrap();
            assert_eq!(failure_usage.tokens_prompt, Some(3));
            assert_eq!(failure_usage.tokens_completion, Some(2));
            assert!(failure_usage.cost_usd.unwrap() > 0.0);
            assert_eq!(failure_usage.provider, failure.provider);
            assert_eq!(failure_usage.model, failure.model);
            assert_eq!(failure_usage.request_id, failure.request_id);
            let success = audit
                .iter()
                .find(|e| e.status == RequestStatus::Success)
                .unwrap();
            let success_usage = usage
                .iter()
                .find(|e| e.status == RequestStatus::Success)
                .unwrap();
            let events = sse_json(&body);
            let terminal_usage = &events.last().unwrap()["response"]["usage"];
            let observed = success.usage.as_ref().unwrap();
            assert_eq!(terminal_usage["input_tokens"], observed.prompt_tokens);
            assert_eq!(terminal_usage["output_tokens"], observed.completion_tokens);
            assert_eq!(terminal_usage["total_tokens"], observed.total_tokens);
            assert_eq!(
                terminal_usage["input_tokens"].as_u64(),
                success_usage.tokens_prompt
            );
            assert_eq!(
                terminal_usage["output_tokens"].as_u64(),
                success_usage.tokens_completion
            );
            super::responses::fixture("validation-first-combo-fallback.sse", &body);
        } else {
            assert!(body.contains("response.failed"), "{body}");
            assert!(!body.contains("response.completed"));
            assert_failure(&g.accounting, false);
            assert_failed_terminal_usage(&body, &g.accounting);
            super::responses::fixture("validation-first-standard.sse", &body);
        }
        assert!(!body.contains("call_bad"));
    }
}

async fn rejected_nonstream(combo: bool) {
    let mut body = openai_ok_body();
    body["choices"][0]["message"]["tool_calls"] =
        json!([{"id":"call_a","type":"function","function":{"name":"bad name","arguments":"{}"}}]);
    body["choices"][0]["finish_reason"] = json!("tool_calls");
    let g = build_gateway_planned(vec![], UpstreamMock::Json(body), UpstreamMock::Fail500).await;
    let (status, response) = post_json(
        g.app,
        "/v1/responses",
        json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hola"}),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    super::responses::fixture(
        &format!(
            "validation-http-error-{}.json",
            if combo { "combo" } else { "standard" }
        ),
        &response.to_string(),
    );
    assert_failure(&g.accounting, combo);
    assert!(g
        .fallback
        .circuit_states()
        .iter()
        .all(|(_, state)| state.failures == 0));
    assert!(g
        .upstreams
        .anthropic
        .received_requests()
        .await
        .unwrap()
        .is_empty());
}

async fn rejected_stream(combo: bool, push: bool, prior_frame: bool) {
    let first = json!({"model":"gpt-4o","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":if push {"bad name"} else {"lookup"},"arguments":"[]"}}]},"finish_reason":null}],"usage":{"prompt_tokens":if push {3} else {1},"completion_tokens":if push {2} else {1},"total_tokens":if push {5} else {2}}});
    let last =
        json!({"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
    let mut sse = String::new();
    if prior_frame {
        let prior = json!({"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"hola"},"finish_reason":null}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}});
        sse.push_str(&format!("data: {prior}\n\n"));
    }
    sse.push_str(&format!("data: {first}\n\ndata: {last}\n\ndata: {{\"model\":\"gpt-4o\",\"choices\":[],\"usage\":{{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}}}\n\ndata: [DONE]\n\n"));
    let g = build_gateway_planned(vec![], UpstreamMock::Sse(sse), UpstreamMock::Fail500).await;
    let (_, _, body) = post_sse(
        g.app,
        "/v1/responses",
        json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hola","stream":true}),
    )
    .await;
    assert!(body.contains("response.failed"), "{body}");
    assert!(!body.contains("response.completed"));
    if push {
        assert_failure(&g.accounting, combo);
    } else {
        // El rechazo ocurre al recibir finish; no se lee el usage posterior.
        assert_failure_tokens(&g.accounting, combo, 1, 1);
    }
    assert_failed_terminal_usage(&body, &g.accounting);
    assert!(!sse_json(&body)
        .iter()
        .any(|event| event["type"].as_str().unwrap().ends_with(".done")));
    if push {
        assert!(!body.contains("bad name"));
        assert!(!body.contains("call_a"));
        assert!(!body.contains("response.function_call_arguments.delta"));
        assert_eq!(body.contains("response.output_text.delta"), prior_frame);
    }
    assert!(g
        .fallback
        .circuit_states()
        .iter()
        .all(|(_, state)| state.failures == 0));
    assert!(g
        .upstreams
        .anthropic
        .received_requests()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn standard_nonstream_validation_precedes_accounting() {
    rejected_nonstream(false).await;
}
#[tokio::test]
async fn combo_nonstream_validation_precedes_accounting() {
    rejected_nonstream(true).await;
}
#[tokio::test]
async fn standard_push_rejection_records_observed_usage() {
    rejected_stream(false, true, false).await;
}
#[tokio::test]
async fn combo_push_rejection_records_observed_usage() {
    rejected_stream(true, true, false).await;
}
#[tokio::test]
async fn standard_late_push_rejection_records_observed_usage() {
    rejected_stream(false, true, true).await;
}
#[tokio::test]
async fn combo_late_push_rejection_records_observed_usage() {
    rejected_stream(true, true, true).await;
}
#[tokio::test]
async fn standard_finish_rejection_records_observed_usage() {
    rejected_stream(false, false, false).await;
}
#[tokio::test]
async fn combo_finish_rejection_records_observed_usage() {
    rejected_stream(true, false, false).await;
}

#[tokio::test]
async fn repeated_name_fragments_survive_the_http_boundary() {
    let mut sse = String::new();
    for delta in [
        json!({"tool_calls":[{"index":0,"id":"call_a","function":{"name":"foo"}}]}),
        json!({"tool_calls":[{"index":0,"function":{"name":"foo"}}]}),
        json!({"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]}),
    ] {
        sse.push_str(&format!(
            "data: {}\n\n",
            json!({"model":"gpt-4o","choices":[{"index":0,"delta":delta,"finish_reason":null}]})
        ));
    }
    sse.push_str("data: {\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n");
    let g = build_gateway_planned(vec![], UpstreamMock::Sse(sse), UpstreamMock::Fail500).await;
    let (_, _, body) = post_sse(
        g.app,
        "/v1/responses",
        json!({"model":"gpt-4o","input":"hola","stream":true}),
    )
    .await;
    assert!(body.contains("response.completed"), "{body}");
    let events: Vec<serde_json::Value> = body
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events.last().unwrap()["response"]["output"][0]["name"],
        "foofoo"
    );
}

#[tokio::test]
async fn incoherent_nonstream_terminal_states_fail_at_http_boundary() {
    for finish in ["unknown", "tool_calls"] {
        for combo in [false, true] {
            let mut body = openai_ok_body();
            body["choices"][0]["finish_reason"] = json!(finish);
            let g = build_gateway_planned(vec![], UpstreamMock::Json(body), UpstreamMock::Fail500)
                .await;
            let (status, _) = post_json(
                g.app,
                "/v1/responses",
                json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hola"}),
                &[],
            )
            .await;
            assert_eq!(status, StatusCode::BAD_GATEWAY);
            assert_failure(&g.accounting, combo);
        }
    }
}

#[tokio::test]
async fn nonstream_functions_require_tool_calls_or_incomplete_finish() {
    for finish in ["stop", "tool_calls", "length", "content_filter"] {
        for combo in [false, true] {
            let mut body = openai_ok_body();
            body["choices"][0]["message"]["tool_calls"] = json!([{"id":"call_a","type":"function","function":{"name":"lookup","arguments":"{}"}}]);
            body["choices"][0]["finish_reason"] = json!(finish);
            let g = build_gateway_planned(vec![], UpstreamMock::Json(body), UpstreamMock::Fail500)
                .await;
            let (status, response) = post_json(
                g.app,
                "/v1/responses",
                json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hola"}),
                &[],
            )
            .await;
            if finish == "stop" {
                assert_eq!(status, StatusCode::BAD_GATEWAY, "{response}");
                assert_failure(&g.accounting, combo);
                assert!(g
                    .fallback
                    .circuit_states()
                    .iter()
                    .all(|(_, state)| state.failures == 0));
                assert!(g
                    .upstreams
                    .anthropic
                    .received_requests()
                    .await
                    .unwrap()
                    .is_empty());
            } else {
                assert_eq!(status, StatusCode::OK, "{response}");
                assert_eq!(
                    response["status"],
                    if finish == "tool_calls" {
                        "completed"
                    } else {
                        "incomplete"
                    }
                );
                assert_eq!(
                    g.accounting.audit.lock().unwrap()[0].status,
                    RequestStatus::Success
                );
            }
        }
    }
}

#[tokio::test]
async fn responses_disconnect_keeps_observed_usage_without_provider_failure() {
    use futures::StreamExt;
    for combo in [false, true] {
        let first = json!({"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"hola"},"finish_reason":null}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}});
        let sse = format!("data: {first}\n\ndata: {{\"model\":\"gpt-4o\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n");
        let g = build_gateway_planned(vec![], UpstreamMock::Sse(sse), UpstreamMock::Fail500).await;
        let response = g.app.oneshot(Request::builder().method(Method::POST).uri("/v1/responses")
            .header("content-type", "application/json").header("authorization", "Bearer e2e-key-1")
            .body(axum::body::Body::from(json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hola","stream":true}).to_string())).unwrap()).await.unwrap();
        let mut body = response.into_body().into_data_stream();
        loop {
            let frame = body.next().await.unwrap().unwrap();
            if String::from_utf8_lossy(&frame).contains("response.output_text.delta") {
                break;
            }
        }
        drop(body);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while g.accounting.usage.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_failure(&g.accounting, combo);
        assert!(g
            .fallback
            .circuit_states()
            .iter()
            .all(|(_, state)| state.failures == 0));
    }
}

#[tokio::test]
async fn missing_stream_finish_records_failure_with_usage_before_eof() {
    for combo in [false, true] {
        let first = json!({"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"hola"},"finish_reason":null}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}});
        let g = build_gateway_planned(
            vec![],
            UpstreamMock::Sse(format!("data: {first}\n\ndata: [DONE]\n\n")),
            UpstreamMock::Fail500,
        )
        .await;
        let (_, _, body) = post_sse(
            g.app,
            "/v1/responses",
            json!({"model":if combo {"rook/e2e"} else {"gpt-4o"},"input":"hola","stream":true}),
        )
        .await;
        assert!(body.contains("response.failed"), "{body}");
        assert!(!body.contains("response.completed"));
        assert_failure(&g.accounting, combo);
        assert_failed_terminal_usage(&body, &g.accounting);
        assert!(g
            .fallback
            .circuit_states()
            .iter()
            .all(|(_, state)| state.failures == 0));
    }
}

#[tokio::test]
async fn concurrent_responses_streams_have_independent_validation_state() {
    let sse = concat!(
        "data: {\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hola\"},\"finish_reason\":null}]}\n\n",
        "data: {\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let g =
        build_gateway_planned(vec![], UpstreamMock::Sse(sse.into()), UpstreamMock::Fail500).await;
    let request = json!({"model":"gpt-4o","input":"hola","stream":true});
    let (first, second) = tokio::join!(
        post_sse(g.app.clone(), "/v1/responses", request.clone()),
        post_sse(g.app, "/v1/responses", request)
    );
    for (_, _, body) in [first, second] {
        assert!(body.contains("response.completed"), "{body}");
        assert!(!body.contains("response.failed"));
    }
    let audit = g.accounting.audit.lock().unwrap();
    assert_eq!(audit.len(), 2);
    assert!(audit
        .iter()
        .all(|entry| entry.status == RequestStatus::Success));
    assert_ne!(audit[0].request_id, audit[1].request_id);
}
