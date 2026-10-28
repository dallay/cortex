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
        let mut req = req;
        // Responses normaliza funciones al contrato OpenAI. La traducción al
        // formato Anthropic ocurre en el límite de formato existente, no en routing.
        // Los contratos de los endpoints anteriores permanecen sin cambios.
        if req.metadata.origin == "responses"
            && from == ApiFormat::OpenAI
            && to == ApiFormat::Anthropic
        {
            if let Some(serde_json::Value::Array(tools)) = req.tools.as_mut() {
                for tool in tools {
                    let function = &tool["function"];
                    let mut native = serde_json::json!({"name":function["name"], "input_schema":function["parameters"]});
                    if let Some(description) = function.get("description") {
                        native["description"] = description.clone();
                    }
                    *tool = native;
                }
            }
            if let Some(choice) = req.tool_choice.as_mut() {
                *choice = match choice.as_str() {
                    Some("auto") => serde_json::json!({"type":"auto"}),
                    Some("none") => serde_json::json!({"type":"none"}),
                    Some("required") => serde_json::json!({"type":"any"}),
                    _ => serde_json::json!({"type":"tool","name":choice["function"]["name"]}),
                };
            }
        }
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
            "openai" | "ollama" | "ollama-cloud" | "gemini" | "groq" => Some(ApiFormat::OpenAI),
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
        assert_eq!(registry.format_for("ollama-cloud"), Some(ApiFormat::OpenAI));
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

    #[test]
    fn test_translate_request_responses_origin_tool_choice_variants() {
        let mut registry = FormatRegistry::new();
        registry.register(
            ApiFormat::OpenAI,
            ApiFormat::Anthropic,
            DomainPivotTranslator,
        );

        let make_req = |tool_choice: serde_json::Value| CompletionRequest {
            id: shared_kernel::RequestId::new(),
            model: shared_kernel::ModelId::new("model"),
            messages: Vec::new(),
            stream: false,
            max_tokens: None,
            temperature: None,
            tools: Some(serde_json::json!([
                {
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Get current weather",
                        "parameters": {"type": "object"}
                    }
                }
            ])),
            tool_choice: Some(tool_choice),
            metadata: rook_core::RequestMetadata {
                origin: "responses".to_string(),
                cacheable: false,
                priority: 0,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: rook_core::ApiKeyRestrictions::default(),
        };

        let req_req = registry
            .translate_request(
                ApiFormat::OpenAI,
                ApiFormat::Anthropic,
                make_req(serde_json::json!("required")),
            )
            .unwrap();
        assert_eq!(
            req_req.tool_choice,
            Some(serde_json::json!({"type": "any"}))
        );

        let req_none = registry
            .translate_request(
                ApiFormat::OpenAI,
                ApiFormat::Anthropic,
                make_req(serde_json::json!("none")),
            )
            .unwrap();
        assert_eq!(
            req_none.tool_choice,
            Some(serde_json::json!({"type": "none"}))
        );

        let req_func = registry
            .translate_request(
                ApiFormat::OpenAI,
                ApiFormat::Anthropic,
                make_req(serde_json::json!({
                    "type": "function",
                    "function": {"name": "get_weather"}
                })),
            )
            .unwrap();
        assert_eq!(
            req_func.tool_choice,
            Some(serde_json::json!({"type": "tool", "name": "get_weather"}))
        );

        let tool = &req_func.tools.as_ref().unwrap()[0];
        assert_eq!(tool["name"], "get_weather");
        assert_eq!(tool["description"], "Get current weather");
        assert_eq!(tool["input_schema"], serde_json::json!({"type": "object"}));
    }

    #[test]
    fn test_translate_response_success() {
        let mut registry = FormatRegistry::new();
        registry.register(
            ApiFormat::OpenAI,
            ApiFormat::Anthropic,
            DomainPivotTranslator,
        );

        let resp = CompletionResponse {
            id: shared_kernel::RequestId::new(),
            provider: shared_kernel::ProviderId::new("p"),
            model: shared_kernel::ModelId::new("m"),
            content: "hello".into(),
            content_blocks: vec![],
            thinking: None,
            tool_calls: vec![],
            finish_reason: Some(rook_core::FinishReason::Stop),
            usage: rook_core::TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 10,
            cache_hit: None,
        };

        let translated = registry
            .translate_response(ApiFormat::OpenAI, ApiFormat::Anthropic, resp)
            .unwrap();
        assert_eq!(translated.content, "hello");
    }
}
