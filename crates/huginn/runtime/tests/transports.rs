#[path = "support/runfiles.rs"]
mod runfiles;

use async_trait::async_trait;
use axum::{http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use futures::StreamExt;
use huginn_core::{
    ApprovalPolicy, ApprovalRequest, ModelDelta, ModelProvider, ModelRequest, Result, ToolCall,
    ToolContext, ToolRegistry,
};
use huginn_runtime::{
    mcp::{McpClients, ServerConfig},
    model::OpenAiProvider,
    tools::Registry,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct Approvals {
    allow: bool,
    seen: Mutex<Vec<String>>,
}
#[async_trait]
impl ApprovalPolicy for Approvals {
    async fn approve(
        &self,
        request: &ApprovalRequest,
        _: CancellationToken,
    ) -> Result<huginn_core::ApprovalDecision> {
        self.seen.lock().unwrap().push(request.action.clone());
        Ok(if self.allow {
            huginn_core::ApprovalDecision::ApproveOnce
        } else {
            huginn_core::ApprovalDecision::Reject
        })
    }
}
const fn request() -> ModelRequest {
    ModelRequest {
        messages: vec![],
        tools: vec![],
        max_tokens: 128,
    }
}
async fn server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (url, task)
}
#[tokio::test]
async fn http_adapter_sends_auth_and_assembles_tool_calls() {
    let seen = Arc::new(Mutex::new(None));
    let capture = seen.clone();
    let (url, task) = server(Router::new().route("/v1/chat/completions", post(move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
        let capture = capture.clone();
        async move {
            assert_eq!(headers["authorization"], "Bearer fixture-secret");
            *capture.lock().unwrap() = Some(body);
            let events = [
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"list_files","arguments":"{\"pa"}}]},"finish_reason":null}]}),
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\".\"}"}}]},"finish_reason":"tool_calls"}]}),
                json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}}),
            ];
            let mut data = events.iter().map(|e| format!("data: {e}\n\n")).collect::<String>();
            data.push_str("data: [DONE]\n\n");
            ([("content-type", "text/event-stream")], data)
        }
    }))).await;
    let provider = OpenAiProvider::with_transport(
        &url,
        "fixture-model".into(),
        Some("fixture-secret".into()),
        2,
        true,
    )
    .unwrap();
    let mut model_request = request();
    model_request.messages = vec![huginn_core::Message::text(
        huginn_core::Role::User,
        "fixture",
    )];
    let deltas: Vec<_> = provider
        .stream(model_request, CancellationToken::new())
        .await
        .unwrap()
        .collect()
        .await;
    assert!(deltas.iter().any(|d| matches!(d, Ok(ModelDelta::ToolCall(call)) if call.name == "list_files" && call.arguments == json!({"path":"."}))));
    assert!(matches!(deltas.last(), Some(Ok(ModelDelta::Finished))));
    assert_eq!(
        seen.lock().unwrap().as_ref().unwrap()["model"],
        "fixture-model"
    );
    assert_eq!(
        seen.lock().unwrap().as_ref().unwrap()["messages"][0]["role"],
        "user"
    );
    assert!(seen
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .get("reasoning_effort")
        .is_none());
    task.abort();
}

#[tokio::test]
async fn openai_provider_serializes_reasoning_effort_only_when_configured() {
    let seen = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let capture = seen.clone();
    let (url, task) = server(Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<serde_json::Value>| {
            let capture = capture.clone();
            async move {
                capture.lock().unwrap().push(body);
                let data = format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]})
                );
                ([("content-type", "text/event-stream")], data)
            }
        }),
    ))
    .await;

    let with_high = OpenAiProvider::with_transport(
        &url,
        "o3-mini".into(),
        Some("fixture-secret".into()),
        2,
        true,
    )
    .unwrap()
    .with_reasoning_effort(Some(huginn_core::providers::ReasoningEffort::High));
    let mut req = request();
    req.messages = vec![huginn_core::Message::text(
        huginn_core::Role::User,
        "reason",
    )];
    let _deltas: Vec<_> = with_high
        .stream(req.clone(), CancellationToken::new())
        .await
        .unwrap()
        .collect()
        .await;

    let without_effort = OpenAiProvider::with_transport(
        &url,
        "o3-mini".into(),
        Some("fixture-secret".into()),
        2,
        true,
    )
    .unwrap()
    .with_reasoning_effort(None);
    let _deltas: Vec<_> = without_effort
        .stream(req, CancellationToken::new())
        .await
        .unwrap()
        .collect()
        .await;

    let bodies = seen.lock().unwrap();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["reasoning_effort"], "high");
    assert!(bodies[1].get("reasoning_effort").is_none());
    task.abort();
}

