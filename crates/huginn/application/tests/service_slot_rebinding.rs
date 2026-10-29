use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::{
    composition,
    loop_engine::LoopConfig,
    providers::{catalog_id, providers_id},
};
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, SubstitutionPolicy},
    providers::{
        ConnectionIdentity, ConnectionPhase, ConnectionSnapshot, ModelCatalog, ModelSelection,
        ProviderAction, ProviderActionOutput, ProviderCatalog, ProviderCatalogService,
        ProviderDescriptor, ProviderManagement, ProviderOnboarding, ProviderRegistration,
        ProvidersDiagnostic, ProvidersService, SelectionMode,
    },
    AgentError, ApprovalPolicy, ApprovalRequest, CancellationToken, Event, EventSink,
    InstructionSource, LoopService, ModelDelta, ModelProvider, ModelRequest, ModelService, Result,
    Session, SessionInfo, SessionStore, Tool, ToolDefinition, ToolRegistry,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::Notify;

#[derive(Default)]
struct ModelCalls(Mutex<Vec<String>>);

struct FakeModel {
    label: &'static str,
    calls: Arc<ModelCalls>,
    block: Option<Arc<ModelBlock>>,
}

#[derive(Default)]
struct ModelBlock {
    started: AtomicBool,
    entered: Notify,
    release: Notify,
}

impl ModelBlock {
    async fn wait_once(&self) {
        if !self.started.swap(true, Ordering::SeqCst) {
            self.entered.notify_one();
            tokio::time::timeout(Duration::from_secs(5), self.release.notified())
                .await
                .expect("model block was not released");
        }
    }
}

#[async_trait]
impl ModelProvider for FakeModel {
    async fn stream(
        &self,
        request: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        let is_summary = request
            .messages
            .first()
            .is_some_and(|message| message.content.starts_with("Summarize repository work"));
        self.calls
            .0
            .lock()
            .unwrap()
            .push(format!("{}:{}", self.label, is_summary));
        if let Some(block) = &self.block {
            block.wait_once().await;
        }
        Ok(futures::stream::iter([
            Ok(ModelDelta::Text(self.label.into())),
            Ok(ModelDelta::Finished),
        ])
        .boxed())
    }
}

#[derive(Default)]
struct FakeStore(Mutex<Vec<Session>>);

#[async_trait]
impl SessionStore for FakeStore {
    async fn save(&self, session: &Session) -> Result<()> {
        self.0.lock().unwrap().push(session.clone());
        Ok(())
    }

    async fn load(&self, _: &str) -> Result<Session> {
        Err(AgentError::Session("load is not used by this test".into()))
    }

    async fn list(&self) -> Result<Vec<SessionInfo>> {
        Ok(Vec::new())
    }
}

struct Instructions;

#[async_trait]
impl InstructionSource for Instructions {
    async fn load(&self, _: &std::path::Path) -> Result<String> {
        Ok(String::new())
    }
}

struct EmptyTools;

impl ToolRegistry for EmptyTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }

    fn get(&self, _: &str) -> Option<Arc<dyn Tool>> {
        None
    }
}

struct NoApproval;

#[async_trait]
impl ApprovalPolicy for NoApproval {
    async fn approve(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
        Err(AgentError::Tool("unexpected approval request".into()))
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<Event>>);

impl EventSink for Events {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

struct ReplacementModelPlugin {
    model: Arc<dyn ModelProvider>,
}

#[async_trait]
impl Plugin for ReplacementModelPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "test.model.replacement".into(),
            provides: vec![composition::model_id(), providers_id(), catalog_id()],
            requires: Vec::new(),
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(composition::model_id(), ModelService(self.model.clone()))?;
        ctx.provide(
            providers_id(),
            ProvidersService(Arc::new(UnavailableProviders)),
        )?;
        ctx.provide(
            catalog_id(),
            ProviderCatalogService(Arc::new(UnavailableProviders)),
        )
    }
}

struct UnavailableProviders;

#[async_trait]
impl ProviderManagement for UnavailableProviders {
    fn subscribe(&self) -> tokio::sync::watch::Receiver<ConnectionSnapshot> {
        let (_, receiver) = tokio::sync::watch::channel(ConnectionSnapshot {
            source: uuid::Uuid::nil(),
            revision: 0,
            phase: ConnectionPhase::Unavailable,
            selection: None,
        });
        receiver
    }

    fn publish_selection(&self, _: &ModelSelection, _: SelectionMode) -> Result<()> {
        Err(AgentError::Provider(
            huginn_core::providers::ProviderError::Unavailable,
        ))
    }

