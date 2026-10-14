use async_trait::async_trait;
use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt;
use huginn_application::{
    composition::model_id,
    providers::{providers_id, ProvidersPlugin},
};
use huginn_core::{
    connections::*, kernel::Supervisor, providers::*, AgentError, CancellationToken, Message,
    ModelDelta, ModelProvider, ModelRequest, ModelService, Role, ToolDefinition,
};
use huginn_runtime::providers::{BuiltinInferencePlugin, InferenceAdapter, ModelCatalog};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const SECRET: &str = "p04-sentinel-secret";
fn material(provider: &str, endpoint: &str) -> ConnectionMaterial {
    ConnectionMaterial {
        settings: ConnectionSettings {
            provider: provider.into(),
            endpoint: Some(endpoint.into()),
            model: "fixture".into(),
            requires_secret: true,
            allow_loopback_http: true,
        },
        secret: Some(Secret::new(SECRET.into()).unwrap()),
    }
}
fn selection(provider: &str) -> ModelSelection {
    ModelSelection {
        connection: ConnectionIdentity {
            id: "connection".into(),
            revision: 1,
            adapter: ProviderRegistration::new(provider.into(), 1),
        },
        model: "fixture".into(),
    }
}
fn request() -> ModelRequest {
    ModelRequest {
        messages: vec![Message::text(Role::User, "hello")],
        tools: vec![],
        max_tokens: 128,
    }
}
async fn server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    (
        endpoint,
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }),
    )
}
fn streaming() -> Body {
    // Split through JSON, UTF-8, SSE boundaries, and tool argument fragments.
    let data = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"héllo\"}}]}\r\n\r\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call\",\"function\":{\"name\":\"list_files\",\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\".\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n");
    Body::from_stream(futures::stream::iter(data.as_bytes().iter().map(|byte| {
        Ok::<_, std::convert::Infallible>(bytes::Bytes::from(vec![*byte]))
    })))
}
fn fixture(capture: Arc<Mutex<Vec<(String, Value)>>>, tools: bool) -> Router {
    Router::new()
        .route("/v1/models", get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }))
        .route("/api/tags", get(|| async { Json(json!({"models":[{"name":"fixture"}]})) }))
        .route("/api/show", post(move || async move { Json(json!({"capabilities": if tools { vec!["completion", "tools"] } else { vec!["completion"] }})) }))
        .route("/v1/chat/completions", post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let capture = capture.clone();
            async move {
                capture.lock().unwrap().push((headers["authorization"].to_str().unwrap().into(), body));
                ([("content-type", "text/event-stream")], streaming())
            }
        }))
}

