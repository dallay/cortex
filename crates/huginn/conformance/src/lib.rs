//! Behavioral and security conformance suite for `AgentLoop` implementations.
//!
//! The suite drives an implementation through the public contracts only and reports one
//! outcome per case. Passing it is the certificate: two different loops that pass the
//! same cases honor the same host boundaries.
//!
//! The suite also ships a deliberately broken loop (`broken::ReplayingLoop`) that must
//! fail. A suite that cannot fail certifies nothing.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::{composition, loop_engine::LoopConfig};
use huginn_core::{
    hooks::{tool_hooks_id, ToolHook, ToolHooks, ToolHooksService},
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    AgentError, AgentLoop, ApprovalPolicy, ApprovalRequest, CancellationToken, Event, EventSink,
    InstructionSource, ModelDelta, ModelProvider, ModelRequest, PreparedAction, Result, Role,
    Session, SessionInfo, SessionStore, StepId, Tool, ToolCall, ToolContext, ToolDefinition,
    ToolRegistry,
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

/// One scripted model response.
#[derive(Clone)]
pub enum Reply {
    Text(&'static str),
    ToolCalls(Vec<ToolCall>),
    /// Never yields; only cancellation ends the step.
    Hang,
}

/// The only effect any case is allowed to observe.
pub fn write_call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "write".into(),
        arguments: serde_json::json!({ "body": "x" }),
    }
}

pub struct ScriptModel {
    replies: Mutex<VecDeque<Reply>>,
    calls: AtomicUsize,
}
impl ScriptModel {
    pub fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            calls: AtomicUsize::new(0),
        }
    }
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ModelProvider for ScriptModel {
    async fn stream(
        &self,
        _: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Reply::Text("done"));
        let deltas: BoxStream<'static, Result<ModelDelta>> = match reply {
            Reply::Hang => futures::stream::pending().boxed(),
            Reply::Text(text) => {
                futures::stream::iter([Ok(ModelDelta::Text(text.into())), Ok(ModelDelta::Finished)])
                    .boxed()
            }
            Reply::ToolCalls(calls) => futures::stream::iter(
                calls
                    .into_iter()
                    .map(|call| Ok(ModelDelta::ToolCall(call)))
                    .chain(std::iter::once(Ok(ModelDelta::Finished))),
            )
            .boxed(),
        };
        Ok(deltas)
    }
}

/// Counts every attempt to reach an effect, before and after approval.
#[derive(Default)]
pub struct Effects {
    pub prepared: AtomicUsize,
    pub executed: AtomicUsize,
}
impl Effects {
    pub fn prepared(&self) -> usize {
        self.prepared.load(Ordering::SeqCst)
    }
    pub fn executed(&self) -> usize {
        self.executed.load(Ordering::SeqCst)
    }
}

struct WriteTool {
    effects: Arc<Effects>,
    target: PathBuf,
    cancel_after_execute: Option<CancellationToken>,
}
#[async_trait]
impl Tool for WriteTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write".into(),
            description: "writes one file".into(),
            input_schema: serde_json::json!({}),
        }
    }
    async fn prepare(&self, call: &ToolCall, _: &ToolContext) -> Result<PreparedAction> {
        self.effects.prepared.fetch_add(1, Ordering::SeqCst);
        Ok(PreparedAction {
            approval: Some(ApprovalRequest {
                id: call.id.clone(),
                action: "native.write".into(),
                preview: "write one file".into(),
            }),
            payload: serde_json::json!({}),
        })
    }
    async fn execute(&self, _: PreparedAction, _: &ToolContext) -> Result<String> {
        self.effects.executed.fetch_add(1, Ordering::SeqCst);
        std::fs::write(&self.target, "effect")?;
        if let Some(cancel) = &self.cancel_after_execute {
            cancel.cancel();
        }
        Ok("written".into())
    }
}

struct EffectsRegistry(Arc<WriteTool>);
impl ToolRegistry for EffectsRegistry {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![self.0.definition()]
    }
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        (name == "write").then(|| self.0.clone() as Arc<dyn Tool>)
    }
}

