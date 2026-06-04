// OpenAI provider implementation — stub until existing impls are migrated

use async_trait::async_trait;
use futures::{Stream, TryStreamExt};
use reqwest::Client;
use rook_core::{
    ApiFormat, CompletionRequest, CompletionResponse, FinishReason, HealthStatus, ModelId,
    ProviderPort, StreamChunk, TokenUsage,
};

use shared_kernel::{CortexError, CortexResult, ProviderId, RequestId};
use sse_stream::SseBuffer;

/// Truncate a string to at most `max` chars, safe across UTF-8 multi-byte boundaries.
fn char_safe_truncate(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{truncated}… (truncated)")
    } else {
        truncated
    }
}

/// Sanitize and truncate error response body to prevent sensitive data leakage.
fn sanitize_error_body(body: &str) -> String {
    const MAX_LENGTH: usize = 200;
    const SENSITIVE_KEYS: &[&str] = &[
        "api_key",
        "authorization",
        "token",
        "access_token",
        "secret",
        "headers",
    ];

    // Try to parse as JSON and redact sensitive fields
    if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(obj) = json.as_object_mut() {
            let keys_to_redact: Vec<String> = obj
                .keys()
                .filter(|k| {
                    let lower = k.to_lowercase();
                    SENSITIVE_KEYS.iter().any(|s| lower.contains(s))
                })
                .cloned()
                .collect();
            for key in keys_to_redact {
                obj.insert(key, serde_json::Value::String("(redacted)".to_string()));
            }
        }
        let sanitized = serde_json::to_string(&json).unwrap_or_else(|_| body.to_string());
        char_safe_truncate(&sanitized, MAX_LENGTH)
    } else {
        // Fall back to plain text truncation
        char_safe_truncate(body, MAX_LENGTH)
    }
}

/// Map an OpenAI HTTP error response to a typed `CortexError`.
///
/// Reads `Retry-After` header for 429 and sanitizes the body to prevent leakage.
async fn map_openai_http_error(provider_id: &ProviderId, resp: reqwest::Response) -> CortexError {
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
    let sanitized = sanitize_error_body(&raw_body);

    match status.as_u16() {
        401 => CortexError::auth_failed("OpenAI authentication failed"),
        429 => {
            let retry_secs = retry_after.unwrap_or(60);
            if let Some(reset) = reset_at {
                CortexError::rate_limited_with_reset(provider_id.clone(), retry_secs, reset)
            } else {
                CortexError::rate_limited(provider_id.clone(), retry_secs)
            }
        }
        400 => CortexError::invalid_request(sanitized),
        _ => CortexError::provider(format!("OpenAI error {status}: {sanitized}")),
    }
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<OpenAIStreamOptions>,
}

#[derive(Debug, serde::Serialize)]
struct OpenAIStreamOptions {
    include_usage: bool,
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
    #[serde(default)]
    prompt_tokens_details: Option<OpenAIPromptTokensDetails>,
    #[serde(default)]
    completion_tokens_details: Option<OpenAICompletionTokensDetails>,
}

#[derive(Debug, serde::Deserialize)]
struct OpenAIPromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
struct OpenAICompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
struct OpenAIStreamResponse {
    #[allow(dead_code)]
    id: String,
    model: String,
    choices: Vec<OpenAIStreamChoice>,
    usage: Option<OpenAIUsage>,
}

#[derive(Debug, serde::Deserialize)]
struct OpenAIStreamChoice {
    delta: OpenAIStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct OpenAIStreamDelta {
    content: Option<String>,
}

fn parse_finish_reason(reason: &str) -> Option<FinishReason> {
    match reason {
        "stop" => Some(FinishReason::Stop),
        "length" => Some(FinishReason::Length),
        "content_filter" => Some(FinishReason::ContentFilter),
        "tool_calls" => Some(FinishReason::ToolCalls),
        _ => None,
    }
}

#[async_trait]
impl ProviderPort for OpenAIProvider {
    fn id(&self) -> &ProviderId {
        &self.config.id
    }

    fn supported_models(&self) -> &[ModelId] {
        &self.config.models
    }

