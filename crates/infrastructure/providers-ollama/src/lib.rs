// providers-ollama — Ollama local API provider adapter

use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt};
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

    fn build_stream_request(req: &CompletionRequest) -> serde_json::Value {
        serde_json::json!({
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
        })
    }

    async fn send_stream_request(
        &self,
        body: &serde_json::Value,
    ) -> CortexResult<reqwest::Response> {
        self.client
            .post(format!("{}/api/chat", self.config.base_url))
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
                "Ollama error {status}: {body}"
            )));
        }
        Ok(resp)
    }

    fn parse_line_to_chunk(
        line: String,
        request_id: &shared_kernel::RequestId,
    ) -> Option<Result<StreamChunk, CortexError>> {
        let parsed: OllamaChatResponse = serde_json::from_str(&line).ok()?;
        let prompt_tokens = parsed.prompt_eval_count.unwrap_or(0);
        let completion_tokens = parsed.eval_count.unwrap_or(0);

        Some(Ok(StreamChunk {
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
        }))
    }

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
                        None => return None,
                    };

                    // Convert to UTF-8 string and append to line buffer
                    let text = match String::from_utf8(bytes.to_vec()) {
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
