//! HTTP boundary for the local spaCy stage.
use async_trait::async_trait;
use muninn_core::extraction::{EntityExtractor, ExtractOptions, ExtractionResult};
use muninn_core::{MemoryError, MemoryResult};
use std::time::Duration;

pub struct SpacyEntityExtractor {
    client: reqwest::Client,
    endpoint: reqwest::Url,
}
impl SpacyEntityExtractor {
    /// Endpoint must be a trusted local sidecar. Redirects and URL credentials
    /// are disallowed; errors never include URLs, response bodies or input text.
    pub fn new(endpoint: &str, timeout: Duration) -> MemoryResult<Self> {
        let endpoint = reqwest::Url::parse(endpoint)
            .map_err(|_| MemoryError::provider("spaCy extraction invalid endpoint"))?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || timeout.is_zero()
        {
            return Err(MemoryError::provider(
                "spaCy extraction invalid configuration",
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| MemoryError::provider("spaCy extraction client initialization failed"))?;
        Ok(Self { client, endpoint })
    }
}
#[async_trait]
impl EntityExtractor for SpacyEntityExtractor {
    async fn extract(
        &self,
        text: &str,
        options: &ExtractOptions,
    ) -> MemoryResult<ExtractionResult> {
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .json(&serde_json::json!({"text":text, "entity_types":options.entity_types}))
            .send()
            .await
            .map_err(|_| MemoryError::provider("spaCy extraction transport failed"))?;
        if !response.status().is_success() {
            return Err(MemoryError::provider("spaCy extraction service failed"));
        }
        const MAX_RESPONSE: usize = 1_048_576;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| MemoryError::provider("spaCy extraction transport failed"))?
        {
            if chunk.len() > MAX_RESPONSE.saturating_sub(bytes.len()) {
                return Err(MemoryError::provider("spaCy extraction response too large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut result: ExtractionResult = serde_json::from_slice(&bytes)
            .map_err(|_| MemoryError::provider("spaCy extraction invalid response"))?;
        for entity in &result.entities {
            entity
                .validate(text)
                .map_err(|_| MemoryError::provider("spaCy extraction invalid entity"))?;
            if entity.extractor.as_deref() != Some("spacy") {
                return Err(MemoryError::provider("spaCy extraction invalid entity"));
            }
        }
        result.entities.retain(|entity| {
            entity.confidence >= 0.85
                && options
                    .entity_types
                    .as_ref()
                    .is_none_or(|types| types.is_empty() || types.contains(&entity.entity_type))
        });
        // spaCy cannot supply relationships, preferences or private reasoning.
        result.relations.clear();
        result.preferences.clear();
        result.source_text = None;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn spacy_maps_spans_and_filters_inclusive_boundary() {
        let server = MockServer::start().await;
        let entity = |confidence| {
            json!({"name":"Alice", "type":"PERSON", "subtype":null,
            "start_pos":0, "end_pos":5, "confidence":confidence, "extractor":"spacy"})
        };
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(body_json(
                json!({"text":"Alice", "entity_types":["PERSON"]}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entities":[entity(0.849999),entity(0.85),entity(0.850001)],
                "relations":[],"preferences":[],"source_text":"Alice"})))
            .expect(1)
            .mount(&server)
            .await;
        let adapter =
            SpacyEntityExtractor::new(&format!("{}/extract", server.uri()), Duration::from_secs(1))
                .unwrap();
        let result = adapter
            .extract(
                "Alice",
                &ExtractOptions {
                    entity_types: Some(vec!["PERSON".into()]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(result.entities.len(), 2);
        let entity = &result.entities[0];
        assert_eq!(entity.name, "Alice");
        assert_eq!(entity.entity_type, "PERSON");
        assert_eq!(entity.subtype, None);
        assert_eq!((entity.start_pos, entity.end_pos), (Some(0), Some(5)));
        assert_eq!(entity.confidence, 0.85);
        assert_eq!(entity.extractor.as_deref(), Some("spacy"));
        assert!(result.relations.is_empty() && result.preferences.is_empty());
    }

    #[tokio::test]
    async fn spacy_rejects_forged_spans() {
        let server = MockServer::start().await;
        let forged = json!({"entities":[{"name":"Mallory","type":"PERSON","subtype":null,
            "start_pos":0,"end_pos":5,"confidence":0.85,"extractor":"spacy"}],
            "relations":[],"preferences":[],"source_text":null});
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(forged))
            .mount(&server)
            .await;
        let adapter =
            SpacyEntityExtractor::new(&format!("{}/extract", server.uri()), Duration::from_secs(2))
                .unwrap();
        let error = adapter
            .extract("Alice", &Default::default())
            .await
            .unwrap_err();
        assert!(matches!(error, MemoryError::Provider(_)));
        assert!(!error.to_string().contains("Mallory"));
    }

    #[tokio::test]
    async fn spacy_rejects_oversized_valid_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entities":[],"relations":[],"preferences":[],"source_text":"x".repeat(1_048_577)
            })))
            .mount(&server)
            .await;
        let adapter =
            SpacyEntityExtractor::new(&format!("{}/extract", server.uri()), Duration::from_secs(2))
                .unwrap();
        assert_eq!(
            adapter
                .extract("Alice", &Default::default())
                .await
                .unwrap_err()
                .to_string(),
            "provider error: spaCy extraction response too large"
        );
    }

    #[tokio::test]
    async fn spacy_rejects_malformed_types_and_spans_and_keeps_unicode_offsets() {
        let server = MockServer::start().await;
        let entity = json!({"name":"Éva","type":"PERSON","subtype":null,
            "start_pos":1,"end_pos":4,"confidence":0.85,"extractor":"spacy"});
        let adapter =
            SpacyEntityExtractor::new(&format!("{}/extract", server.uri()), Duration::from_secs(2))
                .unwrap();
        for (key, value) in [
            ("type", json!("INJECT")),
            ("type", json!(1)),
            ("start_pos", json!(-1)),
            ("end_pos", json!(99)),
            ("start_pos", json!(null)),
            ("confidence", json!("0.85")),
            ("extractor", json!("private")),
        ] {
            let mut invalid = entity.clone();
            invalid[key] = value;
            server.reset().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "entities":[entity,invalid],"relations":[],"preferences":[],"source_text":null
                })))
                .mount(&server)
                .await;
            let error = adapter
                .extract(" Éva ", &Default::default())
                .await
                .unwrap_err();
            assert!(matches!(error, MemoryError::Provider(_)));
            assert!(!error.to_string().contains("private"));
        }
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entities":[entity],"relations":[],"preferences":[],"source_text":" Éva "
            })))
            .mount(&server)
            .await;
        let result = adapter.extract(" Éva ", &Default::default()).await.unwrap();
        assert_eq!(
            (result.entities[0].start_pos, result.entities[0].end_pos),
            (Some(1), Some(4))
        );
        assert_eq!(result.source_text, None);
    }

    #[tokio::test]
    async fn spacy_rejects_redirect_with_a_valid_extraction_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(302).set_body_json(json!({
                "entities":[],"relations":[],"preferences":[],"source_text":null
            })))
            .mount(&server)
            .await;
        let adapter =
            SpacyEntityExtractor::new(&format!("{}/extract", server.uri()), Duration::from_secs(2))
                .unwrap();
        assert_eq!(
            adapter
                .extract("Alice", &Default::default())
                .await
                .unwrap_err()
                .to_string(),
            "provider error: spaCy extraction service failed"
        );
    }

    #[test]
    fn spacy_invalid_configuration_does_not_disclose_credentials() {
        for url in [
            "http://user:synthetic-secret@127.0.0.1/extract",
            "file:///invalid",
            "not-a-url",
        ] {
            let error = SpacyEntityExtractor::new(url, Duration::from_secs(1))
                .err()
                .unwrap();
            assert!(matches!(error, MemoryError::Provider(_)));
            assert!(!error.to_string().contains("synthetic-secret"));
            assert!(!error.to_string().contains(url));
        }
        assert!(SpacyEntityExtractor::new("http://127.0.0.1/extract", Duration::ZERO).is_err());
    }

    #[tokio::test]
    async fn spacy_errors_are_explicit_and_redacted() {
        let server = MockServer::start().await;
        for response in [
            ResponseTemplate::new(503).set_body_string("private payload"),
            ResponseTemplate::new(200).set_body_string("private malformed payload"),
            ResponseTemplate::new(200).set_delay(Duration::from_millis(100)),
        ] {
            server.reset().await;
            Mock::given(method("POST"))
                .respond_with(response)
                .mount(&server)
                .await;
            let adapter = SpacyEntityExtractor::new(
                &format!("{}/extract", server.uri()),
                Duration::from_millis(20),
            )
            .unwrap();
            let error = adapter
                .extract("secret message", &Default::default())
                .await
                .unwrap_err();
            assert!(matches!(error, MemoryError::Provider(_)));
            assert!(
                error
                    .to_string()
                    .starts_with("provider error: spaCy extraction "),
                "{error}"
            );
            for secret in ["private", "secret", &server.uri()] {
                assert!(!error.to_string().contains(secret));
            }
        }
        let address = server.uri();
        drop(server);
        let adapter =
            SpacyEntityExtractor::new(&format!("{address}/extract"), Duration::from_millis(20))
                .unwrap();
        assert!(matches!(
            adapter.extract("x", &Default::default()).await,
            Err(MemoryError::Provider(_))
        ));
    }
}
