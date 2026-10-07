use agent_core::{
    AgentError, AgentLoop, ApprovalPolicy, ApprovalRequest, CancellationToken, Event, EventSink,
    Message, ModelDelta, ModelProvider, ModelRequest, Result, Role, Session, SessionStore,
    ToolCall, ToolContext, ToolRegistry,
};
use agent_runtime::{
    loop_engine::{recover, LoopConfig, StandardLoop},
    model::{MockProvider, SseParser},
    sessions::SqliteSessions,
    tools::{resolve_path, Registry},
};
use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

struct Policy(bool);
#[async_trait]
impl ApprovalPolicy for Policy {
    async fn approve(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
        Ok(self.0)
    }
}
#[derive(Default)]
struct Events(Mutex<Vec<Event>>);
impl EventSink for Events {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}
struct Script {
    replies: Mutex<VecDeque<Vec<ModelDelta>>>,
    requests: Mutex<Vec<ModelRequest>>,
}
#[async_trait]
impl ModelProvider for Script {
    async fn stream(
        &self,
        request: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        self.requests.lock().unwrap().push(request);
        let output = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AgentError::Model("script exhausted".into()))?;
        Ok(futures::stream::iter(output.into_iter().map(Ok)).boxed())
    }
}
fn call(name: &str, args: serde_json::Value) -> ModelDelta {
    ModelDelta::ToolCall(ToolCall {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        arguments: args,
    })
}
fn fixture(
    model: Arc<dyn ModelProvider>,
    db: &std::path::Path,
) -> (StandardLoop, Arc<SqliteSessions>) {
    let sessions = Arc::new(SqliteSessions::open(db).unwrap());
    (
        StandardLoop {
            model,
            tools: Arc::new(Registry::native(2).unwrap()),
            sessions: sessions.clone(),
            config: LoopConfig::default(),
            lifecycle: CancellationToken::new(),
        },
        sessions,
    )
}

#[tokio::test]
async fn approved_edit_and_command_complete_then_session_resumes_without_replay() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("lib.txt"), "before\n").unwrap();
    let model = Arc::new(Script {
        replies: Mutex::new(VecDeque::from([
            vec![
                call(
                    "edit_file",
                    json!({"path":"lib.txt","old_text":"before","new_text":"after"}),
                ),
                ModelDelta::Finished,
            ],
            vec![
                call(
                    "shell",
                    json!({"command":"test \"$(cat lib.txt)\" = after && printf 'tests passed'"}),
                ),
                ModelDelta::Finished,
            ],
            vec![
                ModelDelta::Text("Changed and tested.".into()),
                ModelDelta::Finished,
            ],
        ])),
        requests: Mutex::new(vec![]),
    });
    let (engine, sessions) = fixture(model.clone(), &data.path().join("sessions.db"));
    let mut session = Session::new(workspace.path().canonicalize().unwrap());
    let events = Events::default();
    engine
        .run(
            &mut session,
            "change before to after and test it".into(),
            &Policy(true),
            &events,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("lib.txt")).unwrap(),
        "after\n"
    );
    assert!(!session.interrupted);
    assert!(session
        .messages
        .iter()
        .any(|m| m.role == Role::Tool && m.content.contains("tests passed")));
    assert!(events
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, Event::ApprovalResolved { approved: true, .. })));
    drop(engine);
    drop(sessions);
    let reopened = SqliteSessions::open(&data.path().join("sessions.db")).unwrap();
    let resumed = reopened.load(&session.id).await.unwrap();
    assert_eq!(resumed.messages.len(), session.messages.len());
    assert!(!resumed.interrupted);
    assert!(model.requests.lock().unwrap()[1]
        .messages
        .iter()
        .any(|m| m.role == Role::Tool));
}

#[tokio::test]
async fn denied_edit_does_not_modify_the_file() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("file"), "original").unwrap();
    let model = Arc::new(Script {
        replies: Mutex::new(VecDeque::from([
            vec![
                call("write_file", json!({"path":"file","content":"changed"})),
                ModelDelta::Finished,
            ],
            vec![ModelDelta::Text("Denied.".into()), ModelDelta::Finished],
        ])),
        requests: Mutex::new(vec![]),
    });
    let (engine, _) = fixture(model, &data.path().join("sessions.db"));
    let mut session = Session::new(workspace.path().canonicalize().unwrap());
    engine
        .run(
            &mut session,
            "edit".into(),
            &Policy(false),
            &Events::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("file")).unwrap(),
        "original"
    );
    assert!(session
        .messages
        .iter()
        .any(|m| m.role == Role::Tool && m.content.contains("denied")));
}

