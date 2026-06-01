// providers-anthropic — Anthropic API provider adapter

use async_trait::async_trait;
use futures::{Stream, TryStreamExt};
use reqwest::Client;
use rook_core::{
    CompletionRequest, CompletionResponse, FinishReason, HealthStatus, ModelId, ProviderPort,
    RequestId, Role, StreamChunk, TokenUsage,
};
use serde::Deserialize;
use shared_kernel::{CortexError, CortexResult, ModelId as KModelId, ProviderId};
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
    let raw_body = resp.text().await.unwrap_or_default();
    let sanitized = sanitize_body(&raw_body);

    match status.as_u16() {
        401 => CortexError::auth_failed("Anthropic authentication failed"),
        429 => CortexError::rate_limited(provider_id.clone(), retry_after.unwrap_or(60)),
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
            content: text,
            usage: TokenUsage {
                prompt_tokens: anthropic_resp.usage.input_tokens,
                completion_tokens: anthropic_resp.usage.output_tokens,
                total_tokens: anthropic_resp.usage.input_tokens
                    + anthropic_resp.usage.output_tokens,
                estimated_cost_usd: None,
            },
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        // Extract system/developer messages into the top-level `system` field.
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
                        _ => unreachable!(),
                    },
                    content: m.content.as_text().to_string(),
                })
                .collect(),
            system: system_text,
            stream: true,
            max_tokens: req.max_tokens.or(Some(4096)), // SC-08: default max_tokens
            temperature: req.temperature,
        };

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
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CortexError::provider(format!("{status}: {body}")));
        }

        let request_id = req.id.clone();
        let model = req.model.clone();
        let mut sse_buffer = SseBuffer::new();

        fn process_bytes(
            request_id: &RequestId,
            model: &ModelId,
            sse_buffer: &mut SseBuffer,
            bytes: &[u8],
        ) -> impl Stream<Item = Result<StreamChunk, CortexError>> {
            let events = sse_buffer.push(bytes);
            let mut chunks: Vec<Result<StreamChunk, CortexError>> = Vec::new();

            for event_text in events {
                for data_line in event_text.lines().filter_map(|l| l.strip_prefix("data: ")) {
                    if data_line.trim().is_empty() {
                        continue;
                    }

                    let parsed: AnthropicStreamEvent = match serde_json::from_str(data_line) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };

                    match parsed {
                        AnthropicStreamEvent::ContentBlockDelta { index: _, delta } => {
                            chunks.push(Ok(StreamChunk {
                                id: request_id.clone(),
                                model: model.clone(),
                                delta: delta.text,
                                finish_reason: None,
                                usage: None,
                            }));
                        }
                        AnthropicStreamEvent::MessageDelta { usage, .. } => {
                            chunks.push(Ok(StreamChunk {
                                id: request_id.clone(),
                                model: model.clone(),
                                delta: String::new(),
                                finish_reason: Some(FinishReason::Stop),
                                usage: Some(TokenUsage {
                                    prompt_tokens: usage.input_tokens.unwrap_or(0),
                                    completion_tokens: usage.output_tokens,
                                    total_tokens: usage
                                        .input_tokens
                                        .unwrap_or(0)
                                        .saturating_add(usage.output_tokens),
                                    estimated_cost_usd: None,
                                }),
                            }));
                        }
                        AnthropicStreamEvent::Error { error } => {
                            chunks.push(Err(CortexError::provider(format!(
                                "Anthropic error: {} - {}",
                                error.error_type, error.message
                            ))));
                        }
                        _ => {}
                    }
                }
            }

            futures::stream::iter(chunks)
        }

        let stream = resp
            .bytes_stream()
            .map_err(|e| CortexError::provider(format!("stream read failed: {e}")))
            .and_then(move |bytes| {
                let request_id = request_id.clone();
                let model = model.clone();
                futures::future::ok(process_bytes(&request_id, &model, &mut sse_buffer, &bytes))
            })
            .try_flatten();

        Ok(Box::pin(stream))
    }
}
