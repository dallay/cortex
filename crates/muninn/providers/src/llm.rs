//! OpenAI-compatible chat-completion provider.

use async_trait::async_trait;
use muninn_core::extraction::{ChatMessage, CompletionOptions, LlmProvider};
use muninn_core::{MemoryError, MemoryResult};
use serde::Deserialize;
use serde_json::json;

/// `/chat/completions` client for OpenAI (upstream default `gpt-4o-mini`),
/// Rook, Ollama and other compatible gateways.
#[derive(Clone)]
pub struct OpenAiChat {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
}

impl std::fmt::Debug for OpenAiChat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiChat")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
}

impl OpenAiChat {
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
    ) -> MemoryResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|err| MemoryError::provider(err.to_string()))?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key,
            model: model.into(),
        })
    }
}

#[async_trait]
impl LlmProvider for OpenAiChat {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        options: &CompletionOptions,
    ) -> MemoryResult<String> {
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": options.temperature,
        });
        if options.json_object {
            body["response_format"] = json!({"type": "json_object"});
        }
        let mut request = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .await
            .map_err(|err| MemoryError::provider(format!("LLM request failed: {err}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(MemoryError::provider(format!(
                "LLM provider returned HTTP {status}"
            )));
        }
        let body: ChatResponse = response
            .json()
            .await
            .map_err(|err| MemoryError::provider(format!("invalid LLM response: {err}")))?;
        body.choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message.content)
            .ok_or_else(|| MemoryError::provider("LLM response has no content"))
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn requests_json_object_and_returns_first_choice() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(json!({
                "model": "m",
                "response_format": {"type": "json_object"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"content": "{\"ok\": true}"}}]
            })))
            .mount(&server)
            .await;
        let chat = OpenAiChat::new(server.uri(), None, "m").unwrap();
        let options = CompletionOptions {
            json_object: true,
            ..CompletionOptions::default()
        };
        let reply = chat
            .complete(&[ChatMessage::user("hi")], &options)
            .await
            .unwrap();
        assert_eq!(reply, "{\"ok\": true}");
    }

    #[tokio::test]
    async fn http_errors_are_provider_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let chat = OpenAiChat::new(server.uri(), None, "m").unwrap();
        let result = chat
            .complete(&[ChatMessage::user("hi")], &CompletionOptions::default())
            .await;
        assert!(matches!(result, Err(MemoryError::Provider(_))));
    }
}
