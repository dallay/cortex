// providers-ollama — Ollama local API provider adapter

use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt};
use providers_core::{process_bytes, role_to_string};
use reqwest::{Client, StatusCode};
use rook_core::{
    ApiFormat, CompletionRequest, CompletionResponse, HealthStatus, ModelId, ProviderPort,
    StreamChunk, TokenUsage, ToolCallDelta, ToolCallFunctionDelta,
};
use serde::Deserialize;
use shared_kernel::{CortexError, CortexResult, ModelId as KModelId, ProviderId};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    model: String,
    message: OllamaMessage,
    done: bool,
    #[allow(dead_code)]
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    total_duration: Option<u64>,
    #[serde(default)]
    load_duration: Option<u64>,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    prompt_eval_cached_count: Option<u32>,
    #[serde(default)]
    prompt_eval_duration: Option<u64>,
    #[serde(default)]
    eval_count: Option<u32>,
    #[serde(default)]
    eval_duration: Option<u64>,
    #[allow(dead_code)]
    #[serde(default)]
    logprobs: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    #[allow(dead_code)]
    role: String,
    content: String,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaToolCall>>,
    #[allow(dead_code)]
    #[serde(default)]
    images: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OllamaToolCall {
    #[serde(default)]
    id: Option<String>,
    function: OllamaFunctionCall,
}

