//! Composed boundary tests: real UI/management/SQLite/router/adapters, local HTTP
//! and credential doubles only. Not native keychain or installed Ollama acceptance.
use async_trait::async_trait;
use axum::{
    extract::{Path, Request},
    http::HeaderMap,
    middleware::{self, Next},
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt;
use huginn_application::{
    composition::model_id,
    connections::{ConnectionAccess, ConnectionManager, ConnectionsPlugin},
    providers::{catalog_id, providers_id, ProvidersPlugin},
};
use huginn_core::{
    connections::*,
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    providers::*,
    CancellationToken, Message, ModelDelta, ModelProvider, ModelRequest, ModelService, Role,
    Session, SessionStore,
};
use huginn_plugin_api::v1::{contributions_id, Contributions, UiService};
use huginn_presentation::{providers::ProvidersUiPlugin, state::Ui};
use huginn_runtime::{
    connections::SqliteConnections, providers::BuiltinInferencePlugin, sessions::SqliteSessions,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

const A: &str = "d674-connection-a-secret-never-public";
const B: &str = "d674-connection-b-secret-never-public";

struct Secrets {
    mode: SecretMode,
    values: Mutex<BTreeMap<SecretReference, String>>,
    locked: AtomicBool,
    read_errors: Mutex<BTreeMap<SecretReference, ConnectionError>>,
    fail_write: AtomicBool,
    suspend_write: AtomicBool,
    write_entered: tokio::sync::Notify,
    write_release: tokio::sync::Notify,
}
impl Secrets {
    fn new(mode: SecretMode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            values: Mutex::new(BTreeMap::new()),
            locked: AtomicBool::new(false),
            read_errors: Mutex::new(BTreeMap::new()),
            fail_write: AtomicBool::new(false),
            suspend_write: AtomicBool::new(false),
            write_entered: tokio::sync::Notify::new(),
            write_release: tokio::sync::Notify::new(),
        })
    }
    fn ready(&self) -> ConnectionResult<()> {
        if self.locked.load(Ordering::SeqCst) {
            Err(ConnectionError::Locked)
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl SecretStore for Secrets {
    fn mode(&self) -> SecretMode {
        self.mode
    }
    async fn read(&self, reference: &SecretReference) -> ConnectionResult<Secret> {
        self.ready()?;
        if let Some(error) = self.read_errors.lock().unwrap().get(reference) {
            return Err(*error);
        }
        Secret::new(
            self.values
                .lock()
                .unwrap()
                .get(reference)
                .cloned()
                .ok_or(ConnectionError::MissingSecret)?,
        )
    }
    async fn write(&self, reference: &SecretReference, secret: &Secret) -> ConnectionResult<()> {
        self.ready()?;
        self.values
            .lock()
            .unwrap()
            .insert(reference.clone(), secret.expose().into());
        if self.suspend_write.swap(false, Ordering::SeqCst) {
            self.write_entered.notify_one();
            self.write_release.notified().await;
        }
        // Fail after the side effect, exercising durable intent cleanup.
        if self.fail_write.swap(false, Ordering::SeqCst) {
            Err(ConnectionError::SecretStore)
        } else {
            Ok(())
        }
    }
    async fn delete(&self, reference: &SecretReference) -> ConnectionResult<()> {
        self.ready()?;
        self.values.lock().unwrap().remove(reference);
        Ok(())
    }
}

// Route explicit modes to independent injected backends, just like the host.
// Restart can lose the ephemeral backend while a competing secure key stays usable.
struct RoutedSecrets {
    secure: Arc<Secrets>,
    ephemeral: Arc<Secrets>,
}
impl RoutedSecrets {
    fn backend(&self, mode: SecretMode) -> &Secrets {
        match mode {
            SecretMode::Secure => &self.secure,
            SecretMode::Ephemeral => &self.ephemeral,
        }
    }
}
#[async_trait]
impl SecretStore for RoutedSecrets {
    fn mode(&self) -> SecretMode {
        SecretMode::Secure
    }
    async fn read(&self, reference: &SecretReference) -> ConnectionResult<Secret> {
        self.secure.read(reference).await
    }
    async fn write(&self, reference: &SecretReference, secret: &Secret) -> ConnectionResult<()> {
        self.secure.write(reference, secret).await
    }
    async fn delete(&self, reference: &SecretReference) -> ConnectionResult<()> {
        self.secure.delete(reference).await
    }
    async fn read_mode(
        &self,
        mode: SecretMode,
        reference: &SecretReference,
    ) -> ConnectionResult<Secret> {
        self.backend(mode).read(reference).await
    }
    async fn write_mode(
        &self,
        mode: SecretMode,
        reference: &SecretReference,
        secret: &Secret,
    ) -> ConnectionResult<()> {
        self.backend(mode).write(reference, secret).await
    }
    async fn delete_mode(
        &self,
        mode: SecretMode,
        reference: &SecretReference,
    ) -> ConnectionResult<()> {
        self.backend(mode).delete(reference).await
    }
}

struct UiHost(Arc<Contributions>);
#[async_trait]
impl Plugin for UiHost {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "test.ui-host".into(),
            provides: vec![contributions_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> huginn_core::Result<()> {
        ctx.provide(contributions_id(), UiService(self.0.clone()))
    }
}
struct Harness {
    supervisor: Supervisor,
    ui: Arc<Ui>,
    manager: Arc<ConnectionManager>,
    model: Arc<ModelService>,
    store: Arc<SqliteConnections>,
    session: Session,
}
async fn open(root: &std::path::Path, secrets: Arc<dyn SecretStore>) -> Harness {
    let store = Arc::new(SqliteConnections::open(&root.join("providers.db")).unwrap());
    let access = ConnectionAccess::new(secrets);
    let contributions = Arc::new(Contributions::default());
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ProvidersPlugin::with_credentials(access.clone())))
        .unwrap();
    supervisor
        .register(Box::new(BuiltinInferencePlugin { timeout_secs: 2 }))
        .unwrap();
    supervisor
        .register(Box::new(UiHost(contributions.clone())))
        .unwrap();
    supervisor.start().await.unwrap();
    let providers = supervisor
        .resolve::<ProvidersService>(&providers_id())
        .unwrap();
    let catalog = supervisor
        .resolve::<ProviderCatalogService>(&catalog_id())
        .unwrap();
    let model = supervisor.resolve::<ModelService>(&model_id()).unwrap();
    let manager = Arc::new(
        ConnectionManager::open(
            store.clone(),
            access,
            providers.0.clone(),
            catalog.0.clone(),
        )
        .await
        .unwrap(),
    );
    supervisor
        .register(Box::new(ConnectionsPlugin(manager.clone())))
        .unwrap();
    supervisor.register(Box::new(ProvidersUiPlugin)).unwrap();
    supervisor.start().await.unwrap();
    let session = Session::new(root.into());
    let ui = Ui::new(&session, contributions, CancellationToken::new()).0;
    Harness {
        supervisor,
        ui,
        manager,
        model,
        store,
        session,
    }
}

#[derive(Debug)]
struct Call {
    connection: String,
    authorization: Option<String>,
    body: Value,
}
#[derive(Debug)]
struct Inbound {
    method: String,
    path: String,
    authorization: Option<Vec<u8>>,
}
struct Service {
    endpoint: String,
    calls: Arc<Mutex<Vec<Call>>>,
    inbound: Arc<Mutex<Vec<Inbound>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Service {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn service() -> Service {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let capture = calls.clone();
    let inbound = Arc::new(Mutex::new(Vec::new()));
    let all_requests = inbound.clone();
    let router = Router::new()
        .route("/{connection}/v1/models", get(|| async { Json(json!({"data":[{"id":"alpha"},{"id":"beta"}]})) }))
        .route("/{connection}/v1/chat/completions", post(move |Path(connection): Path<String>, headers: HeaderMap, Json(body): Json<Value>| {
            let capture = capture.clone();
            async move {
                capture.lock().unwrap().push(Call { connection: connection.clone(), authorization: headers.get("authorization").and_then(|v| v.to_str().ok()).map(str::to_owned), body });
                ([("content-type", "text/event-stream")], format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"answer-{connection}\"}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"))
            }
        }))
        // This outer layer records before route extraction/header validation,
        // including catalog requests, unmatched paths, and absent/invalid auth.
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let all_requests = all_requests.clone();
            async move {
                all_requests.lock().unwrap().push(Inbound {
                    method: request.method().to_string(),
                    path: request.uri().path().into(),
                    authorization: request.headers().get("authorization").map(|v| v.as_bytes().to_vec()),
                });
                next.run(request).await
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Service {
        endpoint,
        calls,
        inbound,
        task,
    }
}

#[tokio::test]
async fn recorder_sees_catalog_unmatched_and_rejected_requests_without_authorization() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let service = service().await;
    for (method, path, body) in [
        ("GET", "/a/v1/models", ""),
        ("GET", "/unmatched", ""),
        ("POST", "/a/v1/chat/completions", "not-json"),
        ("POST", "/a/v1/chat/completions", "{}"),
    ] {
        tokio::time::timeout(Duration::from_secs(3), async {
            let mut stream = tokio::net::TcpStream::connect(service.endpoint.trim_start_matches("http://")).await.unwrap();
            stream.write_all(format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).await.unwrap();
            assert!(!response.is_empty());
        }).await.unwrap();
    }
    let inbound = service.inbound.lock().unwrap();
    assert_eq!(
        inbound
            .iter()
            .map(|r| (r.method.as_str(), r.path.as_str()))
            .collect::<Vec<_>>(),
        [
            ("GET", "/a/v1/models"),
            ("GET", "/unmatched"),
            ("POST", "/a/v1/chat/completions"),
            ("POST", "/a/v1/chat/completions"),
        ]
    );
    assert!(inbound.iter().all(|r| r.authorization.is_none()));
    drop(inbound);
    let calls = service.calls.lock().unwrap();
    assert_eq!(
        calls.len(),
        1,
        "invalid JSON must still be visible in the outer recorder"
    );
    assert!(calls[0].authorization.is_none());
    drop(calls);
}
async fn settle(ui: &Ui) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            ui.refresh_contributions();
            if !ui.lock().workflow.as_ref().is_some_and(|w| w.is_pending()) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("UI action did not settle");
}
async fn command(ui: &Ui, name: &str) {
    assert!(ui.dispatch_command(name));
    settle(ui).await;
}
async fn submit(ui: &Ui) {
    assert!(ui.submit_workflow());
    settle(ui).await;
}
fn replace(ui: &Ui, old: &str, new: &str) {
    let mut state = ui.lock();
    let form = state.workflow.as_mut().unwrap();
    for _ in old.chars() {
        form.backspace();
    }
    form.insert(new);
    drop(state);
}
async fn login_form(h: &Harness, endpoint: &str, secret: &str, mode: SecretMode) {
    command(&h.ui, "/login").await;
    let catalog = h
        .supervisor
        .resolve::<ProviderCatalogService>(&catalog_id())
        .unwrap();
    let index = catalog
        .0
        .adapters()
        .into_iter()
        .flat_map(|(_, d)| d.authentication.into_iter().map(move |a| (d.id.clone(), a)))
        .position(|(id, a)| id == "openai" && a == AuthenticationMethod::ApiKey)
        .unwrap();
    for _ in 0..index {
        h.ui.lock().workflow.as_mut().unwrap().choice(true);
    }
    submit(&h.ui).await;
    {
        let mut state = h.ui.lock();
        let form = state.workflow.as_mut().unwrap();
        form.insert(endpoint);
        form.navigate(true);
        form.insert("alpha");
        form.navigate(true);
        form.choice(true); // explicit loopback HTTP
        form.navigate(true);
        if mode == SecretMode::Ephemeral {
            form.choice(true);
        }
        form.navigate(true);
        form.insert(secret);
        assert_public(&format!("{:?}", form.snapshot()));
        drop(state);
    }
}
async fn login(h: &Harness, endpoint: &str, secret: &str, mode: SecretMode) -> Option<uuid::Uuid> {
    let before = h.manager.list().await.unwrap();
    login_form(h, endpoint, secret, mode).await;
    submit(&h.ui).await;
    h.manager
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|r| !before.iter().any(|b| b.id == r.id))
        .map(|r| r.id)
}
async fn select(h: &Harness, id: uuid::Uuid, model: &str) {
    let records = h.manager.list().await.unwrap();
    let index = records.iter().position(|r| r.id == id).unwrap();
    command(&h.ui, "/model").await;
    for _ in 0..index {
        h.ui.lock().workflow.as_mut().unwrap().choice(true);
    }
    submit(&h.ui).await;
    replace(&h.ui, &records[index].settings.model, model);
    submit(&h.ui).await;
}
async fn logout(h: &Harness, id: uuid::Uuid) {
    let index = h
        .manager
        .list()
        .await
        .unwrap()
        .iter()
        .position(|r| r.id == id)
        .unwrap();
    command(&h.ui, "/logout").await;
    {
        let mut state = h.ui.lock();
        let form = state.workflow.as_mut().unwrap();
        for _ in 0..index {
            form.choice(true);
        }
        form.navigate(true);
        form.choice(true);
        drop(state);
    }
    submit(&h.ui).await;
}
fn request() -> ModelRequest {
    ModelRequest {
        messages: vec![Message::text(Role::User, "hello")],
        tools: vec![],
        max_tokens: 32,
    }
}
async fn answer(model: &dyn ModelProvider) -> huginn_core::Result<String> {
    tokio::time::timeout(Duration::from_secs(4), async {
        let mut stream = model.stream(request(), CancellationToken::new()).await?;
        let mut text = String::new();
        let mut finished = false;
        while let Some(delta) = stream.next().await {
            match delta? {
                ModelDelta::Text(value) => text.push_str(&value),
                ModelDelta::Finished => finished = true,
                _ => {}
            }
        }
        assert!(finished);
        Ok(text)
    })
    .await
    .expect("inference did not settle")
}
async fn assert_blocked(model: &dyn ModelProvider, service: &Service) {
    let count = service.inbound.lock().unwrap().len();
    assert!(answer(model).await.is_err());
    assert_eq!(
        service.inbound.lock().unwrap().len(),
        count,
        "blocked inference must not send any HTTP request"
    );
}
fn assert_public(value: &str) {
    for secret in [A, B] {
        assert!(!value.contains(secret), "secret leaked to public surface");
    }
}
async fn assert_surfaces(h: &Harness, root: &std::path::Path) {
    {
        let state = h.ui.lock();
        assert_public(&format!(
            "{:?}{:?}{:?}{:?}{}",
            state.committed,
            state.events,
            state.tail,
            state.editor.text,
            state
                .workflow
                .as_ref()
                .map(|w| format!("{:?}", w.snapshot()))
                .unwrap_or_default()
        ));
        drop(state);
    }
    assert_public(&format!("{:?}", h.ui.contributions.widgets()));
    assert_public(&serde_json::to_string(&h.store.load().await.unwrap()).unwrap());
    assert_public(&serde_json::to_string(&h.session).unwrap());
    // Inspect live database/WAL/SHM bytes, not just a post-checkpoint SQL query.
    // The directory listing is untrusted for path-injection purposes: rebuild
    // each read from a validated single file name inside the fixture root.
    for entry in std::fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.contains('/') && !name.contains('\\') && !name.contains(".."),
                "unexpected fixture entry: {name}"
            );
            let path = root.join(name.as_ref());
            let bytes = std::fs::read(&path).unwrap();
            for secret in [A, B] {
                assert!(
                    !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                    "secret in {}",
                    path.display()
                );
            }
        }
    }
}

