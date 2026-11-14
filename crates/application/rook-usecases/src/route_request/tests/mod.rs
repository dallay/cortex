use super::outcome::{classify_attempt_result, AttemptOutcome, AttemptResult};
use super::*;
mod capabilities;
use async_trait::async_trait;
use chrono::Utc;
use futures::{stream, StreamExt};
use rook_core::{
    ApiFormat, ApiKeyId, CacheStats, Combo, ComboAlias, ComboAliasRepositoryError,
    ComboAliasRepositoryPort, ComboRepositoryError, ComboStep, ComboStrategy, CostBreakdown,
    HealthStatus, Message, ModelAlias, ModelAliasRepositoryError, ModelId, Pagination, ProviderId,
    ProviderPort, ProviderRepositoryPort, QuotaObservation, QuotaObservationRecorderPort,
    RequestMetadata, Role, SignatureEntry, StreamChunk, TokenCacheStats, TokenUsage, UsageEntry,
    UsageFilters, UsageRecorderPort, UsageSummary,
};
use shared_kernel::{CacheKey, ConnectionId, CortexResult, RequestId};
use std::collections::HashMap;
use std::sync::Mutex;

// These routing fixtures deliberately accept tools and streaming. Production
// adapters default to unknown; capability-denial tests use separate fixtures.
fn fixture_capabilities() -> rook_core::ModelCapabilities {
    let mut capabilities = rook_core::ModelCapabilities::default();
    capabilities.apply(
        &rook_core::CapabilityOverrides {
            tools: Some(rook_core::CapabilityState::Supported),
            streaming: Some(rook_core::CapabilityState::Supported),
            ..Default::default()
        },
        "test_fixture",
    );
    capabilities
}

#[test]
fn classifies_success_local_backpressure_upstream_failure_and_timeout() {
    assert!(matches!(
        classify_attempt_result(AttemptResult::Provider(Ok(()))),
        AttemptOutcome::Success(())
    ));

    assert!(matches!(
        classify_attempt_result::<()>(AttemptResult::Provider(Err(CortexError::local_backpressure()))),
        AttemptOutcome::LocalBackpressure(error) if error.is_local_backpressure()
    ));

    assert!(matches!(
        classify_attempt_result::<()>(AttemptResult::Provider(Err(CortexError::provider(
            "upstream failed"
        )))),
        AttemptOutcome::UpstreamFailure(error) if error.is_retryable()
    ));

    assert!(matches!(
        classify_attempt_result::<()>(AttemptResult::TimedOut),
        AttemptOutcome::TimedOut
    ));
}

mod default_combo;
mod output_validation;
mod preflight_guards;
mod progressive_combo;
mod spending_budget;

struct TestProvider {
    id: ProviderId,
    complete_error: Option<CortexError>,
    stream_error: Option<CortexError>,
}

#[async_trait]
impl ProviderPort for TestProvider {
    fn model_capabilities(&self, _: &ModelId) -> rook_core::ModelCapabilities {
        fixture_capabilities()
    }
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
        if self
            .complete_error
            .as_ref()
            .is_some_and(CortexError::is_local_backpressure)
        {
            return Err(CortexError::local_backpressure());
        }
        if self
            .complete_error
            .as_ref()
            .is_some_and(CortexError::is_invalid_request)
        {
            let msg = self.complete_error.as_ref().unwrap().to_string();
            return Err(CortexError::invalid_request(msg));
        }
        if let Some(err) = &self.complete_error {
            return Err(CortexError::provider(err.to_string()));
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
        if let Some(error) = &self.stream_error {
            return Ok(Box::pin(stream::iter(vec![Err(CortexError::provider(
                error.to_string(),
            ))])));
        }
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

struct UnavailableProvider {
    inner: Arc<dyn ProviderPort>,
}

#[async_trait]
impl ProviderPort for UnavailableProvider {
    fn id(&self) -> &ProviderId {
        self.inner.id()
    }

    fn supported_models(&self) -> &[ModelId] {
        self.inner.supported_models()
    }

    fn model_capabilities(&self, model: &ModelId) -> rook_core::ModelCapabilities {
        self.inner.model_capabilities(model)
    }

    fn api_format(&self) -> ApiFormat {
        self.inner.api_format()
    }

    fn is_available(&self) -> bool {
        false
    }

    async fn health_check(&self) -> HealthStatus {
        self.inner.health_check().await
    }

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        self.inner.complete(req).await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        self.inner.stream(req).await
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
        stream_error: None,
    });
    let provider_id = provider.id().clone();
    let router = crate::FallbackRouter::new(vec![provider], rook_core::PolicyKind::Priority);

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
    fn model_capabilities(&self, _: &ModelId) -> rook_core::ModelCapabilities {
        fixture_capabilities()
    }
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
            stream_error: None,
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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
}

#[tokio::test]
async fn policy_selected_completion_preserves_local_backpressure_classification() {
    let (provider, holder) = busy_limited_test_provider().await;
    let usecase = RouteRequest::new(
        Arc::new(crate::FallbackRouter::new(
            vec![provider],
            rook_core::PolicyKind::Priority,
        )),
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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    );
    let mut req = request();
    req.stream = false;
    req.metadata.cacheable = false;

    let error = usecase
        .execute(req)
        .await
        .expect_err("backpressure should be returned");
    holder.abort();
    assert!(error.is_local_backpressure(), "unexpected error: {error}");
}

#[test]
fn input_token_bound_is_unknown_for_structured_tool_content_calls_and_declarations() {
    let tool_content = rook_core::MessageContent::ToolUse {
        id: "call-1".to_string(),
        name: "search".to_string(),
        input: serde_json::json!({ "query": "a structured query payload" }),
    };
    let mut message_tool_call = request();
    message_tool_call.messages[0]
        .tool_calls
        .push(rook_core::MessageToolCall {
            id: Some("call-2".to_string()),
            name: "search".to_string(),
            arguments: serde_json::json!({ "query": "another payload" }),
        });
    let mut declared_tools = request();
    declared_tools.tools =
        Some(serde_json::json!([{ "type": "function", "function": { "name": "search" } }]));

    assert_eq!(
        super::input_token_bound(&CompletionRequest {
            messages: vec![Message {
                role: Role::Assistant,
                content: tool_content,
                tool_calls: Vec::new(),
            }],
            ..request()
        }),
        None
    );
    assert_eq!(super::input_token_bound(&message_tool_call), None);
    assert_eq!(super::input_token_bound(&declared_tools), None);
    assert_eq!(super::input_token_bound(&request()), Some(5));
}

#[tokio::test]
async fn completion_retries_eligible_provider_after_upstream_failure_then_backpressure() {
    let failing = Arc::new(TestProvider {
        id: ProviderId::new("upstream-failure"),
        complete_error: Some(CortexError::provider("provider failed")),
        stream_error: None,
    });
    let (busy, holder) = busy_limited_test_provider().await;
    let audit = Arc::new(TestAudit {
        entries: Mutex::new(Vec::new()),
    });
    let telemetry = Arc::new(TelemetryTracker::new(
        observability::TelemetryConfig::default(),
    ));
    let mut usecase = usecase_with_router_and_audit(
        Arc::new(PriorityFallbackRouter {
            providers: vec![failing, busy],
        }),
        audit.clone(),
    );
    usecase.telemetry = Some(telemetry.clone());
    let mut req = request();
    req.stream = false;
    req.metadata.cacheable = false;

    let error = usecase
        .execute(req)
        .await
        .expect_err("attempts exhaust after local backpressure");
    holder.abort();

    assert!(error.to_string().contains("all providers exhausted"));
    assert_eq!(audit.entries.lock().unwrap().len(), 1);
    assert_eq!(
        audit.entries.lock().unwrap()[0].provider,
        ProviderId::new("upstream-failure")
    );
    assert_eq!(
        audit.entries.lock().unwrap()[0].status,
        RequestStatus::Failure
    );
    let summary = telemetry
        .get_provider_summary(&ProviderId::new("upstream-failure"))
        .expect("upstream failure observation");
    assert_eq!(summary.failure_count, 1);
    assert!(
        telemetry
            .get_provider_summary(&ProviderId::new("test-provider"))
            .is_none(),
        "local backpressure must not be attributed as provider failure"
    );
}

struct SequenceRouter {
    providers: Vec<Arc<dyn ProviderPort>>,
    index: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl RouterPort for SequenceRouter {
    async fn select(&self, _req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        let idx = self.index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.providers
            .get(idx)
            .cloned()
            .ok_or_else(CortexError::all_providers_exhausted)
    }

    async fn select_excluding(
        &self,
        _req: &CompletionRequest,
        _excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        let idx = self.index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.providers
            .get(idx)
            .cloned()
            .ok_or_else(CortexError::all_providers_exhausted)
    }

    async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}

    fn providers(&self) -> Vec<ProviderId> {
        self.providers.iter().map(|p| p.id().clone()).collect()
    }
}

#[tokio::test]
async fn terminal_stop_records_earlier_saved_provider_failure() {
    let failing = Arc::new(TestProvider {
        id: ProviderId::new("upstream-failure"),
        complete_error: Some(CortexError::provider("transient upstream failure")),
        stream_error: None,
    });
    let disallowed = Arc::new(TestProvider {
        id: ProviderId::new("disallowed-provider"),
        complete_error: None,
        stream_error: None,
    });
    let audit = Arc::new(TestAudit {
        entries: Mutex::new(Vec::new()),
    });
    let telemetry = Arc::new(TelemetryTracker::new(
        observability::TelemetryConfig::default(),
    ));
    let mut usecase = usecase_with_router_and_audit(
        Arc::new(SequenceRouter {
            providers: vec![failing, disallowed],
            index: std::sync::atomic::AtomicUsize::new(0),
        }),
        audit.clone(),
    );
    usecase.telemetry = Some(telemetry.clone());
    let mut req = request();
    req.stream = false;
    req.metadata.cacheable = false;
    // Disallow the second provider so selection stops via AttemptPreparation::Stop
    req.restrictions.allowed_providers = vec![ProviderId::new("upstream-failure")];

    let error = usecase
        .execute(req)
        .await
        .expect_err("selection stops when only an unallowed provider remains");

    assert!(
        error.is_forbidden(),
        "expected forbidden error from restricted provider, got {error}"
    );
    let entries = audit.entries.lock().unwrap();
    assert_eq!(
        entries.len(),
        1,
        "earlier upstream failure must be recorded even when subsequent provider stops via AttemptPreparation::Stop"
    );
    assert_eq!(entries[0].provider, ProviderId::new("upstream-failure"));
    assert_eq!(entries[0].status, RequestStatus::Failure);
    let summary = telemetry
        .get_provider_summary(&ProviderId::new("upstream-failure"))
        .expect("telemetry observation for upstream failure");
    assert_eq!(summary.failure_count, 1);
}

#[tokio::test]
async fn non_retryable_error_from_later_provider_records_earlier_saved_failure() {
    let first_failing = Arc::new(TestProvider {
        id: ProviderId::new("retryable-failure"),
        complete_error: Some(CortexError::provider("retryable failure")),
        stream_error: None,
    });
    let second_failing = Arc::new(TestProvider {
        id: ProviderId::new("non-retryable-failure"),
        complete_error: Some(CortexError::invalid_request("client error")),
        stream_error: None,
    });
    let audit = Arc::new(TestAudit {
        entries: Mutex::new(Vec::new()),
    });
    let telemetry = Arc::new(TelemetryTracker::new(
        observability::TelemetryConfig::default(),
    ));
    let mut usecase = usecase_with_router_and_audit(
        Arc::new(PriorityFallbackRouter {
            providers: vec![first_failing, second_failing],
        }),
        audit.clone(),
    );
    usecase.telemetry = Some(telemetry.clone());
    let mut req = request();
    req.stream = false;
    req.metadata.cacheable = false;

    let error = usecase
        .execute(req)
        .await
        .expect_err("non-retryable failure terminates request");

    assert!(
        error.is_invalid_request(),
        "expected non-retryable error, got {error}"
    );
    let entries = audit.entries.lock().unwrap();
    assert_eq!(
        entries.len(),
        2,
        "both the earlier retryable failure and the later non-retryable failure must be recorded"
    );
    assert_eq!(entries[0].provider, ProviderId::new("retryable-failure"));
    assert_eq!(entries[0].status, RequestStatus::Failure);
    assert_eq!(
        entries[1].provider,
        ProviderId::new("non-retryable-failure")
    );
    assert_eq!(entries[1].status, RequestStatus::Failure);

    let first_summary = telemetry
        .get_provider_summary(&ProviderId::new("retryable-failure"))
        .expect("telemetry for first provider");
    assert_eq!(first_summary.failure_count, 1);

    let second_summary = telemetry
        .get_provider_summary(&ProviderId::new("non-retryable-failure"))
        .expect("telemetry for second provider");
    assert_eq!(second_summary.failure_count, 1);
}

#[tokio::test]
async fn policy_selected_completion_falls_back_after_local_backpressure() {
    let (primary, holder) = busy_limited_test_provider().await;
    let secondary = ordinary_provider("policy-secondary");
    let usecase = usecase_with_router(Arc::new(crate::FallbackRouter::new(
        vec![primary, secondary],
        rook_core::PolicyKind::Priority,
    )));
    let mut req = request();
    req.stream = false;
    req.metadata.cacheable = false;

    let response = usecase
        .execute(req)
        .await
        .expect("an eligible provider should accept the request");
    holder.abort();

    assert_eq!(response.provider, ProviderId::new("policy-secondary"));
}

#[tokio::test]
async fn policy_selected_stream_falls_back_after_local_backpressure() {
    let (primary, holder) = busy_limited_test_provider().await;
    let secondary: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("policy-secondary"),
        complete_error: None,
        stream_error: None,
    });
    let usecase = usecase_with_router(Arc::new(crate::FallbackRouter::new(
        vec![primary, secondary],
        rook_core::PolicyKind::Priority,
    )));
    let mut req = request();
    req.stream = true;
    req.metadata.cacheable = false;

    let mut stream = usecase
        .execute_stream(req)
        .await
        .expect("an eligible provider should accept the stream");
    let first_chunk = stream.next().await.expect("first stream chunk").unwrap();
    holder.abort();

    assert_eq!(first_chunk.delta, "hel");
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
    usecase_with_router_and_audit(
        router,
        Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        }),
    )
}

fn usecase_with_router_and_provider_repository(
    router: Arc<dyn RouterPort>,
    provider_repository: Arc<dyn ProviderRepositoryPort>,
) -> RouteRequest {
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
        Some(provider_repository),
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
}

