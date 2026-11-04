//! Replacing the production loop through the plugin path invokes only the replacement.
//!
//! This lives here rather than in `huginn-application` because the architecture guard
//! forbids the application crate from depending on the authoring SDK, tests included.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::{composition, loop_engine::LoopConfig};
use huginn_core::{
    agent_loop_id,
    kernel::{Manifest, Plugin, PluginContext, SubstitutionPolicy},
    AgentError, AgentLoop, ApprovalPolicy, ApprovalRequest, CancellationToken, Event, EventSink,
    InstructionSource, LoopService, Message, ModelDelta, ModelProvider, ModelRequest, Result, Role,
    Session, SessionInfo, SessionStore, StepId, Tool, ToolDefinition, ToolRegistry,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[derive(Default)]
struct Counts {
    model: AtomicUsize,
    plugin_runs: AtomicUsize,
}

struct CountingModel(Arc<Counts>);
#[async_trait]
impl ModelProvider for CountingModel {
    async fn stream(
        &self,
        _: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        self.0.model.fetch_add(1, Ordering::SeqCst);
        Ok(futures::stream::iter([
            Ok(ModelDelta::Text("host".into())),
            Ok(ModelDelta::Finished),
        ])
        .boxed())
    }
}

struct NoTools;
impl ToolRegistry for NoTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![]
    }
    fn get(&self, _: &str) -> Option<Arc<dyn Tool>> {
        None
    }
}

#[derive(Clone, Default)]
struct Memory(Arc<Mutex<Option<Session>>>);
#[async_trait]
impl SessionStore for Memory {
    async fn save(&self, session: &Session) -> Result<()> {
        *self.0.lock().unwrap() = Some(session.clone());
        Ok(())
    }
    async fn load(&self, id: &str) -> Result<Session> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Session::new(id.into())))
    }
    async fn list(&self) -> Result<Vec<SessionInfo>> {
        Ok(vec![])
    }
}

struct Instructions;
#[async_trait]
impl InstructionSource for Instructions {
    async fn load(&self, _: &std::path::Path) -> Result<String> {
        Ok(String::new())
    }
}

