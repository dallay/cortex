//! Routing-level coverage for strict per-key spending budgets.
//!
//! These tests exercise the whole admission path: the ledger reservation must
//! exist before any upstream I/O, the final cost must be settled afterwards,
//! and every rejection must happen before the provider is contacted.

use super::*;
use rook_core::{
    ApiKeyRestrictions, SpendingBudget, SpendingBudgetLedgerPort, SpendingBudgetReservation,
    SpendingBudgetReservationError, SpendingBudgetWindow, SpendingReservationOutcome,
};

/// Observable sequence of budget events during one request.
#[derive(Debug, Clone, PartialEq, Eq)]
enum BudgetEvent {
    Reserve,
    UpstreamCall,
    Settle(Option<u64>),
    Release,
}

type EventLog = Arc<Mutex<Vec<BudgetEvent>>>;

struct RecordingLedger {
    events: EventLog,
    rows: Mutex<std::collections::HashMap<RequestId, BudgetEvent>>,
    release_notify: Arc<tokio::sync::Notify>,
    reserve_outcome:
        Mutex<Option<Result<SpendingReservationOutcome, SpendingBudgetReservationError>>>,
}

impl RecordingLedger {
    fn new(events: EventLog) -> Self {
        Self {
            events,
            rows: Mutex::new(std::collections::HashMap::new()),
            release_notify: Arc::new(tokio::sync::Notify::new()),
            reserve_outcome: Mutex::new(None),
        }
    }

    fn with_reserve_outcome(
        events: EventLog,
        outcome: Result<SpendingReservationOutcome, SpendingBudgetReservationError>,
    ) -> Self {
        Self {
            events,
            rows: Mutex::new(std::collections::HashMap::new()),
            release_notify: Arc::new(tokio::sync::Notify::new()),
            reserve_outcome: Mutex::new(Some(outcome)),
        }
    }
}

#[async_trait]
impl SpendingBudgetLedgerPort for RecordingLedger {
    async fn reserve_spending(
        &self,
        reservation: &SpendingBudgetReservation,
    ) -> Result<SpendingReservationOutcome, SpendingBudgetReservationError> {
        self.events.lock().unwrap().push(BudgetEvent::Reserve);
        if let Some(outcome) = self.reserve_outcome.lock().unwrap().take() {
            return outcome;
        }
        let mut rows = self.rows.lock().unwrap();
        if rows.contains_key(&reservation.request_id) {
            return Ok(SpendingReservationOutcome::Existing);
        }
        rows.insert(reservation.request_id.clone(), BudgetEvent::Reserve);
        Ok(SpendingReservationOutcome::Reserved)
    }

    async fn settle_spending(
        &self,
        _api_key_id: &ApiKeyId,
        request_id: &RequestId,
        cost_micros_usd: Option<u64>,
    ) -> Result<(), SpendingBudgetReservationError> {
        let mut rows = self.rows.lock().unwrap();
        assert_eq!(rows.get(request_id), Some(&BudgetEvent::Reserve));
        rows.insert(request_id.clone(), BudgetEvent::Settle(cost_micros_usd));
        self.events
            .lock()
            .unwrap()
            .push(BudgetEvent::Settle(cost_micros_usd));
        Ok(())
    }

    async fn release_spending_before_upstream(
        &self,
        _api_key_id: &ApiKeyId,
        request_id: &RequestId,
    ) -> Result<(), SpendingBudgetReservationError> {
        let mut rows = self.rows.lock().unwrap();
        assert_eq!(rows.get(request_id), Some(&BudgetEvent::Reserve));
        rows.insert(request_id.clone(), BudgetEvent::Release);
        self.events.lock().unwrap().push(BudgetEvent::Release);
        self.release_notify.notify_one();
        Ok(())
    }
}

/// Provider that can demonstrate a strict cost bound and an exact final cost.
struct BoundedProvider {
    id: ProviderId,
    events: EventLog,
    bound: Option<u64>,
    realized_cost: Option<u64>,
    complete_error: Option<String>,
    local_backpressure: bool,
    stream_error: Option<String>,
}

impl BoundedProvider {
    fn new(id: &str, events: EventLog) -> Self {
        Self {
            id: ProviderId::new(id),
            events,
            bound: None,
            realized_cost: None,
            complete_error: None,
            local_backpressure: false,
            stream_error: None,
        }
    }

    fn with_bound(mut self, bound: u64) -> Self {
        self.bound = Some(bound);
        self
    }

    fn with_realized_cost(mut self, cost: u64) -> Self {
        self.realized_cost = Some(cost);
        self
    }

    fn failing(mut self, message: &str) -> Self {
        self.complete_error = Some(message.to_string());
        self
    }

    fn locally_backpressured(mut self) -> Self {
        self.local_backpressure = true;
        self
    }

    fn stream_failing(mut self, message: &str) -> Self {
        self.stream_error = Some(message.to_string());
        self
    }
}