async fn evaluate_eligibility(
    usecase: &RouteRequest,
    req: &CompletionRequest,
) -> crate::EligibilityResult {
    usecase
        .eligibility_result(req)
        .await
        .expect("connection eligibility state is available")
        .expect("FallbackRouter supplies the candidate snapshot")
        .result
}

fn usecase_with_router_and_audit(
    router: Arc<dyn RouterPort>,
    audit: Arc<dyn AuditPort>,
) -> RouteRequest {
    RouteRequest::new(
        router,
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        audit,
        None,
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
}

fn ordinary_provider(id: &str) -> Arc<dyn ProviderPort> {
    Arc::new(TestProvider {
        id: ProviderId::new(id),
        complete_error: None,
        stream_error: None,
    })
}

fn connection_bound_provider(
    provider: Arc<dyn ProviderPort>,
    connection_id: ConnectionId,
) -> Arc<dyn ProviderPort> {
    Arc::new(
        crate::provider_concurrency::ConcurrencyLimitedProvider::with_connection_id(
            provider,
            Arc::new(crate::provider_concurrency::AdmissionState::new(8)),
            connection_id,
            1,
        ),
    )
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
    fn model_capabilities(&self, _: &ModelId) -> rook_core::ModelCapabilities {
        fixture_capabilities()
    }
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
        .execute_combo(&combo, req)
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

struct BlockingAudit {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait]
impl AuditPort for BlockingAudit {
    async fn record(&self, _entry: AuditEntry) -> CortexResult<()> {
        self.entered.notify_one();
        self.release.notified().await;
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

    async fn list_routing_state(
        &self,
    ) -> Result<Vec<rook_core::ProviderConnectionRoutingState>, rook_core::RepositoryError> {
        Ok(self
            .connection_id
            .map(|id| {
                vec![rook_core::ProviderConnectionRoutingState {
                    id,
                    provider_runtime_id: ProviderId::new("test-provider"),
                    adapter_revision: 1,
                    priority: 1,
                    created_at: Utc::now(),
                    is_active: true,
                    auth_expired: false,
                }]
            })
            .unwrap_or_default())
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

struct FailingProviderRepository {
    fail_routing_state: bool,
    pending_routing_state: bool,
}

#[async_trait]
impl ProviderRepositoryPort for FailingProviderRepository {
    async fn list(&self) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(vec![])
    }

    async fn list_routing_state(
        &self,
    ) -> Result<Vec<rook_core::ProviderConnectionRoutingState>, rook_core::RepositoryError> {
        if self.pending_routing_state {
            return std::future::pending().await;
        }
        if self.fail_routing_state {
            Err(rook_core::RepositoryError::Database(
                "routing state unavailable".to_string(),
            ))
        } else {
            Ok(Vec::new())
        }
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
async fn completion_failures_update_real_router_circuit() {
    let provider_id = ProviderId::new("completion-failing-provider");
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: provider_id.clone(),
        complete_error: Some(CortexError::provider("upstream failure")),
        stream_error: None,
    });
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let usecase = usecase_with_router(router.clone());
    let mut req = request();
    req.stream = false;
    req.metadata.cacheable = false;

    for failures in 1..=3 {
        assert!(usecase.execute(req.clone()).await.is_err());
        assert_eq!(router.circuit_states()[0].1.failures, failures);
    }
    assert!(router.circuit_states()[0].1.is_open);
}

#[tokio::test]
async fn stream_failures_are_counted_when_consumed_and_open_real_router_circuit() {
    let provider_id = ProviderId::new("stream-failing-provider");
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: provider_id.clone(),
        complete_error: None,
        stream_error: Some(CortexError::provider("late stream failure")),
    });
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let usecase = usecase_with_router(router.clone());

    for attempt in 1..=3 {
        let mut stream = usecase
            .execute_stream(request())
            .await
            .expect("stream opens before upstream emits failure");
        assert!(matches!(stream.next().await, Some(Err(_))));
        assert_eq!(router.circuit_states()[0].1.failures, attempt);
    }
    assert!(router.circuit_states()[0].1.is_open);
}

#[tokio::test]
async fn real_router_holds_stream_trial_until_eof_and_releases_on_cancellation() {
    use crate::router_impl::CircuitClock;
    use std::time::Instant;

    struct TestClock(Mutex<Instant>);
    impl CircuitClock for TestClock {
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
    }

    let provider_id = ProviderId::new("half-open-stream");
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: provider_id.clone(),
        complete_error: None,
        stream_error: None,
    });
    let clock = Arc::new(TestClock(Mutex::new(Instant::now())));
    let router = Arc::new(crate::FallbackRouter::with_clock(
        vec![provider],
        rook_core::PolicyKind::Priority,
        clock.clone(),
    ));
    for _ in 0..3 {
        notify_provider_failure(
            router.as_ref(),
            &provider_id,
            &CortexError::provider("prior failure"),
        )
        .await;
    }
    *clock.0.lock().unwrap() += std::time::Duration::from_secs(3600);
    let usecase = usecase_with_router(router.clone());
    let mut stream = usecase
        .execute_stream(request())
        .await
        .expect("trial stream");
    assert!(router.try_acquire_trial(&provider_id).await.is_none());
    while stream.next().await.is_some() {}
    assert!(router.try_acquire_trial(&provider_id).await.is_some());

    for _ in 0..3 {
        notify_provider_failure(
            router.as_ref(),
            &provider_id,
            &CortexError::provider("prior failure"),
        )
        .await;
    }
    *clock.0.lock().unwrap() += std::time::Duration::from_secs(3600);
    let mut stream = usecase
        .execute_stream(request())
        .await
        .expect("second half-open stream");
    assert!(router.try_acquire_trial(&provider_id).await.is_none());
    assert!(stream.next().await.is_some());
    drop(stream);
    assert!(router.try_acquire_trial(&provider_id).await.is_some());
}

#[tokio::test]
async fn execute_stream_bypasses_cache_and_audits_final_usage() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
        stream_error: None,
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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
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
        stream_error: None,
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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
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
        stream_error: None,
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
    let result = usecase.execute(req).await;

    assert!(result.is_err());
    assert!(alias_lookup_called.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn route_request_runs_with_nullable_usage_recorder_and_warns_through_connection_lookup_failure(
) {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
        stream_error: None,
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
        Some(Arc::new(FailingProviderRepository {
            fail_routing_state: false,
            pending_routing_state: false,
        })),
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    );

    let result = usecase.execute(request()).await;

    assert!(
        result.is_ok(),
        "lookup failure should not fail routing: {result:?}"
    );
}

#[tokio::test]
async fn routing_fails_closed_when_connection_state_is_unavailable() {
    let seen_models = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(SpyProvider {
        id: ProviderId::new("managed-target"),
        model: TEST_MODEL.clone(),
        seen_models: seen_models.clone(),
    });
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(FailingProviderRepository {
        fail_routing_state: true,
        pending_routing_state: false,
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository);

    let result = usecase.execute(request()).await;

    assert!(
        result.is_err(),
        "routing must stop without connection state"
    );
    assert!(
        seen_models.lock().unwrap().is_empty(),
        "no upstream call is allowed"
    );
}

#[tokio::test]
async fn key_provider_denial_stays_forbidden_when_connection_state_is_unavailable() {
    let seen_models = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(SpyProvider {
        id: ProviderId::new("blocked-target"),
        model: TEST_MODEL.clone(),
        seen_models: seen_models.clone(),
    });
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(FailingProviderRepository {
        fail_routing_state: true,
        pending_routing_state: false,
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository);
    let mut req = request();
    req.restrictions.allowed_providers = vec![ProviderId::new("allowed-target")];

    let error = usecase
        .execute(req)
        .await
        .expect_err("key-disallowed provider must keep the existing 403 result");

    assert!(error.is_forbidden());
    assert!(seen_models.lock().unwrap().is_empty());
}

#[tokio::test]
async fn legacy_router_keeps_provider_allowlist_denial_before_state_lookup() {
    let seen_models = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: ProviderId::new("blocked-target"),
        model: TEST_MODEL.clone(),
        seen_models: seen_models.clone(),
    });
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![provider],
        connection_providers: HashMap::new(),
    });
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(FailingProviderRepository {
        fail_routing_state: true,
        pending_routing_state: false,
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository);
    let mut req = request();
    req.restrictions.allowed_providers = vec![ProviderId::new("allowed-target")];

    let error = usecase
        .execute(req)
        .await
        .expect_err("provider allowlist denial must retain its 403 classification");

    assert!(error.is_forbidden());
    assert!(seen_models.lock().unwrap().is_empty());
}

#[tokio::test]
async fn routing_state_read_timeout_fails_closed_before_upstream() {
    let seen_models = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(SpyProvider {
        id: ProviderId::new("managed-target"),
        model: TEST_MODEL.clone(),
        seen_models: seen_models.clone(),
    });
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(FailingProviderRepository {
        fail_routing_state: false,
        pending_routing_state: true,
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository);

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        usecase.execute(request()),
    )
    .await
    .expect("the connection-state read must have a deadline");

    assert!(
        result.is_err(),
        "routing must fail closed after the read deadline"
    );
    assert!(
        seen_models.lock().unwrap().is_empty(),
        "no upstream call is allowed"
    );
}

#[tokio::test]
async fn non_stream_usage_records_success_with_latency_as_ttft_and_cost_metadata() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
        stream_error: None,
    });
    let usage_recorder = Arc::new(TestUsageRecorder {
        entries: Mutex::new(Vec::new()),
        fail_recording: false,
    });
    let audit = Arc::new(TestAudit {
        entries: Mutex::new(Vec::new()),
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
        audit.clone(),
        Some(usage_recorder.clone()),
        Some(Arc::new(TestProviderRepository {
            connection_id: Some(connection_id),
            fail_lookup: false,
        })),
        None,
        Arc::new(pricing),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
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
    let audit_entries = audit.entries.lock().unwrap();
    assert_eq!(audit_entries.len(), 1);
    assert_eq!(audit_entries[0].request_id, req.id);
    assert_eq!(audit_entries[0].connection_id, Some(connection_id));
}

#[tokio::test]
async fn non_stream_success_preserves_unknown_connection_in_audit_and_usage() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
        stream_error: None,
    });
    let usage_recorder = Arc::new(TestUsageRecorder {
        entries: Mutex::new(Vec::new()),
        fail_recording: false,
    });
    let audit = Arc::new(TestAudit {
        entries: Mutex::new(Vec::new()),
    });
    let usecase = RouteRequest::new(
        Arc::new(TestRouter { provider }),
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        audit.clone(),
        Some(usage_recorder.clone()),
        None,
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    );
    let req = request();

    usecase.execute(req.clone()).await.expect("success");

    let entries = usage_recorder.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].request_id, req.id);
    assert_eq!(entries[0].connection_id, None);
    let audit_entries = audit.entries.lock().unwrap();
    assert_eq!(audit_entries.len(), 1);
    assert_eq!(audit_entries[0].request_id, req.id);
    assert_eq!(audit_entries[0].connection_id, None);
}

#[tokio::test]
async fn non_stream_usage_records_failure_with_nullable_tokens_and_ttft() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: Some(CortexError::provider("provider failed")),
        stream_error: None,
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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
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
        stream_error: None,
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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
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
        stream_error: None,
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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
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
    let usecase = usecase_with_router(Arc::new(crate::FallbackRouter::new(
        vec![ordinary_provider("test-provider")],
        rook_core::PolicyKind::Priority,
    )));
    let mut req = request();
    // test-provider is the provider selected; restrict to a different one
    req.restrictions.allowed_providers = vec![ProviderId::new("anthropic")];

    let result = usecase.execute(req).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.is_forbidden(), "expected forbidden, got: {err}");
}

#[tokio::test]
async fn execute_filters_key_disallowed_provider_before_selection() {
    let router = Arc::new(crate::FallbackRouter::new(
        vec![ordinary_provider("blocked"), ordinary_provider("allowed")],
        rook_core::PolicyKind::Priority,
    ));
    let usecase = usecase_with_router(router);
    let mut req = request();
    req.restrictions.allowed_providers = vec![ProviderId::new("allowed")];

    let response = usecase
        .execute(req)
        .await
        .expect("an allowed provider must be selected when one is eligible");

    assert_eq!(response.provider, ProviderId::new("allowed"));
}

#[tokio::test]
async fn weighted_selection_cannot_reintroduce_key_disallowed_provider() {
    let router = Arc::new(crate::FallbackRouter::new(
        vec![ordinary_provider("blocked"), ordinary_provider("allowed")],
        rook_core::PolicyKind::Weighted,
    ));
    let mut weights = std::collections::HashMap::new();
    weights.insert(ProviderId::new("blocked"), 1.0);
    weights.insert(ProviderId::new("allowed"), 0.0);
    router.update_weights(weights);

    let usecase = usecase_with_router(router);
    let mut req = request();
    req.restrictions.allowed_providers = vec![ProviderId::new("allowed")];

    // When the only remaining eligible candidate has zero weight, selection
    // must fail with an explicit weighted config error, not silently select it.
    let error = usecase
        .execute(req)
        .await
        .expect_err("zero-weight-only remaining candidate must produce a weighted config error");
    assert!(
        error.is_weighted_config_error(),
        "expected weighted config error, got: {error}"
    );
}

#[tokio::test]
async fn selection_excludes_providers_added_after_eligibility_snapshot() {
    let router = Arc::new(crate::FallbackRouter::new(
        vec![ordinary_provider("observed")],
        rook_core::PolicyKind::Priority,
    ));
    let usecase = usecase_with_router(router.clone());
    let req = request();
    let eligibility = evaluate_eligibility(&usecase, &req).await;
    rook_core::ProviderRegistryPort::replace_all(
        router.as_ref(),
        vec![ordinary_provider("added-after-snapshot")],
    )
    .expect("replace registry");

    let excluded = usecase.selection_exclusions(&req, &[], Some(&eligibility));
    let selected = router.select_excluding(&req, &excluded).await;

    assert!(excluded.contains(&ProviderId::new("added-after-snapshot")));
    assert!(
        selected.is_err(),
        "an unassessed provider must not be selected"
    );
}

