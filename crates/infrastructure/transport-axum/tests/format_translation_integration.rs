// format_translation_integration.rs
//
// Integration tests for the provider format translation layer.
//
// These tests verify the full adapter chain:
//   JSON body → adapter struct (serde) → CompletionRequest (domain) →
//   CompletionResponse (mock) → response struct → JSON body
//
// ACs covered:
//   SC-04 + SC-09: OpenAI round-trip (content preserved, object == "chat.completion")
//   SC-05 + SC-10: Anthropic round-trip (content[0].type == "text", stop_reason == "end_turn")
//   SC-01 + SC-02: No parse error on requests that include `tools` or `stream_options` fields

use async_trait::async_trait;
use rook_core::{
    ApiFormat, AuditEntry, AuditPort, CachePort, CompletionRequest, CompletionResponse,
    HealthStatus, MessageContent, ModelAlias, ModelAliasRepositoryError, ModelAliasRepositoryPort,
    ModelId, ProviderPort, RequestMetadata, Role, RouterPort, StreamChunk, TokenUsage,
};
use rook_usecases::{route_request::ModelAliasesConfig, RouteRequest, TokenCacheConfig};
use shared_kernel::{CacheKey, ProviderId, RequestId};
use std::sync::Arc;
use std::time::Duration;
use transport_axum::{
    anthropic_adapter::{AnthropicMessagesRequest, AnthropicMessagesResponse},
    openai_adapter::{OpenAIChatRequest, OpenAIChatResponse},
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn mock_completion_response(content: &str) -> CompletionResponse {
    CompletionResponse {
        id: RequestId::new(),
        provider: ProviderId::new("test-provider"),
        model: ModelId::new("test-model"),
        content: content.to_string(),
        content_blocks: vec![MessageContent::Text(content.to_string())],
        usage: TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            reasoning_tokens: None,
            estimated_cost_usd: None,
        },
        latency_ms: 42,
        cache_hit: None,
    }
}

// ---------------------------------------------------------------------------
// OpenAI format round-trip
// ---------------------------------------------------------------------------

#[test]
fn openai_minimal_request_round_trip() {
    // Deserialize a minimal OpenAI request
    let json = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}]}"#;
    let req: OpenAIChatRequest = serde_json::from_str(json).expect("should parse minimal request");

    // Convert to domain
    let domain_req = rook_core::CompletionRequest::from(req);
    assert_eq!(domain_req.model.as_str(), "gpt-4o");
    assert_eq!(domain_req.messages.len(), 1);
    assert_eq!(domain_req.messages[0].role, Role::User);
    assert_eq!(
        domain_req.messages[0].content,
        MessageContent::Text("Hello".to_string())
    );

    // Build a mock response and convert back to OpenAI format
    let mock_resp = mock_completion_response("Hello back!");
    let openai_resp = OpenAIChatResponse::from(&mock_resp);
    let resp_json = serde_json::to_value(&openai_resp).expect("should serialize");

    // SC-09: object == "chat.completion"
    assert_eq!(resp_json["object"], "chat.completion");
    // SC-04: choices[0].message.content preserved
    assert_eq!(resp_json["choices"][0]["message"]["content"], "Hello back!");
}

#[test]
fn openai_request_with_tools_and_stream_options_does_not_error() {
    // SC-01: tools / stream_options must not cause a parse error (no 422)
    let json = r#"{
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Hi"}],
        "tools": [{"type": "function", "function": {"name": "get_weather"}}],
        "tool_choice": "auto",
        "stream_options": {"include_usage": true},
        "response_format": {"type": "text"}
    }"#;

    let req: OpenAIChatRequest =
        serde_json::from_str(json).expect("should parse request with tools and stream_options");

    assert_eq!(req.model, "gpt-4o");
    assert!(req.tools.is_some(), "tools field should be present");
    assert!(
        req.stream_options.is_some(),
        "stream_options should be present"
    );
    assert!(
        req.response_format.is_some(),
        "response_format should be present"
    );
}

