//! Tool registration controlled by the plugin lifecycle.
use crate::{kernel::PluginContext, kernel::ServiceId, AgentError, Result, Tool};
use std::sync::Arc;

pub fn tool_registration_id() -> ServiceId {
    ServiceId::new("agent:tool-registration", 1)
}

/// The adapter reserves the name and publishes it only after the context commits.
/// The context owns revocation; registration does not grant execution permissions.
pub trait ToolRegistration: Send + Sync {
    fn register(&self, ctx: &mut PluginContext, tool: Arc<dyn Tool>) -> Result<()>;
}

/// The caller uses `register`; the inner port is not part of the supported API.
///
/// ```compile_fail,E0616
/// use huginn_core::tools::ToolRegistrationService;
/// fn direct_access(service: &ToolRegistrationService) {
///     let _ = &service.0;
/// }
/// ```
pub struct ToolRegistrationService(Arc<dyn ToolRegistration>);
impl ToolRegistrationService {
    /// The host builds the service from its registration adapter.
    pub fn new(port: Arc<dyn ToolRegistration>) -> Self {
        Self(port)
    }

    /// Requires declaring the dependency and using the instance published by the host.
    pub fn register(&self, ctx: &mut PluginContext, tool: Arc<dyn Tool>) -> Result<()> {
        let service = ctx.resolve::<Self>(&tool_registration_id())?;
        if !Arc::ptr_eq(&self.0, &service.0) {
            return Err(AgentError::Composition(
                "tool registration service does not match declared dependency".into(),
            ));
        }
        self.0.register(ctx, tool)
    }
}
