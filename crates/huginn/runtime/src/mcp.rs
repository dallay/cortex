use crate::tools::{bounded_text, Registry, ToolLease};
use async_trait::async_trait;
use huginn_core::{
    AgentError, ApprovalDecision, ApprovalPolicy, ApprovalRequest, CancellationToken,
    PreparedAction, Result, Tool, ToolCall, ToolContext, ToolDefinition,
};
use process_wrap::tokio::{CommandWrap, KillOnDrop, ProcessGroup};
use rmcp::{
    model::CallToolRequestParams, service::RunningService, transport::TokioChildProcess,
    RoleClient, ServiceExt,
};
use serde::Deserialize;
use std::future::Future;
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
#[derive(Default)]
pub struct McpClients {
    clients: Vec<SharedClient>,
    tools: Vec<Arc<dyn Tool>>,
    leases: Vec<ToolLease>,
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
        Self::connect_with(
            configs,
            workspace,
            registry,
            policy,
            cancel,
            timeout_secs,
            |config| async move {
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
                ().serve(transport).await.map_err(|_| {
                    AgentError::Tool(format!("MCP {} initialization failed", config.name))
                })
            },
        )
        .await
    }

    async fn connect_with<F, Fut>(
        configs: &[ServerConfig],
        workspace: &Path,
        registry: &Registry,
        policy: &dyn ApprovalPolicy,
        cancel: CancellationToken,
        timeout_secs: u64,
        mut connector: F,
    ) -> Result<Self>
    where
        F: FnMut(ServerConfig) -> Fut,
        Fut: Future<Output = Result<Client>>,
    {
        let mut clients = Self::default();
        if let Err(error) = clients
            .initialize_with(
                configs,
                workspace,
                policy,
                cancel,
                timeout_secs,
                &mut connector,
            )
            .await
        {
            let _ = clients.shutdown().await;
            return Err(error);
        }
        match registry.batch(std::mem::take(&mut clients.tools)) {
            Ok(leases) => clients.leases = leases,
            Err(error) => {
                let _ = clients.shutdown().await;
                return Err(error);
            }
        }
        Ok(clients)
    }
    async fn discover(
        configs: &[ServerConfig],
        workspace: &Path,
        policy: &dyn ApprovalPolicy,
        cancel: CancellationToken,
        timeout_secs: u64,
    ) -> Result<Self> {
        let mut clients = Self {
            clients: vec![],
            cancellations: vec![],
            tools: vec![],
            leases: vec![],
        };
        if let Err(error) = clients
            .initialize(configs, workspace, policy, cancel, timeout_secs)
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
        policy: &dyn ApprovalPolicy,
        cancel: CancellationToken,
        timeout_secs: u64,
    ) -> Result<()> {
        self.initialize_with(
            configs,
            workspace,
            policy,
            cancel,
            timeout_secs,
            &mut |config| async move {
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
                ().serve(transport).await.map_err(|_| {
                    AgentError::Tool(format!("MCP {} initialization failed", config.name))
                })
            },
        )
        .await
    }

    async fn initialize_with<F, Fut>(
        &mut self,
        configs: &[ServerConfig],
        workspace: &Path,
        policy: &dyn ApprovalPolicy,
        cancel: CancellationToken,
        timeout_secs: u64,
        connector: &mut F,
    ) -> Result<()>
    where
        F: FnMut(ServerConfig) -> Fut,
        Fut: Future<Output = Result<Client>>,
    {
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
            let request = ApprovalRequest {
                id: uuid::Uuid::new_v4().to_string(),
                action: format!("mcp.{}.start", config.name),
                preview: format!("Server: {} (trusted local process; startup may have external side effects)\nCommand: {} {:?}\nWorking directory: {}\nSandboxing: none; process inherits host privileges.\nEnvironment variable names provided: {:?}",
                                 config.name, config.command, config.args, workspace.display(), config.env.keys().chain(config.env_from.keys()).collect::<Vec<_>>()),
                resource: None,
                provenance: None,
            };
            if policy.approve(&request, cancel.clone()).await? != ApprovalDecision::ApproveOnce {
                continue;
            }
            let connection = tokio::select! {
                _=cancel.cancelled()=>return Err(AgentError::Cancelled),
                value=tokio::time::timeout(Duration::from_secs(timeout_secs),connector(config.clone()))=>value
                    .map_err(|_|AgentError::Tool(format!("MCP {} initialization timed out",config.name)))?
                    ?,
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
                self.tools.push(Arc::new(McpTool {
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
                }));
            }
        }
        Ok(())
    }
    pub async fn shutdown(&mut self) -> Result<()> {
        self.leases.clear();
        self.tools.clear();
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
                resource: None,
                provenance: None,
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

/// Each server participates in the kernel lifecycle, including discovery rollback.
pub struct McpPlugin {
    pub config: ServerConfig,
    pub workspace: std::path::PathBuf,
    pub policy: Arc<dyn ApprovalPolicy>,
    pub cancel: CancellationToken,
    pub timeout_secs: u64,
}
#[async_trait]
impl huginn_core::kernel::Plugin for McpPlugin {
    fn manifest(&self) -> huginn_core::kernel::Manifest {
        huginn_core::kernel::Manifest {
            id: format!("mcp.{}", self.config.name),
            provides: vec![],
            requires: vec![crate::tools::registry_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut huginn_core::kernel::PluginContext) -> Result<()> {
        let registry = ctx.resolve::<Registry>(&crate::tools::registry_id())?;
        let mut clients = McpClients::discover(
            std::slice::from_ref(&self.config),
            &self.workspace,
            self.policy.as_ref(),
            self.cancel.clone(),
            self.timeout_secs,
        )
        .await?;
        // Own clients before any fallible registration. Failed activation waits for shutdown too.
        let tools = std::mem::take(&mut clients.tools);
        let cancel = ctx.cancellation.clone();
        ctx.own_task(tokio::spawn(async move {
            cancel.cancelled().await;
            let _ = clients.shutdown().await;
        }));
        registry.register_batch(ctx, tools)
    }
}
/// The host supplies its policy; presentation is never an authorization service.
pub fn register_plugins(
    supervisor: &mut huginn_core::kernel::Supervisor,
    configs: &[ServerConfig],
    workspace: &Path,
    policy: Arc<dyn ApprovalPolicy>,
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
            return Err(AgentError::Configuration(
                "MCP names must be unique ASCII letters/digits/underscores, up to 32 characters"
                    .into(),
            ));
        }
    }
    for config in configs {
        supervisor.register(Box::new(McpPlugin {
            config: config.clone(),
            workspace: workspace.into(),
            policy: policy.clone(),
            cancel: cancel.clone(),
            timeout_secs,
        }))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use huginn_application::authorization::HostPolicy;
    use huginn_core::{
        ApprovalDecision, ApprovalPrompt, ApprovalRequest, ToolCall, ToolContext, ToolRegistry,
    };
    use rmcp::ServiceExt;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    struct Decisions(Mutex<Vec<(String, bool)>>);

    struct FixedHandler(Arc<AtomicUsize>);

    #[async_trait]
    impl muninn_mcp::ToolHandler for FixedHandler {
        async fn call(
            &self,
            arguments: &serde_json::Value,
        ) -> std::result::Result<serde_json::Value, String> {
            if arguments.get("user_identifier").is_some() {
                return Err("unsupported argument".into());
            }
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::json!({"owner":"fixed-owner","items":["fixed-owner result"]}))
        }
    }

    #[async_trait]
    impl ApprovalPrompt for Decisions {
        async fn request_decision(
            &self,
            request: &ApprovalRequest,
            allowed: &[ApprovalDecision],
            _: CancellationToken,
        ) -> Result<ApprovalDecision> {
            let mut decisions = self.0.lock().unwrap();
            let approved = decisions
                .iter()
                .find(|(action, _)| action == &request.action)
                .map(|(_, approved)| *approved)
                .unwrap_or(false);
            decisions.push((request.action.clone(), approved));
            drop(decisions);
            // The answer must be one of the options the policy offers; any
            // other answer is rejected by the policy.
            let decision = if approved {
                ApprovalDecision::ApproveOnce
            } else {
                ApprovalDecision::Reject
            };
            Ok(if allowed.contains(&decision) {
                decision
            } else {
                ApprovalDecision::Reject
            })
        }
    }

    #[tokio::test]
    async fn actual_muninn_server_uses_huginn_startup_and_invocation_approvals() {
        let prompt = Arc::new(Decisions(Mutex::new(vec![
            ("mcp.denied.start".into(), false),
            ("mcp.memory.start".into(), true),
            ("mcp.memory.muninn_search_messages".into(), true),
            ("mcp.memory.muninn_search_preferences".into(), false),
        ])));
        let policy = HostPolicy::new(prompt.clone(), Default::default());
        let registry = Registry::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let (client_io, server_io) = tokio::io::duplex(16 * 1024);
        let handler_calls = calls.clone();
        let mut server = muninn_mcp::Server::default();
        for name in ["muninn_search_messages", "muninn_search_preferences"] {
            server
                .register(
                    muninn_mcp::Catalog::ScopedRetrieval,
                    name,
                    Box::new(FixedHandler(handler_calls.clone())),
                )
                .unwrap();
        }
        let server_task = tokio::spawn(async move {
            let (reader, writer) = tokio::io::split(server_io);
            muninn_mcp::serve(tokio::io::BufReader::new(reader), writer, &mut server).await
        });
        let connects = Arc::new(AtomicUsize::new(0));
        let connect_counter = connects.clone();
        let client_io = Arc::new(Mutex::new(Some(client_io)));
        let configs = [config("denied"), config("memory")];
        let mut clients = McpClients::connect_with(
            &configs,
            Path::new("."),
            &registry,
            &policy,
            CancellationToken::new(),
            5,
            move |_config| {
                let connects = connect_counter.clone();
                let client_io = client_io.clone();
                async move {
                    connects.fetch_add(1, Ordering::SeqCst);
                    let io = client_io.lock().unwrap().take().unwrap();
                    ().serve(io)
                        .await
                        .map_err(|e| AgentError::Tool(e.to_string()))
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(connects.load(Ordering::SeqCst), 1);
        assert_eq!(registry.definitions().len(), 2);

        let context = ToolContext {
            selected_skills: vec![],
            workspace: ".".into(),
            cancellation: CancellationToken::new(),
        };
        let messages = registry.get("mcp_memory_0").unwrap();
        let prepared = messages
            .prepare(
                &ToolCall {
                    id: "approved".into(),
                    name: "mcp_memory_0".into(),
                    arguments: serde_json::json!({"query":"x"}),
                },
                &context,
            )
            .await
            .unwrap();
        assert_eq!(
            policy
                .approve(
                    prepared.approval.as_ref().unwrap(),
                    context.cancellation.clone()
                )
                .await
                .unwrap(),
            ApprovalDecision::ApproveOnce
        );
        let output = messages.execute(prepared, &context).await.unwrap();
        assert!(output.contains("fixed-owner"));

        let client = clients.clients[0].lock().await;
        let scoped_attempt = client
            .call_tool(
                CallToolRequestParams::new("muninn_search_messages").with_arguments(
                    serde_json::json!({"query":"x", "user_identifier":"attacker"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        assert_eq!(scoped_attempt.is_error, Some(true));
        drop(client);

        let preferences = registry.get("mcp_memory_1").unwrap();
        let denied = preferences
            .prepare(
                &ToolCall {
                    id: "denied-call".into(),
                    name: "mcp_memory_1".into(),
                    arguments: serde_json::json!({"query":"x"}),
                },
                &context,
            )
            .await
            .unwrap();
        assert_eq!(
            policy
                .approve(
                    denied.approval.as_ref().unwrap(),
                    context.cancellation.clone()
                )
                .await
                .unwrap(),
            ApprovalDecision::Reject
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        clients.shutdown().await.unwrap();
        server_task.abort();
        let _ = server_task.await;
    }

    fn config(name: &str) -> ServerConfig {
        ServerConfig {
            name: name.into(),
            command: "unused".into(),
            args: vec![],
            env: Default::default(),
            env_from: Default::default(),
        }
    }
}
