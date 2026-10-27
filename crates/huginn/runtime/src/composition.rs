//! Adapter composition. The application kernel contracts are re-exported for compatibility.
pub use huginn_application::composition::{
    instructions_id, loop_id, model_id, sessions_id, tools_id, HooksPlugin, InstructionsPlugin,
    LoopPlugin, ModelPlugin, SessionsPlugin, ToolsPlugin,
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

fn configured_home(home: Option<std::ffi::OsString>) -> Option<std::path::PathBuf> {
    home.filter(|home| !home.is_empty()).map(Into::into)
}

/// Production composition: each native tool is a lifecycle-owned capability.
pub async fn build_native(
    model: Arc<dyn ModelProvider>,
    registry: crate::tools::Registry,
    sessions: Arc<dyn SessionStore>,
    config: LoopConfig,
    timeout_secs: u64,
) -> Result<Supervisor> {
    build_native_plugin(
        Box::new(ModelPlugin(model)),
        registry,
        sessions,
        config,
        timeout_secs,
    )
    .await
}

pub async fn build_native_with_connections(
    model: Arc<dyn ModelProvider>,
    access: Arc<huginn_application::connections::ConnectionAccess>,
    registry: crate::tools::Registry,
    sessions: Arc<dyn SessionStore>,
    config: LoopConfig,
    timeout_secs: u64,
) -> Result<Supervisor> {
    build_native_plugin(
        Box::new(
            huginn_application::providers::ProvidersPlugin::with_credentials_and_legacy(
                access, model,
            ),
        ),
        registry,
        sessions,
        config,
        timeout_secs,
    )
    .await
}

async fn build_native_plugin(
    providers: Box<dyn huginn_core::kernel::Plugin>,
    registry: crate::tools::Registry,
    sessions: Arc<dyn SessionStore>,
    mut config: LoopConfig,
    timeout_secs: u64,
) -> Result<Supervisor> {
    let skills = Arc::new(crate::skills::FilesystemSkillLoader::new(configured_home(
        std::env::var_os("HOME"),
    )));
    config.skills = Some(skills.clone());
    let mut supervisor = Supervisor::default();
    supervisor.register(providers)?;
    supervisor.register(Box::new(crate::tools::RegistryPlugin(registry)))?;
    supervisor.register(Box::new(crate::tools::NativeToolPlugin(Arc::new(
        huginn_application::skills::SkillTool(skills),
    ))))?;
    for tool in crate::tools::native_tools(timeout_secs) {
        supervisor.register(Box::new(crate::tools::NativeToolPlugin(tool)))?;
    }
    supervisor.register(Box::new(SessionsPlugin(sessions)))?;
    supervisor.register(Box::new(InstructionsPlugin(Arc::new(
        crate::instructions::FilesystemInstructions,
    ))))?;
    supervisor.register(Box::new(HooksPlugin(Arc::default())))?;
    supervisor.register(Box::new(LoopPlugin(config)))?;
    if let Err(error) = supervisor.start().await {
        supervisor.shutdown().await;
        return Err(error);
    }
    Ok(supervisor)
}

#[cfg(test)]
mod tests {
    use super::configured_home;
    use std::{ffi::OsString, path::PathBuf};

    #[test]
    fn empty_home_is_treated_as_unset() {
        assert_eq!(configured_home(Some(OsString::new())), None);
    }

    #[test]
    fn nonempty_home_is_preserved() {
        assert_eq!(
            configured_home(Some(OsString::from("/home/test"))),
            Some(PathBuf::from("/home/test"))
        );
    }
}
