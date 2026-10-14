//! Non-secret, headless provider contracts. Native adapters remain trusted code.
use crate::{kernel::PluginContext, CancellationToken, ModelProvider, Result};
use async_trait::async_trait;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelDescriptor {
    pub id: String,
    pub supports_tools: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthenticationMethod {
    None,
    ApiKey,
    /// Describes a capability, not an implemented or authorized generic flow.
    OAuth,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub id: String,
    pub display_name: String,
    pub models: Vec<ModelDescriptor>,
    pub authentication: Vec<AuthenticationMethod>,
}

/// Exact registration, not a name-based lookup that can adopt a replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRegistration {
    pub provider: String,
    pub generation: u64,
    identity: uuid::Uuid,
}
impl ProviderRegistration {
    /// Mint an identity even when separate supervisors reuse generation numbers.
    pub fn new(provider: String, generation: u64) -> Self {
        Self {
            provider,
            generation,
            identity: uuid::Uuid::new_v4(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionIdentity {
    pub id: String,
    pub adapter: ProviderRegistration,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSelection {
    pub connection: ConnectionIdentity,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("no model configured; connect a provider and select a model")]
    Unconfigured,
    #[error("provider registration or connection revoked; remote completion is unknown")]
    Revoked,
    #[error("provider or connection identity is unavailable")]
    Unavailable,
    #[error("provider or connection already registered")]
    Duplicate,
    #[error("model is not available for this connection")]
    InvalidModel,
    #[error("invalid provider metadata")]
    InvalidMetadata,
}

/// Factories receive identity and model, never terminal ownership or raw secrets.
/// P0.1 uses in-memory doubles; credential-backed connections are a later port.
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    /// Cancellation is scoped to factory work and ends on success too. The
    /// returned model must use the fresh token supplied to each stream call,
    /// not retain this token as the authorization/lifetime of the whole model.
    async fn create_model(
        &self,
        selection: &ModelSelection,
        cancel: CancellationToken,
    ) -> Result<Arc<dyn ModelProvider>>;
    /// P0.2 trusted boundary; raw credentials never enter presentation actions.
    async fn create_connection_model(
        &self,
        _selection: &ModelSelection,
        _material: crate::connections::ConnectionMaterial,
        _cancel: CancellationToken,
    ) -> Result<Arc<dyn ModelProvider>> {
        // Even no-auth material carries routing settings that legacy factories cannot honor.
        Err(ProviderError::Unavailable.into())
    }
}

pub trait ProviderCatalog: Send + Sync {
    fn adapters(&self) -> Vec<(ProviderRegistration, ProviderDescriptor)>;
    /// Reserve invisibly, then publish only when the host commits activation.
    fn register(
        &self,
        ctx: &mut PluginContext,
        adapter: Arc<dyn ProviderAdapter>,
    ) -> Result<ProviderRegistration>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvidersDiagnostic {
    pub adapters: Vec<(ProviderRegistration, ProviderDescriptor)>,
    pub connections: Vec<ConnectionIdentity>,
    pub selection: Option<ModelSelection>,
    pub ready: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderAction {
    Diagnose,
    /// Ephemeral authorization only; no authentication or secret persistence.
    Connect {
        id: String,
        adapter: ProviderRegistration,
    },
    Select(ModelSelection),
    Revoke(ConnectionIdentity),
    Replace {
        connection: ConnectionIdentity,
        adapter: ProviderRegistration,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderActionOutput {
    Diagnostic(ProvidersDiagnostic),
    Connected(ConnectionIdentity),
    Selected(ModelSelection),
    Revoked,
}

/// Presentation may expose these states without performing I/O in callbacks.
/// The host sets Loading, awaits execute, and maps its result to a terminal state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderActionState {
    Loading,
    Success(ProviderActionOutput),
    Error(ProviderError),
    Cancelled,
}

#[async_trait]
pub trait ProviderManagement: Send + Sync {
    fn diagnostic(&self) -> Result<ProvidersDiagnostic>;
    /// Stop new selected admission without revoking already pinned turns.
    fn clear_selection(&self) -> Result<()>;
    /// Synchronous, exact-identity revocation for cancellation/drop recovery.
    fn invalidate(&self, identity: &ConnectionIdentity) -> Result<()>;
    async fn execute(
        &self,
        action: ProviderAction,
        cancel: CancellationToken,
    ) -> Result<ProviderActionOutput>;
}

pub struct ProviderCatalogService(pub Arc<dyn ProviderCatalog>);
pub struct ProvidersService(pub Arc<dyn ProviderManagement>);
