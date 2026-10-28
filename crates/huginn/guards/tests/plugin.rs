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

#[tokio::test]
async fn guard_hook_checks_each_native_path_tool_and_rejects_missing_shell_commands() {
    let root = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        workspace: root.path().into(),
        selected_skills: vec![],
        cancellation: Default::default(),
    };
    let hooks = Arc::new(ToolHooks::default());
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(Hooks(hooks.clone()))).unwrap();
    supervisor
        .register(Box::new(GuardsPlugin(Arc::new(
            Guardrails::from_toml("block_paths=['private']").unwrap(),
        ))))
        .unwrap();
    supervisor.start().await.unwrap();
    for name in [
        "read_file",
        "write_file",
        "edit_file",
        "list_files",
        "search_files",
    ] {
        for path in [".env", "private/new"] {
            let call = ToolCall {
                id: "path".into(),
                name: name.into(),
                arguments: serde_json::json!({"path":path}),
            };
            let blocked = hooks.before(&call, &ctx).await.unwrap();
            assert_eq!(blocked.hook, "guards.native");
            assert!(blocked.reason.contains("sensitive path blocked"));
        }
    }
    for arguments in [serde_json::json!({}), serde_json::json!({"command":42})] {
        let call = ToolCall {
            id: "missing".into(),
            name: "shell".into(),
            arguments,
        };
        assert_eq!(
            hooks.before(&call, &ctx).await.unwrap().reason,
            "guardrails: missing shell command"
        );
    }
    // Non-native tools retain their own validation and authorization boundaries.
    let call = ToolCall {
        id: "other".into(),
        name: "extension.custom".into(),
        arguments: serde_json::json!({"path":".env"}),
    };
    assert!(hooks.before(&call, &ctx).await.is_none());
    supervisor.shutdown().await;
}