    fn diagnostic(&self) -> Result<ProvidersDiagnostic> {
        Err(AgentError::Provider(
            huginn_core::providers::ProviderError::Unavailable,
        ))
    }

    fn clear_selection(&self) -> Result<()> {
        Err(AgentError::Provider(
            huginn_core::providers::ProviderError::Unavailable,
        ))
    }

    fn invalidate(&self, _: &ConnectionIdentity) -> Result<()> {
        Err(AgentError::Provider(
            huginn_core::providers::ProviderError::Unavailable,
        ))
    }

    async fn execute(
        &self,
        _: ProviderAction,
        _: CancellationToken,
    ) -> Result<ProviderActionOutput> {
        Err(AgentError::Provider(
            huginn_core::providers::ProviderError::Unavailable,
        ))
    }
}

#[async_trait]
impl ProviderCatalog for UnavailableProviders {
    fn adapters(&self) -> Vec<(ProviderRegistration, ProviderDescriptor)> {
        Vec::new()
    }

    fn register(
        &self,
        _: &mut PluginContext,
        _: Arc<dyn huginn_core::providers::ProviderAdapter>,
    ) -> Result<ProviderRegistration> {
        Err(AgentError::Provider(
            huginn_core::providers::ProviderError::Unavailable,
        ))
    }

    fn onboarding(&self, _: &ProviderRegistration) -> Result<ProviderOnboarding> {
        Ok(ProviderOnboarding::default())
    }

    async fn discover_models(
        &self,
        _: &ProviderRegistration,
        _: &huginn_core::connections::ConnectionMaterial,
        _: CancellationToken,
    ) -> Result<ModelCatalog> {
        Ok(ModelCatalog::Unsupported)
    }
}

struct ReplacementSessionsPlugin(Arc<dyn SessionStore>);

#[async_trait]
impl Plugin for ReplacementSessionsPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "test.sessions.replacement".into(),
            provides: vec![composition::sessions_id()],
            requires: Vec::new(),
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(
            composition::sessions_id(),
            huginn_core::SessionsService(self.0.clone()),
        )
    }
}

struct WrongSessionsPlugin;

#[async_trait]
impl Plugin for WrongSessionsPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "test.sessions.wrong-type".into(),
            provides: vec![composition::sessions_id()],
            requires: Vec::new(),
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(composition::sessions_id(), 42_u64)
    }
}

struct DrainProbe {
    entered: Arc<Notify>,
}

#[async_trait]
impl Plugin for DrainProbe {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "aaa.test.drain-probe".into(),
            provides: Vec::new(),
            requires: vec![composition::sessions_id()],
        }
    }

    async fn activate(&mut self, _: &mut PluginContext) -> Result<()> {
        Ok(())
    }

    fn supports_drain(&self) -> bool {
        true
    }

    async fn drain(&mut self) -> Result<()> {
        self.entered.notify_one();
        Ok(())
    }
}

struct ModelDrainProbe {
    entered: Arc<Notify>,
}

#[async_trait]
impl Plugin for ModelDrainProbe {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "aaa.test.model-drain-probe".into(),
            provides: Vec::new(),
            requires: vec![composition::model_id()],
        }
    }

    async fn activate(&mut self, _: &mut PluginContext) -> Result<()> {
        Ok(())
    }

    fn supports_drain(&self) -> bool {
        true
    }

    async fn drain(&mut self) -> Result<()> {
        self.entered.notify_one();
        Ok(())
    }
}

struct ResumeProbe {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl Plugin for ResumeProbe {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "zzz.test.resume-probe".into(),
            provides: Vec::new(),
            requires: vec![composition::sessions_id()],
        }
    }

    async fn activate(&mut self, _: &mut PluginContext) -> Result<()> {
        Ok(())
    }

    fn supports_drain(&self) -> bool {
        true
    }

    async fn resume(&mut self) -> Result<()> {
        self.entered.notify_one();
        tokio::time::timeout(Duration::from_secs(5), self.release.notified())
            .await
            .map_err(|_| AgentError::Composition("resume probe timed out".into()))?;
        Ok(())
    }
}

async fn add_drain_probes(
    supervisor: &mut huginn_core::kernel::Supervisor,
    drain_entered: Arc<Notify>,
    resume_entered: Arc<Notify>,
    resume_release: Arc<Notify>,
) {
    supervisor
        .register(Box::new(DrainProbe {
            entered: drain_entered,
        }))
        .unwrap();
    supervisor
        .register(Box::new(ResumeProbe {
            entered: resume_entered,
            release: resume_release,
        }))
        .unwrap();
    supervisor.start().await.unwrap();
}