#[async_trait]
impl ProviderPort for BoundedProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn supported_models(&self) -> &[ModelId] {
        std::slice::from_ref(&TEST_MODEL)
    }

    fn model_capabilities(&self, _: &ModelId) -> rook_core::ModelCapabilities {
        fixture_capabilities()
    }

    fn api_format(&self) -> rook_core::ApiFormat {
        rook_core::ApiFormat::OpenAI
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn health_check(&self) -> rook_core::HealthStatus {
        rook_core::HealthStatus::Healthy {
            provider: self.id.clone(),
            latency_ms: 1,
        }
    }

    fn strict_max_cost_micros_usd(&self, _req: &CompletionRequest) -> Option<u64> {
        self.bound
    }

    fn strict_realized_cost_micros_usd(
        &self,
        _req: &CompletionRequest,
        _resp: &CompletionResponse,
    ) -> Option<u64> {
        self.realized_cost
    }

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        if self.local_backpressure {
            return Err(CortexError::local_backpressure());
        }
        self.events.lock().unwrap().push(BudgetEvent::UpstreamCall);
        if let Some(message) = &self.complete_error {
            return Err(CortexError::provider(message.clone()));
        }
        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.id.clone(),
            model: req.model.clone(),
            content: "ok".to_string(),
            content_blocks: vec![],
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
    ) -> CortexResult<
        futures::stream::BoxStream<'static, Result<rook_core::StreamChunk, CortexError>>,
    > {
        if self.local_backpressure {
            return Err(CortexError::local_backpressure());
        }
        self.events.lock().unwrap().push(BudgetEvent::UpstreamCall);
        if let Some(message) = &self.stream_error {
            return Err(CortexError::provider(message.clone()));
        }
        Ok(Box::pin(stream::iter(vec![Ok(rook_core::StreamChunk {
            id: RequestId::new(),
            model: TEST_MODEL.clone(),
            delta: "ok".to_string(),
            thinking: None,
            tool_calls: vec![],
            finish_reason: None,
            usage: None,
        })])))
    }
}

fn budgeted_request() -> CompletionRequest {
    let mut req = request();
    req.metadata.cacheable = false;
    req.restrictions = ApiKeyRestrictions {
        allowed_models: vec![],
        allowed_providers: vec![],
        spending_budget: Some(SpendingBudget {
            api_key_id: ApiKeyId::new("budget-key"),
            amount_micros_usd: 10_000_000,
            request_limit_micros_usd: 1_000_000,
            window: SpendingBudgetWindow::Daily,
        }),
    };
    req
}

fn usecase_with_budget(
    provider: Arc<dyn ProviderPort>,
    ledger: Arc<dyn SpendingBudgetLedgerPort>,
) -> RouteRequest {
    make_usecase_with_provider(provider).with_spending_budget_ledger(ledger)
}

#[tokio::test]
async fn budgeted_request_reserves_before_upstream_and_settles_afterwards() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(
        BoundedProvider::new("bounded-provider", events.clone())
            .with_bound(500_000)
            .with_realized_cost(120_000),
    );
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_budget(provider, ledger);

    usecase
        .execute(budgeted_request())
        .await
        .expect("a bounded budgeted request is served");

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            BudgetEvent::Reserve,
            BudgetEvent::UpstreamCall,
            BudgetEvent::Settle(Some(120_000)),
        ],
        "the reservation must exist before upstream I/O and the final cost must be settled"
    );
}

#[tokio::test]
async fn budget_reservation_is_released_if_managed_connection_is_disabled_before_upstream() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let connection_id = ConnectionId::new();
    let provider = super::connection_bound_provider(
        Arc::new(
            BoundedProvider::new(&connection_id.to_string(), events.clone()).with_bound(500_000),
        ),
        connection_id,
    );
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(super::InactiveAfterFirstRoutingReadRepository {
            connection: super::test_connection(
                connection_id,
                ProviderId::new("budgeted-managed-runtime"),
                true,
            ),
            reads: std::sync::atomic::AtomicUsize::new(0),
            pending_after_first: None,
            adapter_revision_after_first: None,
        });
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = super::usecase_with_router_and_provider_repository(router, repository)
        .with_spending_budget_ledger(ledger);

    let error = usecase
        .execute(budgeted_request())
        .await
        .expect_err("disabled connection must not receive the request");

    assert!(error.to_string().contains("all providers exhausted"));
    assert_eq!(
        *events.lock().unwrap(),
        vec![BudgetEvent::Reserve, BudgetEvent::Release],
        "a reservation must be released when no upstream I/O occurred"
    );
}