#[tokio::test]
async fn stale_diff_and_symlink_escape_are_rejected() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("file"), "before").unwrap();
    let registry = Registry::native(2).unwrap();
    let tool = registry.get("write_file").unwrap();
    let ctx = ToolContext {
        workspace: workspace.path().canonicalize().unwrap(),
        cancellation: CancellationToken::new(),
    };
    let action = tool
        .prepare(
            &ToolCall {
                id: "edit".into(),
                name: "write_file".into(),
                arguments: json!({"path":"file","content":"after"}),
            },
            &ctx,
        )
        .await
        .unwrap();
    assert!(action
        .approval
        .as_ref()
        .unwrap()
        .preview
        .contains("-before"));
    std::fs::write(workspace.path().join("file"), "user edit").unwrap();
    assert!(tool
        .execute(action, &ctx)
        .await
        .unwrap_err()
        .to_string()
        .contains("changed after approval"));
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("file")).unwrap(),
        "user edit"
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), workspace.path().join("escape")).unwrap();
        assert!(resolve_path(&ctx.workspace, "escape/new", true).is_err());
        std::os::unix::fs::symlink(
            outside.path().join("missing"),
            workspace.path().join("dangling"),
        )
        .unwrap();
        assert!(resolve_path(&ctx.workspace, "dangling", true).is_err());
    }
}

#[test]
fn sse_assembles_fragmented_arguments_and_utf8_and_rejects_truncation() {
    let frames = [
        json!({"choices":[{"delta":{"content":"Hola 🧠"},"finish_reason":null}]}),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"read_file","arguments":"{\"pa"}}]},"finish_reason":null}]}),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"x\"}"}}]},"finish_reason":"tool_calls"}]}),
    ];
    let source = frames
        .iter()
        .map(|f| format!("data: {f}\r\n\r\n"))
        .collect::<String>()
        + "data: [DONE]\r\n\r\n";
    let mut parser = SseParser::default();
    let mut deltas = vec![];
    for byte in source.bytes() {
        deltas.extend(parser.push(&[byte]).unwrap());
    }
    assert!(deltas
        .iter()
        .any(|d| matches!(d,ModelDelta::Text(t) if t=="Hola 🧠")));
    assert!(deltas
        .iter()
        .any(|d| matches!(d,ModelDelta::ToolCall(c) if c.arguments==json!({"path":"x"}))));
    assert!(matches!(deltas.last(), Some(ModelDelta::Finished)));
    let mut incomplete = SseParser::default();
    incomplete.push(b"data: {\"choices\":[]}").unwrap();
    assert!(incomplete.eof().is_err());
    let mut malformed = SseParser::default();
    assert!(malformed.push(b"data: invalid\n\n").is_err());
}

#[tokio::test]
async fn session_lock_and_recovery_prevent_concurrent_or_replayed_effects() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteSessions::open(&dir.path().join("sessions.db")).unwrap();
    let mut session = Session::new(dir.path().canonicalize().unwrap());
    session.interrupted = true;
    session.messages.push(Message {
        role: Role::Assistant,
        content: String::new(),
        tool_call_id: None,
        tool_calls: vec![ToolCall {
            id: "pending".into(),
            name: "shell".into(),
            arguments: json!({"command":"echo unsafe"}),
        }],
    });
    store.save(&session).await.unwrap();
    let lease = store.lease(&session.id).unwrap();
    assert!(store.lease(&session.id).is_err());
    drop(lease);
    assert!(store.lease(&session.id).is_ok());
    recover(&mut session);
    assert_eq!(session.messages.len(), 2);
    assert!(session.messages[1]
        .content
        .contains("completion is unknown"));
    recover(&mut session);
    assert_eq!(session.messages.len(), 2);
}

