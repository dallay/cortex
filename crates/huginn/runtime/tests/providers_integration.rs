//! Composed loop/manager/HTTP adapter tests; no native credential backend or live service.
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use huginn_application::{
    composition::model_id,
    connections::{ConnectionAccess, ConnectionManager},
    providers::{catalog_id, providers_id, ProvidersPlugin},
};
use huginn_core::{
    connections::*, kernel::Supervisor, providers::*, AgentError, AgentLoop, ApprovalPolicy,
    ApprovalRequest, CancellationToken, Event, EventSink, Message, ModelProvider, ModelService,
    PreparedAction, Result, Role, Session, Tool, ToolCall, ToolContext, ToolDefinition,
    ToolRegistry,
};
use huginn_runtime::{
    connections::{EphemeralSecrets, SqliteConnections},
    loop_engine::{LoopConfig, StandardLoop},
    providers::BuiltinInferencePlugin,
    sessions::SqliteSessions,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Notify;

const SECRETS: [&str; 3] = [
    "loop-sentinel-a-674",
    "loop-sentinel-b-674",
    "loop-sentinel-rotated-674",
];

#[derive(Clone, Default)]
struct Gate {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}
impl Gate {
    async fn stop(&self) {
        self.entered.notify_one();
        wait(&self.release).await;
    }
}
async fn wait(signal: &Notify) {
    tokio::time::timeout(Duration::from_secs(5), signal.notified())
        .await
        .expect("fixture barrier timed out");
}

enum Reply {
    Tool,
    Text(&'static str),
    Summary(Gate),
    Http(StatusCode),
    StreamError,
    GatedStream(Gate),
    // The first text is delivered, but the HTTP body never completes normally.
    Stalled,
}
#[derive(Clone, Debug)]
struct Request {
    path: &'static str,
    auth: String,
    body: Value,
}
struct Endpoint {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    metadata: Arc<Mutex<Vec<Request>>>,
    task: tokio::task::JoinHandle<()>,
}
fn metadata_route(
    path: &'static str,
    response: Value,
    captured: Arc<Mutex<Vec<Request>>>,
    is_post: bool,
) -> axum::routing::MethodRouter {
    let handler = move |headers: HeaderMap, body: bytes::Bytes| {
        let captured = captured.clone();
        let response = response.clone();
        async move {
            captured.lock().unwrap().push(Request {
                path,
                auth: headers
                    .get("authorization")
                    .map(|header| header.to_str().unwrap())
                    .unwrap_or("")
                    .into(),
                body: if body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&body).expect("metadata body must be JSON")
                },
            });
            Json(response)
        }
    };
    if is_post {
        post(handler)
    } else {
        get(handler)
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn sse(delta: Value, finish: &str) -> String {
    format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"delta":delta,"finish_reason":finish}]})
    )
}
async fn endpoint(replies: Vec<Reply>) -> Endpoint {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let metadata = Arc::new(Mutex::new(Vec::new()));
    let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
    let router = Router::new()
        .route("/v1/models", metadata_route("/v1/models", json!({"data":[{"id":"alpha"},{"id":"beta"}]}), metadata.clone(), false))
        .route("/api/tags", metadata_route("/api/tags", json!({"models":[{"name":"alpha"},{"name":"beta"}]}), metadata.clone(), false))
        .route("/api/show", metadata_route("/api/show", json!({"capabilities":["completion","tools"]}), metadata.clone(), true))
        .route("/v1/chat/completions", post({
            let requests = requests.clone();
            move |headers: HeaderMap, Json(body): Json<Value>| {
                let requests = requests.clone();
                let queue = queue.clone();
                async move {
                    requests.lock().unwrap().push(Request {
                        path: "/v1/chat/completions",
                        auth: headers.get("authorization").map(|h| h.to_str().unwrap()).unwrap_or("").into(),
                        body,
                    });
                    let reply = queue.lock().unwrap().pop_front().expect("unexpected inference request");
                    let data = match reply {
                        Reply::Tool => sse(json!({"tool_calls":[{"index":0,"id":"switch-call","type":"function","function":{"name":"switch","arguments":"{}"}}]}), "tool_calls"),
                        Reply::Text(text) => sse(json!({"content":text}), "stop"),
                        Reply::Summary(gate) => { gate.stop().await; sse(json!({"content":"short repository summary"}), "stop") },
                        Reply::Http(status) => return (status, Body::from(SECRETS.join(" "))),
                        Reply::StreamError => format!("data: {}\n\n", json!({"error":{"message":SECRETS.join(" ")}})),
                        Reply::GatedStream(gate) => {
                            let first = futures::stream::once(async { Ok::<_, std::convert::Infallible>(bytes::Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"content\":\"started\"}}]}\n\n")) });
                            let rest = futures::stream::once(async move {
                                gate.stop().await;
                                Ok::<_, std::convert::Infallible>(bytes::Bytes::from(sse(json!({"content":" finished"}), "stop")))
                            });
                            return (StatusCode::OK, Body::from_stream(futures::StreamExt::chain(first, rest)));
                        }
                        Reply::Stalled => {
                            let first = futures::stream::once(async { Ok::<_, std::convert::Infallible>(bytes::Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"content\":\"started\"}}]}\n\n")) });
                            return (StatusCode::OK, Body::from_stream(futures::StreamExt::chain(first, futures::stream::pending())));
                        }
                    };
                    (StatusCode::OK, Body::from(data))
                }
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Endpoint {
        url,
        requests,
        metadata,
        task,
    }
}

struct Fixture {
    supervisor: Supervisor,
    manager: Arc<ConnectionManager>,
    model: Arc<dyn ModelProvider>,
    records: [ConnectionRecord; 2],
    endpoints: [Endpoint; 2],
    directory: tempfile::TempDir,
}
impl Fixture {
    async fn new(provider: &str, a: Vec<Reply>, b: Vec<Reply>) -> Self {
        let endpoints = [endpoint(a).await, endpoint(b).await];
        let directory = tempfile::tempdir().unwrap();
        let access = ConnectionAccess::new(Arc::new(EphemeralSecrets::default()));
        let mut supervisor = Supervisor::default();
        supervisor
            .register(Box::new(ProvidersPlugin::with_credentials(access.clone())))
            .unwrap();
        supervisor
            .register(Box::new(BuiltinInferencePlugin { timeout_secs: 10 }))
            .unwrap();
        supervisor.start().await.unwrap();
        let manager = Arc::new(
            ConnectionManager::open(
                Arc::new(
                    SqliteConnections::open(&directory.path().join("connections.db")).unwrap(),
                ),
                access,
                supervisor
                    .resolve::<ProvidersService>(&providers_id())
                    .unwrap()
                    .0
                    .clone(),
                supervisor
                    .resolve::<ProviderCatalogService>(&catalog_id())
                    .unwrap()
                    .0
                    .clone(),
            )
            .await
            .unwrap(),
        );
        let mut records = Vec::new();
        for (index, endpoint) in endpoints.iter().enumerate() {
            records.push(
                manager
                    .create(
                        ConnectionSettings {
                            provider: provider.into(),
                            endpoint: Some(if provider == "openai" {
                                format!("{}/v1", endpoint.url)
                            } else {
                                endpoint.url.clone()
                            }),
                            model: if index == 0 { "alpha" } else { "beta" }.into(),
                            requires_secret: true,
                            allow_loopback_http: true,
                        },
                        Some(Secret::new(SECRETS[index].into()).unwrap()),
                    )
                    .await
                    .unwrap(),
            );
        }
        manager.select(records[0].id).await.unwrap();
        let model = supervisor
            .resolve::<ModelService>(&model_id())
            .unwrap()
            .0
            .clone();
        Self {
            supervisor,
            manager,
            model,
            records: records.try_into().unwrap(),
            endpoints,
            directory,
        }
    }
    fn engine(&self, gate: Option<Gate>) -> StandardLoop {
        StandardLoop {
            model: self.model.clone(),
            tools: Arc::new(Tools(gate.map(|g| Arc::new(Switch(g)) as Arc<dyn Tool>))),
            sessions: Arc::new(
                SqliteSessions::open(&self.directory.path().join("sessions.db")).unwrap(),
            ),
            instructions: Arc::new(huginn_runtime::instructions::FilesystemInstructions),
            config: LoopConfig::default(),
            lifecycle: CancellationToken::new(),
        }
    }
    fn session(&self) -> Session {
        Session::new(self.directory.path().to_path_buf())
    }
    fn assert_requests(&self, index: usize, count: usize, secret: &str, model: &str) {
        let requests = self.endpoints[index].requests.lock().unwrap();
        assert_eq!(requests.len(), count, "wrong destination/request count");
        for request in requests.iter() {
            assert_eq!(request.path, "/v1/chat/completions");
            assert_eq!(request.auth, format!("Bearer {secret}"));
            assert_eq!(request.body["model"], model);
            assert_clean(&request.body.to_string());
        }
    }
}
struct Policy;
#[async_trait]
impl ApprovalPolicy for Policy {
    async fn approve(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
        panic!("switch is effect-free")
    }
}
#[derive(Default)]
struct Events {
    values: Mutex<Vec<Event>>,
    text: Notify,
}
impl EventSink for Events {
    fn emit(&self, event: Event) {
        // Notify after a delta is actually consumed by StandardLoop, not just sent by HTTP.
        if matches!(event, Event::Text { .. }) {
            self.text.notify_one();
        }
        self.values.lock().unwrap().push(event);
    }
}
struct Switch(Gate);
#[async_trait]
impl Tool for Switch {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "switch".into(),
            description: "deterministic iteration barrier".into(),
            input_schema: json!({"type":"object"}),
        }
    }
    async fn prepare(&self, _: &ToolCall, _: &ToolContext) -> Result<PreparedAction> {
        Ok(PreparedAction {
            approval: None,
            payload: json!({}),
        })
    }
    async fn execute(&self, _: PreparedAction, _: &ToolContext) -> Result<String> {
        self.0.stop().await;
        Ok("switch complete".into())
    }
}
struct Tools(Option<Arc<dyn Tool>>);
impl ToolRegistry for Tools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.0.iter().map(|tool| tool.definition()).collect()
    }
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.0
            .as_ref()
            .filter(|tool| tool.definition().name == name)
            .cloned()
    }
}
fn assert_clean(value: &str) {
    for secret in SECRETS {
        assert!(
            !value.contains(secret),
            "credential leaked outside authorization"
        );
    }
}
async fn run(
    engine: &StandardLoop,
    session: &mut Session,
    prompt: &str,
    events: &Events,
) -> Result<()> {
    engine
        .run(
            session,
            prompt.into(),
            &Policy,
            events,
            CancellationToken::new(),
        )
        .await
}
fn history(mut session: Session) -> Session {
    for _ in 0..2 {
        session
            .messages
            .push(Message::text(Role::User, "u".repeat(1000)));
        session
            .messages
            .push(Message::text(Role::Assistant, "a".repeat(1000)));
    }
    session
}

