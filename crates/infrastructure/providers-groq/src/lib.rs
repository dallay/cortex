// providers-groq — Groq API provider adapter

use async_trait::async_trait;
use futures::{Stream, TryStreamExt};
use providers_core::role_to_string;
use reqwest::Client;
use rook_core::{
    ApiFormat, CompletionRequest, CompletionResponse, HealthStatus, ModelId, ProviderPort,
    StreamChunk, TokenUsage,
};
use serde::Deserialize;
use shared_kernel::{CortexError, CortexResult, ModelId as KModelId, ProviderId};
use sse_stream::SseBuffer;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct GroqResponse {
    #[allow(dead_code)]
    id: String,
    model: String,
    choices: Vec<GroqChoice>,
    usage: GroqUsage,
}

#[derive(Debug, Deserialize)]
struct GroqChoice {
    message: GroqMessage,
    #[allow(dead_code)]
    finish_reason: String,
}

#[derive(Debug, Deserialize)]
struct GroqMessage {
    #[allow(dead_code)]
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct GroqUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct GroqStreamResponse {
    #[allow(dead_code)]
    id: String,
    model: String,
    choices: Vec<GroqStreamChoice>,
    usage: Option<GroqUsage>,
}

#[derive(Debug, Deserialize)]
struct GroqStreamChoice {
    delta: GroqStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GroqStreamDelta {
    content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GroqProviderConfig {
    pub id: ProviderId,
    pub api_key: String,
    pub base_url: Option<String>,
    pub models: Vec<KModelId>,
    pub timeout_secs: u64,
}

const GROQ_DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1";

impl GroqProviderConfig {
    fn base_url(&self) -> &str {
        self.base_url.as_deref().unwrap_or(GROQ_DEFAULT_BASE_URL)
    }
}

pub struct GroqProvider {
    config: GroqProviderConfig,
    client: Client,
}

impl GroqProvider {
    pub fn new(config: GroqProviderConfig) -> anyhow::Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()?;
        Ok(Arc::new(Self { config, client }))
    }

    fn build_stream_request(req: &CompletionRequest) -> GroqRequest {
        GroqRequest {
            model: req.model.to_string(),
            messages: req
                .messages
                .iter()
                .map(|m| GroqRequestMessage {
                    role: role_to_string(m.role).to_string(),
                    content: m.content.as_text().to_string(),
                })
                .collect(),
            stream: true,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
        }
    }

    async fn send_stream_request(&self, body: &GroqRequest) -> CortexResult<reqwest::Response> {
        self.client
            .post(format!("{}/chat/completions", self.config.base_url()))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("content-type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| CortexError::provider(format!("request failed: {e}")))
    }

    async fn validate_response(resp: reqwest::Response) -> CortexResult<reqwest::Response> {
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CortexError::provider(format!(
                "Groq error {status}: {body}"
            )));
        }
        Ok(resp)
    }

    fn process_bytes(
        request_id: &shared_kernel::RequestId,
        sse_buffer: &mut SseBuffer,
        bytes: &[u8],
    ) -> impl Stream<Item = Result<StreamChunk, CortexError>> {
        let events = sse_buffer.push(bytes);
        let chunks: Vec<StreamChunk> = events
            .into_iter()
            .flat_map(|event_text| Self::parse_event_text(&event_text, request_id))
            .collect();

        futures::stream::iter(chunks.into_iter().map(Ok))
    }

    fn parse_event_text(
        event_text: &str,
        request_id: &shared_kernel::RequestId,
    ) -> Vec<StreamChunk> {
        event_text
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter(|line| !line.trim().is_empty() && line.trim() != "[DONE]")
            .filter_map(|data_line| serde_json::from_str::<GroqStreamResponse>(data_line).ok())
            .map(|parsed| Self::response_to_chunk(parsed, request_id))
            .collect()
    }

    fn response_to_chunk(
        parsed: GroqStreamResponse,
        request_id: &shared_kernel::RequestId,
    ) -> StreamChunk {
        let choice = parsed.choices.first();
        let usage = parsed.usage.as_ref().map(|u| TokenUsage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            reasoning_tokens: None,
            estimated_cost_usd: None,
        });

        StreamChunk {
            id: request_id.clone(),
            model: ModelId::new(parsed.model),
            delta: choice
                .and_then(|c| c.delta.content.clone())
                .unwrap_or_default(),
            finish_reason: choice
                .and_then(|c| c.finish_reason.as_deref())
                .and_then(Self::map_finish_reason),
            usage,
        }
    }

    fn map_finish_reason(reason: &str) -> Option<rook_core::FinishReason> {
        match reason {
            "stop" => Some(rook_core::FinishReason::Stop),
            "length" => Some(rook_core::FinishReason::Length),
            "content_filter" => Some(rook_core::FinishReason::ContentFilter),
            "tool_calls" => Some(rook_core::FinishReason::ToolCalls),
            _ => None,
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct GroqRequest {
    model: String,
    messages: Vec<GroqRequestMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, serde::Serialize)]
struct GroqRequestMessage {
    role: String,
    content: String,
}

#[async_trait]
impl ProviderPort for GroqProvider {
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
        let body = GroqRequest {
            model: req.model.to_string(),
            messages: req
                .messages
                .iter()
                .map(|m| GroqRequestMessage {
                    role: role_to_string(m.role).to_string(),
                    content: m.content.as_text().to_string(),
                })
                .collect(),
            stream: false,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
        };

        let start = std::time::Instant::now();
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.config.base_url()))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| CortexError::provider(format!("request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CortexError::provider(format!(
                "Groq error {status}: {body}"
            )));
        }

        let groq_resp: GroqResponse = resp
            .json()
            .await
            .map_err(|e| CortexError::provider(format!("json parse failed: {e}")))?;

        let choice = groq_resp
            .choices
            .first()
            .ok_or_else(|| CortexError::not_found("no choices in response"))?;

        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.config.id.clone(),
            model: ModelId::new(groq_resp.model),
            content: choice.message.content.clone(),
            content_blocks: vec![rook_core::MessageContent::Text(
                choice.message.content.clone(),
            )],
            usage: TokenUsage {
                prompt_tokens: groq_resp.usage.prompt_tokens,
                completion_tokens: groq_resp.usage.completion_tokens,
                total_tokens: groq_resp.usage.total_tokens,
                // Groq doesn't support cache or reasoning tokens in this API shape
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: start.elapsed().as_millis() as u64,
            cache_hit: None,
        })
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        let body = Self::build_stream_request(req);
        let resp = self.send_stream_request(&body).await?;
        let resp = Self::validate_response(resp).await?;

        let request_id = req.id.clone();
        let mut sse_buffer = SseBuffer::new();

        let stream = resp
            .bytes_stream()
            .map_err(|e| CortexError::provider(format!("stream read failed: {e}")))
            .and_then(move |bytes| {
                let request_id = request_id.clone();
                futures::future::ok(Self::process_bytes(&request_id, &mut sse_buffer, &bytes))
            })
            .try_flatten();

        Ok(Box::pin(stream))
    }
}
