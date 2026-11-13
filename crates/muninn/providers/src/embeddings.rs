//! Embedding providers.

use async_trait::async_trait;
use muninn_core::ports::{Embedder, EmbeddingIdentity};
use muninn_core::{MemoryError, MemoryResult};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

/// Deterministic embedder equivalent to the TCK `MockEmbedder`: the SHA-256
/// hex digest of the text, read as bytes scaled to `[0, 1]` and zero padded.
///
/// Used for conformance runs and offline tests; it carries no semantics.
#[derive(Debug, Clone)]
pub struct HashEmbedder {
    dimensions: usize,
}

impl HashEmbedder {
    pub const fn new(dimensions: usize) -> Self {
        Self { dimensions }
    }

    pub fn vector(&self, text: &str) -> Vec<f64> {
        let digest = Sha256::digest(text.as_bytes());
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut vector: Vec<f64> = hex
            .as_bytes()
            .chunks(2)
            .take(self.dimensions)
            .map(|pair| {
                let text = std::str::from_utf8(pair).unwrap_or("00");
                f64::from(u8::from_str_radix(text, 16).unwrap_or(0)) / 255.0
            })
            .collect();
        vector.resize(self.dimensions, 0.0);
        vector
    }
}

impl Default for HashEmbedder {
    fn default() -> Self {
        Self::new(1536)
    }
}

#[async_trait]
impl Embedder for HashEmbedder {
    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn identity(&self) -> MemoryResult<EmbeddingIdentity> {
        EmbeddingIdentity::new("muninn:tck:sha256-bytes:v1", self.dimensions)
    }

    async fn embed(&self, text: &str) -> MemoryResult<Vec<f64>> {
        Ok(self.vector(text))
    }
}

/// OpenAI-compatible `/embeddings` client. Works with OpenAI (upstream
/// default `text-embedding-3-small`), Rook, Ollama and other compatible
/// gateways.
#[derive(Clone)]
pub struct OpenAiEmbedder {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    dimensions: usize,
    identity: Option<EmbeddingIdentity>,
}

impl std::fmt::Debug for OpenAiEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiEmbedder")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Deserialize)]
struct EmbeddingItem {
    index: usize,
    embedding: Vec<f64>,
}

/// Maximum inputs per request, matching OpenAI's batch limit.
const MAX_BATCH: usize = 2048;

impl OpenAiEmbedder {
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
        dimensions: usize,
    ) -> MemoryResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|err| MemoryError::provider(err.to_string()))?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key,
            identity: None,
            model: model.into(),
            dimensions,
        })
    }

    /// Configure a pinned deployment/revision and preprocessing contract. Mutable
    /// remote model aliases alone do not establish a stable vector space.
    pub fn with_identity(mut self, identity: EmbeddingIdentity) -> MemoryResult<Self> {
        if identity.dimensions() != self.dimensions {
            return Err(MemoryError::invalid(
                "embedding identity dimensions mismatch",
            ));
        }
        self.identity = Some(identity);
        Ok(self)
    }

    async fn request(&self, inputs: &[String]) -> MemoryResult<Vec<Vec<f64>>> {
        let mut request = self
            .http
            .post(format!("{}/embeddings", self.base_url))
            .json(&json!({"model": self.model, "input": inputs}));
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .await
            .map_err(|err| MemoryError::provider(format!("embedding request failed: {err}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(MemoryError::provider(format!(
                "embedding provider returned HTTP {status}"
            )));
        }
        let mut body: EmbeddingResponse = response
            .json()
            .await
            .map_err(|err| MemoryError::provider(format!("invalid embedding response: {err}")))?;
        if body.data.len() != inputs.len() {
            return Err(MemoryError::provider(format!(
                "embedding provider returned {} vectors for {} inputs",
                body.data.len(),
                inputs.len()
            )));
        }
        body.data.sort_by_key(|item| item.index);
        body.data
            .into_iter()
            .map(|item| {
                if item.embedding.len() == self.dimensions {
                    Ok(item.embedding)
                } else {
                    Err(MemoryError::DimensionMismatch(format!(
                        "model '{}' returned {} dimensions, configured {}",
                        self.model,
                        item.embedding.len(),
                        self.dimensions
                    )))
                }
            })
            .collect()
    }
}

#[async_trait]
impl Embedder for OpenAiEmbedder {
    fn identity(&self) -> MemoryResult<EmbeddingIdentity> {
        self.identity
            .clone()
            .ok_or_else(|| MemoryError::invalid("remote embedding identity is required"))
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn embed(&self, text: &str) -> MemoryResult<Vec<f64>> {
        let mut vectors = self.request(&[text.to_owned()]).await?;
        vectors
            .pop()
            .ok_or_else(|| MemoryError::provider("empty embedding response"))
    }

    async fn embed_batch(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f64>>> {
        let mut vectors = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(MAX_BATCH) {
            vectors.extend(self.request(chunk).await?);
        }
        Ok(vectors)
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn hash_embedder_matches_tck_mock_embedder() {
        // Python: hashlib.sha256(b"hello").hexdigest()[:4] == "2cf2"
        let vector = HashEmbedder::default().embed("hello").await.unwrap();
        assert_eq!(vector.len(), 1536);
        assert!((vector[0] - f64::from(0x2c_u8) / 255.0).abs() < f64::EPSILON);
        assert!((vector[1] - f64::from(0xf2_u8) / 255.0).abs() < f64::EPSILON);
        assert_eq!(vector[32], 0.0);
    }

    #[tokio::test]
    async fn openai_embedder_orders_by_index_and_sends_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("authorization", "Bearer secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    {"index": 1, "embedding": [0.0, 1.0]},
                    {"index": 0, "embedding": [1.0, 0.0]}
                ]
            })))
            .mount(&server)
            .await;
        let embedder = OpenAiEmbedder::new(
            format!("{}/v1", server.uri()),
            Some("secret".into()),
            "m",
            2,
        )
        .unwrap();
        let vectors = embedder
            .embed_batch(&["a".to_owned(), "b".to_owned()])
            .await
            .unwrap();
        assert_eq!(vectors, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
    }

    #[tokio::test]
    async fn openai_embedder_rejects_wrong_dimensions() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"index": 0, "embedding": [1.0]}]
            })))
            .mount(&server)
            .await;
        let embedder = OpenAiEmbedder::new(server.uri(), None, "m", 2).unwrap();
        assert!(matches!(
            embedder.embed("a").await,
            Err(MemoryError::DimensionMismatch(_))
        ));
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn hash_space_is_explicit_and_distinct_from_semantic_models() {
        let identity = HashEmbedder::new(384).identity().unwrap();
        assert_eq!(identity.key(), "muninn:tck:sha256-bytes:v1");
        assert_eq!(identity.dimensions(), 384);
    }

    #[test]
    fn remote_models_require_an_explicit_revision_identity() {
        let embedder =
            OpenAiEmbedder::new("https://example.invalid", None, "mutable-alias", 384).unwrap();
        assert!(embedder.identity().is_err());
    }
}
