use crate::{
    loop_engine::{skill_commands, LoopConfig, LoopExecution, StandardLoop},
    providers::ProvidersPlugin,
};
use async_trait::async_trait;
use huginn_core::{
    hooks::{tool_hooks_id, ToolHooks, ToolHooksService},
    kernel::{Manifest, Plugin, PluginContext, ServiceId, ServiceSlotHandle, Supervisor},
    AgentError, AgentLoop, ApprovalPolicy, CancellationToken, EventSink, InstructionSource,
    InstructionsService, LoopService, ModelProvider, ModelService, Result, Session, SessionStore,
    SessionsService, ToolRegistry, ToolsService,
};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

pub fn model_id() -> ServiceId {
    ServiceId::new("agent:model", 1)
}
pub fn tools_id() -> ServiceId {
    ServiceId::new("agent:tools", 1)
}
pub fn sessions_id() -> ServiceId {
    ServiceId::new("agent:sessions", 1)
}
/// Delegates to the contract crate so the host and an authoring plugin cannot drift.
pub fn loop_id() -> ServiceId {
    huginn_core::agent_loop_id()
}

pub struct ModelPlugin(pub Arc<dyn ModelProvider>);
#[async_trait]
impl Plugin for ModelPlugin {
    fn manifest(&self) -> Manifest {
        ProvidersPlugin::with_legacy(self.0.clone()).manifest()
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ProvidersPlugin::with_legacy(self.0.clone())
            .activate(ctx)
            .await
    }
}
pub struct ToolsPlugin(pub Arc<dyn ToolRegistry>);
#[async_trait]
impl Plugin for ToolsPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "core.tools".into(),
            provides: vec![tools_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(tools_id(), ToolsService(self.0.clone()))
    }
}
pub struct SessionsPlugin(pub Arc<dyn SessionStore>);
#[async_trait]
impl Plugin for SessionsPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "core.sessions".into(),
            provides: vec![sessions_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(sessions_id(), SessionsService(self.0.clone()))
    }
}
pub fn instructions_id() -> ServiceId {
    ServiceId::new("agent:instructions", 1)
}
pub struct InstructionsPlugin(pub Arc<dyn InstructionSource>);
#[async_trait]
impl Plugin for InstructionsPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "core.instructions".into(),
            provides: vec![instructions_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(instructions_id(), InstructionsService(self.0.clone()))
    }
}

/// Host-owned registry where plugins add restrict-only turn-level hooks.
pub struct InferenceHooksPlugin(pub Arc<huginn_core::hooks::InferenceHooks>);
#[async_trait]
impl Plugin for InferenceHooksPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "core.inference-hooks".into(),
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

/// Host-owned registry where plugins add restrict-only tool hooks.
pub struct HooksPlugin(pub Arc<ToolHooks>);
#[async_trait]
impl Plugin for HooksPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "core.hooks".into(),
            provides: vec![tool_hooks_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(tool_hooks_id(), ToolHooksService(self.0.clone()))
    }
}

/// The kernel owns the hook registries.
///
/// A caller that supplies a different registry would see its guards dropped silently,
/// which fails open, so a mismatch is refused instead of overwritten.
fn adopt_hook_registries(ctx: &PluginContext, config: &mut LoopConfig) -> Result<()> {
    let kernel_tool_hooks = ctx.resolve::<ToolHooksService>(&tool_hooks_id())?.0.clone();
    let kernel_inference_hooks = ctx
        .resolve::<huginn_core::hooks::InferenceHooksService>(
            &huginn_core::hooks::inference_hooks_id(),
        )?
        .0
        .clone();
    if config
        .hooks
        .as_ref()
        .is_some_and(|supplied| !Arc::ptr_eq(supplied, &kernel_tool_hooks))
    {
        return Err(AgentError::Composition(
            "loop config supplies a tool-hook registry the kernel does not own".into(),
        ));
    }
    if config
        .inference_hooks
        .as_ref()
        .is_some_and(|supplied| !Arc::ptr_eq(supplied, &kernel_inference_hooks))
    {
        return Err(AgentError::Composition(
            "loop config supplies an inference-hook registry the kernel does not own".into(),
        ));
    }
    config.hooks = Some(kernel_tool_hooks);
    config.inference_hooks = Some(kernel_inference_hooks);
    Ok(())
}

