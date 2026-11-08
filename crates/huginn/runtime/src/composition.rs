//! Adapter composition. The application kernel contracts are re-exported for compatibility.
pub use huginn_application::composition::{
    instructions_id, loop_id, model_id, sessions_id, tools_id, HooksPlugin, InferenceHooksPlugin,
    InstructionsPlugin, LoopPlugin, ModelPlugin, SessionsPlugin, ToolsPlugin,
};
use huginn_application::{composition::leased_loop_plugin, loop_engine::LoopConfig};
use huginn_core::{
    kernel::Supervisor, ModelProvider, Result, SessionStore, SteeringQueue, ToolRegistry,
};
use std::sync::Arc;

/// Install the host's single steering queue in the loop config and return the same
/// handle for the presentation.
///
/// Sharing one queue is the contract: the composer writes into it while a turn is
/// busy and the loop drains it at step boundaries. Hosts must pass the returned
/// handle to the Ratatui plugin instead of creating a second mailbox.
pub fn shared_steering(config: &mut LoopConfig) -> Arc<SteeringQueue> {
    let steering = Arc::new(SteeringQueue::default());
    config.steering = Some(steering.clone());
    steering
}
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
        None,
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
        None,
    )
    .await
}

/// Guarded composition; the policy is a snapshot selected by the host.
pub async fn build_native_with_connections_and_guards(
    model: Arc<dyn ModelProvider>,
    access: Arc<huginn_application::connections::ConnectionAccess>,
    registry: crate::tools::Registry,
    sessions: Arc<dyn SessionStore>,
    config: LoopConfig,
    timeout_secs: u64,
    guards: Arc<huginn_guards::Guardrails>,
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
        Some(guards),
    )
    .await
}

async fn build_native_plugin(
    providers: Box<dyn huginn_core::kernel::Plugin>,
    registry: crate::tools::Registry,
    sessions: Arc<dyn SessionStore>,
    mut config: LoopConfig,
    timeout_secs: u64,
    guards: Option<Arc<huginn_guards::Guardrails>>,
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
    let tools = guards.as_ref().map_or_else(
        || crate::tools::native_tools(timeout_secs),
        |policy| crate::tools::native_tools_with_guards(timeout_secs, policy.clone()),
    );
    let (file_builtins, other_tools): (Vec<_>, Vec<_>) = tools
        .into_iter()
        .partition(|tool| matches!(tool.definition().name.as_str(), "write_file" | "edit_file"));
    if !file_builtins.is_empty() {
        supervisor.register(Box::new(crate::tools::NativeBuiltinToolPlugin(
            file_builtins,
        )))?;
    }
    for tool in other_tools {
        supervisor.register(Box::new(crate::tools::NativeToolPlugin(tool)))?;
    }
    supervisor.register(Box::new(SessionsPlugin(sessions)))?;
    supervisor.register(Box::new(InstructionsPlugin(Arc::new(
        crate::instructions::FilesystemInstructions,
    ))))?;
    supervisor.register(Box::new(HooksPlugin(Arc::default())))?;
    supervisor.register(Box::new(InferenceHooksPlugin(Arc::default())))?;
    if let Some(guards) = guards {
        supervisor.register(Box::new(huginn_guards::GuardsPlugin(guards)))?;
    }
    supervisor.register(leased_loop_plugin(config))?;
    if let Err(error) = supervisor.start().await {
        supervisor.shutdown().await;
        return Err(error);
    }
    Ok(supervisor)
}

#[cfg(test)]
mod tests {
    use super::{configured_home, shared_steering};
    use huginn_application::loop_engine::LoopConfig;
    use std::{ffi::OsString, path::PathBuf, sync::Arc};

    #[test]
    fn shared_steering_installs_the_returned_queue() {
        let mut config = LoopConfig::default();
        assert!(config.steering.is_none(), "steering starts unset");
        let steering = shared_steering(&mut config);
        let installed = config.steering.as_ref().expect("steering is installed");
        assert!(
            Arc::ptr_eq(installed, &steering),
            "the presentation must receive the loop's queue"
        );
        steering
            .push("narrow the change")
            .expect("queue accepts a message");
        assert_eq!(installed.take(), vec!["narrow the change".to_string()]);
    }

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
