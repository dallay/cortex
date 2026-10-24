use super::*;
use async_trait::async_trait;
use chrono::Utc;
use futures::{stream, StreamExt};
use rook_core::{
    ApiKeyId, CacheStats, Combo, ComboAlias, ComboAliasRepositoryError, ComboAliasRepositoryPort,
    ComboRepositoryError, ComboStep, ComboStrategy, CostBreakdown, HealthStatus, Message,
    ModelAlias, ModelAliasRepositoryError, ModelId, Pagination, ProviderId, ProviderPort,
    ProviderRepositoryPort, QuotaObservation, QuotaObservationRecorderPort, RequestMetadata, Role,
    SignatureEntry, StreamChunk, TokenCacheStats, TokenUsage, UsageEntry, UsageFilters,
    UsageRecorderPort, UsageSummary,
};
use shared_kernel::{CacheKey, ConnectionId, CortexResult, RequestId};
use std::collections::HashMap;
use std::sync::Mutex;

struct TestProvider {
    id: ProviderId,
    complete_error: Option<CortexError>,
}

#[async_trait]
impl ProviderPort for TestProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn supported_models(&self) -> &[ModelId] {
        std::slice::from_ref(&TEST_MODEL)
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
            latency_ms: 1,
        }
    }

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        if self.complete_error.is_some() {
            return Err(CortexError::provider("provider failed"));
        }

        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.id.clone(),
            model: req.model.clone(),
            content: "cached path".to_string(),
            content_blocks: vec![rook_core::MessageContent::Text("cached path".to_string())],
            thinking: None,
            tool_calls: vec![],
            finish_reason: None,
            usage: TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 1,
            cache_hit: Some(true),
        })
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        Ok(Box::pin(stream::iter(vec![
            Ok(StreamChunk {
                id: RequestId::new(),
                model: TEST_MODEL.clone(),
                delta: "hel".to_string(),
                thinking: None,
                tool_calls: vec![],
                finish_reason: None,
                usage: None,
            }),
            Ok(StreamChunk {
                id: RequestId::new(),
                model: TEST_MODEL.clone(),
                delta: "lo".to_string(),
                thinking: None,
                tool_calls: vec![],
                finish_reason: Some(rook_core::FinishReason::Stop),
                usage: Some(TokenUsage {
                    prompt_tokens: 2,
                    completion_tokens: 3,
                    total_tokens: 5,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    reasoning_tokens: None,
                    estimated_cost_usd: None,
                }),
            }),
        ])))
    }
}

struct TestRouter {
    provider: Arc<dyn ProviderPort>,
}

#[async_trait]
impl RouterPort for TestRouter {
    async fn select(&self, _req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        Ok(self.provider.clone())
    }

    async fn select_excluding(
        &self,
        _req: &CompletionRequest,
        _excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        Ok(self.provider.clone())
    }

    async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}

    fn providers(&self) -> Vec<ProviderId> {
        vec![self.provider.id().clone()]
    }
}

#[tokio::test]
async fn local_backpressure_skips_circuit_but_upstream_rate_limit_opens_cooldown() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("limited"),
        complete_error: None,
    });
    let provider_id = provider.id().clone();
    let router = crate::FallbackRouter::new(vec![provider], crate::RoutingStrategy::Priority);

    notify_provider_failure(&router, &provider_id, &CortexError::local_backpressure()).await;
    let local_state = router.circuit_states();
    assert_eq!(local_state[0].1.failures, 0);
    assert!(local_state[0].1.cooldown_until.is_none());

    notify_provider_failure(
        &router,
        &provider_id,
        &CortexError::rate_limited(provider_id.clone(), 30),
    )
    .await;
    let upstream_state = router.circuit_states();
    assert!(upstream_state[0].1.cooldown_until.is_some());
}

struct BlockingTestProvider {
    inner: TestProvider,
    started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

#[async_trait]
impl ProviderPort for BlockingTestProvider {
    fn id(&self) -> &ProviderId {
        &self.inner.id
    }
    fn supported_models(&self) -> &[ModelId] {
        self.inner.supported_models()
    }
    fn api_format(&self) -> ApiFormat {
        self.inner.api_format()
    }
    fn is_available(&self) -> bool {
        true
    }
    async fn health_check(&self) -> HealthStatus {
        self.inner.health_check().await
    }
    async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(());
        }
        std::future::pending().await
    }
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        self.inner.stream(req).await
    }
}

async fn busy_limited_test_provider() -> (Arc<dyn ProviderPort>, tokio::task::JoinHandle<()>) {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let inner: Arc<dyn ProviderPort> = Arc::new(BlockingTestProvider {
        inner: TestProvider {
            id: ProviderId::new("test-provider"),
            complete_error: None,
        },
        started: Mutex::new(Some(started_tx)),
    });
    let limited = Arc::new(crate::provider_concurrency::ConcurrencyLimitedProvider::new(inner, 1));
    let holding_provider = limited.clone();
    let holder = tokio::spawn(async move {
        let _ = holding_provider.complete(&request()).await;
    });
    started_rx.await.expect("provider call holds admission");
    (limited, holder)
}

fn make_usecase_with_provider(provider: Arc<dyn ProviderPort>) -> RouteRequest {
    RouteRequest::new(
        Arc::new(TestRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
}

#[tokio::test]
async fn execute_returns_local_backpressure_when_provider_admission_is_busy() {
    let (provider, holder) = busy_limited_test_provider().await;
    let usecase = make_usecase_with_provider(provider);
    let mut req = request();
    req.stream = false;
    req.metadata.cacheable = false;

    let error = usecase
        .execute(req)
        .await
        .expect_err("busy provider rejects admission");
    holder.abort();
    assert!(
        error.is_local_backpressure(),
        "expected local 429 classification, got {error}"
    );
}

struct ExhaustingAfterExcludedRouter {
    provider: Arc<dyn ProviderPort>,
}

#[async_trait]
impl RouterPort for ExhaustingAfterExcludedRouter {
    async fn select(&self, _req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        Ok(self.provider.clone())
    }

    async fn select_excluding(
        &self,
        _req: &CompletionRequest,
        excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        if excluded.contains(self.provider.id()) {
            Err(CortexError::all_providers_exhausted())
        } else {
            Ok(self.provider.clone())
        }
    }

    async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}

    fn providers(&self) -> Vec<ProviderId> {
        vec![
            self.provider.id().clone(),
            ProviderId::new("not-supporting-model"),
        ]
    }
}

#[tokio::test]
async fn local_backpressure_survives_router_exhaustion_after_exclusion() {
    let (provider, holder) = busy_limited_test_provider().await;
    let router = Arc::new(ExhaustingAfterExcludedRouter { provider });
    let usecase = RouteRequest::new(
        router,
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    );
    let mut req = request();
    req.metadata.cacheable = false;

    let error = usecase
        .execute(req)
        .await
        .expect_err("local backpressure should survive provider selection exhaustion");
    holder.abort();
    assert!(
        error.is_local_backpressure(),
        "expected local 429 classification, got {error}"
    );
}

struct PriorityFallbackRouter {
    providers: Vec<Arc<dyn ProviderPort>>,
}

#[async_trait]
impl RouterPort for PriorityFallbackRouter {
    async fn select(&self, req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        self.select_excluding(req, &[]).await
    }

    async fn select_excluding(
        &self,
        req: &CompletionRequest,
        excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        self.providers
            .iter()
            .find(|provider| {
                !excluded.contains(provider.id()) && provider.supports_model(&req.model)
            })
            .cloned()
            .ok_or_else(CortexError::all_providers_exhausted)
    }

    async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}

    fn providers(&self) -> Vec<ProviderId> {
        self.providers
            .iter()
            .map(|provider| provider.id().clone())
            .collect()
    }
}

fn usecase_with_router(router: Arc<dyn RouterPort>) -> RouteRequest {
    RouteRequest::new(
        router,
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
}

fn ordinary_provider(id: &str) -> Arc<dyn ProviderPort> {
    Arc::new(TestProvider {
        id: ProviderId::new(id),
        complete_error: None,
    })
}

fn counting_stream_provider(id: &str) -> Arc<CountingStreamProvider> {
    Arc::new(CountingStreamProvider {
        id: ProviderId::new(id),
        models: vec![TEST_MODEL.clone()],
        stream_calls: std::sync::atomic::AtomicUsize::new(0),
    })
}

struct CountingStreamProvider {
    id: ProviderId,
    models: Vec<ModelId>,
    stream_calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ProviderPort for CountingStreamProvider {
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
            latency_ms: 0,
        }
    }
    async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        Err(CortexError::provider("unused fallback complete path"))
    }
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        self.stream_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Box::pin(futures::stream::iter(vec![Ok(StreamChunk {
            id: RequestId::new(),
            model: req.model.clone(),
            delta: "fallback stream".to_string(),
            thinking: None,
            tool_calls: Vec::new(),
            finish_reason: Some(rook_core::FinishReason::Stop),
            usage: None,
        })])))
    }
}

#[tokio::test]
async fn stream_falls_back_when_primary_provider_is_at_concurrency_limit() {
    let (primary, holder) = busy_limited_test_provider().await;
    let secondary = counting_stream_provider("secondary");
    let router = Arc::new(PriorityFallbackRouter {
        providers: vec![primary, secondary.clone()],
    });
    let usecase = usecase_with_router(router);
    let mut req = request();
    req.stream = true;

    let mut stream = usecase
        .execute_stream(req)
        .await
        .expect("idle secondary provider should accept the stream");
    let chunk = stream.next().await.unwrap().unwrap();
    holder.abort();

    assert_eq!(chunk.delta, "fallback stream");
    assert_eq!(
        secondary
            .stream_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn execute_reaches_fifth_allowed_provider_after_skipping_restricted_candidates() {
    let (primary, holder) = busy_limited_test_provider().await;
    let mut providers = vec![primary];
    providers.extend([
        ordinary_provider("disallowed-2"),
        ordinary_provider("disallowed-3"),
        ordinary_provider("disallowed-4"),
        ordinary_provider("allowed-5"),
    ]);
    let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter { providers }));
    let mut req = request();
    req.metadata.cacheable = false;
    req.restrictions.allowed_providers = vec![
        ProviderId::new("test-provider"),
        ProviderId::new("allowed-5"),
    ];

    let response = usecase
        .execute(req)
        .await
        .expect("restricted skips must not consume provider-call attempts");
    holder.abort();

    assert_eq!(response.provider, ProviderId::new("allowed-5"));
}