#[tokio::test]
async fn cancellation_during_managed_connection_recheck_releases_budget_reservation() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("pending-managed-runtime");
    let provider = super::connection_bound_provider(
        Arc::new(
            BoundedProvider::new(&connection_id.to_string(), events.clone()).with_bound(500_000),
        ),
        connection_id,
    );
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let recheck_started = Arc::new(tokio::sync::Notify::new());
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(super::InactiveAfterFirstRoutingReadRepository {
            connection: super::test_connection(connection_id, runtime_id, true),
            reads: std::sync::atomic::AtomicUsize::new(0),
            pending_after_first: Some(recheck_started.clone()),
            adapter_revision_after_first: None,
        });
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let release_notify = ledger.release_notify.clone();
    let usecase = super::usecase_with_router_and_provider_repository(router, repository)
        .with_spending_budget_ledger(ledger);
    let task = tokio::spawn(async move { usecase.execute(budgeted_request()).await });

    recheck_started.notified().await;
    assert_eq!(*events.lock().unwrap(), vec![BudgetEvent::Reserve]);
    task.abort();
    let _ = task.await;

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if events.lock().unwrap().contains(&BudgetEvent::Release) {
                break;
            }
            release_notify.notified().await;
        }
    })
    .await
    .expect("dropping the preflight must schedule reservation release");
    assert_eq!(
        *events.lock().unwrap(),
        vec![BudgetEvent::Reserve, BudgetEvent::Release]
    );
}

#[tokio::test]
async fn budgeted_request_without_a_verifiable_bound_never_reaches_the_provider() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(BoundedProvider::new("unbounded-provider", events.clone()));
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_budget(provider, ledger);

    let error = usecase
        .execute(budgeted_request())
        .await
        .expect_err("a request without a verifiable bound must be rejected");

    assert!(error.is_cost_unavailable());
    assert!(error.to_string().contains("cost unavailable"));
    assert_eq!(
        *events.lock().unwrap(),
        Vec::<BudgetEvent>::new(),
        "no reservation may be created and no upstream call may happen without a bound"
    );
}

#[tokio::test]
async fn budgeted_request_above_the_per_request_limit_never_reaches_the_provider() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let provider =
        Arc::new(BoundedProvider::new("expensive-provider", events.clone()).with_bound(2_000_000));
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_budget(provider, ledger);

    let error = usecase
        .execute(budgeted_request())
        .await
        .expect_err("a bound above the per-request limit must be rejected");

    assert!(error.is_cost_unavailable());
    assert!(error.to_string().contains("per-request limit"));
    assert_eq!(
        *events.lock().unwrap(),
        Vec::<BudgetEvent>::new(),
        "a rejected bound must not create a reservation"
    );
}

#[tokio::test]
async fn exhausted_budget_never_reaches_the_provider() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let provider =
        Arc::new(BoundedProvider::new("bounded-provider", events.clone()).with_bound(500_000));
    let ledger = Arc::new(RecordingLedger::with_reserve_outcome(
        events.clone(),
        Err(SpendingBudgetReservationError::BudgetExceeded),
    ));
    let usecase = usecase_with_budget(provider, ledger);

    let error = usecase
        .execute(budgeted_request())
        .await
        .expect_err("an exhausted budget must be rejected");

    assert!(error.is_spending_budget_exceeded());
    assert_eq!(
        *events.lock().unwrap(),
        vec![BudgetEvent::Reserve],
        "an exhausted budget must not reach the provider"
    );
}

#[tokio::test]
async fn a_failed_upstream_call_keeps_the_reservation_charged_as_unresolved() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(
        BoundedProvider::new("failing-provider", events.clone())
            .with_bound(500_000)
            .failing("upstream exploded"),
    );
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_budget(provider, ledger);

    usecase
        .execute(budgeted_request())
        .await
        .expect_err("the upstream failure is reported to the caller");

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            BudgetEvent::Reserve,
            BudgetEvent::UpstreamCall,
            BudgetEvent::Settle(None),
        ],
        "an unknown final cost keeps the reservation charged instead of releasing it"
    );
}

#[tokio::test]
async fn a_budgeted_request_advances_after_local_backpressure() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(
        BoundedProvider::new("first-provider", events.clone())
            .with_bound(500_000)
            .locally_backpressured(),
    );
    let second =
        Arc::new(BoundedProvider::new("second-provider", events.clone()).with_bound(500_000));
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter {
        providers: vec![first, second],
    }))
    .with_spending_budget_ledger(ledger);
    let mut req = budgeted_request();
    req.stream = false;

    let response = usecase
        .execute(req)
        .await
        .expect("local admission pressure may advance provider");

    assert_eq!(response.provider, ProviderId::new("second-provider"));
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            BudgetEvent::Reserve,
            BudgetEvent::Release,
            BudgetEvent::Reserve,
            BudgetEvent::UpstreamCall,
            BudgetEvent::Settle(None)
        ],
        "pre-upstream backpressure releases its reservation before the second provider is admitted"
    );
}

