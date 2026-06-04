// format_registry.rs — maps provider kind strings to `ApiFormat` variants and
// registered domain-pivot translators for explicit multi-format routing.

use std::collections::HashMap;
use std::sync::Arc;

use rook_core::{ApiFormat, CompletionRequest, CompletionResponse, FormatTranslatorPort};
use shared_kernel::{CortexError, CortexResult};

/// Translates a domain completion request before it is sent to a provider using
/// another API format.
pub trait RequestTranslator: Send + Sync + 'static {
    fn translate_request(&self, req: CompletionRequest) -> CortexResult<CompletionRequest>;
}

/// Translates a domain completion response from provider format back to the
/// client's requested API format.
pub trait ResponseTranslator: Send + Sync + 'static {
    fn translate_response(&self, resp: CompletionResponse) -> CortexResult<CompletionResponse>;
}

/// Convenience trait for translators that support both request and response
/// directions for a single `(from, to)` format pair.
pub trait Translator: RequestTranslator + ResponseTranslator {}

impl<T> Translator for T where T: RequestTranslator + ResponseTranslator {}

#[derive(Debug, Default)]
pub struct IdentityTranslator;

impl RequestTranslator for IdentityTranslator {
    fn translate_request(&self, req: CompletionRequest) -> CortexResult<CompletionRequest> {
        Ok(req)
    }
}

impl ResponseTranslator for IdentityTranslator {
    fn translate_response(&self, resp: CompletionResponse) -> CortexResult<CompletionResponse> {
        Ok(resp)
    }
}

/// Domain-pivot translator between API formats.
///
/// The request and response are already normalized into `CompletionRequest` and
/// `CompletionResponse`, so this translator intentionally does not perform
/// wire-to-wire serde transformations. It exists as an explicit, pluggable
/// routing contract for the transport/use-case boundary.
#[derive(Debug, Default)]
pub struct DomainPivotTranslator;

impl RequestTranslator for DomainPivotTranslator {
    fn translate_request(&self, req: CompletionRequest) -> CortexResult<CompletionRequest> {
        Ok(req)
    }
}

impl ResponseTranslator for DomainPivotTranslator {
    fn translate_response(&self, resp: CompletionResponse) -> CortexResult<CompletionResponse> {
        Ok(resp)
    }
}

/// Registry that resolves provider kind strings to an `ApiFormat` and stores
/// request/response translators keyed by `(client_format, provider_format)`.
///
/// Constructed once at startup and shared via `Arc`.
#[derive(Default, Clone)]
pub struct FormatRegistry {
    request_translators: HashMap<(ApiFormat, ApiFormat), Arc<dyn RequestTranslator>>,
    response_translators: HashMap<(ApiFormat, ApiFormat), Arc<dyn ResponseTranslator>>,
}

impl FormatRegistry {
    /// Create a new registry with same-format identity translators.
    pub fn new() -> Self {
        let mut registry = Self::default();
        registry.register(ApiFormat::OpenAI, ApiFormat::OpenAI, IdentityTranslator);
        registry.register(
            ApiFormat::Anthropic,
            ApiFormat::Anthropic,
            IdentityTranslator,
        );
        registry
    }

    /// Register request and response translators for one `(from, to)` pair.
    pub fn register<T>(&mut self, from: ApiFormat, to: ApiFormat, translator: T)
    where
        T: Translator,
    {
        let translator = Arc::new(translator);
        self.request_translators
            .insert((from, to), translator.clone() as Arc<dyn RequestTranslator>);
        self.response_translators
            .insert((from, to), translator as Arc<dyn ResponseTranslator>);
    }

    pub fn get_request_translator(
        &self,
        from: ApiFormat,
        to: ApiFormat,
    ) -> Option<Arc<dyn RequestTranslator>> {
        self.request_translators.get(&(from, to)).cloned()
    }

    pub fn get_response_translator(
        &self,
        from: ApiFormat,
        to: ApiFormat,
    ) -> Option<Arc<dyn ResponseTranslator>> {
        self.response_translators.get(&(from, to)).cloned()
    }