#[tokio::test]
async fn ui_two_connections_select_switch_pin_revoke_and_persist_without_secret_leaks() {
    let root = tempfile::tempdir().unwrap();
    let service = service().await;
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(root.path(), secrets.clone()).await;
    // 1: Unconfigured is explicit and does not perform inference.
    assert_blocked(h.model.0.as_ref(), &service).await;
    command(&h.ui, "/providers").await;
    assert!(h.ui.lock().committed.join("\n").contains("No connections"));
    assert!(service.inbound.lock().unwrap().is_empty());
    // 2–3: Host-owned login, isolated credentials, real SQLite metadata.
    let a = login(
        &h,
        &format!("{}/a/v1", service.endpoint),
        A,
        SecretMode::Secure,
    )
    .await
    .unwrap();
    let b = login(
        &h,
        &format!("{}/b/v1", service.endpoint),
        B,
        SecretMode::Secure,
    )
    .await
    .unwrap();
    assert_eq!(secrets.values.lock().unwrap().len(), 2);
    assert_blocked(h.model.0.as_ref(), &service).await; // login must not implicitly select
    assert_surfaces(&h, root.path()).await;
    // 4–5: UI selection reaches the actual adapter; model changes need no restart.
    select(&h, a, "alpha").await;
    let pinned = h
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(answer(pinned.as_ref()).await.unwrap(), "answer-a");
    select(&h, a, "beta").await;
    assert_eq!(answer(h.model.0.as_ref()).await.unwrap(), "answer-a");
    assert_eq!(answer(pinned.as_ref()).await.unwrap(), "answer-a");
    select(&h, b, "beta").await;
    assert_eq!(answer(h.model.0.as_ref()).await.unwrap(), "answer-b");
    assert_eq!(answer(pinned.as_ref()).await.unwrap(), "answer-a");
    {
        let calls = service.calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .map(|c| (c.connection.as_str(), c.body["model"].as_str().unwrap()))
                .collect::<Vec<_>>(),
            [
                ("a", "alpha"),
                ("a", "beta"),
                ("a", "alpha"),
                ("b", "beta"),
                ("a", "alpha")
            ]
        );
        for call in calls.iter() {
            assert_eq!(
                call.authorization,
                Some(format!(
                    "Bearer {}",
                    if call.connection == "a" { A } else { B }
                ))
            );
        }
    }
    {
        let inbound = service.inbound.lock().unwrap();
        for request in inbound.iter() {
            let secret = match request.path.as_str() {
                "/a/v1/models" | "/a/v1/chat/completions" => A,
                "/b/v1/models" | "/b/v1/chat/completions" => B,
                path => panic!("unexpected HTTP request: {path}"),
            };
            assert_eq!(
                request.authorization.as_deref(),
                Some(format!("Bearer {secret}").as_bytes()),
                "catalog and inference credentials must both stay connection-local"
            );
        }
        drop(inbound);
    }
    // 6: UI revoke invalidates an old pinned turn, but never another connection.
    logout(&h, a).await;
    let count = service.inbound.lock().unwrap().len();
    assert_blocked(pinned.as_ref(), &service).await;
    assert_eq!(service.inbound.lock().unwrap().len(), count);
    assert_eq!(answer(h.model.0.as_ref()).await.unwrap(), "answer-b");
    assert_eq!(h.manager.list().await.unwrap().len(), 1);
    assert_eq!(secrets.values.lock().unwrap().len(), 1);
    h.session
        .messages
        .push(Message::text(Role::Assistant, "answer-b"));
    SqliteSessions::open(&root.path().join("sessions.db"))
        .unwrap()
        .save(&h.session)
        .await
        .unwrap();
    assert_surfaces(&h, root.path()).await;
    drop(pinned);
    drop(h);
    // 7: Fresh composition restores the saved selection and credential binding.
    let h = open(root.path(), secrets).await;
    assert_eq!(answer(h.model.0.as_ref()).await.unwrap(), "answer-b");
    assert_eq!(h.store.load().await.unwrap().selected, Some(b));
    logout(&h, b).await;
    assert_blocked(h.model.0.as_ref(), &service).await;
    assert_surfaces(&h, root.path()).await;
}

