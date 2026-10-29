use async_trait::async_trait;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    AgentError, ApprovalPolicy, ApprovalRequest, CancellationToken, Event, EventSink, LoopService,
    Result, Session, SessionInfo, SessionStore, Tool, ToolCall, ToolContext, ToolRegistry,
};
use huginn_runtime::tools::{
    native_tools, registry_id, NativeToolPlugin, Registry, RegistryPlugin,
};
use std::sync::{Arc, Mutex};

struct Fails {
    tools: Vec<Arc<dyn Tool>>,
}
#[async_trait]
impl Plugin for Fails {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "failure".into(),
            provides: vec![],
            requires: vec![registry_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let registry = ctx.resolve::<Registry>(&registry_id())?;
        for tool in &self.tools {
            registry.register(ctx, tool.clone())?;
        }
        assert!(
            registry.definitions().is_empty(),
            "staged tools must stay hidden"
        );
        Err(AgentError::Composition("injected failure".into()))
    }
}
#[tokio::test]
async fn failed_activation_discards_all_staged_tools_and_allows_replacement() {
    let registry = Registry::default();
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(RegistryPlugin(registry.clone())))
        .unwrap();
    supervisor.start().await.unwrap();
    supervisor
        .register(Box::new(Fails {
            tools: native_tools(1),
        }))
        .unwrap();
    assert!(supervisor.start().await.is_err());
    assert!(registry.definitions().is_empty());
    supervisor.remove("failure").await.unwrap();
    for tool in native_tools(1) {
        supervisor
            .register(Box::new(NativeToolPlugin(tool)))
            .unwrap();
    }
    supervisor.start().await.unwrap();
    assert_eq!(registry.definitions().len(), 6);
    supervisor.shutdown().await;
    assert!(registry.definitions().is_empty());
}
#[tokio::test]
async fn unload_revokes_stale_handles_and_reactivation_has_new_generation() {
    let registry = Registry::default();
    let tool = native_tools(1).remove(0);
    let name = tool.definition().name;
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(RegistryPlugin(registry.clone())))
        .unwrap();
    supervisor
        .register(Box::new(NativeToolPlugin(tool.clone())))
        .unwrap();
    supervisor.start().await.unwrap();
    let previous = registry.generation(&name).unwrap();
    let stale = registry.get(&name).unwrap();
    supervisor.remove(&format!("native.{name}")).await.unwrap();
    assert!(registry.get(&name).is_none());
    supervisor
        .register(Box::new(NativeToolPlugin(tool)))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(registry.generation(&name).unwrap() > previous);
    let ctx = ToolContext {
        selected_skills: vec![],
        workspace: ".".into(),
        cancellation: CancellationToken::new(),
    };
    assert!(stale
        .prepare(
            &ToolCall {
                id: "one".into(),
                name: name.clone(),
                arguments: serde_json::json!({})
            },
            &ctx
        )
        .await
        .is_err());
    supervisor.remove("tools.registry").await.unwrap();
    assert!(registry.definitions().is_empty());
}

#[test]
fn provider_factories_accept_extensions_and_reject_ambiguous_names() {
    use huginn_runtime::model::{MockProvider, ProviderConfig, ProviderFactories};
    let mut factories = ProviderFactories::builtin();
    factories
        .register("custom", false, |_| Ok(Arc::new(MockProvider)))
        .unwrap();
    let config = ProviderConfig {
        allow_loopback_http: false,
        base_url: None,
        model: None,
        key: None,
        timeout_secs: 1,
    };
    assert!(factories.create("custom", &config).is_ok());
    assert!(!factories.uses_credentials("custom").unwrap());
    assert!(factories
        .register("custom", true, |_| Ok(Arc::new(MockProvider)))
        .is_err());
    assert!(factories.create("missing", &config).is_err());
}

#[tokio::test]
async fn production_skill_tool_is_lifecycle_owned_and_read_only() {
    let registry = Registry::default();
    let workspace = tempfile::tempdir().unwrap();
    let sessions = Arc::new(
        huginn_runtime::sessions::SqliteSessions::open(&workspace.path().join("sessions.db"))
            .unwrap(),
    );
    let mut supervisor = huginn_runtime::composition::build_native(
        Arc::new(huginn_runtime::model::MockProvider),
        registry.clone(),
        sessions,
        huginn_runtime::loop_engine::LoopConfig::default(),
        2,
    )
    .await
    .unwrap();
    let tool = registry
        .get("load_skill")
        .expect("production must advertise loader");
    let ctx = ToolContext {
        workspace: workspace.path().into(),
        cancellation: CancellationToken::new(),
        selected_skills: vec![],
    };
    let call = ToolCall {
        id: "read".into(),
        name: "load_skill".into(),
        arguments: serde_json::json!({"identity":"unavailable"}),
    };
    assert!(tool.prepare(&call, &ctx).await.unwrap().approval.is_none());
    supervisor.remove("native.load_skill").await.unwrap();
    assert!(registry.get("load_skill").is_none());
    assert!(tool.prepare(&call, &ctx).await.is_err());
    supervisor.shutdown().await;
}

#[derive(Default)]
struct CaptureSessions(Mutex<Vec<Session>>);

#[async_trait]
impl SessionStore for CaptureSessions {
    async fn save(&self, session: &Session) -> Result<()> {
        self.0.lock().unwrap().push(session.clone());
        Ok(())
    }

