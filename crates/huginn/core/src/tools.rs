//! Registro de herramientas controlado por el ciclo de vida del plugin.
use crate::{kernel::PluginContext, kernel::ServiceId, AgentError, Result, Tool};
use std::sync::Arc;

pub fn tool_registration_id() -> ServiceId {
    ServiceId::new("agent:tool-registration", 1)
}

/// El adaptador reserva el nombre y publica solo tras el commit del contexto.
/// El contexto posee la revocación; registrar no concede permisos de ejecución.
pub trait ToolRegistration: Send + Sync {
    fn register(&self, ctx: &mut PluginContext, tool: Arc<dyn Tool>) -> Result<()>;
}

/// El autor usa `register`; el puerto interno no forma parte de la API soportada.
///
/// ```compile_fail,E0616
/// use huginn_core::tools::ToolRegistrationService;
/// fn acceso_directo(service: &ToolRegistrationService) {
///     let _ = &service.0;
/// }
/// ```
pub struct ToolRegistrationService(Arc<dyn ToolRegistration>);
impl ToolRegistrationService {
    /// El host construye el servicio a partir de su adaptador de registro.
    pub fn new(port: Arc<dyn ToolRegistration>) -> Self {
        Self(port)
    }

    /// Exige declarar la dependencia y utilizar la instancia publicada por el host.
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
