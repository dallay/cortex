// Integration tests for model and provider restriction enforcement in route_request.
// Tests that RouteRequest::execute properly enforces allowed_models and allowed_providers.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rook_core::{
    ApiFormat, ApiKeyRestrictions, AuditEntry, AuditPort, CachePort, CacheStats, CompletionRequest,
    CompletionResponse, CortexError, CortexResult, FormatTranslatorPort, HealthStatus, Message,
    MessageContent, ModelId, ProviderId, ProviderPort, RequestMetadata, Role, RouterPort,
    StreamChunk, TokenUsage,
};
use rook_usecases::{PricingConfig, RouteRequest};
use shared_kernel::{CacheKey, RequestId};

// --- Fake Implementations ---

#[derive(Clone)]
struct FakeProvider {
    id: ProviderId,
    models: Vec<ModelId>,
}

impl FakeProvider {
    fn new(id: &str, models: Vec<&str>) -> Self {
        Self {
            id: ProviderId::new(id),
            models: models.into_iter().map(ModelId::new).collect(),
        }
    }
}

#[async_trait]
impl ProviderPort for FakeProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn supported_models(&self) -> &[ModelId] {
        &self.models
    }

    fn api_format(&self) -> ApiFormat {
        ApiFormat::OpenAI
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy {
            provider: self.id.clone(),
            latency_ms: 10,
        }
    }

    async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        Ok(CompletionResponse {
            id: RequestId::new(),
            provider: self.id.clone(),
            model: self.models[0].clone(),
            content: "test response".to_string(),
            content_blocks: vec![MessageContent::Text("test response".to_string())],
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 10,
        })
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        let chunk = StreamChunk {
            id: RequestId::new(),
            model: self.models[0].clone(),
            delta: "test".to_string(),
            finish_reason: None,
            usage: None,
        };
        Ok(Box::pin(futures::stream::once(async move { Ok(chunk) })))
    }
}

struct FakeRouter {
    provider: Arc<dyn ProviderPort>,
}

impl FakeRouter {
    fn new(provider: Arc<dyn ProviderPort>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl RouterPort for FakeRouter {
    async fn select(&self, _req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        Ok(self.provider.clone())
    }

    async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}

    fn providers(&self) -> Vec<ProviderId> {
        vec![self.provider.id().clone()]
    }
}

struct NoOpCache;

#[async_trait]
impl CachePort for NoOpCache {
    async fn get(&self, _key: &CacheKey) -> CortexResult<Option<CompletionResponse>> {
        Ok(None)
    }

    async fn set(
        &self,
        _key: &CacheKey,
        _value: &CompletionResponse,
        _ttl: Duration,
    ) -> CortexResult<()> {
        Ok(())
    }

    async fn delete(&self, _key: &CacheKey) -> CortexResult<()> {
        Ok(())
    }

    async fn clear(&self) -> CortexResult<()> {
        Ok(())
    }

    async fn stats(&self) -> CortexResult<CacheStats> {
        Ok(CacheStats {
            hits: 0,
            misses: 0,
            evictions: 0,
            entries: 0,
            max_entries: 0,
        })
    }

    async fn delete_by_signature(&self, _signature: &str) -> CortexResult<usize> {
        Ok(0)
    }
}

struct NoOpAudit;

#[async_trait]
impl AuditPort for NoOpAudit {
    async fn record(&self, _entry: AuditEntry) -> CortexResult<()> {
        Ok(())
    }
}

struct NoOpTranslator;

impl FormatTranslatorPort for NoOpTranslator {
    fn translate_request(
        &self,
        _from: ApiFormat,
        _to: ApiFormat,
        req: CompletionRequest,
    ) -> CortexResult<CompletionRequest> {
        Ok(req)
    }

