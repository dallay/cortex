// providers-anthropic — Anthropic API provider adapter

use async_trait::async_trait;
use futures::StreamExt;
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
use std::collections::VecDeque;
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
    id: providers_core::tool_id::ToolId,
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
        delta: AnthropicMessageDeltaDetails,
        usage: AnthropicMessageDeltaUsage,
    },
    #[serde(rename = "error")]
    Error { error: AnthropicErrorDetail },
    #[serde(rename = "message_start")]
    MessageStart {
        #[serde(default)]
        message: Option<AnthropicStreamMessage>,
    },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: u32,
        content_block: AnthropicContentBlockStart,
    },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
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
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDeltaUsage {
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AnthropicStreamMessage {
    usage: AnthropicMessageDeltaUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlockStart {
    #[serde(rename = "type")]
    block_type: String,
    /// Present when a `tool_use` block starts.
    #[serde(default)]
    id: providers_core::tool_id::ToolId,
    /// Present when a `tool_use` block starts.
    #[serde(default)]
    name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_initial_input")]
    input: Option<serde_json::Value>,
}

fn normalize_tool_id(id: &providers_core::tool_id::ToolId, responses: bool) -> Option<String> {
    if responses {
        // An explicit null must reach Responses validation as an invalid ID,
        // not as an omission eligible for ID generation.
        id.canonical()
    } else {
        id.as_str().map(str::to_owned)
    }
}

fn deserialize_initial_input<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Value>, D::Error> {
    // Distinguir input omitido de input:null para rechazar el segundo.
    serde_json::Value::deserialize(deserializer).map(Some)
}

#[derive(Debug)]
struct TrackedToolUse {
    index: u32,
    id: Option<String>,
    name: Option<String>,
    initial_input: Option<String>,
    received_deltas: bool,
    closed: bool,
}

/// Tracks `tool_use` blocks seen during one stream so the final chunk can
/// carry a tool summary (the transport turns it into `content_block_stop`
/// events). Text-only streams leave this empty and behave exactly as before.
#[derive(Debug, Default)]
struct ToolUseTracker {
    tools: Vec<TrackedToolUse>,
    initial_usage: Option<AnthropicMessageDeltaUsage>,
    argument_bytes: usize,
    responses: bool,
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
    ) -> Vec<Result<StreamChunk, CortexError>> {
        let events = match sse_buffer.push_with_max_event_bytes(bytes, 2 * 1024 * 1024) {
            Ok(events) => events,
            Err(_) => {
                return vec![Err(CortexError::provider(
                    "Anthropic SSE event limit exceeded",
                ))]
            }
        };
        events
            .into_iter()
            .flat_map(|event_text| Self::parse_event_text(&event_text, request_id, model, tracker))
            .collect()
    }

    fn parse_event_text(
        event_text: &str,
        request_id: &RequestId,
        model: &ModelId,
        tracker: &mut ToolUseTracker,
    ) -> Vec<Result<StreamChunk, CortexError>> {
        if tracker.responses {
            let data = event_text
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|line| line.strip_prefix(' ').unwrap_or(line))
                .collect::<Vec<_>>()
                .join("\n");
            if data.trim().is_empty() {
                return vec![];
            }
            return match serde_json::from_str::<AnthropicStreamEvent>(&data) {
                Ok(parsed) => Self::event_to_chunk(parsed, request_id, model, tracker)
                    .into_iter()
                    .collect(),
                Err(_) => vec![Err(CortexError::provider(
                    "Invalid Anthropic SSE JSON event",
                ))],
            };
        }
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
                // Raw absent/null stop_reason denotes usage only, not a replay
                // of completed tool blocks. Explicit unknown reasons are errors.
                .filter(|_| !tracker.responses || delta.stop_reason.is_some())
                .map(|tool| ToolCallDelta {
                    index: tool.index,
                    id: tool.id.clone(),
                    function: ToolCallFunctionDelta {
                        // Responses already received the name at block_start. Repeating it
                        // without an upstream ID would alter the generated call identity.
                        name: if tracker.responses {
                            None
                        } else {
                            tool.name.clone()
                        },
                        arguments: None,
                    },
                })
                .collect(),
            finish_reason: delta
                .stop_reason
                .as_deref()
                .and_then(map_anthropic_finish_reason)
                .or_else(|| (!tracker.responses).then_some(FinishReason::Stop)),
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
            AnthropicStreamEvent::MessageStart { message } => {
                tracker.initial_usage = message.map(|message| message.usage);
                if !tracker.responses {
                    return None;
                }
                // A usage snapshot is a successful domain chunk and commits Responses
                // selection, even before content. Retain it for later usage merging.
                tracker.initial_usage.as_ref().map(|usage| {
                    Ok(Self::tool_summary_chunk(
                        request_id,
                        model,
                        tracker,
                        &AnthropicMessageDeltaDetails { stop_reason: None },
                        usage,
                    ))
                })
            }
            AnthropicStreamEvent::ContentBlockStart {
                index,
                content_block,
            } => {
                // Legacy parsing ignored events with non-string IDs.
                if !tracker.responses && content_block.id.is_invalid_type() {
                    return None;
                }
                if content_block.block_type == "tool_use" {
                    let id = normalize_tool_id(&content_block.id, tracker.responses);
                    if tracker.tools.len() >= 64 {
                        return Some(Err(CortexError::provider(
                            "Anthropic stream exceeded the limit of 64 tool calls",
                        )));
                    }
                    if tracker.tools.iter().any(|tool| tool.index == index)
                        || id
                            .as_ref()
                            .is_some_and(|id| id.len() > 256 || id.chars().any(char::is_control))
                        || content_block
                            .name
                            .as_ref()
                            .is_some_and(|name| name.len() > 4096)
                        || content_block
                            .input
                            .as_ref()
                            .is_some_and(|input| !input.is_object())
                    {
                        return Some(Err(CortexError::provider(
                            "Invalid Anthropic tool_use start",
                        )));
                    }
                    let initial_input = content_block.input.map(|input| input.to_string());
                    let argument_bytes = tracker
                        .argument_bytes
                        .saturating_add(initial_input.as_ref().map_or(0, String::len));
                    if argument_bytes > 1024 * 1024 {
                        return Some(Err(CortexError::provider(
                            "Anthropic tool argument limit exceeded",
                        )));
                    }
                    tracker.argument_bytes = argument_bytes;
                    tracker.tools.push(TrackedToolUse {
                        index,
                        id: id.clone(),
                        name: content_block.name.clone(),
                        initial_input,
                        received_deltas: false,
                        closed: false,
                    });
                    Some(Ok(base(
                        String::new(),
                        vec![ToolCallDelta {
                            index,
                            id,
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
                AnthropicDelta::InputJson { partial_json } => {
                    let Some(tool) = tracker
                        .tools
                        .iter_mut()
                        .find(|tool| tool.index == index && !tool.closed)
                    else {
                        return Some(Err(CortexError::provider(
                            "Anthropic tool delta without an open block",
                        )));
                    };
                    let discarded_bytes = tool.initial_input.as_ref().map_or(0, String::len);
                    let argument_bytes = tracker
                        .argument_bytes
                        .saturating_sub(discarded_bytes)
                        .saturating_add(partial_json.len());
                    if argument_bytes > 1024 * 1024 {
                        return Some(Err(CortexError::provider(
                            "Anthropic tool argument limit exceeded",
                        )));
                    }
                    tracker.argument_bytes = argument_bytes;
                    tool.initial_input = None;
                    tool.received_deltas = true;
                    Some(Ok(base(
                        String::new(),
                        vec![ToolCallDelta {
                            index,
                            id: None,
                            function: ToolCallFunctionDelta {
                                name: None,
                                arguments: Some(partial_json),
                            },
                        }],
                    )))
                }
            },
            AnthropicStreamEvent::ContentBlockStop { index } => {
                let tool = tracker.tools.iter_mut().find(|tool| tool.index == index)?;
                tool.closed = true;
                if tool.received_deltas {
                    return None;
                }
                let arguments = tool.initial_input.take()?;
                Some(Ok(base(
                    String::new(),
                    vec![ToolCallDelta {
                        index,
                        id: None,
                        function: ToolCallFunctionDelta {
                            name: None,
                            arguments: Some(arguments),
                        },
                    }],
                )))
            }
            AnthropicStreamEvent::MessageDelta { delta, mut usage } => {
                // Preserve legacy skipping of a message_delta without stop_reason.
                if !tracker.responses && delta.stop_reason.is_none() {
                    return None;
                }
                if let Some(initial) = tracker.initial_usage.as_ref() {
                    usage.input_tokens = usage.input_tokens.or(initial.input_tokens);
                    usage.cache_read_input_tokens = usage
                        .cache_read_input_tokens
                        .or(initial.cache_read_input_tokens);
                    usage.cache_creation_input_tokens = usage
                        .cache_creation_input_tokens
                        .or(initial.cache_creation_input_tokens);
                }
                let chunk = Self::tool_summary_chunk(request_id, model, tracker, &delta, &usage);
                if tracker.responses && delta.stop_reason.is_some() && chunk.finish_reason.is_none()
                {
                    return Some(Err(rook_core::StreamValidationError::attach(
                        CortexError::provider("Unknown Anthropic stop_reason"),
                        chunk.usage,
                    )));
                }
                if tracker.responses {
                    // Subsequent partial snapshots inherit the latest accepted counters,
                    // including explicit zeros, rather than the message_start counters.
                    tracker.initial_usage = Some(usage);
                }
                Some(Ok(chunk))
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

        if req.metadata.origin != "responses"
            && anthropic_resp
                .content
                .iter()
                .any(|block| block.id.is_invalid_type())
        {
            return Err(CortexError::provider("invalid Anthropic tool-call id type"));
        }

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
                    let canonical_id =
                        normalize_tool_id(&block.id, req.metadata.origin == "responses");
                    let id = canonical_id.clone().unwrap_or_default();
                    let name = block.name.unwrap_or_default();
                    let input = block.input.unwrap_or(serde_json::Value::Null);
                    tool_calls.push(rook_core::MessageToolCall {
                        id: if req.metadata.origin == "responses" {
                            canonical_id
                        } else {
                            Some(id.clone())
                        },
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
                .or_else(|| (req.metadata.origin != "responses").then_some(FinishReason::Stop)),
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
        let stream = futures::stream::unfold(
            (
                resp.bytes_stream(),
                SseBuffer::new(),
                ToolUseTracker {
                    responses: req.metadata.origin == "responses",
                    ..ToolUseTracker::default()
                },
                VecDeque::<CortexResult<StreamChunk>>::new(),
                false,
            ),
            move |(mut bytes, mut buffer, mut tracker, mut pending, mut terminated)| {
                let request_id = request_id.clone();
                let model = model.clone();
                async move {
                    loop {
                        if terminated {
                            return None;
                        }
                        if let Some(result) = pending.pop_front() {
                            terminated = tracker.responses && result.is_err();
                            return Some((result, (bytes, buffer, tracker, pending, terminated)));
                        }
                        // Drain one HTTP frame before polling another: no background
                        // producer or queue accumulating across upstream reads.
                        match bytes.next().await {
                            Some(Ok(chunk)) => pending.extend(Self::process_bytes(
                                &request_id,
                                &model,
                                &mut buffer,
                                &mut tracker,
                                &chunk,
                            )),
                            Some(Err(error)) => {
                                return Some((
                                    Err(CortexError::provider(format!(
                                        "stream read failed: {error}"
                                    ))),
                                    (bytes, buffer, tracker, pending, true),
                                ));
                            }
                            None => {
                                if tracker.responses && buffer.pending_len() > 0 {
                                    return Some((
                                        Err(CortexError::provider(
                                            "Anthropic stream ended with an incomplete SSE event",
                                        )),
                                        (bytes, buffer, tracker, pending, true),
                                    ));
                                }
                                return None;
                            }
                        }
                    }
                }
            },
        );

        Ok(Box::pin(stream))
    }
}


#[cfg(test)]
mod extra_tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    #[test]
    fn test_parse_anthropic_rate_limit_headers_all_metrics() {
        let mut headers = HeaderMap::new();
        headers.insert("anthropic-workspace-id", HeaderValue::from_static("ws_123"));
        headers.insert("anthropic-ratelimit-requests-limit", HeaderValue::from_static("100"));
        headers.insert("anthropic-ratelimit-requests-remaining", HeaderValue::from_static("95"));
        headers.insert("anthropic-ratelimit-requests-reset", HeaderValue::from_static("2026-01-01T00:00:00Z"));

        headers.insert("anthropic-priority-input-tokens-limit", HeaderValue::from_static("5000"));
        headers.insert("anthropic-priority-input-tokens-remaining", HeaderValue::from_static("4000"));

        let provider = ProviderId::new("anthropic-test");
        let now = chrono::Utc::now();
        let obs = parse_anthropic_rate_limit_headers(&headers, provider, now);

        assert!(!obs.is_empty());
        let req_obs = obs.iter().find(|o| o.metric == "requests").unwrap();
        assert_eq!(req_obs.scope, Some("ws_123".to_string()));
        assert_eq!(req_obs.limit, Some(100.0));
        assert_eq!(req_obs.remaining, Some(95.0));

        let prio_obs = obs.iter().find(|o| o.metric == "priority_input_tokens").unwrap();
        assert_eq!(prio_obs.limit, Some(5000.0));
        assert_eq!(prio_obs.remaining, Some(4000.0));
    }

    #[test]
    fn test_map_anthropic_finish_reason_variants() {
        assert_eq!(map_anthropic_finish_reason("end_turn"), Some(FinishReason::Stop));
        assert_eq!(map_anthropic_finish_reason("stop_sequence"), Some(FinishReason::Stop));
        assert_eq!(map_anthropic_finish_reason("pause_turn"), Some(FinishReason::Stop));
        assert_eq!(map_anthropic_finish_reason("max_tokens"), Some(FinishReason::Length));
        assert_eq!(map_anthropic_finish_reason("tool_use"), Some(FinishReason::ToolCalls));
        assert_eq!(map_anthropic_finish_reason("refusal"), Some(FinishReason::ContentFilter));
        assert_eq!(map_anthropic_finish_reason("unknown_reason"), None);
    }
}