#[tokio::test]
async fn eligibility_fails_closed_before_state_query_above_4000_targets() {
    let providers: Vec<_> = (0..=4_000)
        .map(|index| ordinary_provider(&format!("provider-{index}")))
        .collect();
    let router = Arc::new(crate::FallbackRouter::new(
        providers,
        rook_core::PolicyKind::Priority,
    ));
    let connection_id = ConnectionId::new();
    let repository = Arc::new(InactiveAfterFirstRoutingReadRepository {
        connection: test_connection(connection_id, ProviderId::new("unused-runtime"), true),
        reads: std::sync::atomic::AtomicUsize::new(0),
        pending_after_first: None,
        adapter_revision_after_first: None,
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository.clone());

    let result = usecase.eligibility_result(&request()).await;

    assert!(matches!(result, Err(error) if error.to_string().contains("all providers exhausted")));
    assert_eq!(
        repository.reads.load(std::sync::atomic::Ordering::SeqCst),
        0
    );

    let mut disallowed = request();
    disallowed.restrictions.allowed_providers = vec![ProviderId::new("not-registered")];
    let error = match usecase.eligibility_result(&disallowed).await {
        Ok(_) => panic!("all-disallowed candidates keep the key's forbidden result"),
        Err(error) => error,
    };
    assert!(error.is_forbidden());
    assert_eq!(
        repository.reads.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

struct RegistryRefreshOnProviderResolutionRouter {
    inner: Arc<crate::FallbackRouter>,
    replacement: Vec<Arc<dyn ProviderPort>>,
    refreshed: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl RouterPort for RegistryRefreshOnProviderResolutionRouter {
    async fn select(&self, req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        self.inner.select(req).await
    }

    async fn select_excluding(
        &self,
        req: &CompletionRequest,
        excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        self.inner.select_excluding(req, excluded).await
    }

    async fn on_failure(&self, provider: &ProviderId, error: &CortexError) {
        self.inner.on_failure(provider, error).await;
    }

    fn providers(&self) -> Vec<ProviderId> {
        self.inner.providers()
    }

    fn eligibility_candidates(&self) -> Option<Vec<rook_core::RouterCandidateSnapshot>> {
        self.inner.eligibility_candidates()
    }

    fn resolve_provider(&self, provider: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
        if !self
            .refreshed
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            rook_core::ProviderRegistryPort::replace_all(
                self.inner.as_ref(),
                self.replacement.clone(),
            )
            .expect("registry refresh succeeds");
        }
        rook_core::RouterPort::resolve_provider(self.inner.as_ref(), provider)
    }

    fn policy_kind(&self) -> rook_core::PolicyKind {
        rook_core::PolicyKind::LeastLatency
    }
}

#[tokio::test]
async fn metric_policy_reselects_when_registry_removes_selected_provider() {
    let removed_calls = Arc::new(Mutex::new(Vec::new()));
    let removed: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: ProviderId::new("removed-first"),
        model: TEST_MODEL.clone(),
        seen_models: removed_calls.clone(),
    });
    let retained_calls = Arc::new(Mutex::new(Vec::new()));
    let retained: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: ProviderId::new("retained-second"),
        model: TEST_MODEL.clone(),
        seen_models: retained_calls.clone(),
    });
    let router = Arc::new(RegistryRefreshOnProviderResolutionRouter {
        inner: Arc::new(crate::FallbackRouter::new(
            vec![removed],
            rook_core::PolicyKind::LeastLatency,
        )),
        replacement: vec![retained],
        refreshed: std::sync::atomic::AtomicBool::new(false),
    });
    let usecase = usecase_with_router(router);
    let mut req = request();
    req.metadata.cacheable = false;

    let response = usecase
        .execute(req)
        .await
        .expect("metric routing should reselect from the refreshed eligible registry");

    assert_eq!(response.provider, ProviderId::new("retained-second"));
    assert!(removed_calls.lock().unwrap().is_empty());
    assert_eq!(
        retained_calls.lock().unwrap().as_slice(),
        [TEST_MODEL.to_string()]
    );
}

struct RegistryRefreshDuringSelectionRouter {
    inner: Arc<crate::FallbackRouter>,
    replacement: Vec<Arc<dyn ProviderPort>>,
    refreshed: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl RouterPort for RegistryRefreshDuringSelectionRouter {
    async fn select(&self, req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        self.select_excluding(req, &[]).await
    }

    async fn select_excluding(
        &self,
        req: &CompletionRequest,
        excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        self.inner.select_excluding(req, excluded).await
    }

    async fn on_failure(&self, provider: &ProviderId, error: &CortexError) {
        self.inner.on_failure(provider, error).await;
    }

    fn providers(&self) -> Vec<ProviderId> {
        self.inner.providers()
    }

    fn eligibility_candidates(&self) -> Option<Vec<rook_core::RouterCandidateSnapshot>> {
        self.inner.eligibility_candidates()
    }

    fn resolve_provider(&self, provider: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
        if !self
            .refreshed
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            rook_core::ProviderRegistryPort::replace_all(
                self.inner.as_ref(),
                self.replacement.clone(),
            )
            .expect("registry refresh succeeds");
        }
        rook_core::RouterPort::resolve_provider(self.inner.as_ref(), provider)
    }
}

struct RegistryRefreshAfterEligibilitySnapshotRouter {
    inner: Arc<crate::FallbackRouter>,
    replacement: Vec<Arc<dyn ProviderPort>>,
    refreshed: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl RouterPort for RegistryRefreshAfterEligibilitySnapshotRouter {
    async fn select(&self, req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        self.inner.select(req).await
    }

    async fn select_excluding(
        &self,
        req: &CompletionRequest,
        excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        self.inner.select_excluding(req, excluded).await
    }

    async fn on_failure(&self, provider: &ProviderId, error: &CortexError) {
        self.inner.on_failure(provider, error).await;
    }

    fn providers(&self) -> Vec<ProviderId> {
        self.inner.providers()
    }

    fn eligibility_candidates(&self) -> Option<Vec<rook_core::RouterCandidateSnapshot>> {
        let snapshot = self.inner.eligibility_candidates();
        if !self
            .refreshed
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            rook_core::ProviderRegistryPort::replace_all(
                self.inner.as_ref(),
                self.replacement.clone(),
            )
            .expect("registry refresh succeeds");
        }
        snapshot
    }

    fn resolve_provider(&self, provider: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
        self.inner.resolve_provider(provider)
    }
}

#[tokio::test]
async fn refreshed_same_id_provider_is_reassessed_before_all_rejected_error() {
    let target = ProviderId::new("same-provider-id");
    let rejected_calls = Arc::new(Mutex::new(Vec::new()));
    let rejected: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: target.clone(),
        model: ModelId::new("other-model"),
        seen_models: rejected_calls.clone(),
    });
    let eligible_calls = Arc::new(Mutex::new(Vec::new()));
    let replacement: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: target.clone(),
        model: TEST_MODEL.clone(),
        seen_models: eligible_calls.clone(),
    });
    let router = Arc::new(RegistryRefreshAfterEligibilitySnapshotRouter {
        inner: Arc::new(crate::FallbackRouter::new(
            vec![rejected],
            rook_core::PolicyKind::Priority,
        )),
        replacement: vec![replacement],
        refreshed: std::sync::atomic::AtomicBool::new(false),
    });
    let usecase = usecase_with_router(router);
    let mut req = request();
    req.metadata.cacheable = false;

    let response = usecase
        .execute(req)
        .await
        .expect("the current provider snapshot should be reevaluated");

    assert_eq!(response.provider, target);
    assert!(rejected_calls.lock().unwrap().is_empty());
    assert_eq!(
        eligible_calls.lock().unwrap().as_slice(),
        [TEST_MODEL.to_string()]
    );
}

/// Upper bound on candidate snapshot reads during a churned request.
///
/// The routing stage evaluates a snapshot, validates it against the registry,
/// and allows one re-evaluation before it fails closed. Each evaluation reads
/// the candidate list a small number of times, so the bound scales with that
/// fixed retry budget instead of pinning one exact call pattern.
const MAX_CHURN_CANDIDATE_READS: usize = 12;