#[tokio::test]
async fn openai_provider_serializes_image_attachments_as_multimodal_content() {
    let seen = Arc::new(Mutex::new(None::<serde_json::Value>));
    let capture = seen.clone();
    let (url, task) = server(Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<serde_json::Value>| {
            let capture = capture.clone();
            async move {
                *capture.lock().unwrap() = Some(body);
                let data = format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"delta":{"content":"looks good"},"finish_reason":"stop"}]})
                );
                ([("content-type", "text/event-stream")], data)
            }
        }),
    ))
    .await;

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&4u32.to_be_bytes());
    png.extend_from_slice(&4u32.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0]);
    let attachment =
        huginn_core::ImageAttachment::from_bytes(png, Some("shot.png".into())).unwrap();

    let provider = OpenAiProvider::with_transport(
        &url,
        "gpt-4o".into(),
        Some("fixture-secret".into()),
        2,
        true,
    )
    .unwrap();
    let mut req = request();
    req.messages = vec![huginn_core::Message::user_with_images(
        "what is in this image?",
        vec![attachment],
    )];
    let _deltas: Vec<_> = provider
        .stream(req, CancellationToken::new())
        .await
        .unwrap()
        .collect()
        .await;

    let body = seen.lock().unwrap().clone().unwrap();
    let content = body["messages"][0]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "what is in this image?");
    assert_eq!(content[1]["type"], "image_url");
    assert!(content[1]["image_url"]["url"]
        .as_str()
        .unwrap()
        .starts_with("data:image/png;base64,"));
    task.abort();
}

