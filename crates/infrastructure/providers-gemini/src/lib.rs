// providers-gemini — Google Gemini API provider adapter

use async_trait::async_trait;
use reqwest::Client;
use rook_core::{
    ApiFormat, CompletionRequest, CompletionResponse, HealthStatus, ModelId, ProviderPort,
    StreamChunk, TokenUsage,
};
use serde::Deserialize;
use shared_kernel::{CortexError, CortexResult, ModelId as KModelId, ProviderId};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct GeminiGenerateResponse {
    candidates: Vec<GeminiCandidate>,
    #[serde(rename = "usageMetadata", default)]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    content: GeminiContent,
    #[serde(rename = "finishReason")]
    #[allow(dead_code)]
    finish_reason: Option<String>,
    #[allow(dead_code)]
    index: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GeminiContent {
    #[allow(dead_code)]
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Deserialize)]
struct GeminiPart {
    text: Option<String>,
    #[allow(dead_code)]
    function_call: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GeminiUsageMetadata {
    #[serde(rename = "promptTokenCount", default)]
    prompt_token_count: Option<u32>,
    #[serde(rename = "candidatesTokenCount", default)]
    candidates_token_count: Option<u32>,
    #[serde(rename = "totalTokenCount", default)]
    total_token_count: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct GeminiProviderConfig {
    pub id: ProviderId,
    pub api_key: String,
    pub base_url: Option<String>,
    pub models: Vec<KModelId>,
    pub timeout_secs: u64,
}

pub struct GeminiProvider {
    config: GeminiProviderConfig,
    client: Client,
}

impl GeminiProvider {
    pub fn new(config: GeminiProviderConfig) -> anyhow::Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()?;
        Ok(Arc::new(Self { config, client }))
    }

    fn base_url(&self) -> String {
        self.config
            .base_url
            .clone()
            .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_string())
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
    fn api_format(&self) -> ApiFormat {
        ApiFormat::OpenAI
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

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        let model = req.model.to_string();

        // Collect system/developer messages into a single systemInstruction string.
        // Gemini expects system-level instructions in systemInstruction field, not contents.
        let mut system_instruction = String::new();
        let contents: Vec<serde_json::Value> = req
            .messages
            .iter()
            .filter_map(|m| match m.role {
                rook_core::Role::System | rook_core::Role::Developer => {
                    if !system_instruction.is_empty() {
                        system_instruction.push('\n');
                    }
                    system_instruction.push_str(m.content.as_text());
                    None
                }
                rook_core::Role::User => Some(serde_json::json!({
                    "role": "user",
                    "parts": [{ "text": m.content.as_text() }]
                })),
                rook_core::Role::Assistant => Some(serde_json::json!({
                    "role": "model",
                    "parts": [{ "text": m.content.as_text() }]
                })),
            })
            .collect();

        let mut body = serde_json::json!({
            "contents": contents
        });
        // Add systemInstruction only when we have system/developer content
        if !system_instruction.is_empty() {
            body["systemInstruction"] = serde_json::json!({
                "parts": [{ "text": system_instruction }]
            });
        }

        let start = std::time::Instant::now();
        let url = format!(
            "{}/v1beta/models/{}:generateContent",
            self.base_url(),
            model
        );

        let resp = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .header("x-goog-api-key", &self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| CortexError::provider(format!("request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CortexError::provider(format!(
                "Gemini error {status}: {body}"
            )));
        }

        let parsed: GeminiGenerateResponse = resp
            .json()
            .await
            .map_err(|e| CortexError::provider(format!("json parse failed: {e}")))?;

        let content_text = parsed
            .candidates
            .first()
            .and_then(|c| c.content.parts.first())
            .and_then(|p| p.text.clone())
            .unwrap_or_default();

        let (prompt_tokens, completion_tokens, total_tokens) =
            if let Some(ref metadata) = parsed.usage_metadata {
                (
                    metadata.prompt_token_count.unwrap_or(0),
                    metadata.candidates_token_count.unwrap_or(0),
                    metadata.total_token_count.unwrap_or_else(|| {
                        metadata
                            .prompt_token_count
                            .unwrap_or(0)
                            .saturating_add(metadata.candidates_token_count.unwrap_or(0))
                    }),
                )
            } else {
                tracing::warn!(
                    provider = %self.config.id,
                    model = %model,
                    request_id = %req.id,
                    "Gemini usage_metadata missing from response"
                );
                (0, 0, 0)
            };

        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.config.id.clone(),
            model: ModelId::new(model),
            content: content_text.clone(),
            content_blocks: vec![rook_core::MessageContent::Text(content_text)],
            usage: TokenUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        // TODO: translate Gemini streamGenerateContent responses into StreamChunk
        Ok(Box::pin(futures::stream::once(async {
            Err(CortexError::provider(
                "Gemini streaming adapter not yet implemented",
            ))
        })))
    }
}
