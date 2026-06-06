// providers-anthropic — Anthropic API provider adapter

use async_trait::async_trait;
use futures::{Stream, TryStreamExt};
use reqwest::Client;
use rook_core::{
    ApiFormat, CompletionRequest, CompletionResponse, FinishReason, HealthStatus, ModelId,
    ProviderPort, Role, StreamChunk, TokenUsage,
};

use serde::Deserialize;
use shared_kernel::{CortexError, CortexResult, ModelId as KModelId, ProviderId, RequestId};
use sse_stream::SseBuffer;
use std::sync::Arc;

/// Non-streaming response body from Anthropic API
#[derive(Debug, Deserialize)]
struct AnthropicNonStreamResponse {
    #[allow(dead_code)]
    id: String,
    model: String,
    content: Vec<AnthropicNonStreamContentBlock>,
    usage: AnthropicNonStreamUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicNonStreamContentBlock {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    block_type: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicNonStreamUsage {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
}

/// Anthropic streaming request body
#[derive(Debug, serde::Serialize)]
struct AnthropicStreamRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    stream: bool,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
}

#[derive(Debug, serde::Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

/// SSE event types from Anthropic API
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicStreamEvent {
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        #[allow(dead_code)]
        index: u32,
        delta: AnthropicTextDelta,
    },
    #[serde(rename = "message_delta")]
    MessageDelta {
        #[allow(dead_code)]
        delta: AnthropicMessageDeltaDetails,
        usage: AnthropicMessageDeltaUsage,
    },
    #[serde(rename = "error")]
    Error { error: AnthropicErrorDetail },
    #[serde(rename = "message_start")]
    #[allow(dead_code)]
    MessageStart { message: AnthropicMessageStart },
    #[serde(rename = "content_block_start")]
    #[allow(dead_code)]
    ContentBlockStart {
        content_block: AnthropicContentBlockStart,
    },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop,
    #[serde(rename = "message_stop")]
    MessageStop,
}

#[derive(Debug, Deserialize)]
struct AnthropicTextDelta {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    delta_type: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDeltaDetails {
    #[allow(dead_code)]
    stop_reason: String,
    #[serde(default)]
    #[allow(dead_code)]
    stop_sequence: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDeltaUsage {
    output_tokens: u32,
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageStart {
    #[allow(dead_code)]
    id: String,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    message_type: String,
    #[allow(dead_code)]
    role: String,
    #[allow(dead_code)]
    content: Vec<()>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlockStart {
    #[allow(dead_code)]
    index: u32,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    block_type: String,
}

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

/// Map an Anthropic HTTP error response to a typed `CortexError`.
///
/// Reads `Retry-After` header for 429 and sanitizes the body to prevent leakage.
async fn map_anthropic_http_error(
    provider_id: &ProviderId,
    resp: reqwest::Response,
) -> CortexError {
    let status = resp.status();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    let reset_at = resp
        .headers()
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    let raw_body = resp.text().await.unwrap_or_default();
    let sanitized = sanitize_body(&raw_body);

    match status.as_u16() {
        401 => CortexError::auth_failed("Anthropic authentication failed"),
        429 => {
            let retry_secs = retry_after.unwrap_or(60);
            if let Some(reset) = reset_at {
                CortexError::rate_limited_with_reset(provider_id.clone(), retry_secs, reset)
            } else {
                CortexError::rate_limited(provider_id.clone(), retry_secs)
            }
        }
        400 => CortexError::invalid_request(sanitized),
        _ => CortexError::provider(format!("Anthropic error {status}: {sanitized}")),
    }
}

/// Sanitize and truncate body to avoid sensitive data leakage.
fn sanitize_body(body: &str) -> String {
    const MAX: usize = 200;
    let mut chars = body.chars();
    let truncated: String = chars.by_ref().take(MAX).collect();
    if chars.next().is_some() {
        format!("{truncated}… (truncated)")
    } else {
        truncated
    }
}

impl AnthropicProvider {
    pub fn new(config: AnthropicProviderConfig) -> anyhow::Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()?;
        Ok(Arc::new(Self { config, client }))
    }

    fn build_stream_request(req: &CompletionRequest) -> AnthropicStreamRequest {
        let system_text = Self::extract_system_messages(req);
        AnthropicStreamRequest {
            model: req.model.to_string(),
            messages: req
                .messages
                .iter()
                .filter(|m| matches!(m.role, Role::User | Role::Assistant))
                .map(|m| AnthropicMessage {
                    role: match m.role {
                        Role::User => "user".to_string(),
                        Role::Assistant => "assistant".to_string(),
                        _ => unreachable!(),
                    },
                    content: m.content.as_text().to_string(),
                })
                .collect(),
            system: system_text,
            stream: true,
            max_tokens: req.max_tokens.or(Some(4096)),
            temperature: req.temperature,
            tools: req.tools.clone(),
            tool_choice: req.tool_choice.clone(),
        }
    }

    fn extract_system_messages(req: &CompletionRequest) -> Option<String> {
        let parts: Vec<&str> = req
            .messages
            .iter()
            .filter(|m| matches!(m.role, Role::System | Role::Developer))
            .map(|m| m.content.as_text())
            .collect();

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n"))
        }
    }

    async fn send_stream_request(
        &self,
        body: &AnthropicStreamRequest,
    ) -> CortexResult<reqwest::Response> {
        self.client
            .post(format!("{}/v1/messages", self.config.base_url))
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
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
            return Err(CortexError::provider(format!("{status}: {body}")));
        }
        Ok(resp)
    }

