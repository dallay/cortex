// providers-anthropic — Anthropic API provider adapter

use async_trait::async_trait;
use futures::{Stream, TryStreamExt};
use reqwest::Client;
use rook_core::{
    ApiFormat, CompletionRequest, CompletionResponse, FinishReason, HealthStatus, ModelId,
    ProviderPort, QuotaObservationRecorderPort, Role, StreamChunk, TokenUsage, ToolCallDelta,
    ToolCallFunctionDelta,
};

use providers_core::sanitize_body;
use serde::Deserialize;
use shared_kernel::{CortexError, CortexResult, ModelId as KModelId, ProviderId, RequestId};
use sse_stream::SseBuffer;
use std::sync::Arc;

/// Parse Anthropic rate-limit response headers into quota observations.
///
/// `scope` carries the `anthropic-workspace-id` response header when the
/// upstream sends one, and stays `None` otherwise. That header identifies
/// the workspace attached to the request — it is NOT a guaranteed statement
/// about the level (account / workspace / key) at which the accompanying
/// limit is enforced.
pub fn parse_anthropic_rate_limit_headers(
    headers: &reqwest::header::HeaderMap,
    provider: ProviderId,
    observed_at: chrono::DateTime<chrono::Utc>,
) -> Vec<rook_core::QuotaObservation> {
    use rook_core::{ProviderKind, QuotaObservation};

    let scope = headers
        .get("anthropic-workspace-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let buckets = [
        ("requests", "anthropic-ratelimit-", "requests"),
        ("tokens", "anthropic-ratelimit-", "tokens"),
        ("input_tokens", "anthropic-ratelimit-", "input-tokens"),
        ("output_tokens", "anthropic-ratelimit-", "output-tokens"),
        (
            "priority_input_tokens",
            "anthropic-priority-",
            "input-tokens",
        ),
        (
            "priority_output_tokens",
            "anthropic-priority-",
            "output-tokens",
        ),
    ];
    buckets
        .into_iter()
        .filter_map(|(metric, prefix, header_metric)| {
            let limit =
                anthropic_header_number(headers, &format!("{prefix}{header_metric}-limit"))?;
            let remaining =
                anthropic_header_number(headers, &format!("{prefix}{header_metric}-remaining"))?;
            let reset_at = headers
                .get(format!("{prefix}{header_metric}-reset"))
                .and_then(|value| value.to_str().ok())
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&chrono::Utc));
            Some(QuotaObservation {
                provider_kind: ProviderKind::Anthropic,
                provider: provider.clone(),
                connection_id: None,
                metric: metric.to_string(),
                source: "response_headers".to_string(),
                scope: scope.clone(),
                window: Some("minute".to_string()),
                limit: Some(limit),
                remaining: Some(remaining),
                reset_at,
                reset_after_milliseconds: None,
                observed_at,
            })
        })
        .collect()
}

fn anthropic_header_number(headers: &reqwest::header::HeaderMap, name: &str) -> Option<f64> {
    let value = headers.get(name)?.to_str().ok()?.parse::<f64>().ok()?;
    (value.is_finite() && value >= 0.0).then_some(value)
}

fn map_anthropic_finish_reason(reason: &str) -> Option<FinishReason> {
    match reason {
        "end_turn" | "stop_sequence" | "pause_turn" => Some(FinishReason::Stop),
        "max_tokens" => Some(FinishReason::Length),
        "tool_use" => Some(FinishReason::ToolCalls),
        "refusal" => Some(FinishReason::ContentFilter),
        _ => None,
    }
}

/// Non-streaming response body from Anthropic API
#[derive(Debug, Deserialize)]
struct AnthropicNonStreamResponse {
    #[allow(dead_code)]
    id: String,
    model: String,
    #[serde(default)]
    stop_reason: Option<String>,
    content: Vec<AnthropicNonStreamContentBlock>,
    usage: AnthropicNonStreamUsage,
}

/// A single response content block. Only `type` is required: `text` blocks
/// carry `text`, `tool_use` blocks carry `id`/`name`/`input`, anything else
/// (e.g. `image`, `redacted_thinking`) is skipped instead of failing the
/// whole response.
#[derive(Debug, Deserialize)]
struct AnthropicNonStreamContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
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
    messages: Vec<AnthropicWireMessage>,
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

/// Wire message with STRUCTURED content blocks (text / tool_use / tool_result).
/// The previous `content: String` shape silently dropped every tool block.
#[derive(Debug, serde::Serialize)]
struct AnthropicWireMessage {
    role: String,
    content: Vec<AnthropicWireBlock>,
}

#[derive(Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicWireBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: Vec<AnthropicWireBlock>,
    },
}

/// Map one domain content value to a wire block. Empty text is dropped
/// (Anthropic rejects empty text blocks).
fn domain_content_to_wire(content: &rook_core::MessageContent) -> Option<AnthropicWireBlock> {
    match content {
        rook_core::MessageContent::Text(text) if text.is_empty() => None,
        rook_core::MessageContent::Text(text) => {
            Some(AnthropicWireBlock::Text { text: text.clone() })
        }
        rook_core::MessageContent::ToolUse { id, name, input } => {
            Some(AnthropicWireBlock::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            })
        }
        rook_core::MessageContent::ToolResult {
            tool_use_id,
            content,
        } => Some(AnthropicWireBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.iter().filter_map(domain_content_to_wire).collect(),
        }),
    }
}