#[tokio::test]
async fn eligibility_registry_churn_is_rechecked_at_most_once() {
    let target = ProviderId::new("churning-provider");
    let first_calls = Arc::new(Mutex::new(Vec::new()));
    let second_calls = Arc::new(Mutex::new(Vec::new()));
    let first: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: target.clone(),
        model: TEST_MODEL.clone(),
        seen_models: first_calls.clone(),
    });
    let second: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: target,
        model: TEST_MODEL.clone(),
        seen_models: second_calls.clone(),
    });
    let router = Arc::new(RegistryChurnDuringEligibilityRouter {
        inner: Arc::new(crate::FallbackRouter::new(
            vec![first.clone()],
            rook_core::PolicyKind::Priority,
        )),
        providers: [first, second],
        candidate_reads: std::sync::atomic::AtomicUsize::new(0),
        exposed_providers: Mutex::new(Vec::new()),
    });
    let usecase = usecase_with_router(router.clone());
    let mut req = request();
    req.metadata.cacheable = false;

    let error = usecase
        .execute(req)
        .await
        .expect_err("repeated registry churn must fail closed after one refresh");

    assert!(error.to_string().contains("all providers exhausted"));
    assert!(first_calls.lock().unwrap().is_empty());
    assert!(second_calls.lock().unwrap().is_empty());
    assert!(
        router
            .candidate_reads
            .load(std::sync::atomic::Ordering::SeqCst)
            <= MAX_CHURN_CANDIDATE_READS,
        "churn must stop after one refresh instead of retrying without a bound: reads={}",
        router
            .candidate_reads
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    assert!(
        router.exposed_providers.lock().unwrap().len() >= 2,
        "the registry must actually churn for this test to cover the refresh path"
    );

    let stream_router = Arc::new(RegistryChurnDuringEligibilityRouter {
        inner: Arc::new(crate::FallbackRouter::new(
            vec![router.providers[0].clone()],
            rook_core::PolicyKind::Priority,
        )),
        providers: router.providers.clone(),
        candidate_reads: std::sync::atomic::AtomicUsize::new(0),
        exposed_providers: Mutex::new(Vec::new()),
    });
    let stream_usecase = usecase_with_router(stream_router.clone());
    let mut stream_req = request();
    stream_req.stream = true;
    stream_req.metadata.cacheable = false;

    let error = match stream_usecase.execute_stream(stream_req).await {
        Ok(_) => panic!("stream selection must stop after one snapshot refresh"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("all providers exhausted"));
    assert!(first_calls.lock().unwrap().is_empty());
    assert!(second_calls.lock().unwrap().is_empty());
    assert!(
        stream_router
            .candidate_reads
            .load(std::sync::atomic::Ordering::SeqCst)
            <= MAX_CHURN_CANDIDATE_READS,
        "stream selection must also stop after one snapshot re-evaluation"
    );
    assert!(
        stream_router.exposed_providers.lock().unwrap().len() >= 2,
        "the registry must actually churn for this test to cover the refresh path"
    );
}

struct RegistryChurnDuringEligibilityRouter {
    inner: Arc<crate::FallbackRouter>,
    providers: [Arc<dyn ProviderPort>; 2],
    candidate_reads: std::sync::atomic::AtomicUsize,
    exposed_providers: Mutex<Vec<ProviderId>>,
}

#[async_trait]
impl RouterPort for RegistryChurnDuringEligibilityRouter {
    async fn select(&self, req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        self.inner.select(req).await
    }

    async fn select_excluding(
        &self,
        req: &CompletionRequest,
        excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        self.inner.select_excluding(req, excluded).await
    }

    async fn on_failure(&self, provider: &ProviderId, error: &CortexError) {
        self.inner.on_failure(provider, error).await;
    }

    fn providers(&self) -> Vec<ProviderId> {
        self.inner.providers()
    }

    fn eligibility_candidates(&self) -> Option<Vec<rook_core::RouterCandidateSnapshot>> {
        let snapshot = self.inner.eligibility_candidates();
        let index = self
            .candidate_reads
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let provider = self.providers[index % 2].clone();
        self.exposed_providers
            .lock()
            .unwrap()
            .push(provider.id().clone());
        rook_core::ProviderRegistryPort::replace_all(self.inner.as_ref(), vec![provider])
            .expect("registry refresh succeeds");
        snapshot
    }
}

#[tokio::test]
async fn provider_added_during_selection_is_checked_before_upstream_io() {
    let new_connection_id = ConnectionId::new();
    let new_runtime_id = ProviderId::new("late-provider-runtime");
    let original_calls = Arc::new(Mutex::new(Vec::new()));
    let late_calls = Arc::new(Mutex::new(Vec::new()));
    let original: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: ProviderId::new("original-provider"),
        model: TEST_MODEL.clone(),
        seen_models: original_calls.clone(),
    });
    let placeholder: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: ProviderId::new("unsupported-placeholder"),
        model: ModelId::new("other-model"),
        seen_models: Arc::new(Mutex::new(Vec::new())),
    });
    let late_provider = connection_bound_provider(
        Arc::new(SpyProvider {
            id: ProviderId::new(new_connection_id.to_string()),
            model: TEST_MODEL.clone(),
            seen_models: late_calls.clone(),
        }),
        new_connection_id,
    );
    let router = Arc::new(RegistryRefreshDuringSelectionRouter {
        inner: Arc::new(crate::FallbackRouter::new(
            vec![original.clone(), placeholder],
            rook_core::PolicyKind::Priority,
        )),
        replacement: vec![late_provider, original.clone()],
        refreshed: std::sync::atomic::AtomicBool::new(false),
    });
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(QuotaAwareProviderRepository {
        connections: HashMap::from([(
            new_connection_id,
            test_connection(new_connection_id, new_runtime_id.clone(), true),
        )]),
        runtime_connections: HashMap::new(),
    });
    let now = Utc::now();
    let quota_reader: Arc<dyn QuotaObservationRecorderPort> =
        Arc::new(FakeQuotaStore::new(vec![quota_observation(
            new_connection_id,
            new_runtime_id,
            0.0,
            now,
            Some(now + chrono::Duration::seconds(60)),
        )]));
    let usecase = usecase_with_router_and_provider_repository(router, repository)
        .with_quota_observation_reader(quota_reader);
    let mut req = request();
    req.metadata.cacheable = false;

    let response = usecase
        .execute(req)
        .await
        .expect("the original eligible provider should serve the request");

    assert_eq!(response.provider, ProviderId::new("original-provider"));
    assert_eq!(
        original_calls.lock().unwrap().as_slice(),
        [TEST_MODEL.to_string()]
    );
    assert!(late_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn provider_replaced_with_managed_adapter_after_snapshot_is_not_called() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("replacement-runtime");
    let target = ProviderId::new(connection_id.to_string());
    let replaced_calls = Arc::new(Mutex::new(Vec::new()));
    let placeholder: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: ProviderId::new("unsupported-placeholder"),
        model: ModelId::new("other-model"),
        seen_models: Arc::new(Mutex::new(Vec::new())),
    });
    let original: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: target.clone(),
        model: TEST_MODEL.clone(),
        seen_models: replaced_calls.clone(),
    });
    let managed_calls = Arc::new(Mutex::new(Vec::new()));
    let replacement = connection_bound_provider(
        Arc::new(SpyProvider {
            id: target.clone(),
            model: TEST_MODEL.clone(),
            seen_models: managed_calls.clone(),
        }),
        connection_id,
    );
    let router = Arc::new(RegistryRefreshDuringSelectionRouter {
        inner: Arc::new(crate::FallbackRouter::new(
            vec![original, placeholder.clone()],
            rook_core::PolicyKind::Priority,
        )),
        replacement: vec![replacement, placeholder],
        refreshed: std::sync::atomic::AtomicBool::new(false),
    });
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(QuotaAwareProviderRepository {
        connections: HashMap::from([(
            connection_id,
            test_connection(connection_id, runtime_id.clone(), true),
        )]),
        runtime_connections: HashMap::new(),
    });
    let now = Utc::now();
    let quota_reader: Arc<dyn QuotaObservationRecorderPort> =
        Arc::new(FakeQuotaStore::new(vec![quota_observation(
            connection_id,
            runtime_id,
            0.0,
            now,
            Some(now + chrono::Duration::seconds(60)),
        )]));
    let usecase = usecase_with_router_and_provider_repository(router, repository)
        .with_quota_observation_reader(quota_reader);
    let mut req = request();
    req.metadata.cacheable = false;

    let error = usecase
        .execute(req)
        .await
        .expect_err("replacement adapter must be evaluated before it receives traffic");

    assert!(error.to_string().contains("all providers exhausted"));
    assert!(replaced_calls.lock().unwrap().is_empty());
    assert!(managed_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn eligible_replacement_adapter_is_reassessed_without_permanent_exclusion() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("eligible-replacement-runtime");
    let target = ProviderId::new(connection_id.to_string());
    let placeholder = Arc::new(SpyProvider {
        id: ProviderId::new("unsupported-placeholder"),
        model: ModelId::new("other-model"),
        seen_models: Arc::new(Mutex::new(Vec::new())),
    }) as Arc<dyn ProviderPort>;
    let original = ordinary_provider(target.as_str());
    let replacement_calls = Arc::new(Mutex::new(Vec::new()));
    let replacement = connection_bound_provider(
        Arc::new(SpyProvider {
            id: target.clone(),
            model: TEST_MODEL.clone(),
            seen_models: replacement_calls.clone(),
        }),
        connection_id,
    );
    let router = Arc::new(RegistryRefreshDuringSelectionRouter {
        inner: Arc::new(crate::FallbackRouter::new(
            vec![original, placeholder.clone()],
            rook_core::PolicyKind::Priority,
        )),
        replacement: vec![replacement, placeholder],
        refreshed: std::sync::atomic::AtomicBool::new(false),
    });
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(QuotaAwareProviderRepository {
        connections: HashMap::from([(
            connection_id,
            test_connection(connection_id, runtime_id, true),
        )]),
        runtime_connections: HashMap::new(),
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository);
    let mut req = request();
    req.metadata.cacheable = false;

    let response = usecase
        .execute(req)
        .await
        .expect("a refreshed adapter should be reassessed and remain eligible");

    assert_eq!(response.provider, target);
    assert_eq!(
        replacement_calls.lock().unwrap().as_slice(),
        [TEST_MODEL.to_string()]
    );
}

#[tokio::test]
async fn eligibility_stage_reports_key_allowlist_reasons() {
    let router = Arc::new(crate::FallbackRouter::new(
        vec![ordinary_provider("one"), ordinary_provider("two")],
        rook_core::PolicyKind::Priority,
    ));
    let usecase = usecase_with_router(router);
    let mut req = request();
    req.restrictions.allowed_models = vec![ModelId::new("not-requested")];

    let model_result = evaluate_eligibility(&usecase, &req).await;

    assert!(model_result.eligible.is_empty());
    assert_eq!(model_result.rejected.len(), 2);
    assert!(model_result
        .rejected
        .iter()
        .all(|rejection| rejection.reason == crate::RejectionReason::KeyModelNotAllowed));

    req.restrictions.allowed_models.clear();
    req.restrictions.allowed_providers = vec![ProviderId::new("not-registered")];
    let provider_result = evaluate_eligibility(&usecase, &req).await;

    assert!(provider_result.eligible.is_empty());
    assert_eq!(provider_result.rejected.len(), 2);
    assert!(provider_result
        .rejected
        .iter()
        .all(|rejection| { rejection.reason == crate::RejectionReason::KeyProviderNotAllowed }));
}

#[tokio::test]
async fn eligibility_does_not_read_quota_for_key_disallowed_connections() {
    let blocked_connection = ConnectionId::new();
    let allowed_connection = ConnectionId::new();
    let blocked_runtime = ProviderId::new("blocked-runtime");
    let allowed_runtime = ProviderId::new("allowed-runtime");
    let blocked_target = ProviderId::new(blocked_connection.to_string());
    let allowed_target = ProviderId::new(allowed_connection.to_string());
    let router = Arc::new(crate::FallbackRouter::new(
        vec![
            connection_bound_provider(
                ordinary_provider(blocked_target.as_str()),
                blocked_connection,
            ),
            connection_bound_provider(
                ordinary_provider(allowed_target.as_str()),
                allowed_connection,
            ),
        ],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(QuotaAwareProviderRepository {
        connections: HashMap::from([
            (
                blocked_connection,
                test_connection(blocked_connection, blocked_runtime, true),
            ),
            (
                allowed_connection,
                test_connection(allowed_connection, allowed_runtime, true),
            ),
        ]),
        runtime_connections: HashMap::new(),
    });
    let quota_store = Arc::new(FakeQuotaStore::new(Vec::new()));
    let quota_reader: Arc<dyn QuotaObservationRecorderPort> = quota_store.clone();
    let usecase = usecase_with_router_and_provider_repository(router, repository)
        .with_quota_observation_reader(quota_reader);
    let mut req = request();
    req.restrictions.allowed_providers = vec![allowed_target.clone()];

    let result = evaluate_eligibility(&usecase, &req).await;

    assert_eq!(result.eligible, vec![crate::TargetRef(allowed_target)]);
    assert_eq!(result.rejected.len(), 1);
    assert_eq!(
        result.rejected[0].reason,
        crate::RejectionReason::KeyProviderNotAllowed
    );
    let queries = quota_store.queries.lock().unwrap();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].connection_ids, vec![allowed_connection]);
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
    let usecase = usecase_with_router(Arc::new(crate::FallbackRouter::new(
        vec![ordinary_provider("test-provider")],
        rook_core::PolicyKind::Priority,
    )));
    let mut req = request();
    req.restrictions.allowed_providers = vec![ProviderId::new("openai")];

    let result = usecase.execute_stream(req).await;
    assert!(result.is_err());
    assert!(result.err().unwrap().is_forbidden());
}

#[tokio::test]
async fn execute_stream_filters_key_disallowed_provider_before_selection() {
    let router = Arc::new(crate::FallbackRouter::new(
        vec![ordinary_provider("blocked"), ordinary_provider("allowed")],
        rook_core::PolicyKind::Priority,
    ));
    let usecase = usecase_with_router(router);
    let mut req = request();
    req.stream = true;
    req.restrictions.allowed_providers = vec![ProviderId::new("allowed")];

    let result = usecase.execute_stream(req).await;

    assert!(
        result.is_ok(),
        "an allowed provider must serve the stream: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn eligibility_stage_reports_open_circuit_and_eligible_targets() {
    let open_provider = ordinary_provider("open-circuit");
    let open_provider_id = open_provider.id().clone();
    let eligible_provider = ordinary_provider("eligible");
    let eligible_provider_id = eligible_provider.id().clone();
    let router = Arc::new(crate::FallbackRouter::new(
        vec![open_provider, eligible_provider],
        rook_core::PolicyKind::Priority,
    ));
    for _ in 0..3 {
        router
            .on_failure(&open_provider_id, &CortexError::provider("failure"))
            .await;
    }
    let usecase = usecase_with_router(router);

    let result = evaluate_eligibility(&usecase, &request()).await;

    assert_eq!(
        result.eligible,
        vec![crate::TargetRef(eligible_provider_id)]
    );
    assert_eq!(
        result.rejected,
        vec![crate::Rejection {
            target: crate::TargetRef(open_provider_id),
            reason: crate::RejectionReason::CircuitOpen,
        }]
    );
}

#[tokio::test]
async fn eligibility_rechecks_circuit_after_connection_state_read() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("cooldown-runtime");
    let target = ProviderId::new(connection_id.to_string());
    let clock = Arc::new(EligibilityTestClock(Mutex::new(std::time::Instant::now())));
    let provider = connection_bound_provider(ordinary_provider(target.as_str()), connection_id);
    let router = Arc::new(crate::FallbackRouter::with_clock(
        vec![provider],
        rook_core::PolicyKind::Priority,
        clock.clone(),
    ));
    for _ in 0..3 {
        router
            .on_failure(&target, &CortexError::provider("failure"))
            .await;
    }
    let connection = test_connection(connection_id, runtime_id.clone(), true);
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(EligibilityAdvancingRoutingStateRepository {
            states: vec![rook_core::ProviderConnectionRoutingState {
                id: connection.id,
                provider_runtime_id: connection.provider_runtime_id,
                adapter_revision: 1,
                priority: connection.priority,
                created_at: connection.created_at,
                is_active: connection.is_active,
                auth_expired: false,
            }],
            clock: clock.clone(),
        });
    let usecase = usecase_with_router_and_provider_repository(router, repository);

    let result = evaluate_eligibility(&usecase, &request()).await;

    assert_eq!(result.eligible, vec![crate::TargetRef(target)]);
    assert!(result.rejected.is_empty());
}

#[tokio::test]
async fn eligibility_stage_reports_disabled_missing_and_expired_auth() {
    let refreshable_expired_id = ConnectionId::new();
    let unavailable_expired_id = ConnectionId::new();
    let disabled_id = ConnectionId::new();
    let refreshable_expired_provider = connection_bound_provider(
        ordinary_provider(&refreshable_expired_id.to_string()),
        refreshable_expired_id,
    );
    let refreshable_expired_provider_id = refreshable_expired_provider.id().clone();
    let unavailable_expired_provider = connection_bound_provider(
        Arc::new(UnavailableProvider {
            inner: ordinary_provider(&unavailable_expired_id.to_string()),
        }),
        unavailable_expired_id,
    );
    let missing_provider: Arc<dyn ProviderPort> = Arc::new(UnavailableProvider {
        inner: ordinary_provider("missing-auth"),
    });
    let disabled_provider =
        connection_bound_provider(ordinary_provider(&disabled_id.to_string()), disabled_id);
    let eligible_provider = ordinary_provider("eligible-auth");
    let router = Arc::new(crate::FallbackRouter::new(
        vec![
            refreshable_expired_provider,
            unavailable_expired_provider,
            missing_provider,
            disabled_provider,
            eligible_provider.clone(),
        ],
        rook_core::PolicyKind::Priority,
    ));

    let mut refreshable_expired_connection = test_connection(
        refreshable_expired_id,
        ProviderId::new("refreshable-expired-runtime"),
        true,
    );
    refreshable_expired_connection.test_status = rook_core::TestStatus::Expired {
        last_test_at: Utc::now(),
        expires_at: Utc::now().timestamp() - 1,
    };
    let mut unavailable_expired_connection = test_connection(
        unavailable_expired_id,
        ProviderId::new("unavailable-expired-runtime"),
        true,
    );
    unavailable_expired_connection.test_status = rook_core::TestStatus::Expired {
        last_test_at: Utc::now(),
        expires_at: Utc::now().timestamp() - 1,
    };
    let disabled_connection =
        test_connection(disabled_id, ProviderId::new("disabled-runtime"), false);
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([
            (refreshable_expired_id, refreshable_expired_connection),
            (unavailable_expired_id, unavailable_expired_connection),
            (disabled_id, disabled_connection),
        ]),
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository);

    let result = evaluate_eligibility(&usecase, &request()).await;

    assert_eq!(
        result.eligible,
        vec![
            crate::TargetRef(refreshable_expired_provider_id),
            crate::TargetRef(eligible_provider.id().clone()),
        ]
    );
    assert_eq!(result.rejected.len(), 3);
    assert!(result.rejected.contains(&crate::Rejection {
        target: crate::TargetRef(ProviderId::new(unavailable_expired_id.to_string())),
        reason: crate::RejectionReason::AuthExpired,
    }));
    assert!(result.rejected.contains(&crate::Rejection {
        target: crate::TargetRef(ProviderId::new("missing-auth")),
        reason: crate::RejectionReason::AuthMissing,
    }));
    assert!(result.rejected.contains(&crate::Rejection {
        target: crate::TargetRef(ProviderId::new(disabled_id.to_string())),
        reason: crate::RejectionReason::ConnectionDisabled,
    }));
}

#[tokio::test]
async fn eligibility_rejects_managed_adapter_with_stale_configuration_revision() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("revisioned-runtime");
    let target = ProviderId::new(connection_id.to_string());
    let provider = connection_bound_provider(ordinary_provider(target.as_str()), connection_id);
    let mut connection = test_connection(connection_id, runtime_id, true);
    connection.adapter_revision = 2;
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([(connection_id, connection)]),
    });
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let usecase = usecase_with_router_and_provider_repository(router, repository);

    let result = evaluate_eligibility(&usecase, &request()).await;

    assert!(result.eligible.is_empty());
    assert_eq!(
        result.rejected,
        vec![crate::Rejection {
            target: crate::TargetRef(target),
            reason: crate::RejectionReason::ConnectionDisabled,
        }]
    );
}