    fn process_bytes(
        request_id: &RequestId,
        model: &ModelId,
        sse_buffer: &mut SseBuffer,
        bytes: &[u8],
    ) -> impl Stream<Item = Result<StreamChunk, CortexError>> {
        let events = sse_buffer.push(bytes);
        let chunks: Vec<Result<StreamChunk, CortexError>> = events
            .into_iter()
            .flat_map(|event_text| Self::parse_event_text(&event_text, request_id, model))
            .collect();

        futures::stream::iter(chunks)
    }

    fn parse_event_text(
        event_text: &str,
        request_id: &RequestId,
        model: &ModelId,
    ) -> Vec<Result<StreamChunk, CortexError>> {
        event_text
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter(|line| !line.trim().is_empty())
            .filter_map(|data_line| serde_json::from_str::<AnthropicStreamEvent>(data_line).ok())
            .filter_map(|parsed| Self::event_to_chunk(parsed, request_id, model))
            .collect()
    }

    fn event_to_chunk(
        event: AnthropicStreamEvent,
        request_id: &RequestId,
        model: &ModelId,
    ) -> Option<Result<StreamChunk, CortexError>> {
        match event {
            AnthropicStreamEvent::ContentBlockDelta { index: _, delta } => Some(Ok(StreamChunk {
                id: request_id.clone(),
                model: model.clone(),
                delta: delta.text,
                finish_reason: None,
                usage: None,
            })),
            AnthropicStreamEvent::MessageDelta { delta, usage } => {
                let finish_reason = if delta.stop_reason == "tool_use" {
                    FinishReason::ToolCalls
                } else {
                    FinishReason::Stop
                };
                Some(Ok(StreamChunk {
                    id: request_id.clone(),
                    model: model.clone(),
                    delta: String::new(),
                    finish_reason: Some(finish_reason),
                    usage: Some(TokenUsage {
                        prompt_tokens: usage.input_tokens.unwrap_or(0),
                        completion_tokens: usage.output_tokens,
                        total_tokens: usage
                            .input_tokens
                            .unwrap_or(0)
                            .saturating_add(usage.output_tokens),
                        cache_read_tokens: usage.cache_read_input_tokens,
                        cache_creation_tokens: usage.cache_creation_input_tokens,
                        reasoning_tokens: None,
                        estimated_cost_usd: None,
                    }),
                }))
            }
            AnthropicStreamEvent::Error { error } => Some(Err(CortexError::provider(format!(
                "Anthropic error: {} - {}",
                error.error_type, error.message
            )))),
            _ => None,
        }
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
    fn api_format(&self) -> ApiFormat {
        ApiFormat::Anthropic
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
        // Extract system/developer messages into the top-level `system` field.
        // The Anthropic Messages API does not accept role:"system" inside the messages array.
        let system_text: Option<String> = {
            let parts: Vec<&str> = req
                .messages
                .iter()
                .filter(|m| matches!(m.role, Role::System | Role::Developer))
                .map(|m| m.content.as_text())
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        };
        let body = AnthropicStreamRequest {
            model: req.model.to_string(),
            messages: req
                .messages
                .iter()
                .filter(|m| matches!(m.role, Role::User | Role::Assistant))
                .map(|m| AnthropicMessage {
                    role: match m.role {
                        Role::User => "user".to_string(),
                        Role::Assistant => "assistant".to_string(),
                        // System/Developer already extracted above
                        _ => unreachable!(),
                    },
                    content: m.content.as_text().to_string(),
                })
                .collect(),
            system: system_text,
            stream: false,
            max_tokens: req.max_tokens.or(Some(4096)), // SC-08: default max_tokens
            temperature: req.temperature,
            tools: req.tools.clone(),
            tool_choice: req.tool_choice.clone(),
        };

        let start = std::time::Instant::now();
        let resp = self
            .client
            .post(format!("{}/v1/messages", self.config.base_url))
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| CortexError::provider(format!("request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(map_anthropic_http_error(&self.config.id, resp).await);
        }

        let anthropic_resp: AnthropicNonStreamResponse = resp
            .json()
            .await
            .map_err(|e| CortexError::provider(format!("json parse failed: {e}")))?;

        let text = anthropic_resp
            .content
            .into_iter()
            .map(|b| b.text)
            .collect::<Vec<_>>()
            .join("");

        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.config.id.clone(),
            model: ModelId::new(anthropic_resp.model),
            content: text.clone(),
            content_blocks: vec![rook_core::MessageContent::Text(text)],
            usage: TokenUsage {
                prompt_tokens: anthropic_resp.usage.input_tokens,
                completion_tokens: anthropic_resp.usage.output_tokens,
                total_tokens: anthropic_resp.usage.input_tokens
                    + anthropic_resp.usage.output_tokens,
                cache_read_tokens: anthropic_resp.usage.cache_read_input_tokens,
                cache_creation_tokens: anthropic_resp.usage.cache_creation_input_tokens,
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
        let model = req.model.clone();
        let mut sse_buffer = SseBuffer::new();

        let stream = resp
            .bytes_stream()
            .map_err(|e| CortexError::provider(format!("stream read failed: {e}")))
            .and_then(move |bytes| {
                let request_id = request_id.clone();
                let model = model.clone();
                futures::future::ok(Self::process_bytes(
                    &request_id,
                    &model,
                    &mut sse_buffer,
                    &bytes,
                ))
            })
            .try_flatten();

        Ok(Box::pin(stream))
    }
}
