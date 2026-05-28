// OpenAI provider implementation — stub until existing impls are migrated

use async_trait::async_trait;
use reqwest::Client;
use rook_core::{
    CompletionRequest, CompletionResponse, HealthStatus, ModelId, ProviderPort, StreamChunk,
    TokenUsage,
};
use shared_kernel::{NuxaError, NuxaResult, ProviderId};

/// Configuration for the OpenAI provider
#[derive(Debug, Clone)]
pub struct OpenAIProviderConfig {
    pub id: ProviderId,
    pub api_key: String,
    pub base_url: String,
    pub models: Vec<ModelId>,
    pub timeout_secs: u64,
}

pub struct OpenAIProvider {
    pub(crate) config: OpenAIProviderConfig,
    pub(crate) client: Client,
}

impl OpenAIProvider {
    pub fn new(config: OpenAIProviderConfig) -> anyhow::Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()?;
        Ok(Self { config, client })
    }
}

#[derive(Debug, serde::Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    stream: bool,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct OpenAIMessage {
    role: String,
    content: String,
}

#[derive(Debug, serde::Deserialize)]
struct OpenAIResponse {
    #[allow(dead_code)]
    id: String,
    model: String,
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
}

#[derive(Debug, serde::Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessageResp,
    #[allow(dead_code)]
    finish_reason: String,
}

#[derive(Debug, serde::Deserialize)]
struct OpenAIMessageResp {
    #[allow(dead_code)]
    role: String,
    content: String,
}

#[derive(Debug, serde::Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[async_trait]
impl ProviderPort for OpenAIProvider {
    fn id(&self) -> &ProviderId {
        &self.config.id
    }

    fn supported_models(&self) -> &[ModelId] {
        &self.config.models
    }

    fn is_available(&self) -> bool {
        !self.config.api_key.is_empty()
    }

    async fn health_check(&self) -> HealthStatus {
        let start = std::time::Instant::now();
        match self
            .client
            .get(format!("{}/models", self.config.base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => HealthStatus {
                provider: self.config.id.clone(),
                is_healthy: true,
                latency_ms: Some(start.elapsed().as_millis() as u64),
                last_error: None,
            },
            Ok(resp) => HealthStatus {
                provider: self.config.id.clone(),
                is_healthy: false,
                latency_ms: Some(start.elapsed().as_millis() as u64),
                last_error: Some(format!("HTTP {}", resp.status())),
            },
            Err(e) => HealthStatus {
                provider: self.config.id.clone(),
                is_healthy: false,
                latency_ms: None,
                last_error: Some(e.to_string()),
            },
        }
    }

    async fn complete(&self, req: &CompletionRequest) -> NuxaResult<CompletionResponse> {
        let body = OpenAIRequest {
            model: req.model.to_string(),
            messages: req
                .messages
                .iter()
                .map(|m| OpenAIMessage {
                    role: match m.role {
                        rook_core::Role::System => "system",
                        rook_core::Role::User => "user",
                        rook_core::Role::Assistant => "assistant",
                        rook_core::Role::Developer => "developer",
                    }
                    .to_string(),
                    content: m.content.clone(),
                })
                .collect(),
            stream: false,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
        };

        let start = std::time::Instant::now();
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.config.base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| NuxaError::provider(format!("request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(NuxaError::provider(format!("{status}: {body}")));
        }

        let openai_resp: OpenAIResponse = resp
            .json()
            .await
            .map_err(|e| NuxaError::provider(format!("json parse failed: {e}")))?;

        let choice = openai_resp
            .choices
            .first()
            .ok_or_else(|| NuxaError::not_found("no choices in response"))?;

        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.config.id.clone(),
            model: ModelId::new(openai_resp.model),
            content: choice.message.content.clone(),
            usage: TokenUsage {
                prompt_tokens: openai_resp.usage.prompt_tokens,
                completion_tokens: openai_resp.usage.completion_tokens,
                total_tokens: openai_resp.usage.total_tokens,
                estimated_cost_usd: None, // TODO: calculate from model pricing
            },
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> NuxaResult<futures::stream::BoxStream<'_, NuxaResult<StreamChunk>>> {
        // TODO: implement streaming with reqwest Events
        Err(NuxaError::provider("streaming not yet implemented"))
    }
}