#[tokio::test]
async fn restart_locked_or_ephemeral_is_visible_fail_closed_and_never_falls_back() {
    for mode in [SecretMode::Secure, SecretMode::Ephemeral] {
        let root = tempfile::tempdir().unwrap();
        let service = service().await;
        let secure = Secrets::new(SecretMode::Secure);
        let ephemeral = Secrets::new(SecretMode::Ephemeral);
        let h = open(
            root.path(),
            Arc::new(RoutedSecrets {
                secure: secure.clone(),
                ephemeral: ephemeral.clone(),
            }),
        )
        .await;
        let id = login(&h, &format!("{}/a/v1", service.endpoint), A, mode)
            .await
            .unwrap();
        let competitor = login(
            &h,
            &format!("{}/b/v1", service.endpoint),
            B,
            SecretMode::Secure,
        )
        .await
        .unwrap();
        // Positive control: B is a genuinely usable competing endpoint/key.
        select(&h, competitor, "beta").await;
        assert_eq!(answer(h.model.0.as_ref()).await.unwrap(), "answer-b");
        select(&h, id, "alpha").await;
        assert_eq!(answer(h.model.0.as_ref()).await.unwrap(), "answer-a");
        let reference = h
            .manager
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.id == id)
            .unwrap()
            .secret
            .unwrap();
        drop(h);
        let restarted_ephemeral = if mode == SecretMode::Secure {
            secure
                .read_errors
                .lock()
                .unwrap()
                .insert(reference.clone(), ConnectionError::Locked);
            ephemeral
        } else {
            Secrets::new(SecretMode::Ephemeral)
        };
        let count = service.inbound.lock().unwrap().len();
        let h = open(
            root.path(),
            Arc::new(RoutedSecrets {
                secure: secure.clone(),
                ephemeral: restarted_ephemeral,
            }),
        )
        .await;
        let expected = if mode == SecretMode::Secure {
            ConnectionError::Locked
        } else {
            ConnectionError::MissingSecret
        };
        assert_eq!(h.manager.status(id).await, Err(expected));
        assert_eq!(h.manager.status(competitor).await, Ok(()));
        command(&h.ui, "/providers").await;
        let index = h
            .manager
            .list()
            .await
            .unwrap()
            .iter()
            .position(|r| r.id == id)
            .unwrap();
        for _ in 0..index {
            h.ui.lock().workflow.as_mut().unwrap().choice(true);
        }
        assert!(
            format!("{:?}", h.ui.lock().workflow.as_ref().unwrap().snapshot())
                .contains(&expected.to_string())
        );
        assert_blocked(h.model.0.as_ref(), &service).await;
        assert_eq!(
            service.inbound.lock().unwrap().len(),
            count,
            "unavailable selection must contact neither A nor usable B, including catalogs"
        );
        assert_eq!(h.manager.list().await.unwrap().len(), 2);
        assert_surfaces(&h, root.path()).await;
        h.ui.cancel_workflow();
        // B remains usable after restart, but only after deliberate UI selection.
        select(&h, competitor, "beta").await;
        assert_eq!(answer(h.model.0.as_ref()).await.unwrap(), "answer-b");
        assert_eq!(
            service.calls.lock().unwrap().last().unwrap().authorization,
            Some(format!("Bearer {B}"))
        );
        if mode == SecretMode::Secure {
            secure.read_errors.lock().unwrap().remove(&reference);
            select(&h, id, "beta").await;
            assert_eq!(answer(h.model.0.as_ref()).await.unwrap(), "answer-a");
        }
    }
}