#[tokio::test]
async fn siwc_vertical_slice_covers_pkce_models_responses_refresh_and_oidc_revoke() {
    use axum::routing::get;
    use huginn_core::{
        connections::{ConnectionMaterial, ConnectionSettings, Secret},
        providers::{
            ConnectionIdentity, ModelCatalog, ModelSelection, ProviderAdapter, ProviderRegistration,
        },
        siwc::{
            SiwcAuthPort, SiwcRevocationOutcome, SIWC_INITIAL_CLIENT_ID, SIWC_PROVIDER_ID,
            SIWC_REQUESTED_SCOPES,
        },
    };
    use huginn_runtime::siwc::{SiwcAdapter, SiwcEndpoints, SiwcOAuthClient};

    let revoked_tokens = Arc::new(Mutex::new(Vec::<String>::new()));
    let revoked_capture = revoked_tokens.clone();
    let seen_responses_body = Arc::new(Mutex::new(None::<serde_json::Value>));
    let body_capture = seen_responses_body.clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let revoke_url = format!("{base}/api/accounts/oauth/revoke");
    let revoke_url_for_oidc = revoke_url.clone();

    let router = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || {
                let url = revoke_url_for_oidc.clone();
                async move { Json(json!({ "revocation_endpoint": url })) }
            }),
        )
        .route(
            "/api/accounts/oauth/token",
            post(|body: String| async move {
                if body.contains("grant_type=refresh_token") {
                    Json(json!({
                        "access_token": "siwc-access-rotated",
                        "refresh_token": "siwc-refresh-rotated",
                        "client_id": "issued-client-123",
                        "expires_in": 3600,
                        "scope": SIWC_REQUESTED_SCOPES,
                    }))
                } else {
                    Json(json!({
                        "access_token": "siwc-access-initial",
                        "refresh_token": "siwc-refresh-initial",
                        "client_id": "issued-client-123",
                        "expires_in": 3600,
                        "scope": SIWC_REQUESTED_SCOPES,
                    }))
                }
            }),
        )
        .route(
            "/api/accounts/oauth/revoke",
            post(move |body: String| {
                let revoked_capture = revoked_capture.clone();
                async move {
                    revoked_capture.lock().unwrap().push(body);
                    StatusCode::OK
                }
            }),
        )
        .route(
            "/v1/models",
            get(|| async {
                Json(json!({
                    "models": [
                        { "slug": "gpt-5", "visibility": "list" },
                        { "slug": "internal-hidden", "visibility": "hide" }
                    ]
                }))
            }),
        )
        .route(
            "/v1/responses",
            post(move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
                let body_capture = body_capture.clone();
                async move {
                    assert_eq!(headers["authorization"], "Bearer siwc-access-rotated");
                    *body_capture.lock().unwrap() = Some(body);
                    let events = [
                        json!({"type": "response.output_text.delta", "delta": "Hello "}),
                        json!({"type": "response.output_text.delta", "delta": "SIWC"}),
                        json!({"type": "response.completed", "response": {"usage": {"input_tokens": 12, "output_tokens": 4}}}),
                    ];
                    let data = events
                        .iter()
                        .map(|e| format!("data: {e}\n\n"))
                        .collect::<String>();
                    ([("content-type", "text/event-stream")], data)
                }
            }),
        );
    let server_task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let endpoints = SiwcEndpoints {
        authorize_url: format!("{base}/api/accounts/authorize"),
        token_url: format!("{base}/api/accounts/oauth/token"),
        oidc_discovery_url: format!("{base}/.well-known/openid-configuration"),
        api_base_url: format!("{base}/v1"),
        allow_loopback_http: true,
        open_browser: false,
    };
    let oauth = Arc::new(SiwcOAuthClient::with_endpoints(endpoints.clone(), 5));

    // 1. PKCE + authorize URL + code exchange.
    let (verifier, challenge) = SiwcOAuthClient::generate_pkce();
    let auth_url = oauth
        .build_authorize_url(
            "http://127.0.0.1:4321/callback",
            "state-xyz",
            &challenge,
            "host-abc",
            SIWC_INITIAL_CLIENT_ID,
        )
        .unwrap();
    assert!(auth_url.contains("client_id=dynamic_agent_client"));
    assert!(auth_url.contains("code_challenge_method=S256"));
    let creds = oauth
        .exchange_code(
            "code-123",
            "http://127.0.0.1:4321/callback",
            &verifier,
            "host-abc",
            SIWC_INITIAL_CLIENT_ID,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(creds.client_id, "issued-client-123");

    // 2. Refresh rotation.
    let refreshed = oauth
        .refresh(&creds, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(refreshed.access_token, "siwc-access-rotated");
    assert_eq!(refreshed.refresh_token, "siwc-refresh-rotated");

    // 3. Discovery filters visibility == "list" and passes slug.
    let adapter = SiwcAdapter::with_endpoints(oauth.clone(), endpoints, 5);
    let material = ConnectionMaterial {
        settings: ConnectionSettings {
            provider: SIWC_PROVIDER_ID.into(),
            endpoint: Some(format!("{base}/v1")),
            model: "gpt-5".into(),
            requires_secret: true,
            allow_loopback_http: true,
            reasoning_effort: None,
        },
        secret: Some(Secret::new(refreshed.to_secret_json().unwrap()).unwrap()),
    };
    let catalog = adapter
        .discover_models(&material, CancellationToken::new())
        .await
        .unwrap();
    match catalog {
        ModelCatalog::Available(models) => {
            assert_eq!(models.len(), 1);
            assert_eq!(models[0].id, "gpt-5");
        }
        other => panic!("expected available SIWC catalog, got {other:?}"),
    }

    // 4. Responses API streaming (store: false, stream: true) and text-only enforcement.
    let selection = ModelSelection {
        connection: ConnectionIdentity {
            id: "siwc-conn".into(),
            adapter: ProviderRegistration::new(SIWC_PROVIDER_ID.into(), 1),
            revision: 1,
        },
        model: "gpt-5".into(),
        reasoning_effort: None,
    };
    let model = adapter
        .create_connection_model(&selection, material, CancellationToken::new())
        .await
        .unwrap();

    // Reject tools cleanly in first text-only slice.
    let mut tool_req = request();
    tool_req.tools.push(huginn_core::ToolDefinition {
        name: "list_files".into(),
        description: "list".into(),
        input_schema: json!({}),
    });
    assert!(model
        .stream(tool_req, CancellationToken::new())
        .await
        .is_err());

    let mut text_req = request();
    text_req.messages = vec![huginn_core::Message::text(huginn_core::Role::User, "hello")];
    let deltas: Vec<_> = model
        .stream(text_req, CancellationToken::new())
        .await
        .unwrap()
        .collect()
        .await;
    let combined: String = deltas
        .iter()
        .filter_map(|d| match d {
            Ok(ModelDelta::Text(t)) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(combined, "Hello SIWC");
    assert!(deltas.iter().any(|d| matches!(
        d,
        Ok(ModelDelta::Usage {
            input_tokens: Some(12),
            output_tokens: Some(4),
            ..
        })
    )));
    let sent_body = seen_responses_body.lock().unwrap().clone().unwrap();
    assert_eq!(sent_body["store"], false);
    assert_eq!(sent_body["stream"], true);
    assert_eq!(sent_body["model"], "gpt-5");

    // 5. OIDC-discovered remote revocation.
    let outcome = oauth
        .revoke_remote(&refreshed, CancellationToken::new())
        .await;
    assert_eq!(outcome, SiwcRevocationOutcome::RemoteConfirmed);
    assert_eq!(revoked_tokens.lock().unwrap().len(), 1);
    assert!(revoked_tokens.lock().unwrap()[0].contains("siwc-refresh-rotated"));

    server_task.abort();
}
#[tokio::test]
async fn http_errors_timeout_and_cancellation_do_not_leak_credentials() {
    let (url, task) = server(Router::new().route(
        "/v1/chat/completions",
        post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "fixture-secret").into_response() }),
    ))
    .await;
    let provider =
        OpenAiProvider::with_transport(&url, "m".into(), Some("fixture-secret".into()), 1, true)
            .unwrap();
    let error = match provider.stream(request(), CancellationToken::new()).await {
        Err(e) => e,
        Ok(_) => panic!("expected HTTP error"),
    };
    assert!(error.to_string().contains("500"));
    assert!(!error.to_string().contains("fixture-secret"));
    task.abort();
    let (url, task) = server(Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            ""
        }),
    ))
    .await;
    let provider = OpenAiProvider::with_transport(&url, "m".into(), None, 1, true).unwrap();
    assert!(provider
        .stream(request(), CancellationToken::new())
        .await
        .is_err());
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        provider.stream(request(), token).await,
        Err(huginn_core::AgentError::Cancelled)
    ));
    task.abort();
}