#[tokio::test]
async fn eligibility_rejects_missing_state_only_for_connection_bound_targets() {
    let missing_connection_id = ConnectionId::new();
    let managed_target = ProviderId::new(missing_connection_id.to_string());
    let managed_provider: Arc<dyn ProviderPort> = Arc::new(
        crate::provider_concurrency::ConcurrencyLimitedProvider::with_connection_id(
            ordinary_provider(managed_target.as_str()),
            Arc::new(crate::provider_concurrency::AdmissionState::new(1)),
            missing_connection_id,
            1,
        ),
    );
    let static_target = ProviderId::new(ConnectionId::new().to_string());
    let static_provider = ordinary_provider(static_target.as_str());
    let router = Arc::new(crate::FallbackRouter::new(
        vec![managed_provider, static_provider],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::new(),
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository);

    let result = evaluate_eligibility(&usecase, &request()).await;

    assert_eq!(result.eligible, vec![crate::TargetRef(static_target)]);
    assert_eq!(
        result.rejected,
        vec![crate::Rejection {
            target: crate::TargetRef(managed_target),
            reason: crate::RejectionReason::ConnectionDisabled,
        }]
    );
}

#[tokio::test]
async fn unmanaged_uuid_provider_is_not_attributed_to_a_connection_with_the_same_id() {
    let connection_id = ConnectionId::new();
    let target = ProviderId::new(connection_id.to_string());
    let provider = ordinary_provider(target.as_str());
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([(
            connection_id,
            test_connection(connection_id, ProviderId::new("managed-runtime"), true),
        )]),
    });
    let audit = Arc::new(TestAudit {
        entries: Mutex::new(Vec::new()),
    });
    let usecase = RouteRequest::new(
        router,
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        audit.clone(),
        None,
        Some(repository),
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    );
    let mut req = request();
    req.metadata.cacheable = false;

    usecase
        .execute(req)
        .await
        .expect("static provider succeeds");

    let entries = audit.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].provider, target);
    assert_eq!(entries[0].connection_id, None);
}

#[tokio::test]
async fn unmanaged_uuid_provider_is_not_bound_by_runtime_id_collision() {
    let target = ProviderId::new(ConnectionId::new().to_string());
    let connection_id = ConnectionId::new();
    let provider = ordinary_provider(target.as_str());
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([(
            connection_id,
            test_connection(connection_id, target.clone(), true),
        )]),
    });
    let now = Utc::now();
    let quota_reader: Arc<dyn QuotaObservationRecorderPort> =
        Arc::new(FakeQuotaStore::new(vec![quota_observation(
            connection_id,
            target.clone(),
            0.0,
            now,
            Some(now + chrono::Duration::seconds(60)),
        )]));
    let audit = Arc::new(TestAudit {
        entries: Mutex::new(Vec::new()),
    });
    let usecase = RouteRequest::new(
        router,
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        audit.clone(),
        None,
        Some(repository),
        None,
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_quota_observation_reader(quota_reader);
    let mut req = request();
    req.metadata.cacheable = false;

    usecase
        .execute(req)
        .await
        .expect("an unmanaged UUID provider is not bound to a runtime-id match");

    let entries = audit.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].provider, target);
    assert_eq!(entries[0].connection_id, None);
}

#[tokio::test]
async fn managed_provider_disabled_after_eligibility_read_is_not_called() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("changing-runtime");
    let connection = test_connection(connection_id, runtime_id, true);
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(InactiveAfterFirstRoutingReadRepository {
            connection,
            reads: std::sync::atomic::AtomicUsize::new(0),
            pending_after_first: None,
            adapter_revision_after_first: None,
        });
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = connection_bound_provider(
        Arc::new(SpyProvider {
            id: ProviderId::new(connection_id.to_string()),
            model: TEST_MODEL.clone(),
            seen_models: calls.clone(),
        }),
        connection_id,
    );
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let usecase = usecase_with_router_and_provider_repository(router, repository);
    let mut req = request();
    req.metadata.cacheable = false;

    let error = usecase
        .execute(req)
        .await
        .expect_err("newly disabled connection must not reach the provider");

    assert!(error.to_string().contains("all providers exhausted"));
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn managed_provider_revision_change_after_eligibility_is_not_called() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("changing-revision-runtime");
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(InactiveAfterFirstRoutingReadRepository {
            connection: test_connection(connection_id, runtime_id, true),
            reads: std::sync::atomic::AtomicUsize::new(0),
            pending_after_first: None,
            adapter_revision_after_first: Some(2),
        });
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = connection_bound_provider(
        Arc::new(SpyProvider {
            id: ProviderId::new(connection_id.to_string()),
            model: TEST_MODEL.clone(),
            seen_models: calls.clone(),
        }),
        connection_id,
    );
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let usecase = usecase_with_router_and_provider_repository(router, repository);
    let mut req = request();
    req.metadata.cacheable = false;

    let error = usecase
        .execute(req)
        .await
        .expect_err("stale provider configuration must not reach upstream");

    assert!(error.to_string().contains("all providers exhausted"));
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn runtime_matched_provider_disabled_after_eligibility_read_is_not_called() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("runtime-matched-provider");
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(InactiveAfterFirstRoutingReadRepository {
            connection: test_connection(connection_id, runtime_id.clone(), true),
            reads: std::sync::atomic::AtomicUsize::new(0),
            pending_after_first: None,
            adapter_revision_after_first: None,
        });
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: runtime_id.clone(),
        model: TEST_MODEL.clone(),
        seen_models: calls.clone(),
    });
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let usecase = usecase_with_router_and_provider_repository(router, repository);
    let mut req = request();
    req.metadata.cacheable = false;

    let error = usecase
        .execute(req)
        .await
        .expect_err("runtime-matched disabled connection must not receive the request");

    assert!(error.to_string().contains("all providers exhausted"));
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn runtime_matched_provider_revision_change_triggers_one_fresh_eligibility_pass() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("changing-runtime-revision");
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(InactiveAfterFirstRoutingReadRepository {
            connection: test_connection(connection_id, runtime_id.clone(), true),
            reads: std::sync::atomic::AtomicUsize::new(0),
            pending_after_first: None,
            adapter_revision_after_first: Some(2),
        });
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: runtime_id.clone(),
        model: TEST_MODEL.clone(),
        seen_models: calls.clone(),
    });
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let usecase = usecase_with_router_and_provider_repository(router, repository);
    let mut req = request();
    req.metadata.cacheable = false;

    let response = usecase
        .execute(req)
        .await
        .expect("runtime-matched state change is reassessed before routing");

    assert_eq!(response.provider, runtime_id);
    assert_eq!(calls.lock().unwrap().as_slice(), [TEST_MODEL.to_string()]);
}

#[tokio::test]
async fn legacy_router_rechecks_runtime_matched_connection_before_upstream() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("legacy-runtime-provider");
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(InactiveAfterFirstRoutingReadRepository {
            connection: test_connection(connection_id, runtime_id.clone(), true),
            reads: std::sync::atomic::AtomicUsize::new(0),
            pending_after_first: None,
            adapter_revision_after_first: None,
        });
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: runtime_id,
        model: TEST_MODEL.clone(),
        seen_models: calls.clone(),
    });
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![provider],
        connection_providers: HashMap::new(),
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository);
    let mut req = request();
    req.metadata.cacheable = false;

    let error = usecase
        .execute(req)
        .await
        .expect_err("disabled runtime-matched connection must not receive the request");

    assert!(error.to_string().contains("all providers exhausted"));
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn legacy_stream_router_rechecks_runtime_matched_connection_before_upstream() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("legacy-stream-runtime-provider");
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(InactiveAfterFirstRoutingReadRepository {
            connection: test_connection(connection_id, runtime_id.clone(), true),
            reads: std::sync::atomic::AtomicUsize::new(0),
            pending_after_first: None,
            adapter_revision_after_first: None,
        });
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: runtime_id,
        model: TEST_MODEL.clone(),
        seen_models: calls.clone(),
    });
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![provider],
        connection_providers: HashMap::new(),
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository);
    let mut req = request();
    req.stream = true;
    req.metadata.cacheable = false;

    let error = match usecase.execute_stream(req).await {
        Ok(_) => panic!("disabled runtime-matched connection must not start a stream"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("all providers exhausted"));
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn streaming_usage_records_ttft_from_first_chunk_and_final_usage_on_success() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("test-provider"),
        complete_error: None,
        stream_error: None,
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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
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
        fn model_capabilities(&self, _: &ModelId) -> rook_core::ModelCapabilities {
            fixture_capabilities()
        }
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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
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
        stream_error: None,
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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
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

#[tokio::test]
async fn budgeted_request_without_verifiable_provider_bound_is_rejected_before_provider_io() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(SpyProvider {
        id: ProviderId::new("test-provider"),
        model: TEST_MODEL.clone(),
        seen_models: calls.clone(),
    });
    let usecase = make_usecase_with_provider(provider);
    let mut req = request();
    req.metadata.cacheable = false;
    req.restrictions = rook_core::ApiKeyRestrictions {
        allowed_models: vec![],
        allowed_providers: vec![],
        spending_budget: Some(rook_core::SpendingBudget {
            api_key_id: ApiKeyId::new("budget-key"),
            amount_micros_usd: 10_000_000,
            request_limit_micros_usd: 1_000_000,
            window: rook_core::SpendingBudgetWindow::Daily,
        }),
    };

    let error = usecase
        .execute(req)
        .await
        .expect_err("no provider exposes an enforceable preflight cost bound");

    assert!(error.is_cost_unavailable());
    assert!(error.to_string().contains("cost unavailable"));
    assert!(
        calls.lock().unwrap().is_empty(),
        "provider must not be called"
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
    fn model_capabilities(&self, _: &ModelId) -> rook_core::ModelCapabilities {
        fixture_capabilities()
    }
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
    fn model_capabilities(&self, _: &ModelId) -> rook_core::ModelCapabilities {
        fixture_capabilities()
    }
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
        Err(CortexError::rate_limited(self.id.clone(), 30))
    }
}

struct MapProviderRepository {
    connections: HashMap<ConnectionId, rook_core::ProviderConnection>,
}

struct RevisionChangingProviderRepository {
    connection: rook_core::ProviderConnection,
}

#[async_trait]
impl ProviderRepositoryPort for RevisionChangingProviderRepository {
    async fn list(&self) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(vec![self.connection.clone()])
    }

    async fn list_routing_state(
        &self,
    ) -> Result<Vec<rook_core::ProviderConnectionRoutingState>, rook_core::RepositoryError> {
        Ok(vec![rook_core::ProviderConnectionRoutingState {
            id: self.connection.id,
            provider_runtime_id: self.connection.provider_runtime_id.clone(),
            adapter_revision: self.connection.adapter_revision + 1,
            priority: self.connection.priority,
            created_at: self.connection.created_at,
            is_active: self.connection.is_active,
            auth_expired: false,
        }])
    }

    async fn find(
        &self,
        id: &ConnectionId,
    ) -> Result<Option<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok((*id == self.connection.id).then(|| self.connection.clone()))
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
        _expected_updated_at: chrono::DateTime<Utc>,
    ) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }

    async fn delete(&self, _id: &ConnectionId) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }
}

struct FailsOneRoutingStateReadRepository {
    connections: HashMap<ConnectionId, rook_core::ProviderConnection>,
    fail_connection_id: ConnectionId,
}

struct FailsSecondRoutingStateReadRepository {
    connections: HashMap<ConnectionId, rook_core::ProviderConnection>,
    fail_connection_id: ConnectionId,
    reads: std::sync::atomic::AtomicUsize,
    timeout: bool,
    /// When set, every connection fails the routing-state re-read instead of
    /// only `fail_connection_id`. The first read always succeeds so the
    /// failure lands on the post-permission re-read.
    fail_every_connection: bool,
}

#[async_trait]
impl ProviderRepositoryPort for FailsSecondRoutingStateReadRepository {
    async fn list(&self) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(self.connections.values().cloned().collect())
    }

    async fn list_routing_state_for_candidates(
        &self,
        targets: &[ProviderId],
    ) -> Result<Vec<rook_core::ProviderConnectionRoutingState>, rook_core::RepositoryError> {
        let is_failing_target = self.fail_every_connection
            || targets
                .iter()
                .any(|target| target.as_str() == self.fail_connection_id.to_string());
        if is_failing_target && self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
            if self.timeout {
                return std::future::pending().await;
            }
            return Err(rook_core::RepositoryError::Database(
                "routing-state reread test failure".to_string(),
            ));
        }

        Ok(self
            .connections
            .values()
            .filter(|connection| {
                targets
                    .iter()
                    .any(|target| target.as_str() == connection.id.to_string())
            })
            .map(|connection| rook_core::ProviderConnectionRoutingState {
                id: connection.id,
                provider_runtime_id: connection.provider_runtime_id.clone(),
                adapter_revision: connection.adapter_revision,
                priority: connection.priority,
                created_at: connection.created_at,
                is_active: connection.is_active,
                auth_expired: matches!(
                    &connection.test_status,
                    rook_core::TestStatus::Expired { .. }
                ),
            })
            .collect())
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
        _expected_updated_at: chrono::DateTime<Utc>,
    ) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }

    async fn delete(&self, _id: &ConnectionId) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }
}

#[async_trait]
impl ProviderRepositoryPort for FailsOneRoutingStateReadRepository {
    async fn list(&self) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(self.connections.values().cloned().collect())
    }

    async fn list_routing_state_for_candidates(
        &self,
        targets: &[ProviderId],
    ) -> Result<Vec<rook_core::ProviderConnectionRoutingState>, rook_core::RepositoryError> {
        if targets
            .iter()
            .any(|target| target.as_str() == self.fail_connection_id.to_string())
        {
            return Err(rook_core::RepositoryError::Database(
                "routing-state test failure".to_string(),
            ));
        }
        Ok(self
            .connections
            .values()
            .filter(|connection| {
                targets
                    .iter()
                    .any(|target| target.as_str() == connection.id.to_string())
            })
            .map(|connection| rook_core::ProviderConnectionRoutingState {
                id: connection.id,
                provider_runtime_id: connection.provider_runtime_id.clone(),
                adapter_revision: connection.adapter_revision,
                priority: connection.priority,
                created_at: connection.created_at,
                is_active: connection.is_active,
                auth_expired: matches!(
                    &connection.test_status,
                    rook_core::TestStatus::Expired { .. }
                ),
            })
            .collect())
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
        _expected_updated_at: chrono::DateTime<Utc>,
    ) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }

    async fn delete(&self, _id: &ConnectionId) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }
}

struct InactiveAfterFirstRoutingReadRepository {
    connection: rook_core::ProviderConnection,
    reads: std::sync::atomic::AtomicUsize,
    pending_after_first: Option<Arc<tokio::sync::Notify>>,
    adapter_revision_after_first: Option<i64>,
}