#[tokio::test]
async fn failed_secret_write_rolls_back_and_cancelled_form_cannot_publish() {
    let root = tempfile::tempdir().unwrap();
    let service = service().await;
    let secrets = Secrets::new(SecretMode::Secure);
    let h = open(root.path(), secrets.clone()).await;
    secrets.fail_write.store(true, Ordering::SeqCst);
    assert!(login(
        &h,
        &format!("{}/a/v1", service.endpoint),
        A,
        SecretMode::Secure
    )
    .await
    .is_none());
    assert!(h
        .ui
        .lock()
        .committed
        .join("\n")
        .contains("Connection action failed"));
    command(&h.ui, "/providers").await;
    assert!(h.manager.list().await.unwrap().is_empty());
    // A failed secret write retains an orphan cleanup intent until reopen.
    assert_eq!(h.store.load().await.unwrap().cleanup.len(), 1);
    assert_surfaces(&h, root.path()).await;
    drop(h);
    let h = open(root.path(), secrets.clone()).await;
    assert!(h.store.load().await.unwrap().cleanup.is_empty());
    assert!(secrets.values.lock().unwrap().is_empty());
    assert_blocked(h.model.0.as_ref(), &service).await;
    // A dismissed, unsubmitted secret form cannot write or contaminate a later generation.
    login_form(
        &h,
        &format!("{}/a/v1", service.endpoint),
        A,
        SecretMode::Secure,
    )
    .await;
    h.ui.cancel_workflow();
    assert!(!h.ui.submit_workflow());
    assert!(h.manager.list().await.unwrap().is_empty());
    assert!(secrets.values.lock().unwrap().is_empty());
    let id = login(
        &h,
        &format!("{}/b/v1", service.endpoint),
        B,
        SecretMode::Secure,
    )
    .await
    .unwrap();
    select(&h, id, "beta").await;
    assert_eq!(answer(h.model.0.as_ref()).await.unwrap(), "answer-b");
    assert_eq!(secrets.values.lock().unwrap().len(), 1);
    assert_surfaces(&h, root.path()).await;
}

