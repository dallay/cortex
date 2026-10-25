//! Connection-scoped built-in inference adapters. No environment credential lookup.
use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::providers::catalog_id;
use huginn_core::{
    connections::ConnectionMaterial,
    kernel::{Manifest, Plugin, PluginContext},
    providers::{
        AuthenticationMethod, ModelSelection, ProviderAdapter, ProviderCatalogService,
        ProviderDescriptor, ProviderError, ProviderOnboarding,
    },
    AgentError, CancellationToken, ModelDelta, ModelProvider, ModelRequest, Result,
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

pub use huginn_core::providers::{DiscoveredModel, ModelCatalog, ToolSupport};

// Permite catálogos de agregadores sin eliminar el límite de entradas ni el de bytes.
const MAX_DISCOVERED_MODELS: usize = 16_384;
const MAX_CATALOG_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy)]
enum Kind {
    OpenAi,
    Ollama,
    OllamaCloud,
}

pub struct InferenceAdapter {
    kind: Kind,
    timeout_secs: u64,
}
impl InferenceAdapter {
    pub const fn openai_compatible(timeout_secs: u64) -> Self {
        Self {
            kind: Kind::OpenAi,
            timeout_secs,
        }
    }
    pub const fn ollama(timeout_secs: u64) -> Self {
        Self {
            kind: Kind::Ollama,
            timeout_secs,
        }
    }
    pub const fn ollama_cloud(timeout_secs: u64) -> Self {
        Self {
            kind: Kind::OllamaCloud,
            timeout_secs,
        }
    }
    const fn id(&self) -> &'static str {
        match self.kind {
            Kind::OpenAi => "openai",
            Kind::Ollama => "ollama",
            Kind::OllamaCloud => "ollama-cloud",
        }
    }
    pub fn validate_connection(&self, material: &ConnectionMaterial) -> Result<()> {
        self.endpoint(material).map(|_| ())
    }
    fn endpoint<'a>(&self, material: &'a ConnectionMaterial) -> Result<&'a str> {
        material.settings.validate()?;
        if material.settings.provider != self.id()
            || (material.settings.requires_secret && material.secret.is_none())
        {
            return Err(ProviderError::Unavailable.into());
        }
        let endpoint =
            material.settings.endpoint.as_deref().ok_or_else(|| {
                AgentError::Configuration("connection endpoint is required".into())
            })?;
        // The local adapter cannot silently become a remote Ollama/cloud adapter.
        if matches!(self.kind, Kind::OllamaCloud) {
            let url = reqwest::Url::parse(endpoint).map_err(|_| ProviderError::Unavailable)?;
            // A cloud key must never be sent to a user-supplied host or path.
            if url.as_str() != "https://ollama.com/"
                || !material.settings.requires_secret
                || material.secret.is_none()
            {
                return Err(ProviderError::Unavailable.into());
            }
        }
        if matches!(self.kind, Kind::Ollama) {
            let url = reqwest::Url::parse(endpoint).map_err(|_| ProviderError::InvalidMetadata)?;
            if !huginn_core::connections::is_loopback_endpoint(&url)
                || !url.path().trim_end_matches('/').is_empty()
            {
                return Err(AgentError::Configuration(
                    "local Ollama requires a literal loopback root endpoint".into(),
                ));
            }
        }
        Ok(endpoint.trim_end_matches('/'))
    }
    async fn json(
        &self,
        material: &ConnectionMaterial,
        path: &str,
        body: Option<Value>,
        cancel: &CancellationToken,
    ) -> Result<(reqwest::StatusCode, Option<Value>)> {
        let endpoint = self.endpoint(material)?;
        let client = crate::model::transport_client(
            endpoint,
            self.timeout_secs,
            material.settings.allow_loopback_http,
        )?;
        let url = format!("{endpoint}{path}");
        let mut request = match body {
            Some(body) => client.post(url).json(&body),
            None => client.get(url),
        };
        if let Some(secret) = &material.secret {
            request = request.bearer_auth(secret.expose());
        }
        let operation = async {
            let response = request
                .send()
                .await
                .map_err(|_| ProviderError::Unavailable)?;
            let status = response.status();
            if !status.is_success() {
                return Ok((status, None));
            }
            let mut bytes = Vec::new();
            let mut chunks = response.bytes_stream();
            while let Some(chunk) = chunks.next().await {
                let chunk = chunk.map_err(|_| ProviderError::Unavailable)?;
                if bytes.len() + chunk.len() > MAX_CATALOG_RESPONSE_BYTES {
                    return Err(ProviderError::InvalidMetadata.into());
                }
                bytes.extend_from_slice(&chunk);
            }
            let value =
                serde_json::from_slice(&bytes).map_err(|_| ProviderError::InvalidMetadata)?;
            Ok((status, Some(value)))
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AgentError::Cancelled),
            result = tokio::time::timeout(Duration::from_secs(self.timeout_secs), operation) => result.map_err(|_| AgentError::from(ProviderError::Unavailable))?,
        }
    }
    pub async fn discover_models(
        &self,
        material: &ConnectionMaterial,
        cancel: CancellationToken,
    ) -> Result<ModelCatalog> {
        // Configuration failures remain actionable; network/catalog failures are explicit states.
        self.endpoint(material)?;
        let path = match self.kind {
            Kind::OpenAi => "/models",
            Kind::Ollama | Kind::OllamaCloud => "/api/tags",
        };
        let (status, value) = match self.json(material, path, None, &cancel).await {
            Ok(value) => value,
            Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
            Err(AgentError::Provider(ProviderError::InvalidMetadata)) => {
                return Ok(ModelCatalog::Invalid)
            }
            Err(_) => return Ok(ModelCatalog::Unavailable),
        };
        if matches!(status.as_u16(), 404 | 405 | 501) {
            return Ok(ModelCatalog::Unsupported);
        }
        if !status.is_success() {
            return Ok(ModelCatalog::Unavailable);
        }
        let value = value.unwrap_or(Value::Null);
        let (field, id) = match self.kind {
            Kind::OpenAi => ("data", "id"),
            Kind::Ollama | Kind::OllamaCloud => ("models", "name"),
        };
        let Some(items) = value[field]
            .as_array()
            .filter(|items| items.len() <= MAX_DISCOVERED_MODELS)
        else {
            return Ok(ModelCatalog::Invalid);
        };
        let mut models = Vec::new();
        let mut ids = std::collections::BTreeSet::new();
        for item in items {
            let Some(id) = item[id].as_str().filter(|id| {
                !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
            }) else {
                return Ok(ModelCatalog::Invalid);
            };
            if !ids.insert(id) {
                return Ok(ModelCatalog::Invalid);
            }
            models.push(DiscoveredModel {
                id: id.into(),
                tools: ToolSupport::Unknown,
            });
        }
        Ok(ModelCatalog::Available(models))
    }
}

