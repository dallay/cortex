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
use std::sync::Arc;

/// Anthropic streaming request body
#[derive(Debug, serde::Serialize)]
struct AnthropicStreamRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
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

/// SSE buffer that accumulates raw bytes until a complete SSE event is available.
struct SseBuffer {
    buffer: Vec<u8>,
}

impl SseBuffer {
    fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Push incoming bytes, return an iterator of complete SSE event strings.
    fn push(&mut self, incoming: &[u8]) -> impl Iterator<Item = String> + '_ {
        self.buffer.extend_from_slice(incoming);
        let mut events = Vec::new();
        let mut start = 0;

        loop {
            let mut search_start = start;
            let mut found_double_nl = None;

            while search_start < self.buffer.len() {
                if let Some(pos) = byte_memchr(b'\n', &self.buffer[search_start..]) {
                    let abs_pos = search_start + pos;
                    if abs_pos + 1 < self.buffer.len() && self.buffer[abs_pos + 1] == b'\n' {
                        found_double_nl = Some(abs_pos);
                        break;
                    }
                    search_start = abs_pos + 1;
                } else {
                    break;
                }
            }

            match found_double_nl {
                Some(event_end) => {
                    let event_text_len = event_end - start;
                    if let Ok(event) =
                        String::from_utf8(self.buffer[start..start + event_text_len].to_vec())
                    {
                        events.push(event);
                    }
                    start = event_end + 2;
                }
                None => break,
            }
        }

        if start > 0 {
            self.buffer.drain(0..start);
        }

        events.into_iter()
    }
}

impl Default for SseBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn byte_memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
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

    async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        Err(CortexError::provider(
            "Anthropic provider not yet implemented",
        ))
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        let body = AnthropicStreamRequest {
            model: req.model.to_string(),
            messages: req
                .messages
                .iter()
                .map(|m| AnthropicMessage {
                    role: match m.role {
                        Role::System => "system".to_string(),
                        Role::User => "user".to_string(),
                        Role::Assistant => "assistant".to_string(),
                        Role::Developer => "developer".to_string(),
                    },
                    content: m.content.clone(),
                })
                .collect(),
            stream: true,
            max_tokens: req.max_tokens,
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