#[tokio::test]
async fn cancelling_pending_login_prevents_stale_completion_and_next_save_cleans_orphan() {
    let root = tempfile::tempdir().unwrap();
    let service = service().await;
    let secrets = Secrets::new(SecretMode::Secure);
    let h = open(root.path(), secrets.clone()).await;
    secrets.suspend_write.store(true, Ordering::SeqCst);
    login_form(
        &h,
        &format!("{}/a/v1", service.endpoint),
        A,
        SecretMode::Secure,
    )
    .await;
    assert!(h.ui.submit_workflow());
    tokio::time::timeout(Duration::from_secs(3), secrets.write_entered.notified())
        .await
        .unwrap();
    // Durable intent and secret side effect exist, but no public connection yet.
    assert_eq!(h.store.load().await.unwrap().cleanup.len(), 1);
    h.ui.cancel_workflow();
    command(&h.ui, "/providers").await;
    secrets.write_release.notify_one();
    assert!(h.manager.list().await.unwrap().is_empty());
    assert!(h.ui.lock().workflow.is_none());
    assert!(h
        .ui
        .lock()
        .committed
        .join("\n")
        .contains("cancelled locally"));
    let id = login(
        &h,
        &format!("{}/b/v1", service.endpoint),
        B,
        SecretMode::Secure,
    )
    .await
    .unwrap();
    select(&h, id, "beta").await;
    assert_eq!(answer(h.model.0.as_ref()).await.unwrap(), "answer-b");
    assert_eq!(h.manager.list().await.unwrap().len(), 1);
    assert!(h.store.load().await.unwrap().cleanup.is_empty());
    assert_eq!(
        secrets
            .values
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>(),
        [B]
    );
    assert_surfaces(&h, root.path()).await;
}

