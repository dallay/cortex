//! Anthropic Messages API chat provider.
//!
//! Equivalent to the `AnthropicProvider` in `neo4j_agent_memory.llm.adapters`.
//! Uses the native Anthropic `/v1/messages` endpoint rather than going through
//! an OpenAI-compatible proxy.

use async_trait::async_trait;
use muninn_core::extraction::{ChatMessage, CompletionOptions, LlmProvider};
use muninn_core::{MemoryError, MemoryResult};
use serde::{Deserialize, Serialize};

/// Default Anthropic API base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Default Anthropic model.
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-20250514";

/// Default Anthropic API version.
pub const DEFAULT_API_VERSION: &str = "2023-06-01";

/// Anthropic Messages API chat provider.
///
/// Supports system prompts, user/assistant messages, and JSON mode.
#[derive(Clone)]
pub struct AnthropicChat {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    api_version: String,
}

impl std::fmt::Debug for AnthropicChat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicChat")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    system: String,
    temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<AnthropicResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct AnthropicResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
}

impl AnthropicChat {
    /// Create an Anthropic chat provider.
    ///
    /// # Arguments
    /// * `api_key` - Anthropic API key (required, never empty).
    /// * `model` - Model name (defaults to `claude-sonnet-4-20250514`).
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> MemoryResult<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(MemoryError::invalid("Anthropic API key must not be empty"));
        }
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|err| MemoryError::provider(err.to_string()))?;
        Ok(Self {
            http,
            base_url: DEFAULT_BASE_URL.to_owned(),
            api_key,
            model: model.into(),
            api_version: DEFAULT_API_VERSION.to_owned(),
        })
    }

    /// Create an Anthropic chat provider with a custom base URL (for testing).
    pub fn with_base_url(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> MemoryResult<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(MemoryError::invalid("Anthropic API key must not be empty"));
        }
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|err| MemoryError::provider(err.to_string()))?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key,
            model: model.into(),
            api_version: DEFAULT_API_VERSION.to_owned(),
        })
    }

    /// Return the model name.
    pub fn model_as_str(&self) -> &str {
        &self.model
    }

    /// Return the base URL.
    pub fn base_url_as_str(&self) -> &str {
        &self.base_url
    }
}