#[derive(Debug, Deserialize)]
struct OllamaFunctionCall {
    name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct OllamaProviderConfig {
    pub id: ProviderId,
    pub base_url: String,
    pub models: Vec<KModelId>,
    pub timeout_secs: u64,
    /// Optional API key. When set, requests include
    /// `Authorization: Bearer <key>` — required for Ollama Cloud.
    /// Local Ollama instances ignore this header.
    pub api_key: Option<String>,
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

    /// Build a request builder for the Ollama chat endpoint, attaching
    /// the optional `Authorization: Bearer` header when an API key is
    /// configured. Local Ollama ignores the header; Ollama Cloud requires
    /// it.
    fn chat_request(&self, body: &serde_json::Value) -> reqwest::RequestBuilder {
        let url = format!("{}/api/chat", self.config.base_url);
        let mut req = self.client.post(url).json(body);
        if let Some(api_key) = self.config.api_key.as_deref() {
            if !api_key.is_empty() {
                req = req.bearer_auth(api_key);
            }
        }
        req
    }

    /// Build a request builder for `GET /api/tags` (lightweight probe).
    /// Note: Ollama Cloud's `/api/tags` is publicly accessible (returns
    /// the same model catalog for everyone, no auth required). So this
    /// endpoint alone cannot validate credentials — we use it only as
    /// a "is the host reachable?" check, and follow up with a mini
    /// chat request to verify the Bearer token.
    fn tags_request(&self) -> reqwest::RequestBuilder {
        let url = format!("{}/api/tags", self.config.base_url);
        let mut req = self.client.get(url);
        if let Some(api_key) = self.config.api_key.as_deref() {
            if !api_key.is_empty() {
                req = req.bearer_auth(api_key);
            }
        }
        req
    }

    /// Build the request body JSON for the Ollama chat API.
    fn build_request_body(req: &CompletionRequest, stream: bool) -> serde_json::Value {
        let messages: Vec<serde_json::Value> = req
            .messages
            .iter()
            .map(|m| {
                let mut message = serde_json::json!({
                    "role": role_to_string(m.role),
                    "content": m.content.as_text(),
                });
                if !m.tool_calls.is_empty() {
                    message["tool_calls"] = serde_json::Value::Array(
                        m.tool_calls
                            .iter()
                            .map(|tool_call| {
                                serde_json::json!({
                                    "function": {
                                        "name": tool_call.name,
                                        "arguments": tool_call.arguments,
                                    }
                                })
                            })
                            .collect(),
                    );
                }
                message
            })
            .collect();
        serde_json::json!({
            "model": req.model.to_string(),
            "messages": messages,
            "stream": stream,
        })
    }

    async fn send_request(&self, body: &serde_json::Value) -> CortexResult<reqwest::Response> {
        self.chat_request(body)
            .send()
            .await
            .map_err(|e| CortexError::provider(format!("request failed: {e}")))
    }

    /// Extract rate limit info from response headers, if present.
    /// Ollama Cloud may return Retry-After or X-RateLimit-* headers.
    fn extract_rate_limit_info(resp: &reqwest::Response) -> (Option<u64>, Option<u64>) {
        let retry_after_secs = resp
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        let reset_at = resp
            .headers()
            .get("X-RateLimit-Reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        (retry_after_secs, reset_at)
    }

    fn parse_line_to_chunk(
        line: String,
        request_id: &shared_kernel::RequestId,
    ) -> Option<Result<StreamChunk, CortexError>> {
        let parsed: OllamaChatResponse = serde_json::from_str(&line).ok()?;
        let prompt_tokens = parsed.prompt_eval_count.unwrap_or(0);
        let completion_tokens = parsed.eval_count.unwrap_or(0);
        if parsed.done {
            Self::trace_timings(&parsed);
        }
        let finish_reason = Self::finish_reason(&parsed);
        let tool_calls = parsed.message.tool_calls.unwrap_or_default();

        Some(Ok(StreamChunk {
            id: request_id.clone(),
            model: ModelId::new(parsed.model.clone()),
            delta: parsed.message.content,
            thinking: parsed.message.thinking,
            tool_calls: tool_calls
                .into_iter()
                .enumerate()
                .map(|(index, tool_call)| ToolCallDelta {
                    index: u32::try_from(index).unwrap_or(u32::MAX),
                    id: tool_call.id,
                    function: ToolCallFunctionDelta {
                        name: Some(tool_call.function.name),
                        arguments: Some(
                            serde_json::to_string(&tool_call.function.arguments)
                                .unwrap_or_else(|_| "null".to_string()),
                        ),
                    },
                })
                .collect(),
            finish_reason,
            usage: if parsed.done {
                Some(TokenUsage {
                    prompt_tokens,
                    completion_tokens,
                    total_tokens: prompt_tokens.saturating_add(completion_tokens),
                    cache_read_tokens: parsed.prompt_eval_cached_count.map(u64::from),
                    cache_creation_tokens: None,
                    reasoning_tokens: None,
                    estimated_cost_usd: None,
                })
            } else {
                None
            },
        }))
    }

    fn map_finish_reason(reason: &str) -> Option<rook_core::FinishReason> {
        match reason {
            "stop" => Some(rook_core::FinishReason::Stop),
            "length" => Some(rook_core::FinishReason::Length),
            "tool_calls" => Some(rook_core::FinishReason::ToolCalls),
            _ => None,
        }
    }

    fn finish_reason(parsed: &OllamaChatResponse) -> Option<rook_core::FinishReason> {
        if !parsed.done {
            return None;
        }
        match parsed.done_reason.as_deref() {
            Some(reason) => Self::map_finish_reason(reason),
            // Older Ollama responses omit done_reason. Keep the prior
            // compatibility behavior for absence only; unknown explicit
            // reasons remain unknown instead of being coerced to Stop.
            None => Some(rook_core::FinishReason::Stop),
        }
    }

    fn trace_timings(parsed: &OllamaChatResponse) {
        if parsed.total_duration.is_some()
            || parsed.load_duration.is_some()
            || parsed.prompt_eval_duration.is_some()
            || parsed.eval_duration.is_some()
        {
            tracing::info!(
                total_duration_ns = parsed.total_duration,
                load_duration_ns = parsed.load_duration,
                prompt_eval_duration_ns = parsed.prompt_eval_duration,
                eval_duration_ns = parsed.eval_duration,
                "Ollama response timings"
            );
        }
    }

    /// Extract complete lines from the line buffer.
    ///
    /// Splits on newlines and returns all non-empty lines.
    /// The remaining partial line stays in `line_buffer`.
    fn extract_complete_lines(line_buffer: &mut String) -> Vec<String> {
        let mut complete_lines = Vec::new();
        while let Some(newline_pos) = line_buffer.find('\n') {
            let line = line_buffer[..newline_pos].to_string();
            *line_buffer = line_buffer[newline_pos + 1..].to_string();
            if !line.trim().is_empty() {
                complete_lines.push(line);
            }
        }
        complete_lines
    }

    /// Map HTTP error status to typed CortexError.
    ///
    /// Handles 400 Bad Request and 401 Unauthorized with specific error types.
    /// All other non-success statuses become generic provider errors.
    fn map_status_to_error(status: StatusCode, body_text: &str) -> CortexError {
        match status {
            StatusCode::BAD_REQUEST => {
                CortexError::invalid_request(format!("Ollama bad request: {body_text}"))
            }
            StatusCode::UNAUTHORIZED => {
                CortexError::auth_failed(format!("Ollama auth failed: {body_text}"))
            }
            _ => CortexError::provider(format!("Ollama error {status}: {body_text}")),
        }
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
        // Two-step probe:
        // 1. `GET /api/tags` — verify the host is reachable. On local
        //    Ollama this also confirms the daemon is up; on Ollama
        //    Cloud this endpoint is public, so 200 here only proves
        //    reachability, NOT auth.
        // 2. If the provider has a configured API key, follow up with
        //    a tiny `POST /api/chat` (one token in, zero tokens out —
        //    we send `stream: false` and inspect only the HTTP status).
        //    Ollama Cloud returns 401 on this endpoint when the Bearer
        //    token is missing or invalid, which is the real "are my
        //    credentials accepted?" check.
        //
        // Per-step latency: when step 2 fails (e.g. 429, 401), we
        // report the failing step's latency, not total elapsed (ADR-5
        // in design.md). When both steps succeed, we report the total.
        let total_start = std::time::Instant::now();
        let total_latency_ms = || total_start.elapsed().as_millis() as u64;

        // Step 1: reachability.
        let step1_start = std::time::Instant::now();
        let tags_result = self.tags_request().send().await;
        match tags_result {
            Ok(resp) if resp.status().is_success() => {}
            Ok(resp) => {
                return HealthStatus::Unhealthy {
                    provider: self.config.id.clone(),
                    latency_ms: Some(step1_start.elapsed().as_millis() as u64),
                    error: format!("GET /api/tags returned HTTP {}", resp.status()),
                };
            }
            Err(e) => {
                return HealthStatus::Unhealthy {
                    provider: self.config.id.clone(),
                    latency_ms: Some(step1_start.elapsed().as_millis() as u64),
                    error: format!("GET /api/tags failed: {e}"),
                };
            }
        }

        // Step 2: credential check (only when a key is configured).
        if self.config.api_key.as_deref().is_none_or(str::is_empty) {
            // No key configured — surface as a yellow warning so the
            // user is nudged to add a key (e.g. for Ollama Cloud).
            return HealthStatus::Warning {
                provider: self.config.id.clone(),
                latency_ms: 0,
                reason: "No API key configured. You can add one later via Edit.".to_string(),
            };
        }

        // Use a known free-tier model for the probe. The catalog may have
        // subscription-gated models (e.g. deepseek-v4-pro) which return 404,
        // so we explicitly use gpt-oss:20b which works on Ollama Cloud's free tier.
        // Also strip any provider-prefix from the model name (e.g.
        // "ollamacloud/deepseek-v4-pro" → "deepseek-v4-pro") since the API
        // expects just the raw model ID.
        let raw_model = "gpt-oss:20b";
        let probe_model = raw_model.strip_prefix("ollamacloud/").unwrap_or(raw_model);
        let probe_body = serde_json::json!({
            "model": probe_model,
            "messages": [{"role": "user", "content": "hi"}],
            "stream": false,
        });

        // Per-step timing: report the failing step's latency, not total.
        let step2_start = std::time::Instant::now();
        match self.chat_request(&probe_body).send().await {
            Ok(resp) if resp.status().is_success() => HealthStatus::Healthy {
                provider: self.config.id.clone(),
                latency_ms: total_latency_ms(),
            },
            Ok(resp) => {
                let step2_latency = step2_start.elapsed().as_millis() as u64;
                let status = resp.status();
                match rook_core::probes::classify_status_code(status.as_u16()) {
                    rook_core::probes::ProbeClassification::AuthRejected(code) => {
                        HealthStatus::Unhealthy {
                            provider: self.config.id.clone(),
                            latency_ms: Some(step2_latency),
                            error: format!(
                                "auth rejected: HTTP {code} — check that your API key is valid and has access to the model"
                            ),
                        }
                    }
                    rook_core::probes::ProbeClassification::RateLimited => HealthStatus::Warning {
                        provider: self.config.id.clone(),
                        latency_ms: step2_latency,
                        reason: "Rate limited, but credentials are valid".to_string(),
                    },
                    rook_core::probes::ProbeClassification::ServerError(code)
                    | rook_core::probes::ProbeClassification::ClientError(code) => {
                        HealthStatus::Unhealthy {
                            provider: self.config.id.clone(),
                            latency_ms: Some(step2_latency),
                            error: format!("POST /api/chat returned HTTP {code}"),
                        }
                    }
                    // Network errors are constructed in the Err arm below.
                    rook_core::probes::ProbeClassification::Ok
                    | rook_core::probes::ProbeClassification::NetworkError(_) => {
                        HealthStatus::Unhealthy {
                            provider: self.config.id.clone(),
                            latency_ms: Some(step2_latency),
                            error: format!("POST /api/chat returned an unexpected status {status}"),
                        }
                    }
                }
            }
            Err(e) => {
                let step2_latency = step2_start.elapsed().as_millis() as u64;
                HealthStatus::Unhealthy {
                    provider: self.config.id.clone(),
                    latency_ms: Some(step2_latency),
                    error: format!("POST /api/chat failed: {e}"),
                }
            }
        }
    }

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        let body = Self::build_request_body(req, false);

        let start = std::time::Instant::now();
        let resp = self.send_request(&body).await?;

        if !resp.status().is_success() {
            let status = resp.status();

            // Handle 429 Rate Limited — extract headers BEFORE consuming response
            if status == StatusCode::TOO_MANY_REQUESTS {
                let (retry_after_secs, reset_at) = Self::extract_rate_limit_info(&resp);
                let retry_secs = retry_after_secs.unwrap_or(60);
                let provider_id = ProviderId::new(self.config.id.as_str());
                if let Some(reset) = reset_at {
                    return Err(CortexError::rate_limited_with_reset(
                        provider_id,
                        retry_secs,
                        reset,
                    ));
                } else {
                    return Err(CortexError::rate_limited(provider_id, retry_secs));
                }
            }

            // Map non-success status to typed error (handles 400, 401, etc.)
            let body_text = resp.text().await.unwrap_or_default();
            return Err(Self::map_status_to_error(status, &body_text));
        }

        let parsed: OllamaChatResponse = resp
            .json()
            .await
            .map_err(|e| CortexError::provider(format!("json parse failed: {e}")))?;

        Self::trace_timings(&parsed);
        let prompt_tokens = parsed.prompt_eval_count.unwrap_or(0);
        let completion_tokens = parsed.eval_count.unwrap_or(0);
        let finish_reason = Self::finish_reason(&parsed);
        let tool_calls = parsed
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tool_call| rook_core::MessageToolCall {
                id: tool_call.id,
                name: tool_call.function.name,
                arguments: tool_call.function.arguments,
            })
            .collect();

        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.config.id.clone(),
            model: ModelId::new(parsed.model),
            content: parsed.message.content.clone(),
            content_blocks: vec![rook_core::MessageContent::Text(parsed.message.content)],
            thinking: parsed.message.thinking,
            tool_calls,
            finish_reason,
            usage: TokenUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens.saturating_add(completion_tokens),
                cache_read_tokens: parsed.prompt_eval_cached_count.map(u64::from),
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None, // Local model — no cost
            },
            latency_ms: start.elapsed().as_millis() as u64,
            cache_hit: None,
        })
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        let body = Self::build_request_body(req, true);
        let resp = self.send_request(&body).await?;

        if !resp.status().is_success() {
            let status = resp.status();

            // Handle 429 Rate Limited — extract headers BEFORE consuming response
            if status == StatusCode::TOO_MANY_REQUESTS {
                let (retry_after_secs, reset_at) = Self::extract_rate_limit_info(&resp);
                let retry_secs = retry_after_secs.unwrap_or(60);
                let provider_id = ProviderId::new(self.config.id.as_str());
                if let Some(reset) = reset_at {
                    return Err(CortexError::rate_limited_with_reset(
                        provider_id,
                        retry_secs,
                        reset,
                    ));
                } else {
                    return Err(CortexError::rate_limited(provider_id, retry_secs));
                }
            }

            // Map non-success status to typed error (handles 400, 401, etc.)
            let body_text = resp.text().await.unwrap_or_default();
            return Err(Self::map_status_to_error(status, &body_text));
        }

        let request_id = req.id.clone();

        // Line-buffered SSE stream: accumulate bytes into lines, parse each complete line.
        let stream = futures::stream::unfold(
            (resp.bytes_stream(), String::new()),
            move |(mut byte_stream, mut line_buffer)| {
                let request_id = request_id.clone();
                async move {
                    // Read next byte chunk
                    let bytes = match byte_stream.next().await {
                        Some(Ok(b)) => b,
                        Some(Err(e)) => {
                            return Some((
                                Err(CortexError::provider(format!("stream read failed: {e}"))),
                                (byte_stream, line_buffer),
                            ));
                        }
                        None if line_buffer.trim().is_empty() => return None,
                        None => {
                            let final_line = std::mem::take(&mut line_buffer);
                            let chunks = Self::parse_line_to_chunk(final_line, &request_id)
                                .into_iter()
                                .collect::<Vec<_>>();
                            return Some((
                                Ok(futures::stream::iter(chunks)),
                                (byte_stream, line_buffer),
                            ));
                        }
                    };

                    // Convert to UTF-8 string using providers_core::process_bytes
                    let text = match process_bytes(&bytes) {
                        Ok(t) => t,
                        Err(e) => {
                            return Some((
                                Err(CortexError::provider(format!("invalid utf-8: {e}"))),
                                (byte_stream, line_buffer),
                            ));
                        }
                    };
                    line_buffer.push_str(&text);

                    // Extract all complete lines
                    let complete_lines = Self::extract_complete_lines(&mut line_buffer);

                    // Parse each line into StreamChunk
                    let chunks: Vec<Result<StreamChunk, CortexError>> = complete_lines
                        .into_iter()
                        .filter_map(|line| Self::parse_line_to_chunk(line, &request_id))
                        .collect();

                    Some((
                        Ok(futures::stream::iter(chunks)),
                        (byte_stream, line_buffer),
                    ))
                }
            },
        )
        .try_flatten();

        Ok(Box::pin(stream))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_complete_lines_single_line() {
        let mut buffer = "hello world\n".to_string();
        let lines = OllamaProvider::extract_complete_lines(&mut buffer);
        assert_eq!(lines, vec!["hello world"]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_extract_complete_lines_multiple_lines() {
        let mut buffer = "line1\nline2\nline3\n".to_string();
        let lines = OllamaProvider::extract_complete_lines(&mut buffer);
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_extract_complete_lines_empty_lines_filtered() {
        let mut buffer = "line1\n\nline2\n\n\nline3\n".to_string();
        let lines = OllamaProvider::extract_complete_lines(&mut buffer);
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn test_extract_complete_lines_whitespace_only_filtered() {
        let mut buffer = "line1\n   \nline2\n".to_string();
        let lines = OllamaProvider::extract_complete_lines(&mut buffer);
        assert_eq!(lines, vec!["line1", "line2"]);
    }

    #[test]
    fn test_extract_complete_lines_partial_line_kept() {
        let mut buffer = "line1\npartial".to_string();
        let lines = OllamaProvider::extract_complete_lines(&mut buffer);
        assert_eq!(lines, vec!["line1"]);
        assert_eq!(buffer, "partial");
    }

    #[test]
    fn fragmented_ndjson_preserves_thinking_tool_calls_and_finish_reason() {
        let line = r#"{"model":"qwen3","message":{"role":"assistant","content":"","thinking":"checking","tool_calls":[{"function":{"name":"lookup","arguments":{"key":"value"}}}]},"done":true,"done_reason":"length"}"#;
        let mut buffer = String::new();
        let mut lines = Vec::new();
        for fragment in line.as_bytes().chunks(11) {
            buffer.push_str(std::str::from_utf8(fragment).unwrap());
            lines.extend(OllamaProvider::extract_complete_lines(&mut buffer));
        }
        buffer.push('\n');
        lines.extend(OllamaProvider::extract_complete_lines(&mut buffer));

        assert_eq!(lines.len(), 1);
        let chunk = OllamaProvider::parse_line_to_chunk(
            lines.pop().unwrap(),
            &shared_kernel::RequestId::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(chunk.thinking.as_deref(), Some("checking"));
        assert_eq!(chunk.tool_calls[0].function.name.as_deref(), Some("lookup"));
        assert_eq!(chunk.finish_reason, Some(rook_core::FinishReason::Length));
    }

    #[test]
    fn unknown_ollama_done_reason_is_not_reported_as_stop() {
        let response: OllamaChatResponse = serde_json::from_value(serde_json::json!({
            "model": "qwen3",
            "message": {"role": "assistant", "content": ""},
            "done": true,
            "done_reason": "future_reason"
        }))
        .unwrap();
        assert_eq!(OllamaProvider::finish_reason(&response), None);
    }

    #[test]
    fn test_extract_complete_lines_no_newline() {
        let mut buffer = "no newline".to_string();
        let lines = OllamaProvider::extract_complete_lines(&mut buffer);
        assert!(lines.is_empty());
        assert_eq!(buffer, "no newline");
    }

    #[test]
    fn test_extract_complete_lines_empty_buffer() {
        let mut buffer = String::new();
        let lines = OllamaProvider::extract_complete_lines(&mut buffer);
        assert!(lines.is_empty());
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_role_to_string_system() {
        assert_eq!(role_to_string(rook_core::Role::System), "system");
    }

    #[test]
    fn test_role_to_string_user() {
        assert_eq!(role_to_string(rook_core::Role::User), "user");
    }

    #[test]
    fn test_role_to_string_assistant() {
        assert_eq!(role_to_string(rook_core::Role::Assistant), "assistant");
    }

    #[test]
    fn test_role_to_string_developer() {
        assert_eq!(role_to_string(rook_core::Role::Developer), "developer");
    }
}