#[async_trait]
impl ProviderAdapter for InferenceAdapter {
    fn onboarding(&self) -> ProviderOnboarding {
        ProviderOnboarding {
            authentication: if matches!(self.kind, Kind::Ollama) {
                Some(vec![AuthenticationMethod::None])
            } else {
                None
            },
            default_endpoint: match self.kind {
                Kind::OpenAi => None,
                Kind::Ollama => Some("http://127.0.0.1:11434".into()),
                Kind::OllamaCloud => Some("https://ollama.com".into()),
            },
            allow_endpoint_edit: !matches!(self.kind, Kind::OllamaCloud),
        }
    }
    async fn discover_models(
        &self,
        material: &ConnectionMaterial,
        cancel: CancellationToken,
    ) -> Result<ModelCatalog> {
        Self::discover_models(self, material, cancel).await
    }
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: self.id().into(),
            display_name: match self.kind {
                Kind::OpenAi => "Custom — OpenAI-compatible",
                Kind::Ollama => "Ollama local",
                Kind::OllamaCloud => "Ollama Cloud",
            }
            .into(),
            // Dynamic per connection, not a fabricated global model inventory.
            models: vec![],
            authentication: match self.kind {
                // Retain the headless capability for existing local connections with
                // credentials. Interactive onboarding deliberately offers no key.
                Kind::Ollama => vec![AuthenticationMethod::None, AuthenticationMethod::ApiKey],
                Kind::OllamaCloud => vec![AuthenticationMethod::ApiKey],
                Kind::OpenAi => vec![AuthenticationMethod::None, AuthenticationMethod::ApiKey],
            },
        }
    }
    fn has_connection_catalog(&self) -> bool {
        true
    }
    async fn create_model(
        &self,
        _: &ModelSelection,
        _: CancellationToken,
    ) -> Result<Arc<dyn ModelProvider>> {
        Err(AgentError::Configuration(
            "adapter requires isolated connection material".into(),
        ))
    }
    async fn create_connection_model(
        &self,
        selection: &ModelSelection,
        material: ConnectionMaterial,
        cancel: CancellationToken,
    ) -> Result<Arc<dyn ModelProvider>> {
        // Settings carry the connection's default model; the router's explicit
        // pinned model may differ, but must pass this connection's live catalog.
        if selection.connection.adapter.provider != self.id() {
            return Err(ProviderError::InvalidModel.into());
        }
        let endpoint = self.endpoint(&material)?.to_owned();
        let models = match self.discover_models(&material, cancel.clone()).await? {
            ModelCatalog::Available(models) => models,
            ModelCatalog::Unsupported if matches!(self.kind, Kind::OpenAi) => {
                // Only a genuinely unsupported catalog permits an explicit manual
                // model. Authentication and transport failures remain fail-closed.
                vec![DiscoveredModel {
                    id: selection.model.clone(),
                    tools: ToolSupport::Unknown,
                }]
            }
            ModelCatalog::Unsupported => {
                return Err(AgentError::Model(
                    "model catalog unsupported; selection not validated".into(),
                ))
            }
            ModelCatalog::Unavailable => {
                return Err(AgentError::Model(
                    "model catalog unavailable; selection not validated".into(),
                ))
            }
            ModelCatalog::Invalid => {
                return Err(AgentError::Model(
                    "invalid model catalog; selection not validated".into(),
                ))
            }
        };
        if !models.iter().any(|model| model.id == selection.model) {
            return Err(ProviderError::InvalidModel.into());
        }
        let tools = if matches!(self.kind, Kind::Ollama | Kind::OllamaCloud) {
            let (status, info) = self
                .json(
                    &material,
                    "/api/show",
                    Some(json!({"model": selection.model})),
                    &cancel,
                )
                .await?;
            if !status.is_success() {
                return Err(AgentError::Model(
                    "Ollama model capabilities unavailable".into(),
                ));
            }
            let info = info.unwrap_or(Value::Null);
            let capabilities = info["capabilities"]
                .as_array()
                .ok_or_else(|| AgentError::Model("Ollama model capabilities absent".into()))?;
            if !capabilities
                .iter()
                .any(|capability| capability == "completion")
            {
                return Err(AgentError::Model(
                    "Ollama model does not support completion".into(),
                ));
            }
            if capabilities.iter().any(|capability| capability == "tools") {
                ToolSupport::Supported
            } else {
                ToolSupport::Unsupported
            }
        } else {
            ToolSupport::Unknown
        };
        let base = match self.kind {
            Kind::OpenAi => endpoint,
            Kind::Ollama | Kind::OllamaCloud => format!("{}/v1", endpoint.trim_end_matches('/')),
        };
        let inner = crate::model::OpenAiProvider::with_transport(
            &base,
            selection.model.clone(),
            material
                .secret
                .as_ref()
                .map(|secret| secret.expose().to_owned()),
            self.timeout_secs,
            material.settings.allow_loopback_http,
        )?;
        Ok(Arc::new(CapabilityModel { inner, tools }))
    }
}