pub struct LoopPlugin(pub LoopConfig);
#[async_trait]
impl Plugin for LoopPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "core.loop".into(),
            provides: vec![loop_id()],
            requires: vec![
                model_id(),
                tools_id(),
                sessions_id(),
                instructions_id(),
                tool_hooks_id(),
                huginn_core::hooks::inference_hooks_id(),
            ],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let mut config = self.0.clone();
        adopt_hook_registries(ctx, &mut config)?;
        let implementation = StandardLoop {
            model: ctx.resolve::<ModelService>(&model_id())?.0.clone(),
            tools: ctx.resolve::<ToolsService>(&tools_id())?.0.clone(),
            sessions: ctx.resolve::<SessionsService>(&sessions_id())?.0.clone(),
            instructions: ctx
                .resolve::<InstructionsService>(&instructions_id())?
                .0
                .clone(),
            config,
            lifecycle: ctx.cancellation.clone(),
        };
        let implementation: Arc<dyn AgentLoop> = Arc::new(implementation);
        ctx.provide(loop_id(), LoopService(implementation))
    }
}

struct LoopAdmission {
    state: Mutex<LoopAdmissionState>,
    idle: Notify,
}

struct LoopAdmissionState {
    accepting: bool,
    active: usize,
}

impl LoopAdmission {
    fn enter(self: &Arc<Self>) -> Result<LoopAdmissionLease> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AgentError::Composition("loop admission lock poisoned".into()))?;
        if !state.accepting {
            return Err(AgentError::Composition("loop is draining".into()));
        }
        state.active = state
            .active
            .checked_add(1)
            .ok_or_else(|| AgentError::Composition("loop operation counter overflow".into()))?;
        drop(state);
        Ok(LoopAdmissionLease(self.clone()))
    }

    async fn drain(&self) -> Result<()> {
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| AgentError::Composition("loop admission lock poisoned".into()))?;
            state.accepting = false;
        }

        loop {
            let notified = self.idle.notified();
            if self
                .state
                .lock()
                .map_err(|_| AgentError::Composition("loop admission lock poisoned".into()))?
                .active
                == 0
            {
                return Ok(());
            }
            notified.await;
        }
    }

    fn resume(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AgentError::Composition("loop admission lock poisoned".into()))?;
        state.accepting = true;
        drop(state);
        Ok(())
    }
}

struct LoopAdmissionLease(Arc<LoopAdmission>);

impl Drop for LoopAdmissionLease {
    fn drop(&mut self) {
        let became_idle = self.0.state.lock().is_ok_and(|mut state| {
            state.active = state.active.saturating_sub(1);
            state.active == 0
        });
        if became_idle {
            self.0.idle.notify_one();
        }
    }
}

struct LeasedAgentLoop {
    model: ServiceSlotHandle<ModelService>,
    sessions: ServiceSlotHandle<SessionsService>,
    /// Acquired per operation: a substituted registry must be the one evaluated.
    hooks: ServiceSlotHandle<ToolHooksService>,
    inference_hooks: ServiceSlotHandle<huginn_core::hooks::InferenceHooksService>,
    tools: Arc<dyn ToolRegistry>,
    instructions: Arc<dyn InstructionSource>,
    config: LoopConfig,
    lifecycle: CancellationToken,
    admission: Arc<LoopAdmission>,
}

impl LeasedAgentLoop {
    fn execution<'a>(
        &'a self,
        model: &'a ModelService,
        sessions: &'a SessionsService,
        config: &'a LoopConfig,
    ) -> LoopExecution<'a> {
        LoopExecution::new(
            model.0.as_ref(),
            self.tools.as_ref(),
            sessions.0.as_ref(),
            self.instructions.as_ref(),
            config,
            &self.lifecycle,
        )
    }

    /// Resolves the registries this run must evaluate.
    ///
    /// The config copy taken at activation is not authoritative: `Substitute` can replace
    /// either service, and a loop that kept the retired registry would silently skip a
    /// blocking hook installed afterwards.
    fn live_config(
        &self,
        hooks: &ToolHooksService,
        inference_hooks: &huginn_core::hooks::InferenceHooksService,
    ) -> LoopConfig {
        let mut config = self.config.clone();
        config.hooks = Some(hooks.0.clone());
        config.inference_hooks = Some(inference_hooks.0.clone());
        config
    }
}

#[async_trait]
impl AgentLoop for LeasedAgentLoop {
    async fn skill_commands(
        &self,
        workspace: &std::path::Path,
        cancellation: CancellationToken,
    ) -> Result<Vec<(String, String)>> {
        skill_commands(&self.config, &self.lifecycle, workspace, cancellation).await
    }