    fn translate_response(
        &self,
        _from: ApiFormat,
        _to: ApiFormat,
        resp: CompletionResponse,
    ) -> CortexResult<CompletionResponse> {
        Ok(resp)
    }
}

// --- Test Cases ---

#[tokio::test]
async fn allowed_models_contains_requested_model_passes() {
    let provider = Arc::new(FakeProvider::new("openai", vec!["gpt-4"]));
    let router = Arc::new(FakeRouter::new(provider)) as Arc<dyn RouterPort>;
    let cache = Arc::new(NoOpCache) as Arc<dyn CachePort>;
    let audit = Arc::new(NoOpAudit) as Arc<dyn AuditPort>;
    let translator = Arc::new(NoOpTranslator) as Arc<dyn FormatTranslatorPort>;

    let route_request = RouteRequest::new(
        router,
        cache,
        audit,
        None,
        None,
        None,
        Arc::new(PricingConfig::default()),
        translator,
    );

    let req = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("gpt-4"),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("test".to_string()),
        }],
        stream: false,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".to_string(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions {
            allowed_models: vec![ModelId::new("gpt-4"), ModelId::new("gpt-3.5-turbo")],
            allowed_providers: vec![],
        },
    };

    let result = route_request
        .execute_with_format(req, ApiFormat::OpenAI)
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn allowed_models_missing_requested_model_returns_403_with_structured_code() {
    let provider = Arc::new(FakeProvider::new("openai", vec!["gpt-4"]));
    let router = Arc::new(FakeRouter::new(provider)) as Arc<dyn RouterPort>;
    let cache = Arc::new(NoOpCache) as Arc<dyn CachePort>;
    let audit = Arc::new(NoOpAudit) as Arc<dyn AuditPort>;
    let translator = Arc::new(NoOpTranslator) as Arc<dyn FormatTranslatorPort>;

    let route_request = RouteRequest::new(
        router,
        cache,
        audit,
        None,
        None,
        None,
        Arc::new(PricingConfig::default()),
        translator,
    );

    let req = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("gpt-4o"),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("test".to_string()),
        }],
        stream: false,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".to_string(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions {
            allowed_models: vec![ModelId::new("gpt-4")],
            allowed_providers: vec![],
        },
    };

    let result = route_request
        .execute_with_format(req, ApiFormat::OpenAI)
        .await;
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(error.is_forbidden());
    assert_eq!(error.forbidden_code(), Some("model_not_allowed"));
    assert!(error.to_string().contains("gpt-4o"));
}

#[tokio::test]
async fn allowed_providers_contains_selected_provider_passes() {
    let provider = Arc::new(FakeProvider::new("openai", vec!["gpt-4"]));
    let router = Arc::new(FakeRouter::new(provider)) as Arc<dyn RouterPort>;
    let cache = Arc::new(NoOpCache) as Arc<dyn CachePort>;
    let audit = Arc::new(NoOpAudit) as Arc<dyn AuditPort>;
    let translator = Arc::new(NoOpTranslator) as Arc<dyn FormatTranslatorPort>;

    let route_request = RouteRequest::new(
        router,
        cache,
        audit,
        None,
        None,
        None,
        Arc::new(PricingConfig::default()),
        translator,
    );

    let req = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("gpt-4"),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("test".to_string()),
        }],
        stream: false,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".to_string(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions {
            allowed_models: vec![],
            allowed_providers: vec![ProviderId::new("openai"), ProviderId::new("anthropic")],
        },
    };

    let result = route_request
        .execute_with_format(req, ApiFormat::OpenAI)
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn allowed_providers_missing_selected_provider_returns_403_with_structured_code() {
    let provider = Arc::new(FakeProvider::new("anthropic", vec!["claude-3"]));
    let router = Arc::new(FakeRouter::new(provider)) as Arc<dyn RouterPort>;
    let cache = Arc::new(NoOpCache) as Arc<dyn CachePort>;
    let audit = Arc::new(NoOpAudit) as Arc<dyn AuditPort>;
    let translator = Arc::new(NoOpTranslator) as Arc<dyn FormatTranslatorPort>;

    let route_request = RouteRequest::new(
        router,
        cache,
        audit,
        None,
        None,
        None,
        Arc::new(PricingConfig::default()),
        translator,
    );

    let req = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("claude-3"),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("test".to_string()),
        }],
        stream: false,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".to_string(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions {
            allowed_models: vec![],
            allowed_providers: vec![ProviderId::new("openai")],
        },
    };

    let result = route_request
        .execute_with_format(req, ApiFormat::OpenAI)
        .await;
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(error.is_forbidden());
    assert_eq!(error.forbidden_code(), Some("provider_not_allowed"));
    assert!(error.to_string().contains("anthropic"));
}