#[tokio::test]
async fn active_response_stream_can_exceed_the_header_timeout() {
    use axum::body::Body;
    use bytes::Bytes;
    use futures::stream;
    use std::convert::Infallible;

    let (url, task) = server(Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            let chunks = stream::unfold(0, |index| async move {
                if index == 3 {
                    return None;
                }
                tokio::time::sleep(Duration::from_millis(600)).await;
                let chunk = match index {
                    0 => Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n"),
                    1 => Bytes::from_static(b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n"),
                    _ => Bytes::from_static(b"data: [DONE]\n\n"),
                };
                Some((Ok::<_, Infallible>(chunk), index + 1))
            });
            ([("content-type", "text/event-stream")], Body::from_stream(chunks))
        }),
    )).await;
    let provider = OpenAiProvider::with_transport(&url, "m".into(), None, 1, true).unwrap();
    let deltas: Vec<_> = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap()
        .collect()
        .await;
    assert!(deltas
        .iter()
        .any(|delta| matches!(delta, Ok(ModelDelta::Text(text)) if text == "ok")));
    assert!(deltas
        .iter()
        .any(|delta| matches!(delta, Ok(ModelDelta::Finished))));
    task.abort();
}
fn mcp_config(marker: &std::path::Path) -> ServerConfig {
    ServerConfig {
        name: "fixture".into(),
        command: "python3".into(),
        args: vec![runfiles::resolve(
            "_main/crates/huginn/runtime/tests/fixtures/mcp_server.py",
            std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/mcp_server.py"
            )),
        )
        .display()
        .to_string()],
        env: BTreeMap::from([("MARKER".into(), marker.display().to_string())]),
        env_from: BTreeMap::new(),
    }
}