#[test]
fn openai_response_has_correct_structure() {
    let mock_resp = mock_completion_response("The answer is 42.");
    let openai_resp = OpenAIChatResponse::from(&mock_resp);
    let json = serde_json::to_value(&openai_resp).expect("should serialize");

    assert_eq!(json["object"], "chat.completion");
    assert!(json["id"].is_string());
    assert!(json["created"].is_number());
    assert_eq!(json["choices"][0]["message"]["role"], "assistant");
    assert_eq!(
        json["choices"][0]["message"]["content"],
        "The answer is 42."
    );
    assert_eq!(json["choices"][0]["finish_reason"], "stop");
    assert_eq!(json["usage"]["prompt_tokens"], 10);
    assert_eq!(json["usage"]["completion_tokens"], 5);
}

// ---------------------------------------------------------------------------
// Anthropic format round-trip
// ---------------------------------------------------------------------------

#[test]
fn anthropic_minimal_request_round_trip() {
    // Deserialize a minimal Anthropic request
    let json = r#"{"model":"claude-opus-4-5","messages":[{"role":"user","content":"Hello"}],"max_tokens":1024}"#;
    let req: AnthropicMessagesRequest =
        serde_json::from_str(json).expect("should parse minimal Anthropic request");

    // Convert to domain
    let domain_req = rook_core::CompletionRequest::from(req);
    assert_eq!(domain_req.model.as_str(), "claude-opus-4-5");
    assert_eq!(domain_req.messages[0].role, Role::User);

    // Build a mock response and convert back to Anthropic format
    let mock_resp = mock_completion_response("Bonjour!");
    let anthropic_resp = AnthropicMessagesResponse::from(&mock_resp);
    let resp_json = serde_json::to_value(&anthropic_resp).expect("should serialize");

    // SC-10: content[0].type == "text"
    assert_eq!(resp_json["content"][0]["type"], "text");
    // SC-05: content preserved
    assert_eq!(resp_json["content"][0]["text"], "Bonjour!");
    // stop_reason == "end_turn"
    assert_eq!(resp_json["stop_reason"], "end_turn");
}

#[test]
fn anthropic_request_with_tools_does_not_error() {
    // SC-02: tools / tool_choice must not cause a parse error (no 422)
    let json = r#"{
        "model": "claude-opus-4-5",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "What is the weather?"}],
        "tools": [{"name": "get_weather", "description": "Get weather", "input_schema": {}}],
        "tool_choice": {"type": "auto"}
    }"#;

    let req: AnthropicMessagesRequest =
        serde_json::from_str(json).expect("should parse request with tools");

    assert_eq!(req.model, "claude-opus-4-5");
    assert!(req.tools.is_some(), "tools field should be present");
    assert!(req.tool_choice.is_some(), "tool_choice should be present");
}

#[test]
fn anthropic_system_field_prepends_system_message() {
    // SC-16: system field at the top level is prepended as a Role::System message
    let json = r#"{
        "model": "claude-opus-4-5",
        "max_tokens": 512,
        "system": "You are a helpful assistant.",
        "messages": [{"role": "user", "content": "Hi"}]
    }"#;

    let req: AnthropicMessagesRequest =
        serde_json::from_str(json).expect("should parse request with system field");
    let domain_req = rook_core::CompletionRequest::from(req);

    // Should have 2 messages: system first, then user
    assert_eq!(domain_req.messages.len(), 2);
    assert_eq!(domain_req.messages[0].role, Role::System);
    assert_eq!(
        domain_req.messages[0].content,
        MessageContent::Text("You are a helpful assistant.".to_string())
    );
    assert_eq!(domain_req.messages[1].role, Role::User);
}

#[test]
fn anthropic_response_has_correct_structure() {
    let mock_resp = mock_completion_response("42 is the answer.");
    let anthropic_resp = AnthropicMessagesResponse::from(&mock_resp);
    let json = serde_json::to_value(&anthropic_resp).expect("should serialize");

    assert_eq!(json["type"], "message");
    assert_eq!(json["role"], "assistant");
    assert_eq!(json["stop_reason"], "end_turn");
    assert_eq!(json["content"][0]["type"], "text");
    assert_eq!(json["content"][0]["text"], "42 is the answer.");
    assert_eq!(json["usage"]["input_tokens"], 10);
    assert_eq!(json["usage"]["output_tokens"], 5);
}

