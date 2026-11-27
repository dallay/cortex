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
    // Compatible (non-official) endpoints preserve `max_tokens` instead of switching to
    // OpenAI's `max_completion_tokens`.
    assert_eq!(bodies[0]["max_tokens"], request().max_tokens);
    assert!(bodies[0].get("max_completion_tokens").is_none());
    assert_eq!(bodies[1]["max_tokens"], request().max_tokens);
    assert!(bodies[1].get("max_completion_tokens").is_none());
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

fn test_token(prefix: &str, suffix: &str) -> String {
    format!("{prefix}-{suffix}")
}

fn sign_test_id_token(
    keypair: &ring::signature::Ed25519KeyPair,
    issuer: &str,
    aud: &str,
    nonce: &str,
    sub: &str,
) -> String {
    use base64::Engine as _;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({"alg": "EdDSA", "kid": "test-k1", "typ": "JWT"})).unwrap(),
    );
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({
            "iss": issuer,
            "aud": aud,
            "sub": sub,
            "nonce": nonce,
            "email": "dev@example.com",
            "exp": now + 3600,
        }))
        .unwrap(),
    );
    let signing_input = format!("{header}.{claims}");
    let sig = keypair.sign(signing_input.as_bytes());
    let sig_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig.as_ref());
    format!("{signing_input}.{sig_b64}")
}

struct InMemoryPersistence {
    stored: tokio::sync::Mutex<Option<huginn_core::siwc::SiwcCredentials>>,
    writes: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl huginn_core::siwc::SiwcCredentialPersistence for InMemoryPersistence {
    async fn persist_rotated(
        &self,
        _: huginn_core::connections::ConnectionRevision,
        credentials: &huginn_core::siwc::SiwcCredentials,
    ) -> Result<()> {
        self.writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *self.stored.lock().await = Some(credentials.clone());
        Ok(())
    }
    async fn load_latest(
        &self,
        _: huginn_core::connections::ConnectionRevision,
    ) -> Result<Option<huginn_core::siwc::SiwcCredentials>> {
        Ok(self.stored.lock().await.clone())
    }
}

async fn spawn_loopback_mock_server(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), task)
}

struct FailOncePersistence {
    stored: tokio::sync::Mutex<Option<huginn_core::siwc::SiwcCredentials>>,
    writes: std::sync::atomic::AtomicUsize,
    write_before_error: bool,
}

#[async_trait]
impl huginn_core::siwc::SiwcCredentialPersistence for FailOncePersistence {
    async fn persist_rotated(
        &self,
        _: huginn_core::connections::ConnectionRevision,
        credentials: &huginn_core::siwc::SiwcCredentials,
    ) -> Result<()> {
        let attempt = self
            .writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if attempt == 0 {
            if self.write_before_error {
                *self.stored.lock().await = Some(credentials.clone());
            }
            return Err(huginn_core::AgentError::Configuration(
                "fixture persistence failure".into(),
            ));
        }
        *self.stored.lock().await = Some(credentials.clone());
        Ok(())
    }

    async fn load_latest(
        &self,
        _: huginn_core::connections::ConnectionRevision,
    ) -> Result<Option<huginn_core::siwc::SiwcCredentials>> {
        Ok(self.stored.lock().await.clone())
    }
}

struct RotatingSiwcAuth {
    refreshed: huginn_core::siwc::SiwcCredentials,
    refreshes: std::sync::atomic::AtomicUsize,
    input_refresh_tokens: Mutex<Vec<String>>,
}

#[async_trait]
impl huginn_core::siwc::SiwcAuthPort for RotatingSiwcAuth {
    async fn sign_in(&self, _: CancellationToken) -> Result<huginn_core::siwc::SiwcCredentials> {
        Err(huginn_core::AgentError::Configuration(
            "sign-in is not used by this fixture".into(),
        ))
    }

    async fn refresh(
        &self,
        credentials: &huginn_core::siwc::SiwcCredentials,
        _: CancellationToken,
    ) -> Result<huginn_core::siwc::SiwcCredentials> {
        self.refreshes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.input_refresh_tokens
            .lock()
            .unwrap()
            .push(credentials.refresh_token.clone());
        Ok(self.refreshed.clone())
    }

    async fn revoke_remote(
        &self,
        _: &huginn_core::siwc::SiwcCredentials,
        _: CancellationToken,
    ) -> huginn_core::siwc::SiwcRevocationOutcome {
        huginn_core::siwc::SiwcRevocationOutcome::RemoteConfirmed
    }
}