struct CapabilityModel {
    inner: crate::model::OpenAiProvider,
    tools: ToolSupport,
}
#[async_trait]
impl ModelProvider for CapabilityModel {
    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        if !request.tools.is_empty() && self.tools == ToolSupport::Unsupported {
            return Err(AgentError::Model(
                "selected Ollama model does not support tools; no fallback".into(),
            ));
        }
        self.inner.stream(request, cancel).await
    }
}

/// Registers adapters only; ProvidersPlugin remains the sole model publisher.
///
/// Compatibility aggregate for existing embedders. The executable registers the
/// three provider plugins separately so their lifetimes remain independent.
pub struct BuiltinInferencePlugin {
    pub timeout_secs: u64,
}
#[async_trait]
impl Plugin for BuiltinInferencePlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "builtin.providers.inference".into(),
            provides: vec![],
            requires: vec![catalog_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let catalog = ctx
            .resolve::<ProviderCatalogService>(&catalog_id())?
            .0
            .clone();
        catalog.register(
            ctx,
            Arc::new(InferenceAdapter::openai_compatible(self.timeout_secs)),
        )?;
        catalog.register(ctx, Arc::new(InferenceAdapter::ollama(self.timeout_secs)))?;
        catalog.register(
            ctx,
            Arc::new(InferenceAdapter::ollama_cloud(self.timeout_secs)),
        )?;
        Ok(())
    }
}

// Each provider owns its own catalog registration and activation lifetime.
macro_rules! provider_plugin {
    ($name:ident, $id:literal, $adapter:ident) => {
        pub struct $name {
            pub timeout_secs: u64,
        }
        #[async_trait]
        impl Plugin for $name {
            fn manifest(&self) -> Manifest {
                Manifest {
                    id: $id.into(),
                    provides: vec![],
                    requires: vec![catalog_id()],
                }
            }
            async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
                let catalog = ctx
                    .resolve::<ProviderCatalogService>(&catalog_id())?
                    .0
                    .clone();
                catalog.register(ctx, Arc::new(InferenceAdapter::$adapter(self.timeout_secs)))?;
                Ok(())
            }
        }
    };
}
provider_plugin!(OllamaLocalPlugin, "builtin.inference.ollama-local", ollama);
provider_plugin!(
    OllamaCloudPlugin,
    "builtin.inference.ollama-cloud",
    ollama_cloud
);
provider_plugin!(
    CustomProviderPlugin,
    "builtin.inference.custom",
    openai_compatible
);
