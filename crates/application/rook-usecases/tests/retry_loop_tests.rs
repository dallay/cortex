// Integration tests for provider retry loop in RouteRequest.
// Tests failover behavior when providers fail with retryable errors.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rook_core::{
    ApiFormat, ApiKeyRestrictions, AuditEntry, AuditPort, CachePort, CacheStats, CompletionRequest,
    CompletionResponse, CortexError, CortexResult, FormatTranslatorPort, HealthStatus, Message,
    MessageContent, ModelAlias, ModelAliasRepositoryError, ModelAliasRepositoryPort, ModelId,
    ProviderId, ProviderPort, RequestMetadata, Role, RouterPort, SignatureEntry, StreamChunk,
    TokenCacheStats, TokenUsage,
};
use rook_usecases::{route_request::ModelAliasesConfig, PricingConfig, RouteRequest};
use shared_kernel::{CacheKey, RequestId};

// --- Fake Providers ---

/// Provider that fails with a retryable error (rate limited).
#[derive(Clone)]
struct RateLimitedProvider {
    id: ProviderId,
    models: Vec<ModelId>,
}

impl RateLimitedProvider {
    fn new(id: &str, models: Vec<&str>) -> Self {
        Self {
            id: ProviderId::new(id),
            models: models.into_iter().map(ModelId::new).collect(),
        }
    }
}

#[async_trait]
impl ProviderPort for RateLimitedProvider {
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
        // Return rate limited error - this is retryable
        Err(CortexError::rate_limited(self.id.clone(), 60))
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        Err(CortexError::provider("streaming not supported"))
    }
}

/// Provider that succeeds with a response.
#[derive(Clone)]
struct SuccessfulProvider {
    id: ProviderId,
    models: Vec<ModelId>,
}

impl SuccessfulProvider {
    fn new(id: &str, models: Vec<&str>) -> Self {
        Self {
            id: ProviderId::new(id),
            models: models.into_iter().map(ModelId::new).collect(),
        }
    }
}

#[async_trait]
impl ProviderPort for SuccessfulProvider {
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

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        let model = self
            .models
            .first()
            .ok_or_else(|| CortexError::invalid_request("SuccessfulProvider has no models"))?;
        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.id.clone(),
            model: model.clone(),
            content: "successful response".to_string(),
            content_blocks: vec![MessageContent::Text("successful response".to_string())],
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
            cache_hit: None,
        })
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        Err(CortexError::provider("streaming not supported"))
    }
}

/// Provider that fails with a non-retryable error (auth failed).
#[derive(Clone)]
struct AuthFailedProvider {
    id: ProviderId,
    models: Vec<ModelId>,
}

impl AuthFailedProvider {
    fn new(id: &str, models: Vec<&str>) -> Self {
        Self {
            id: ProviderId::new(id),
            models: models.into_iter().map(ModelId::new).collect(),
        }
    }
}

#[async_trait]
impl ProviderPort for AuthFailedProvider {
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
        // Return auth failed error - this is NOT retryable
        Err(CortexError::auth_failed("invalid API key"))
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        Err(CortexError::provider("streaming not supported"))
    }
}

// --- Fake Router ---

/// Router that cycles through a list of providers using RoundRobin on available ones.
/// First call returns first provider, second call returns second, etc.
#[derive(Clone)]
struct CyclingRouter {
    providers: Vec<Arc<dyn ProviderPort>>,
    round_robin_index: std::sync::Arc<std::sync::Mutex<usize>>,
}

impl CyclingRouter {
    fn new(providers: Vec<Arc<dyn ProviderPort>>) -> Self {
        Self {
            providers,
            round_robin_index: Arc::new(std::sync::Mutex::new(0)),
        }
    }
}

#[async_trait]
impl RouterPort for CyclingRouter {
    async fn select(&self, _req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        let mut index = self.round_robin_index.lock().unwrap();
        let idx = *index % self.providers.len();
        *index = idx + 1;
        Ok(self.providers[idx].clone())
    }

    async fn select_excluding(
        &self,
        _req: &CompletionRequest,
        excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        // Filter to non-excluded providers
        let available: Vec<_> = self
            .providers
            .iter()
            .filter(|p| !excluded.contains(p.id()))
            .collect();

        if available.is_empty() {
            return Err(CortexError::all_providers_exhausted());
        }

        // Apply RoundRobin to available providers
        let mut index = self.round_robin_index.lock().unwrap();
        let idx = *index % available.len();
        *index = (idx + 1) % available.len();
        Ok(available[idx].clone())
    }

    async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {
        // No-op for testing
    }

    fn providers(&self) -> Vec<ProviderId> {
        self.providers.iter().map(|p| p.id().clone()).collect()
    }
}