#[tokio::test]
async fn execute_stream_reaches_fifth_allowed_provider_after_skipping_restricted_candidates() {
    let (primary, holder) = busy_limited_test_provider().await;
    let disallowed = [
        counting_stream_provider("disallowed-2"),
        counting_stream_provider("disallowed-3"),
        counting_stream_provider("disallowed-4"),
    ];
    let allowed_fifth = counting_stream_provider("allowed-5");
    let mut providers: Vec<Arc<dyn ProviderPort>> = vec![primary];
    providers.extend(
        disallowed
            .into_iter()
            .map(|provider| provider as Arc<dyn ProviderPort>),
    );
    providers.push(allowed_fifth.clone());
    let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter { providers }));
    let mut req = request();
    req.stream = true;
    req.restrictions.allowed_providers = vec![
        ProviderId::new("test-provider"),
        ProviderId::new("allowed-5"),
    ];

    let mut stream = usecase
        .execute_stream(req)
        .await
        .expect("restricted skips must not consume provider-call attempts");
    let chunk = stream.next().await.unwrap().unwrap();
    holder.abort();

    assert_eq!(chunk.delta, "fallback stream");
    assert_eq!(
        allowed_fifth
            .stream_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn execute_preserves_local_backpressure_when_fallback_is_disallowed() {
    let (primary, holder) = busy_limited_test_provider().await;
    let secondary: Arc<dyn ProviderPort> = Arc::new(CountingStreamProvider {
        id: ProviderId::new("secondary"),
        models: vec![TEST_MODEL.clone()],
        stream_calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let usecase = RouteRequest::new(
        Arc::new(PriorityFallbackRouter {
            providers: vec![primary, secondary],
        }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    );
    let mut req = request();
    req.stream = false;
    req.metadata.cacheable = false;
    req.restrictions.allowed_providers = vec![ProviderId::new("test-provider")];

    let error = usecase
        .execute(req)
        .await
        .expect_err("disallowed fallback must not mask local backpressure");
    holder.abort();
    assert!(
        error.is_local_backpressure(),
        "expected local 429 classification, got {error}"
    );
}

#[tokio::test]
async fn execute_stream_preserves_local_backpressure_when_fallback_is_disallowed() {
    let (primary, holder) = busy_limited_test_provider().await;
    let secondary = Arc::new(CountingStreamProvider {
        id: ProviderId::new("secondary"),
        models: vec![TEST_MODEL.clone()],
        stream_calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let usecase = RouteRequest::new(
        Arc::new(PriorityFallbackRouter {
            providers: vec![primary, secondary],
        }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    );
    let mut req = request();
    req.stream = true;
    req.restrictions.allowed_providers = vec![ProviderId::new("test-provider")];

    let error = match usecase.execute_stream(req).await {
        Err(error) => error,
        Ok(_) => panic!("disallowed fallback must not mask local backpressure"),
    };
    holder.abort();
    assert!(
        error.is_local_backpressure(),
        "expected local 429 classification, got {error}"
    );
}

#[tokio::test]
async fn execute_combo_returns_local_backpressure_when_all_admission_is_busy() {
    let (provider, holder) = busy_limited_test_provider().await;
    let usecase = make_usecase_with_provider(provider);
    let combo = Combo::new(
        "busy".to_string(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: ProviderId::new("test-provider"),
            model: TEST_MODEL.clone(),
            connection_id: None,
            priority: 1,
        }],
    );
    let mut req = request();
    req.stream = false;
    req.metadata.cacheable = false;

    let error = usecase
        .execute_combo(&combo, req, ApiFormat::OpenAI)
        .await
        .expect_err("busy combo provider rejects admission");
    holder.abort();
    assert!(
        error.is_local_backpressure(),
        "expected local 429 classification, got {error}"
    );
}

struct TestCache {
    get_calls: Mutex<u32>,
    set_calls: Mutex<u32>,
}

#[async_trait]
impl CachePort for TestCache {
    async fn get(&self, _key: &CacheKey) -> CortexResult<Option<CompletionResponse>> {
        *self.get_calls.lock().unwrap() += 1;
        Ok(None)
    }

    async fn set(
        &self,
        _key: &CacheKey,
        _value: &CompletionResponse,
        _ttl: Duration,
    ) -> CortexResult<()> {
        *self.set_calls.lock().unwrap() += 1;
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

struct TestAudit {
    entries: Mutex<Vec<AuditEntry>>,
}

#[async_trait]
impl AuditPort for TestAudit {
    async fn record(&self, entry: AuditEntry) -> CortexResult<()> {
        self.entries.lock().unwrap().push(entry);
        Ok(())
    }
}

struct TestUsageRecorder {
    entries: Mutex<Vec<UsageEntry>>,
    fail_recording: bool,
}

#[async_trait]
impl UsageRecorderPort for TestUsageRecorder {
    async fn record(&self, entry: UsageEntry) -> CortexResult<()> {
        if self.fail_recording {
            return Err(CortexError::provider("usage recording failed"));
        }
        self.entries.lock().unwrap().push(entry);
        Ok(())
    }

    async fn list(
        &self,
        _filters: UsageFilters,
        _pagination: Pagination,
    ) -> CortexResult<Vec<UsageEntry>> {
        Ok(vec![])
    }

    async fn count(&self, _filters: UsageFilters) -> CortexResult<u64> {
        Ok(0)
    }

    async fn summary(&self, _filters: UsageFilters) -> CortexResult<UsageSummary> {
        Ok(UsageSummary::default())
    }

    async fn cost_breakdown(&self, _filters: UsageFilters) -> CortexResult<CostBreakdown> {
        Ok(CostBreakdown::default())
    }
}

struct TestProviderRepository {
    connection_id: Option<ConnectionId>,
    fail_lookup: bool,
}

#[async_trait]
impl ProviderRepositoryPort for TestProviderRepository {
    async fn list(&self) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(vec![])
    }

    async fn find(
        &self,
        _id: &ConnectionId,
    ) -> Result<Option<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(None)
    }

    async fn find_connection_id_by_runtime(
        &self,
        _provider: &ProviderId,
    ) -> Result<Option<ConnectionId>, rook_core::RepositoryError> {
        if self.fail_lookup {
            Err(rook_core::RepositoryError::Database(
                "lookup failed".to_string(),
            ))
        } else {
            Ok(self.connection_id)
        }
    }

    async fn create(
        &self,
        _conn: &rook_core::ProviderConnection,
    ) -> Result<(), rook_core::RepositoryError> {
        Ok(())
    }

    async fn update(
        &self,
        _conn: &rook_core::ProviderConnection,
        _expected_updated_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), rook_core::RepositoryError> {
        Ok(())
    }

    async fn delete(&self, _id: &ConnectionId) -> Result<(), rook_core::RepositoryError> {
        Ok(())
    }
}

/// Test stub for ModelAliasRepositoryPort — returns no aliases
struct TestAliasRepository;

#[async_trait]
impl ModelAliasRepositoryPort for TestAliasRepository {
    async fn find_by_alias(
        &self,
        _alias: &shared_kernel::ModelId,
        _provider_id: Option<&ProviderId>,
    ) -> Result<Option<rook_core::ModelAlias>, rook_core::ModelAliasRepositoryError> {
        Ok(None) // No aliases in tests by default
    }

    async fn list(
        &self,
    ) -> Result<Vec<rook_core::ModelAlias>, rook_core::ModelAliasRepositoryError> {
        Ok(vec![])
    }

    async fn create(
        &self,
        _alias: rook_core::ModelAlias,
    ) -> Result<(), rook_core::ModelAliasRepositoryError> {
        Ok(())
    }

    async fn delete(
        &self,
        _alias: &shared_kernel::ModelId,
    ) -> Result<bool, rook_core::ModelAliasRepositoryError> {
        Ok(false)
    }

    async fn seed(
        &self,
        _aliases: Vec<rook_core::ModelAlias>,
    ) -> Result<usize, rook_core::ModelAliasRepositoryError> {
        Ok(0)
    }
}

fn test_alias_config() -> ModelAliasesConfig {
    ModelAliasesConfig {
        enabled: false, // Disabled by default in tests
        auto_seed: false,
    }
}

struct FailingProviderRepository;

#[async_trait]
impl ProviderRepositoryPort for FailingProviderRepository {
    async fn list(&self) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(vec![])
    }

    async fn find(
        &self,
        _id: &ConnectionId,
    ) -> Result<Option<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(None)
    }

    async fn find_connection_id_by_runtime(
        &self,
        _provider: &ProviderId,
    ) -> Result<Option<ConnectionId>, rook_core::RepositoryError> {
        Err(rook_core::RepositoryError::Database(
            "lookup failed".to_string(),
        ))
    }

    async fn create(
        &self,
        _conn: &rook_core::ProviderConnection,
    ) -> Result<(), rook_core::RepositoryError> {
        Ok(())
    }

    async fn update(
        &self,
        _conn: &rook_core::ProviderConnection,
        _expected_updated_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), rook_core::RepositoryError> {
        Ok(())
    }

    async fn delete(&self, _id: &ConnectionId) -> Result<(), rook_core::RepositoryError> {
        Ok(())
    }
}

struct TestFormatTranslator;

impl FormatTranslatorPort for TestFormatTranslator {
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

static TEST_MODEL: std::sync::LazyLock<ModelId> =
    std::sync::LazyLock::new(|| ModelId::new("gpt-test"));

fn request() -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: TEST_MODEL.clone(),
        messages: vec![Message {
            tool_calls: vec![],
            role: Role::User,
            content: "hello".into(),
        }],
        stream: true,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".to_string(),
            cacheable: true,
            priority: 1,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    }
}

#[tokio::test]
async fn execute_stream_bypasses_cache_and_audits_final_usage() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
    });
    let cache = Arc::new(TestCache {
        get_calls: Mutex::new(0),
        set_calls: Mutex::new(0),
    });
    let audit = Arc::new(TestAudit {
        entries: Mutex::new(Vec::new()),
    });
    let usecase = RouteRequest::new(
        Arc::new(TestRouter { provider }),
        cache.clone(),
        audit.clone(),
        None,
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None, // telemetry
    );

    let mut stream = usecase
        .execute_stream(request())
        .await
        .expect("stream starts");
    let chunks: Vec<_> = stream
        .by_ref()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("stream chunks succeed");

    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.delta.as_str())
            .collect::<String>(),
        "hello"
    );
    assert_eq!(*cache.get_calls.lock().unwrap(), 0);
    assert_eq!(*cache.set_calls.lock().unwrap(), 0);

    let entries = audit.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, RequestStatus::Success);
    assert_eq!(entries[0].usage.as_ref().unwrap().total_tokens, 5);
}

fn make_usecase() -> RouteRequest {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
    });
    RouteRequest::new(
        Arc::new(TestRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None, // telemetry
    )
}