    async fn load(&self, _: &str) -> Result<Session> {
        Err(AgentError::Session("load is not used in this test".into()))
    }

    async fn list(&self) -> Result<Vec<SessionInfo>> {
        Ok(Vec::new())
    }
}

struct CaptureSessionsPlugin(Arc<CaptureSessions>);

#[async_trait]
impl Plugin for CaptureSessionsPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "test.capture-sessions".into(),
            provides: vec![huginn_runtime::composition::sessions_id()],
            requires: Vec::new(),
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(
            huginn_runtime::composition::sessions_id(),
            huginn_core::SessionsService(self.0.clone()),
        )
    }
}

struct NoApproval;

#[async_trait]
impl ApprovalPolicy for NoApproval {
    async fn approve(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
        Err(AgentError::Tool("unexpected approval request".into()))
    }
}

struct IgnoreEvents;

impl EventSink for IgnoreEvents {
    fn emit(&self, _: Event) {}
}

async fn assert_retained_loop_rebinds_sessions(
    mut supervisor: Supervisor,
    store_a: Arc<CaptureSessions>,
    store_b: Arc<CaptureSessions>,
    workspace: &std::path::Path,
) {
    let loop_service: Arc<LoopService> = supervisor
        .resolve(&huginn_runtime::composition::loop_id())
        .unwrap();
    let mut session = Session::new(workspace.to_path_buf());
    let session_id = session.id.clone();
    loop_service
        .0
        .run(
            &mut session,
            "before store replacement".into(),
            &NoApproval,
            &IgnoreEvents,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(store_a
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|saved| saved.id == session_id));
    let old_store_saves = store_a.0.lock().unwrap().len();

    supervisor
        .substitute(
            &huginn_runtime::composition::sessions_id(),
            Box::new(CaptureSessionsPlugin(store_b.clone())),
            huginn_core::kernel::SubstitutionPolicy::ReportUndrained,
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap();
    loop_service
        .0
        .run(
            &mut session,
            "after store replacement".into(),
            &NoApproval,
            &IgnoreEvents,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(store_a.0.lock().unwrap().len(), old_store_saves);
    assert!(store_b
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|saved| saved.id == session_id
            && saved
                .messages
                .iter()
                .any(|message| message.content == "after store replacement")));
    supervisor.shutdown().await;
}

#[tokio::test]
async fn connections_builder_rebinds_retained_loop_to_new_sessions_store() {
    let workspace = tempfile::tempdir().unwrap();
    let store_a = Arc::new(CaptureSessions::default());
    let store_b = Arc::new(CaptureSessions::default());
    let supervisor = huginn_runtime::composition::build_native_with_connections(
        Arc::new(huginn_runtime::model::MockProvider),
        huginn_application::connections::ConnectionAccess::new(Arc::new(
            huginn_runtime::connections::HostSecrets::default(),
        )),
        Registry::default(),
        store_a.clone(),
        huginn_runtime::loop_engine::LoopConfig::default(),
        2,
    )
    .await
    .unwrap();

    assert_retained_loop_rebinds_sessions(supervisor, store_a, store_b, workspace.path()).await;
}

#[tokio::test]
async fn connections_and_guards_builder_rebinds_retained_loop_to_new_sessions_store() {
    let workspace = tempfile::tempdir().unwrap();
    let store_a = Arc::new(CaptureSessions::default());
    let store_b = Arc::new(CaptureSessions::default());
    let supervisor = huginn_runtime::composition::build_native_with_connections_and_guards(
        Arc::new(huginn_runtime::model::MockProvider),
        huginn_application::connections::ConnectionAccess::new(Arc::new(
            huginn_runtime::connections::HostSecrets::default(),
        )),
        Registry::default(),
        store_a.clone(),
        huginn_runtime::loop_engine::LoopConfig::default(),
        2,
        Arc::new(huginn_guards::Guardrails::default()),
    )
    .await
    .unwrap();

    assert_retained_loop_rebinds_sessions(supervisor, store_a, store_b, workspace.path()).await;
}

#[tokio::test]
async fn runtime_composition_build_retains_a_dynamically_rebound_loop_service() {
    let workspace = tempfile::tempdir().unwrap();
    let database = workspace.path().join("sessions.db");
    let sqlite = Arc::new(huginn_runtime::sessions::SqliteSessions::open(&database).unwrap());
    let mut supervisor = huginn_runtime::composition::build_native(
        Arc::new(huginn_runtime::model::MockProvider),
        Registry::default(),
        sqlite,
        huginn_runtime::loop_engine::LoopConfig::default(),
        2,
    )
    .await
    .unwrap();
    let loop_service = supervisor
        .resolve::<LoopService>(&huginn_runtime::composition::loop_id())
        .unwrap();
    let capture = Arc::new(CaptureSessions::default());

    supervisor
        .substitute(
            &huginn_runtime::composition::sessions_id(),
            Box::new(CaptureSessionsPlugin(capture.clone())),
            huginn_core::kernel::SubstitutionPolicy::ReportUndrained,
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap();

    let mut session = Session::new(workspace.path().into());
    loop_service
        .0
        .run(
            &mut session,
            "runtime composition rebind".into(),
            &NoApproval,
            &IgnoreEvents,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(!capture.0.lock().unwrap().is_empty());
    supervisor.shutdown().await;
}