#[tokio::test]
async fn compaction_preserves_history_and_tool_pairs() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let model = Arc::new(Script {
        replies: Mutex::new(VecDeque::from([
            vec![
                ModelDelta::Text("Earlier work summarized.".into()),
                ModelDelta::Finished,
            ],
            vec![ModelDelta::Text("Continuing.".into()), ModelDelta::Finished],
        ])),
        requests: Mutex::new(vec![]),
    });
    let (mut engine, _) = fixture(model.clone(), &data.path().join("sessions.db"));
    engine.config.context_tokens = 16000;
    engine.config.max_output_tokens = 512;
    let mut session = Session::new(workspace.path().canonicalize().unwrap());
    for index in 0..6 {
        session.messages.push(Message::text(
            Role::User,
            format!("{index}: {}", "x".repeat(1600)),
        ));
        let mut assistant = Message::text(Role::Assistant, "previous answer");
        assistant.tool_calls.push(ToolCall {
            id: format!("old-{index}"),
            name: "read_file".into(),
            arguments: json!({"path":"file"}),
        });
        session.messages.push(assistant);
        session
            .messages
            .push(Message::tool(format!("old-{index}"), "read result".into()));
    }
    let original = session.messages.len();
    engine
        .run(
            &mut session,
            "next".into(),
            &Policy(false),
            &Events::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(session.summary.is_some());
    assert!(session.summary_through > 0);
    assert_eq!(session.messages.len(), original + 2);
    assert!(session.messages[0].content.contains("0:"));
    let requests = model.requests.lock().unwrap();
    let context = requests.last().unwrap().messages.clone();
    drop(requests);
    for message in &context {
        for call in &message.tool_calls {
            assert!(context
                .iter()
                .any(|m| m.tool_call_id.as_ref() == Some(&call.id)));
        }
    }
}

#[tokio::test]
async fn cancellation_keeps_an_interrupted_record_and_does_not_execute_tools() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (engine, store) = fixture(Arc::new(MockProvider), &data.path().join("sessions.db"));
    let mut session = Session::new(workspace.path().canonicalize().unwrap());
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        engine
            .run(
                &mut session,
                "next".into(),
                &Policy(false),
                &Events::default(),
                cancellation
            )
            .await,
        Err(AgentError::Cancelled)
    ));
    assert!(store.load(&session.id).await.unwrap().interrupted);
}

#[test]
fn root_and_nested_instructions_are_scoped_and_symlinks_are_contained() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join("src")).unwrap();
    std::fs::write(workspace.path().join("AGENTS.md"), "root instruction").unwrap();
    std::fs::write(workspace.path().join("nested.md"), "nested instruction").unwrap();
    std::os::unix::fs::symlink("../nested.md", workspace.path().join("src/AGENTS.md")).unwrap();
    let text = agent_runtime::instructions::load(workspace.path()).unwrap();
    assert!(text.contains("root instruction"));
    assert!(text.contains("Instructions for src:"));
    assert!(text.contains("nested instruction"));
    assert!(text.contains("cannot grant permissions"));
    std::fs::remove_file(workspace.path().join("src/AGENTS.md")).unwrap();
    std::fs::write(outside.path().join("AGENTS.md"), "outside").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("AGENTS.md"),
        workspace.path().join("src/AGENTS.md"),
    )
    .unwrap();
    assert!(agent_runtime::instructions::load(workspace.path()).is_err());
}

#[tokio::test]
async fn failed_compaction_preserves_original_history_and_stops() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let model = Arc::new(Script {
        replies: Mutex::new(VecDeque::from([vec![ModelDelta::Finished]])),
        requests: Mutex::new(vec![]),
    });
    let (mut engine, store) = fixture(model, &data.path().join("sessions.db"));
    engine.config.context_tokens = 14_000;
    engine.config.max_output_tokens = 512;
    let mut session = Session::new(workspace.path().canonicalize().unwrap());
    for _ in 0..6 {
        session
            .messages
            .push(Message::text(Role::User, "x".repeat(1600)));
        session
            .messages
            .push(Message::text(Role::Assistant, "answer"));
    }
    assert!(engine
        .run(
            &mut session,
            "continue".into(),
            &Policy(true),
            &Events::default(),
            CancellationToken::new()
        )
        .await
        .is_err());
    assert!(session.summary.is_none());
    assert_eq!(session.summary_through, 0);
    assert_eq!(session.messages.len(), 13);
    assert!(store.load(&session.id).await.unwrap().interrupted);
}