#[tokio::test]
async fn both_adapters_preserve_fragmented_text_tools_and_finish() {
    for provider in ["openai", "ollama"] {
        let capture = Arc::new(Mutex::new(vec![]));
        let (endpoint, task) = server(fixture(capture.clone(), true)).await;
        let endpoint = if provider == "openai" {
            format!("{endpoint}/v1")
        } else {
            endpoint
        };
        let adapter = if provider == "openai" {
            InferenceAdapter::openai_compatible(2)
        } else {
            InferenceAdapter::ollama(2)
        };
        let model = adapter
            .create_connection_model(
                &selection(provider),
                material(provider, &endpoint),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let deltas = model
            .stream(request(), CancellationToken::new())
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(deltas
            .iter()
            .any(|d| matches!(d, Ok(ModelDelta::Text(t)) if t == "héllo")));
        assert!(deltas.iter().any(|d| matches!(d, Ok(ModelDelta::ToolCall(t)) if t.name == "list_files" && t.arguments == json!({"path":"."}))));
        assert!(matches!(deltas.last(), Some(Ok(ModelDelta::Finished))));
        assert_eq!(capture.lock().unwrap()[0].0, format!("Bearer {SECRET}"));
        task.abort();
    }
}

#[tokio::test]
async fn missing_failed_and_absent_catalogs_are_explicit_without_inference() {
    for (status, expected) in [
        (StatusCode::NOT_FOUND, ModelCatalog::Unsupported),
        (StatusCode::INTERNAL_SERVER_ERROR, ModelCatalog::Unavailable),
    ] {
        let (endpoint, task) =
            server(Router::new().route("/v1/models", get(move || async move { (status, SECRET) })))
                .await;
        let adapter = InferenceAdapter::openai_compatible(1);
        let endpoint = format!("{endpoint}/v1");
        assert_eq!(
            adapter
                .discover_models(&material("openai", &endpoint), CancellationToken::new())
                .await
                .unwrap(),
            expected
        );
        let error = adapter
            .create_connection_model(
                &selection("openai"),
                material("openai", &endpoint),
                CancellationToken::new(),
            )
            .await
            .err()
            .unwrap();
        assert!(!error.to_string().contains(SECRET));
        task.abort();
    }
    let capture = Arc::new(Mutex::new(vec![]));
    let (endpoint, task) = server(fixture(capture.clone(), true)).await;
    let mut missing = selection("openai");
    missing.model = "missing".into();
    let mut connection = material("openai", &format!("{endpoint}/v1"));
    connection.settings.model = "missing".into();
    assert!(InferenceAdapter::openai_compatible(1)
        .create_connection_model(&missing, connection, CancellationToken::new())
        .await
        .is_err());
    assert!(capture.lock().unwrap().is_empty());
    task.abort();
}

#[tokio::test]
async fn ollama_show_failure_or_missing_capabilities_fails_cleanly() {
    for (status, body) in [
        (StatusCode::NOT_FOUND, json!({"error": "model not found"})),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": "server error"}),
        ),
        (StatusCode::OK, json!({"capabilities": ["tools"]})), // missing "completion"
        (StatusCode::OK, json!({"other": true})),             // missing capabilities entirely
    ] {
        let (endpoint, task) = server(
            Router::new()
                .route(
                    "/api/tags",
                    get(|| async { Json(json!({"models":[{"name":"fixture"}]})) }),
                )
                .route(
                    "/api/show",
                    post(move || async move { (status, Json(body)) }),
                ),
        )
        .await;
        let result = InferenceAdapter::ollama(1)
            .create_connection_model(
                &selection("ollama"),
                material("ollama", &endpoint),
                CancellationToken::new(),
            )
            .await;
        assert!(result.is_err(), "expected error for status {status}");
        task.abort();
    }
}