/// Build Anthropic wire messages from a domain request. System/developer
/// messages are extracted (returned separately); user/assistant messages keep
/// text AND tool blocks. A message that ends up block-less is dropped
/// (Anthropic rejects empty content arrays).
fn build_wire_messages(req: &CompletionRequest) -> Vec<AnthropicWireMessage> {
    req.messages
        .iter()
        .filter(|m| matches!(m.role, Role::User | Role::Assistant))
        .filter_map(|m| {
            let mut blocks: Vec<AnthropicWireBlock> = Vec::new();
            if let Some(block) = domain_content_to_wire(&m.content) {
                // A bare ToolUse content block would duplicate message.tool_calls;
                // prefer the explicit tool_calls list when both describe tools.
                let content_is_tool_use =
                    matches!(m.content, rook_core::MessageContent::ToolUse { .. });
                if !(content_is_tool_use && !m.tool_calls.is_empty()) {
                    blocks.push(block);
                }
            }
            for call in &m.tool_calls {
                blocks.push(AnthropicWireBlock::ToolUse {
                    id: call.id.clone().unwrap_or_default(),
                    name: call.name.clone(),
                    input: call.arguments.clone(),
                });
            }
            if blocks.is_empty() {
                return None;
            }
            Some(AnthropicWireMessage {
                role: match m.role {
                    Role::User => "user".to_string(),
                    _ => "assistant".to_string(),
                },
                content: blocks,
            })
        })
        .collect()
}

/// SSE event types from Anthropic API
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicStreamEvent {
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: u32, delta: AnthropicDelta },
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
        index: u32,
        content_block: AnthropicContentBlockStart,
    },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop,
    #[serde(rename = "message_stop")]
    MessageStop,
}

/// Delta payload: text fragments or tool-argument JSON fragments. Untagged so
/// a new delta shape fails open (skipped below) instead of discarding the
/// whole event in `serde` — the previous `text`-only struct silently dropped
/// every `input_json_delta`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AnthropicDelta {
    Text { text: String },
    InputJson { partial_json: String },
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
    #[serde(rename = "type")]
    block_type: String,
    /// Present when a `tool_use` block starts.
    #[serde(default)]
    id: Option<String>,
    /// Present when a `tool_use` block starts.
    #[serde(default)]
    name: Option<String>,
}

/// Tracks `tool_use` blocks seen during one stream so the final chunk can
/// carry a tool summary (the transport turns it into `content_block_stop`
/// events). Text-only streams leave this empty and behave exactly as before.
#[derive(Debug, Default)]
struct ToolUseTracker {
    tools: Vec<(u32, Option<String>, Option<String>)>,
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
    quota_recorder: Option<Arc<dyn QuotaObservationRecorderPort>>,
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

impl AnthropicProvider {
    pub fn new(config: AnthropicProviderConfig) -> anyhow::Result<Arc<Self>> {
        Self::new_with_quota_recorder(config, None)
    }