struct AliasLookupSpy {
    called: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl ModelAliasRepositoryPort for AliasLookupSpy {
    async fn find_by_alias(
        &self,
        alias: &ModelId,
        _provider_id: Option<&ProviderId>,
    ) -> Result<Option<ModelAlias>, ModelAliasRepositoryError> {
        assert_eq!(alias.as_str(), "gpt-4-turbo");
        self.called.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(Some(ModelAlias {
            alias: ModelId::new("gpt-4-turbo"),
            canonical: ModelId::new("gpt-4-turbo-2024-04-09"),
            provider_id: None,
            created_at: Utc::now(),
        }))
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

struct AliasOrderingComboRepository {
    alias_lookup_called: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl ComboRepositoryPort for AliasOrderingComboRepository {
    async fn list(&self) -> Result<Vec<Combo>, ComboRepositoryError> {
        Ok(vec![])
    }

    async fn find(&self, _id: &ComboId) -> Result<Option<Combo>, ComboRepositoryError> {
        assert!(
            self.alias_lookup_called
                .load(std::sync::atomic::Ordering::SeqCst),
            "combo dispatch must happen after model alias resolution"
        );
        Ok(None)
    }

    async fn find_by_name(&self, _name: &str) -> Result<Option<Combo>, ComboRepositoryError> {
        Ok(None)
    }

    async fn create(&self, _combo: &Combo) -> Result<(), ComboRepositoryError> {
        Ok(())
    }

    async fn update(&self, _combo: &Combo) -> Result<(), ComboRepositoryError> {
        Ok(())
    }

    async fn delete(&self, _id: &ComboId) -> Result<(), ComboRepositoryError> {
        Ok(())
    }
}

#[tokio::test]
async fn execute_with_format_resolves_alias_before_combo_dispatch() {
    let alias_lookup_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
    });
    let usecase = RouteRequest::new(
        Arc::new(TestRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        None,
        Some(Arc::new(AliasOrderingComboRepository {
            alias_lookup_called: alias_lookup_called.clone(),
        })),
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(AliasLookupSpy {
            called: alias_lookup_called.clone(),
        }),
        ModelAliasesConfig {
            enabled: true,
            auto_seed: false,
        },
        None,
    );
    let mut req = request();
    req.model = ModelId::new("gpt-4-turbo");
    req.metadata.combo_id = Some(ComboId::new());

    // The combo is intentionally absent; reaching its lookup proves the
    // alias lookup happened first. The expected not-found is incidental.
    let result = usecase.execute_with_format(req, ApiFormat::OpenAI).await;

    assert!(result.is_err());
    assert!(alias_lookup_called.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn route_request_runs_with_nullable_usage_recorder_and_warns_through_connection_lookup_failure(
) {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
    });
    let usecase = RouteRequest::new(
        Arc::new(TestRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        Some(Arc::new(FailingProviderRepository)),
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None, // telemetry
    );

    let result = usecase.execute(request()).await;

    assert!(
        result.is_ok(),
        "lookup failure should not fail routing: {result:?}"
    );
}

#[tokio::test]
async fn non_stream_usage_records_success_with_latency_as_ttft_and_cost_metadata() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
    });
    let usage_recorder = Arc::new(TestUsageRecorder {
        entries: Mutex::new(Vec::new()),
        fail_recording: false,
    });
    let connection_id = ConnectionId::new();
    let mut pricing = crate::PricingConfig::default();
    pricing.providers.insert(
        "test-provider".to_string(),
        HashMap::from([(
            TEST_MODEL.as_str().to_string(),
            crate::PricingEntry {
                prompt_per_million: 1.0,
                completion_per_million: 2.0,
                cache_read_per_million: None,
                cache_creation_per_million: None,
                reasoning_per_million: None,
            },
        )]),
    );
    let usecase = RouteRequest::new(
        Arc::new(TestRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        Some(usage_recorder.clone()),
        Some(Arc::new(TestProviderRepository {
            connection_id: Some(connection_id),
            fail_lookup: false,
        })),
        None,
        Arc::new(pricing),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None, // telemetry
    );
    let mut req = request();
    req.metadata.api_key_id = Some(ApiKeyId::new("key_123"));
    req.metadata.requested_tier = Some("premium".to_string());

    let response = usecase.execute(req.clone()).await.expect("success");

    assert_eq!(response.content, "cached path");
    let entries = usage_recorder.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.request_id, req.id);
    assert_eq!(entry.provider, ProviderId::new("test-provider"));
    assert_eq!(entry.model, TEST_MODEL.clone());
    assert_eq!(entry.status, RequestStatus::Success);
    assert_eq!(entry.requested_tier.as_deref(), Some("premium"));
    assert_eq!(entry.api_key_id, Some(ApiKeyId::new("key_123")));
    assert_eq!(entry.connection_id, Some(connection_id));
    assert_eq!(entry.tokens_prompt, Some(1));
    assert_eq!(entry.tokens_completion, Some(1));
    assert_eq!(entry.tokens_cache_read, None);
    assert_eq!(entry.tokens_cache_creation, None);
    assert_eq!(entry.tokens_reasoning, None);
    assert_eq!(entry.ttft_ms, Some(entry.latency_ms));
    assert_eq!(entry.cost_usd, Some(0.000003));
}

#[tokio::test]
async fn non_stream_usage_records_failure_with_nullable_tokens_and_ttft() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: Some(CortexError::provider("provider failed")),
    });
    let usage_recorder = Arc::new(TestUsageRecorder {
        entries: Mutex::new(Vec::new()),
        fail_recording: false,
    });
    let usecase = RouteRequest::new(
        Arc::new(TestRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        Some(usage_recorder.clone()),
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None, // telemetry
    );
    let req = request();

    let result = usecase.execute(req.clone()).await;

    assert!(result.is_err());
    let entries = usage_recorder.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.request_id, req.id);
    assert_eq!(entry.status, RequestStatus::Failure);
    assert_eq!(entry.tokens_prompt, None);
    assert_eq!(entry.tokens_completion, None);
    assert_eq!(entry.tokens_cache_read, None);
    assert_eq!(entry.tokens_cache_creation, None);
    assert_eq!(entry.tokens_reasoning, None);
    assert_eq!(entry.ttft_ms, None);
    assert_eq!(entry.cost_usd, None);
}

#[tokio::test]
async fn non_stream_usage_recording_failure_does_not_fail_successful_client_response() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
    });
    let usage_recorder = Arc::new(TestUsageRecorder {
        entries: Mutex::new(Vec::new()),
        fail_recording: true,
    });
    let usecase = RouteRequest::new(
        Arc::new(TestRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        Some(usage_recorder.clone()),
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None, // telemetry
    );

    let result = usecase.execute(request()).await;

    assert!(
        result.is_ok(),
        "usage recorder failures should not fail response: {result:?}"
    );
    assert!(usage_recorder.entries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn non_stream_usage_missing_pricing_records_null_cost_not_zero() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
    });
    let usage_recorder = Arc::new(TestUsageRecorder {
        entries: Mutex::new(Vec::new()),
        fail_recording: false,
    });
    let usecase = RouteRequest::new(
        Arc::new(TestRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        Some(usage_recorder.clone()),
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None, // telemetry
    );

    let result = usecase.execute(request()).await;

    assert!(result.is_ok());
    let entries = usage_recorder.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].cost_usd, None);
}

#[tokio::test]
async fn execute_is_forbidden_when_model_not_in_allowed_list() {
    let usecase = make_usecase();
    let mut req = request();
    req.restrictions.allowed_models = vec![ModelId::new("gpt-4"), ModelId::new("claude-3-opus")];
    // TEST_MODEL is "gpt-test" — not in the allowed list

    let result = usecase.execute(req).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.is_forbidden(), "expected forbidden, got: {err}");
}

#[tokio::test]
async fn execute_succeeds_when_model_is_in_allowed_list() {
    let usecase = make_usecase();
    let mut req = request();
    req.restrictions.allowed_models = vec![TEST_MODEL.clone()];

    let result = usecase.execute(req).await;
    assert!(result.is_ok(), "expected success, got: {:?}", result.err());
}

#[tokio::test]
async fn execute_succeeds_when_allowed_models_is_empty() {
    let usecase = make_usecase();
    // default restrictions — empty = unrestricted
    let result = usecase.execute(request()).await;
    assert!(result.is_ok(), "expected success, got: {:?}", result.err());
}

#[tokio::test]
async fn execute_is_forbidden_when_provider_not_in_allowed_list() {
    let usecase = make_usecase();
    let mut req = request();
    // test-provider is the provider selected; restrict to a different one
    req.restrictions.allowed_providers = vec![ProviderId::new("anthropic")];

    let result = usecase.execute(req).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.is_forbidden(), "expected forbidden, got: {err}");
}

#[tokio::test]
async fn execute_succeeds_when_provider_is_in_allowed_list() {
    let usecase = make_usecase();
    let mut req = request();
    req.restrictions.allowed_providers = vec![ProviderId::new("test-provider")];

    let result = usecase.execute(req).await;
    assert!(result.is_ok(), "expected success, got: {:?}", result.err());
}

#[tokio::test]
async fn execute_stream_is_forbidden_when_model_not_allowed() {
    let usecase = make_usecase();
    let mut req = request();
    req.restrictions.allowed_models = vec![ModelId::new("gpt-4")];

    let result = usecase.execute_stream(req).await;
    assert!(result.is_err());
    assert!(result.err().unwrap().is_forbidden());
}

#[tokio::test]
async fn execute_stream_is_forbidden_when_provider_not_allowed() {
    let usecase = make_usecase();
    let mut req = request();
    req.restrictions.allowed_providers = vec![ProviderId::new("openai")];

    let result = usecase.execute_stream(req).await;
    assert!(result.is_err());
    assert!(result.err().unwrap().is_forbidden());
}

#[tokio::test]
async fn streaming_usage_records_ttft_from_first_chunk_and_final_usage_on_success() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
    });
    let usage_recorder = Arc::new(TestUsageRecorder {
        entries: Mutex::new(Vec::new()),
        fail_recording: false,
    });
    let connection_id = ConnectionId::new();
    let mut pricing = crate::PricingConfig::default();
    pricing.providers.insert(
        "test-provider".to_string(),
        HashMap::from([(
            TEST_MODEL.as_str().to_string(),
            crate::PricingEntry {
                prompt_per_million: 1.0,
                completion_per_million: 2.0,
                cache_read_per_million: None,
                cache_creation_per_million: None,
                reasoning_per_million: None,
            },
        )]),
    );
    let usecase = RouteRequest::new(
        Arc::new(TestRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        Some(usage_recorder.clone()),
        Some(Arc::new(TestProviderRepository {
            connection_id: Some(connection_id),
            fail_lookup: false,
        })),
        None,
        Arc::new(pricing),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None, // telemetry
    );
    let mut req = request();
    req.metadata.api_key_id = Some(ApiKeyId::new("key_streaming"));

    let stream = usecase
        .execute_stream(req.clone())
        .await
        .expect("stream starts");
    let _chunks: Vec<_> = stream.collect::<Vec<_>>().await;

    let entries = usage_recorder.entries.lock().unwrap();
    assert_eq!(entries.len(), 1, "should record one usage entry");
    let entry = &entries[0];
    assert_eq!(entry.request_id, req.id);
    assert_eq!(entry.status, RequestStatus::Success);
    assert_eq!(entry.tokens_prompt, Some(2));
    assert_eq!(entry.tokens_completion, Some(3));
    assert!(
        entry.ttft_ms.is_some(),
        "ttft_ms should be captured from first chunk"
    );
    assert!(
        entry.ttft_ms.unwrap() <= entry.latency_ms,
        "ttft should be <= total latency"
    );
    assert_eq!(entry.api_key_id, Some(ApiKeyId::new("key_streaming")));
    assert_eq!(entry.connection_id, Some(connection_id));
    assert!(entry.cost_usd.is_some(), "cost should be calculated");
}

#[tokio::test]
async fn streaming_usage_records_failure_with_nullable_tokens_and_rate_limited_status() {
    struct FailingStreamProvider {
        id: ProviderId,
    }

    #[async_trait]
    impl ProviderPort for FailingStreamProvider {
        fn id(&self) -> &ProviderId {
            &self.id
        }

        fn supported_models(&self) -> &[ModelId] {
            std::slice::from_ref(&TEST_MODEL)
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
                latency_ms: 1,
            }
        }

        async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
            Err(CortexError::rate_limited(self.id.clone(), 60))
        }

        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
            Ok(Box::pin(stream::iter(vec![
                Ok(StreamChunk {
                    id: RequestId::new(),
                    model: TEST_MODEL.clone(),
                    delta: "start".to_string(),
                    thinking: None,
                    tool_calls: vec![],
                    finish_reason: None,
                    usage: None,
                }),
                Err(CortexError::rate_limited(self.id.clone(), 60)),
            ])))
        }
    }

    let provider: Arc<dyn ProviderPort> = Arc::new(FailingStreamProvider {
        id: ProviderId::new("test-provider"),
    });
    let usage_recorder = Arc::new(TestUsageRecorder {
        entries: Mutex::new(Vec::new()),
        fail_recording: false,
    });
    let usecase = RouteRequest::new(
        Arc::new(TestRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        Some(usage_recorder.clone()),
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None, // telemetry
    );

    let mut stream = usecase
        .execute_stream(request())
        .await
        .expect("stream starts");
    let _first_chunk = stream.next().await.expect("first chunk").expect("ok");
    let second_result = stream.next().await.expect("second item");
    assert!(second_result.is_err(), "second chunk should error");

    let entries = usage_recorder.entries.lock().unwrap();
    assert_eq!(entries.len(), 1, "should record failure");
    let entry = &entries[0];
    assert_eq!(entry.status, RequestStatus::RateLimited);
    assert_eq!(entry.tokens_prompt, None);
    assert_eq!(entry.tokens_completion, None);
    assert!(
        entry.ttft_ms.is_some(),
        "ttft captured before failure from first chunk"
    );
    assert_eq!(entry.cost_usd, None);
}

#[tokio::test]
async fn streaming_usage_recording_failure_emits_warn_and_increments_metric_without_aborting_stream(
) {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
    });
    let usage_recorder = Arc::new(TestUsageRecorder {
        entries: Mutex::new(Vec::new()),
        fail_recording: true,
    });
    let usecase = RouteRequest::new(
        Arc::new(TestRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        Some(usage_recorder.clone()),
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None, // telemetry
    );

    let stream = usecase
        .execute_stream(request())
        .await
        .expect("stream starts");
    let chunks: Vec<_> = stream.collect::<Vec<_>>().await;

    assert_eq!(
        chunks.len(),
        2,
        "stream should complete despite recording failure"
    );
    assert!(
        chunks.iter().all(|c| c.is_ok()),
        "all chunks should succeed"
    );
    assert!(
        usage_recorder.entries.lock().unwrap().is_empty(),
        "recording should have failed"
    );
}

// ------------------------------------------------------------------
// R2 virtual combo routing (Task 2 IO)
// ------------------------------------------------------------------