#[tokio::test]
async fn budgeted_request_falls_back_when_provider_bound_fits_initial_reservation() {
    // Reserve exactly once before the first upstream attempt.
    // Retain the original admission across retryable provider failovers.
    // Do NOT re-admit against each provider.
    // Earlier upstream spend is unknown, so settle as unresolved.
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(
        BoundedProvider::new("first-provider", events.clone())
            .with_bound(500_000)
            .failing("upstream exploded"),
    );
    let second = Arc::new(
        BoundedProvider::new("second-provider", events.clone())
            .with_bound(500_000)
            .with_realized_cost(120_000),
    );
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter {
        providers: vec![first, second],
    }))
    .with_spending_budget_ledger(ledger);

    let response = usecase
        .execute(budgeted_request())
        .await
        .expect("a retryable upstream failure may fall back under the original reservation");

    assert_eq!(response.provider, ProviderId::new("second-provider"));
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            BudgetEvent::Reserve,
            BudgetEvent::UpstreamCall,
            BudgetEvent::UpstreamCall,
            BudgetEvent::Settle(None),
        ],
        "reserve once, failover without re-admit, settle once with unresolved cost"
    );
}

#[tokio::test]
async fn budgeted_failover_stops_before_provider_bound_exceeds_initial_reservation() {
    // First provider has a lower bound (500k), second has a higher bound (900k).
    // Only one reserve should happen (against the first provider's bound).
    // The second provider must NOT trigger a new admission.
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(
        BoundedProvider::new("first-provider", events.clone())
            .with_bound(500_000)
            .failing("upstream exploded"),
    );
    let second = Arc::new(
        BoundedProvider::new("second-provider", events.clone())
            .with_bound(900_000)
            .with_realized_cost(120_000),
    );
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter {
        providers: vec![first, second],
    }))
    .with_spending_budget_ledger(ledger);

    let error = usecase
        .execute(budgeted_request())
        .await
        .expect_err("fallback must stop when its strict bound exceeds the initial reservation");

    assert!(
        error.is_cost_unavailable(),
        "the request must fail with a cost-bound error: {error}"
    );
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            BudgetEvent::Reserve,
            BudgetEvent::UpstreamCall,
            BudgetEvent::Settle(None),
        ],
        "reserve once, do not call the over-bound fallback, and settle unknown spend once"
    );
}

#[tokio::test]
async fn budgeted_failover_chain_terminating_in_failure_settles_none_once() {
    // When all providers fail, the reservation is settled once with None.
    // No intermediate settles between attempts.
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(
        BoundedProvider::new("first-provider", events.clone())
            .with_bound(500_000)
            .failing("first failed"),
    );
    let second = Arc::new(
        BoundedProvider::new("second-provider", events.clone())
            .with_bound(500_000)
            .failing("second also failed"),
    );
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter {
        providers: vec![first, second],
    }))
    .with_spending_budget_ledger(ledger);

    usecase
        .execute(budgeted_request())
        .await
        .expect_err("all providers failed");

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            BudgetEvent::Reserve,
            BudgetEvent::UpstreamCall,
            BudgetEvent::UpstreamCall,
            BudgetEvent::Settle(None),
        ],
        "reserve once, all attempts fail, settle None once at the end"
    );
}

#[tokio::test]
async fn a_budgeted_combo_request_is_rejected_before_any_upstream_call() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let provider =
        Arc::new(BoundedProvider::new("bounded-provider", events.clone()).with_bound(500_000));
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_budget(provider, ledger);

    let mut req = budgeted_request();
    req.metadata.combo_id = Some(shared_kernel::ComboId::new());

    let error = usecase
        .execute(req)
        .await
        .expect_err("a budgeted combo request must be rejected");

    assert!(error.is_cost_unavailable());
    assert!(error.to_string().contains("combo"));
    assert_eq!(
        *events.lock().unwrap(),
        Vec::<BudgetEvent>::new(),
        "a rejected combo must not reserve nor call the provider"
    );
}

#[tokio::test]
async fn direct_and_virtual_combo_calls_reject_budgets_before_upstream() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let provider =
        Arc::new(BoundedProvider::new("bounded-provider", events.clone()).with_bound(500_000));
    let mut usecase = make_usecase_with_provider(provider);
    let combo = Combo::new(
        "budget-combo".into(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: ProviderId::new("bounded-provider"),
            model: TEST_MODEL.clone(),
            connection_id: None,
            priority: 1,
        }],
    );
    usecase.combo_repository = Some(Arc::new(MapComboRepository {
        combos: HashMap::from([(combo.id, combo.clone())]),
    }));
    usecase.combo_alias_repository = Some(Arc::new(MapComboAliasRepository {
        aliases: HashMap::from([(
            "rook/budget".into(),
            ComboAlias::new(ModelId::new("rook/budget"), combo.id),
        )]),
    }));
    let error = usecase
        .execute_combo(&combo, budgeted_request())
        .await
        .unwrap_err();
    assert!(error.is_cost_unavailable());
    let mut req = budgeted_request();
    req.model = ModelId::new("rook/budget");
    let error = usecase.execute_virtual_combo(req).await.unwrap_err();
    assert!(error.is_cost_unavailable());
    assert!(events.lock().unwrap().is_empty());
}