#[async_trait]
impl ProviderRepositoryPort for InactiveAfterFirstRoutingReadRepository {
    async fn list(&self) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(vec![self.connection.clone()])
    }

    async fn list_routing_state(
        &self,
    ) -> Result<Vec<rook_core::ProviderConnectionRoutingState>, rook_core::RepositoryError> {
        let read_index = self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if read_index > 0 {
            if let Some(started) = &self.pending_after_first {
                started.notify_one();
                return std::future::pending().await;
            }
        }
        let is_active = read_index == 0 || self.adapter_revision_after_first.is_some();
        let adapter_revision = if read_index == 0 {
            self.connection.adapter_revision
        } else {
            self.adapter_revision_after_first
                .unwrap_or(self.connection.adapter_revision)
        };
        Ok(vec![rook_core::ProviderConnectionRoutingState {
            id: self.connection.id,
            provider_runtime_id: self.connection.provider_runtime_id.clone(),
            adapter_revision,
            priority: self.connection.priority,
            created_at: self.connection.created_at,
            is_active,
            auth_expired: false,
        }])
    }

    async fn find(
        &self,
        id: &ConnectionId,
    ) -> Result<Option<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok((*id == self.connection.id).then(|| self.connection.clone()))
    }

    async fn find_connection_id_by_runtime(
        &self,
        provider: &ProviderId,
    ) -> Result<Option<ConnectionId>, rook_core::RepositoryError> {
        Ok((provider == &self.connection.provider_runtime_id).then_some(self.connection.id))
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
        _expected_updated_at: chrono::DateTime<Utc>,
    ) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }

    async fn delete(&self, _id: &ConnectionId) -> Result<(), rook_core::RepositoryError> {
        unreachable!("test repository is read-only")
    }
}

#[async_trait]
impl ProviderRepositoryPort for MapProviderRepository {
    async fn list(&self) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(self.connections.values().cloned().collect())
    }

    async fn list_routing_state(
        &self,
    ) -> Result<Vec<rook_core::ProviderConnectionRoutingState>, rook_core::RepositoryError> {
        Ok(self
            .connections
            .values()
            .map(|connection| rook_core::ProviderConnectionRoutingState {
                id: connection.id,
                provider_runtime_id: connection.provider_runtime_id.clone(),
                adapter_revision: connection.adapter_revision,
                priority: connection.priority,
                created_at: connection.created_at,
                is_active: connection.is_active,
                auth_expired: matches!(
                    &connection.test_status,
                    rook_core::TestStatus::Expired { .. }
                ),
            })
            .collect())
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
        self.connection_providers
            .get(connection_id)
            .cloned()
            .map(|provider| {
                if provider.connection_id() == Some(connection_id) {
                    provider
                } else {
                    connection_bound_provider(provider, *connection_id)
                }
            })
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
        AuthType, ConnectionConfig, Credentials, EncryptedBlob, QuotaWindowThresholds, TestStatus,
    };

    let now = chrono::Utc::now();
    rook_core::ProviderConnection {
        id,
        provider: rook_core::descriptor_by_id("openai").expect("openai descriptor"),
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
        adapter_revision: 1,
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

struct AuditOrderRouter {
    provider: Arc<dyn ProviderPort>,
    failures: Arc<std::sync::atomic::AtomicUsize>,
    releases: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl RouterPort for AuditOrderRouter {
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

    async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {
        self.failures
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    async fn try_acquire_trial(
        &self,
        _provider: &ProviderId,
    ) -> Option<rook_core::RouterTrialPermit> {
        let releases = self.releases.clone();
        Some(rook_core::RouterTrialPermit::with_release(move || {
            releases.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }))
    }

    fn providers(&self) -> Vec<ProviderId> {
        vec![self.provider.id().clone()]
    }
}

#[tokio::test]
async fn combo_success_closes_real_router_circuit() {
    use crate::router_impl::CircuitClock;
    use std::time::Instant;

    struct TestClock(Mutex<Instant>);
    impl CircuitClock for TestClock {
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
    }

    let provider_id = ProviderId::new("openai");
    let provider: Arc<dyn ProviderPort> = ordinary_provider("openai");
    let clock = Arc::new(TestClock(Mutex::new(Instant::now())));
    let router = Arc::new(crate::FallbackRouter::with_clock(
        vec![provider],
        rook_core::PolicyKind::Priority,
        clock.clone(),
    ));
    for _ in 0..3 {
        notify_provider_failure(
            router.as_ref(),
            &provider_id,
            &CortexError::provider("prior failure"),
        )
        .await;
    }
    *clock.0.lock().unwrap() += std::time::Duration::from_secs(3600);
    let combo = Combo::new(
        "real-router-combo-success".to_string(),
        ComboStrategy::Priority,
        vec![step("openai", TEST_MODEL.as_str(), 1)],
    );
    let combo_id = combo.id;
    let harness = combo_harness_with_router(
        combo.clone(),
        vec![("rook/coding", combo_id)],
        router.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );
    let mut req = request();
    req.model = ModelId::new("rook/coding");
    req.stream = false;
    req.metadata.cacheable = false;

    harness
        .usecase
        .execute_combo(&combo, req)
        .await
        .expect("half-open combo succeeds");
    let circuit = &router.circuit_states()[0].1;
    assert!(!circuit.is_open);
    assert_eq!(circuit.failures, 0);
}

#[tokio::test]
async fn combo_upstream_failure_updates_real_router_circuit() {
    let provider_id = ProviderId::new("openai");
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: provider_id.clone(),
        complete_error: Some(CortexError::provider("upstream failure")),
        stream_error: None,
    });
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let combo = Combo::new(
        "real-router-combo".to_string(),
        ComboStrategy::Priority,
        vec![step("openai", TEST_MODEL.as_str(), 1)],
    );
    let combo_id = combo.id;
    let harness = combo_harness_with_router(
        combo.clone(),
        vec![("rook/coding", combo_id)],
        router.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );
    let mut req = request();
    req.model = ModelId::new("rook/coding");
    req.stream = false;
    req.metadata.cacheable = false;

    assert!(harness.usecase.execute_combo(&combo, req).await.is_err());
    assert_eq!(router.circuit_states()[0].1.failures, 1);
}

#[tokio::test(start_paused = true)]
async fn combo_timeout_updates_real_router_circuit_and_records_failed_audit_step() {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let provider_id = ProviderId::new("openai");
    let provider: Arc<dyn ProviderPort> = Arc::new(BlockingTestProvider {
        inner: TestProvider {
            id: provider_id.clone(),
            complete_error: None,
            stream_error: None,
        },
        started: Mutex::new(Some(started_tx)),
    });
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let combo = Combo::new(
        "real-router-combo-timeout".to_string(),
        ComboStrategy::Priority,
        vec![step("openai", TEST_MODEL.as_str(), 1)],
    );
    let combo_id = combo.id;
    let harness = combo_harness_with_router(
        combo.clone(),
        vec![("rook/coding", combo_id)],
        router.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );
    let audit = harness.audit.clone();
    let mut req = request();
    req.model = ModelId::new("rook/coding");
    req.stream = false;
    req.metadata.cacheable = false;
    let task = tokio::spawn(async move { harness.usecase.execute_combo(&combo, req).await });

    started_rx.await.expect("combo provider call started");
    assert!(!task.is_finished());
    assert_eq!(router.circuit_states()[0].1.failures, 0);
    assert!(audit.entries.lock().unwrap().is_empty());

    tokio::time::advance(RouteRequest::COMBO_STEP_TIMEOUT + Duration::from_millis(1)).await;
    let error = task
        .await
        .expect("combo task completes")
        .expect_err("combo step times out");
    let errors = error.combo_skip_reasons().expect("combo exhausted");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].0, provider_id);
    assert!(errors[0].1.contains("timed out"));
    assert_eq!(router.circuit_states()[0].1.failures, 1);

    let entries = audit.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, RequestStatus::Failure);
    assert_eq!(entries[0].provider, provider_id);
    assert_eq!(entries[0].combo_id, Some(combo_id));
    assert_eq!(entries[0].combo_step_index, Some(0));
}

#[tokio::test]
async fn combo_cancellation_while_audit_is_blocked_updates_circuit_first() {
    let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
        id: ProviderId::new("openai"),
        complete_error: Some(CortexError::provider("upstream failure")),
        stream_error: None,
    });
    let failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let router = Arc::new(AuditOrderRouter {
        provider,
        failures: failures.clone(),
        releases: releases.clone(),
    });
    let combo = Combo::new(
        "blocked-audit".to_string(),
        ComboStrategy::Priority,
        vec![step("openai", TEST_MODEL.as_str(), 1)],
    );
    let combo_id = combo.id;
    let combos = HashMap::from([(combo_id, combo.clone())]);
    let aliases = HashMap::from([(
        "rook/coding".to_string(),
        ComboAlias::new(ModelId::new("rook/coding"), combo_id),
    )]);
    let blocking_audit = Arc::new(BlockingAudit {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let entered = blocking_audit.entered.notified();
    tokio::pin!(entered);
    let mut usecase = RouteRequest::new(
        router,
        Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        }),
        blocking_audit.clone(),
        None,
        None,
        Some(Arc::new(MapComboRepository { combos })),
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    );
    usecase = usecase.with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases }));

    let mut req = request();
    req.model = ModelId::new("rook/coding");
    req.stream = false;
    req.metadata.cacheable = false;
    let task = tokio::spawn(async move { usecase.execute_combo(&combo, req).await });
    entered.await;
    assert_eq!(failures.load(std::sync::atomic::Ordering::SeqCst), 1);
    task.abort();
    assert!(task.await.expect_err("combo task aborted").is_cancelled());
    assert_eq!(releases.load(std::sync::atomic::Ordering::SeqCst), 0);
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
    queries: Mutex<Vec<QuotaObservationQuery>>,
}

impl FakeQuotaStore {
    fn new(observations: Vec<QuotaObservation>) -> Self {
        Self {
            observations: Mutex::new(observations),
            queries: Mutex::new(Vec::new()),
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

    async fn latest_observations(
        &self,
        query: &QuotaObservationQuery,
    ) -> CortexResult<Vec<QuotaObservation>> {
        self.queries.lock().unwrap().push(query.clone());
        let observations = self.list_observations(query.provider.clone()).await?;
        Ok(rook_core::reduce_latest_observations(observations, query))
    }
}

struct EligibilityTestClock(Mutex<std::time::Instant>);

impl EligibilityTestClock {
    fn advance(&self, duration: std::time::Duration) {
        *self.0.lock().unwrap() += duration;
    }
}

impl crate::router_impl::CircuitClock for EligibilityTestClock {
    fn now(&self) -> std::time::Instant {
        *self.0.lock().unwrap()
    }
}

struct EligibilityAdvancingRoutingStateRepository {
    states: Vec<rook_core::ProviderConnectionRoutingState>,
    clock: Arc<EligibilityTestClock>,
}

#[async_trait]
impl ProviderRepositoryPort for EligibilityAdvancingRoutingStateRepository {
    async fn list(&self) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        unreachable!("eligibility uses the credential-free routing-state view")
    }

    async fn list_routing_state(
        &self,
    ) -> Result<Vec<rook_core::ProviderConnectionRoutingState>, rook_core::RepositoryError> {
        self.clock.advance(std::time::Duration::from_secs(31));
        Ok(self.states.clone())
    }

    async fn find(
        &self,
        _id: &ConnectionId,
    ) -> Result<Option<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(None)
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
        _expected_updated_at: chrono::DateTime<Utc>,
    ) -> Result<(), rook_core::RepositoryError> {
        Ok(())
    }

    async fn delete(&self, _id: &ConnectionId) -> Result<(), rook_core::RepositoryError> {
        Ok(())
    }
}