#[tokio::test]
async fn ollama_without_tool_capability_fails_before_inference() {
    let capture = Arc::new(Mutex::new(vec![]));
    let (endpoint, task) = server(fixture(capture.clone(), false)).await;
    let model = InferenceAdapter::ollama(1)
        .create_connection_model(
            &selection("ollama"),
            material("ollama", &endpoint),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let mut request = request();
    request.tools.push(ToolDefinition {
        name: "list_files".into(),
        description: "list".into(),
        input_schema: json!({"type":"object"}),
    });
    assert!(model
        .stream(request, CancellationToken::new())
        .await
        .is_err());
    assert!(capture.lock().unwrap().is_empty());
    task.abort();
}

#[tokio::test]
async fn redirects_never_send_authorization_to_destination() {
    let received = Arc::new(Mutex::new(0));
    let copy = received.clone();
    let (destination, target) = server(Router::new().fallback(move || {
        let copy = copy.clone();
        async move {
            *copy.lock().unwrap() += 1;
            ""
        }
    }))
    .await;
    let (source, task) = server(Router::new().fallback(move || {
        let destination = destination.clone();
        async move {
            (
                StatusCode::TEMPORARY_REDIRECT,
                [("location", destination)],
                SECRET,
            )
        }
    }))
    .await;
    let model = huginn_runtime::model::OpenAiProvider::with_transport(
        &source,
        "fixture".into(),
        Some(SECRET.into()),
        1,
        true,
    )
    .unwrap();
    let error = model
        .stream(request(), CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert!(!error.to_string().contains(SECRET));
    assert_eq!(*received.lock().unwrap(), 0);
    task.abort();
    target.abort();
}

struct Credentials {
    endpoints: [String; 2],
}
#[async_trait]
impl ConnectionCredentials for Credentials {
    async fn resolve(
        &self,
        identity: &ConnectionIdentity,
        _: CancellationToken,
    ) -> ConnectionResult<ConnectionMaterial> {
        let mut material = material(
            &identity.adapter.provider,
            &self.endpoints[usize::from(identity.id == "b")],
        );
        material.secret = Some(Secret::new(format!("{SECRET}-{}", identity.id)).unwrap());
        Ok(material)
    }
}

#[tokio::test]
async fn router_pins_both_adapters_and_revocation_stops_subsequent_requests() {
    let seen_a = Arc::new(Mutex::new(vec![]));
    let seen_b = Arc::new(Mutex::new(vec![]));
    let (a, task_a) = server(fixture(seen_a.clone(), true)).await;
    let (b, task_b) = server(fixture(seen_b.clone(), true)).await;
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ProvidersPlugin::with_credentials(Arc::new(
            Credentials {
                endpoints: [format!("{a}/v1"), b],
            },
        ))))
        .unwrap();
    supervisor
        .register(Box::new(BuiltinInferencePlugin { timeout_secs: 2 }))
        .unwrap();
    supervisor.start().await.unwrap();
    let management = supervisor
        .resolve::<ProvidersService>(&providers_id())
        .unwrap()
        .0
        .clone();
    let router = supervisor
        .resolve::<ModelService>(&model_id())
        .unwrap()
        .0
        .clone();
    let mut identities = vec![];
    for (id, provider) in [("a", "openai"), ("b", "ollama")] {
        let adapter = management
            .diagnostic()
            .unwrap()
            .adapters
            .into_iter()
            .find(|(_, d)| d.id == provider)
            .unwrap()
            .0;
        let ProviderActionOutput::Connected(identity) = management
            .execute(
                ProviderAction::Connect {
                    id: id.into(),
                    adapter,
                },
                CancellationToken::new(),
            )
            .await
            .unwrap()
        else {
            panic!("connect")
        };
        identities.push(identity);
    }
    let select = |identity: ConnectionIdentity| {
        ProviderAction::Select(ModelSelection {
            connection: identity,
            model: "fixture".into(),
        })
    };
    management
        .execute(select(identities[0].clone()), CancellationToken::new())
        .await
        .unwrap();
    let pinned = router
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    management
        .execute(select(identities[1].clone()), CancellationToken::new())
        .await
        .unwrap();
    // Continuation and compaction are repeated inference on the same pinned handle.
    for _ in 0..3 {
        let result = pinned
            .stream(request(), CancellationToken::new())
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(result.iter().all(Result::is_ok));
    }
    assert_eq!(seen_a.lock().unwrap().len(), 3);
    assert!(seen_b.lock().unwrap().is_empty());
    assert!(seen_a
        .lock()
        .unwrap()
        .iter()
        .all(|(auth, _)| auth == &format!("Bearer {SECRET}-a")));
    router
        .stream(request(), CancellationToken::new())
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(seen_b.lock().unwrap()[0].0, format!("Bearer {SECRET}-b"));
    management
        .execute(
            ProviderAction::Revoke(identities[0].clone()),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(
        pinned.stream(request(), CancellationToken::new()).await,
        Err(AgentError::Provider(ProviderError::Revoked))
    ));
    assert_eq!(seen_a.lock().unwrap().len(), 3);
    supervisor.shutdown().await;
    task_a.abort();
    task_b.abort();
}

#[tokio::test]
async fn catalog_timeout_and_cancellation_are_bounded() {
    let (endpoint, task) = server(Router::new().fallback(|| async {
        tokio::time::sleep(Duration::from_secs(20)).await;
        ""
    }))
    .await;
    let adapter = InferenceAdapter::openai_compatible(1);
    assert_eq!(
        adapter
            .discover_models(&material("openai", &endpoint), CancellationToken::new())
            .await
            .unwrap(),
        ModelCatalog::Unavailable
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        adapter
            .discover_models(&material("openai", &endpoint), cancel)
            .await,
        Err(AgentError::Cancelled)
    ));
    task.abort();
}