#[async_trait]
impl LlmProvider for AnthropicChat {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        options: &CompletionOptions,
    ) -> MemoryResult<String> {
        let mut system = String::new();
        let mut anthropic_messages: Vec<AnthropicMessage> = Vec::new();

        for msg in messages {
            if msg.role == "system" {
                // Anthropic system prompts are a separate field, not a message.
                system = msg.content.clone();
            } else {
                anthropic_messages.push(AnthropicMessage {
                    role: msg.role.clone(),
                    content: msg.content.clone(),
                });
            }
        }

        let response_format = if options.json_object {
            Some(AnthropicResponseFormat {
                format_type: "json_object".to_owned(),
            })
        } else {
            None
        };

        let body = AnthropicRequest {
            model: self.model.clone(),
            messages: anthropic_messages,
            system,
            temperature: options.temperature,
            response_format,
            max_tokens: Some(4096),
        };

        let response = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.api_version)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|err| MemoryError::provider(format!("Anthropic request failed: {err}")))?;

        let status = response.status();
        if !status.is_success() {
            let body_text = response
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_owned());
            return Err(MemoryError::provider(format!(
                "Anthropic provider returned HTTP {status}: {body_text}"
            )));
        }

        let body: AnthropicResponse = response
            .json()
            .await
            .map_err(|err| MemoryError::provider(format!("invalid Anthropic response: {err}")))?;

        body.content
            .into_iter()
            .find(|block| block.block_type == "text")
            .and_then(|block| block.text)
            .ok_or_else(|| MemoryError::provider("Anthropic response has no text content"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn defaults_are_anthropic_appropriate() {
        assert_eq!(DEFAULT_BASE_URL, "https://api.anthropic.com");
        assert_eq!(DEFAULT_MODEL, "claude-sonnet-4-20250514");
        assert_eq!(DEFAULT_API_VERSION, "2023-06-01");
    }

    #[test]
    fn empty_api_key_is_rejected() {
        let result = AnthropicChat::new("", "claude-sonnet-4-20250514");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn sends_system_prompt_separately() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("anthropic-version", "2023-06-01"))
            .and(body_partial_json(json!({
                "model": "claude-sonnet-4-20250514",
                "system": "You are helpful.",
                "max_tokens": 4096
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": "Hello!"}]
            })))
            .mount(&server)
            .await;

        let chat =
            AnthropicChat::with_base_url(server.uri(), "test-key", "claude-sonnet-4-20250514")
                .unwrap();

        let reply = chat
            .complete(
                &[
                    ChatMessage::system("You are helpful."),
                    ChatMessage::user("Hi"),
                ],
                &CompletionOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(reply, "Hello!");
    }

    #[tokio::test]
    async fn json_object_mode_sets_response_format() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({
                "response_format": {"type": "json_object"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": "{\"key\": \"value\"}"}]
            })))
            .mount(&server)
            .await;

        let chat = AnthropicChat::with_base_url(server.uri(), "key", "m").unwrap();
        let options = CompletionOptions {
            json_object: true,
            ..CompletionOptions::default()
        };
        let reply = chat
            .complete(&[ChatMessage::user("extract")], &options)
            .await
            .unwrap();
        assert_eq!(reply, "{\"key\": \"value\"}");
    }

    #[tokio::test]
    async fn http_errors_are_provider_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": {"message": "Invalid API key"}
            })))
            .mount(&server)
            .await;

        let chat = AnthropicChat::with_base_url(server.uri(), "bad-key", "m").unwrap();
        let result = chat
            .complete(&[ChatMessage::user("hi")], &CompletionOptions::default())
            .await;
        assert!(matches!(result, Err(MemoryError::Provider(_))));
    }

    #[tokio::test]
    async fn no_text_content_returns_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "tool_use", "id": "tool_1"}]
            })))
            .mount(&server)
            .await;

        let chat = AnthropicChat::with_base_url(server.uri(), "key", "m").unwrap();
        let result = chat
            .complete(&[ChatMessage::user("hi")], &CompletionOptions::default())
            .await;
        assert!(matches!(result, Err(MemoryError::Provider(_))));
    }

    #[tokio::test]
    async fn temperature_is_passed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({
                "temperature": 0.8
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": "ok"}]
            })))
            .mount(&server)
            .await;

        let chat = AnthropicChat::with_base_url(server.uri(), "key", "m").unwrap();
        let options = CompletionOptions {
            temperature: 0.8,
            ..CompletionOptions::default()
        };
        let reply = chat
            .complete(&[ChatMessage::user("hi")], &options)
            .await
            .unwrap();
        assert_eq!(reply, "ok");
    }

    #[test]
    fn new_accepts_a_key_and_uses_the_default_endpoint() {
        let chat = AnthropicChat::new("secret-key", "claude-sonnet-4")
            .expect("a non-empty API key should be accepted");
        assert_eq!(chat.model_as_str(), "claude-sonnet-4");
        assert_eq!(chat.base_url_as_str(), DEFAULT_BASE_URL);
    }

    #[test]
    fn debug_output_excludes_the_api_key() {
        let chat = AnthropicChat::new("secret-key", "claude-test-model")
            .expect("a non-empty API key is accepted");
        let debug = format!("{chat:?}");
        assert!(debug.contains("https://api.anthropic.com"));
        assert!(debug.contains("claude-test-model"));
        assert!(!debug.contains("secret-key"));
    }

    #[test]
    fn with_base_url_rejects_an_empty_api_key() {
        let result = AnthropicChat::with_base_url("https://example.invalid", "   ", "m");
        assert!(matches!(
            result,
            Err(MemoryError::InvalidInput(message))
                if message == "Anthropic API key must not be empty"
        ));
    }

    #[test]
    fn with_base_url_trims_trailing_slashes() {
        let chat = AnthropicChat::with_base_url("https://example.invalid/", "key", "m")
            .expect("a non-empty API key should be accepted");
        assert_eq!(chat.base_url_as_str(), "https://example.invalid");
    }

    #[tokio::test]
    async fn request_failures_are_provider_errors() {
        let chat = AnthropicChat::with_base_url("not a url", "key", "m")
            .expect("the endpoint URL is not parsed during construction");
        let result = chat
            .complete(&[ChatMessage::user("hi")], &CompletionOptions::default())
            .await;
        assert!(matches!(
            result,
            Err(MemoryError::Provider(message)) if message.contains("Anthropic request failed")
        ));
    }

    /// Serve a response that advertises more bytes than it sends so the client
    /// fails while reading the error body.
    async fn serve_truncated_body(listener: tokio::net::TcpListener) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut socket, _) = listener
            .accept()
            .await
            .expect("the test listener accepts the request");
        let mut chunk = [0u8; 4096];
        let _ = socket.read(&mut chunk).await;
        let headers = "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 100\r\nconnection: close\r\n\r\n";
        let _ = socket.write_all(headers.as_bytes()).await;
        let _ = socket.flush().await;
        // Send the headers first so the client observes the status code and only
        // fails once it reads the truncated body.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let _ = socket.write_all(b"short").await;
        let _ = socket.flush().await;
        let _ = socket.shutdown().await;
    }

    #[tokio::test]
    async fn unreadable_error_body_uses_the_no_body_fallback() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the test listener binds");
        let address = listener
            .local_addr()
            .expect("the listener exposes its address");

        let chat = AnthropicChat::with_base_url(format!("http://{address}"), "key", "m").unwrap();
        let messages = [ChatMessage::user("hi")];
        let options = CompletionOptions::default();
        let ((), result) = tokio::join!(
            serve_truncated_body(listener),
            chat.complete(&messages, &options)
        );
        let actual = format!("{result:?}");
        assert!(
            matches!(
                result,
                Err(MemoryError::Provider(message))
                    if message.contains("HTTP 500") && message.contains("<no body>")
            ),
            "actual: {actual}"
        );
    }

    #[tokio::test]
    async fn malformed_response_is_a_provider_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let chat = AnthropicChat::with_base_url(server.uri(), "key", "m").unwrap();
        let result = chat
            .complete(&[ChatMessage::user("hi")], &CompletionOptions::default())
            .await;
        assert!(matches!(
            result,
            Err(MemoryError::Provider(message)) if message.contains("invalid Anthropic response")
        ));
    }

    #[tokio::test]
    async fn multiple_text_blocks_returns_first() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [
                    {"type": "text", "text": "first"},
                    {"type": "text", "text": "second"}
                ]
            })))
            .mount(&server)
            .await;

        let chat = AnthropicChat::with_base_url(server.uri(), "key", "m").unwrap();
        let reply = chat
            .complete(&[ChatMessage::user("hi")], &CompletionOptions::default())
            .await
            .unwrap();
        assert_eq!(reply, "first");
    }
}