#[tokio::test]
async fn mcp_protocol_error_does_not_cancel_server_for_later_calls() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("marker");
    let registry = Registry::native(1).unwrap();
    let policy = Approvals {
        allow: true,
        seen: Mutex::new(vec![]),
    };
    let mut clients = McpClients::connect(
        &[mcp_config(&marker)],
        temp.path(),
        &registry,
        &policy,
        CancellationToken::new(),
        3,
    )
    .await
    .unwrap();
    let tool = registry.get("mcp_fixture_0").unwrap();
    let ctx = huginn_core::ToolContext {
        selected_skills: vec![],
        workspace: temp.path().into(),
        cancellation: CancellationToken::new(),
    };
    let protocol_error_call = ToolCall {
        id: "call-protocol-error".into(),
        name: "mcp_fixture_0".into(),
        arguments: json!({"mode":"protocol_error"}),
    };
    let prepared = tool.prepare(&protocol_error_call, &ctx).await.unwrap();
    let error = tool.execute(prepared, &ctx).await.unwrap_err();
    assert!(error.to_string().contains("MCP error"));
    let next_call = ToolCall {
        id: "call-ok".into(),
        name: "mcp_fixture_0".into(),
        arguments: json!({"mode":"ok"}),
    };
    let prepared = tool.prepare(&next_call, &ctx).await.unwrap();
    assert!(tool
        .execute(prepared, &ctx)
        .await
        .unwrap()
        .contains("fixture result"));
    assert_eq!(
        std::fs::read_to_string(marker).unwrap(),
        "started\ncalled\ncalled\n"
    );
    clients.shutdown().await.unwrap();
}
#[tokio::test]
async fn mcp_denied_launch_and_approved_discovery_call_and_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("marker");
    let registry = Registry::native(1).unwrap();
    let policy = Approvals {
        allow: false,
        seen: Mutex::new(vec![]),
    };
    let mut clients = McpClients::connect(
        &[mcp_config(&marker)],
        temp.path(),
        &registry,
        &policy,
        CancellationToken::new(),
        2,
    )
    .await
    .unwrap();
    assert!(!marker.exists());
    assert!(registry.get("mcp_fixture_0").is_none());
    clients.shutdown().await.unwrap();
    let policy = Approvals {
        allow: true,
        seen: Mutex::new(vec![]),
    };
    let mut clients = McpClients::connect(
        &[mcp_config(&marker)],
        temp.path(),
        &registry,
        &policy,
        CancellationToken::new(),
        10,
    )
    .await
    .unwrap();
    assert_eq!(*policy.seen.lock().unwrap(), ["mcp.fixture.start"]);
    let tool = registry.get("mcp_fixture_0").unwrap();
    let ctx = ToolContext {
        selected_skills: vec![],
        workspace: temp.path().to_path_buf(),
        cancellation: CancellationToken::new(),
    };
    let prepared = tool
        .prepare(
            &ToolCall {
                id: "call".into(),
                name: "mcp_fixture_0".into(),
                arguments: json!({}),
            },
            &ctx,
        )
        .await
        .unwrap();
    let approval = prepared.approval.as_ref().unwrap();
    assert_eq!(approval.action, "mcp.fixture.echo");
    assert!(approval.preview.contains("Server/action: fixture"));
    assert!(approval.preview.contains("Tool: echo"));
    assert!(approval
        .preview
        .contains("sent to the configured MCP server"));
    assert!(tool
        .execute(prepared, &ctx)
        .await
        .unwrap()
        .contains("fixture result"));
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        "started\ncalled\n"
    );
    tokio::time::timeout(Duration::from_secs(6), clients.shutdown())
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn mcp_crash_and_timeout_are_reported_and_shutdown_is_bounded() {
    for mode in ["crash", "timeout"] {
        let temp = tempfile::tempdir().unwrap();
        let registry = Registry::native(1).unwrap();
        let policy = Approvals {
            allow: true,
            seen: Mutex::new(vec![]),
        };
        let mut clients = McpClients::connect(
            &[mcp_config(&temp.path().join("marker"))],
            temp.path(),
            &registry,
            &policy,
            CancellationToken::new(),
            5,
        )
        .await
        .unwrap();
        let ctx = ToolContext {
            selected_skills: vec![],
            workspace: temp.path().into(),
            cancellation: CancellationToken::new(),
        };
        let tool = registry.get("mcp_fixture_0").unwrap();
        let prepared = tool
            .prepare(
                &ToolCall {
                    id: "call".into(),
                    name: "mcp_fixture_0".into(),
                    arguments: json!({"mode":mode}),
                },
                &ctx,
            )
            .await
            .unwrap();
        let error = tool.execute(prepared, &ctx).await.unwrap_err();
        assert!(error.to_string().contains("unknown"));
        tokio::time::timeout(Duration::from_secs(6), clients.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
}

#[test]
fn sse_parser_distinguishes_missing_partial_zero_and_cumulative_usage() {
    use huginn_core::{RequestUsageAccumulator, UsageCompleteness, UsageSemantics};
    use huginn_runtime::model::SseParser;

    // 1. Missing completion_tokens must remain None, never become 0.
    let mut parser = SseParser::default();
    let deltas = parser
        .push(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10}}\n\ndata: [DONE]\n\n",
        )
        .unwrap();
    let usage = deltas
        .iter()
        .find_map(|d| match d {
            ModelDelta::Usage {
                input_tokens,
                output_tokens,
                semantics,
            } => Some((*input_tokens, *output_tokens, *semantics)),
            _ => None,
        })
        .expect("usage delta");
    assert_eq!(usage, (Some(10), None, UsageSemantics::Cumulative));

    // 2. Explicit null or empty usage object must not fabricate zeros.
    let mut parser = SseParser::default();
    let deltas = parser
        .push(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":null,\"completion_tokens\":null}}\n\ndata: [DONE]\n\n",
        )
        .unwrap();
    let usage = deltas
        .iter()
        .find_map(|d| match d {
            ModelDelta::Usage {
                input_tokens,
                output_tokens,
                semantics,
            } => Some((*input_tokens, *output_tokens, *semantics)),
            _ => None,
        })
        .expect("usage delta");
    assert_eq!(usage, (None, None, UsageSemantics::Cumulative));

    // 3. Explicit zero remains Some(0), distinguishable from None.
    let mut parser = SseParser::default();
    let deltas = parser
        .push(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":0,\"completion_tokens\":0}}\n\ndata: [DONE]\n\n",
        )
        .unwrap();
    let usage = deltas
        .iter()
        .find_map(|d| match d {
            ModelDelta::Usage {
                input_tokens,
                output_tokens,
                semantics,
            } => Some((*input_tokens, *output_tokens, *semantics)),
            _ => None,
        })
        .expect("usage delta");
    assert_eq!(usage, (Some(0), Some(0), UsageSemantics::Cumulative));

    // 4. Total absence emits no usage delta.
    let mut parser = SseParser::default();
    let deltas = parser
        .push(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}],\"usage\":null}\n\ndata: [DONE]\n\n",
        )
        .unwrap();
    assert!(!deltas.iter().any(|d| matches!(d, ModelDelta::Usage { .. })));

    // 5. Multiple cumulative snapshots in one SSE response do not double-count.
    let mut parser = SseParser::default();
    let deltas = parser
        .push(
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":5}}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"b\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":12}}\n\n",
                "data: [DONE]\n\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let mut acc = RequestUsageAccumulator::default();
    for delta in deltas {
        if let ModelDelta::Usage {
            input_tokens,
            output_tokens,
            semantics,
        } = delta
        {
            acc.observe(input_tokens, output_tokens, semantics);
        }
    }
    let reported = acc.reported();
    assert_eq!(reported.input_tokens, Some(100));
    assert_eq!(reported.output_tokens, Some(12));
    assert_eq!(reported.completeness(), UsageCompleteness::Complete);
}
