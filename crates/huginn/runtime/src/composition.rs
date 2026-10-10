//! Adapter composition. The application kernel contracts are re-exported for compatibility.
pub use huginn_application::composition::{
    instructions_id, loop_id, model_id, sessions_id, tools_id, InstructionsPlugin, LoopPlugin,
    ModelPlugin, SessionsPlugin, ToolsPlugin,
};
use huginn_application::loop_engine::LoopConfig;
use huginn_core::{kernel::Supervisor, ModelProvider, Result, SessionStore, ToolRegistry};
use std::sync::Arc;
pub async fn build(
    model: Arc<dyn ModelProvider>,
    tools: Arc<dyn ToolRegistry>,
    sessions: Arc<dyn SessionStore>,
    config: LoopConfig,
) -> Result<Supervisor> {
    huginn_application::composition::build(
        model,
        tools,
        sessions,
        config,
        Arc::new(crate::instructions::FilesystemInstructions),
    )
    .await
}

/// Production composition: each native tool is a lifecycle-owned capability.
pub async fn build_native(
    model: Arc<dyn ModelProvider>,
    registry: crate::tools::Registry,
    sessions: Arc<dyn SessionStore>,
    config: LoopConfig,
    timeout_secs: u64,
) -> Result<Supervisor> {
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(ModelPlugin(model)))?;
    supervisor.register(Box::new(crate::tools::RegistryPlugin(registry)))?;
    for tool in crate::tools::native_tools(timeout_secs) {
        supervisor.register(Box::new(crate::tools::NativeToolPlugin(tool)))?;
    }
    supervisor.register(Box::new(SessionsPlugin(sessions)))?;
    supervisor.register(Box::new(InstructionsPlugin(Arc::new(
        crate::instructions::FilesystemInstructions,
    ))))?;
    supervisor.register(Box::new(LoopPlugin(config)))?;
    if let Err(error) = supervisor.start().await {
        supervisor.shutdown().await;
        return Err(error);
    }
    Ok(supervisor)
}