    fn api_format(&self) -> ApiFormat {
        ApiFormat::OpenAI
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
            Ok(resp) if resp.status().is_success() => HealthStatus::Healthy {
                provider: self.config.id.clone(),
                latency_ms: start.elapsed().as_millis() as u64,
            },
            Ok(resp) => HealthStatus::Unhealthy {
                provider: self.config.id.clone(),
                latency_ms: Some(start.elapsed().as_millis() as u64),
                error: format!("HTTP {}", resp.status()),
            },
            Err(e) => HealthStatus::Unhealthy {
                provider: self.config.id.clone(),
                latency_ms: None,
                error: e.to_string(),
            },
        }
    }

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
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
                    content: m.content.as_text().to_string(),
                })
                .collect(),
            stream: false,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            tools: req.tools.clone(),
            tool_choice: req.tool_choice.clone(),
            stream_options: None,
        };

        let start = std::time::Instant::now();
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.config.base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| CortexError::provider(format!("request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(map_openai_http_error(&self.config.id, resp).await);
        }

        let openai_resp: OpenAIResponse = resp
            .json()
            .await
            .map_err(|e| CortexError::provider(format!("json parse failed: {e}")))?;

        let choice = openai_resp
            .choices
            .first()
            .ok_or_else(|| CortexError::not_found("no choices in response"))?;

        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.config.id.clone(),
            model: ModelId::new(openai_resp.model),
            content: choice.message.content.clone(),
            content_blocks: vec![rook_core::MessageContent::Text(
                choice.message.content.clone(),
            )],
            usage: TokenUsage {
                prompt_tokens: openai_resp.usage.prompt_tokens,
                completion_tokens: openai_resp.usage.completion_tokens,
                total_tokens: openai_resp.usage.total_tokens,
                cache_read_tokens: openai_resp
                    .usage
                    .prompt_tokens_details
                    .as_ref()
                    .and_then(|d| d.cached_tokens),
                cache_creation_tokens: None,
                reasoning_tokens: openai_resp
                    .usage
                    .completion_tokens_details
                    .as_ref()
                    .and_then(|d| d.reasoning_tokens),
                estimated_cost_usd: None, // Cost calculated in usecase layer
            },
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
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
                    content: m.content.as_text().to_string(),
                })
                .collect(),
            stream: true,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            tools: req.tools.clone(),
            tool_choice: req.tool_choice.clone(),
            stream_options: Some(OpenAIStreamOptions {
                include_usage: true,
            }),
        };

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.config.base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| CortexError::provider(format!("request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(map_openai_http_error(&self.config.id, resp).await);
        }

        let request_id = req.id.clone();
        let mut sse_buffer = SseBuffer::new();

        fn process_bytes(
            request_id: &RequestId,
            sse_buffer: &mut SseBuffer,
            bytes: &[u8],
        ) -> impl Stream<Item = Result<StreamChunk, CortexError>> {
            let events = sse_buffer.push(bytes);
            let mut chunks = Vec::new();

            for event_text in events {
                for data_line in event_text.lines().filter_map(|l| l.strip_prefix("data: ")) {
                    if data_line.trim() == "[DONE]" {
                        continue;
                    }

                    let parsed: OpenAIStreamResponse = match serde_json::from_str(data_line) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };

                    let choice = parsed.choices.first();

                    let usage = parsed.usage.map(|usage| TokenUsage {
                        prompt_tokens: usage.prompt_tokens,
                        completion_tokens: usage.completion_tokens,
                        total_tokens: usage.total_tokens,
                        cache_read_tokens: usage
                            .prompt_tokens_details
                            .as_ref()
                            .and_then(|d| d.cached_tokens),
                        cache_creation_tokens: None,
                        reasoning_tokens: usage
                            .completion_tokens_details
                            .as_ref()
                            .and_then(|d| d.reasoning_tokens),
                        estimated_cost_usd: None,
                    });

                    chunks.push(StreamChunk {
                        id: request_id.clone(),
                        model: ModelId::new(parsed.model),
                        delta: choice
                            .and_then(|c| c.delta.content.clone())
                            .unwrap_or_default(),
                        finish_reason: choice
                            .and_then(|c| c.finish_reason.as_deref())
                            .and_then(parse_finish_reason),
                        usage,
                    });
                }
            }

            futures::stream::iter(chunks.into_iter().map(Ok))
        }

        let stream = resp
            .bytes_stream()
            .map_err(|e| CortexError::provider(format!("stream read failed: {e}")))
            .and_then(move |bytes| {
                let request_id = request_id.clone();
                futures::future::ok(process_bytes(&request_id, &mut sse_buffer, &bytes))
            })
            .try_flatten();

        Ok(Box::pin(stream))
    }
}