    pub fn new_with_quota_recorder(
        config: AnthropicProviderConfig,
        quota_recorder: Option<Arc<dyn QuotaObservationRecorderPort>>,
    ) -> anyhow::Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Arc::new(Self {
            config,
            client,
            quota_recorder,
        }))
    }

    async fn record_quota_headers(&self, headers: &reqwest::header::HeaderMap) {
        let Some(recorder) = self.quota_recorder.as_ref() else {
            return;
        };
        let observations =
            parse_anthropic_rate_limit_headers(headers, self.config.id.clone(), chrono::Utc::now());
        if let Err(error) = recorder.record_observations(observations).await {
            tracing::warn!(%error, "failed to record Anthropic quota observations");
        }
    }

    fn build_stream_request(req: &CompletionRequest) -> AnthropicStreamRequest {
        let system_text = Self::extract_system_messages(req);
        AnthropicStreamRequest {
            model: req.model.to_string(),
            messages: build_wire_messages(req),
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

    fn process_bytes(
        request_id: &RequestId,
        model: &ModelId,
        sse_buffer: &mut SseBuffer,
        tracker: &mut ToolUseTracker,
        bytes: &[u8],
    ) -> impl Stream<Item = Result<StreamChunk, CortexError>> {
        let events = sse_buffer.push(bytes);
        let chunks: Vec<Result<StreamChunk, CortexError>> = events
            .into_iter()
            .flat_map(|event_text| Self::parse_event_text(&event_text, request_id, model, tracker))
            .collect();

        futures::stream::iter(chunks)
    }

    fn parse_event_text(
        event_text: &str,
        request_id: &RequestId,
        model: &ModelId,
        tracker: &mut ToolUseTracker,
    ) -> Vec<Result<StreamChunk, CortexError>> {
        event_text
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter(|line| !line.trim().is_empty())
            .filter_map(|data_line| serde_json::from_str::<AnthropicStreamEvent>(data_line).ok())
            .filter_map(|parsed| Self::event_to_chunk(parsed, request_id, model, tracker))
            .collect()
    }

    fn tool_summary_chunk(
        request_id: &RequestId,
        model: &ModelId,
        tracker: &ToolUseTracker,
        delta: &AnthropicMessageDeltaDetails,
        usage: &AnthropicMessageDeltaUsage,
    ) -> StreamChunk {
        StreamChunk {
            id: request_id.clone(),
            model: model.clone(),
            delta: String::new(),
            thinking: None,
            tool_calls: tracker
                .tools
                .iter()
                .map(|(index, id, name)| ToolCallDelta {
                    index: *index,
                    id: id.clone(),
                    function: ToolCallFunctionDelta {
                        name: name.clone(),
                        arguments: None,
                    },
                })
                .collect(),
            finish_reason: map_anthropic_finish_reason(&delta.stop_reason)
                .or(Some(FinishReason::Stop)),
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
        }
    }

    fn event_to_chunk(
        event: AnthropicStreamEvent,
        request_id: &RequestId,
        model: &ModelId,
        tracker: &mut ToolUseTracker,
    ) -> Option<Result<StreamChunk, CortexError>> {
        let base = |delta: String, tool_calls: Vec<ToolCallDelta>| StreamChunk {
            id: request_id.clone(),
            model: model.clone(),
            delta,
            thinking: None,
            tool_calls,
            finish_reason: None,
            usage: None,
        };
        match event {
            AnthropicStreamEvent::ContentBlockStart {
                index,
                content_block,
            } => {
                if content_block.block_type == "tool_use" {
                    tracker.tools.push((
                        index,
                        content_block.id.clone(),
                        content_block.name.clone(),
                    ));
                    Some(Ok(base(
                        String::new(),
                        vec![ToolCallDelta {
                            index,
                            id: content_block.id,
                            function: ToolCallFunctionDelta {
                                name: content_block.name,
                                arguments: None,
                            },
                        }],
                    )))
                } else {
                    None
                }
            }
            AnthropicStreamEvent::ContentBlockDelta { index, delta } => match delta {
                AnthropicDelta::Text { text } => Some(Ok(base(text, vec![]))),
                AnthropicDelta::InputJson { partial_json } => Some(Ok(base(
                    String::new(),
                    vec![ToolCallDelta {
                        index,
                        id: None,
                        function: ToolCallFunctionDelta {
                            name: None,
                            arguments: Some(partial_json),
                        },
                    }],
                ))),
            },
            AnthropicStreamEvent::MessageDelta { delta, usage } => Some(Ok(
                Self::tool_summary_chunk(request_id, model, tracker, &delta, &usage),
            )),
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
            messages: build_wire_messages(req),
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

        self.record_quota_headers(resp.headers()).await;

        if !resp.status().is_success() {
            return Err(map_anthropic_http_error(&self.config.id, resp).await);
        }

        let anthropic_resp: AnthropicNonStreamResponse = resp
            .json()
            .await
            .map_err(|e| CortexError::provider(format!("json parse failed: {e}")))?;

        // Preserve text AND tool_use blocks. Unknown block types are skipped
        // (never fail the whole response because of one block).
        let mut text_parts: Vec<String> = Vec::new();
        let mut content_blocks: Vec<rook_core::MessageContent> = Vec::new();
        let mut tool_calls: Vec<rook_core::MessageToolCall> = Vec::new();
        for block in anthropic_resp.content {
            match block.block_type.as_str() {
                "text" => {
                    let text = block.text.unwrap_or_default();
                    text_parts.push(text.clone());
                    if !text.is_empty() {
                        content_blocks.push(rook_core::MessageContent::Text(text));
                    }
                }
                "tool_use" => {
                    let id = block.id.unwrap_or_default();
                    let name = block.name.unwrap_or_default();
                    let input = block.input.unwrap_or(serde_json::Value::Null);
                    tool_calls.push(rook_core::MessageToolCall {
                        id: Some(id.clone()),
                        name: name.clone(),
                        arguments: input.clone(),
                    });
                    content_blocks.push(rook_core::MessageContent::ToolUse { id, name, input });
                }
                _ => continue,
            }
        }
        let text = text_parts.concat();

        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.config.id.clone(),
            model: ModelId::new(anthropic_resp.model),
            content: text,
            content_blocks,
            thinking: None,
            tool_calls,
            finish_reason: anthropic_resp
                .stop_reason
                .as_deref()
                .and_then(map_anthropic_finish_reason)
                .or(Some(FinishReason::Stop)),
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

        // Check HTTP status before processing the stream
        if !resp.status().is_success() {
            return Err(map_anthropic_http_error(&self.config.id, resp).await);
        }

        let request_id = req.id.clone();
        let model = req.model.clone();
        let mut sse_buffer = SseBuffer::new();
        let mut tracker = ToolUseTracker::default();

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
                    &mut tracker,
                    &bytes,
                ))
            })
            .try_flatten();

        Ok(Box::pin(stream))
    }
}