struct ComboTargetAliasRepository;

#[async_trait]
impl ModelAliasRepositoryPort for ComboTargetAliasRepository {
    async fn find_by_alias(
        &self,
        alias: &ModelId,
        _: Option<&ProviderId>,
    ) -> Result<Option<ModelAlias>, ModelAliasRepositoryError> {
        Ok(Some(ModelAlias {
            alias: alias.clone(),
            canonical: ModelId::new("rook/budget"),
            provider_id: None,
            created_at: Utc::now(),
        }))
    }
    async fn list(&self) -> Result<Vec<ModelAlias>, ModelAliasRepositoryError> {
        Ok(vec![])
    }
    async fn create(&self, _: ModelAlias) -> Result<(), ModelAliasRepositoryError> {
        Ok(())
    }
    async fn delete(&self, _: &ModelId) -> Result<bool, ModelAliasRepositoryError> {
        Ok(false)
    }
    async fn seed(&self, _: Vec<ModelAlias>) -> Result<usize, ModelAliasRepositoryError> {
        Ok(0)
    }
}

#[tokio::test]
async fn aliases_resolving_to_combos_reject_budgets_for_both_execution_modes() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let provider =
        Arc::new(BoundedProvider::new("bounded-provider", events.clone()).with_bound(500_000));
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let mut usecase = usecase_with_budget(provider, ledger);
    usecase.alias_config.enabled = true;
    usecase.alias_repository = Arc::new(ComboTargetAliasRepository);
    let mut req = budgeted_request();
    // This repository maps any ordinary model to a virtual combo model.
    req.model = ModelId::new("ordinary-alias");
    let error = usecase.execute(req.clone()).await.unwrap_err();
    assert!(error.is_cost_unavailable(), "{error}");
    req.stream = true;
    let error = usecase
        .execute_stream(req)
        .await
        .err()
        .expect("combo stream rejected");
    assert!(error.is_cost_unavailable(), "{error}");
    assert!(events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_budgeted_stream_reserves_before_the_stream_and_settles_as_unresolved() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let provider =
        Arc::new(BoundedProvider::new("bounded-provider", events.clone()).with_bound(500_000));
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_budget(provider, ledger);

    let mut stream = usecase
        .execute_stream(budgeted_request())
        .await
        .expect("a bounded budgeted stream is admitted");
    while stream.next().await.is_some() {}

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            BudgetEvent::Reserve,
            BudgetEvent::UpstreamCall,
            BudgetEvent::Settle(None),
        ],
        "a stream without an adapter-verified exact cost keeps the reservation charged"
    );
}

#[tokio::test]
async fn completion_traverses_local_backpressure_after_retry_limit_until_success() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let providers: Vec<Arc<dyn ProviderPort>> = vec![
        Arc::new(BoundedProvider::new("upstream-failure", events.clone()).failing("failed")),
        Arc::new(BoundedProvider::new("backpressure-1", events.clone()).locally_backpressured()),
        Arc::new(BoundedProvider::new("backpressure-2", events.clone()).locally_backpressured()),
        Arc::new(BoundedProvider::new("success", events.clone())),
        Arc::new(BoundedProvider::new("unused", events.clone())),
    ];
    let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter {
        providers: providers.clone(),
    }));
    let mut req = request();
    req.stream = false;

    let response = usecase
        .execute(req)
        .await
        .expect("candidate traversal must continue past upstream retry limit on local pressure");

    assert_eq!(response.provider, ProviderId::new("success"));
    assert_eq!(
        *events.lock().unwrap(),
        vec![BudgetEvent::UpstreamCall, BudgetEvent::UpstreamCall],
        "local backpressure must not count as an upstream retry"
    );
}

#[tokio::test]
async fn a_budgeted_stream_advances_after_local_backpressure() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(
        BoundedProvider::new("first-provider", events.clone())
            .with_bound(500_000)
            .locally_backpressured(),
    );
    let second =
        Arc::new(BoundedProvider::new("second-provider", events.clone()).with_bound(500_000));
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter {
        providers: vec![first, second],
    }))
    .with_spending_budget_ledger(ledger);

    let mut stream = usecase
        .execute_stream(budgeted_request())
        .await
        .expect("local admission pressure may advance before stream data is exposed");
    while stream.next().await.is_some() {}

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            BudgetEvent::Reserve,
            BudgetEvent::Release,
            BudgetEvent::Reserve,
            BudgetEvent::UpstreamCall,
            BudgetEvent::Settle(None),
        ],
        "local backpressure releases its reservation and permits the next provider"
    );
}