    pub fn translate_request(
        &self,
        from: ApiFormat,
        to: ApiFormat,
        req: CompletionRequest,
    ) -> CortexResult<CompletionRequest> {
        self.get_request_translator(from, to)
            .ok_or_else(|| missing_translator("request", from, to))?
            .translate_request(req)
    }

    pub fn translate_response(
        &self,
        from: ApiFormat,
        to: ApiFormat,
        resp: CompletionResponse,
    ) -> CortexResult<CompletionResponse> {
        self.get_response_translator(from, to)
            .ok_or_else(|| missing_translator("response", from, to))?
            .translate_response(resp)
    }

    /// Look up the `ApiFormat` for the given provider kind.
    ///
    /// Returns `None` for unknown kinds so callers can decide the fallback policy.
    pub fn format_for(&self, kind: &str) -> Option<ApiFormat> {
        match kind {
            "openai" | "ollama" | "gemini" | "groq" => Some(ApiFormat::OpenAI),
            "anthropic" => Some(ApiFormat::Anthropic),
            _ => None,
        }
    }
}

impl FormatTranslatorPort for FormatRegistry {
    fn translate_request(
        &self,
        from: ApiFormat,
        to: ApiFormat,
        req: CompletionRequest,
    ) -> CortexResult<CompletionRequest> {
        self.translate_request(from, to, req)
    }

    fn translate_response(
        &self,
        from: ApiFormat,
        to: ApiFormat,
        resp: CompletionResponse,
    ) -> CortexResult<CompletionResponse> {
        self.translate_response(from, to, resp)
    }
}

fn missing_translator(kind: &str, from: ApiFormat, to: ApiFormat) -> CortexError {
    CortexError::invalid_request(format!("missing {kind} translator for {from:?} -> {to:?}"))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_for_openai_returns_openai_variant() {
        let registry = FormatRegistry::new();
        assert_eq!(registry.format_for("openai"), Some(ApiFormat::OpenAI));
    }

    #[test]
    fn format_for_anthropic_returns_anthropic_variant() {
        let registry = FormatRegistry::new();
        assert_eq!(registry.format_for("anthropic"), Some(ApiFormat::Anthropic));
    }

    #[test]
    fn openai_compatible_provider_kinds_return_openai_variant() {
        let registry = FormatRegistry::new();
        assert_eq!(registry.format_for("ollama"), Some(ApiFormat::OpenAI));
        assert_eq!(registry.format_for("gemini"), Some(ApiFormat::OpenAI));
        assert_eq!(registry.format_for("groq"), Some(ApiFormat::OpenAI));
    }

    #[test]
    fn format_for_unknown_returns_none() {
        let registry = FormatRegistry::new();
        assert_eq!(registry.format_for("unknown"), None);
        assert_eq!(registry.format_for(""), None);
    }

    #[test]
    fn register_stores_request_and_response_translators() {
        let mut registry = FormatRegistry::new();
        registry.register(
            ApiFormat::OpenAI,
            ApiFormat::Anthropic,
            DomainPivotTranslator,
        );

        assert!(registry
            .get_request_translator(ApiFormat::OpenAI, ApiFormat::Anthropic)
            .is_some());
        assert!(registry
            .get_response_translator(ApiFormat::OpenAI, ApiFormat::Anthropic)
            .is_some());
    }

    #[test]
    fn missing_translator_returns_invalid_request_error() {
        let registry = FormatRegistry::new();
        let req = CompletionRequest {
            id: shared_kernel::RequestId::new(),
            model: shared_kernel::ModelId::new("model"),
            messages: Vec::new(),
            stream: false,
            max_tokens: None,
            temperature: None,
            tools: None,
            tool_choice: None,
            metadata: rook_core::RequestMetadata {
                origin: "test".to_string(),
                cacheable: false,
                priority: 0,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: rook_core::ApiKeyRestrictions::default(),
        };

        let error = registry
            .translate_request(ApiFormat::OpenAI, ApiFormat::Anthropic, req)
            .expect_err("translator should be missing");
        assert!(error.is_invalid_request());
    }
}