async fn verify_siwc_rotation_retry(write_before_error: bool) {
    use axum::{http::HeaderMap, routing::get};
    use huginn_core::{
        connections::{ConnectionMaterial, ConnectionRevision, ConnectionSettings, Secret},
        providers::{ConnectionIdentity, ModelSelection, ProviderAdapter, ProviderRegistration},
        siwc::{SiwcAuthPort, SiwcCredentialPersistence, SiwcCredentials, SIWC_PROVIDER_ID},
    };
    use huginn_runtime::siwc::{SiwcAdapter, SiwcEndpoints};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        OnceLock,
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let scopes: Vec<String> = huginn_core::siwc::SIWC_REQUIRED_SCOPES
        .iter()
        .map(|scope| (*scope).to_owned())
        .collect();
    let stale = SiwcCredentials {
        host_id: "host-1".into(),
        client_id: "client-1".into(),
        access_token: "access-r0".into(),
        refresh_token: "refresh-r0".into(),
        expires_at_unix: 0,
        granted_scopes: scopes.clone(),
        account_sub: Some("account-1".into()),
        account_email: Some("dev@example.com".into()),
    };
    let rotated = SiwcCredentials {
        access_token: "access-r1".into(),
        refresh_token: "refresh-r1".into(),
        expires_at_unix: now + 3600,
        ..stale.clone()
    };
    let persistence = Arc::new(FailOncePersistence {
        stored: tokio::sync::Mutex::new(Some(stale.clone())),
        writes: AtomicUsize::new(0),
        write_before_error,
    });
    let auth = Arc::new(RotatingSiwcAuth {
        refreshed: rotated.clone(),
        refreshes: AtomicUsize::new(0),
        input_refresh_tokens: Mutex::new(Vec::new()),
    });
    let persistence_slot: Arc<OnceLock<Arc<dyn SiwcCredentialPersistence>>> =
        Arc::new(OnceLock::new());
    let _ = persistence_slot.set(persistence.clone());
    let models_requests = Arc::new(AtomicUsize::new(0));
    let responses_requests = Arc::new(AtomicUsize::new(0));
    let models_requests_handler = models_requests.clone();
    let responses_requests_handler = responses_requests.clone();
    let persistence_handler = persistence.clone();
    let expected_rotated = rotated.clone();
    let (base, task) = spawn_loopback_mock_server(
        Router::new()
            .route(
                "/v1/models",
                get(move |headers: HeaderMap| {
                    let requests = models_requests_handler.clone();
                    let persistence = persistence_handler.clone();
                    let expected = expected_rotated.clone();
                    async move {
                        requests.fetch_add(1, Ordering::SeqCst);
                        assert_eq!(headers["authorization"], "Bearer access-r1");
                        assert_eq!(persistence.stored.lock().await.as_ref(), Some(&expected));
                        Json(json!({"models": [{"slug": "gpt-5-chatgpt", "visibility": "list"}]}))
                    }
                }),
            )
            .route(
                "/v1/responses",
                post(move || {
                    let requests = responses_requests_handler.clone();
                    async move {
                        requests.fetch_add(1, Ordering::SeqCst);
                        ([("content-type", "text/event-stream")], "data: [DONE]\n\n")
                    }
                }),
            ),
    )
    .await;
    let endpoints = SiwcEndpoints {
        authorize_url: format!("{base}/authorize"),
        token_url: format!("{base}/token"),
        oidc_discovery_url: format!("{base}/.well-known/openid-configuration"),
        api_base_url: format!("{base}/v1"),
        allow_loopback_http: true,
        open_browser: false,
    };
    let adapter = SiwcAdapter::with_endpoints(
        auth.clone() as Arc<dyn SiwcAuthPort>,
        Some(persistence_slot),
        endpoints,
        5,
    );
    let selection = ModelSelection {
        connection: ConnectionIdentity {
            id: "siwc-retry".into(),
            adapter: ProviderRegistration::new(SIWC_PROVIDER_ID.into(), 1),
            revision: 1,
        },
        model: "gpt-5-chatgpt".into(),
        reasoning_effort: None,
    };
    let origin = Some(ConnectionRevision {
        id: Default::default(),
        revision: Default::default(),
    });
    let material = |credentials: &SiwcCredentials| ConnectionMaterial {
        origin,
        settings: ConnectionSettings {
            provider: SIWC_PROVIDER_ID.into(),
            endpoint: Some(format!("{base}/v1")),
            model: "gpt-5-chatgpt".into(),
            requires_secret: true,
            allow_loopback_http: false,
            reasoning_effort: None,
        },
        secret: Some(Secret::new(credentials.to_secret_json().unwrap()).unwrap()),
    };

    let first_attempt = adapter
        .create_connection_model(&selection, material(&stale), CancellationToken::new())
        .await;
    let first_error = first_attempt
        .err()
        .expect("the first persistence error must fail model creation");
    assert!(first_error
        .to_string()
        .contains("fixture persistence failure"));
    assert_eq!(models_requests.load(Ordering::SeqCst), 0);
    assert_eq!(responses_requests.load(Ordering::SeqCst), 0);
    assert_eq!(auth.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(
        persistence.stored.lock().await.as_ref(),
        write_before_error.then_some(&rotated).or(Some(&stale))
    );

    adapter
        .create_connection_model(&selection, material(&stale), CancellationToken::new())
        .await
        .expect("retry should persist the pending rotation before discovery");

    assert_eq!(auth.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(
        auth.input_refresh_tokens.lock().unwrap().as_slice(),
        ["refresh-r0"]
    );
    assert_eq!(
        persistence.writes.load(Ordering::SeqCst),
        if write_before_error { 1 } else { 2 }
    );
    assert_eq!(persistence.stored.lock().await.as_ref(), Some(&rotated));
    assert_eq!(models_requests.load(Ordering::SeqCst), 1);
    assert_eq!(responses_requests.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn siwc_rotation_is_retained_and_retried_after_persistence_errors() {
    verify_siwc_rotation_retry(false).await;
    verify_siwc_rotation_retry(true).await;
}

#[tokio::test]
async fn siwc_full_vertical_slice_oauth_id_token_refresh_persistence_responses_and_revocation() {
    use axum::routing::get;
    use base64::Engine as _;
    use huginn_core::{
        connections::{ConnectionMaterial, ConnectionSettings, Secret},
        providers::{ModelSelection, ProviderAdapter, ProviderRegistration},
        siwc::{
            SiwcAuthPort, SiwcCredentialPersistence, SiwcRevocationOutcome, SIWC_INITIAL_CLIENT_ID,
            SIWC_PROVIDER_ID, SIWC_REQUESTED_SCOPES,
        },
    };
    use huginn_runtime::siwc::{
        await_loopback_callback, SiwcAdapter, SiwcEndpoints, SiwcOAuthClient,
    };
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::OnceLock;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::Mutex as AsyncMutex;

    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let keypair = Arc::new(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap());
    let pub_x =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(keypair.public_key().as_ref());

    let token_refreshes = Arc::new(AtomicUsize::new(0));
    let revoke_attempts = Arc::new(AtomicUsize::new(0));
    let responses_requests = Arc::new(AtomicUsize::new(0));
    let seen_responses_body = Arc::new(AsyncMutex::new(None::<serde_json::Value>));
    let expected_verifier = Arc::new(AsyncMutex::new(String::new()));
    let expected_nonce = Arc::new(AsyncMutex::new(String::new()));
    let expected_redirect_uri = Arc::new(AsyncMutex::new(String::new()));
    let base_slot = Arc::new(OnceLock::<String>::new());

    let token_refreshes_c = token_refreshes.clone();
    let revoke_attempts_c = revoke_attempts.clone();
    let responses_requests_c = responses_requests.clone();
    let seen_body_c = seen_responses_body.clone();
    let verifier_c = expected_verifier.clone();
    let nonce_c = expected_nonce.clone();
    let redirect_uri_c = expected_redirect_uri.clone();
    let keypair_c = keypair.clone();
    let base_for_oidc = base_slot.clone();
    let base_for_token = base_slot.clone();

    let (base, task) = spawn_loopback_mock_server(
        Router::new()
            .route(
                "/oauth/token",
                post(move |body: String| {
                    let refreshes = token_refreshes_c.clone();
                    let verifier_c = verifier_c.clone();
                    let nonce_c = nonce_c.clone();
                    let keypair_c = keypair_c.clone();
                    let base_for_token = base_for_token.clone();
                    async move {
                        let params: std::collections::HashMap<String, String> =
                            reqwest::Url::parse(&format!("http://127.0.0.1/?{body}"))
                                .unwrap()
                                .query_pairs()
                                .into_owned()
                                .collect();
                        let issuer = base_for_token.get().unwrap().clone();
                        assert_eq!(
                            params.get("resource").map(String::as_str),
                            Some(format!("{issuer}/v1").as_str())
                        );
                        if params.get("grant_type").map(String::as_str)
                            == Some("refresh_token")
                        {
                            refreshes.fetch_add(1, Ordering::SeqCst);
                            Json(json!({
                                "access_token": test_token("acc", "rotated"),
                                "refresh_token": test_token("ref", "rotated"),
                                "client_id": "issued-client-1",
                                "expires_in": 3600,
                                "scope": SIWC_REQUESTED_SCOPES,
                            }))
                        } else {
                            assert_eq!(
                                params.get("grant_type").map(String::as_str),
                                Some("authorization_code")
                            );
                            assert_eq!(
                                params.get("client_id").map(String::as_str),
                                Some("issued-client-1")
                            );
                            assert_eq!(
                                params.get("code").map(String::as_str),
                                Some("auth-code-1")
                            );
                            assert_eq!(
                                params.get("code_verifier").map(String::as_str),
                                Some(verifier_c.lock().await.as_str())
                            );
                            let expected_redirect_uri = redirect_uri_c.lock().await.clone();
                            assert_eq!(
                                params.get("redirect_uri").map(String::as_str),
                                Some(expected_redirect_uri.as_str())
                            );
                            let nonce = nonce_c.lock().await.clone();
                            let id_token = sign_test_id_token(
                                &keypair_c,
                                &issuer,
                                "issued-client-1",
                                &nonce,
                                "chatgpt-sub-42",
                            );
                            Json(json!({
                                "access_token": test_token("acc", "initial"),
                                "refresh_token": test_token("ref", "initial"),
                                "id_token": id_token,
                                "client_id": "issued-client-1",
                                "expires_in": 10,
                                "scope": SIWC_REQUESTED_SCOPES,
                            }))
                        }
                    }
                }),
            )
            .route(
                "/.well-known/openid-configuration",
                get(move || {
                    let base_for_oidc = base_for_oidc.clone();
                    async move {
                        let issuer = base_for_oidc.get().unwrap().clone();
                        Json(json!({
                            "issuer": issuer,
                            "jwks_uri": format!("{issuer}/.well-known/jwks.json"),
                            "revocation_endpoint": format!("{issuer}/oauth/revoke"),
                        }))
                    }
                }),
            )
            .route(
                "/.well-known/jwks.json",
                get(move || {
                    let pub_x = pub_x.clone();
                    async move {
                        Json(json!({
                            "keys": [{
                                "kty": "OKP",
                                "crv": "Ed25519",
                                "kid": "test-k1",
                                "x": pub_x,
                            }]
                        }))
                    }
                }),
            )
            .route(
                "/oauth/revoke",
                post(move || {
                    let attempts = revoke_attempts_c.clone();
                    async move {
                        let n = attempts.fetch_add(1, Ordering::SeqCst);
                        if n == 0 {
                            StatusCode::SERVICE_UNAVAILABLE
                        } else {
                            StatusCode::OK
                        }
                    }
                }),
            )
            .route(
                "/v1/models",
                get(|| async {
                    Json(json!({
                        "models": [
                            {"slug": "gpt-5-chatgpt", "visibility": "list"},
                            {"slug": "hidden-model", "visibility": "hide"}
                        ]
                    }))
                }),
            )
            .route(
                "/v1/responses",
                post(move |Json(body): Json<serde_json::Value>| {
                    let requests = responses_requests_c.clone();
                    let seen_body = seen_body_c.clone();
                    async move {
                        requests.fetch_add(1, Ordering::SeqCst);
                        *seen_body.lock().await = Some(body);
                        let sse = concat!(
                            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"siwc \"}\n\n",
                            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n",
                            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":5,\"output_tokens\":2}}}\n\n"
                        );
                        ([("content-type", "text/event-stream")], sse)
                    }
                }),
            ),
    )
    .await;
    base_slot.set(base.clone()).unwrap();

    let endpoints = SiwcEndpoints {
        authorize_url: format!("{base}/authorize"),
        token_url: format!("{base}/oauth/token"),
        oidc_discovery_url: format!("{base}/.well-known/openid-configuration"),
        api_base_url: format!("{base}/v1"),
        allow_loopback_http: true,
        open_browser: false,
    };
    let oauth = Arc::new(SiwcOAuthClient::with_endpoints(endpoints.clone(), 5));

    // 1. PKCE + documented authorize URL parameters.
    let (verifier, challenge) = SiwcOAuthClient::generate_pkce();
    let dynamic_nonce = format!("nonce-{}", uuid::Uuid::new_v4());
    let callback_redirect_uri = "http://127.0.0.1:4321/callback";
    *expected_verifier.lock().await = verifier.clone();
    *expected_nonce.lock().await = dynamic_nonce.clone();
    *expected_redirect_uri.lock().await = callback_redirect_uri.to_owned();
    let auth_url = oauth
        .build_authorize_url(
            callback_redirect_uri,
            "state-1",
            &dynamic_nonce,
            &challenge,
            "host-1",
            SIWC_INITIAL_CLIENT_ID,
        )
        .unwrap();
    assert!(auth_url.contains("code_challenge_method=S256"));
    assert!(auth_url.contains("agent_name_hint=Huginn"));
    assert!(auth_url.contains("ext_agent_host_id=host-1"));
    assert!(auth_url.contains(&format!("nonce={dynamic_nonce}")));
    assert!(auth_url.contains("resource="));

    // 2. Loopback callback extracts issued client_id + state mismatch rejected.
    let cb_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cb_addr = cb_listener.local_addr().unwrap();
    let sender = tokio::spawn(async move {
        let mut s = tokio::net::TcpStream::connect(cb_addr).await.unwrap();
        s.write_all(
            b"GET /callback?code=auth-code-1&state=expected-state&client_id=issued-client-1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        )
        .await
        .unwrap();
    });
    let cb = await_loopback_callback(cb_listener, "expected-state", CancellationToken::new())
        .await
        .unwrap();
    sender.await.unwrap();
    assert_eq!(cb.code, "auth-code-1");
    assert_eq!(cb.client_id.as_deref(), Some("issued-client-1"));

    // 3. Exchange code verifies ID token signature, iss, aud, exp, nonce, and sub.
    let creds = oauth
        .exchange_code(
            &cb.code,
            callback_redirect_uri,
            &verifier,
            "host-1",
            cb.client_id.as_deref().unwrap(),
            &dynamic_nonce,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(creds.client_id, "issued-client-1");
    assert_eq!(creds.account_sub.as_deref(), Some("chatgpt-sub-42"));
    assert_eq!(creds.account_email.as_deref(), Some("dev@example.com"));

    // 4. Adapter single-flight refresh persists rotated credentials before use and does not double-refresh.
    let persistence_impl = Arc::new(InMemoryPersistence {
        stored: AsyncMutex::new(Some(creds.clone())),
        writes: AtomicUsize::new(0),
    });
    let persistence_slot = Arc::new(OnceLock::new());
    let _ = persistence_slot.set(persistence_impl.clone() as Arc<dyn SiwcCredentialPersistence>);
    let adapter = SiwcAdapter::with_endpoints(oauth.clone(), Some(persistence_slot), endpoints, 5);
    let material = ConnectionMaterial {
        origin: Some(huginn_core::connections::ConnectionRevision {
            id: Default::default(),
            revision: Default::default(),
        }),
        settings: ConnectionSettings {
            provider: SIWC_PROVIDER_ID.into(),
            endpoint: Some(format!("{base}/v1")),
            model: "gpt-5-chatgpt".into(),
            requires_secret: true,
            allow_loopback_http: false,
            reasoning_effort: None,
        },
        secret: Some(Secret::new(creds.to_secret_json().unwrap()).unwrap()),
    };
    let selection = ModelSelection {
        connection: huginn_core::providers::ConnectionIdentity {
            id: "siwc-1".into(),
            adapter: ProviderRegistration::new(SIWC_PROVIDER_ID.into(), 1),
            revision: 1,
        },
        model: "gpt-5-chatgpt".into(),
        reasoning_effort: None,
    };
    let provider = adapter
        .create_connection_model(&selection, material, CancellationToken::new())
        .await
        .unwrap();
    // Exactly ONE refresh occurred (shared between resolve + catalog validation) and it was persisted.
    assert_eq!(token_refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(persistence_impl.writes.load(Ordering::SeqCst), 1);
    let persisted = persistence_impl.stored.lock().await.clone().unwrap();
    assert_eq!(persisted.refresh_token, test_token("ref", "rotated"));

    // 5. Text-only prompt succeeds even when workspace tools are registered, and forwards max_output_tokens.
    let deltas: Vec<_> = provider
        .stream(
            ModelRequest {
                messages: vec![huginn_core::Message::text(huginn_core::Role::User, "hi")],
                tools: vec![huginn_core::ToolDefinition {
                    name: "read_file".into(),
                    description: "read".into(),
                    input_schema: json!({}),
                }],
                max_tokens: 256,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .collect()
        .await;
    assert!(deltas.iter().all(|delta| delta.is_ok()), "{deltas:?}");
    assert!(
        matches!(deltas.last(), Some(Ok(ModelDelta::Finished))),
        "Responses stream must finish successfully: {deltas:?}"
    );
    let text: String = deltas
        .iter()
        .filter_map(|d| match d {
            Ok(ModelDelta::Text(t)) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "siwc ok");
    let sent = seen_responses_body.lock().await.clone().unwrap();
    assert_eq!(sent["store"], false);
    assert_eq!(sent["stream"], true);
    assert_eq!(sent["max_output_tokens"], 256);
    assert!(sent.get("tools").is_none());
    assert_eq!(responses_requests.load(Ordering::SeqCst), 1);

    // 6. Image attachments are still rejected.
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&4u32.to_be_bytes());
    png.extend_from_slice(&4u32.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0]);
    let img = huginn_core::ImageAttachment::from_bytes(png, None).unwrap();
    let image_error = provider
        .stream(
            ModelRequest {
                messages: vec![huginn_core::Message::user_with_images("see", vec![img])],
                tools: vec![],
                max_tokens: 64,
            },
            CancellationToken::new(),
        )
        .await
        .err()
        .expect("SIWC must reject image attachments");
    assert!(
        image_error
            .to_string()
            .contains("image attachments are not enabled yet"),
        "unexpected image rejection: {image_error}"
    );
    assert_eq!(
        responses_requests.load(Ordering::SeqCst),
        1,
        "image rejection must happen before the Responses request"
    );

    // 7. Remote revocation retries transient 503 and succeeds on the second attempt.
    let outcome = oauth
        .revoke_remote(&persisted, CancellationToken::new())
        .await;
    assert_eq!(outcome, SiwcRevocationOutcome::RemoteConfirmed);
    assert_eq!(revoke_attempts.load(Ordering::SeqCst), 2);

    // 8. Durable host ID is stable across sign-in attempts and client reloads.
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_path = temp_dir.path().join("siwc_host_id");
    let client1 = SiwcOAuthClient::with_storage(5, Some(storage_path.clone()));
    let host_id1 = client1.get_or_create_host_id().unwrap();
    assert!(host_id1.starts_with("huginn-"));
    assert_eq!(client1.get_or_create_host_id().unwrap(), host_id1);
    let client2 = SiwcOAuthClient::with_storage(5, Some(storage_path));
    assert_eq!(client2.get_or_create_host_id().unwrap(), host_id1);

    task.abort();
}

#[test]
fn siwc_sse_parser_delivers_text_before_failed_rejects_done_without_completed_and_caps_frame_size()
{
    use huginn_runtime::siwc::{drain_responses_sse, MAX_SSE_FRAME_BYTES};

    // Text delta + response.failed in one chunk delivers the text delta first, then error.
    let mut buf = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"prefix\"}\n\n",
        "data: {\"type\":\"response.failed\"}\n\n"
    )
    .as_bytes()
    .to_vec();
    let mut completed = false;
    let out = drain_responses_sse(&mut buf, &mut completed);
    assert!(out.ended);
    assert!(!completed);
    assert_eq!(out.items.len(), 2);
    assert!(matches!(&out.items[0], Ok(ModelDelta::Text(t)) if t == "prefix"));
    assert!(out.items[1].is_err());

    // [DONE] without response.completed is treated as an incomplete error.
    let mut done_buf = b"data: [DONE]\n\n".to_vec();
    let mut done_completed = false;
    let done_out = drain_responses_sse(&mut done_buf, &mut done_completed);
    assert!(done_out.ended);
    assert!(matches!(done_out.items.as_slice(), [Err(_)]));

    // Oversized pending SSE frame without delimiter fails cleanly.
    let mut huge = vec![b'a'; MAX_SSE_FRAME_BYTES + 1];
    let mut huge_completed = false;
    let huge_out = drain_responses_sse(&mut huge, &mut huge_completed);
    assert!(huge_out.ended);
    assert!(matches!(huge_out.items.as_slice(), [Err(_)]));
}
