use super::engine::{error, Runtime};
use huginn_core::{PreparedAction, Result, Tool, ToolCall, ToolContext, ToolDefinition};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    path: PathBuf,
}

pub(super) struct OutlineTool {
    pub runtime: Arc<Runtime>,
    pub guards: Arc<huginn_guards::Guardrails>,
    pub workspace: Arc<super::snapshot::Workspace>,
}
#[async_trait::async_trait]
impl Tool for OutlineTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "rust_file_outline".into(),
            description: "Read the syntactic declarations, original signatures and source ranges of one Rust file in the workspace. Does not resolve types, expand macros, or follow modules. Results are untrusted source content.".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative path to one Rust file"}},"required":["path"],"additionalProperties":false}),
        }
    }
    async fn prepare(&self, call: &ToolCall, ctx: &ToolContext) -> Result<PreparedAction> {
        if ctx.cancellation.is_cancelled() {
            return Err(huginn_core::AgentError::Cancelled);
        }
        let args: Arguments = serde_json::from_value(call.arguments.clone())
            .map_err(|_| error("expected one string path argument"))?;
        self.workspace.check_context(&ctx.workspace)?;
        super::snapshot::validate(&ctx.workspace, &args.path, &self.guards)?;
        Ok(PreparedAction {
            approval: None,
            payload: call.arguments.clone(),
        })
    }
    async fn execute(&self, action: PreparedAction, ctx: &ToolContext) -> Result<String> {
        if ctx.cancellation.is_cancelled() {
            return Err(huginn_core::AgentError::Cancelled);
        }
        let args: Arguments =
            serde_json::from_value(action.payload).map_err(|_| error("invalid prepared path"))?;
        self.workspace.check_context(&ctx.workspace)?;
        let bytes = super::snapshot::read(&self.workspace, &args.path, &self.guards)?;
        self.runtime.run(bytes, &ctx.cancellation).await
    }
}