struct SpyProvider {
    id: ProviderId,
    model: ModelId,
    seen_models: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ProviderPort for SpyProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn supported_models(&self) -> &[ModelId] {
        std::slice::from_ref(&self.model)
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
            latency_ms: 1,
        }
    }

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        self.seen_models.lock().unwrap().push(req.model.to_string());
        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.id.clone(),
            model: req.model.clone(),
            content: "combo says hi".to_string(),
            content_blocks: vec![rook_core::MessageContent::Text("combo says hi".to_string())],
            thinking: None,
            tool_calls: vec![],
            finish_reason: None,
            usage: TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 1,
            cache_hit: None,
        })
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        self.seen_models.lock().unwrap().push(req.model.to_string());
        Ok(Box::pin(futures::stream::iter(vec![Ok(StreamChunk {
            id: RequestId::new(),
            model: req.model.clone(),
            delta: "spy stream".to_string(),
            thinking: None,
            tool_calls: Vec::new(),
            finish_reason: Some(rook_core::FinishReason::Stop),
            usage: None,
        })])))
    }
}

struct MapComboRepository {
    combos: HashMap<ComboId, Combo>,
}

/// Provider that always fails with a rate-limit error, used to prove the
/// combo fallback policy still advances to the next step.
struct RateLimitedProvider {
    id: ProviderId,
}

#[async_trait]
impl ProviderPort for RateLimitedProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn supported_models(&self) -> &[ModelId] {
        std::slice::from_ref(&TEST_MODEL)
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
            latency_ms: 1,
        }
    }

    async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        Err(CortexError::rate_limited(self.id.clone(), 30))
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        unreachable!("combo tests use complete()")
    }
}

struct MapProviderRepository {
    connections: HashMap<ConnectionId, rook_core::ProviderConnection>,
}

#[async_trait]
impl ProviderRepositoryPort for MapProviderRepository {
    async fn list(&self) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(self.connections.values().cloned().collect())
    }

    async fn find(
        &self,
        id: &ConnectionId,
    ) -> Result<Option<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(self.connections.get(id).cloned())
    }

    async fn create(
        &self,
        _connection: &rook_core::ProviderConnection,
    ) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }

    async fn update(
        &self,
        _connection: &rook_core::ProviderConnection,
        _expected_updated_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }

    async fn delete(&self, _id: &ConnectionId) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }
}

#[async_trait]
impl ComboRepositoryPort for MapComboRepository {
    async fn list(&self) -> Result<Vec<Combo>, ComboRepositoryError> {
        Ok(self.combos.values().cloned().collect())
    }

    async fn find(&self, id: &ComboId) -> Result<Option<Combo>, ComboRepositoryError> {
        Ok(self.combos.get(id).cloned())
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Combo>, ComboRepositoryError> {
        Ok(self.combos.values().find(|c| c.name == name).cloned())
    }

    async fn create(&self, _combo: &Combo) -> Result<(), ComboRepositoryError> {
        Ok(())
    }

    async fn update(&self, _combo: &Combo) -> Result<(), ComboRepositoryError> {
        Ok(())
    }

    async fn delete(&self, _id: &ComboId) -> Result<(), ComboRepositoryError> {
        Ok(())
    }
}

struct MapComboAliasRepository {
    aliases: HashMap<String, ComboAlias>,
}

#[async_trait]
impl ComboAliasRepositoryPort for MapComboAliasRepository {
    async fn find_by_alias(
        &self,
        alias: &ModelId,
    ) -> Result<Option<ComboAlias>, ComboAliasRepositoryError> {
        Ok(self.aliases.get(alias.as_str()).cloned())
    }

    async fn list(&self) -> Result<Vec<ComboAlias>, ComboAliasRepositoryError> {
        Ok(self.aliases.values().cloned().collect())
    }

    async fn create(&self, _alias: ComboAlias) -> Result<(), ComboAliasRepositoryError> {
        Ok(())
    }

    async fn delete(&self, _alias: &ModelId) -> Result<bool, ComboAliasRepositoryError> {
        Ok(false)
    }
}

struct ComboHarness {
    usecase: RouteRequest,
    seen_models: Arc<Mutex<Vec<String>>>,
    audit: Arc<TestAudit>,
    usage: Arc<TestUsageRecorder>,
}

struct RecordingRouter {
    provider: Arc<dyn ProviderPort>,
}

struct OrderedRouter {
    providers: Vec<Arc<dyn ProviderPort>>,
    connection_providers: HashMap<ConnectionId, Arc<dyn ProviderPort>>,
}

#[async_trait]
impl RouterPort for OrderedRouter {
    async fn select(&self, _req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        self.providers
            .first()
            .cloned()
            .ok_or_else(CortexError::all_providers_exhausted)
    }

    async fn select_excluding(
        &self,
        _req: &CompletionRequest,
        excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        self.providers
            .iter()
            .find(|provider| !excluded.contains(provider.id()))
            .cloned()
            .ok_or_else(CortexError::all_providers_exhausted)
    }

    async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}

    fn providers(&self) -> Vec<ProviderId> {
        self.providers
            .iter()
            .map(|provider| provider.id().clone())
            .collect()
    }

    fn resolve_connection(&self, connection_id: &ConnectionId) -> Option<Arc<dyn ProviderPort>> {
        self.connection_providers.get(connection_id).cloned()
    }
}

#[async_trait]
impl RouterPort for RecordingRouter {
    async fn select(&self, _req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        Ok(self.provider.clone())
    }
    async fn select_excluding(
        &self,
        _req: &CompletionRequest,
        _excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        Ok(self.provider.clone())
    }
    async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}
    fn providers(&self) -> Vec<ProviderId> {
        vec![self.provider.id().clone()]
    }
}

fn combo_harness_with_router(
    combo: Combo,
    aliases: Vec<(&str, ComboId)>,
    router: Arc<dyn RouterPort>,
    seen_models: Arc<Mutex<Vec<String>>>,
) -> ComboHarness {
    combo_harness_with_repositories(combo, aliases, router, seen_models, None)
}

fn combo_harness_with_repositories(
    combo: Combo,
    aliases: Vec<(&str, ComboId)>,
    router: Arc<dyn RouterPort>,
    seen_models: Arc<Mutex<Vec<String>>>,
    provider_repository: Option<Arc<dyn ProviderRepositoryPort>>,
) -> ComboHarness {
    combo_harness_with_repositories_and_quota(
        combo,
        aliases,
        router,
        seen_models,
        provider_repository,
        None,
    )
}

fn combo_harness_with_repositories_and_quota(
    combo: Combo,
    aliases: Vec<(&str, ComboId)>,
    router: Arc<dyn RouterPort>,
    seen_models: Arc<Mutex<Vec<String>>>,
    provider_repository: Option<Arc<dyn ProviderRepositoryPort>>,
    quota_reader: Option<Arc<dyn QuotaObservationRecorderPort>>,
) -> ComboHarness {
    let combo_id = combo.id;
    let mut combos = HashMap::new();
    combos.insert(combo_id, combo);
    let alias_map: HashMap<String, ComboAlias> = aliases
        .into_iter()
        .map(|(a, id)| (a.to_string(), ComboAlias::new(ModelId::new(a), id)))
        .collect();
    let audit = Arc::new(TestAudit {
        entries: Mutex::new(Vec::new()),
    });
    let usage = Arc::new(TestUsageRecorder {
        entries: Mutex::new(Vec::new()),
        fail_recording: false,
    });
    let mut usecase = RouteRequest::new(
        router,
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        audit.clone(),
        Some(usage.clone()),
        provider_repository,
        Some(Arc::new(MapComboRepository { combos })),
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases: alias_map }));
    if let Some(reader) = quota_reader {
        usecase = usecase.with_quota_observation_reader(reader);
    }
    ComboHarness {
        usecase,
        seen_models,
        audit,
        usage,
    }
}

fn combo_harness(combo: Combo, aliases: Vec<(&str, ComboId)>) -> ComboHarness {
    let seen_models = Arc::new(Mutex::new(Vec::new()));
    let spy: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: ProviderId::new("openai"),
        model: TEST_MODEL.clone(),
        seen_models: seen_models.clone(),
    });
    combo_harness_with_router(
        combo,
        aliases,
        Arc::new(RecordingRouter { provider: spy }),
        seen_models,
    )
}

fn test_connection(
    id: ConnectionId,
    runtime_id: ProviderId,
    is_active: bool,
) -> rook_core::ProviderConnection {
    use rook_core::{
        AuthType, ConnectionConfig, Credentials, EncryptedBlob, ProviderKind,
        QuotaWindowThresholds, TestStatus,
    };

    let now = chrono::Utc::now();
    rook_core::ProviderConnection {
        id,
        provider_kind: ProviderKind::OpenAI,
        provider_runtime_id: runtime_id,
        name: "combo-test".to_string(),
        priority: 1,
        is_active,
        auth_type: AuthType::ApiKey,
        credentials: Credentials::ApiKey {
            api_key: EncryptedBlob("encrypted".to_string()),
        },
        config: ConnectionConfig {
            max_concurrent: 1,
            quota_window_thresholds: QuotaWindowThresholds {
                warning: 0.7,
                error: 0.9,
            },
            default_model: None,
            base_url: None,
            api_format: Some(rook_core::ApiFormat::OpenAI),
            supported_models: Vec::new(),
        },
        test_status: TestStatus::NeverTested,
        created_at: now,
        updated_at: now,
    }
}

fn combo_req(model: &str, restrictions: rook_core::ApiKeyRestrictions) -> CompletionRequest {
    let mut r = request();
    r.model = ModelId::new(model);
    r.stream = false;
    r.metadata.cacheable = false;
    r.restrictions = restrictions;
    r
}

fn step(provider: &str, model: &str, priority: u8) -> ComboStep {
    ComboStep {
        provider_id: ProviderId::new(provider),
        model: ModelId::new(model),
        connection_id: None,
        priority,
    }
}

fn harness_combo() -> Combo {
    Combo::new(
        "harness".to_string(),
        ComboStrategy::Priority,
        vec![step("openai", "gpt-4o", 1)],
    )
}

#[tokio::test]
async fn combo_step_with_connection_id_uses_exact_provider_not_router_selection() {
    let target_connection = ConnectionId::new();
    let adapter_id = ProviderId::new(target_connection.to_string());
    let runtime_id = ProviderId::new("openai-primary");
    let combo = Combo::new(
        "exact-connection".to_string(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: runtime_id.clone(),
            model: ModelId::new("gpt-4o"),
            connection_id: Some(target_connection),
            priority: 1,
        }],
    );
    let router_selected_calls = Arc::new(Mutex::new(Vec::new()));
    let exact_connection_calls = Arc::new(Mutex::new(Vec::new()));
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![Arc::new(SpyProvider {
            id: ProviderId::new("other-account"),
            model: ModelId::new("gpt-4o"),
            seen_models: router_selected_calls,
        })],
        connection_providers: HashMap::from([(
            target_connection,
            Arc::new(SpyProvider {
                id: adapter_id.clone(),
                model: ModelId::new("gpt-4o"),
                seen_models: exact_connection_calls.clone(),
            }) as Arc<dyn ProviderPort>,
        )]),
    });
    let connection = test_connection(target_connection, runtime_id.clone(), true);
    let provider_repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([(target_connection, connection)]),
    });
    let h = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(provider_repository),
    );

    let result = h
        .usecase
        .execute_combo(
            &combo,
            combo_req("gpt-4o", rook_core::ApiKeyRestrictions::default()),
            ApiFormat::OpenAI,
        )
        .await;

    assert!(
        result.is_ok(),
        "the configured connection should execute successfully: {result:?}"
    );
    assert_eq!(
        exact_connection_calls.lock().unwrap().as_slice(),
        &["gpt-4o".to_string()],
        "the exact configured connection must receive the request"
    );
    assert!(h.seen_models.lock().unwrap().is_empty());
}

// ------------------------------------------------------------------
// Upstream quota eligibility preflight
// ------------------------------------------------------------------

/// In-memory quota store used by the eligibility tests.
struct FakeQuotaStore {
    observations: Mutex<Vec<QuotaObservation>>,
}

impl FakeQuotaStore {
    fn new(observations: Vec<QuotaObservation>) -> Self {
        Self {
            observations: Mutex::new(observations),
        }
    }
}

