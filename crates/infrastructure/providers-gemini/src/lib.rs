// providers-gemini — Google Gemini API provider adapter

use async_trait::async_trait;
use reqwest::Client;
use rook_core::{CompletionRequest, CompletionResponse, HealthStatus, ProviderPort, StreamChunk};
use shared_kernel::{CortexError, CortexResult, ModelId as KModelId, ProviderId};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct GeminiProviderConfig {
    pub id: ProviderId,
    pub api_key: String,
    pub models: Vec<KModelId>,
    pub timeout_secs: u64,
}

pub struct GeminiProvider {
    config: GeminiProviderConfig,
    #[allow(dead_code)]
    client: Client,
}

impl GeminiProvider {
    pub fn new(config: GeminiProviderConfig) -> anyhow::Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()?;
        Ok(Arc::new(Self { config, client }))
    }
}

#[async_trait]
impl ProviderPort for GeminiProvider {
    fn id(&self) -> &ProviderId {
        &self.config.id
    }
    fn supported_models(&self) -> &[KModelId] {
        &self.config.models
    }
    fn is_available(&self) -> bool {
        !self.config.api_key.is_empty()
    }

    async fn health_check(&self) -> HealthStatus {
        HealthStatus::Unknown {
            provider: self.config.id.clone(),
            reason: "health_check_not_supported".to_string(),
        }
    }

    async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        Err(CortexError::provider("Gemini provider not yet implemented"))
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        // TODO: translate Gemini streamGenerateContent responses into StreamChunk once the
        // non-streaming adapter is implemented. The port is wired for route support.
        Ok(Box::pin(futures::stream::once(async {
            Err(CortexError::provider(
                "Gemini streaming adapter not yet implemented",
            ))
        })))
    }
}
