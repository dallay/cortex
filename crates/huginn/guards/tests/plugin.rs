use async_trait::async_trait;
use huginn_guards::{Guardrails, GuardsPlugin};
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    v1::hooks::{tool_hooks_id, ToolCall, ToolContext, ToolHooks, ToolHooksService},
    Result,
};
use std::sync::Arc;

struct Hooks(Arc<ToolHooks>);
#[async_trait]
impl Plugin for Hooks {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "hooks".into(),
            provides: vec![tool_hooks_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(tool_hooks_id(), ToolHooksService(self.0.clone()))
    }
}

#[tokio::test]
async fn guards_are_lifecycle_owned_and_block_before_preparation() {
    let root = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        workspace: root.path().into(),
        selected_skills: vec![],
        cancellation: Default::default(),
    };
    let call = ToolCall {
        id: "blocked".into(),
        name: "shell".into(),
        arguments: serde_json::json!({"command":"rm -rf ."}),
    };
    let hooks = Arc::new(ToolHooks::default());
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(Hooks(hooks.clone()))).unwrap();
    supervisor
        .register(Box::new(GuardsPlugin(Arc::new(Guardrails::default()))))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(hooks.before(&call, &ctx).await.is_some());
    supervisor.remove("guards.native").await.unwrap();
    assert!(hooks.before(&call, &ctx).await.is_none());
    supervisor.shutdown().await;
}