async fn build_loop(
    model: Arc<dyn ModelProvider>,
    store: Arc<FakeStore>,
) -> (huginn_core::kernel::Supervisor, Arc<LoopService>) {
    let supervisor = composition::build(
        model,
        Arc::new(EmptyTools),
        store,
        LoopConfig::default(),
        Arc::new(Instructions),
    )
    .await
    .unwrap();
    let loop_service = supervisor.resolve(&composition::loop_id()).unwrap();
    (supervisor, loop_service)
}

fn assert_no_run_effects(session: &Session, events: &Events) {
    assert!(session.messages.is_empty());
    assert!(session.events.is_empty());
    assert!(!session.interrupted);
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn successful_model_substitution_waits_for_active_run_then_rebinds_same_loop() {
    let block = Arc::new(ModelBlock::default());
    let calls_a = Arc::new(ModelCalls::default());
    let calls_b = Arc::new(ModelCalls::default());
    let store = Arc::new(FakeStore::default());
    let (mut supervisor, loop_service) = build_loop(
        Arc::new(FakeModel {
            label: "model-A",
            calls: calls_a.clone(),
            block: Some(block.clone()),
        }),
        store,
    )
    .await;
    let drain_entered = Arc::new(Notify::new());
    supervisor
        .register(Box::new(ModelDrainProbe {
            entered: drain_entered.clone(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();

    let active_loop = loop_service.0.clone();
    let active_run = tokio::spawn(async move {
        let mut session = Session::new("/nonexistent/active-model-rebind".into());
        let result = active_loop
            .run(
                &mut session,
                "in flight on A".into(),
                &NoApproval,
                &Events::default(),
                CancellationToken::new(),
            )
            .await;
        (session, result)
    });
    tokio::time::timeout(Duration::from_secs(1), block.entered.notified())
        .await
        .unwrap();

    let replacement = Box::new(ReplacementModelPlugin {
        model: Arc::new(FakeModel {
            label: "model-B",
            calls: calls_b.clone(),
            block: None,
        }),
    });
    let substitution = tokio::spawn(async move {
        let result = supervisor
            .substitute(
                &composition::model_id(),
                replacement,
                SubstitutionPolicy::ReportUndrained,
                Duration::from_secs(2),
            )
            .await;
        (supervisor, result)
    });

    tokio::time::timeout(Duration::from_secs(1), drain_entered.notified())
        .await
        .unwrap();
    assert!(!active_run.is_finished());
    assert!(!substitution.is_finished());

    block.release.notify_one();
    let (active_session, active_result) = active_run.await.unwrap();
    active_result.unwrap();
    assert_eq!(active_session.messages.last().unwrap().content, "model-A");

    let (mut supervisor, result) = substitution.await.unwrap();
    result.unwrap();
    loop_service
        .0
        .run(
            &mut Session::new("/nonexistent/next-model-rebind".into()),
            "next operation on B".into(),
            &NoApproval,
            &Events::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(!calls_b.0.lock().unwrap().is_empty());
    supervisor.shutdown().await;
}

#[tokio::test]
async fn drain_timeout_does_not_cancel_run_and_admission_reopens_after_resume() {
    let calls = Arc::new(ModelCalls::default());
    let block = Arc::new(ModelBlock::default());
    let store = Arc::new(FakeStore::default());
    let (mut supervisor, loop_service) = build_loop(
        Arc::new(FakeModel {
            label: "model-A",
            calls: calls.clone(),
            block: Some(block.clone()),
        }),
        store.clone(),
    )
    .await;
    let drain_entered = Arc::new(Notify::new());
    let resume_entered = Arc::new(Notify::new());
    let resume_release = Arc::new(Notify::new());
    add_drain_probes(
        &mut supervisor,
        drain_entered.clone(),
        resume_entered.clone(),
        resume_release.clone(),
    )
    .await;

    let active_loop = loop_service.0.clone();
    let active_run = tokio::spawn(async move {
        let mut session = Session::new("/nonexistent/active-run".into());
        let events = Events::default();
        let result = active_loop
            .run(
                &mut session,
                "active".into(),
                &NoApproval,
                &events,
                CancellationToken::new(),
            )
            .await;
        (session, events, result)
    });
    tokio::time::timeout(Duration::from_secs(1), block.entered.notified())
        .await
        .unwrap();

    let substitute = tokio::spawn(async move {
        let result = supervisor
            .substitute(
                &composition::sessions_id(),
                Box::new(ReplacementSessionsPlugin(Arc::new(FakeStore::default()))),
                SubstitutionPolicy::ReportUndrained,
                Duration::from_millis(100),
            )
            .await;
        (supervisor, result)
    });
    tokio::time::timeout(Duration::from_secs(1), drain_entered.notified())
        .await
        .unwrap();

    let saves_before_rejection = store.0.lock().unwrap().len();
    let mut during_drain = Session::new("/nonexistent/rejected-drain".into());
    let drain_events = Events::default();
    assert!(loop_service
        .0
        .run(
            &mut during_drain,
            "must be rejected".into(),
            &NoApproval,
            &drain_events,
            CancellationToken::new(),
        )
        .await
        .is_err());
    assert_no_run_effects(&during_drain, &drain_events);
    assert_eq!(store.0.lock().unwrap().len(), saves_before_rejection);

    let (mut supervisor, substitution_result) = substitute.await.unwrap();
    assert!(substitution_result.is_err());
    assert!(!active_run.is_finished());

    let mut resumed = Session::new("/nonexistent/resumed-after-abort".into());
    loop_service
        .0
        .run(
            &mut resumed,
            "after abort".into(),
            &NoApproval,
            &Events::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(resumed.messages.last().unwrap().content, "model-A");
    assert_eq!(calls.0.lock().unwrap().len(), 2);

    block.release.notify_one();
    let (active_session, _, active_result) = active_run.await.unwrap();
    active_result.unwrap();
    assert_eq!(active_session.messages.last().unwrap().content, "model-A");
    supervisor.shutdown().await;
}

#[tokio::test]
async fn second_slot_failure_has_no_effects_and_releases_first_lease() {
    let calls_a = Arc::new(ModelCalls::default());
    let store_a = Arc::new(FakeStore::default());
    let (mut supervisor, loop_service) = build_loop(
        Arc::new(FakeModel {
            label: "model-A",
            calls: calls_a.clone(),
            block: None,
        }),
        store_a.clone(),
    )
    .await;
    supervisor
        .substitute(
            &composition::sessions_id(),
            Box::new(WrongSessionsPlugin),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();

    let mut session = Session::new("/nonexistent/second-lease-failure".into());
    let events = Events::default();
    assert!(loop_service
        .0
        .run(
            &mut session,
            "no effects".into(),
            &NoApproval,
            &events,
            CancellationToken::new(),
        )
        .await
        .is_err());
    assert_no_run_effects(&session, &events);
    assert!(calls_a.0.lock().unwrap().is_empty());
    assert!(store_a.0.lock().unwrap().is_empty());

    let calls_b = Arc::new(ModelCalls::default());
    supervisor
        .substitute(
            &composition::model_id(),
            Box::new(ReplacementModelPlugin {
                model: Arc::new(FakeModel {
                    label: "model-B",
                    calls: calls_b.clone(),
                    block: None,
                }),
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_millis(100),
        )
        .await
        .unwrap();
    let store_b = Arc::new(FakeStore::default());
    supervisor
        .substitute(
            &composition::sessions_id(),
            Box::new(ReplacementSessionsPlugin(store_b.clone())),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_millis(100),
        )
        .await
        .unwrap();

    loop_service
        .0
        .run(
            &mut session,
            "recovered".into(),
            &NoApproval,
            &Events::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(session.messages.last().unwrap().content, "model-B");
    assert!(!calls_b.0.lock().unwrap().is_empty());
    assert!(!store_b.0.lock().unwrap().is_empty());
    supervisor.shutdown().await;
}

#[tokio::test]
async fn active_operation_saves_only_to_old_store_and_next_run_uses_new_store() {
    let block = Arc::new(ModelBlock::default());
    let model_calls = Arc::new(ModelCalls::default());
    let store_a = Arc::new(FakeStore::default());
    let store_b = Arc::new(FakeStore::default());
    let (mut supervisor, loop_service) = build_loop(
        Arc::new(FakeModel {
            label: "model-A",
            calls: model_calls.clone(),
            block: Some(block.clone()),
        }),
        store_a.clone(),
    )
    .await;
    let drain_entered = Arc::new(Notify::new());
    let resume_entered = Arc::new(Notify::new());
    let resume_release = Arc::new(Notify::new());
    add_drain_probes(
        &mut supervisor,
        drain_entered.clone(),
        resume_entered.clone(),
        resume_release.clone(),
    )
    .await;

    let operation_loop = loop_service.0.clone();
    let operation = tokio::spawn(async move {
        let mut session = Session::new("/nonexistent/multi-save-old-store".into());
        let id = session.id.clone();
        let result = operation_loop
            .run(
                &mut session,
                "active save sequence".into(),
                &NoApproval,
                &Events::default(),
                CancellationToken::new(),
            )
            .await;
        (id, session, result)
    });
    tokio::time::timeout(Duration::from_secs(1), block.entered.notified())
        .await
        .unwrap();

    let store_b_for_substitution = store_b.clone();
    let substitute = tokio::spawn(async move {
        let result = supervisor
            .substitute(
                &composition::sessions_id(),
                Box::new(ReplacementSessionsPlugin(store_b_for_substitution)),
                SubstitutionPolicy::ReportUndrained,
                Duration::from_secs(2),
            )
            .await;
        (supervisor, result)
    });
    tokio::time::timeout(Duration::from_secs(1), drain_entered.notified())
        .await
        .unwrap();

    block.release.notify_one();
    let (active_id, active_session, active_result) = operation.await.unwrap();
    active_result.unwrap();
    assert!(
        store_a
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|saved| saved.id == active_id)
            .count()
            >= 4
    );
    assert!(store_b.0.lock().unwrap().is_empty());

    tokio::time::timeout(Duration::from_secs(1), resume_entered.notified())
        .await
        .unwrap();
    let saves_before_rejection = store_a.0.lock().unwrap().len();
    let mut during_resume = Session::new("/nonexistent/rejected-before-resume".into());
    let events = Events::default();
    assert!(loop_service
        .0
        .run(
            &mut during_resume,
            "must wait for resume".into(),
            &NoApproval,
            &events,
            CancellationToken::new(),
        )
        .await
        .is_err());
    assert_no_run_effects(&during_resume, &events);
    assert_eq!(store_a.0.lock().unwrap().len(), saves_before_rejection);

    resume_release.notify_one();
    let (mut supervisor, result) = substitute.await.unwrap();
    result.unwrap();
    assert!(store_b
        .0
        .lock()
        .unwrap()
        .iter()
        .all(|saved| saved.id != active_id));

    let mut next_session = Session::new("/nonexistent/new-store".into());
    let next_id = next_session.id.clone();
    loop_service
        .0
        .run(
            &mut next_session,
            "next operation".into(),
            &NoApproval,
            &Events::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(
        store_b
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|saved| saved.id == next_id)
            .count()
            >= 4
    );
    assert!(store_a
        .0
        .lock()
        .unwrap()
        .iter()
        .all(|saved| saved.id != next_id));
    assert_eq!(active_session.id, active_id);
    assert!(!model_calls.0.lock().unwrap().is_empty());
    supervisor.shutdown().await;
}

#[tokio::test]
async fn retained_loop_service_uses_replaced_model_and_store_for_run_and_compact() {
    let calls_a = Arc::new(ModelCalls::default());
    let calls_b = Arc::new(ModelCalls::default());
    let store_a = Arc::new(FakeStore::default());
    let store_b = Arc::new(FakeStore::default());
    let mut supervisor = composition::build(
        Arc::new(FakeModel {
            label: "model-A",
            calls: calls_a.clone(),
            block: None,
        }),
        Arc::new(EmptyTools),
        store_a.clone(),
        LoopConfig::default(),
        Arc::new(Instructions),
    )
    .await
    .unwrap();
    let loop_service: Arc<LoopService> = supervisor.resolve(&composition::loop_id()).unwrap();
    let mut session = Session::new("/nonexistent/service-slot-rebinding".into());
    let events = Events::default();

    loop_service
        .0
        .run(
            &mut session,
            "first turn".into(),
            &NoApproval,
            &events,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(session.messages.last().unwrap().content, "model-A");

    supervisor
        .substitute(
            &composition::model_id(),
            Box::new(ReplacementModelPlugin {
                model: Arc::new(FakeModel {
                    label: "model-B",
                    calls: calls_b.clone(),
                    block: None,
                }),
            }),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    supervisor
        .substitute(
            &composition::sessions_id(),
            Box::new(ReplacementSessionsPlugin(store_b.clone())),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();

    loop_service
        .0
        .run(
            &mut session,
            "second turn".into(),
            &NoApproval,
            &events,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(session.messages.last().unwrap().content, "model-B");
    loop_service
        .0
        .compact(&mut session, &events, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(session.summary.as_deref(), Some("model-B"));
    assert!(!calls_b.0.lock().unwrap().is_empty());
    assert!(store_a
        .0
        .lock()
        .unwrap()
        .iter()
        .all(|saved| saved.summary.is_none()));
    assert!(store_b
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|saved| { saved.summary.as_deref() == Some("model-B") }));
    supervisor.shutdown().await;
}