/// In-memory session store that can be reloaded, to model a fresh process.
#[derive(Clone, Default)]
pub struct MemorySessions(Arc<Mutex<Option<Session>>>);
#[async_trait]
impl SessionStore for MemorySessions {
    async fn save(&self, session: &Session) -> Result<()> {
        *self.0.lock().unwrap() = Some(session.clone());
        Ok(())
    }
    async fn load(&self, id: &str) -> Result<Session> {
        self.0
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| AgentError::Session(format!("no session {id}")))
    }
    async fn list(&self) -> Result<Vec<SessionInfo>> {
        Ok(vec![])
    }
}
impl MemorySessions {
    pub fn snapshot(&self) -> Option<Session> {
        self.0.lock().unwrap().clone()
    }
}

struct Instructions(&'static str);
#[async_trait]
impl InstructionSource for Instructions {
    async fn load(&self, _: &Path) -> Result<String> {
        Ok(self.0.to_string())
    }
}

/// Records the host decision a loop asked for, so a case can prove it was asked once.
#[derive(Default)]
pub struct Approvals {
    pub prompts: AtomicUsize,
}
struct Approve {
    allow: bool,
    counts: Arc<Approvals>,
}
#[async_trait]
impl ApprovalPolicy for Approve {
    async fn approve(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
        self.counts.prompts.fetch_add(1, Ordering::SeqCst);
        Ok(self.allow)
    }
}

#[derive(Default)]
pub struct Events(Mutex<Vec<Event>>);
impl EventSink for Events {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}
impl Events {
    pub fn snapshot(&self) -> Vec<Event> {
        self.0.lock().unwrap().clone()
    }
}

struct HookPlugin(Arc<dyn ToolHook>);
#[async_trait]
impl Plugin for HookPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "conformance.hook".into(),
            provides: vec![],
            requires: vec![tool_hooks_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let hooks = ctx.resolve::<ToolHooksService>(&tool_hooks_id())?.0.clone();
        hooks.register(ctx, "conformance.hook".into(), self.0.clone())
    }
}

/// Everything a loop is allowed to receive from the host, plus the effects it must not
/// reach without a host decision.
pub struct Host {
    pub script: Arc<ScriptModel>,
    pub effects: Arc<Effects>,
    pub sessions: Arc<MemorySessions>,
    pub approvals: Arc<Approvals>,
    pub hooks: Arc<ToolHooks>,
    pub workspace: PathBuf,
    pub target: PathBuf,
    pub config: LoopConfig,
    tools: Arc<dyn ToolRegistry>,
    _dir: tempfile::TempDir,
    _supervisor: Supervisor,
}

impl Host {
    pub async fn new(script: Vec<Reply>) -> Self {
        Self::assemble(script, None, None).await
    }
    /// A host whose only hook blocks every write before any preview exists.
    pub async fn with_blocking_hook(script: Vec<Reply>) -> Self {
        Self::assemble(script, Some(Arc::new(BlockWrites)), None).await
    }
    /// A host that cancels the turn from inside the first executed effect.
    pub async fn cancelling_after_first_effect(
        script: Vec<Reply>,
        cancel: CancellationToken,
    ) -> Self {
        Self::assemble(script, None, Some(cancel)).await
    }

    async fn assemble(
        script: Vec<Reply>,
        hook: Option<Arc<dyn ToolHook>>,
        cancel_after_execute: Option<CancellationToken>,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_path_buf();
        let target = workspace.join("effect.txt");
        let effects = Arc::new(Effects::default());
        let tools: Arc<dyn ToolRegistry> = Arc::new(EffectsRegistry(Arc::new(WriteTool {
            effects: effects.clone(),
            target: target.clone(),
            cancel_after_execute,
        })));
        let mut supervisor = Supervisor::default();
        supervisor
            .register(Box::new(composition::HooksPlugin(Arc::default())))
            .unwrap();
        if let Some(hook) = hook {
            supervisor.register(Box::new(HookPlugin(hook))).unwrap();
        }
        supervisor.start().await.unwrap();
        let hooks = supervisor
            .resolve::<ToolHooksService>(&tool_hooks_id())
            .unwrap()
            .0
            .clone();
        Self {
            script: Arc::new(ScriptModel::new(script)),
            effects,
            sessions: Arc::new(MemorySessions::default()),
            approvals: Arc::new(Approvals::default()),
            hooks,
            workspace,
            target,
            config: LoopConfig::default(),
            tools,
            _dir: dir,
            _supervisor: supervisor,
        }
    }

