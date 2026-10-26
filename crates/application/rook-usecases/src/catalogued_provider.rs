use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use rook_core::{
    ApiFormat, BoxStream, CompletionRequest, CompletionResponse, CortexError, CortexResult,
    HealthStatus, ModelCapabilities, ModelId, ProviderId, ProviderPort, RequiredCapabilities,
    StreamChunk,
};

/// Immutable metadata travels with the adapter snapshot through registry refreshes.
pub struct CataloguedProvider {
    inner: Arc<dyn ProviderPort>,
    metadata: HashMap<ModelId, ModelCapabilities>,
}

impl CataloguedProvider {
    pub fn new(
        inner: Arc<dyn ProviderPort>,
        mut metadata: HashMap<ModelId, ModelCapabilities>,
        ceiling: ModelCapabilities,
    ) -> Self {
        for capabilities in metadata.values_mut() {
            capabilities.constrain(&ceiling);
        }
        Self { inner, metadata }
    }

    fn check(&self, req: &CompletionRequest, streaming: bool) -> CortexResult<()> {
        let mut required = RequiredCapabilities::from_request(req);
        required.streaming = streaming;
        required
            .check(&self.model_capabilities(&req.model))
            .map_err(CortexError::invalid_request)
    }
}

#[async_trait]
impl ProviderPort for CataloguedProvider {
    fn id(&self) -> &ProviderId {
        self.inner.id()
    }
    fn supported_models(&self) -> &[ModelId] {
        self.inner.supported_models()
    }
    fn supports_model(&self, model: &ModelId) -> bool {
        self.inner.supports_model(model)
    }
    fn model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        self.metadata.get(model).cloned().unwrap_or_default()
    }
    fn api_format(&self) -> ApiFormat {
        self.inner.api_format()
    }
    fn is_available(&self) -> bool {
        self.inner.is_available()
    }
    async fn health_check(&self) -> HealthStatus {
        self.inner.health_check().await
    }
    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        self.check(req, false)?;
        self.inner.complete(req).await
    }
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>> {
        self.check(req, true)?;
        self.inner.stream(req).await
    }
}
