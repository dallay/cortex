// request.rs — HTTP request building utilities for providers
//
// This module provides a template/builder pattern for sending stream requests
// with common headers across different providers.

use serde::Serialize;

/// Common headers used across provider requests.
#[derive(Debug, Clone)]
pub struct CommonHeaders {
    /// The Authorization header value (e.g., "Bearer sk-...")
    pub authorization: Option<String>,
    /// Content type (default: "application/json")
    pub content_type: Option<String>,
    /// Provider-specific headers (e.g., "anthropic-version")
    pub extra: Vec<(&'static str, String)>,
}

impl Default for CommonHeaders {
    fn default() -> Self {
        Self {
            authorization: None,
            content_type: Some("application/json".to_string()),
            extra: Vec::new(),
        }
    }
}

impl CommonHeaders {
    /// Create a new empty headers builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the Authorization header.
    /// Takes the full Authorization value (e.g., "Bearer sk-xxx" or "ApiKey xxx")
    pub fn with_authorization(mut self, auth_value: &str) -> Self {
        self.authorization = Some(auth_value.to_string());
        self
    }

    /// Set the Authorization header using an API key directly (no Bearer prefix).
    pub fn with_api_key(mut self, api_key: &str) -> Self {
        self.authorization = Some(api_key.to_string());
        self
    }

    /// Set a custom content type.
    pub fn with_content_type(mut self, content_type: &str) -> Self {
        self.content_type = Some(content_type.to_string());
        self
    }

    /// Add an extra header.
    pub fn with_extra(mut self, name: &'static str, value: &str) -> Self {
        self.extra.push((name, value.to_string()));
        self
    }

    /// Build the headers as a Vec of (name, value) tuples.
    /// This is suitable for use with reqwest or other HTTP clients.
    pub fn to_vec(&self) -> Vec<(&'static str, String)> {
        let mut headers = Vec::new();
        if let Some(ref auth) = self.authorization {
            headers.push(("authorization", auth.clone()));
        }
        if let Some(ref ct) = self.content_type {
            headers.push(("content-type", ct.clone()));
        }
        for (name, value) in &self.extra {
            headers.push((*name, value.clone()));
        }
        headers
    }
}

/// A builder for constructing provider request bodies.
///
/// This is a template that can be specialized for different providers
/// (OpenAI, Anthropic, Groq, etc.) by setting the appropriate fields.
#[derive(Debug, Clone)]
pub struct RequestTemplate {
    /// The model identifier
    pub model: String,
    /// Whether to stream the response
    pub stream: bool,
    /// Maximum tokens to generate
    pub max_tokens: Option<u32>,
    /// Sampling temperature
    pub temperature: Option<f32>,
    /// System prompt (for providers that separate it)
    pub system: Option<String>,
}

impl RequestTemplate {
    /// Create a new request template for a model.
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            stream: true,
            max_tokens: None,
            temperature: None,
            system: None,
        }
    }

    /// Set the stream flag.
    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }

    /// Set max_tokens.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Set temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set the system prompt.
    pub fn with_system(mut self, system: &str) -> Self {
        self.system = Some(system.to_string());
        self
    }
}

/// Serialize a request body to JSON bytes, suitable for sending in an HTTP request.
///
/// Returns `None` if serialization fails.
pub fn serialize_body<T: Serialize>(body: &T) -> Option<Vec<u8>> {
    serde_json::to_vec(body).ok()
}

/// Serialize a request body to a string, suitable for logging.
///
/// Returns "[serialization failed]" if serialization fails.
pub fn serialize_body_for_log<T: Serialize>(body: &T) -> String {
    serde_json::to_string(body).unwrap_or_else(|_| "[serialization failed]".to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_common_headers_default() {
        let headers = CommonHeaders::default();
        assert!(headers.authorization.is_none());
        assert_eq!(headers.content_type, Some("application/json".to_string()));
        assert!(headers.extra.is_empty());
    }

    #[test]
    fn test_common_headers_with_authorization() {
        // Pass the full Authorization value (providers construct "Bearer <key>" themselves)
        let headers = CommonHeaders::new().with_authorization("Bearer sk-test");
        assert_eq!(headers.authorization, Some("Bearer sk-test".to_string()));
    }

    #[test]
    fn test_common_headers_with_api_key() {
        let headers = CommonHeaders::new().with_api_key("x-api-key-value");
        assert_eq!(headers.authorization, Some("x-api-key-value".to_string()));
    }

    #[test]
    fn test_common_headers_with_extra() {
        let headers = CommonHeaders::new()
            .with_extra("anthropic-version", "2023-06-01")
            .with_extra("x-custom", "value");
        assert_eq!(headers.extra.len(), 2);
        assert_eq!(headers.extra[0].0, "anthropic-version");
        assert_eq!(headers.extra[1].1, "value");
    }

    #[test]
    fn test_common_headers_to_vec() {
        let headers = CommonHeaders::new()
            .with_authorization("Bearer sk-test")
            .with_extra("anthropic-version", "2023-06-01");

        let vec = headers.to_vec();
        // Check that expected headers are present
        assert!(vec
            .iter()
            .any(|(k, v)| *k == "authorization" && v == "Bearer sk-test"));
        assert!(vec
            .iter()
            .any(|(k, v)| *k == "content-type" && v == "application/json"));
        assert!(vec
            .iter()
            .any(|(k, v)| *k == "anthropic-version" && v == "2023-06-01"));
    }

    #[test]
    fn test_request_template_new() {
        let tmpl = RequestTemplate::new("gpt-4");
        assert_eq!(tmpl.model, "gpt-4");
        assert!(tmpl.stream);
        assert!(tmpl.max_tokens.is_none());
        assert!(tmpl.temperature.is_none());
        assert!(tmpl.system.is_none());
    }

    #[test]
    fn test_request_template_with_options() {
        let tmpl = RequestTemplate::new("gpt-4")
            .with_stream(false)
            .with_max_tokens(1000)
            .with_temperature(0.7)
            .with_system("You are helpful.");

        assert_eq!(tmpl.model, "gpt-4");
        assert!(!tmpl.stream);
        assert_eq!(tmpl.max_tokens, Some(1000));
        assert_eq!(tmpl.temperature, Some(0.7));
        assert_eq!(tmpl.system, Some("You are helpful.".to_string()));
    }

    #[test]
    fn test_serialize_body() {
        #[derive(Serialize)]
        struct TestBody {
            key: String,
            value: i32,
        }
        let body = TestBody {
            key: "hello".to_string(),
            value: 42,
        };
        let bytes = serialize_body(&body);
        assert!(bytes.is_some());
        let json = String::from_utf8(bytes.unwrap()).unwrap();
        assert!(json.contains("\"key\""));
        assert!(json.contains("\"hello\""));
    }

    #[test]
    fn test_serialize_body_for_log() {
        #[derive(Serialize)]
        struct TestBody {
            secret: String,
        }
        let body = TestBody {
            secret: "sk-12345".to_string(),
        };
        let log = serialize_body_for_log(&body);
        assert!(log.contains("sk-12345")); // Content is preserved
    }

    #[test]
    fn test_request_template_clone() {
        let tmpl = RequestTemplate::new("gpt-4").with_max_tokens(100);
        let tmpl2 = tmpl.clone();
        assert_eq!(tmpl.model, tmpl2.model);
        assert_eq!(tmpl.max_tokens, tmpl2.max_tokens);
    }
}