#[tokio::test]
async fn persisted_connection_manager_drives_dynamic_adapter_and_revocation() {
    use huginn_application::{
        connections::{ConnectionAccess, ConnectionManager},
        providers::catalog_id,
    };
    use huginn_runtime::connections::{EphemeralSecrets, SqliteConnections};
    let capture = Arc::new(Mutex::new(vec![]));
    let (endpoint, task) = server(fixture(capture.clone(), true)).await;
    let directory = tempfile::tempdir().unwrap();
    let access = ConnectionAccess::new(Arc::new(EphemeralSecrets::default()));
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ProvidersPlugin::with_credentials(access.clone())))
        .unwrap();
    supervisor
        .register(Box::new(BuiltinInferencePlugin { timeout_secs: 2 }))
        .unwrap();
    supervisor.start().await.unwrap();
    let manager = ConnectionManager::open(
        Arc::new(SqliteConnections::open(&directory.path().join("connections.db")).unwrap()),
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
    .unwrap();
    let material = material("ollama", &endpoint);
    let record = manager
        .create(material.settings, material.secret)
        .await
        .unwrap();
    manager.select(record.id).await.unwrap();
    let router = supervisor
        .resolve::<ModelService>(&model_id())
        .unwrap()
        .0
        .clone();
    let pinned = router
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    assert!(pinned
        .stream(request(), CancellationToken::new())
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .iter()
        .all(Result::is_ok));
    manager.revoke(record.id).await.unwrap();
    assert!(pinned
        .stream(request(), CancellationToken::new())
        .await
        .is_err());
    assert_eq!(capture.lock().unwrap().len(), 1);
    supervisor.shutdown().await;
    task.abort();
}

#[tokio::test]
async fn malformed_and_oversized_catalogs_are_invalid_not_unavailable() {
    for body in ["{".to_owned(), "x".repeat(1_048_577)] {
        let (endpoint, task) = server(Router::new().route(
            "/models",
            get(move || {
                let body = body.clone();
                async move { body }
            }),
        ))
        .await;
        assert_eq!(
            InferenceAdapter::openai_compatible(2)
                .discover_models(&material("openai", &endpoint), CancellationToken::new())
                .await
                .unwrap(),
            ModelCatalog::Invalid
        );
        task.abort();
    }
}

#[tokio::test]
async fn cancellation_precedes_already_buffered_success_deltas() {
    let (endpoint, task) = server(Router::new().route("/chat/completions", post(|| async {
        "data: {\"choices\":[{\"delta\":{\"content\":\"first\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"second\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
    }))).await;
    let provider = huginn_runtime::model::OpenAiProvider::with_transport(
        &endpoint,
        "fixture".into(),
        None,
        2,
        true,
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let mut stream = provider.stream(request(), cancel.clone()).await.unwrap();
    assert!(matches!(stream.next().await, Some(Ok(ModelDelta::Text(_)))));
    cancel.cancel();
    assert!(matches!(
        stream.next().await,
        Some(Err(AgentError::Cancelled))
    ));
    assert!(stream.next().await.is_none());
    task.abort();
}

#[tokio::test]
async fn pinned_selection_can_choose_another_catalog_model_on_same_connection() {
    let capture = Arc::new(Mutex::new(vec![]));
    let (endpoint, task) = server(fixture(capture.clone(), true)).await;
    let endpoint = format!("{endpoint}/v1");
    let mut connection = material("openai", &endpoint);
    connection.settings.model = "previous-default".into();
    let model = InferenceAdapter::openai_compatible(2)
        .create_connection_model(&selection("openai"), connection, CancellationToken::new())
        .await
        .unwrap();
    model
        .stream(request(), CancellationToken::new())
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(capture.lock().unwrap()[0].1["model"], "fixture");
    task.abort();
}

#[tokio::test]
async fn fragmented_stream_errors_do_not_reflect_remote_secrets() {
    let data = format!("data: {{\"error\":{{\"message\":\"{SECRET}\"}}}}\n\n");
    let (endpoint, task) = server(Router::new().route(
        "/chat/completions",
        post(move || {
            let data = data.clone();
            async move {
                Body::from_stream(futures::stream::iter(data.into_bytes().into_iter().map(
                    |byte| Ok::<_, std::convert::Infallible>(bytes::Bytes::from(vec![byte])),
                )))
            }
        }),
    ))
    .await;
    let model = huginn_runtime::model::OpenAiProvider::with_transport(
        &endpoint,
        "fixture".into(),
        Some(SECRET.into()),
        2,
        true,
    )
    .unwrap();
    let mut stream = model
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let error = stream.next().await.unwrap().unwrap_err();
    assert!(!format!("{error:?} {error}").contains(SECRET));
    assert!(stream.next().await.is_none());
    task.abort();
}