// ---------------------------------------------------------------------------
// Registry-routed multi-format use case integration
// ---------------------------------------------------------------------------

use futures::stream;
use rook_core::FormatTranslatorPort;
use transport_axum::format_registry::{DomainPivotTranslator, FormatRegistry};

struct RegistryTestProvider {
    id: ProviderId,
    format: ApiFormat,
    content: &'static str,
}

#[async_trait]
impl ProviderPort for RegistryTestProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn supported_models(&self) -> &[ModelId] {
        std::slice::from_ref(&REGISTRY_TEST_MODEL)
    }

    fn api_format(&self) -> ApiFormat {
        self.format
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy {
            provider: self.id.clone(),
            latency_ms: 1,
        }
    }

    async fn complete(
        &self,
        req: &CompletionRequest,
    ) -> shared_kernel::CortexResult<CompletionResponse> {
        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.id.clone(),
            model: req.model.clone(),
            content: self.content.to_string(),
            content_blocks: vec![MessageContent::Text(self.content.to_string())],
            usage: TokenUsage {
                prompt_tokens: 3,
                completion_tokens: 5,
                total_tokens: 8,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 7,
            cache_hit: None,
        })
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> shared_kernel::CortexResult<
        futures::stream::BoxStream<'static, shared_kernel::CortexResult<StreamChunk>>,
    > {
        Ok(Box::pin(stream::empty()))
    }
}

struct RegistryTestRouter {
    provider: Arc<dyn ProviderPort>,
}

#[async_trait]
impl RouterPort for RegistryTestRouter {
    async fn select(
        &self,
        _req: &CompletionRequest,
    ) -> shared_kernel::CortexResult<Arc<dyn ProviderPort>> {
        Ok(self.provider.clone())
    }

    async fn on_failure(&self, _provider: &ProviderId, _error: &shared_kernel::CortexError) {}

    fn providers(&self) -> Vec<ProviderId> {
        vec![self.provider.id().clone()]
    }
}

struct NoopCache;

#[async_trait]
impl CachePort for NoopCache {
    async fn get(
        &self,
        _key: &CacheKey,
    ) -> shared_kernel::CortexResult<Option<CompletionResponse>> {
        Ok(None)
    }

    async fn set(
        &self,
        _key: &CacheKey,
        _value: &CompletionResponse,
        _ttl: Duration,
    ) -> shared_kernel::CortexResult<()> {
        Ok(())
    }

    async fn delete(&self, _key: &CacheKey) -> shared_kernel::CortexResult<()> {
        Ok(())
    }

    async fn clear(&self) -> shared_kernel::CortexResult<()> {
        Ok(())
    }

    async fn stats(&self) -> shared_kernel::CortexResult<rook_core::CacheStats> {
        Ok(rook_core::CacheStats {
            hits: 0,
            misses: 0,
            evictions: 0,
            entries: 0,
            max_entries: 0,
            token_cache: rook_core::TokenCacheStats::default(),
        })
    }

    async fn delete_by_signature(&self, _signature: &str) -> shared_kernel::CortexResult<usize> {
        Ok(0)
    }

    async fn list_signatures(&self) -> shared_kernel::CortexResult<Vec<rook_core::SignatureEntry>> {
        Ok(Vec::new())
    }

    async fn get_by_signature(
        &self,
        _signature: &str,
    ) -> shared_kernel::CortexResult<Option<rook_core::CompletionResponse>> {
        Ok(None)
    }

    async fn increment_token_cache_hit(
        &self,
        _tokens: u64,
        _cost_usd: f64,
    ) -> shared_kernel::CortexResult<()> {
        Ok(())
    }

    async fn increment_token_cache_miss(&self) -> shared_kernel::CortexResult<()> {
        Ok(())
    }
}