#[tokio::test]
async fn a_failed_budgeted_stream_keeps_the_reservation_charged() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(
        BoundedProvider::new("failing-provider", events.clone())
            .with_bound(500_000)
            .stream_failing("upstream exploded"),
    );
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_budget(provider, ledger);

    let failure = usecase.execute_stream(budgeted_request()).await;
    assert!(
        failure.is_err(),
        "the upstream failure is reported to the caller"
    );

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            BudgetEvent::Reserve,
            BudgetEvent::UpstreamCall,
            BudgetEvent::Settle(None),
        ],
        "an upstream stream failure keeps the reservation charged"
    );
}

#[tokio::test]
async fn a_cancelled_budgeted_stream_never_releases_the_reservation() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let provider =
        Arc::new(BoundedProvider::new("bounded-provider", events.clone()).with_bound(500_000));
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_budget(provider, ledger);

    let stream = usecase
        .execute_stream(budgeted_request())
        .await
        .expect("a bounded budgeted stream is admitted");
    // The client disconnects before consuming the stream.
    drop(stream);

    let recorded = events.lock().unwrap().clone();
    assert!(
        recorded.contains(&BudgetEvent::Settle(None)),
        "the reservation must already be durably unresolved before the client can cancel: {recorded:?}"
    );
    assert!(
        !recorded.contains(&BudgetEvent::Release),
        "an abandoned stream must not release the reservation: the cost is unknown"
    );
}

/// Provider that blocks on a signal before returning from `complete`.
struct BlockingBoundedProvider {
    id: ProviderId,
    events: EventLog,
    bound: u64,
    realized_cost: Option<u64>,
    /// When `Some`, the provider waits on this notify before returning.
    block_until: Option<Arc<tokio::sync::Notify>>,
}

impl BlockingBoundedProvider {
    fn new(id: &str, events: EventLog, bound: u64) -> Self {
        Self {
            id: ProviderId::new(id),
            events,
            bound,
            realized_cost: None,
            block_until: None,
        }
    }

    fn with_block(mut self, notify: Arc<tokio::sync::Notify>) -> Self {
        self.block_until = Some(notify);
        self
    }
}

#[async_trait]
impl ProviderPort for BlockingBoundedProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn supported_models(&self) -> &[ModelId] {
        std::slice::from_ref(&TEST_MODEL)
    }

    fn model_capabilities(&self, _: &ModelId) -> rook_core::ModelCapabilities {
        fixture_capabilities()
    }

    fn api_format(&self) -> rook_core::ApiFormat {
        rook_core::ApiFormat::OpenAI
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn health_check(&self) -> rook_core::HealthStatus {
        rook_core::HealthStatus::Healthy {
            provider: self.id.clone(),
            latency_ms: 1,
        }
    }

    fn strict_max_cost_micros_usd(&self, _req: &CompletionRequest) -> Option<u64> {
        Some(self.bound)
    }

    fn strict_realized_cost_micros_usd(
        &self,
        _req: &CompletionRequest,
        _resp: &CompletionResponse,
    ) -> Option<u64> {
        self.realized_cost
    }

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        self.events.lock().unwrap().push(BudgetEvent::UpstreamCall);
        if let Some(notify) = &self.block_until {
            notify.notified().await;
        }
        Ok(CompletionResponse {
            id: req.id.clone(),
            provider: self.id.clone(),
            model: req.model.clone(),
            content: "ok".to_string(),
            content_blocks: vec![],
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
    ) -> CortexResult<
        futures::stream::BoxStream<'static, Result<rook_core::StreamChunk, CortexError>>,
    > {
        Ok(Box::pin(stream::empty()))
    }
}

/// Cancellation during the provider call must NOT release the reservation.
///
/// Once upstream I/O has begun, the reservation may already represent incurred
/// upstream spend. Releasing it would incorrectly refund the budget.
#[tokio::test]
async fn cancellation_during_provider_call_does_not_release_reservation() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let block = Arc::new(tokio::sync::Notify::new());
    let provider = Arc::new(
        BlockingBoundedProvider::new("blocking-provider", events.clone(), 500_000)
            .with_block(block.clone()),
    );
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_budget(provider, ledger);
    let task = tokio::spawn(async move { usecase.execute(budgeted_request()).await });

    // Wait until upstream I/O has actually started.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if events.lock().unwrap().contains(&BudgetEvent::UpstreamCall) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("upstream I/O must start");

    // Abort the task mid-I/O.
    task.abort();
    let _ = task.await;

    // Give the spawned Drop task time to execute (if it exists).
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let recorded = events.lock().unwrap().clone();
    assert!(
        !recorded.contains(&BudgetEvent::Release),
        "cancellation after upstream I/O must NOT release the reservation; events: {recorded:?}"
    );
}