struct ConcurrentQuotaStore {
    queries: Mutex<Vec<QuotaObservationQuery>>,
    active: std::sync::atomic::AtomicUsize,
    peak: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl QuotaObservationRecorderPort for ConcurrentQuotaStore {
    async fn record_observation(&self, _: QuotaObservation) -> CortexResult<()> {
        Ok(())
    }

    async fn list_observations(
        &self,
        _: Option<ProviderId>,
    ) -> CortexResult<Vec<QuotaObservation>> {
        Ok(Vec::new())
    }

    async fn latest_observations(
        &self,
        query: &QuotaObservationQuery,
    ) -> CortexResult<Vec<QuotaObservation>> {
        use std::sync::atomic::Ordering::SeqCst;
        self.queries.lock().unwrap().push(query.clone());
        let active = self.active.fetch_add(1, SeqCst) + 1;
        self.peak.fetch_max(active, SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        self.active.fetch_sub(1, SeqCst);
        Ok(vec![quota_observation(
            query.connection_ids[0],
            query.provider.clone().unwrap(),
            0.0,
            Utc::now(),
            Some(Utc::now() + chrono::Duration::seconds(60)),
        )])
    }
}

#[tokio::test(start_paused = true)]
async fn eligibility_quota_reads_are_concurrent_bounded_and_deduplicated() {
    let states: Vec<_> = (0..QUOTA_READ_CONCURRENCY * 2 + 1)
        .map(|index| rook_core::ProviderConnectionRoutingState {
            id: ConnectionId::new(),
            provider_runtime_id: ProviderId::new(format!("quota-{index}")),
            adapter_revision: 1,
            priority: 1,
            created_at: Utc::now(),
            is_active: true,
            auth_expired: false,
        })
        .collect();
    let reader = Arc::new(ConcurrentQuotaStore {
        queries: Mutex::new(Vec::new()),
        active: std::sync::atomic::AtomicUsize::new(0),
        peak: std::sync::atomic::AtomicUsize::new(0),
    });
    let usecase = usecase_with_router(Arc::new(crate::FallbackRouter::new(
        vec![ordinary_provider("quota-target")],
        rook_core::PolicyKind::Priority,
    )))
    .with_quota_observation_reader(reader.clone());
    let connections: Vec<_> = states.iter().chain(states.iter()).collect();

    let (exhausted, _) = usecase.exhausted_connection_ids(&connections).await;

    assert_eq!(exhausted, states.iter().map(|state| state.id).collect());
    let queries = reader.queries.lock().unwrap();
    assert_eq!(queries.len(), states.len());
    for state in &states {
        let matching: Vec<_> = queries
            .iter()
            .filter(|query| query.connection_ids == vec![state.id])
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(
            matching[0].provider.as_ref(),
            Some(&state.provider_runtime_id)
        );
        assert_eq!(matching[0].limit, QuotaObservationQuery::MAX_LIMIT);
    }
    let peak = reader.peak.load(std::sync::atomic::Ordering::SeqCst);
    assert!(peak > 1, "quota reads must overlap");
    assert!(
        peak <= QUOTA_READ_CONCURRENCY,
        "quota reads must be bounded"
    );
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

    async fn list_routing_state(
        &self,
    ) -> Result<Vec<rook_core::ProviderConnectionRoutingState>, rook_core::RepositoryError> {
        Ok(self
            .connections
            .values()
            .map(|connection| rook_core::ProviderConnectionRoutingState {
                id: connection.id,
                provider_runtime_id: connection.provider_runtime_id.clone(),
                adapter_revision: connection.adapter_revision,
                priority: connection.priority,
                created_at: connection.created_at,
                is_active: connection.is_active,
                auth_expired: matches!(
                    &connection.test_status,
                    rook_core::TestStatus::Expired { .. }
                ),
            })
            .collect())
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

    let router: Arc<dyn RouterPort> = Arc::new(crate::FallbackRouter::new(
        vec![
            connection_bound_provider(
                Arc::new(SpyProvider {
                    id: exhausted_provider.clone(),
                    model: TEST_MODEL.clone(),
                    seen_models: exhausted_calls.clone(),
                }),
                exhausted_connection,
            ),
            connection_bound_provider(
                Arc::new(SpyProvider {
                    id: healthy_provider.clone(),
                    model: TEST_MODEL.clone(),
                    seen_models: healthy_calls.clone(),
                }),
                healthy_connection,
            ),
        ],
        rook_core::PolicyKind::Priority,
    ));

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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_quota_observation_reader(quota_reader);

    let mut req = request();
    req.stream = false;
    let result = usecase.execute(req).await;

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
    let router: Arc<dyn RouterPort> = Arc::new(crate::FallbackRouter::new(
        vec![
            connection_bound_provider(
                Arc::new(SpyProvider {
                    id: exhausted_provider.clone(),
                    model: TEST_MODEL.clone(),
                    seen_models: exhausted_calls.clone(),
                }),
                exhausted_connection,
            ),
            connection_bound_provider(
                Arc::new(SpyProvider {
                    id: healthy_provider.clone(),
                    model: TEST_MODEL.clone(),
                    seen_models: healthy_calls.clone(),
                }),
                healthy_connection,
            ),
        ],
        rook_core::PolicyKind::Priority,
    ));
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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_quota_observation_reader(quota_reader);

    let mut req = request();
    req.stream = true;
    let mut stream = usecase
        .execute_stream(req)
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
        provider_id: "openai".to_string(),
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

#[tokio::test]
async fn eligibility_stage_reports_exhausted_quota_before_selection() {
    let exhausted_connection = ConnectionId::new();
    let healthy_connection = ConnectionId::new();
    let exhausted_runtime = ProviderId::new("exhausted-runtime");
    let healthy_runtime = ProviderId::new("healthy-runtime");
    let exhausted_target = exhausted_runtime.clone();
    let healthy_target = ProviderId::new(healthy_connection.to_string());
    let router = Arc::new(crate::FallbackRouter::new(
        vec![
            ordinary_provider(exhausted_target.as_str()),
            connection_bound_provider(
                ordinary_provider(healthy_target.as_str()),
                healthy_connection,
            ),
        ],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(QuotaAwareProviderRepository {
        connections: HashMap::from([
            (
                exhausted_connection,
                test_connection(exhausted_connection, exhausted_runtime.clone(), true),
            ),
            (
                healthy_connection,
                test_connection(healthy_connection, healthy_runtime, true),
            ),
        ]),
        runtime_connections: HashMap::new(),
    });
    let now = Utc::now();
    let quota_reader: Arc<dyn QuotaObservationRecorderPort> =
        Arc::new(FakeQuotaStore::new(vec![quota_observation(
            exhausted_connection,
            exhausted_runtime,
            0.0,
            now,
            Some(now + chrono::Duration::seconds(60)),
        )]));
    let usecase = usecase_with_router_and_provider_repository(router, repository)
        .with_quota_observation_reader(quota_reader);

    let result = evaluate_eligibility(&usecase, &request()).await;

    assert_eq!(result.eligible, vec![crate::TargetRef(healthy_target)]);
    assert_eq!(
        result.rejected,
        vec![crate::Rejection {
            target: crate::TargetRef(exhausted_target),
            reason: crate::RejectionReason::QuotaExhausted,
        }]
    );
}

#[tokio::test]
async fn eligibility_quota_series_limit_is_scoped_per_connection() {
    let filler_connection = ConnectionId::new();
    let exhausted_connection = ConnectionId::new();
    let filler_runtime = ProviderId::new("filler-runtime");
    let exhausted_runtime = ProviderId::new("exhausted-runtime");
    let filler_target = ProviderId::new(filler_connection.to_string());
    let exhausted_target = ProviderId::new(exhausted_connection.to_string());
    let router = Arc::new(crate::FallbackRouter::new(
        vec![
            connection_bound_provider(ordinary_provider(filler_target.as_str()), filler_connection),
            connection_bound_provider(
                ordinary_provider(exhausted_target.as_str()),
                exhausted_connection,
            ),
        ],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(QuotaAwareProviderRepository {
        connections: HashMap::from([
            (
                filler_connection,
                test_connection(filler_connection, filler_runtime.clone(), true),
            ),
            (
                exhausted_connection,
                test_connection(exhausted_connection, exhausted_runtime.clone(), true),
            ),
        ]),
        runtime_connections: HashMap::new(),
    });
    let now = Utc::now();
    let mut observations = Vec::with_capacity(QuotaObservationQuery::MAX_LIMIT + 1);
    for index in 0..QuotaObservationQuery::MAX_LIMIT {
        let mut observation = quota_observation(
            filler_connection,
            filler_runtime.clone(),
            1.0,
            now - chrono::Duration::seconds((index as i64) % 100),
            Some(now + chrono::Duration::seconds(60)),
        );
        observation.scope = Some(format!("scope-{index}"));
        observations.push(observation);
    }
    observations.push(quota_observation(
        exhausted_connection,
        exhausted_runtime,
        0.0,
        now - chrono::Duration::seconds(299),
        Some(now + chrono::Duration::seconds(60)),
    ));
    let quota_reader: Arc<dyn QuotaObservationRecorderPort> =
        Arc::new(FakeQuotaStore::new(observations));
    let usecase = usecase_with_router_and_provider_repository(router, repository)
        .with_quota_observation_reader(quota_reader);

    let result = evaluate_eligibility(&usecase, &request()).await;

    assert_eq!(result.eligible, vec![crate::TargetRef(filler_target)]);
    assert_eq!(
        result.rejected,
        vec![crate::Rejection {
            target: crate::TargetRef(exhausted_target),
            reason: crate::RejectionReason::QuotaExhausted,
        }]
    );
}

#[tokio::test]
async fn capped_legacy_quota_read_is_unknown_not_exhausted() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("legacy-capped-quota-runtime");
    let now = Utc::now();
    let observations = (0..QuotaObservationQuery::MAX_LIMIT)
        .map(|index| {
            let mut observation = quota_observation(
                connection_id,
                runtime_id.clone(),
                1.0,
                now - chrono::Duration::seconds(index as i64),
                Some(now + chrono::Duration::seconds(60)),
            );
            observation.scope = Some(format!("scope-{index}"));
            observation
        })
        .collect();
    let reader = Arc::new(FakeQuotaStore::new(observations));
    let usecase = usecase_with_router(Arc::new(crate::FallbackRouter::new(
        vec![ordinary_provider("quota-target")],
        rook_core::PolicyKind::Priority,
    )))
    .with_quota_observation_reader(reader);

    let state = usecase
        .connection_quota_state(&connection_id, Some(&runtime_id))
        .await;

    assert!(matches!(state, QuotaState::Unknown));
}

#[tokio::test]
async fn capped_quota_series_remains_unknown_and_eligible_even_with_exhaustion_samples() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("capped-exhausted-runtime");
    let target = ProviderId::new(connection_id.to_string());
    let router = Arc::new(crate::FallbackRouter::new(
        vec![connection_bound_provider(
            ordinary_provider(target.as_str()),
            connection_id,
        )],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(QuotaAwareProviderRepository {
        connections: HashMap::from([(
            connection_id,
            test_connection(connection_id, runtime_id.clone(), true),
        )]),
        runtime_connections: HashMap::new(),
    });
    let now = Utc::now();
    let observations = (0..QuotaObservationQuery::MAX_LIMIT)
        .map(|index| {
            let mut observation = quota_observation(
                connection_id,
                runtime_id.clone(),
                1.0,
                now - chrono::Duration::seconds(index as i64),
                Some(now + chrono::Duration::seconds(60)),
            );
            observation.scope = Some(format!("scope-{index}"));
            observation
        })
        .collect();
    let quota_reader: Arc<dyn QuotaObservationRecorderPort> =
        Arc::new(FakeQuotaStore::new(observations));
    let usecase = usecase_with_router_and_provider_repository(router, repository)
        .with_quota_observation_reader(quota_reader);

    let result = evaluate_eligibility(&usecase, &request()).await;

    assert_eq!(result.eligible, vec![crate::TargetRef(target)]);
    assert!(result.rejected.is_empty());
}

#[tokio::test]
async fn eligibility_stage_quota_timeout_keeps_candidate_eligible() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("pending-quota-runtime");
    let target = ProviderId::new(connection_id.to_string());
    let router = Arc::new(crate::FallbackRouter::new(
        vec![connection_bound_provider(
            ordinary_provider(target.as_str()),
            connection_id,
        )],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(QuotaAwareProviderRepository {
        connections: HashMap::from([(
            connection_id,
            test_connection(connection_id, runtime_id, true),
        )]),
        runtime_connections: HashMap::new(),
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository)
        .with_quota_observation_reader(Arc::new(PendingQuotaStore));

    let result = tokio::time::timeout(
        QUOTA_READ_TIMEOUT + std::time::Duration::from_secs(1),
        evaluate_eligibility(&usecase, &request()),
    )
    .await
    .expect("the batch quota read must have a deadline");

    assert_eq!(result.eligible, vec![crate::TargetRef(target)]);
    assert!(result.rejected.is_empty());
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

/// Combo under test, the per-provider call recorders, the router, and the
/// repository whose routing-state re-read fails.
type ComboStateRereadSetup = (
    Combo,
    Arc<Mutex<Vec<String>>>,
    Arc<Mutex<Vec<String>>>,
    Arc<dyn RouterPort>,
    Arc<dyn ProviderRepositoryPort>,
);

/// Streaming harness for the same combo, plus the per-provider call recorders.
type ComboStreamStateRereadSetup = (
    RouteRequest,
    Combo,
    Arc<Mutex<Vec<String>>>,
    Arc<Mutex<Vec<String>>>,
);

fn combo_state_reread_setup(timeout: bool, fail_every_connection: bool) -> ComboStateRereadSetup {
    let first_connection = ConnectionId::new();
    let second_connection = ConnectionId::new();
    let runtime_id = ProviderId::new("combo-reread-runtime");
    let first_calls = Arc::new(Mutex::new(Vec::new()));
    let second_calls = Arc::new(Mutex::new(Vec::new()));
    let first_provider = connection_bound_provider(
        Arc::new(SpyProvider {
            id: ProviderId::new(first_connection.to_string()),
            model: ModelId::new("gpt-test"),
            seen_models: first_calls.clone(),
        }),
        first_connection,
    );
    let second_provider = connection_bound_provider(
        Arc::new(SpyProvider {
            id: ProviderId::new(second_connection.to_string()),
            model: ModelId::new("gpt-test"),
            seen_models: second_calls.clone(),
        }),
        second_connection,
    );
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: Vec::new(),
        connection_providers: HashMap::from([
            (first_connection, first_provider),
            (second_connection, second_provider),
        ]),
    });
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(FailsSecondRoutingStateReadRepository {
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
            fail_connection_id: first_connection,
            reads: std::sync::atomic::AtomicUsize::new(0),
            timeout,
            fail_every_connection,
        });
    let combo = Combo::new(
        "combo-state-reread".to_string(),
        ComboStrategy::Priority,
        vec![
            ComboStep {
                provider_id: runtime_id.clone(),
                model: ModelId::new("gpt-test"),
                connection_id: Some(first_connection),
                priority: 1,
            },
            ComboStep {
                provider_id: runtime_id,
                model: ModelId::new("gpt-test"),
                connection_id: Some(second_connection),
                priority: 2,
            },
        ],
    );
    (combo, first_calls, second_calls, router, repository)
}

#[tokio::test]
async fn combo_state_reread_error_skips_step_and_falls_back() {
    let (combo, first_calls, second_calls, router, repository) =
        combo_state_reread_setup(false, false);
    let harness = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(repository),
    );

    let result = harness
        .usecase
        .execute_combo(
            &combo,
            combo_req("gpt-test", rook_core::ApiKeyRestrictions::default()),
        )
        .await;

    assert!(
        result.is_ok(),
        "the next verified step must serve the request: {result:?}"
    );
    assert!(
        first_calls.lock().unwrap().is_empty(),
        "the unreadable connection must not reach upstream"
    );
    assert_eq!(second_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn combo_state_reread_timeout_skips_step_and_falls_back() {
    let (combo, first_calls, second_calls, router, repository) =
        combo_state_reread_setup(true, false);
    let harness = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(repository),
    );

    let result = harness
        .usecase
        .execute_combo(
            &combo,
            combo_req("gpt-test", rook_core::ApiKeyRestrictions::default()),
        )
        .await;

    assert!(
        result.is_ok(),
        "the next verified step must serve the request: {result:?}"
    );
    assert!(
        first_calls.lock().unwrap().is_empty(),
        "the timed-out connection must not reach upstream"
    );
    assert_eq!(second_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn combo_state_reread_failure_records_connection_state_unavailable_reason() {
    let (combo, first_calls, second_calls, router, repository) =
        combo_state_reread_setup(false, true);
    let harness = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(repository),
    );

    let error = harness
        .usecase
        .execute_combo(
            &combo,
            combo_req("gpt-test", rook_core::ApiKeyRestrictions::default()),
        )
        .await
        .expect_err("every step must be skipped when the routing-state re-read fails");

    let reasons = error
        .combo_skip_reasons()
        .expect("the exhausted combo must expose the recorded skip reasons");
    assert!(
        reasons
            .iter()
            .any(|(_, reason)| reason == "connection_state_unavailable"),
        "the failed re-read must record the connection_state_unavailable skip reason: {reasons:?}"
    );
    assert!(
        first_calls.lock().unwrap().is_empty(),
        "the unreadable connection must not reach upstream"
    );
    assert!(
        second_calls.lock().unwrap().is_empty(),
        "every skipped connection must not reach upstream"
    );
}

fn combo_stream_state_reread_setup(
    timeout: bool,
    fail_every_connection: bool,
) -> ComboStreamStateRereadSetup {
    let (combo, first_calls, second_calls, router, repository) =
        combo_state_reread_setup(timeout, fail_every_connection);
    let combo_id = combo.id;
    let aliases = HashMap::from([(
        "rook/state-reread".to_string(),
        ComboAlias::new(ModelId::new("rook/state-reread"), combo_id),
    )]);
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
        Some(Arc::new(MapComboRepository {
            combos: HashMap::from([(combo_id, combo.clone())]),
        })),
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases }));
    (usecase, combo, first_calls, second_calls)
}

#[tokio::test]
async fn combo_stream_state_reread_error_skips_step_and_falls_back() {
    let (usecase, _combo, first_calls, second_calls) =
        combo_stream_state_reread_setup(false, false);
    let stream = usecase
        .execute_stream(stream_req("rook/state-reread"))
        .await
        .expect("the next verified stream step must start");
    let chunks = stream.collect::<Vec<_>>().await;

    assert!(!chunks.is_empty());
    assert!(chunks.iter().all(Result::is_ok));
    assert!(first_calls.lock().unwrap().is_empty());
    assert_eq!(second_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn combo_stream_state_reread_timeout_skips_step_and_falls_back() {
    let (usecase, _combo, first_calls, second_calls) = combo_stream_state_reread_setup(true, false);
    let stream = usecase
        .execute_stream(stream_req("rook/state-reread"))
        .await
        .expect("the next verified stream step must start");
    let chunks = stream.collect::<Vec<_>>().await;

    assert!(!chunks.is_empty());
    assert!(chunks.iter().all(Result::is_ok));
    assert!(first_calls.lock().unwrap().is_empty());
    assert_eq!(second_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn combo_stream_state_reread_failure_records_connection_state_unavailable_reason() {
    let (usecase, _combo, first_calls, second_calls) = combo_stream_state_reread_setup(false, true);

    let error = match usecase
        .execute_stream(stream_req("rook/state-reread"))
        .await
    {
        Ok(_) => panic!("every stream step must be skipped when the routing-state re-read fails"),
        Err(error) => error,
    };

    let reasons = error
        .combo_skip_reasons()
        .expect("the exhausted combo must expose the recorded skip reasons");
    assert!(
        reasons
            .iter()
            .any(|(_, reason)| reason == "connection_state_unavailable"),
        "the failed re-read must record the connection_state_unavailable skip reason: {reasons:?}"
    );
    assert!(
        first_calls.lock().unwrap().is_empty(),
        "the unreadable connection must not reach upstream"
    );
    assert!(
        second_calls.lock().unwrap().is_empty(),
        "every skipped connection must not reach upstream"
    );
}

#[tokio::test]
async fn combo_skips_managed_adapter_with_stale_configuration_revision() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("stale-combo-runtime");
    let adapter_id = ProviderId::new(connection_id.to_string());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = connection_bound_provider(
        Arc::new(SpyProvider {
            id: adapter_id,
            model: ModelId::new("gpt-test"),
            seen_models: calls.clone(),
        }),
        connection_id,
    );
    let router: Arc<dyn RouterPort> = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let mut connection = test_connection(connection_id, runtime_id.clone(), true);
    connection.adapter_revision = 2;
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(MapProviderRepository {
        connections: HashMap::from([(connection_id, connection)]),
    });
    let combo = Combo::new(
        "stale-adapter-revision".to_string(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: runtime_id,
            model: ModelId::new("gpt-test"),
            connection_id: Some(connection_id),
            priority: 1,
        }],
    );
    let h = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        calls.clone(),
        Some(repository),
    );

    let error = h
        .usecase
        .execute_combo(
            &combo,
            combo_req("gpt-test", rook_core::ApiKeyRestrictions::default()),
        )
        .await
        .expect_err("combo must not call a stale adapter");

    assert!(error.to_string().contains("all providers exhausted"));
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn combo_rechecks_adapter_revision_after_connection_lookup() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("changing-combo-runtime");
    let adapter_id = ProviderId::new(connection_id.to_string());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = connection_bound_provider(
        Arc::new(SpyProvider {
            id: adapter_id,
            model: ModelId::new("gpt-test"),
            seen_models: calls.clone(),
        }),
        connection_id,
    );
    let router: Arc<dyn RouterPort> = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(RevisionChangingProviderRepository {
            connection: test_connection(connection_id, runtime_id.clone(), true),
        });
    let combo = Combo::new(
        "combo-revision-race".to_string(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: runtime_id,
            model: ModelId::new("gpt-test"),
            connection_id: Some(connection_id),
            priority: 1,
        }],
    );
    let harness = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        calls.clone(),
        Some(repository),
    );

    let error = harness
        .usecase
        .execute_combo(
            &combo,
            combo_req("gpt-test", rook_core::ApiKeyRestrictions::default()),
        )
        .await
        .expect_err("combo must reject a revision change before upstream I/O");

    assert!(error.to_string().contains("all providers exhausted"));
    assert!(calls.lock().unwrap().is_empty());
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
                    spending_budget: None,
                },
            ),
        )
        .await;

    assert!(matches!(result, Err(error) if error.is_forbidden()));
    assert!(first_calls.lock().unwrap().is_empty());
    assert!(unauthorized_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn combo_fails_closed_before_upstream_when_authorization_state_is_unavailable() {
    let first_connection = ConnectionId::new();
    let later_connection = ConnectionId::new();
    let runtime_id = ProviderId::new("combo-state-runtime");
    let first_adapter = ProviderId::new(first_connection.to_string());
    let later_adapter = ProviderId::new(later_connection.to_string());
    let first_calls = Arc::new(Mutex::new(Vec::new()));
    let later_calls = Arc::new(Mutex::new(Vec::new()));
    let combo = Combo::new(
        "combo-state-unavailable".to_string(),
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
                connection_id: Some(later_connection),
                priority: 2,
            },
        ],
    );
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: Vec::new(),
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
                later_connection,
                Arc::new(SpyProvider {
                    id: later_adapter.clone(),
                    model: ModelId::new("gpt-test"),
                    seen_models: later_calls.clone(),
                }) as Arc<dyn ProviderPort>,
            ),
        ]),
    });
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(FailsOneRoutingStateReadRepository {
            connections: HashMap::from([
                (
                    first_connection,
                    test_connection(first_connection, runtime_id.clone(), true),
                ),
                (
                    later_connection,
                    test_connection(later_connection, runtime_id, true),
                ),
            ]),
            fail_connection_id: later_connection,
        });
    let harness = combo_harness_with_repositories(
        combo.clone(),
        vec![],
        router,
        Arc::new(Mutex::new(Vec::new())),
        Some(repository),
    );

    let error = harness
        .usecase
        .execute_combo(
            &combo,
            combo_req(
                "gpt-test",
                rook_core::ApiKeyRestrictions {
                    allowed_models: Vec::new(),
                    allowed_providers: vec![first_adapter, later_adapter],
                    spending_budget: None,
                },
            ),
        )
        .await
        .expect_err("unavailable authorization state must stop the combo preflight");

    assert!(error.to_string().contains("all providers exhausted"));
    assert!(first_calls.lock().unwrap().is_empty());
    assert!(later_calls.lock().unwrap().is_empty());
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
        spending_budget: None,
    };

    let error = h
        .usecase
        .execute_combo(&combo, combo_req("gpt-4o", restrictions))
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
                    spending_budget: None,
                },
            ),
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
                    spending_budget: None,
                },
            ),
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
                    spending_budget: None,
                },
            ),
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
                    spending_budget: None,
                },
            ),
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
        spending_budget: None,
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
        spending_budget: None,
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
        stream_error: Some(CortexError::provider("boom")),
    })
}

