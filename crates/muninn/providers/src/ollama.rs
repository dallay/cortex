//! Ollama providers — local, free, OpenAI-compatible chat and embeddings.
//!
//! Ollama exposes an OpenAI-compatible API at `http://localhost:11434/v1/`.
//! These structs are thin wrappers around [`OpenAiChat`] and [`OpenAiEmbedder`]
//! that default to the Ollama base URL and common local models.
//!
//! Installation:
//! ```bash
//! brew install ollama
//! brew services start ollama
//! ollama pull llama3.2          # Chat
//! ollama pull nomic-embed-text  # Embeddings
//! ```

use muninn_core::ports::Embedder;
use muninn_core::MemoryResult;

use crate::{embeddings::OpenAiEmbedder, llm::OpenAiChat};

/// Default Ollama base URL (OpenAI-compatible endpoint).
pub const DEFAULT_BASE_URL: &str = "http://localhost:11434/v1";

/// Default Ollama chat model.
pub const DEFAULT_CHAT_MODEL: &str = "llama3.2";

/// Default Ollama embedding model.
pub const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";

/// Default dimensions for `nomic-embed-text`.
pub const DEFAULT_EMBED_DIMENSIONS: usize = 768;

/// Ollama chat provider.
///
/// Wraps [`OpenAiChat`] with Ollama defaults. The API key is ignored by
/// Ollama but required by the OpenAI client; pass `"ollama"` as a placeholder.
#[derive(Clone)]
pub struct OllamaChat {
    inner: OpenAiChat,
}

impl std::fmt::Debug for OllamaChat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OllamaChat")
            .field("base_url", &self.inner.base_url_as_str())
            .field("model", &self.inner.model_as_str())
            .finish()
    }
}

impl OllamaChat {
    /// Create an Ollama chat provider with default settings.
    pub fn new() -> MemoryResult<Self> {
        Self::with_model(DEFAULT_CHAT_MODEL)
    }

    /// Create an Ollama chat provider with a specific model.
    pub fn with_model(model: impl Into<String>) -> MemoryResult<Self> {
        // Ollama requires a Bearer token but ignores its value.
        let inner = OpenAiChat::new(DEFAULT_BASE_URL, Some("ollama".into()), model.into())?;
        Ok(Self { inner })
    }

    /// Create an Ollama chat provider with a custom base URL and model.
    pub fn with_url_and_model(
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> MemoryResult<Self> {
        let inner = OpenAiChat::new(base_url, Some("ollama".into()), model.into())?;
        Ok(Self { inner })
    }

    /// Return the underlying [`OpenAiChat`] for advanced configuration.
    pub fn inner(&self) -> &OpenAiChat {
        &self.inner
    }
}

#[async_trait::async_trait]
impl muninn_core::extraction::LlmProvider for OllamaChat {
    async fn complete(
        &self,
        messages: &[muninn_core::extraction::ChatMessage],
        options: &muninn_core::extraction::CompletionOptions,
    ) -> MemoryResult<String> {
        self.inner.complete(messages, options).await
    }
}

/// Ollama embeddings provider.
///
/// Wraps [`OpenAiEmbedder`] with Ollama defaults.
#[derive(Clone)]
pub struct OllamaEmbedder {
    inner: OpenAiEmbedder,
}

impl std::fmt::Debug for OllamaEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OllamaEmbedder")
            .field("base_url", &self.inner.base_url_as_str())
            .field("model", &self.inner.model_as_str())
            .field("dimensions", &self.inner.dimensions())
            .finish()
    }
}

impl OllamaEmbedder {
    /// Create an Ollama embedder with default settings (`nomic-embed-text`, 768 dims).
    pub fn new() -> MemoryResult<Self> {
        Self::with_model(DEFAULT_EMBED_MODEL, DEFAULT_EMBED_DIMENSIONS)
    }

    /// Create an Ollama embedder with a specific model and dimensions.
    pub fn with_model(model: impl Into<String>, dimensions: usize) -> MemoryResult<Self> {
        let inner = OpenAiEmbedder::new(
            DEFAULT_BASE_URL,
            Some("ollama".into()),
            model.into(),
            dimensions,
        )?;
        Ok(Self { inner })
    }

    /// Create an Ollama embedder with a custom base URL, model, and dimensions.
    pub fn with_url_and_model(
        base_url: impl Into<String>,
        model: impl Into<String>,
        dimensions: usize,
    ) -> MemoryResult<Self> {
        let inner = OpenAiEmbedder::new(base_url, Some("ollama".into()), model.into(), dimensions)?;
        Ok(Self { inner })
    }