#[async_trait]
impl QuotaObservationRecorderPort for FakeQuotaStore {
    async fn record_observation(&self, observation: QuotaObservation) -> CortexResult<()> {
        self.observations.lock().unwrap().push(observation);
        Ok(())
    }

    async fn list_observations(
        &self,
        provider: Option<ProviderId>,
    ) -> CortexResult<Vec<QuotaObservation>> {
        let observations = self.observations.lock().unwrap();
        Ok(observations
            .iter()
            .filter(|observation| match &provider {
                Some(provider) => &observation.provider == provider,
                None => true,
            })
            .cloned()
            .collect())
    }
}

struct PendingQuotaStore;

#[async_trait]
impl QuotaObservationRecorderPort for PendingQuotaStore {
    async fn record_observation(&self, _observation: QuotaObservation) -> CortexResult<()> {
        Ok(())
    }

    async fn list_observations(
        &self,
        _provider: Option<ProviderId>,
    ) -> CortexResult<Vec<QuotaObservation>> {
        Ok(Vec::new())
    }

    async fn latest_observations(
        &self,
        _query: &QuotaObservationQuery,
    ) -> CortexResult<Vec<QuotaObservation>> {
        std::future::pending().await
    }
}

/// Provider repository that also resolves the connection of a runtime.
struct QuotaAwareProviderRepository {
    connections: HashMap<ConnectionId, rook_core::ProviderConnection>,
    runtime_connections: HashMap<ProviderId, ConnectionId>,
}

#[async_trait]
impl ProviderRepositoryPort for QuotaAwareProviderRepository {
    async fn list(&self) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(self.connections.values().cloned().collect())
    }

    async fn find(
        &self,
        id: &ConnectionId,
    ) -> Result<Option<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(self.connections.get(id).cloned())
    }

    async fn find_connection_id_by_runtime(
        &self,
        provider: &ProviderId,
    ) -> Result<Option<ConnectionId>, rook_core::RepositoryError> {
        Ok(self.runtime_connections.get(provider).copied())
    }

    async fn create(
        &self,
        _conn: &rook_core::ProviderConnection,
    ) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }

    async fn update(
        &self,
        _conn: &rook_core::ProviderConnection,
        _expected_updated_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }

    async fn delete(&self, _id: &ConnectionId) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }
}

#[tokio::test]
async fn quota_read_timeout_keeps_connection_eligible() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("openai-pending-runtime");
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(QuotaAwareProviderRepository {
        connections: HashMap::from([(
            connection_id,
            test_connection(connection_id, runtime_id.clone(), true),
        )]),
        runtime_connections: HashMap::new(),
    });
    let provider_id = ProviderId::new(connection_id.to_string());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![Arc::new(SpyProvider {
            id: provider_id,
            model: TEST_MODEL.clone(),
            seen_models: calls.clone(),
        })],
        connection_providers: HashMap::new(),
    });
    let usecase = RouteRequest::new(
        router,
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        Some(repository),
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_quota_observation_reader(Arc::new(PendingQuotaStore));

    let mut req = request();
    req.metadata.cacheable = false;
    let result = tokio::time::timeout(
        QUOTA_READ_TIMEOUT + std::time::Duration::from_secs(1),
        usecase.execute(req),
    )
    .await
    .expect("quota timeout should bound request latency");
    assert!(result.is_ok(), "timeout must fail open: {result:?}");
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn uuid_provider_id_resolves_exact_connection_for_standard_and_streaming_requests() {
    let exhausted_connection = ConnectionId::new();
    let healthy_connection = ConnectionId::new();
    let exhausted_runtime = ProviderId::new("openai-exhausted-runtime");
    let healthy_runtime = ProviderId::new("openai-healthy-runtime");
    let exhausted_calls = Arc::new(Mutex::new(Vec::new()));
    let healthy_calls = Arc::new(Mutex::new(Vec::new()));
    let exhausted_provider = ProviderId::new(exhausted_connection.to_string());
    let healthy_provider = ProviderId::new(healthy_connection.to_string());

    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![
            Arc::new(SpyProvider {
                id: exhausted_provider.clone(),
                model: TEST_MODEL.clone(),
                seen_models: exhausted_calls.clone(),
            }),
            Arc::new(SpyProvider {
                id: healthy_provider.clone(),
                model: TEST_MODEL.clone(),
                seen_models: healthy_calls.clone(),
            }),
        ],
        connection_providers: HashMap::new(),
    });

    let provider_repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(QuotaAwareProviderRepository {
            connections: HashMap::from([
                (
                    exhausted_connection,
                    test_connection(exhausted_connection, exhausted_runtime.clone(), true),
                ),
                (
                    healthy_connection,
                    test_connection(healthy_connection, healthy_runtime.clone(), true),
                ),
            ]),
            runtime_connections: HashMap::new(),
        });

    let now = chrono::Utc::now();
    let quota_reader: Arc<dyn QuotaObservationRecorderPort> =
        Arc::new(FakeQuotaStore::new(vec![quota_observation(
            exhausted_connection,
            exhausted_runtime.clone(),
            0.0,
            now,
            Some(now + chrono::Duration::seconds(60)),
        )]));

    let usecase = RouteRequest::new(
        router,
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        Some(provider_repository),
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_quota_observation_reader(quota_reader);

    let mut req = request();
    req.stream = false;
    let result = usecase.execute_with_format(req, ApiFormat::OpenAI).await;

    assert!(
        result.is_ok(),
        "the healthy provider should serve the request: {result:?}"
    );
    assert!(
        exhausted_calls.lock().unwrap().is_empty(),
        "the exhausted connection must not reach the upstream provider"
    );
    assert_eq!(
        healthy_calls.lock().unwrap().as_slice(),
        &[TEST_MODEL.to_string()],
        "the healthy provider must receive the request"
    );
}

#[tokio::test]
async fn streaming_request_resolves_uuid_provider_id_before_quota_check() {
    let exhausted_connection = ConnectionId::new();
    let healthy_connection = ConnectionId::new();
    let exhausted_runtime = ProviderId::new("openai-exhausted-runtime");
    let healthy_runtime = ProviderId::new("openai-healthy-runtime");
    let exhausted_calls = Arc::new(Mutex::new(Vec::new()));
    let healthy_calls = Arc::new(Mutex::new(Vec::new()));
    let exhausted_provider = ProviderId::new(exhausted_connection.to_string());
    let healthy_provider = ProviderId::new(healthy_connection.to_string());
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![
            Arc::new(SpyProvider {
                id: exhausted_provider.clone(),
                model: TEST_MODEL.clone(),
                seen_models: exhausted_calls.clone(),
            }),
            Arc::new(SpyProvider {
                id: healthy_provider.clone(),
                model: TEST_MODEL.clone(),
                seen_models: healthy_calls.clone(),
            }),
        ],
        connection_providers: HashMap::new(),
    });
    let provider_repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(QuotaAwareProviderRepository {
            connections: HashMap::from([
                (
                    exhausted_connection,
                    test_connection(exhausted_connection, exhausted_runtime.clone(), true),
                ),
                (
                    healthy_connection,
                    test_connection(healthy_connection, healthy_runtime.clone(), true),
                ),
            ]),
            runtime_connections: HashMap::new(),
        });
    let now = chrono::Utc::now();
    let quota_reader: Arc<dyn QuotaObservationRecorderPort> =
        Arc::new(FakeQuotaStore::new(vec![quota_observation(
            exhausted_connection,
            exhausted_runtime,
            0.0,
            now,
            Some(now + chrono::Duration::seconds(60)),
        )]));
    let usecase = RouteRequest::new(
        router,
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        Some(provider_repository),
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_quota_observation_reader(quota_reader);

    let mut req = request();
    req.stream = true;
    let mut stream = usecase
        .execute_stream_with_format(req, ApiFormat::OpenAI)
        .await
        .expect("healthy provider should start streaming");
    while stream.next().await.is_some() {}

    assert!(exhausted_calls.lock().unwrap().is_empty());
    assert_eq!(
        healthy_calls.lock().unwrap().as_slice(),
        &[TEST_MODEL.to_string()],
    );
}

/// Skip reasons recorded by a failed combo.
fn combo_skip_reasons(error: &CortexError) -> Vec<String> {
    error
        .combo_skip_reasons()
        .map(|reasons| reasons.iter().map(|(_, reason)| reason.clone()).collect())
        .unwrap_or_default()
}

fn quota_observation(
    connection_id: ConnectionId,
    runtime_id: ProviderId,
    remaining: f64,
    observed_at: chrono::DateTime<chrono::Utc>,
    reset_at: Option<chrono::DateTime<chrono::Utc>>,
) -> QuotaObservation {
    QuotaObservation {
        provider_kind: rook_core::ProviderKind::OpenAI,
        provider: runtime_id,
        connection_id: Some(connection_id),
        metric: "requests".to_string(),
        source: "response_headers".to_string(),
        scope: None,
        window: None,
        limit: Some(100.0),
        remaining: Some(remaining),
        reset_at,
        reset_after_milliseconds: None,
        observed_at,
    }
}

/// Two connection-bound combo steps plus a quota store.
struct QuotaComboSetup {
    combo: Combo,
    router: Arc<dyn RouterPort>,
    provider_repository: Arc<dyn ProviderRepositoryPort>,
    quota_reader: Arc<dyn QuotaObservationRecorderPort>,
    first_calls: Arc<Mutex<Vec<String>>>,
    second_calls: Arc<Mutex<Vec<String>>>,
}

fn quota_combo_setup(
    remaining_first: Option<f64>,
    observed_at_offset_seconds: i64,
    reset_in_seconds: Option<i64>,
) -> QuotaComboSetup {
    let first_connection = ConnectionId::new();
    let second_connection = ConnectionId::new();
    let runtime_id = ProviderId::new("openai-primary");
    let first_calls = Arc::new(Mutex::new(Vec::new()));
    let second_calls = Arc::new(Mutex::new(Vec::new()));

    let combo = Combo::new(
        "quota-preflight".to_string(),
        ComboStrategy::Priority,
        vec![
            ComboStep {
                provider_id: runtime_id.clone(),
                model: ModelId::new("gpt-test"),
                connection_id: Some(first_connection),
                priority: 1,
            },
            ComboStep {
                provider_id: runtime_id.clone(),
                model: ModelId::new("gpt-test"),
                connection_id: Some(second_connection),
                priority: 2,
            },
        ],
    );

    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![],
        connection_providers: HashMap::from([
            (
                first_connection,
                Arc::new(SpyProvider {
                    id: ProviderId::new(first_connection.to_string()),
                    model: ModelId::new("gpt-test"),
                    seen_models: first_calls.clone(),
                }) as Arc<dyn ProviderPort>,
            ),
            (
                second_connection,
                Arc::new(SpyProvider {
                    id: ProviderId::new(second_connection.to_string()),
                    model: ModelId::new("gpt-test"),
                    seen_models: second_calls.clone(),
                }) as Arc<dyn ProviderPort>,
            ),
        ]),
    });

    let provider_repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([
            (
                first_connection,
                test_connection(first_connection, runtime_id.clone(), true),
            ),
            (
                second_connection,
                test_connection(second_connection, runtime_id.clone(), true),
            ),
        ]),
    });

    let now = chrono::Utc::now();
    let observations = match remaining_first {
        Some(remaining) => vec![quota_observation(
            first_connection,
            runtime_id.clone(),
            remaining,
            now + chrono::Duration::seconds(observed_at_offset_seconds),
            reset_in_seconds.map(|seconds| now + chrono::Duration::seconds(seconds)),
        )],
        None => Vec::new(),
    };
    let quota_reader: Arc<dyn QuotaObservationRecorderPort> =
        Arc::new(FakeQuotaStore::new(observations));

    QuotaComboSetup {
        combo,
        router,
        provider_repository,
        quota_reader,
        first_calls,
        second_calls,
    }
}