#[tokio::test]
async fn shell_timeout_kills_descendants_before_they_can_write() {
    let workspace = tempfile::tempdir().unwrap();
    let registry = Registry::native(1).unwrap();
    let shell = registry.get("shell").unwrap();
    let ctx = ToolContext {
        workspace: workspace.path().canonicalize().unwrap(),
        cancellation: CancellationToken::new(),
    };
    let prepared = shell
        .prepare(
            &ToolCall {
                id: "shell".into(),
                name: "shell".into(),
                arguments: json!({"command":"(sleep 2; printf leaked > escaped) & wait"}),
            },
            &ctx,
        )
        .await
        .unwrap();
    assert!(shell.execute(prepared, &ctx).await.is_err());
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert!(!workspace.path().join("escaped").exists());
}

#[tokio::test]
async fn interruption_during_approved_shell_records_unknown_completion_without_replay() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let model = Arc::new(Script {
        replies: Mutex::new(VecDeque::from([vec![
            call(
                "shell",
                json!({"command":"printf started > marker; sleep 10; printf repeated >> marker"}),
            ),
            ModelDelta::Finished,
        ]])),
        requests: Mutex::new(vec![]),
    });
    let (engine, store) = fixture(model, &data.path().join("sessions.db"));
    let mut session = Session::new(workspace.path().canonicalize().unwrap());
    let token = CancellationToken::new();
    let trigger = token.clone();
    let marker = workspace.path().join("marker");
    let check = marker.clone();
    let cancel_task = tokio::spawn(async move {
        for _ in 0..200 {
            if check.exists() {
                trigger.cancel();
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("shell never started");
    });
    assert!(matches!(
        engine
            .run(
                &mut session,
                "run command".into(),
                &Policy(true),
                &Events::default(),
                token
            )
            .await,
        Err(AgentError::Cancelled)
    ));
    cancel_task.await.unwrap();
    let mut restored = store.load(&session.id).await.unwrap();
    assert!(restored.interrupted);
    recover(&mut restored);
    assert!(restored
        .messages
        .iter()
        .any(|m| m.role == Role::Tool && m.content.contains("unknown")));
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "started");
}

#[tokio::test]
async fn denied_mcp_call_never_reaches_the_started_server() {
    use agent_runtime::mcp::{McpClients, ServerConfig};
    use std::collections::BTreeMap;
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let marker = workspace.path().join("marker");
    let registry = Arc::new(Registry::native(1).unwrap());
    let config = ServerConfig {
        name: "fixture".into(),
        command: "/usr/bin/python3".into(),
        args: vec![format!(
            "{}/tests/fixtures/mcp_server.py",
            env!("CARGO_MANIFEST_DIR")
        )],
        env: BTreeMap::from([("MARKER".into(), marker.display().to_string())]),
        env_from: BTreeMap::new(),
    };
    let mut clients = McpClients::connect(
        &[config],
        workspace.path(),
        &registry,
        &Policy(true),
        CancellationToken::new(),
        5,
    )
    .await
    .unwrap();
    let model = Arc::new(Script {
        replies: Mutex::new(VecDeque::from([
            vec![call("mcp_fixture_0", json!({})), ModelDelta::Finished],
            vec![ModelDelta::Text("Denied".into()), ModelDelta::Finished],
        ])),
        requests: Mutex::new(vec![]),
    });
    let (mut engine, _) = fixture(model, &data.path().join("sessions.db"));
    engine.tools = registry;
    let mut session = Session::new(workspace.path().canonicalize().unwrap());
    engine
        .run(
            &mut session,
            "call MCP".into(),
            &Policy(false),
            &Events::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "started\n");
    clients.shutdown().await.unwrap();
}

#[test]
fn traversal_skips_unix_sockets_and_sse_accepts_mixed_line_endings() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("file"), "content").unwrap();
    let _socket = std::os::unix::net::UnixListener::bind(workspace.path().join("socket")).unwrap();
    let files = agent_runtime::tools::workspace_files(workspace.path(), workspace.path()).unwrap();
    assert_eq!(files, [workspace.path().join("file")]);
    let mut parser = SseParser::default();
    let deltas = parser.push(b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\r\n\r\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n").unwrap();
    assert!(matches!(deltas.first(), Some(ModelDelta::Text(text)) if text == "hello"));
    assert!(matches!(deltas.last(), Some(ModelDelta::Finished)));
}
