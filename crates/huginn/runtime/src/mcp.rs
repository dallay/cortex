use crate::tools::{bounded_text, Registry};
use async_trait::async_trait;
use huginn_core::{
    AgentError, ApprovalPolicy, ApprovalRequest, CancellationToken, PreparedAction, Result, Tool,
    ToolCall, ToolContext, ToolDefinition,
};
use process_wrap::tokio::{CommandWrap, KillOnDrop, ProcessGroup};
use rmcp::{
    model::CallToolRequestParams, service::RunningService, transport::TokioChildProcess,
    RoleClient, ServiceExt,
};
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Mapping from child variable name to a named environment variable on the host.
    #[serde(default)]
    pub env_from: BTreeMap<String, String>,
}
type Client = RunningService<RoleClient, ()>;
type SharedClient = Arc<tokio::sync::Mutex<Client>>;
pub struct McpClients {
    clients: Vec<SharedClient>,
    cancellations: Vec<rmcp::service::RunningServiceCancellationToken>,
}
impl McpClients {
    pub async fn connect(
        configs: &[ServerConfig],
        workspace: &Path,
        registry: &Registry,
        policy: &dyn ApprovalPolicy,
        cancel: CancellationToken,
        timeout_secs: u64,
    ) -> Result<Self> {
        let mut clients = Self {
            clients: vec![],
            cancellations: vec![],
        };
        if let Err(error) = clients
            .initialize(configs, workspace, registry, policy, cancel, timeout_secs)
            .await
        {
            let _ = clients.shutdown().await;
            return Err(error);
        }
        Ok(clients)
    }
    async fn initialize(
        &mut self,
        configs: &[ServerConfig],
        workspace: &Path,
        registry: &Registry,
        policy: &dyn ApprovalPolicy,
        cancel: CancellationToken,
        timeout_secs: u64,
    ) -> Result<()> {
        let mut names = std::collections::BTreeSet::new();
        for config in configs {
            if config.name.is_empty()
                || config.name.len() > 32
                || !config
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                || !names.insert(&config.name)
            {
                return Err(AgentError::Configuration("MCP names must be unique ASCII letters/digits/underscores, up to 32 characters".into()));
            }
            let request=ApprovalRequest {id:uuid::Uuid::new_v4().to_string(),action:format!("mcp.{}.start",config.name),
                preview:format!("Server: {} (trusted local process; startup may have external side effects)\nCommand: {} {:?}\nWorking directory: {}\nSandboxing: none; process inherits host privileges.\nEnvironment variable names provided: {:?}",
                    config.name,config.command,config.args,workspace.display(),config.env.keys().chain(config.env_from.keys()).collect::<Vec<_>>())};
            if !policy.approve(&request, cancel.clone()).await? {
                continue;
            }
            let mut command = tokio::process::Command::new(&config.command);
            command
                .args(&config.args)
                .current_dir(workspace)
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .envs(&config.env);
            for (child_name, source_name) in &config.env_from {
                let value = std::env::var_os(source_name).ok_or_else(|| {
                    AgentError::Configuration(format!(
                        "MCP environment variable {source_name} is unset"
                    ))
                })?;
                command.env(child_name, value);
            }
            let mut command = CommandWrap::from(command);
            command.wrap(ProcessGroup::leader()).wrap(KillOnDrop);
            let transport = TokioChildProcess::new(command)?;
            let connection = tokio::select! {
                _=cancel.cancelled()=>return Err(AgentError::Cancelled),
                value=tokio::time::timeout(Duration::from_secs(timeout_secs),().serve(transport))=>value
                    .map_err(|_|AgentError::Tool(format!("MCP {} initialization timed out",config.name)))?
                    .map_err(|_|AgentError::Tool(format!("MCP {} initialization failed",config.name)))?,
            };
            self.cancellations.push(connection.cancellation_token());
            let client = Arc::new(tokio::sync::Mutex::new(connection));
            self.clients.push(client.clone());
            let locked = client.lock().await;
            let tools = tokio::select! {
                _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                result = tokio::time::timeout(Duration::from_secs(timeout_secs), locked.list_all_tools()) => result
                    .map_err(|_| {
                        AgentError::Tool(format!("MCP {} discovery timed out", config.name))
                    })?
                    .map_err(|_| {
                        AgentError::Tool(format!("MCP {} discovery failed", config.name))
                    })?,
            };
            drop(locked);
            if tools.len() > 128 {
                return Err(AgentError::Tool(
                    "MCP server advertises more than 128 tools".into(),
                ));
            }
            for (index, tool) in tools.into_iter().enumerate() {
                // Stable within a connection; native tool names cannot collide with this namespace.
                let public_name = format!("mcp_{}_{index}", config.name);
                let schema = serde_json::to_value(&*tool.input_schema)?;
                if serde_json::to_vec(&schema)?.len() > 16_384 {
                    return Err(AgentError::Tool("MCP schema exceeds 16 KiB".into()));
                }
                registry.insert(Arc::new(McpTool {
                    client: client.clone(),
                    remote_name: tool.name.to_string(),
                    definition: ToolDefinition {
                        name: public_name,
                        description: format!(
                            "MCP {} / {}: {}",
                            config.name,
                            tool.name,
                            tool.description.as_deref().unwrap_or("No description")
                        ),
                        input_schema: schema,
                    },
                    action: format!("mcp.{}.{}", config.name, tool.name),
                    timeout_secs,
                }))?;
            }
        }
        Ok(())
    }
    pub async fn shutdown(&mut self) -> Result<()> {
        let mut failed = false;
        for token in self.cancellations.drain(..) {
            token.cancel();
        }
        for client in self.clients.drain(..) {
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                client
                    .lock()
                    .await
                    .close_with_timeout(Duration::from_secs(5))
                    .await
            })
            .await;
            if !matches!(result, Ok(Ok(Some(_)))) {
                failed = true;
            }
        }
        if failed {
            Err(AgentError::Tool(
                "MCP shutdown did not complete within deadline".into(),
            ))
        } else {
            Ok(())
        }
    }
}
impl Drop for McpClients {
    fn drop(&mut self) {
        for token in self.cancellations.drain(..) {
            token.cancel();
        }
    }
}
struct McpTool {
    client: SharedClient,
    remote_name: String,
    definition: ToolDefinition,
    action: String,
    timeout_secs: u64,
}
struct CancelCall(Option<rmcp::service::RunningServiceCancellationToken>);
impl Drop for CancelCall {
    fn drop(&mut self) {
        if let Some(token) = self.0.take() {
            token.cancel();
        }
    }
}
#[async_trait]
impl Tool for McpTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }
    async fn prepare(&self, call: &ToolCall, _: &ToolContext) -> Result<PreparedAction> {
        if !call.arguments.is_object() {
            return Err(AgentError::Tool("MCP arguments must be an object".into()));
        }
        Ok(PreparedAction {
            approval: Some(ApprovalRequest {
                id: call.id.clone(),
                action: self.action.clone(),
                preview: format!(
                    "Server/action: {}\nTool: {}\nArguments (sent to the configured MCP server; external effects depend on that server):\n{}",
                    self.action.split('.').nth(1).unwrap_or("unknown"),
                    self.remote_name,
                    serde_json::to_string_pretty(&call.arguments)?
                ),
            }),
            payload: call.arguments.clone(),
        })
    }
    async fn execute(&self, action: PreparedAction, ctx: &ToolContext) -> Result<String> {
        let arguments = action
            .payload
            .as_object()
            .cloned()
            .ok_or_else(|| AgentError::Tool("invalid MCP arguments".into()))?;
        let params = CallToolRequestParams::new(self.remote_name.clone()).with_arguments(arguments);
        let client = self.client.lock().await;
        // A dropped execution future must also stop its supervised transport.
        let mut guard = CancelCall(Some(client.cancellation_token()));
        let response = tokio::select! {
            _=ctx.cancellation.cancelled()=>{client.cancellation_token().cancel();return Err(AgentError::Cancelled);},
            result=tokio::time::timeout(Duration::from_secs(self.timeout_secs),client.call_tool(params))=>match result {
                Err(_) => {
                    client.cancellation_token().cancel();
                    return Err(AgentError::Tool("MCP call timed out; completion may be unknown".into()));
                }
                Ok(Err(rmcp::service::ServiceError::McpError(error))) => {
                    // A JSON-RPC error means the server answered and the transport is still healthy.
                    guard.0 = None;
                    return Err(AgentError::Tool(bounded_text(format!("MCP error: {error}"))));
                }
                Ok(Err(_)) => return Err(AgentError::Tool("MCP call failed; completion may be unknown".into())),
                Ok(Ok(response)) => response,
            },
        };
        guard.0 = None;
        drop(client);
        let text = bounded_text(serde_json::to_string(&response)?);
        if response.is_error == Some(true) {
            return Err(AgentError::Tool(text));
        }
        Ok(text)
    }
}