#[tokio::test]
async fn metadata_and_inference_send_credentials_only_to_their_own_destination() {
    for provider in ["openai", "ollama"] {
        let mut fixture = Fixture::new(
            provider,
            vec![Reply::Text("alpha answer")],
            vec![Reply::Text("beta answer")],
        )
        .await;
        let engine = fixture.engine(None);
        let mut session = fixture.session();
        for (index, secret) in SECRETS.iter().take(2).enumerate() {
            fixture
                .manager
                .select(fixture.records[index].id)
                .await
                .unwrap();
            run(&engine, &mut session, "turn", &Events::default())
                .await
                .unwrap();
            let model = if index == 0 { "alpha" } else { "beta" };
            fixture.assert_requests(index, 1, secret, model);
            let metadata = fixture.endpoints[index].metadata.lock().unwrap().clone();
            let expected_paths = if provider == "openai" {
                vec!["/v1/models"]
            } else {
                vec!["/api/tags", "/api/show"]
            };
            assert_eq!(
                metadata.iter().map(|r| r.path).collect::<Vec<_>>(),
                expected_paths
            );
            for request in metadata {
                assert_eq!(request.auth, format!("Bearer {secret}"));
                assert_clean(&request.body.to_string());
                if request.path == "/api/show" {
                    assert_eq!(request.body, json!({"model": model}));
                } else {
                    assert_eq!(request.body, Value::Null);
                }
            }
        }
        fixture.supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn same_connection_model_change_is_deferred_through_tools_and_compaction() {
    for provider in ["openai", "ollama"] {
        // Exercise an iteration boundary and both compaction entry points separately.
        for phase in ["tool", "automatic", "manual"] {
            test_deferred_model_change(provider, phase).await;
        }
    }
}

async fn test_deferred_model_change(provider: &'static str, phase: &'static str) {
    let gate = Gate::default();
    let mut replies = if phase == "tool" {
        vec![Reply::Tool, Reply::Text("alpha continuation")]
    } else {
        let mut replies = vec![Reply::Summary(gate.clone())];
        if phase == "automatic" {
            replies.push(Reply::Text("alpha after summary"));
        }
        replies
    };
    let active_count = replies.len();
    replies.push(Reply::Text("beta next turn"));
    let mut fixture = Fixture::new(provider, replies, vec![]).await;
    let initial = fixture.manager.select(fixture.records[0].id).await.unwrap();
    let mut engine = fixture.engine((phase == "tool").then(|| gate.clone()));
    engine.config.context_tokens = if phase == "automatic" { 5000 } else { 10000 };
    engine.config.max_output_tokens = 100;
    let mut session = if phase == "tool" {
        fixture.session()
    } else {
        history(fixture.session())
    };
    let task = tokio::spawn(async move {
        if phase == "manual" {
            engine
                .compact(&mut session, &Events::default(), CancellationToken::new())
                .await
                .unwrap();
        } else {
            run(&engine, &mut session, "continue", &Events::default())
                .await
                .unwrap();
        }
        (engine, session)
    });
    wait(&gate.entered).await;
    let selected = fixture
        .manager
        .select_model(fixture.records[0].id, "beta".into())
        .await
        .unwrap();
    // Model selection must not rotate the identity/credential generation.
    assert_eq!(selected.connection, initial.connection);
    assert_eq!(initial.model, "alpha");
    assert_eq!(selected.model, "beta");
    gate.release.notify_one();
    let (engine, mut session) = task.await.unwrap();
    fixture.assert_requests(0, active_count, SECRETS[0], "alpha");
    fixture.assert_requests(1, 0, SECRETS[1], "beta");
    if phase == "tool" {
        assert!(session
            .messages
            .iter()
            .any(|m| m.role == Role::Tool && m.content == "switch complete"));
    } else {
        assert!(session.summary.is_some());
    }
    run(&engine, &mut session, "next turn", &Events::default())
        .await
        .unwrap();
    assert_eq!(session.messages.last().unwrap().content, "beta next turn");
    let requests = fixture.endpoints[0].requests.lock().unwrap().clone();
    assert_eq!(requests.len(), active_count + 1);
    assert_eq!(requests[active_count].body["model"], "beta");
    for request in requests {
        assert_eq!(request.auth, format!("Bearer {}", SECRETS[0]));
        assert_clean(&request.body.to_string());
    }
    fixture.assert_requests(1, 0, SECRETS[1], "beta");
    // Metadata also continues using the original credential after the model-only change.
    let metadata = fixture.endpoints[0].metadata.lock().unwrap().clone();
    assert!(!metadata.is_empty());
    for request in metadata {
        assert_eq!(request.auth, format!("Bearer {}", SECRETS[0]));
        assert_clean(&request.body.to_string());
    }
    fixture.supervisor.shutdown().await;
}

#[tokio::test]
async fn actual_tool_continuation_pins_connection_and_model_until_next_turn() {
    for provider in ["openai", "ollama"] {
        let mut fixture = Fixture::new(
            provider,
            vec![Reply::Tool, Reply::Text("alpha answer")],
            vec![Reply::Text("beta answer")],
        )
        .await;
        let gate = Gate::default();
        let engine = fixture.engine(Some(gate.clone()));
        let mut session = fixture.session();
        let task = tokio::spawn(async move {
            run(&engine, &mut session, "use switch", &Events::default())
                .await
                .unwrap();
            (engine, session)
        });
        wait(&gate.entered).await;
        fixture.manager.select(fixture.records[1].id).await.unwrap();
        gate.release.notify_one();
        let (engine, mut session) = task.await.unwrap();
        fixture.assert_requests(0, 2, SECRETS[0], "alpha");
        fixture.assert_requests(1, 0, SECRETS[1], "beta");
        let requests = fixture.endpoints[0].requests.lock().unwrap().clone();
        assert!(requests[1].body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "tool" && m["content"] == "switch complete"));
        assert!(session.messages.iter().any(|m| m.role == Role::Tool));
        run(&engine, &mut session, "next turn", &Events::default())
            .await
            .unwrap();
        assert_eq!(session.messages.last().unwrap().content, "beta answer");
        fixture.assert_requests(1, 1, SECRETS[1], "beta");
        assert_clean(&format!("{session:?}"));
        fixture.supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn automatic_and_manual_compaction_use_pinned_http_connection() {
    for provider in ["openai", "ollama"] {
        for manual in [false, true] {
            let gate = Gate::default();
            let mut replies = vec![Reply::Summary(gate.clone())];
            if !manual {
                replies.push(Reply::Tool);
                replies.push(Reply::Text("alpha after summary and tool"));
            }
            let mut fixture =
                Fixture::new(provider, replies, vec![Reply::Text("beta next turn")]).await;
            let tool_gate = Gate::default();
            // The summary is the external mutation barrier in this test. The actual
            // tool still executes, but need not block a second time.
            tool_gate.release.notify_one();
            let mut engine = fixture.engine(Some(tool_gate));
            engine.config.context_tokens = if manual { 10000 } else { 5000 };
            engine.config.max_output_tokens = 100;
            let mut session = history(fixture.session());
            let task = tokio::spawn(async move {
                if manual {
                    engine
                        .compact(&mut session, &Events::default(), CancellationToken::new())
                        .await
                        .unwrap();
                } else {
                    run(&engine, &mut session, "continue", &Events::default())
                        .await
                        .unwrap();
                }
                (engine, session)
            });
            wait(&gate.entered).await;
            fixture.manager.select(fixture.records[1].id).await.unwrap();
            gate.release.notify_one();
            let (engine, mut session) = task.await.unwrap();
            assert!(session.summary.is_some());
            fixture.assert_requests(0, if manual { 1 } else { 3 }, SECRETS[0], "alpha");
            fixture.assert_requests(1, 0, SECRETS[1], "beta");
            let requests = fixture.endpoints[0].requests.lock().unwrap().clone();
            assert!(requests[0].body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .starts_with("Summarize repository work"));
            if !manual {
                assert!(session
                    .messages
                    .iter()
                    .any(|m| m.role == Role::Tool && m.content == "switch complete"));
            }
            run(&engine, &mut session, "next turn", &Events::default())
                .await
                .unwrap();
            fixture.assert_requests(1, 1, SECRETS[1], "beta");
            fixture.supervisor.shutdown().await;
        }
    }
}

#[tokio::test]
async fn selected_http_and_stream_errors_never_fall_back_or_leak_credentials() {
    for provider in ["openai", "ollama"] {
        for error in [
            Reply::Http(StatusCode::UNAUTHORIZED),
            Reply::Http(StatusCode::INTERNAL_SERVER_ERROR),
            Reply::StreamError,
        ] {
            let mut fixture = Fixture::new(
                provider,
                vec![Reply::Tool, error],
                vec![Reply::Text("beta recovery")],
            )
            .await;
            let gate = Gate::default();
            let engine = fixture.engine(Some(gate.clone()));
            let mut session = fixture.session();
            let task = tokio::spawn(async move {
                let events = Events::default();
                let error = run(&engine, &mut session, "use switch", &events)
                    .await
                    .unwrap_err();
                assert_clean(&format!(
                    "{error:?} {error} {:?}",
                    events.values.lock().unwrap()
                ));
                (engine, session)
            });
            wait(&gate.entered).await;
            fixture.manager.select(fixture.records[1].id).await.unwrap();
            gate.release.notify_one();
            let (engine, mut session) = task.await.unwrap();
            assert!(session.interrupted);
            assert!(matches!(session.events.last(), Some(Event::Error { .. })));
            fixture.assert_requests(0, 2, SECRETS[0], "alpha");
            fixture.assert_requests(1, 0, SECRETS[1], "beta");
            let persisted = engine.sessions.load(&session.id).await.unwrap();
            assert_clean(&format!("{persisted:?}"));
            run(
                &engine,
                &mut session,
                "explicit next turn",
                &Events::default(),
            )
            .await
            .unwrap();
            fixture.assert_requests(1, 1, SECRETS[1], "beta");
            fixture.supervisor.shutdown().await;
        }
    }
}

#[tokio::test]
async fn revocation_and_rotation_between_tool_iterations_block_old_credentials() {
    for provider in ["openai", "ollama"] {
        for rotate in [false, true] {
            let mut fixture = Fixture::new(
                provider,
                vec![Reply::Tool, Reply::Text("rotated recovery")],
                vec![Reply::Text("beta recovery")],
            )
            .await;
            let gate = Gate::default();
            let engine = fixture.engine(Some(gate.clone()));
            let mut session = fixture.session();
            let task = tokio::spawn(async move {
                let result = run(&engine, &mut session, "use switch", &Events::default()).await;
                (engine, session, result)
            });
            wait(&gate.entered).await;
            if rotate {
                fixture
                    .manager
                    .update(
                        fixture.records[0].id,
                        fixture.records[0].settings.clone(),
                        Some(Secret::new(SECRETS[2].into()).unwrap()),
                    )
                    .await
                    .unwrap();
            } else {
                fixture.manager.revoke(fixture.records[0].id).await.unwrap();
            }
            fixture.manager.select(fixture.records[1].id).await.unwrap();
            gate.release.notify_one();
            let (engine, mut session, result) = task.await.unwrap();
            assert!(matches!(
                result,
                Err(AgentError::Provider(ProviderError::Revoked))
            ));
            assert!(session.interrupted);
            fixture.assert_requests(0, 1, SECRETS[0], "alpha");
            fixture.assert_requests(1, 0, SECRETS[1], "beta");
            if rotate {
                fixture.manager.select(fixture.records[0].id).await.unwrap();
                run(
                    &engine,
                    &mut session,
                    "explicit rotated turn",
                    &Events::default(),
                )
                .await
                .unwrap();
                let requests = fixture.endpoints[0].requests.lock().unwrap();
                assert_eq!(requests.len(), 2);
                assert_eq!(requests[1].auth, format!("Bearer {}", SECRETS[2]));
                assert_eq!(requests[1].body["model"], "alpha");
                assert_clean(&requests[1].body.to_string());
            } else {
                run(
                    &engine,
                    &mut session,
                    "explicit other turn",
                    &Events::default(),
                )
                .await
                .unwrap();
                fixture.assert_requests(1, 1, SECRETS[1], "beta");
            }
            fixture.supervisor.shutdown().await;
        }
    }
}

#[tokio::test]
async fn revocation_and_rotation_cancel_a_locally_active_http_stream() {
    for provider in ["openai", "ollama"] {
        for rotate in [false, true] {
            let mut fixture = Fixture::new(provider, vec![Reply::Stalled], vec![]).await;
            let engine = fixture.engine(None);
            let mut session = fixture.session();
            let events = Arc::new(Events::default());
            let sink = events.clone();
            let task = tokio::spawn(async move {
                let result = run(&engine, &mut session, "start streaming", &sink).await;
                (session, result)
            });
            wait(&events.text).await;
            if rotate {
                fixture
                    .manager
                    .update(
                        fixture.records[0].id,
                        fixture.records[0].settings.clone(),
                        Some(Secret::new(SECRETS[2].into()).unwrap()),
                    )
                    .await
                    .unwrap();
            } else {
                fixture.manager.revoke(fixture.records[0].id).await.unwrap();
            }
            // Only local cancellation is promised: this does not assert remote rollback.
            let (session, result) = tokio::time::timeout(Duration::from_secs(3), task)
                .await
                .expect("revocation must wake a blocked local stream")
                .unwrap();
            assert!(matches!(
                result,
                Err(AgentError::Provider(ProviderError::Revoked))
            ));
            assert!(session.interrupted);
            assert_clean(&format!("{session:?} {:?}", events.values.lock().unwrap()));
            fixture.assert_requests(0, 1, SECRETS[0], "alpha");
            fixture.assert_requests(1, 0, SECRETS[1], "beta");
            fixture.supervisor.shutdown().await;
        }
    }
}

#[tokio::test]
async fn selection_during_http_stream_only_changes_the_next_turn() {
    for provider in ["openai", "ollama"] {
        let gate = Gate::default();
        let mut fixture = Fixture::new(
            provider,
            vec![Reply::GatedStream(gate.clone())],
            vec![Reply::Text("beta next turn")],
        )
        .await;
        let engine = fixture.engine(None);
        let mut session = fixture.session();
        let events = Arc::new(Events::default());
        let sink = events.clone();
        let task = tokio::spawn(async move {
            run(&engine, &mut session, "start streaming", &sink)
                .await
                .unwrap();
            (engine, session)
        });
        wait(&events.text).await;
        wait(&gate.entered).await;
        fixture.manager.select(fixture.records[1].id).await.unwrap();
        fixture.assert_requests(1, 0, SECRETS[1], "beta");
        gate.release.notify_one();
        let (engine, mut session) = task.await.unwrap();
        assert_eq!(session.messages.last().unwrap().content, "started finished");
        fixture.assert_requests(0, 1, SECRETS[0], "alpha");
        run(&engine, &mut session, "next turn", &Events::default())
            .await
            .unwrap();
        fixture.assert_requests(1, 1, SECRETS[1], "beta");
        fixture.supervisor.shutdown().await;
    }
}
