use crate::{
    loop_engine::{LoopConfig, StandardLoop},
    providers::ProvidersPlugin,
};
use async_trait::async_trait;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, ServiceId, Supervisor},
    AgentLoop, InstructionSource, InstructionsService, LoopService, ModelProvider, ModelService,
    Result, SessionStore, SessionsService, ToolRegistry, ToolsService,
};
use std::sync::Arc;

pub fn model_id() -> ServiceId {
    ServiceId::new("agent:model", 1)
}
pub fn tools_id() -> ServiceId {
    ServiceId::new("agent:tools", 1)
}
pub fn sessions_id() -> ServiceId {
    ServiceId::new("agent:sessions", 1)
}
pub fn loop_id() -> ServiceId {
    ServiceId::new("agent:loop", 1)
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

pub struct LoopPlugin(pub LoopConfig);
#[async_trait]
impl Plugin for LoopPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "core.loop".into(),
            provides: vec![loop_id()],
            requires: vec![model_id(), tools_id(), sessions_id(), instructions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let implementation = StandardLoop {
            model: ctx.resolve::<ModelService>(&model_id())?.0.clone(),
            tools: ctx.resolve::<ToolsService>(&tools_id())?.0.clone(),
            sessions: ctx.resolve::<SessionsService>(&sessions_id())?.0.clone(),
            instructions: ctx
                .resolve::<InstructionsService>(&instructions_id())?
                .0
                .clone(),
            config: self.0.clone(),
            lifecycle: ctx.cancellation.clone(),
        };
        let implementation: Arc<dyn AgentLoop> = Arc::new(implementation);
        ctx.provide(loop_id(), LoopService(implementation))
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
    supervisor.register(Box::new(LoopPlugin(config)))?;
    if let Err(error) = supervisor.start().await {
        supervisor.shutdown().await;
        return Err(error);
    }
    Ok(supervisor)
}