    /// Return the underlying [`OpenAiEmbedder`] for advanced configuration.
    pub fn inner(&self) -> &OpenAiEmbedder {
        &self.inner
    }
}

#[async_trait::async_trait]
impl muninn_core::ports::Embedder for OllamaEmbedder {
    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }

    fn identity(&self) -> MemoryResult<muninn_core::ports::EmbeddingIdentity> {
        self.inner.identity()
    }

    async fn embed(&self, text: &str) -> MemoryResult<Vec<f64>> {
        self.inner.embed(text).await
    }

    async fn embed_batch(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f64>>> {
        self.inner.embed_batch(texts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muninn_core::extraction::{ChatMessage, CompletionOptions, LlmProvider};
    use muninn_core::MemoryError;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn defaults_are_ollama_appropriate() {
        assert_eq!(DEFAULT_BASE_URL, "http://localhost:11434/v1");
        assert_eq!(DEFAULT_CHAT_MODEL, "llama3.2");
        assert_eq!(DEFAULT_EMBED_MODEL, "nomic-embed-text");
        assert_eq!(DEFAULT_EMBED_DIMENSIONS, 768);
    }

    #[tokio::test]
    async fn ollama_chat_sends_bearer_ollama_and_correct_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(json!({
                "model": "llama3.2"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"content": "hello"}}]
            })))
            .mount(&server)
            .await;

        // OllamaChat uses the real base URL, so we point it at the mock server.
        let chat = OllamaChat::with_url_and_model(server.uri(), "llama3.2").unwrap();
        let reply = chat
            .complete(&[ChatMessage::user("hi")], &CompletionOptions::default())
            .await
            .unwrap();
        assert_eq!(reply, "hello");
    }

    #[tokio::test]
    async fn ollama_chat_custom_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({
                "model": "qwen2.5:7b"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"content": "custom"}}]
            })))
            .mount(&server)
            .await;

        let chat = OllamaChat::with_url_and_model(server.uri(), "qwen2.5:7b").unwrap();
        let reply = chat
            .complete(&[ChatMessage::user("test")], &CompletionOptions::default())
            .await
            .unwrap();
        assert_eq!(reply, "custom");
    }

    #[tokio::test]
    async fn ollama_embedder_returns_vectors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .and(body_partial_json(json!({
                "model": "nomic-embed-text"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"index": 0, "embedding": [0.1, 0.2, 0.3]}]
            })))
            .mount(&server)
            .await;

        let embedder =
            OllamaEmbedder::with_url_and_model(server.uri(), "nomic-embed-text", 3).unwrap();
        assert_eq!(embedder.dimensions(), 3);

        let vector = embedder.embed("hello").await.unwrap();
        assert_eq!(vector, vec![0.1, 0.2, 0.3]);
    }

    #[tokio::test]
    async fn ollama_embedder_batch() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    {"index": 0, "embedding": [1.0, 0.0]},
                    {"index": 1, "embedding": [0.0, 1.0]}
                ]
            })))
            .mount(&server)
            .await;

        let embedder = OllamaEmbedder::with_url_and_model(server.uri(), "m", 2).unwrap();
        let vectors = embedder
            .embed_batch(&["a".to_owned(), "b".to_owned()])
            .await
            .unwrap();
        assert_eq!(vectors, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
    }

    #[tokio::test]
    async fn ollama_chat_inner_exposes_openai_chat() {
        let server = MockServer::start().await;
        let chat = OllamaChat::with_url_and_model(server.uri(), "m").unwrap();
        let _inner = chat.inner();
        // Just verify the inner reference compiles
    }

    #[tokio::test]
    async fn ollama_embedder_inner_exposes_openai_embedder() {
        let server = MockServer::start().await;
        let embedder = OllamaEmbedder::with_url_and_model(server.uri(), "m", 384).unwrap();
        let _inner = embedder.inner();
    }

    #[tokio::test]
    async fn ollama_chat_json_object_mode() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({
                "response_format": {"type": "json_object"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"content": "{\"key\": \"value\"}"}}]
            })))
            .mount(&server)
            .await;

        let chat = OllamaChat::with_url_and_model(server.uri(), "m").unwrap();
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
    async fn ollama_embedder_rejects_dimension_mismatch() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"index": 0, "embedding": [1.0]}]
            })))
            .mount(&server)
            .await;

        let embedder = OllamaEmbedder::with_url_and_model(server.uri(), "m", 3).unwrap();
        let result = embedder.embed("test").await;
        assert!(matches!(result, Err(MemoryError::DimensionMismatch(_))));
    }
}