    pub fn tools(&self) -> Arc<dyn ToolRegistry> {
        self.tools.clone()
    }
    /// The production loop config, with the host's hook registry attached.
    pub fn loop_config(&self) -> LoopConfig {
        LoopConfig {
            hooks: Some(self.hooks.clone()),
            ..self.config.clone()
        }
    }
    pub fn approval(&self, allow: bool) -> Arc<dyn ApprovalPolicy> {
        Arc::new(Approve {
            allow,
            counts: self.approvals.clone(),
        })
    }
    pub fn instructions(&self) -> Arc<dyn InstructionSource> {
        Arc::new(Instructions(""))
    }
}

/// A hook that must stop every write before preparation.
struct BlockWrites;
#[async_trait]
impl ToolHook for BlockWrites {
    async fn before(
        &self,
        call: &ToolCall,
        _: &ToolContext,
    ) -> Result<huginn_core::hooks::ToolVerdict> {
        Ok(if call.name == "write" {
            huginn_core::hooks::ToolVerdict::Block("writes are disabled".into())
        } else {
            huginn_core::hooks::ToolVerdict::Continue
        })
    }
}

/// What an implementation claims it supports, so a case is skipped rather than faked.
#[derive(Clone, Copy)]
pub struct Capabilities {
    pub compaction: bool,
}
impl Capabilities {
    /// `AgentLoop::compact` has a default error, so compaction is optional.
    pub const STREAMING_ONLY: Self = Self { compaction: false };
    pub const FULL: Self = Self { compaction: true };
}