fn ok_provider(id: &str) -> Arc<dyn ProviderPort> {
    Arc::new(TestProvider {
        id: ProviderId::new(id),
        complete_error: None,
        stream_error: None,
    })
}

fn stream_req(model: &str) -> CompletionRequest {
    let mut r = combo_req(model, rook_core::ApiKeyRestrictions::default());
    r.stream = true;
    r
}

#[tokio::test]
async fn combo_stream_rechecks_adapter_revision_before_starting_upstream() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("changing-stream-combo-runtime");
    let adapter_id = ProviderId::new(connection_id.to_string());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
        id: adapter_id,
        model: ModelId::new("gpt-test"),
        seen_models: calls.clone(),
    });
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: Vec::new(),
        connection_providers: HashMap::from([(connection_id, provider)]),
    });
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(RevisionChangingProviderRepository {
            connection: test_connection(connection_id, runtime_id.clone(), true),
        });
    let combo = Combo::new(
        "stream-combo-revision-race".to_string(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: runtime_id,
            model: ModelId::new("gpt-test"),
            connection_id: Some(connection_id),
            priority: 1,
        }],
    );
    let combo_id = combo.id;
    let combos = HashMap::from([(combo_id, combo)]);
    let alias = ModelId::new("rook/stream-revision");
    let aliases = HashMap::from([(alias.to_string(), ComboAlias::new(alias.clone(), combo_id))]);
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
        Some(Arc::new(MapComboRepository { combos })),
        Arc::new(crate::PricingConfig::default()),
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases }));

    let error = match usecase
        .execute_stream(stream_req("rook/stream-revision"))
        .await
    {
        Ok(_) => panic!("combo stream must reject a stale adapter revision"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("all providers exhausted"));
    assert!(calls.lock().unwrap().is_empty());
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
        .execute_stream(stream_req("rook/coding"))
        .await
        .expect("combo stream starts");
    let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
    assert_eq!(chunks.len(), 2, "content chunk, then final chunk");
    let mut chunks = chunks.into_iter().map(|c| c.expect("chunk ok"));
    let content = chunks.next().unwrap();
    assert_eq!(content.model.as_str(), "rook/coding");
    assert_eq!(content.delta, "hel");
    assert!(content.finish_reason.is_none());
    assert!(content.usage.is_none());
    let final_chunk = chunks.next().unwrap();
    assert_eq!(final_chunk.model.as_str(), "rook/coding");
    assert_eq!(final_chunk.delta, "lo");
    assert!(final_chunk.usage.is_some(), "usage rides the final chunk");
}

/// Test provider returning a tool_call response for combo-stream coverage.
struct ToolProvider {
    id: ProviderId,
}

#[async_trait]
impl ProviderPort for ToolProvider {
    fn model_capabilities(&self, _: &ModelId) -> rook_core::ModelCapabilities {
        fixture_capabilities()
    }
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
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        use rook_core::{ToolCallDelta, ToolCallFunctionDelta};
        let start = ToolCallDelta {
            index: 0,
            id: Some("call_1".into()),
            function: ToolCallFunctionDelta {
                name: Some("lookup".into()),
                arguments: None,
            },
        };
        let args = ToolCallDelta {
            index: 0,
            id: None,
            function: ToolCallFunctionDelta {
                name: None,
                arguments: Some(r#"{"key":"value"}"#.into()),
            },
        };
        let resp = self.complete(req).await?;
        Ok(Box::pin(stream::iter(vec![
            Ok(StreamChunk {
                id: req.id.clone(),
                model: req.model.clone(),
                delta: String::new(),
                thinking: None,
                tool_calls: vec![start.clone(), args],
                finish_reason: None,
                usage: None,
            }),
            Ok(StreamChunk {
                id: req.id.clone(),
                model: req.model.clone(),
                delta: String::new(),
                thinking: None,
                tool_calls: vec![start],
                finish_reason: resp.finish_reason,
                usage: Some(resp.usage),
            }),
        ])))
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
        Arc::new(TestAliasRepository),
        test_alias_config(),
        None,
    )
    .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases }));
    let stream = usecase
        .execute_stream(stream_req("rook/tools"))
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
        .execute_stream(stream_req("rook/fb"))
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
        .execute_stream(stream_req("rook/nope"))
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
    fn model_capabilities(&self, _: &ModelId) -> rook_core::ModelCapabilities {
        fixture_capabilities()
    }
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
    fn model_capabilities(&self, _: &ModelId) -> rook_core::ModelCapabilities {
        fixture_capabilities()
    }
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
            finish_reason: Some(rook_core::FinishReason::Stop),
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
        .execute_stream(stream_req("rook/coding"))
        .await
        .expect("virtual combo stream starts");
    let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
    assert_eq!(chunks.len(), 1, "preserve native upstream chunk boundaries");
    let mut chunks = chunks.into_iter().map(|c| c.expect("chunk ok"));
    let content = chunks.next().unwrap();
    assert_eq!(
        content.model.as_str(),
        "rook/coding",
        "streaming must preserve rook/coding (combo), never rewrite to physical gpt-4o"
    );
    assert!(content.finish_reason.is_some());
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
        .execute_stream(stream_req("coding"))
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

#[test]
fn test_stream_error_status_variants() {
    let rl_err = CortexError::rate_limited(ProviderId::new("p"), 10);
    assert_eq!(
        combo_stream::stream_error_status(&rl_err),
        RequestStatus::RateLimited
    );

    let other_err = CortexError::provider("some error");
    assert_eq!(
        combo_stream::stream_error_status(&other_err),
        RequestStatus::Failure
    );
}

#[test]
fn test_classify_connection_adapter_unusable_unavailable() {
    struct UnavailProvider;
    #[async_trait]
    impl ProviderPort for UnavailProvider {
        fn id(&self) -> &ProviderId {
            static P: std::sync::LazyLock<ProviderId> =
                std::sync::LazyLock::new(|| ProviderId::new("p"));
            &P
        }
        fn supported_models(&self) -> &[shared_kernel::ModelId] {
            &[]
        }
        fn api_format(&self) -> ApiFormat {
            ApiFormat::OpenAI
        }
        fn is_available(&self) -> bool {
            false
        }
        async fn health_check(&self) -> HealthStatus {
            HealthStatus::Healthy {
                provider: ProviderId::new("p"),
                latency_ms: 5,
            }
        }
        async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CortexError> {
            Err(CortexError::provider("no"))
        }
        async fn stream(
            &self,
            _: &CompletionRequest,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<StreamChunk, CortexError>>,
            CortexError,
        > {
            Err(CortexError::provider("no"))
        }
    }

    let p = UnavailProvider;
    let pid = ProviderId::new("p");
    let model = shared_kernel::ModelId::new("m");
    let dec = classify_connection_adapter(&p, &pid, &model, &[]);
    assert_eq!(
        dec,
        ConnectionAdapterDecision::Unusable("provider_unavailable")
    );
}