/// `settle_outstanding_admission` must not race with `Drop`'s release.
///
/// When the retry chain terminates without success, `settle_outstanding_admission`
/// calls `take()` then `settle(None)`. The taken carrier's `Drop` must NOT
/// additionally spawn a `release()` — that would be a double terminal action.
#[tokio::test]
async fn settle_outstanding_admission_does_not_double_act_with_drop() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(
        BoundedProvider::new("first-provider", events.clone())
            .with_bound(500_000)
            .failing("first failed"),
    );
    let second = Arc::new(
        BoundedProvider::new("second-provider", events.clone())
            .with_bound(500_000)
            .failing("second also failed"),
    );
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter {
        providers: vec![first, second],
    }))
    .with_spending_budget_ledger(ledger);

    usecase
        .execute(budgeted_request())
        .await
        .expect_err("all providers failed");

    // Give the spawned Drop task time to execute (if it exists).
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        // Wait a bit for any async Drop release to fire
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    })
    .await
    .ok();

    let recorded = events.lock().unwrap().clone();
    let release_count = recorded
        .iter()
        .filter(|e| matches!(e, BudgetEvent::Release))
        .count();
    assert_eq!(
        release_count, 0,
        "settle_outstanding_admission must settle (not release), and Drop must not release; events: {recorded:?}"
    );
    let settle_count = recorded
        .iter()
        .filter(|e| matches!(e, BudgetEvent::Settle(_)))
        .count();
    assert_eq!(
        settle_count, 1,
        "exactly one settle must occur; events: {recorded:?}"
    );
}

/// Ledger that blocks on `release_spending_before_upstream` until unblocked.
struct BlockingRecordingLedger {
    events: EventLog,
    /// When `Some`, blocks release until this notify is signalled.
    block_release: Mutex<Option<Arc<tokio::sync::Notify>>>,
    /// Signalled when the release method enters its blocked await (before the block).
    release_started: Arc<tokio::sync::Notify>,
    /// Signalled only after the blocked await returns (release truly completed).
    release_completed: Arc<tokio::sync::Notify>,
}

impl BlockingRecordingLedger {
    fn new(
        events: EventLog,
        block: Arc<tokio::sync::Notify>,
        release_started: Arc<tokio::sync::Notify>,
        release_completed: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self {
            events,
            block_release: Mutex::new(Some(block)),
            release_started,
            release_completed,
        }
    }
}

#[async_trait]
impl SpendingBudgetLedgerPort for BlockingRecordingLedger {
    async fn reserve_spending(
        &self,
        _reservation: &SpendingBudgetReservation,
    ) -> Result<SpendingReservationOutcome, SpendingBudgetReservationError> {
        self.events.lock().unwrap().push(BudgetEvent::Reserve);
        Ok(SpendingReservationOutcome::Reserved)
    }

    async fn settle_spending(
        &self,
        _api_key_id: &ApiKeyId,
        _request_id: &RequestId,
        cost_micros_usd: Option<u64>,
    ) -> Result<(), SpendingBudgetReservationError> {
        self.events
            .lock()
            .unwrap()
            .push(BudgetEvent::Settle(cost_micros_usd));
        Ok(())
    }

    async fn release_spending_before_upstream(
        &self,
        _api_key_id: &ApiKeyId,
        _request_id: &RequestId,
    ) -> Result<(), SpendingBudgetReservationError> {
        self.events.lock().unwrap().push(BudgetEvent::Release);
        self.release_started.notify_one();
        let notify = self.block_release.lock().unwrap().take();
        if let Some(n) = notify {
            n.notified().await;
        }
        self.release_completed.notify_one();
        Ok(())
    }
}

/// Cancellation of the task that requests an explicit pre-upstream release
/// must not cancel the ledger release itself.
///
/// The ledger blocks on release. The caller task is aborted while the release
/// is in-flight. After unblocking the ledger, the release must still complete
/// because it runs in a spawned bounded task independent of the caller.
#[tokio::test]
async fn release_cancellation_does_not_prevent_ledger_release() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("release-block-runtime");
    let provider = super::connection_bound_provider(
        Arc::new(
            BoundedProvider::new(&connection_id.to_string(), events.clone()).with_bound(500_000),
        ),
        connection_id,
    );
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        rook_core::PolicyKind::Priority,
    ));
    let recheck_started = Arc::new(tokio::sync::Notify::new());
    let repository: Arc<dyn ProviderRepositoryPort> =
        Arc::new(super::InactiveAfterFirstRoutingReadRepository {
            connection: super::test_connection(connection_id, runtime_id, true),
            reads: std::sync::atomic::AtomicUsize::new(0),
            pending_after_first: Some(recheck_started.clone()),
            adapter_revision_after_first: None,
        });
    // Block the ledger release so we can abort the caller mid-release.
    let block_release = Arc::new(tokio::sync::Notify::new());
    let release_started = Arc::new(tokio::sync::Notify::new());
    let release_completed = Arc::new(tokio::sync::Notify::new());
    let ledger = Arc::new(BlockingRecordingLedger::new(
        events.clone(),
        block_release.clone(),
        release_started.clone(),
        release_completed.clone(),
    ));
    let usecase = super::usecase_with_router_and_provider_repository(router, repository)
        .with_spending_budget_ledger(ledger);

    let task = tokio::spawn(async move { usecase.execute(budgeted_request()).await });

    // Wait until the recheck is in progress (reservation created).
    recheck_started.notified().await;
    assert_eq!(*events.lock().unwrap(), vec![BudgetEvent::Reserve]);

    // Wait until the spawned release task has entered the blocked await (deterministic, no sleep).
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        release_started.notified(),
    )
    .await
    .expect("release must start before we abort");

    // Abort the task while the explicit release is blocked in the ledger.
    task.abort();
    let _ = task.await;

    // Unblock the ledger release.
    block_release.notify_one();

    // The release must still complete despite the caller being cancelled.
    // Waiting for release_completed proves the method returned after the blocked await.
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        release_completed.notified(),
    )
    .await
    .expect("release must complete even after caller cancellation");

    assert_eq!(
        *events.lock().unwrap(),
        vec![BudgetEvent::Reserve, BudgetEvent::Release],
        "reservation must be released despite caller cancellation"
    );
}