#[tokio::test]
async fn combo_skips_connection_with_fresh_exhausted_quota() {
    let QuotaComboSetup {
        combo,
        router,
        provider_repository,
        quota_reader,
        first_calls,
        second_calls,
    } = quota_combo_setup(Some(0.0), 0, Some(60));

    let h = combo_harness_with_repositories_and_quota(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(provider_repository),
        Some(quota_reader),
    );

    let result = h
        .usecase
        .execute_combo(
            &combo,
            combo_req("gpt-test", rook_core::ApiKeyRestrictions::default()),
            ApiFormat::OpenAI,
        )
        .await;

    assert!(
        result.is_ok(),
        "the eligible connection should serve the request: {result:?}"
    );
    assert!(
        first_calls.lock().unwrap().is_empty(),
        "the exhausted connection must not reach the upstream provider"
    );
    assert_eq!(
        second_calls.lock().unwrap().as_slice(),
        &["gpt-test".to_string()],
        "the eligible connection must receive the request"
    );
}

#[tokio::test]
async fn combo_keeps_connection_without_quota_evidence() {
    let QuotaComboSetup {
        combo,
        router,
        provider_repository,
        quota_reader,
        first_calls,
        second_calls,
    } = quota_combo_setup(None, 0, None);

    let h = combo_harness_with_repositories_and_quota(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(provider_repository),
        Some(quota_reader),
    );

    let result = h
        .usecase
        .execute_combo(
            &combo,
            combo_req("gpt-test", rook_core::ApiKeyRestrictions::default()),
            ApiFormat::OpenAI,
        )
        .await;

    assert!(result.is_ok(), "unknown quota must not skip: {result:?}");
    assert_eq!(
        first_calls.lock().unwrap().as_slice(),
        &["gpt-test".to_string()],
        "unknown quota leaves the connection eligible"
    );
    assert!(second_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn combo_keeps_connection_with_stale_quota_evidence() {
    let QuotaComboSetup {
        combo,
        router,
        provider_repository,
        quota_reader,
        first_calls,
        second_calls,
    } = quota_combo_setup(Some(0.0), -3600, None);

    let h = combo_harness_with_repositories_and_quota(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(provider_repository),
        Some(quota_reader),
    );

    let result = h
        .usecase
        .execute_combo(
            &combo,
            combo_req("gpt-test", rook_core::ApiKeyRestrictions::default()),
            ApiFormat::OpenAI,
        )
        .await;

    assert!(result.is_ok(), "stale quota must not skip: {result:?}");
    assert_eq!(
        first_calls.lock().unwrap().as_slice(),
        &["gpt-test".to_string()],
        "stale evidence leaves the connection eligible"
    );
    assert!(second_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn combo_reconsiders_connection_after_the_reset_instant_passed() {
    let QuotaComboSetup {
        combo,
        router,
        provider_repository,
        quota_reader,
        first_calls,
        second_calls,
    } = quota_combo_setup(Some(0.0), -60, Some(-30));

    let h = combo_harness_with_repositories_and_quota(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(provider_repository),
        Some(quota_reader),
    );

    let result = h
        .usecase
        .execute_combo(
            &combo,
            combo_req("gpt-test", rook_core::ApiKeyRestrictions::default()),
            ApiFormat::OpenAI,
        )
        .await;

    assert!(result.is_ok(), "expired quota must not skip: {result:?}");
    assert_eq!(
        first_calls.lock().unwrap().as_slice(),
        &["gpt-test".to_string()],
        "an expired reset restores eligibility"
    );
    assert!(second_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn combo_reports_the_quota_skip_reason_when_every_connection_is_exhausted() {
    let first_connection = ConnectionId::new();
    let runtime_id = ProviderId::new("openai-primary");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let combo = Combo::new(
        "quota-preflight-exhausted".to_string(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: runtime_id.clone(),
            model: ModelId::new("gpt-test"),
            connection_id: Some(first_connection),
            priority: 1,
        }],
    );

    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![],
        connection_providers: HashMap::from([(
            first_connection,
            Arc::new(SpyProvider {
                id: ProviderId::new(first_connection.to_string()),
                model: ModelId::new("gpt-test"),
                seen_models: calls.clone(),
            }) as Arc<dyn ProviderPort>,
        )]),
    });
    let provider_repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([(
            first_connection,
            test_connection(first_connection, runtime_id.clone(), true),
        )]),
    });
    let now = chrono::Utc::now();
    let quota_reader: Arc<dyn QuotaObservationRecorderPort> =
        Arc::new(FakeQuotaStore::new(vec![quota_observation(
            first_connection,
            runtime_id,
            0.0,
            now,
            Some(now + chrono::Duration::seconds(60)),
        )]));

    let h = combo_harness_with_repositories_and_quota(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(provider_repository),
        Some(quota_reader),
    );

    let error = h
        .usecase
        .execute_combo(
            &combo,
            combo_req("gpt-test", rook_core::ApiKeyRestrictions::default()),
            ApiFormat::OpenAI,
        )
        .await
        .expect_err("an exhausted connection must fail the combo");

    assert!(
        calls.lock().unwrap().is_empty(),
        "the exhausted connection must not reach the upstream provider"
    );
    let reasons = combo_skip_reasons(&error);
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("quota_exhausted")),
        "the skip reason must expose the quota decision: {reasons:?}"
    );
    assert!(
        reasons.iter().all(|reason| !reason.contains("api_key")
            && !reason.contains("secret")
            && !reason.contains("token=")),
        "the skip reason must not leak credentials: {reasons:?}"
    );
}

#[tokio::test]
async fn unauthorized_later_connection_step_is_rejected_before_any_upstream_io() {
    let first_connection = ConnectionId::new();
    let unauthorized_connection = ConnectionId::new();
    let runtime_id = ProviderId::new("openai-primary");
    let first_adapter = ProviderId::new(first_connection.to_string());
    let first_calls = Arc::new(Mutex::new(Vec::new()));
    let unauthorized_calls = Arc::new(Mutex::new(Vec::new()));
    let combo = Combo::new(
        "preflight-authorization".to_string(),
        ComboStrategy::Priority,
        vec![
            ComboStep {
                provider_id: runtime_id.clone(),
                model: ModelId::new("gpt-test"),
                connection_id: Some(first_connection),
                priority: 1,
            },
            ComboStep {
                provider_id: runtime_id.clone(),
                model: ModelId::new("gpt-test"),
                connection_id: Some(unauthorized_connection),
                priority: 2,
            },
        ],
    );
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![],
        connection_providers: HashMap::from([
            (
                first_connection,
                Arc::new(SpyProvider {
                    id: first_adapter.clone(),
                    model: ModelId::new("gpt-test"),
                    seen_models: first_calls.clone(),
                }) as Arc<dyn ProviderPort>,
            ),
            (
                unauthorized_connection,
                Arc::new(SpyProvider {
                    id: ProviderId::new(unauthorized_connection.to_string()),
                    model: ModelId::new("gpt-test"),
                    seen_models: unauthorized_calls.clone(),
                }) as Arc<dyn ProviderPort>,
            ),
        ]),
    });
    let provider_repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([
            (
                first_connection,
                test_connection(first_connection, runtime_id.clone(), true),
            ),
            (
                unauthorized_connection,
                test_connection(unauthorized_connection, runtime_id, true),
            ),
        ]),
    });
    let h = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(provider_repository),
    );

    let result = h
        .usecase
        .execute_combo(
            &combo,
            combo_req(
                "gpt-test",
                rook_core::ApiKeyRestrictions {
                    allowed_models: vec![],
                    allowed_providers: vec![first_adapter],
                },
            ),
            ApiFormat::OpenAI,
        )
        .await;

    assert!(matches!(result, Err(error) if error.is_forbidden()));
    assert!(first_calls.lock().unwrap().is_empty());
    assert!(unauthorized_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn connection_bound_step_rejects_another_account_before_upstream_io() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("openai-primary");
    let adapter_id = ProviderId::new(connection_id.to_string());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let combo = Combo::new(
        "cross-account".to_string(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: runtime_id.clone(),
            model: ModelId::new("gpt-4o"),
            connection_id: Some(connection_id),
            priority: 1,
        }],
    );
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![],
        connection_providers: HashMap::from([(
            connection_id,
            Arc::new(SpyProvider {
                id: adapter_id.clone(),
                model: ModelId::new("gpt-4o"),
                seen_models: calls.clone(),
            }) as Arc<dyn ProviderPort>,
        )]),
    });
    let provider_repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([(
            connection_id,
            test_connection(connection_id, runtime_id, true),
        )]),
    });
    let h = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(provider_repository),
    );
    let restrictions = rook_core::ApiKeyRestrictions {
        allowed_models: vec![],
        allowed_providers: vec![ProviderId::new("some-other-account")],
    };

    let error = h
        .usecase
        .execute_combo(&combo, combo_req("gpt-4o", restrictions), ApiFormat::OpenAI)
        .await
        .expect_err("different account must be forbidden");

    assert!(
        error.is_forbidden(),
        "expected 403 classification, got {error}"
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "forbidden account must not receive upstream I/O"
    );
}

#[tokio::test]
async fn connection_bound_step_falls_back_to_the_next_step_on_rate_limit() {
    let first_connection = ConnectionId::new();
    let second_connection = ConnectionId::new();
    let runtime_id = ProviderId::new("openai-primary");
    let first_adapter = ProviderId::new(first_connection.to_string());
    let second_adapter = ProviderId::new(second_connection.to_string());
    let second_calls = Arc::new(Mutex::new(Vec::new()));

    let combo = Combo::new(
        "fallback-429".to_string(),
        ComboStrategy::Priority,
        vec![
            ComboStep {
                provider_id: runtime_id.clone(),
                model: ModelId::new("gpt-test"),
                connection_id: Some(first_connection),
                priority: 1,
            },
            ComboStep {
                provider_id: runtime_id.clone(),
                model: ModelId::new("gpt-test"),
                connection_id: Some(second_connection),
                priority: 2,
            },
        ],
    );

    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![],
        connection_providers: HashMap::from([
            (
                first_connection,
                Arc::new(RateLimitedProvider {
                    id: first_adapter.clone(),
                }) as Arc<dyn ProviderPort>,
            ),
            (
                second_connection,
                Arc::new(SpyProvider {
                    id: second_adapter.clone(),
                    model: ModelId::new("gpt-test"),
                    seen_models: second_calls.clone(),
                }) as Arc<dyn ProviderPort>,
            ),
        ]),
    });
    let provider_repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([
            (
                first_connection,
                test_connection(first_connection, runtime_id.clone(), true),
            ),
            (
                second_connection,
                test_connection(second_connection, runtime_id.clone(), true),
            ),
        ]),
    });
    let h = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(provider_repository),
    );

    let response = h
        .usecase
        .execute_combo(
            &combo,
            combo_req(
                "gpt-test",
                rook_core::ApiKeyRestrictions {
                    allowed_models: vec![],
                    allowed_providers: vec![],
                },
            ),
            ApiFormat::OpenAI,
        )
        .await
        .expect("the second step must run after the 429");

    assert_eq!(response.provider, second_adapter);
    assert_eq!(
        second_calls.lock().unwrap().as_slice(),
        ["gpt-test".to_string()],
        "only the second connection must reach the upstream provider"
    );
}