/// One loop implementation under certification.
pub struct UnderTest {
    pub name: String,
    pub capabilities: Capabilities,
    build: Box<dyn Fn(&Host) -> Arc<dyn AgentLoop>>,
}
impl UnderTest {
    pub fn new(
        name: impl Into<String>,
        capabilities: Capabilities,
        build: impl Fn(&Host) -> Arc<dyn AgentLoop> + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            capabilities,
            build: Box::new(build),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct CaseOutcome {
    pub case: &'static str,
    pub status: Status,
    pub detail: String,
}

#[derive(Debug, Default)]
pub struct Report {
    pub outcomes: Vec<CaseOutcome>,
}
impl Report {
    pub fn failures(&self) -> Vec<&CaseOutcome> {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.status == Status::Failed)
            .collect()
    }
    pub fn failed(&self, case: &str) -> bool {
        self.outcomes
            .iter()
            .any(|outcome| outcome.case == case && outcome.status == Status::Failed)
    }
    pub fn passed(&self, case: &str) -> bool {
        self.outcomes
            .iter()
            .any(|outcome| outcome.case == case && outcome.status == Status::Passed)
    }
    pub fn describe(&self) -> String {
        self.outcomes
            .iter()
            .map(|outcome| {
                format!(
                    "  {:<36} {:?} {}",
                    outcome.case, outcome.status, outcome.detail
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Runs every case against one implementation. Each case gets a fresh host.
pub async fn run(under_test: &UnderTest) -> Report {
    let mut report = Report::default();
    report.outcomes.push(
        guarded(
            "streaming_lifecycle_order",
            streaming_lifecycle_order(under_test).await,
        )
        .await,
    );
    report.outcomes.push(
        guarded(
            "cancel_before_model_stream",
            cancel_before_model_stream(under_test).await,
        )
        .await,
    );
    report.outcomes.push(
        guarded(
            "cancel_inside_model_stream",
            cancel_inside_model_stream(under_test).await,
        )
        .await,
    );
    report.outcomes.push(
        guarded(
            "denial_performs_no_effect",
            denial_performs_no_effect(under_test).await,
        )
        .await,
    );
    report
}

async fn guarded(name: &'static str, outcome: std::result::Result<(), String>) -> CaseOutcome {
    match outcome {
        Ok(()) => CaseOutcome {
            case: name,
            status: Status::Passed,
            detail: String::new(),
        },
        Err(detail) => CaseOutcome {
            case: name,
            status: Status::Failed,
            detail,
        },
    }
}

async fn streaming_lifecycle_order(under_test: &UnderTest) -> std::result::Result<(), String> {
    let host = Host::new(vec![Reply::Text("answer")]).await;
    let implementation = (under_test.build)(&host);
    let events = Events::default();
    let approval = host.approval(true);
    let mut session = Session::new(host.workspace.clone());
    implementation
        .run(
            &mut session,
            "hi".into(),
            &*approval,
            &events,
            CancellationToken::new(),
        )
        .await
        .map_err(|error| format!("run failed: {error}"))?;

    let turn = match session.events.first() {
        Some(Event::TurnStarted { turn, .. }) => *turn,
        other => {
            return Err(format!(
                "the turn must open with TurnStarted, got {other:?}"
            ))
        }
    };
    match session.events.last() {
        Some(Event::TurnFinished { turn: finished }) if *finished == turn => {}
        other => {
            return Err(format!(
                "the turn must close with its own TurnFinished, got {other:?}"
            ))
        }
    }
    let text_steps: Vec<StepId> = events
        .snapshot()
        .iter()
        .filter_map(|event| match event {
            Event::Text { step, .. } => Some(*step),
            _ => None,
        })
        .collect();
    if text_steps != vec![StepId(1)] {
        return Err(format!(
            "the text of the first model call must be step 1, got {text_steps:?}"
        ));
    }
    Ok(())
}

async fn cancel_before_model_stream(under_test: &UnderTest) -> std::result::Result<(), String> {
    let host = Host::new(vec![Reply::Text("never")]).await;
    let implementation = (under_test.build)(&host);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let approval = host.approval(true);
    let mut session = Session::new(host.workspace.clone());
    let outcome = implementation
        .run(
            &mut session,
            "hi".into(),
            &*approval,
            &Events::default(),
            cancel,
        )
        .await;
    if !matches!(outcome, Err(AgentError::Cancelled)) {
        return Err(format!(
            "a cancelled turn must fail as Cancelled, got {outcome:?}"
        ));
    }
    if host.script.calls() != 0 {
        return Err(format!(
            "the model must not be reached, it was called {} times",
            host.script.calls()
        ));
    }
    if !session.interrupted {
        return Err("a failed turn must stay marked interrupted".into());
    }
    if session
        .events
        .iter()
        .any(|event| matches!(event, Event::TurnFinished { .. }))
    {
        return Err("a cancelled turn must not report completion".into());
    }
    Ok(())
}

async fn cancel_inside_model_stream(under_test: &UnderTest) -> std::result::Result<(), String> {
    let host = Host::new(vec![Reply::Hang]).await;
    let implementation = (under_test.build)(&host);
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        trigger.cancel();
    });
    let approval = host.approval(true);
    let mut session = Session::new(host.workspace.clone());
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        implementation.run(
            &mut session,
            "hi".into(),
            &*approval,
            &Events::default(),
            cancel,
        ),
    )
    .await
    .map_err(|_| "cancellation did not interrupt the model stream".to_string())?;
    if !matches!(outcome, Err(AgentError::Cancelled)) {
        return Err(format!(
            "a cancelled stream must fail as Cancelled, got {outcome:?}"
        ));
    }
    Ok(())
}

async fn denial_performs_no_effect(under_test: &UnderTest) -> std::result::Result<(), String> {
    let host = Host::new(vec![
        Reply::ToolCalls(vec![write_call("c1")]),
        Reply::Text("done"),
    ])
    .await;
    let implementation = (under_test.build)(&host);
    let approval = host.approval(false);
    let mut session = Session::new(host.workspace.clone());
    implementation
        .run(
            &mut session,
            "go".into(),
            &*approval,
            &Events::default(),
            CancellationToken::new(),
        )
        .await
        .map_err(|error| format!("a denial is a tool error, not a turn failure: {error}"))?;

    if host.effects.executed() != 0 {
        return Err("a denied action must not execute".into());
    }
    if host.target.exists() {
        return Err("a denied action must leave no observable effect".into());
    }
    if host.approvals.prompts.load(Ordering::SeqCst) != 1 {
        return Err("the host must be asked exactly once".into());
    }
    let denial_visible = session.messages.iter().any(|message| {
        message.role == Role::Tool
            && message.tool_call_id.as_deref().is_some_and(|id| id == "c1")
            && message.content.contains("denied")
    });
    if !denial_visible {
        return Err("the model must see the denial as the result of that call".into());
    }
    Ok(())
}
