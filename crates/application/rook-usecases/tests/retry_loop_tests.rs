// Integration tests for provider retry loop in RouteRequest.
// Tests failover behavior when providers fail with retryable errors.

use std::sync::Arc;

use async_trait::async_trait;
use rook_core::{
    ApiFormat, ApiKeyRestrictions, CompletionRequest, CompletionResponse, CortexError,
    CortexResult, HealthStatus, Message, MessageContent, ModelId, ProviderId, ProviderPort,
    RequestMetadata, Role, RouterPort, StreamChunk, TokenUsage,
};
use rook_usecases::{route_request::ModelAliasesConfig, PricingConfig, RouteRequest};
use shared_kernel::RequestId;

mod common;
use common::{NoOpAliasRepository, NoOpAudit, NoOpCache, NoOpTranslator};

// --- Fake Providers ---

/// Behavior for the test provider's `complete()` method.
#[derive(Clone)]
enum TestCompleteBehavior {
    RateLimited,
    Success,
    AuthFailed,
}

/// Test provider with configurable behavior.
#[derive(Clone)]
struct TestProvider {
    id: ProviderId,
    models: Vec<ModelId>,
    behavior: TestCompleteBehavior,
}

impl TestProvider {
    fn new(id: &str, models: Vec<&str>, behavior: TestCompleteBehavior) -> Self {
        Self {
            id: ProviderId::new(id),
            models: models.into_iter().map(ModelId::new).collect(),
            behavior,
        }
    }

    fn rate_limited(id: &str, models: Vec<&str>) -> Self {
        Self::new(id, models, TestCompleteBehavior::RateLimited)
    }

    fn successful(id: &str, models: Vec<&str>) -> Self {
        Self::new(id, models, TestCompleteBehavior::Success)
    }

    fn auth_failed(id: &str, models: Vec<&str>) -> Self {
        Self::new(id, models, TestCompleteBehavior::AuthFailed)
    }
}

#[async_trait]
impl ProviderPort for TestProvider {
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
        match self.behavior {
            TestCompleteBehavior::RateLimited => {
                Err(CortexError::rate_limited(self.id.clone(), 60))
            }
            TestCompleteBehavior::Success => {
                let model = self
                    .models
                    .first()
                    .ok_or_else(|| CortexError::invalid_request("TestProvider has no models"))?;
                Ok(CompletionResponse {
                    id: req.id.clone(),
                    provider: self.id.clone(),
                    model: model.clone(),
                    content: "successful response".to_string(),
                    content_blocks: vec![MessageContent::Text("successful response".to_string())],
                    thinking: None,
                    tool_calls: vec![],
                    finish_reason: None,
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
            TestCompleteBehavior::AuthFailed => Err(CortexError::auth_failed("invalid API key")),
        }
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

// --- Helper ---

fn make_request(model: &str) -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new(model),
        messages: vec![Message {
            tool_calls: vec![],
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
        Arc::new(NoOpAliasRepository),
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
    let p1 = Arc::new(TestProvider::rate_limited("p1", vec!["model-a"]));
    let p2 = Arc::new(TestProvider::successful("p2", vec!["model-a"]));
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
    let p1 = Arc::new(TestProvider::rate_limited("p1", vec!["model-a"]));
    let p2 = Arc::new(TestProvider::rate_limited("p2", vec!["model-a"]));
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
    let p1 = Arc::new(TestProvider::auth_failed("p1", vec!["model-a"]));
    let p2 = Arc::new(TestProvider::successful("p2", vec!["model-a"]));
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
    let p1 = Arc::new(TestProvider::rate_limited("p1", vec!["model-a"]));
    let p2 = Arc::new(TestProvider::rate_limited("p2", vec!["model-a"]));
    let p3 = Arc::new(TestProvider::rate_limited("p3", vec!["model-a"]));
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
    let p1 = Arc::new(TestProvider::successful("p1", vec!["model-a"]));
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
