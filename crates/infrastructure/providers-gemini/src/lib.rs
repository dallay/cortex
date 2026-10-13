// providers-gemini — Google Gemini API provider adapter

use async_trait::async_trait;
use reqwest::Client;
use rook_core::{
    ApiFormat, CompletionRequest, CompletionResponse, HealthStatus, ModelId, ProviderPort,
    StreamChunk, TokenUsage,
};
use serde::Deserialize;
use shared_kernel::{ConnectionId, CortexError, CortexResult, ModelId as KModelId, ProviderId};
use std::sync::Arc;

fn map_gemini_finish_reason(reason: &str) -> Option<rook_core::FinishReason> {
    match reason {
        "STOP" => Some(rook_core::FinishReason::Stop),
        "MAX_TOKENS" => Some(rook_core::FinishReason::Length),
        "SAFETY" | "RECITATION" => Some(rook_core::FinishReason::ContentFilter),
        "OTHER" => None,
        _ => None,
    }
}

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
    oauth_token_port: Option<Arc<dyn rook_core::OAuthTokenPort>>,
    connection_id: Option<ConnectionId>,
    project_id: Option<String>,
}

impl GeminiProvider {
    pub fn new(config: GeminiProviderConfig) -> anyhow::Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Arc::new(Self {
            config,
            client,
            oauth_token_port: None,
            connection_id: None,
            project_id: None,
        }))
    }

    pub fn new_with_oauth(
        config: GeminiProviderConfig,
        connection_id: ConnectionId,
        project_id: String,
        oauth_token_port: Arc<dyn rook_core::OAuthTokenPort>,
    ) -> anyhow::Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Arc::new(Self {
            config,
            client,
            oauth_token_port: Some(oauth_token_port),
            connection_id: Some(connection_id),
            project_id: Some(project_id),
        }))
    }

    fn base_url(&self) -> String {
        self.config
            .base_url
            .clone()
            .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_string())
    }

    async fn access_token(&self, force_refresh: bool) -> Result<String, String> {
        match (&self.oauth_token_port, &self.connection_id) {
            (Some(port), Some(connection_id)) => port
                .access_token(connection_id, force_refresh)
                .await
                .map_err(|error| error.to_string()),
            _ => Ok(self.config.api_key.clone()),
        }
    }

    async fn send_models_request(&self, url: &str) -> Result<reqwest::Response, String> {
        for attempt in 0..=1 {
            let token = self.access_token(attempt == 1).await?;
            let mut request = self.client.get(url);
            if self.oauth_token_port.is_some() {
                request = request.header("Authorization", format!("Bearer {token}"));
                if let Some(project_id) = &self.project_id {
                    request = request.header("x-goog-user-project", project_id);
                }
            } else {
                request = request.header("x-goog-api-key", token);
            }
            let response = request.send().await.map_err(|error| error.to_string())?;
            if response.status() != reqwest::StatusCode::UNAUTHORIZED
                || self.oauth_token_port.is_none()
                || attempt == 1
            {
                return Ok(response);
            }
        }
        unreachable!("retry loop always returns a response")
    }

    async fn send_generate_request(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> CortexResult<reqwest::Response> {
        for attempt in 0..=1 {
            let token = self
                .access_token(attempt == 1)
                .await
                .map_err(|_| CortexError::auth_failed("Gemini OAuth token unavailable"))?;
            let mut request = self
                .client
                .post(url)
                .header("content-type", "application/json");
            if self.oauth_token_port.is_some() {
                request = request.header("Authorization", format!("Bearer {token}"));
                if let Some(project_id) = &self.project_id {
                    request = request.header("x-goog-user-project", project_id);
                }
            } else {
                request = request.header("x-goog-api-key", token);
            }
            let response = request
                .json(body)
                .send()
                .await
                .map_err(|error| CortexError::provider(format!("request failed: {error}")))?;
            if response.status() != reqwest::StatusCode::UNAUTHORIZED
                || self.oauth_token_port.is_none()
                || attempt == 1
            {
                return Ok(response);
            }
        }
        unreachable!("retry loop always returns a response")
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
        self.oauth_token_port.is_some() || !self.config.api_key.is_empty()
    }

    async fn health_check(&self) -> HealthStatus {
        // No API key configured — surface as a yellow warning so the
        // user can still Save and add a key later via Edit.
        if !self.is_available() {
            return HealthStatus::Warning {
                provider: self.config.id.clone(),
                latency_ms: 0,
                reason: "No API key configured. You can add one later via Edit.".to_string(),
            };
        }

        let start = std::time::Instant::now();
        let probe_url = format!("{}/v1beta/models", self.base_url());
        match self.send_models_request(&probe_url).await {
            Ok(resp) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                let status = resp.status();
                match rook_core::probes::classify_status_code(status.as_u16()) {
                    rook_core::probes::ProbeClassification::Ok => HealthStatus::Healthy {
                        provider: self.config.id.clone(),
                        latency_ms,
                    },
                    rook_core::probes::ProbeClassification::RateLimited => HealthStatus::Warning {
                        provider: self.config.id.clone(),
                        latency_ms,
                        reason: "Rate limited, but credentials are valid".to_string(),
                    },
                    rook_core::probes::ProbeClassification::AuthRejected(code) => {
                        HealthStatus::Unhealthy {
                            provider: self.config.id.clone(),
                            latency_ms: Some(latency_ms),
                            error: format!(
                                "auth rejected: HTTP {code} — check that your API key is valid and has access to the model"
                            ),
                        }
                    }
                    rook_core::probes::ProbeClassification::ServerError(code)
                    | rook_core::probes::ProbeClassification::ClientError(code) => {
                        HealthStatus::Unhealthy {
                            provider: self.config.id.clone(),
                            latency_ms: Some(latency_ms),
                            error: format!("GET /v1beta/models returned HTTP {code}"),
                        }
                    }
                    // Network errors are constructed by the Err arm below.
                    rook_core::probes::ProbeClassification::NetworkError(_) => {
                        HealthStatus::Unhealthy {
                            provider: self.config.id.clone(),
                            latency_ms: Some(latency_ms),
                            error: "GET /v1beta/models returned an unknown status".to_string(),
                        }
                    }
                }
            }
            Err(e) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                HealthStatus::Unhealthy {
                    provider: self.config.id.clone(),
                    latency_ms: Some(latency_ms),
                    error: format!("GET /v1beta/models failed: {e}"),
                }
            }
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

        let resp = self.send_generate_request(&url, &body).await?;

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
            thinking: None,
            tool_calls: vec![],
            finish_reason: parsed
                .candidates
                .first()
                .and_then(|candidate| candidate.finish_reason.as_deref())
                .and_then(map_gemini_finish_reason),
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
            cache_hit: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn test_map_gemini_finish_reason() {
        assert_eq!(
            map_gemini_finish_reason("STOP"),
            Some(rook_core::FinishReason::Stop)
        );
        assert_eq!(
            map_gemini_finish_reason("MAX_TOKENS"),
            Some(rook_core::FinishReason::Length)
        );
        assert_eq!(
            map_gemini_finish_reason("SAFETY"),
            Some(rook_core::FinishReason::ContentFilter)
        );
        assert_eq!(
            map_gemini_finish_reason("RECITATION"),
            Some(rook_core::FinishReason::ContentFilter)
        );
        assert_eq!(map_gemini_finish_reason("OTHER"), None);
        assert_eq!(map_gemini_finish_reason("UNKNOWN"), None);
    }

    #[tokio::test]
    async fn test_gemini_provider_basic_methods() {
        let config = GeminiProviderConfig {
            id: ProviderId::new("gemini-1"),
            api_key: "".to_string(),
            base_url: None,
            models: vec![KModelId::new("gemini-1.5-pro")],
            timeout_secs: 5,
        };

        let provider = GeminiProvider::new(config).unwrap();

        assert_eq!(provider.id().as_str(), "gemini-1");
        assert_eq!(
            provider.supported_models(),
            &[KModelId::new("gemini-1.5-pro")]
        );
        assert_eq!(provider.api_format(), ApiFormat::OpenAI);
        assert!(!provider.is_available());

        // health check with empty key
        let health = provider.health_check().await;
        match health {
            HealthStatus::Warning { reason, .. } => {
                assert!(reason.contains("No API key configured"));
            }
            _ => panic!("expected HealthStatus::Warning"),
        }

        // stream returns unimplemented error
        let req = CompletionRequest {
            id: shared_kernel::RequestId::new(),
            model: ModelId::new("gemini-1.5-pro"),
            messages: vec![rook_core::Message {
                role: rook_core::Role::User,
                content: rook_core::MessageContent::Text("Hello".to_string()),
                tool_calls: vec![],
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            tools: None,
            tool_choice: None,
            metadata: rook_core::RequestMetadata {
                origin: "test".to_string(),
                cacheable: false,
                priority: 1,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: Default::default(),
        };

        let stream_res = provider.stream(&req).await;
        assert!(stream_res.is_ok());
        let mut st = stream_res.unwrap();
        let chunk = st.next().await;
        assert!(chunk.unwrap().is_err());
    }

    #[tokio::test]
    async fn test_gemini_provider_with_wiremock() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        let config = GeminiProviderConfig {
            id: ProviderId::new("gemini-test"),
            api_key: "test-key-123".to_string(),
            base_url: Some(mock_server.uri()),
            models: vec![KModelId::new("gemini-1.5-pro")],
            timeout_secs: 5,
        };

        let provider = GeminiProvider::new(config).unwrap();
        assert!(provider.is_available());

        // Mock health check (GET /v1beta/models)
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .and(header("x-goog-api-key", "test-key-123"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let health = provider.health_check().await;
        match health {
            HealthStatus::Healthy { provider, .. } => {
                assert_eq!(provider.as_str(), "gemini-test");
            }
            _ => panic!("expected HealthStatus::Healthy"),
        }

        // Mock generateContent
        let response_body = serde_json::json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{ "text": "Hello from Gemini!" }]
                },
                "finishReason": "STOP",
                "index": 0
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 5,
                "totalTokenCount": 15
            }
        });

        let expected_body = serde_json::json!({
            "contents": [
                {
                    "role": "user",
                    "parts": [{ "text": "Hi" }]
                }
            ],
            "systemInstruction": {
                "parts": [{ "text": "You are a helpful assistant." }]
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-1.5-pro:generateContent"))
            .and(header("x-goog-api-key", "test-key-123"))
            .and(wiremock::matchers::body_json(expected_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&mock_server)
            .await;

        let req = CompletionRequest {
            id: shared_kernel::RequestId::new(),
            model: ModelId::new("gemini-1.5-pro"),
            messages: vec![
                rook_core::Message {
                    role: rook_core::Role::System,
                    content: rook_core::MessageContent::Text(
                        "You are a helpful assistant.".to_string(),
                    ),
                    tool_calls: vec![],
                },
                rook_core::Message {
                    role: rook_core::Role::User,
                    content: rook_core::MessageContent::Text("Hi".to_string()),
                    tool_calls: vec![],
                },
            ],
            stream: false,
            max_tokens: None,
            temperature: None,
            tools: None,
            tool_choice: None,
            metadata: rook_core::RequestMetadata {
                origin: "test".to_string(),
                cacheable: false,
                priority: 1,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: Default::default(),
        };

        let resp = provider.complete(&req).await.unwrap();
        assert_eq!(resp.content, "Hello from Gemini!");
        assert_eq!(resp.usage.prompt_tokens, 10);
        assert_eq!(resp.usage.completion_tokens, 5);
        assert_eq!(resp.usage.total_tokens, 15);
        assert_eq!(resp.finish_reason, Some(rook_core::FinishReason::Stop));
    }
}
