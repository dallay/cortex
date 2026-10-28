use crate::Guardrails;
use async_trait::async_trait;
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext},
    v1::hooks::{tool_hooks_id, ToolCall, ToolContext, ToolHook, ToolHooksService, ToolVerdict},
    Result,
};
use std::{path::Path, sync::Arc};

pub struct GuardsPlugin(pub Arc<Guardrails>);
struct GuardHook(Arc<Guardrails>);
#[async_trait]
impl Plugin for GuardsPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "guards.native".into(),
            provides: vec![],
            requires: vec![tool_hooks_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.resolve::<ToolHooksService>(&tool_hooks_id())?
            .0
            .register(
                ctx,
                "guards.native".into(),
                Arc::new(GuardHook(self.0.clone())),
            )
    }
}
#[async_trait]
impl ToolHook for GuardHook {
    async fn before(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolVerdict> {
        let verdict = match call.name.as_str() {
            "shell" => call.arguments["command"]
                .as_str()
                .ok_or("guardrails: missing shell command")
                .and_then(|command| self.0.check_shell_in(&ctx.workspace, command)),
            "read_file" | "write_file" | "edit_file" | "list_files" | "search_files" => {
                self.0.check_path(
                    &ctx.workspace,
                    Path::new(call.arguments["path"].as_str().unwrap_or(".")),
                )
            }
            _ => Ok(()),
        };
        Ok(match verdict {
            Ok(()) => ToolVerdict::Continue,
            Err(reason) => ToolVerdict::Block(reason.into()),
        })
    }
}