    async fn run(
        &self,
        session: &mut Session,
        prompt: String,
        approvals: &dyn ApprovalPolicy,
        events: &dyn EventSink,
        cancellation: CancellationToken,
    ) -> Result<()> {
        let _operation = huginn_core::SessionOperation::try_enter(&session.id)?;
        let _admission = self.admission.enter()?;
        let model = self.model.acquire()?;
        let sessions = self.sessions.acquire()?;
        let hooks = self.hooks.acquire()?;
        let inference_hooks = self.inference_hooks.acquire()?;
        let config = self.live_config(&hooks, &inference_hooks);
        self.execution(&model, &sessions, &config)
            .run(session, prompt, approvals, events, cancellation)
            .await
    }

    async fn compact(
        &self,
        session: &mut Session,
        events: &dyn EventSink,
        cancellation: CancellationToken,
    ) -> Result<()> {
        let _operation = huginn_core::SessionOperation::try_enter(&session.id)?;
        let _admission = self.admission.enter()?;
        let model = self.model.acquire()?;
        let sessions = self.sessions.acquire()?;
        let hooks = self.hooks.acquire()?;
        let inference_hooks = self.inference_hooks.acquire()?;
        let config = self.live_config(&hooks, &inference_hooks);
        self.execution(&model, &sessions, &config)
            .compact(session, events, cancellation)
            .await
    }
}

struct LeasedLoopPlugin {
    config: LoopConfig,
    admission: Arc<LoopAdmission>,
}

/// Create the production loop plugin without exposing its concrete adapter type.
pub fn leased_loop_plugin(config: LoopConfig) -> Box<dyn Plugin> {
    Box::new(LeasedLoopPlugin {
        config,
        admission: Arc::new(LoopAdmission {
            state: Mutex::new(LoopAdmissionState {
                accepting: true,
                active: 0,
            }),
            idle: Notify::new(),
        }),
    })
}

#[async_trait]
impl Plugin for LeasedLoopPlugin {
    fn manifest(&self) -> Manifest {
        LoopPlugin(self.config.clone()).manifest()
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let model = ctx.service_slot_handle::<ModelService>(&model_id())?;
        let sessions = ctx.service_slot_handle::<SessionsService>(&sessions_id())?;
        let mut config = self.config.clone();
        adopt_hook_registries(ctx, &mut config)?;

        let implementation = LeasedAgentLoop {
            model,
            sessions,
            hooks: ctx.service_slot_handle::<ToolHooksService>(&tool_hooks_id())?,
            inference_hooks: ctx.service_slot_handle::<huginn_core::hooks::InferenceHooksService>(
                &huginn_core::hooks::inference_hooks_id(),
            )?,
            tools: ctx.resolve::<ToolsService>(&tools_id())?.0.clone(),
            instructions: ctx
                .resolve::<InstructionsService>(&instructions_id())?
                .0
                .clone(),
            config,
            lifecycle: ctx.cancellation.clone(),
            admission: self.admission.clone(),
        };
        let implementation: Arc<dyn AgentLoop> = Arc::new(implementation);
        ctx.provide(loop_id(), LoopService(implementation))
    }

    fn supports_drain(&self) -> bool {
        true
    }

    async fn drain(&mut self) -> Result<()> {
        self.admission.drain().await
    }

    async fn resume(&mut self) -> Result<()> {
        self.admission.resume()
    }
}

pub async fn build(
    model: Arc<dyn ModelProvider>,
    tools: Arc<dyn ToolRegistry>,
    sessions: Arc<dyn SessionStore>,
    config: LoopConfig,
    instructions: Arc<dyn InstructionSource>,
) -> Result<Supervisor> {
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(ModelPlugin(model)))?;
    supervisor.register(Box::new(ToolsPlugin(tools)))?;
    supervisor.register(Box::new(SessionsPlugin(sessions)))?;
    supervisor.register(Box::new(InstructionsPlugin(instructions)))?;
    supervisor.register(Box::new(HooksPlugin(Arc::default())))?;
    supervisor.register(Box::new(InferenceHooksPlugin(Arc::default())))?;
    supervisor.register(leased_loop_plugin(config))?;
    if let Err(error) = supervisor.start().await {
        supervisor.shutdown().await;
        return Err(error);
    }
    Ok(supervisor)
}
