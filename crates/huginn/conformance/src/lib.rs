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
    hooks::{
        inference_hooks_id, tool_hooks_id, InferenceHook, InferenceHooks, InferenceHooksService,
        InferenceVerdict, ToolHook, ToolHooks, ToolHooksService,
    },
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    AgentError, AgentLoop, ApprovalPolicy, ApprovalRequest, CancellationToken, Event, EventSink,
    InstructionSource, Message, ModelDelta, ModelProvider, ModelRequest, PreparedAction, Result,
    Role, Session, SessionInfo, SessionStore, StepId, Tool, ToolCall, ToolContext, ToolDefinition,
    ToolRegistry,
};
use std::{
    collections::VecDeque,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
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
    requests: Mutex<Vec<ModelRequest>>,
    usage: (u64, u64),
}
impl ScriptModel {
    pub fn new(replies: Vec<Reply>) -> Self {
        Self::reporting_usage(replies, (0, 0))
    }
    /// Reports the same usage for every call, so a turn budget has something to sum.
    pub fn reporting_usage(replies: Vec<Reply>, usage: (u64, u64)) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            calls: AtomicUsize::new(0),
            requests: Mutex::new(vec![]),
            usage,
        }
    }
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    /// Every request the loop actually sent, in order.
    pub fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ModelProvider for ScriptModel {
    async fn stream(
        &self,
        request: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(request);
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Reply::Text("done"));
        let usage = ModelDelta::Usage {
            input_tokens: self.usage.0,
            output_tokens: self.usage.1,
        };
        let deltas: BoxStream<'static, Result<ModelDelta>> = match reply {
            Reply::Hang => futures::stream::pending().boxed(),
            Reply::Text(text) => futures::stream::iter([
                Ok(ModelDelta::Text(text.into())),
                Ok(usage),
                Ok(ModelDelta::Finished),
            ])
            .boxed(),
            Reply::ToolCalls(calls) => futures::stream::iter(
                calls
                    .into_iter()
                    .map(|call| Ok(ModelDelta::ToolCall(call)))
                    .chain(std::iter::once(Ok(usage)))
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
    steer_after_execute: Option<Arc<huginn_core::SteeringQueue>>,
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
        if let Some(steering) = &self.steer_after_execute {
            steering.push("steer: narrow the change")?;
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

/// Counts reads and returns different text each time, so drift is observable.
#[derive(Default)]
pub struct ChangingInstructions(AtomicUsize);
impl ChangingInstructions {
    pub fn loads(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}
#[async_trait]
impl InstructionSource for ChangingInstructions {
    async fn load(&self, _: &Path) -> Result<String> {
        let read = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(format!("instructions v{read}"))
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

/// Wraps a host hook as a plugin, for implementations that build their own supervisor
/// and therefore own the tool-hook registry instead of accepting `LoopConfig::hooks`.
pub fn hook_plugin(hook: Arc<dyn ToolHook>) -> Box<dyn Plugin> {
    Box::new(HookPlugin(hook))
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

/// Wraps a turn-level hook as a plugin, for hosts that own that registry.
pub fn inference_hook_plugin(hook: Arc<dyn InferenceHook>) -> Box<dyn Plugin> {
    Box::new(InferenceHookPlugin(hook))
}

struct InferenceHookPlugin(Arc<dyn InferenceHook>);
#[async_trait]
impl Plugin for InferenceHookPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "conformance.inference-hook".into(),
            provides: vec![],
            requires: vec![inference_hooks_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let hooks = ctx
            .resolve::<InferenceHooksService>(&inference_hooks_id())?
            .0
            .clone();
        hooks.register(ctx, "conformance.inference-hook".into(), self.0.clone())
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
    pub inference_hooks: Arc<InferenceHooks>,
    pub steering: Arc<huginn_core::SteeringQueue>,
    instructions: Arc<dyn InstructionSource>,
    pub workspace: PathBuf,
    pub target: PathBuf,
    pub config: LoopConfig,
    hook: Option<Arc<dyn ToolHook>>,
    inference_hook: Option<Arc<dyn InferenceHook>>,
    tools: Arc<dyn ToolRegistry>,
    _dir: tempfile::TempDir,
    _supervisor: Supervisor,
}

impl Host {
    pub async fn new(script: Vec<Reply>) -> Self {
        Self::assemble(script, None, None, None, (0, 0), Arc::new(Instructions(""))).await
    }
    /// A host whose only hook blocks every write before any preview exists.
    pub async fn with_blocking_hook(script: Vec<Reply>) -> Self {
        Self::assemble(
            script,
            Some(Arc::new(BlockWrites)),
            None,
            None,
            (0, 0),
            Arc::new(Instructions("")),
        )
        .await
    }
    /// A host whose repository instructions change on every read.
    pub async fn with_changing_instructions(
        script: Vec<Reply>,
        source: Arc<ChangingInstructions>,
    ) -> Self {
        Self::assemble(script, None, None, None, (0, 0), source).await
    }
    /// A host whose turn budget stops the turn, reporting `usage` per model call.
    pub async fn budgeted(script: Vec<Reply>, max_turn_tokens: u64, usage: (u64, u64)) -> Self {
        let host =
            Self::assemble(script, None, None, None, usage, Arc::new(Instructions(""))).await;
        let mut config = host.config.clone();
        config.max_turn_tokens = Some(max_turn_tokens);
        Self { config, ..host }
    }
    /// A host where the first executed effect injects a steering message mid-turn.
    pub async fn steering_after_first_effect(script: Vec<Reply>) -> Self {
        Self::assemble(script, None, None, None, (0, 0), Arc::new(Instructions(""))).await
    }
    /// A host whose turn-level hook refuses every model call.
    pub async fn with_blocking_inference_hook(script: Vec<Reply>) -> Self {
        Self::assemble(
            script,
            None,
            None,
            Some(Arc::new(BlockModelCalls)),
            (0, 0),
            Arc::new(Instructions("")),
        )
        .await
    }
    /// A host that cancels the turn from inside the first executed effect.
    pub async fn cancelling_after_first_effect(
        script: Vec<Reply>,
        cancel: CancellationToken,
    ) -> Self {
        Self::assemble(
            script,
            None,
            Some(cancel),
            None,
            (0, 0),
            Arc::new(Instructions("")),
        )
        .await
    }

    async fn assemble(
        script: Vec<Reply>,
        hook: Option<Arc<dyn ToolHook>>,
        cancel_after_execute: Option<CancellationToken>,
        inference_hook: Option<Arc<dyn InferenceHook>>,
        usage: (u64, u64),
        instructions: Arc<dyn InstructionSource>,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_path_buf();
        let target = workspace.join("effect.txt");
        let effects = Arc::new(Effects::default());
        let steering = Arc::new(huginn_core::SteeringQueue::default());
        let tools: Arc<dyn ToolRegistry> = Arc::new(EffectsRegistry(Arc::new(WriteTool {
            effects: effects.clone(),
            target: target.clone(),
            cancel_after_execute,
            steer_after_execute: steering.is_empty().then(|| steering.clone()),
        })));
        let mut supervisor = Supervisor::default();
        supervisor
            .register(Box::new(composition::HooksPlugin(Arc::default())))
            .unwrap();
        supervisor
            .register(Box::new(composition::InferenceHooksPlugin(Arc::default())))
            .unwrap();
        if let Some(hook) = &hook {
            supervisor
                .register(Box::new(HookPlugin(hook.clone())))
                .unwrap();
        }
        if let Some(hook) = &inference_hook {
            supervisor
                .register(Box::new(InferenceHookPlugin(hook.clone())))
                .unwrap();
        }
        supervisor.start().await.unwrap();
        let hooks = supervisor
            .resolve::<ToolHooksService>(&tool_hooks_id())
            .unwrap()
            .0
            .clone();
        let inference_hooks = supervisor
            .resolve::<InferenceHooksService>(&inference_hooks_id())
            .unwrap()
            .0
            .clone();
        Self {
            script: Arc::new(ScriptModel::reporting_usage(script, usage)),
            effects,
            sessions: Arc::new(MemorySessions::default()),
            approvals: Arc::new(Approvals::default()),
            hooks,
            inference_hooks,
            steering,
            instructions,
            workspace,
            target,
            config: LoopConfig::default(),
            hook,
            inference_hook,
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
            inference_hooks: Some(self.inference_hooks.clone()),
            steering: Some(self.steering.clone()),
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
        self.instructions.clone()
    }
    /// The hook this host expects to be enforced: `LoopConfig::hooks` for a loop that
    /// takes the registry, `hook_plugin` for a loop that composes its own supervisor.
    pub fn hook(&self) -> Option<Arc<dyn ToolHook>> {
        self.hook.clone()
    }
    /// The turn-level hook this host expects to be enforced.
    pub fn inference_hook(&self) -> Option<Arc<dyn InferenceHook>> {
        self.inference_hook.clone()
    }
}

/// A hook that refuses every model call, so no request is made.
struct BlockModelCalls;
#[async_trait]
impl InferenceHook for BlockModelCalls {
    async fn before(
        &self,
        _: huginn_core::TurnId,
        _: StepId,
        _: &ToolContext,
    ) -> Result<InferenceVerdict> {
        Ok(InferenceVerdict::Block("model calls are disabled".into()))
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

/// Building an implementation may need to compose a supervisor, so it may be async.
pub type BuildFuture<'a> = Pin<Box<dyn Future<Output = Arc<dyn AgentLoop>> + Send + 'a>>;

/// One loop implementation under certification.
pub struct UnderTest {
    pub name: String,
    pub capabilities: Capabilities,
    build: Box<dyn for<'a> Fn(&'a Host) -> BuildFuture<'a> + Send + Sync>,
}
impl UnderTest {
    pub fn new(
        name: impl Into<String>,
        capabilities: Capabilities,
        build: impl for<'a> Fn(&'a Host) -> BuildFuture<'a> + Send + Sync + 'static,
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
    let mut record = |outcome: CaseOutcome| report.outcomes.push(outcome);

    record(
        guarded(
            "streaming_lifecycle_order",
            streaming_lifecycle_order(under_test).await,
        )
        .await,
    );
    record(
        guarded(
            "cancel_before_model_stream",
            cancel_before_model_stream(under_test).await,
        )
        .await,
    );
    record(
        guarded(
            "cancel_inside_model_stream",
            cancel_inside_model_stream(under_test).await,
        )
        .await,
    );
    record(
        guarded(
            "denial_performs_no_effect",
            denial_performs_no_effect(under_test).await,
        )
        .await,
    );
    record(
        guarded(
            "cancel_between_tool_calls",
            cancel_between_tool_calls(under_test).await,
        )
        .await,
    );
    record(
        guarded(
            "effect_unknown_is_not_replayed",
            effect_unknown_is_not_replayed(under_test).await,
        )
        .await,
    );
    record(
        guarded(
            "iteration_limit_is_enforced",
            iteration_limit_is_enforced(under_test).await,
        )
        .await,
    );
    record(
        guarded(
            "hook_block_prevents_effect",
            hook_block_prevents_effect(under_test).await,
        )
        .await,
    );
    record(
        guarded(
            "duplicate_tool_calls_are_rejected",
            duplicate_tool_calls_are_rejected(under_test).await,
        )
        .await,
    );
    record(
        guarded(
            "inference_hook_blocks_the_call",
            inference_hook_blocks_the_call(under_test).await,
        )
        .await,
    );
    record(
        guarded(
            "steering_reaches_the_next_step",
            steering_reaches_the_next_step(under_test).await,
        )
        .await,
    );
    record(
        guarded(
            "turn_budget_stops_the_turn",
            turn_budget_stops_the_turn(under_test).await,
        )
        .await,
    );
    record(
        guarded(
            "context_is_pinned_for_the_turn",
            context_is_pinned_for_the_turn(under_test).await,
        )
        .await,
    );
    record(if under_test.capabilities.compaction {
        guarded(
            "compaction_keeps_turn_boundary",
            compaction_keeps_turn_boundary(under_test).await,
        )
        .await
    } else {
        CaseOutcome {
            case: "compaction_keeps_turn_boundary",
            status: Status::Skipped,
            detail: "the implementation does not declare compaction".into(),
        }
    });
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
    let implementation = (under_test.build)(&host).await;
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
    let implementation = (under_test.build)(&host).await;
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
    let implementation = (under_test.build)(&host).await;
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
    let implementation = (under_test.build)(&host).await;
    let approval = host.approval(false);
    let events = Events::default();
    let mut session = Session::new(host.workspace.clone());
    implementation
        .run(
            &mut session,
            "go".into(),
            &*approval,
            &events,
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

async fn cancel_between_tool_calls(under_test: &UnderTest) -> std::result::Result<(), String> {
    let cancel = CancellationToken::new();
    let host = Host::cancelling_after_first_effect(
        vec![
            Reply::ToolCalls(vec![write_call("c1"), write_call("c2")]),
            Reply::Text("never"),
        ],
        cancel.clone(),
    )
    .await;
    let implementation = (under_test.build)(&host).await;
    let approval = host.approval(true);
    let mut session = Session::new(host.workspace.clone());
    let outcome = implementation
        .run(
            &mut session,
            "go".into(),
            &*approval,
            &Events::default(),
            cancel,
        )
        .await;
    if !matches!(outcome, Err(AgentError::Cancelled)) {
        return Err(format!(
            "cancelling mid-turn must fail as Cancelled, got {outcome:?}"
        ));
    }
    if host.effects.executed() != 1 {
        return Err(format!(
            "the call after cancellation must not run, executed {} tools",
            host.effects.executed()
        ));
    }
    Ok(())
}

async fn effect_unknown_is_not_replayed(under_test: &UnderTest) -> std::result::Result<(), String> {
    let host = Host::new(vec![Reply::Text("done")]).await;
    let implementation = (under_test.build)(&host).await;
    let mut session = Session::new(host.workspace.clone());
    session
        .messages
        .push(Message::text(Role::User, "earlier request"));
    session.messages.push(Message {
        role: Role::Assistant,
        content: String::new(),
        tool_calls: vec![write_call("pending")],
        tool_call_id: None,
    });
    host.sessions.save(&session).await.unwrap();
    let mut resumed = host.sessions.load(&session.id).await.unwrap();

    let approval = host.approval(true);
    implementation
        .run(
            &mut resumed,
            "continue".into(),
            &*approval,
            &Events::default(),
            CancellationToken::new(),
        )
        .await
        .map_err(|error| format!("resuming an interrupted turn failed: {error}"))?;

    if host.effects.executed() != 0 {
        return Err(format!(
            "an effect with unknown outcome must never be replayed, executed {} tools",
            host.effects.executed()
        ));
    }
    let recorded = resumed.messages.iter().any(|message| {
        message.role == Role::Tool && message.tool_call_id.as_deref() == Some("pending")
    });
    if !recorded {
        return Err(
            "the unanswered call must be closed with an explicit unknown-outcome result".into(),
        );
    }
    Ok(())
}

async fn iteration_limit_is_enforced(under_test: &UnderTest) -> std::result::Result<(), String> {
    let host = Host::new(vec![
        Reply::ToolCalls(vec![write_call("c1")]),
        Reply::ToolCalls(vec![write_call("c2")]),
        Reply::Text("never"),
    ])
    .await;
    let mut config = host.config.clone();
    config.max_iterations = 2;
    let host = Host { config, ..host };
    let implementation = (under_test.build)(&host).await;
    let approval = host.approval(true);
    let mut session = Session::new(host.workspace.clone());
    let outcome = implementation
        .run(
            &mut session,
            "go".into(),
            &*approval,
            &Events::default(),
            CancellationToken::new(),
        )
        .await;
    match outcome {
        Err(AgentError::Model(message)) if message.contains("iteration limit") => Ok(()),
        other => Err(format!(
            "the iteration limit must stop the turn with a diagnostic, got {other:?}"
        )),
    }
}

async fn hook_block_prevents_effect(under_test: &UnderTest) -> std::result::Result<(), String> {
    let host = Host::with_blocking_hook(vec![
        Reply::ToolCalls(vec![write_call("c1")]),
        Reply::Text("done"),
    ])
    .await;
    let implementation = (under_test.build)(&host).await;
    let approval = host.approval(true);
    let events = Events::default();
    let mut session = Session::new(host.workspace.clone());
    implementation
        .run(
            &mut session,
            "go".into(),
            &*approval,
            &events,
            CancellationToken::new(),
        )
        .await
        .map_err(|error| format!("a hook block is a tool error, not a turn failure: {error}"))?;

    if host.effects.prepared() != 0 || host.effects.executed() != 0 {
        return Err("a blocked call must produce no preview and no effect".into());
    }
    if host.approvals.prompts.load(Ordering::SeqCst) != 0 {
        return Err("a blocked call must not reach the approval prompt".into());
    }
    if host.target.exists() {
        return Err("a blocked call must leave no observable effect".into());
    }
    let blocked_visible = session
        .messages
        .iter()
        .any(|message| message.role == Role::Tool && message.content.contains("blocked by hook"));
    if !blocked_visible {
        return Err("the model must see the block as the result of that call".into());
    }
    Ok(())
}

async fn duplicate_tool_calls_are_rejected(
    under_test: &UnderTest,
) -> std::result::Result<(), String> {
    let host = Host::new(vec![Reply::ToolCalls(vec![
        write_call("same"),
        write_call("same"),
    ])])
    .await;
    let implementation = (under_test.build)(&host).await;
    let approval = host.approval(true);
    let mut session = Session::new(host.workspace.clone());
    let outcome = implementation
        .run(
            &mut session,
            "go".into(),
            &*approval,
            &Events::default(),
            CancellationToken::new(),
        )
        .await;
    if !matches!(&outcome, Err(AgentError::Model(message)) if message.contains("duplicate")) {
        return Err(format!(
            "duplicate call ids must be rejected, got {outcome:?}"
        ));
    }
    if host.effects.executed() != 0 {
        return Err("a rejected response must not execute anything".into());
    }
    Ok(())
}

async fn compaction_keeps_turn_boundary(under_test: &UnderTest) -> std::result::Result<(), String> {
    let host = Host::new(vec![Reply::Text("summary")]).await;
    let implementation = (under_test.build)(&host).await;
    let mut session = Session::new(host.workspace.clone());
    session.messages.push(Message::text(Role::User, "first"));
    session.messages.push(Message {
        role: Role::Assistant,
        content: String::new(),
        tool_calls: vec![write_call("t1")],
        tool_call_id: None,
    });
    session
        .messages
        .push(Message::tool("t1".into(), "result".into()));
    session
        .messages
        .push(Message::text(Role::Assistant, "first done"));
    session.messages.push(Message::text(Role::User, "second"));
    // A trailing tool-call turn has no answer yet, so it must stay out of the summary.
    session.messages.push(Message {
        role: Role::Assistant,
        content: String::new(),
        tool_calls: vec![write_call("t2")],
        tool_call_id: None,
    });

    implementation
        .compact(&mut session, &Events::default(), CancellationToken::new())
        .await
        .map_err(|error| format!("compaction failed: {error}"))?;

    if session.summary.as_deref() != Some("summary") {
        return Err(format!(
            "expected the summary to be stored, got {:?}",
            session.summary
        ));
    }
    if session.summary_through != 4 {
        return Err(format!(
            "compaction must stop at the last complete turn (4), got {}",
            session.summary_through
        ));
    }
    Ok(())
}

/// A loop that must fail this suite: it executes effects without asking the host and
/// replays a call whose outcome is unknown.
pub mod broken {
    use super::*;

    pub struct ReplayingLoop {
        model: Arc<dyn ModelProvider>,
        tools: Arc<dyn ToolRegistry>,
    }
    impl ReplayingLoop {
        pub fn new(model: Arc<dyn ModelProvider>, tools: Arc<dyn ToolRegistry>) -> Self {
            Self { model, tools }
        }
    }

    #[async_trait]
    impl AgentLoop for ReplayingLoop {
        async fn run(
            &self,
            session: &mut Session,
            prompt: String,
            _approvals: &dyn ApprovalPolicy,
            events: &dyn EventSink,
            cancel: CancellationToken,
        ) -> Result<()> {
            let turn = session.allocate_turn();
            session.events.push(Event::TurnStarted {
                session_id: session.id.clone(),
                turn,
            });
            session.messages.push(Message::text(Role::User, prompt));
            // Replays anything left unanswered by a previous turn.
            let pending: Vec<ToolCall> = session
                .messages
                .iter()
                .flat_map(|message| message.tool_calls.clone())
                .filter(|call| {
                    !session
                        .messages
                        .iter()
                        .any(|message| message.tool_call_id.as_deref() == Some(call.id.as_str()))
                })
                .collect();
            let mut stream = self
                .model
                .stream(
                    ModelRequest {
                        messages: session.messages.clone(),
                        tools: vec![],
                        max_tokens: 64,
                    },
                    cancel.clone(),
                )
                .await?;
            let mut answer = String::new();
            while let Some(delta) = stream.next().await {
                if let ModelDelta::Text(text) = delta? {
                    answer.push_str(&text);
                }
            }
            let context = ToolContext {
                selected_skills: vec![],
                workspace: session.workspace.clone(),
                cancellation: cancel,
            };
            for call in pending {
                let Some(tool) = self.tools.get(&call.name) else {
                    continue;
                };
                // No approval: this is the defect the suite must catch.
                let action = tool.prepare(&call, &context).await?;
                tool.execute(action, &context).await?;
            }
            session
                .messages
                .push(Message::text(Role::Assistant, answer.clone()));
            events.emit(Event::Text {
                step: StepId(1),
                text: answer,
            });
            session.events.push(Event::TurnFinished { turn });
            Ok(())
        }
    }
}

async fn inference_hook_blocks_the_call(under_test: &UnderTest) -> std::result::Result<(), String> {
    let host = Host::with_blocking_inference_hook(vec![Reply::Text("never")]).await;
    let implementation = (under_test.build)(&host).await;
    let approval = host.approval(true);
    let mut session = Session::new(host.workspace.clone());
    let outcome = implementation
        .run(
            &mut session,
            "go".into(),
            &*approval,
            &Events::default(),
            CancellationToken::new(),
        )
        .await;
    match &outcome {
        Err(AgentError::Model(message)) if message.contains("blocked by hook") => {}
        other => {
            return Err(format!(
                "a turn-level block must end the turn with a diagnostic, got {other:?}"
            ))
        }
    }
    if host.script.calls() != 0 {
        return Err(format!(
            "a blocked step must not reach the model, it was called {} times",
            host.script.calls()
        ));
    }
    let audited = session.events.iter().any(|event| {
        matches!(event, Event::InferenceBlocked { hook, .. } if hook == "conformance.inference-hook")
    });
    if !audited {
        return Err("the block must be recorded as an auditable event".into());
    }
    Ok(())
}

async fn steering_reaches_the_next_step(under_test: &UnderTest) -> std::result::Result<(), String> {
    let host = Host::steering_after_first_effect(vec![
        Reply::ToolCalls(vec![write_call("c1")]),
        Reply::Text("done"),
    ])
    .await;
    let implementation = (under_test.build)(&host).await;
    let approval = host.approval(true);
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
        .map_err(|error| format!("run failed: {error}"))?;

    let requests = host.script.requests();
    if requests.len() != 2 {
        return Err(format!(
            "expected two model calls, the loop made {}",
            requests.len()
        ));
    }
    let delivered = requests[1]
        .messages
        .iter()
        .any(|message| message.role == Role::User && message.content == "steer: narrow the change");
    if !delivered {
        return Err("the steering message must reach the model at the next step".into());
    }
    let recorded = session
        .events
        .iter()
        .any(|event| matches!(event, Event::Steered { count: 1, .. }));
    if !recorded {
        return Err("delivery must be recorded as a durable event".into());
    }
    Ok(())
}

async fn turn_budget_stops_the_turn(under_test: &UnderTest) -> std::result::Result<(), String> {
    let host = Host::budgeted(
        vec![
            Reply::ToolCalls(vec![write_call("c1")]),
            Reply::ToolCalls(vec![write_call("c2")]),
            Reply::Text("never"),
        ],
        10,
        (4, 4),
    )
    .await;
    let implementation = (under_test.build)(&host).await;
    let approval = host.approval(true);
    let mut session = Session::new(host.workspace.clone());
    let outcome = implementation
        .run(
            &mut session,
            "go".into(),
            &*approval,
            &Events::default(),
            CancellationToken::new(),
        )
        .await;
    match &outcome {
        Err(AgentError::Model(message)) if message.contains("budget") => {}
        other => {
            return Err(format!(
                "an exhausted turn budget must stop the turn, got {other:?}"
            ))
        }
    }
    if host.script.calls() != 2 {
        return Err(format!(
            "the loop must stop as soon as the budget is exceeded, it made {} calls",
            host.script.calls()
        ));
    }
    // The step that pushed the turn over budget must not dispatch its tools: the
    // budget is a ceiling on effects too, not only on requests.
    if host.effects.executed() != 1 {
        return Err(format!(
            "the over-budget step must not take effects, executed {} tools",
            host.effects.executed()
        ));
    }
    Ok(())
}

async fn context_is_pinned_for_the_turn(under_test: &UnderTest) -> std::result::Result<(), String> {
    let source = Arc::new(ChangingInstructions::default());
    let host = Host::with_changing_instructions(
        vec![
            Reply::ToolCalls(vec![write_call("c1")]),
            Reply::Text("done"),
        ],
        source.clone(),
    )
    .await;
    let implementation = (under_test.build)(&host).await;
    let approval = host.approval(true);
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
        .map_err(|error| format!("run failed: {error}"))?;

    if source.loads() != 1 {
        return Err(format!(
            "instructions must be read once per turn, read {} times",
            source.loads()
        ));
    }
    let requests = host.script.requests();
    if requests.len() != 2 {
        return Err(format!("expected two model calls, got {}", requests.len()));
    }
    let first = requests[0]
        .messages
        .first()
        .map(|message| message.content.clone())
        .unwrap_or_default();
    let second = requests[1]
        .messages
        .first()
        .map(|message| message.content.clone())
        .unwrap_or_default();
    if !first.contains("instructions v1") {
        return Err(format!(
            "the turn did not start from the first read: {first}"
        ));
    }
    if first != second {
        return Err("the system context drifted between steps of the same turn".into());
    }
    let pinned = session
        .events
        .iter()
        .any(|event| matches!(event, Event::ContextPinned { bytes, .. } if *bytes > 0));
    if !pinned {
        return Err("the turn must record which context it was pinned to".into());
    }
    Ok(())
}