#[tokio::test]
async fn ui_credential_rotation_retires_pinned_authorization_until_explicit_reselection() {
    let root = tempfile::tempdir().unwrap();
    let service = service().await;
    let secrets = Secrets::new(SecretMode::Secure);
    let h = open(root.path(), secrets.clone()).await;
    let id = login(
        &h,
        &format!("{}/a/v1", service.endpoint),
        A,
        SecretMode::Secure,
    )
    .await
    .unwrap();
    select(&h, id, "alpha").await;
    let pinned = h
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(answer(pinned.as_ref()).await.unwrap(), "answer-a");
    let previous = h.manager.list().await.unwrap()[0].revision;
    command(&h.ui, "/providers").await;
    {
        let mut state = h.ui.lock();
        let form = state.workflow.as_mut().unwrap();
        form.navigate(true);
        form.choice(true); // update settings/credentials
        drop(state);
    }
    submit(&h.ui).await;
    {
        let mut state = h.ui.lock();
        let form = state.workflow.as_mut().unwrap();
        for _ in 0..4 {
            form.navigate(true);
        }
        form.insert(B);
        drop(state);
    }
    submit(&h.ui).await;
    assert_ne!(h.manager.list().await.unwrap()[0].revision, previous);
    let count = service.inbound.lock().unwrap().len();
    assert_blocked(pinned.as_ref(), &service).await;
    assert_blocked(h.model.0.as_ref(), &service).await;
    assert_eq!(service.inbound.lock().unwrap().len(), count);
    select(&h, id, "beta").await;
    assert_eq!(answer(h.model.0.as_ref()).await.unwrap(), "answer-a");
    assert_blocked(pinned.as_ref(), &service).await; // reselection cannot revive old authorization
    assert_eq!(
        service.calls.lock().unwrap().last().unwrap().authorization,
        Some(format!("Bearer {B}"))
    );
    assert_eq!(
        secrets
            .values
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>(),
        [B]
    );
    assert_surfaces(&h, root.path()).await;
}