#[tokio::test]
async fn unusable_and_disallowed_connection_step_is_skipped_in_both_paths() {
    let first_connection = ConnectionId::new();
    let second_connection = ConnectionId::new();
    let runtime_id = ProviderId::new("openai-primary");
    let first_adapter = ProviderId::new(first_connection.to_string());
    let second_adapter = ProviderId::new(second_connection.to_string());
    let second_calls = Arc::new(Mutex::new(Vec::new()));

    let combo = Combo::new(
        "unusable-disallowed".to_string(),
        ComboStrategy::Priority,
        vec![
            ComboStep {
                provider_id: runtime_id.clone(),
                model: ModelId::new("gpt-test"),
                connection_id: Some(first_connection),
                priority: 1,
            },
            ComboStep {
                provider_id: runtime_id.clone(),
                model: ModelId::new("gpt-test"),
                connection_id: Some(second_connection),
                priority: 2,
            },
        ],
    );

    // The second adapter is disallowed and cannot serve the step model, so
    // both the preflight and the execution loop must classify it as a skip.
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![],
        connection_providers: HashMap::from([
            (
                first_connection,
                Arc::new(RateLimitedProvider {
                    id: first_adapter.clone(),
                }) as Arc<dyn ProviderPort>,
            ),
            (
                second_connection,
                Arc::new(SpyProvider {
                    id: second_adapter.clone(),
                    model: ModelId::new("other-model"),
                    seen_models: second_calls.clone(),
                }) as Arc<dyn ProviderPort>,
            ),
        ]),
    });
    let provider_repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([
            (
                first_connection,
                test_connection(first_connection, runtime_id.clone(), true),
            ),
            (
                second_connection,
                test_connection(second_connection, runtime_id.clone(), true),
            ),
        ]),
    });
    let h = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(provider_repository),
    );

    let error = h
        .usecase
        .execute_combo(
            &combo,
            combo_req(
                "gpt-test",
                rook_core::ApiKeyRestrictions {
                    allowed_models: vec![],
                    allowed_providers: vec![first_adapter],
                },
            ),
            ApiFormat::OpenAI,
        )
        .await
        .expect_err("an unusable step cannot satisfy the request");

    assert!(
        !error.is_forbidden(),
        "an unusable adapter must be skipped, not rejected with 403: {error}"
    );
    assert!(
        error.to_string().contains("all providers exhausted"),
        "the combo must report exhausted providers, got {error}"
    );
    assert!(
        second_calls.lock().unwrap().is_empty(),
        "the unusable adapter must not receive upstream I/O"
    );
}

#[tokio::test]
async fn combo_audit_and_usage_record_the_effective_connection() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("openai-primary");
    let adapter_id = ProviderId::new(connection_id.to_string());
    let calls = Arc::new(Mutex::new(Vec::new()));

    let combo = Combo::new(
        "audit-attribution".to_string(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: runtime_id.clone(),
            model: ModelId::new("gpt-test"),
            connection_id: Some(connection_id),
            priority: 1,
        }],
    );
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![],
        connection_providers: HashMap::from([(
            connection_id,
            Arc::new(SpyProvider {
                id: adapter_id.clone(),
                model: ModelId::new("gpt-test"),
                seen_models: calls.clone(),
            }) as Arc<dyn ProviderPort>,
        )]),
    });
    let provider_repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([(
            connection_id,
            test_connection(connection_id, runtime_id.clone(), true),
        )]),
    });
    let h = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        calls,
        Some(provider_repository),
    );

    h.usecase
        .execute_combo(
            &combo,
            combo_req(
                "gpt-test",
                rook_core::ApiKeyRestrictions {
                    allowed_models: vec![],
                    allowed_providers: vec![],
                },
            ),
            ApiFormat::OpenAI,
        )
        .await
        .expect("combo execution succeeds");

    let audit_entries = h.audit.entries.lock().unwrap();
    assert_eq!(audit_entries.len(), 1, "one audit entry per executed step");
    let entry = &audit_entries[0];
    assert_eq!(
        entry.provider, adapter_id,
        "audit keeps the adapter identity"
    );
    assert_eq!(entry.connection_id, Some(connection_id));
    assert_eq!(entry.provider_runtime_id, Some(runtime_id));
    assert_eq!(entry.combo_id, Some(combo.id));
    assert_eq!(entry.combo_step_index, Some(0));
    drop(audit_entries);

    let usage_entries = h.usage.entries.lock().unwrap();
    assert_eq!(usage_entries.len(), 1, "one usage entry per executed step");
    assert_eq!(usage_entries[0].connection_id, Some(connection_id));
    assert_eq!(usage_entries[0].provider, adapter_id);
}

#[tokio::test]
async fn legacy_step_keeps_its_adapter_id_without_connection_lookup() {
    let legacy_adapter = ProviderId::new("legacy-adapter");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let combo = Combo::new(
        "legacy-compat".to_string(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: legacy_adapter.clone(),
            model: ModelId::new("gpt-test"),
            connection_id: None,
            priority: 1,
        }],
    );
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![Arc::new(SpyProvider {
            id: legacy_adapter.clone(),
            model: ModelId::new("gpt-test"),
            seen_models: calls.clone(),
        })],
        connection_providers: HashMap::new(),
    });
    // An empty repository proves the legacy path never resolves a connection:
    // any lookup would skip the step and exhaust the combo.
    let provider_repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::new(),
    });
    let h = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        calls.clone(),
        Some(provider_repository),
    );

    h.usecase
        .execute_combo(
            &combo,
            combo_req(
                "gpt-test",
                rook_core::ApiKeyRestrictions {
                    allowed_models: vec![],
                    allowed_providers: vec![],
                },
            ),
            ApiFormat::OpenAI,
        )
        .await
        .expect("legacy step must still execute");

    assert_eq!(
        calls.lock().unwrap().as_slice(),
        ["gpt-test".to_string()],
        "the legacy adapter must receive the request"
    );

    let audit_entries = h.audit.entries.lock().unwrap();
    assert_eq!(audit_entries.len(), 1);
    assert_eq!(audit_entries[0].provider, legacy_adapter);
    assert_eq!(audit_entries[0].connection_id, None);
    assert_eq!(audit_entries[0].provider_runtime_id, None);
}

#[tokio::test]
async fn virtual_combo_executes_and_returns_requested_model() {
    let combo = harness_combo();
    let combo_id = combo.id;
    let h = combo_harness(combo, vec![("rook/coding", combo_id)]);
    let resp = h
        .usecase
        .execute(combo_req(
            "rook/coding",
            rook_core::ApiKeyRestrictions::default(),
        ))
        .await
        .expect("virtual combo executes");
    assert_eq!(resp.model.as_str(), "rook/coding");
    assert_eq!(resp.content, "combo says hi");
}

#[tokio::test]
async fn virtual_combo_sends_physical_model_upstream() {
    let combo = harness_combo();
    let combo_id = combo.id;
    let h = combo_harness(combo, vec![("rook/coding", combo_id)]);
    h.usecase
        .execute(combo_req(
            "rook/coding",
            rook_core::ApiKeyRestrictions::default(),
        ))
        .await
        .expect("executes");
    let seen = h.seen_models.lock().unwrap();
    assert_eq!(
        seen.as_slice(),
        ["gpt-4o"],
        "provider must receive the STEP physical model, never rook/coding"
    );
}

#[tokio::test]
async fn unknown_virtual_alias_is_not_found() {
    let h = combo_harness(harness_combo(), vec![]);
    let err = h
        .usecase
        .execute(combo_req(
            "rook/nope",
            rook_core::ApiKeyRestrictions::default(),
        ))
        .await
        .unwrap_err();
    assert!(err.is_not_found(), "expected 404, got: {err}");
}

#[tokio::test]
async fn unauthorized_physical_step_rejects_whole_combo() {
    let combo = harness_combo();
    let combo_id = combo.id;
    let h = combo_harness(combo, vec![("rook/coding", combo_id)]);
    let restrictions = rook_core::ApiKeyRestrictions {
        allowed_models: vec![ModelId::new("rook/coding"), ModelId::new("other-model")],
        allowed_providers: vec![],
    };
    let err = h
        .usecase
        .execute(combo_req("rook/coding", restrictions))
        .await
        .unwrap_err();
    assert!(err.is_forbidden(), "expected 403, got: {err}");
    assert_eq!(err.forbidden_code(), Some("model_not_allowed"));
    assert!(
        h.seen_models.lock().unwrap().is_empty(),
        "no step may execute when unauthorized"
    );
}

#[tokio::test]
async fn unauthorized_alias_rejects_whole_combo() {
    let combo = harness_combo();
    let combo_id = combo.id;
    let h = combo_harness(combo, vec![("rook/coding", combo_id)]);
    let restrictions = rook_core::ApiKeyRestrictions {
        allowed_models: vec![ModelId::new("gpt-4o")],
        allowed_providers: vec![],
    };
    let err = h
        .usecase
        .execute(combo_req("rook/coding", restrictions))
        .await
        .unwrap_err();
    assert!(err.is_forbidden(), "expected 403, got: {err}");
}

#[tokio::test]
async fn orphan_alias_is_integrity_error() {
    let missing = ComboId::new();
    let h = combo_harness(harness_combo(), vec![("rook/orphan", missing)]);
    let err = h
        .usecase
        .execute(combo_req(
            "rook/orphan",
            rook_core::ApiKeyRestrictions::default(),
        ))
        .await
        .unwrap_err();
    // load_combo maps absence to combo_not_found, which is a 404-class error.
    assert!(err.is_not_found(), "expected 404-class, got: {err}");
}

#[tokio::test]
async fn legacy_combo_still_executes() {
    let combo = harness_combo();
    let combo_id = combo.id;
    let h = combo_harness(combo, vec![]);
    let mut req = combo_req("gpt-4o", rook_core::ApiKeyRestrictions::default());
    req.metadata.combo_id = Some(combo_id);
    let resp = h.usecase.execute(req).await.expect("legacy executes");
    assert_eq!(resp.model.as_str(), "gpt-4o");
    assert_eq!(resp.content, "combo says hi");
    assert_eq!(
        h.seen_models.lock().unwrap().as_slice(),
        ["gpt-4o"],
        "legacy steps also receive the physical model"
    );
}

// ------------------------------------------------------------------
// R2 streaming (Task 4): fallback before the first event only
// ------------------------------------------------------------------

struct CountRouter {
    providers: Vec<Arc<dyn ProviderPort>>,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl RouterPort for CountRouter {
    async fn select(&self, _req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.providers[n.min(self.providers.len() - 1)].clone())
    }
    async fn select_excluding(
        &self,
        req: &CompletionRequest,
        _excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        self.select(req).await
    }
    async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}
    fn providers(&self) -> Vec<ProviderId> {
        self.providers.iter().map(|p| p.id().clone()).collect()
    }
}

fn stream_harness(
    combos: HashMap<ComboId, Combo>,
    aliases: HashMap<String, ComboAlias>,
    providers: Vec<Arc<dyn ProviderPort>>,
) -> RouteRequest {
    RouteRequest::new(
        Arc::new(CountRouter {
            providers,
            calls: std::sync::atomic::AtomicUsize::new(0),
        }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        None,
        Some(Arc::new(MapComboRepository { combos })),
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases }))
}

fn failing_provider(id: &str) -> Arc<dyn ProviderPort> {
    Arc::new(TestProvider {
        id: ProviderId::new(id),
        complete_error: Some(CortexError::provider("boom")),
    })
}

fn ok_provider(id: &str) -> Arc<dyn ProviderPort> {
    Arc::new(TestProvider {
        id: ProviderId::new(id),
        complete_error: None,
    })
}

fn stream_req(model: &str) -> CompletionRequest {
    let mut r = combo_req(model, rook_core::ApiKeyRestrictions::default());
    r.stream = true;
    r
}

#[tokio::test]
async fn combo_stream_emits_content_then_final_chunk() {
    let combo = harness_combo();
    let combo_id = combo.id;
    let mut combos = HashMap::new();
    combos.insert(combo_id, combo);
    let mut aliases = HashMap::new();
    aliases.insert(
        "rook/coding".to_string(),
        ComboAlias::new(ModelId::new("rook/coding"), combo_id),
    );
    let usecase = stream_harness(combos, aliases, vec![ok_provider("openai")]);
    let stream = usecase
        .execute_stream_with_format(stream_req("rook/coding"), ApiFormat::OpenAI)
        .await
        .expect("combo stream starts");
    let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
    assert_eq!(chunks.len(), 2, "content chunk, then final chunk");
    let mut chunks = chunks.into_iter().map(|c| c.expect("chunk ok"));
    let content = chunks.next().unwrap();
    assert_eq!(content.model.as_str(), "rook/coding");
    assert_eq!(content.delta, "cached path");
    assert!(content.finish_reason.is_none());
    assert!(content.usage.is_none());
    let final_chunk = chunks.next().unwrap();
    assert_eq!(final_chunk.model.as_str(), "rook/coding");
    assert!(final_chunk.delta.is_empty());
    assert!(final_chunk.usage.is_some(), "usage rides the final chunk");
}

/// Test provider returning a tool_call response for combo-stream coverage.
struct ToolProvider {
    id: ProviderId,
}

