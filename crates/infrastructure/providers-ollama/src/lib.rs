// providers-ollama — Ollama local API provider adapter

use async_trait::async_trait;
use futures::TryStreamExt;
use reqwest::Client;
use rook_core::{
    ApiFormat, CompletionRequest, CompletionResponse, HealthStatus, ModelId, ProviderPort,
    StreamChunk, TokenUsage,
};
use serde::Deserialize;
use shared_kernel::{CortexError, CortexResult, ModelId as KModelId, ProviderId};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    model: String,
    message: OllamaMessage,
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    #[allow(dead_code)]
    role: String,
    content: String,
}

#[derive(Debug, Clone)]
pub struct OllamaProviderConfig {
    pub id: ProviderId,
    pub base_url: String,
    pub models: Vec<KModelId>,
    pub timeout_secs: u64,
}

pub struct OllamaProvider {
    config: OllamaProviderConfig,
    #[allow(dead_code)]
    client: Client,
}

impl OllamaProvider {
    pub fn new(config: OllamaProviderConfig) -> anyhow::Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()?;
        Ok(Arc::new(Self { config, client }))
    }
}

#[async_trait]
impl ProviderPort for OllamaProvider {
    fn id(&self) -> &ProviderId {
        &self.config.id
    }
    fn supported_models(&self) -> &[KModelId] {
        &self.config.models
    }
    fn api_format(&self) -> ApiFormat {
        ApiFormat::OpenAI
    }
    fn is_available(&self) -> bool {
        true
    }

    async fn health_check(&self) -> HealthStatus {
        HealthStatus::Unknown {
            provider: self.config.id.clone(),
            reason: "health_check_not_supported".to_string(),
        }
    }

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        let body = serde_json::json!({
            "model": req.model.to_string(),
            "messages": req
                .messages
                .iter()
                .map(|m| serde_json::json!({
                    "role": match m.role {
                        rook_core::Role::System => "system",
                        rook_core::Role::User => "user",
                        rook_core::Role::Assistant => "assistant",
                        rook_core::Role::Developer => "developer",
                    },
                    "content": m.content.as_text(),
                }))
                .collect::<Vec<_>>(),
            "stream": false,
        });

        let start = std::time::Instant::now();
        let resp = self
            .client
            .post(format!("{}/api/chat", self.config.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| CortexError::provider(format!("request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CortexError::provider(format!(
                "Ollama error {status}: {body}"
            )));
        }

        let parsed: OllamaChatResponse = resp
            .json()
            .await
            .map_err(|e| CortexError::provider(format!("json parse failed: {e}")))?;

        let prompt_tokens = parsed.prompt_eval_count.unwrap_or(0);
        let completion_tokens = parsed.eval_count.unwrap_or(0);

        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.config.id.clone(),
            model: ModelId::new(parsed.model),
            content: parsed.message.content.clone(),
            content_blocks: vec![rook_core::MessageContent::Text(parsed.message.content)],
            usage: TokenUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens.saturating_add(completion_tokens),
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None, // Local model — no cost
            },
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        let body = serde_json::json!({
            "model": req.model.to_string(),
            "messages": req
                .messages
                .iter()
                .map(|m| serde_json::json!({
                    "role": match m.role {
                        rook_core::Role::System => "system",
                        rook_core::Role::User => "user",
                        rook_core::Role::Assistant => "assistant",
                        rook_core::Role::Developer => "developer",
                    },
                    "content": m.content.as_text(),
                }))
                .collect::<Vec<_>>(),
            "stream": true,
        });

        let resp = self
            .client
            .post(format!("{}/api/chat", self.config.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| CortexError::provider(format!("request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CortexError::provider(format!(
                "Ollama error {status}: {body}"
            )));
        }

        let request_id = req.id.clone();

        let stream = resp
            .bytes_stream()
            .map_err(|e| CortexError::provider(format!("stream read failed: {e}")))
            .and_then(move |bytes| {
                let request_id = request_id.clone();
                async move {
                    let text = String::from_utf8(bytes.to_vec())
                        .map_err(|e| CortexError::provider(format!("invalid utf-8: {e}")))?;

                    let parsed: OllamaChatResponse = serde_json::from_str(&text)
                        .map_err(|e| CortexError::provider(format!("json parse failed: {e}")))?;

                    let prompt_tokens = parsed.prompt_eval_count.unwrap_or(0);
                    let completion_tokens = parsed.eval_count.unwrap_or(0);

                    Ok(StreamChunk {
                        id: request_id.clone(),
                        model: ModelId::new(parsed.model.clone()),
                        delta: parsed.message.content,
                        finish_reason: if parsed.done {
                            Some(rook_core::FinishReason::Stop)
                        } else {
                            None
                        },
                        usage: if parsed.done {
                            Some(TokenUsage {
                                prompt_tokens,
                                completion_tokens,
                                total_tokens: prompt_tokens.saturating_add(completion_tokens),
                                cache_read_tokens: None,
                                cache_creation_tokens: None,
                                reasoning_tokens: None,
                                estimated_cost_usd: None,
                            })
                        } else {
                            None
                        },
                    })
                }
            });

        Ok(Box::pin(stream))
    }
}
