use async_trait::async_trait;
use huginn_core::{
    hooks::{tool_hooks_id, ToolHook, ToolHooks, ToolHooksService},
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    AgentError, Result,
};
use std::sync::Arc;

pub struct HostHooks(pub Arc<ToolHooks>);
#[async_trait]
impl Plugin for HostHooks {
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

pub struct Author {
    pub id: &'static str,
    pub hooks: Vec<(&'static str, Arc<dyn ToolHook>)>,
    pub fail: bool,
}
#[async_trait]
impl Plugin for Author {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            provides: vec![],
            requires: vec![tool_hooks_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let hooks = ctx.resolve::<ToolHooksService>(&tool_hooks_id())?.0.clone();
        for (id, hook) in &self.hooks {
            hooks.register(ctx, (*id).into(), hook.clone())?;
            assert!(
                !hooks.ids().contains(&(*id).to_string()),
                "staged hooks stay invisible until commit"
            );
        }
        if self.fail {
            return Err(AgentError::Composition("injected".into()));
        }
        Ok(())
    }
}

pub async fn compose(hooks: Arc<ToolHooks>, authors: Vec<Author>) -> (Supervisor, Result<()>) {
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(HostHooks(hooks))).unwrap();
    for author in authors {
        supervisor.register(Box::new(author)).unwrap();
    }
    let result = supervisor.start().await;
    (supervisor, result)
}
