// providers-anthropic — Anthropic API provider adapter

use async_trait::async_trait;
use reqwest::Client;
use rook_core::{CompletionRequest, CompletionResponse, HealthStatus, ProviderPort, StreamChunk};
use shared_kernel::{ModelId as KModelId, NuxaError, NuxaResult, ProviderId};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AnthropicProviderConfig {
    pub id: ProviderId,
    pub api_key: String,
    pub base_url: String,
    pub models: Vec<KModelId>,
    pub timeout_secs: u64,
}

pub struct AnthropicProvider {
    config: AnthropicProviderConfig,
    #[allow(dead_code)]
    client: Client,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicProviderConfig) -> anyhow::Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()?;
        Ok(Arc::new(Self { config, client }))
    }
}

#[async_trait]
impl ProviderPort for AnthropicProvider {
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
        let start = std::time::Instant::now();
        HealthStatus {
            provider: self.config.id.clone(),
            is_healthy: true, // TODO: real health check
            latency_ms: Some(start.elapsed().as_millis() as u64),
            last_error: None,
        }
    }

    async fn complete(&self, _req: &CompletionRequest) -> NuxaResult<CompletionResponse> {
        Err(NuxaError::provider(
            "Anthropic provider not yet implemented",
        ))
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> NuxaResult<futures::stream::BoxStream<'_, NuxaResult<StreamChunk>>> {
        Err(NuxaError::provider("streaming not yet implemented"))
    }
}
