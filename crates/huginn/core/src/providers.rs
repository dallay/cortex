//! Non-secret, headless provider contracts. Native adapters remain trusted code.
use crate::{kernel::PluginContext, CancellationToken, ModelProvider, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
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
    /// Empty only for connection-scoped discovery; selection remains a candidate
    /// until the adapter validates it during turn pinning. Never means "any model".
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

/// Local preparation only; none of these phases asserts remote connectivity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionPhase {
    Unconfigured,
    Candidate,
    Selected,
    Revoked,
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionMode {
    Managed(crate::connections::SecretMode),
    Unmanaged,
    Legacy,
}
/// Deliberately excludes endpoints, credentials, references and capabilities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionSnapshot {
    pub provider: String,
    pub connection: String,
    pub model: String,
    pub mode: SelectionMode,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionSnapshot {
    pub source: uuid::Uuid,
    pub revision: u64,
    pub phase: ConnectionPhase,
    pub selection: Option<SelectionSnapshot>,
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

/// Public onboarding hints. Secret values must never occur in this metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderOnboarding {
    /// None uses descriptor methods. An override narrows interactive onboarding only.
    pub authentication: Option<Vec<AuthenticationMethod>>,
    pub default_endpoint: Option<String>,
    pub allow_endpoint_edit: bool,
}
impl Default for ProviderOnboarding {
    fn default() -> Self {
        Self {
            authentication: None,
            default_endpoint: None,
            allow_endpoint_edit: true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolSupport {
    Unknown,
    Supported,
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredModel {
    pub id: String,
    pub tools: ToolSupport,
}
/// An unsupported catalog is different from an unavailable or empty catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelCatalog {
    Available(Vec<DiscoveredModel>),
    Unsupported,
    Unavailable,
    Invalid,
}

/// Factories receive identity and model, never terminal ownership or raw secrets.
/// P0.1 uses in-memory doubles; credential-backed connections are a later port.
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    fn onboarding(&self) -> ProviderOnboarding {
        ProviderOnboarding::default()
    }
    async fn discover_models(
        &self,
        _material: &crate::connections::ConnectionMaterial,
        _cancel: CancellationToken,
    ) -> Result<ModelCatalog> {
        Ok(ModelCatalog::Unsupported)
    }
    /// An empty static model list is permitted only for adapters that validate
    /// the selected model against a connection-scoped catalog during pinning.
    fn has_connection_catalog(&self) -> bool {
        false
    }
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

#[async_trait]
pub trait ProviderCatalog: Send + Sync {
    fn onboarding(&self, _registration: &ProviderRegistration) -> Result<ProviderOnboarding> {
        Ok(ProviderOnboarding::default())
    }
    async fn discover_models(
        &self,
        _registration: &ProviderRegistration,
        _material: &crate::connections::ConnectionMaterial,
        _cancel: CancellationToken,
    ) -> Result<ModelCatalog> {
        Ok(ModelCatalog::Unsupported)
    }
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
    /// Installs an exact candidate, never a confirmed selection or new admission.
    StageSelection(ModelSelection),
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
    Staged(ModelSelection),
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
    fn subscribe(&self) -> tokio::sync::watch::Receiver<ConnectionSnapshot>;
    /// Synchronous commit under the router's serialization point.
    fn publish_selection(&self, selection: &ModelSelection, mode: SelectionMode) -> Result<()>;
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