#[tokio::test]
async fn budgeted_request_is_admitted_when_catalog_pricing_bounds_the_cost() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    // The adapter exposes no bound of its own; the catalog pricing must supply it.
    let unbounded = Arc::new(BoundedProvider::new("priced-provider", events.clone()));
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(TEST_MODEL.clone(), rook_core::ModelCapabilities::default());
    let provider = Arc::new(
        crate::catalogued_provider::CataloguedProvider::new(
            unbounded,
            metadata,
            rook_core::ModelCapabilities::default(),
        )
        .with_pricing(Arc::new(crate::PricingConfig::from_entry(
            "priced-provider",
            TEST_MODEL.as_str(),
            0.5,
            1.5,
        ))),
    );
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let mut req = budgeted_request();
    req.stream = false;
    req.max_tokens = Some(100);

    let usecase = usecase_with_budget(provider, ledger);
    usecase
        .execute(req)
        .await
        .expect("budgeted request must be admitted");
    assert!(
        events.lock().unwrap().contains(&BudgetEvent::Reserve),
        "the guard must have reserved against the derived bound"
    );
}

#[tokio::test]
async fn local_backpressure_exhaustion_preserves_ledger_rows() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let providers = ["first", "second"]
        .into_iter()
        .map(|id| {
            Arc::new(
                BoundedProvider::new(id, events.clone())
                    .with_bound(500_000)
                    .locally_backpressured(),
            ) as Arc<dyn ProviderPort>
        })
        .collect();
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter { providers }))
        .with_spending_budget_ledger(ledger.clone());
    assert!(usecase
        .execute(budgeted_request())
        .await
        .unwrap_err()
        .is_local_backpressure());
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            BudgetEvent::Reserve,
            BudgetEvent::Release,
            BudgetEvent::Reserve,
            BudgetEvent::Release
        ]
    );
    assert_eq!(ledger.rows.lock().unwrap().len(), 2);
    assert!(ledger
        .rows
        .lock()
        .unwrap()
        .values()
        .all(|state| *state == BudgetEvent::Release));
}

#[tokio::test]
async fn upstream_failure_then_local_pressure_retains_unresolved_admission() {
    for fallback_succeeds in [false, true] {
        let events: EventLog = Arc::new(Mutex::new(Vec::new()));
        let mut providers: Vec<Arc<dyn ProviderPort>> = vec![
            Arc::new(
                BoundedProvider::new("failed", events.clone())
                    .with_bound(500_000)
                    .failing("unknown spend"),
            ),
            Arc::new(
                BoundedProvider::new("busy", events.clone())
                    .with_bound(500_000)
                    .locally_backpressured(),
            ),
        ];
        if fallback_succeeds {
            providers.push(Arc::new(
                BoundedProvider::new("success", events.clone())
                    .with_bound(500_000)
                    .with_realized_cost(120_000),
            ));
        }
        let ledger = Arc::new(RecordingLedger::new(events.clone()));
        let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter { providers }))
            .with_spending_budget_ledger(ledger.clone());
        let result = usecase.execute(budgeted_request()).await;
        assert_eq!(result.is_ok(), fallback_succeeds);
        let mut expected = vec![BudgetEvent::Reserve, BudgetEvent::UpstreamCall];
        if fallback_succeeds {
            expected.push(BudgetEvent::UpstreamCall);
        }
        expected.push(BudgetEvent::Settle(None));
        assert_eq!(*events.lock().unwrap(), expected);
        assert_eq!(ledger.rows.lock().unwrap().len(), 1);
        assert_eq!(
            ledger.rows.lock().unwrap().values().next(),
            Some(&BudgetEvent::Settle(None))
        );
    }
}
