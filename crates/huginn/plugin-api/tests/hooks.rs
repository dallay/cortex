use async_trait::async_trait;
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    v1::hooks::{
        tool_hooks_id, ToolCall, ToolContext, ToolHook, ToolHooks, ToolHooksService, ToolVerdict,
    },
    Result,
};
use std::sync::Arc;

struct Host(Arc<ToolHooks>);
#[async_trait]
impl Plugin for Host {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "host".into(),
            provides: vec![tool_hooks_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(tool_hooks_id(), ToolHooksService(self.0.clone()))
    }
}
struct DenyShell;
#[async_trait]
impl ToolHook for DenyShell {
    async fn before(&self, call: &ToolCall, _: &ToolContext) -> Result<ToolVerdict> {
        Ok(if call.name == "shell" {
            ToolVerdict::Block("shell is disabled by policy".into())
        } else {
            ToolVerdict::Continue
        })
    }
}
/// Authored only against `huginn-plugin-api`.
struct Author;
#[async_trait]
impl Plugin for Author {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "author".into(),
            provides: vec![],
            requires: vec![tool_hooks_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let hooks = ctx.resolve::<ToolHooksService>(&tool_hooks_id())?.0.clone();
        hooks.register(ctx, "author.deny-shell".into(), Arc::new(DenyShell))
    }
}

#[tokio::test]
async fn authors_register_restrict_only_hooks_through_the_plugin_api() {
    let hooks = Arc::new(ToolHooks::default());
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(Host(hooks.clone()))).unwrap();
    supervisor.register(Box::new(Author)).unwrap();
    supervisor.start().await.unwrap();
    let ctx = ToolContext {
        selected_skills: vec![],
        workspace: ".".into(),
        cancellation: Default::default(),
    };
    let call = |name: &str| ToolCall {
        id: "id".into(),
        name: name.into(),
        arguments: Default::default(),
    };
    assert!(hooks.before(&call("read_file"), &ctx).await.is_none());
    let blocked = hooks.before(&call("shell"), &ctx).await.unwrap();
    assert_eq!(blocked.hook, "author.deny-shell");
    supervisor.shutdown().await;
}