// --- NoOp implementations ---

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
            token_cache: TokenCacheStats::default(),
        })
    }

    async fn delete_by_signature(&self, _signature: &str) -> CortexResult<usize> {
        Ok(0)
    }

    async fn list_signatures(&self) -> CortexResult<Vec<SignatureEntry>> {
        Ok(Vec::new())
    }

    async fn get_by_signature(&self, _signature: &str) -> CortexResult<Option<CompletionResponse>> {
        Ok(None)
    }

    async fn increment_token_cache_hit(&self, _tokens: u64, _cost_usd: f64) -> CortexResult<()> {
        Ok(())
    }

    async fn increment_token_cache_miss(&self) -> CortexResult<()> {
        Ok(())
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

struct NoOpAliasRepo;

#[async_trait]
impl ModelAliasRepositoryPort for NoOpAliasRepo {
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

// --- Helper ---

fn make_request(model: &str) -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new(model),
        messages: vec![Message {
            role: Role::User,
            content: "hello".into(),
        }],
        stream: false,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".into(),
            cacheable: false,
            priority: 1,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions::default(),
    }
}

fn make_route_request(router: Arc<dyn RouterPort>) -> RouteRequest {
    RouteRequest::new(
        router,
        Arc::new(NoOpCache),
        Arc::new(NoOpAudit),
        None, // usage_recorder
        None, // provider_repository
        None, // combo_repository
        Arc::new(PricingConfig::default()),
        Arc::new(NoOpTranslator),
        Arc::new(NoOpAliasRepo),
        ModelAliasesConfig {
            enabled: false,
            auto_seed: false,
        },
        None, // telemetry
    )
}

// --- Tests ---

#[tokio::test]
async fn retry_loop_first_provider_fails_second_succeeds() {
    // Setup: first provider fails with rate limit, second succeeds
    let p1 = Arc::new(RateLimitedProvider::new("p1", vec!["model-a"]));
    let p2 = Arc::new(SuccessfulProvider::new("p2", vec!["model-a"]));
    let router = Arc::new(CyclingRouter::new(vec![p1, p2]));
    let route_request = make_route_request(router);

    let req = make_request("model-a");
    let result = route_request
        .execute_with_format(req, ApiFormat::OpenAI)
        .await;

    // Should succeed with p2's response
    assert!(result.is_ok(), "Expected success, got: {:?}", result);
    let resp = result.unwrap();
    assert_eq!(resp.provider.as_str(), "p2");
    assert_eq!(resp.content, "successful response");
}

#[tokio::test]
async fn retry_loop_all_providers_fail_returns_exhausted_error() {
    // Setup: both providers fail with rate limit
    let p1 = Arc::new(RateLimitedProvider::new("p1", vec!["model-a"]));
    let p2 = Arc::new(RateLimitedProvider::new("p2", vec!["model-a"]));
    let router = Arc::new(CyclingRouter::new(vec![p1, p2]));
    let route_request = make_route_request(router);

    let req = make_request("model-a");
    let result = route_request
        .execute_with_format(req, ApiFormat::OpenAI)
        .await;

    // Should fail with all providers exhausted
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.is_all_providers_exhausted(),
        "Expected AllProvidersExhausted, got: {}",
        err
    );
}

#[tokio::test]
async fn retry_loop_non_retryable_error_fails_immediately() {
    // Setup: provider fails with auth error (not retryable)
    let p1 = Arc::new(AuthFailedProvider::new("p1", vec!["model-a"]));
    let p2 = Arc::new(SuccessfulProvider::new("p2", vec!["model-a"]));
    let router = Arc::new(CyclingRouter::new(vec![p1, p2]));
    let route_request = make_route_request(router);

    let req = make_request("model-a");
    let result = route_request
        .execute_with_format(req, ApiFormat::OpenAI)
        .await;

    // Should fail immediately with auth error (no retry to p2)
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.is_auth_failed(), "Expected auth failed, got: {}", err);
}

#[tokio::test]
async fn retry_loop_exhausts_all_providers() {
    // Setup: 3 providers, all fail with rate limit
    let p1 = Arc::new(RateLimitedProvider::new("p1", vec!["model-a"]));
    let p2 = Arc::new(RateLimitedProvider::new("p2", vec!["model-a"]));
    let p3 = Arc::new(RateLimitedProvider::new("p3", vec!["model-a"]));
    let router = Arc::new(CyclingRouter::new(vec![p1, p2, p3]));
    let route_request = make_route_request(router);

    let req = make_request("model-a");
    let result = route_request
        .execute_with_format(req, ApiFormat::OpenAI)
        .await;

    // Should fail after exhausting all providers
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.is_all_providers_exhausted(),
        "Expected AllProvidersExhausted, got: {}",
        err
    );
}

#[tokio::test]
async fn retry_loop_empty_exclusion_list_works() {
    // Setup: single provider succeeds
    let p1 = Arc::new(SuccessfulProvider::new("p1", vec!["model-a"]));
    let router = Arc::new(CyclingRouter::new(vec![p1]));
    let route_request = make_route_request(router);

    let req = make_request("model-a");
    let result = route_request
        .execute_with_format(req, ApiFormat::OpenAI)
        .await;

    // Should succeed on first try
    assert!(result.is_ok(), "Expected success, got: {:?}", result);
    let resp = result.unwrap();
    assert_eq!(resp.provider.as_str(), "p1");
}