struct NoApproval;
#[async_trait]
impl ApprovalPolicy for NoApproval {
    async fn approve(&self, request: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
        Err(AgentError::Tool(format!(
            "unexpected approval {}",
            request.id
        )))
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<Event>>);
impl EventSink for Events {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

/// An alternative loop written against the contracts only.
struct PluginLoop(Arc<Counts>);
#[async_trait]
impl AgentLoop for PluginLoop {
    async fn run(
        &self,
        session: &mut Session,
        prompt: String,
        _approvals: &dyn ApprovalPolicy,
        events: &dyn EventSink,
        _cancel: CancellationToken,
    ) -> Result<()> {
        self.0.plugin_runs.fetch_add(1, Ordering::SeqCst);
        let turn = session.allocate_turn()?;
        session.events.push(Event::TurnStarted {
            session_id: session.id.clone(),
            turn,
        });
        session.messages.push(Message::text(Role::User, prompt));
        session
            .messages
            .push(Message::text(Role::Assistant, "plugin"));
        events.emit(Event::Text {
            step: StepId(1),
            text: "plugin".into(),
        });
        session.events.push(Event::TurnFinished { turn });
        Ok(())
    }
}

struct PluginLoopPlugin(Arc<Counts>);
#[async_trait]
impl Plugin for PluginLoopPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "author.alternative-loop".into(),
            provides: vec![agent_loop_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(
            agent_loop_id(),
            LoopService(Arc::new(PluginLoop(self.0.clone()))),
        )
    }
}

#[tokio::test]
async fn substituting_the_loop_runs_only_the_plugin_implementation() {
    let counts = Arc::new(Counts::default());
    let mut supervisor = composition::build(
        Arc::new(CountingModel(counts.clone())),
        Arc::new(NoTools),
        Arc::new(Memory::default()),
        LoopConfig::default(),
        Arc::new(Instructions),
    )
    .await
    .unwrap();

    assert_eq!(
        supervisor
            .provider_of(&composition::loop_id())
            .map(|record| record.plugin_id.as_str()),
        Some("core.loop"),
        "the host loop must own the service before the substitution"
    );

    supervisor
        .substitute(
            &composition::loop_id(),
            Box::new(PluginLoopPlugin(counts.clone())),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert_eq!(
        supervisor
            .provider_of(&composition::loop_id())
            .map(|record| record.plugin_id.as_str()),
        Some("author.alternative-loop"),
        "the contract id must now resolve to the plugin provider"
    );

    let slot = supervisor.resolve::<LoopService>(&agent_loop_id()).unwrap();
    let events = Events::default();
    let mut session = Session::new("/nonexistent/substitution".into());
    slot.0
        .run(
            &mut session,
            "hello".into(),
            &NoApproval,
            &events,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(counts.plugin_runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        counts.model.load(Ordering::SeqCst),
        0,
        "the replaced host loop must not be invoked as well"
    );
    assert_eq!(session.messages.last().unwrap().content, "plugin");
    assert_eq!(
        events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, Event::Text { text, .. } if text == "plugin"))
            .count(),
        1
    );
    supervisor.shutdown().await;
}

/// A turn-level guard installed after substituting the registry must be the one that runs.
#[tokio::test]
async fn a_substituted_hook_registry_is_the_one_that_runs() {
    let counts = Arc::new(Counts::default());
    let mut supervisor = composition::build(
        Arc::new(CountingModel(counts.clone())),
        Arc::new(NoTools),
        Arc::new(Memory::default()),
        LoopConfig::default(),
        Arc::new(Instructions),
    )
    .await
    .unwrap();

    supervisor
        .substitute(
            &huginn_core::hooks::inference_hooks_id(),
            Box::new(ReplacementHooks(Arc::new(
                huginn_core::hooks::InferenceHooks::default(),
            ))),
            SubstitutionPolicy::ReportUndrained,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    supervisor.register(Box::new(BlockInferencePlugin)).unwrap();
    supervisor.start().await.unwrap();

    let slot = supervisor.resolve::<LoopService>(&agent_loop_id()).unwrap();
    let mut session = Session::new("/nonexistent/substitution".into());
    let outcome = slot
        .0
        .run(
            &mut session,
            "hello".into(),
            &NoApproval,
            &Events::default(),
            CancellationToken::new(),
        )
        .await;

    assert!(
        matches!(&outcome, Err(AgentError::Model(message)) if message.contains("blocked by hook")),
        "the substituted registry must gate the call, got {outcome:?}"
    );
    assert_eq!(
        counts.model.load(Ordering::SeqCst),
        0,
        "a blocked step must not reach the model"
    );
    supervisor.shutdown().await;
}

struct BlockInference;
#[async_trait]
impl huginn_core::hooks::InferenceHook for BlockInference {
    async fn before(
        &self,
        _: huginn_core::TurnId,
        _: StepId,
        _: &huginn_core::ToolContext,
    ) -> Result<huginn_core::hooks::InferenceVerdict> {
        Ok(huginn_core::hooks::InferenceVerdict::Block(
            "installed after substitution".into(),
        ))
    }
}

struct BlockInferencePlugin;
#[async_trait]
impl Plugin for BlockInferencePlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "test.block-inference".into(),
            provides: vec![],
            requires: vec![huginn_core::hooks::inference_hooks_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let hooks = ctx
            .resolve::<huginn_core::hooks::InferenceHooksService>(
                &huginn_core::hooks::inference_hooks_id(),
            )?
            .0
            .clone();
        hooks.register(ctx, "test.block-inference".into(), Arc::new(BlockInference))
    }
}

/// Provides the same service under a new plugin id, as any alternative would.
struct ReplacementHooks(Arc<huginn_core::hooks::InferenceHooks>);
#[async_trait]
impl Plugin for ReplacementHooks {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "test.replacement-hooks".into(),
            provides: vec![huginn_core::hooks::inference_hooks_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(
            huginn_core::hooks::inference_hooks_id(),
            huginn_core::hooks::InferenceHooksService(self.0.clone()),
        )
    }
}
