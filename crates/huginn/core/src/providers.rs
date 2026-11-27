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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

impl std::fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ReasoningEffort {
    type Err = ();

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            _ => Err(()),
        }
    }
}

/// Whether `endpoint` is an official OpenAI Platform API base URL.
pub fn is_official_openai_api_endpoint(endpoint: &str) -> bool {
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };
    if url.scheme() != "https"
        || url.host_str() != Some("api.openai.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    matches!(url.path().trim_end_matches('/'), "" | "/v1")
}

fn is_iso_date_suffix(suffix: &str) -> bool {
    let bytes = suffix.as_bytes();
    bytes.len() == 10
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

fn matches_base_or_dated_snapshot(normalized: &str, base: &str) -> bool {
    normalized == base
        || normalized
            .strip_prefix(base)
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(is_iso_date_suffix)
}

/// Conservative allowlist of OpenAI API models verified for Chat Completions `reasoning_effort`.
///
/// Explicitly excludes Responses-only models (`o3-pro`, `o3-deep-research`,
/// `o4-mini-deep-research`, `gpt-5-pro`, `gpt-5-codex`, `o1-pro`) and non-reasoning
/// aliases (`o1-mini`, `o1-preview`, `gpt-5-chat-latest`).
pub fn is_verified_openai_reasoning_model(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    const VERIFIED_CHAT_COMPLETIONS_BASES: &[&str] = &[
        "o1",
        "o3",
        "o3-mini",
        "o4-mini",
        "gpt-5",
        "gpt-5-mini",
        "gpt-5-nano",
    ];
    VERIFIED_CHAT_COMPLETIONS_BASES
        .iter()
        .any(|base| matches_base_or_dated_snapshot(&normalized, base))
}

/// Whether the given provider, endpoint, and model support verified OpenAI Chat Completions
/// `reasoning_effort` selection.
pub fn supports_openai_reasoning_effort(
    provider: &str,
    endpoint: Option<&str>,
    model: &str,
) -> bool {
    provider == "openai"
        && endpoint.is_some_and(is_official_openai_api_endpoint)
        && is_verified_openai_reasoning_model(model)
}

/// Conservative allowlist of OpenAI API models verified for Chat Completions image input.
pub fn is_verified_openai_vision_model(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    if normalized == "gpt-4o"
        || (normalized.starts_with("gpt-4o-")
            && !normalized.starts_with("gpt-4o-audio")
            && !normalized.starts_with("gpt-4o-realtime")
            && !normalized.starts_with("gpt-4o-transcribe")
            && !normalized.starts_with("gpt-4o-mini-audio")
            && !normalized.starts_with("gpt-4o-mini-realtime")
            && !normalized.starts_with("gpt-4o-mini-transcribe")
            && !normalized.starts_with("gpt-4o-mini-tts"))
    {
        return true;
    }
    if normalized == "gpt-4.1" || normalized.starts_with("gpt-4.1-") {
        return true;
    }
    if normalized == "gpt-5" || normalized.starts_with("gpt-5-") {
        return true;
    }
    if (normalized == "o3" || normalized.starts_with("o3-"))
        && normalized != "o3-mini"
        && !normalized.starts_with("o3-mini-")
    {
        return true;
    }
    if normalized == "o4-mini" || normalized.starts_with("o4-mini-") {
        return true;
    }
    false
}

/// Whether the given provider, endpoint, and model support verified OpenAI Chat Completions
/// image input.
pub fn supports_openai_vision(provider: &str, endpoint: Option<&str>, model: &str) -> bool {
    provider == "openai"
        && endpoint.is_some_and(is_official_openai_api_endpoint)
        && is_verified_openai_vision_model(model)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSelection {
    pub connection: ConnectionIdentity,
    pub model: String,
    pub reasoning_effort: Option<ReasoningEffort>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
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

/// Maximum number of models accepted in a single discovered catalog.
pub const MAX_DISCOVERED_MODELS: usize = 16_384;

/// Public onboarding hints. Secret values must never occur in this metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderOnboarding {
    /// None uses descriptor methods. An override narrows interactive onboarding only.
    pub authentication: Option<Vec<AuthenticationMethod>>,
    pub default_endpoint: Option<String>,
    pub allow_endpoint_edit: bool,
    /// True only for adapters whose connection-scoped factory permits an explicit
    /// manual model ID when remote discovery returns `ModelCatalog::Unsupported`.
    pub allow_manual_model_when_unsupported: bool,
}
impl Default for ProviderOnboarding {
    fn default() -> Self {
        Self {
            authentication: None,
            default_endpoint: None,
            allow_endpoint_edit: true,
            allow_manual_model_when_unsupported: false,
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