struct NoopAudit;

#[async_trait]
impl AuditPort for NoopAudit {
    async fn record(&self, _entry: AuditEntry) -> shared_kernel::CortexResult<()> {
        Ok(())
    }
}

/// Test stub for ModelAliasRepositoryPort
struct NoopAliasRepository;

#[async_trait]
impl ModelAliasRepositoryPort for NoopAliasRepository {
    async fn find_by_alias(
        &self,
        _alias: &ModelId,
        _provider_id: Option<&ProviderId>,
    ) -> Result<Option<ModelAlias>, ModelAliasRepositoryError> {
        Ok(None)
    }

    async fn list(&self) -> Result<Vec<ModelAlias>, ModelAliasRepositoryError> {
        Ok(vec![])
    }

    async fn create(&self, _alias: ModelAlias) -> Result<(), ModelAliasRepositoryError> {
        Ok(())
    }

    async fn delete(&self, _alias: &ModelId) -> Result<bool, ModelAliasRepositoryError> {
        Ok(false)
    }

    async fn seed(&self, _aliases: Vec<ModelAlias>) -> Result<usize, ModelAliasRepositoryError> {
        Ok(0)
    }
}

static REGISTRY_TEST_MODEL: std::sync::LazyLock<ModelId> =
    std::sync::LazyLock::new(|| ModelId::new("registry-test-model"));

fn registry_with_openai_anthropic_pairs() -> Arc<dyn FormatTranslatorPort> {
    let mut registry = FormatRegistry::new();
    registry.register(
        ApiFormat::OpenAI,
        ApiFormat::Anthropic,
        DomainPivotTranslator,
    );
    registry.register(
        ApiFormat::Anthropic,
        ApiFormat::OpenAI,
        DomainPivotTranslator,
    );
    Arc::new(registry)
}

fn registry_route_request(provider_format: ApiFormat, content: &'static str) -> RouteRequest {
    let provider: Arc<dyn ProviderPort> = Arc::new(RegistryTestProvider {
        id: ProviderId::new(format!("{provider_format:?}-provider")),
        format: provider_format,
        content,
    });

    RouteRequest::new(
        Arc::new(RegistryTestRouter { provider }),
        Arc::new(NoopCache),
        Arc::new(NoopAudit),
        None,
        None,
        None, // combo_repository
        Arc::new(rook_usecases::PricingConfig::default()),
        registry_with_openai_anthropic_pairs(),
        Arc::new(NoopAliasRepository),
        ModelAliasesConfig {
            enabled: false,
            auto_seed: false,
        },
        None, // telemetry
        TokenCacheConfig::default(),
    )
}

fn registry_domain_request() -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: REGISTRY_TEST_MODEL.clone(),
        messages: vec![rook_core::Message {
            role: Role::User,
            content: MessageContent::Text("hello through registry".to_string()),
        }],
        stream: false,
        max_tokens: Some(128),
        temperature: Some(0.2),
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "registry-test".to_string(),
            cacheable: false,
            priority: 1,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
            cache_control_header: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    }
}

#[tokio::test]
async fn anthropic_client_routes_to_openai_provider_via_registry() {
    let usecase = registry_route_request(ApiFormat::OpenAI, "openai provider response");

    let response = usecase
        .execute_with_format(registry_domain_request(), ApiFormat::Anthropic)
        .await
        .expect("registry should route Anthropic client request to OpenAI provider");

    assert_eq!(response.provider.as_str(), "OpenAI-provider");
    assert_eq!(response.content, "openai provider response");
}

#[tokio::test]
async fn openai_client_routes_to_anthropic_provider_via_registry() {
    let usecase = registry_route_request(ApiFormat::Anthropic, "anthropic provider response");

    let response = usecase
        .execute_with_format(registry_domain_request(), ApiFormat::OpenAI)
        .await
        .expect("registry should route OpenAI client request to Anthropic provider");

    assert_eq!(response.provider.as_str(), "Anthropic-provider");
    assert_eq!(response.content, "anthropic provider response");
}