#[async_trait]
impl ProviderPort for ToolProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }
    fn supported_models(&self) -> &[ModelId] {
        std::slice::from_ref(&TEST_MODEL)
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
            latency_ms: 1,
        }
    }
    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.id.clone(),
            model: req.model.clone(),
            content: String::new(),
            content_blocks: vec![],
            thinking: None,
            tool_calls: vec![rook_core::MessageToolCall {
                id: Some("call_1".to_string()),
                name: "lookup".to_string(),
                arguments: serde_json::json!({"key": "value"}),
            }],
            finish_reason: Some(rook_core::FinishReason::ToolCalls),
            usage: TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 1,
            cache_hit: None,
        })
    }
    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        unreachable!("tool combo stream test uses complete()")
    }
}

#[tokio::test]
async fn combo_stream_carries_tool_calls_then_tool_use_finish() {
    let combo = harness_combo();
    let combo_id = combo.id;
    let mut combos = HashMap::new();
    combos.insert(combo_id, combo);
    let mut aliases = HashMap::new();
    aliases.insert(
        "rook/tools".to_string(),
        ComboAlias::new(ModelId::new("rook/tools"), combo_id),
    );
    // stream_harness wires TestProvider; rebuild with ToolProvider here.
    let tool: Arc<dyn ProviderPort> = Arc::new(ToolProvider {
        id: ProviderId::new("openai"),
    });
    let usecase = RouteRequest::new(
        Arc::new(RecordingRouter { provider: tool }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        None,
        Some(Arc::new(MapComboRepository { combos })),
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases }));
    let stream = usecase
        .execute_stream_with_format(stream_req("rook/tools"), ApiFormat::OpenAI)
        .await
        .expect("tool combo stream starts");
    let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
    assert_eq!(chunks.len(), 2);
    let mut chunks = chunks.into_iter().map(|c| c.expect("chunk ok"));
    let content = chunks.next().unwrap();
    assert_eq!(content.tool_calls.len(), 2, "start + args fragments");
    assert_eq!(content.tool_calls[0].id.as_deref(), Some("call_1"));
    assert_eq!(
        content.tool_calls[0].function.name.as_deref(),
        Some("lookup")
    );
    assert!(content.tool_calls[1].function.arguments.is_some());
    assert!(content.finish_reason.is_none());
    let final_chunk = chunks.next().unwrap();
    assert_eq!(
        final_chunk.finish_reason,
        Some(rook_core::FinishReason::ToolCalls)
    );
    assert_eq!(final_chunk.tool_calls.len(), 1, "stop summary");
    assert!(final_chunk.usage.is_some());
}

#[tokio::test]
async fn combo_stream_falls_back_before_first_event() {
    // Step 1 (openai) fails retryable, step 2 (fallback) succeeds. All of
    // this happens BEFORE any SSE event is emitted (point of no return).
    let failing = Combo::new(
        "fb".to_string(),
        ComboStrategy::Priority,
        vec![
            step("openai", "gpt-4o", 1),
            step("fallback", "gpt-4o-mini", 2),
        ],
    );
    let combo_id = failing.id;
    let mut combos = HashMap::new();
    combos.insert(combo_id, failing);
    let mut aliases = HashMap::new();
    aliases.insert(
        "rook/fb".to_string(),
        ComboAlias::new(ModelId::new("rook/fb"), combo_id),
    );
    let usecase = stream_harness(
        combos,
        aliases,
        vec![failing_provider("openai"), ok_provider("fallback")],
    );
    let stream = usecase
        .execute_stream_with_format(stream_req("rook/fb"), ApiFormat::OpenAI)
        .await
        .expect("fallback combo stream starts");
    let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
    assert_eq!(
        chunks.len(),
        2,
        "content + final after pre-emission fallback"
    );
    assert!(chunks.into_iter().all(|c| c.is_ok()));
}

#[tokio::test]
async fn combo_stream_unknown_alias_is_not_found_before_headers() {
    let usecase = stream_harness(HashMap::new(), HashMap::new(), vec![ok_provider("openai")]);
    let err = usecase
        .execute_stream_with_format(stream_req("rook/nope"), ApiFormat::OpenAI)
        .await
        .err()
        .expect("unknown alias must fail before committing headers");
    assert!(
        err.is_not_found(),
        "expected 404 before committing headers, got: {err}"
    );
}

// ------------------------------------------------------------------
// R4 regression: step-physical selection with a model-filtering router
// ------------------------------------------------------------------

/// Router that behaves like the real FallbackRouter: only providers
/// listing `req.model` are selectable. A virtual `rook/*` model matches
/// nothing — combo steps must select with the STEP model.
struct ModelFilteringRouter {
    providers: Vec<Arc<dyn ProviderPort>>,
}

#[async_trait]
impl RouterPort for ModelFilteringRouter {
    async fn select(&self, req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        self.providers
            .iter()
            .find(|p| p.supported_models().contains(&req.model))
            .cloned()
            .ok_or_else(CortexError::all_providers_exhausted)
    }
    async fn select_excluding(
        &self,
        req: &CompletionRequest,
        _excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        self.select(req).await
    }
    async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}
    fn providers(&self) -> Vec<ProviderId> {
        self.providers.iter().map(|p| p.id().clone()).collect()
    }
}

struct PhysicalSpyProvider {
    id: ProviderId,
    models: Vec<ModelId>,
    seen: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ProviderPort for PhysicalSpyProvider {
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
            latency_ms: 1,
        }
    }
    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        self.seen.lock().unwrap().push(req.model.to_string());
        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.id.clone(),
            model: req.model.clone(),
            content: "physical ok".to_string(),
            content_blocks: vec![rook_core::MessageContent::Text("physical ok".to_string())],
            thinking: None,
            tool_calls: vec![],
            finish_reason: None,
            usage: TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 1,
            cache_hit: None,
        })
    }
    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        unreachable!("selection regression uses complete()")
    }
}

#[tokio::test]
async fn virtual_combo_selects_with_step_physical_model() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn ProviderPort> = Arc::new(PhysicalSpyProvider {
        id: ProviderId::new("openai"),
        models: vec![ModelId::new("gpt-4o")],
        seen: seen.clone(),
    });
    let combo = harness_combo();
    let combo_id = combo.id;
    let mut combos = HashMap::new();
    combos.insert(combo_id, combo);
    let mut aliases = HashMap::new();
    aliases.insert(
        "rook/coding".to_string(),
        ComboAlias::new(ModelId::new("rook/coding"), combo_id),
    );
    let usecase = RouteRequest::new(
        Arc::new(ModelFilteringRouter {
            providers: vec![provider],
        }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        None,
        Some(Arc::new(MapComboRepository { combos })),
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases }));
    let resp = usecase
        .execute(combo_req(
            "rook/coding",
            rook_core::ApiKeyRestrictions::default(),
        ))
        .await
        .expect("virtual combo must execute against a model-filtering router");
    assert_eq!(resp.model.as_str(), "rook/coding");
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["gpt-4o"],
        "selection AND upstream must use the step physical model"
    );
}

// ------------------------------------------------------------------
// A01 stream-virtual-alias-hijack regression (TDD)
// ------------------------------------------------------------------

/// ModelAlias repo that hijacks `rook/coding -> gpt-4o` plus a normal
/// alias `coding -> gpt-4o`. Tracks whether it was consulted for rook/*.
struct HijackAliasRepo {
    rook_consulted: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl ModelAliasRepositoryPort for HijackAliasRepo {
    async fn find_by_alias(
        &self,
        alias: &ModelId,
        _provider_id: Option<&ProviderId>,
    ) -> Result<Option<ModelAlias>, ModelAliasRepositoryError> {
        if alias.as_str().starts_with("rook/") {
            self.rook_consulted
                .store(true, std::sync::atomic::Ordering::SeqCst);
            if alias.as_str() == "rook/coding" {
                return Ok(Some(ModelAlias {
                    alias: ModelId::new("rook/coding"),
                    canonical: ModelId::new("gpt-4o"),
                    provider_id: None,
                    created_at: Utc::now(),
                }));
            }
            return Ok(None);
        }
        if alias.as_str() == "coding" {
            return Ok(Some(ModelAlias {
                alias: ModelId::new("coding"),
                canonical: ModelId::new("gpt-4o"),
                provider_id: None,
                created_at: Utc::now(),
            }));
        }
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

/// Streaming provider that echoes the REQUEST model into each chunk, so a
/// hijack (`rook/coding` rewritten to `gpt-4o` before dispatch) is
/// observable as chunk.model == "gpt-4o" instead of "rook/coding".
struct EchoStreamProvider {
    id: ProviderId,
}

#[async_trait]
impl ProviderPort for EchoStreamProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn supported_models(&self) -> &[ModelId] {
        std::slice::from_ref(&TEST_MODEL)
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
            latency_ms: 1,
        }
    }

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.id.clone(),
            model: req.model.clone(),
            content: "echo".to_string(),
            content_blocks: vec![rook_core::MessageContent::Text("echo".to_string())],
            thinking: None,
            tool_calls: vec![],
            finish_reason: None,
            usage: TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 1,
            cache_hit: None,
        })
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        let model = req.model.clone();
        let id = req.id.clone();
        Ok(Box::pin(stream::iter(vec![Ok(StreamChunk {
            id,
            model,
            delta: "hi".to_string(),
            thinking: None,
            tool_calls: vec![],
            finish_reason: None,
            usage: None,
        })])))
    }
}

fn hijack_stream_harness(rook_consulted: Arc<std::sync::atomic::AtomicBool>) -> RouteRequest {
    let combo = harness_combo();
    let combo_id = combo.id;
    let mut combos = HashMap::new();
    combos.insert(combo_id, combo);
    let mut aliases = HashMap::new();
    aliases.insert(
        "rook/coding".to_string(),
        ComboAlias::new(ModelId::new("rook/coding"), combo_id),
    );
    let provider: Arc<dyn ProviderPort> = Arc::new(EchoStreamProvider {
        id: ProviderId::new("openai"),
    });
    RouteRequest::new(
        Arc::new(RecordingRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
        None,
        None,
        Some(Arc::new(MapComboRepository { combos })),
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestFormatTranslator),
        Arc::new(HijackAliasRepo { rook_consulted }),
        ModelAliasesConfig {
            enabled: true,
            auto_seed: false,
        },
        None,
    )
    .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases }))
}

#[tokio::test]
async fn streaming_virtual_model_never_enters_model_alias_resolution() {
    let rook_consulted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let usecase = hijack_stream_harness(rook_consulted.clone());
    // Even though ModelAlias contains rook/coding -> gpt-4o, streaming
    // must NOT rewrite rook/*: it must take the virtual-combo path and
    // keep the requested model.
    let stream = usecase
        .execute_stream_with_format(stream_req("rook/coding"), ApiFormat::OpenAI)
        .await
        .expect("virtual combo stream starts");
    let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
    assert_eq!(chunks.len(), 2, "combo stream emits content + final chunk");
    let mut chunks = chunks.into_iter().map(|c| c.expect("chunk ok"));
    let content = chunks.next().unwrap();
    let final_chunk = chunks.next().unwrap();
    assert_eq!(
        content.model.as_str(),
        "rook/coding",
        "streaming must preserve rook/coding (combo), never rewrite to physical gpt-4o"
    );
    assert_eq!(final_chunk.model.as_str(), "rook/coding");
    assert!(
        !rook_consulted.load(std::sync::atomic::Ordering::SeqCst),
        "ModelAlias repo must never be consulted for rook/*"
    );
}

#[tokio::test]
async fn streaming_normal_alias_still_resolves() {
    let rook_consulted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let usecase = hijack_stream_harness(rook_consulted.clone());
    // Non-virtual alias `coding -> gpt-4o` must still resolve on the
    // streaming path (echo provider surfaces the rewritten model).
    let stream = usecase
        .execute_stream_with_format(stream_req("coding"), ApiFormat::OpenAI)
        .await
        .expect("stream starts");
    let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
    assert_eq!(chunks.len(), 1, "physical stream emits single echo chunk");
    let chunk = chunks.into_iter().next().unwrap().expect("chunk ok");
    assert_eq!(
        chunk.model.as_str(),
        "gpt-4o",
        "normal alias must still resolve in streaming path"
    );
}
