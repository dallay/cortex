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
    reserve_outcome:
        Mutex<Option<Result<SpendingReservationOutcome, SpendingBudgetReservationError>>>,
}

impl RecordingLedger {
    fn new(events: EventLog) -> Self {
        Self {
            events,
            reserve_outcome: Mutex::new(None),
        }
    }

    fn with_reserve_outcome(
        events: EventLog,
        outcome: Result<SpendingReservationOutcome, SpendingBudgetReservationError>,
    ) -> Self {
        Self {
            events,
            reserve_outcome: Mutex::new(Some(outcome)),
        }
    }
}

#[async_trait]
impl SpendingBudgetLedgerPort for RecordingLedger {
    async fn reserve_spending(
        &self,
        _reservation: &SpendingBudgetReservation,
    ) -> Result<SpendingReservationOutcome, SpendingBudgetReservationError> {
        self.events.lock().unwrap().push(BudgetEvent::Reserve);
        if let Some(outcome) = self.reserve_outcome.lock().unwrap().take() {
            return outcome;
        }
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
async fn a_budgeted_request_is_not_retried_on_another_provider() {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(
        BoundedProvider::new("first-provider", events.clone())
            .with_bound(500_000)
            .failing("upstream exploded"),
    );
    let second =
        Arc::new(BoundedProvider::new("second-provider", events.clone()).with_bound(500_000));
    let ledger = Arc::new(RecordingLedger::new(events.clone()));
    let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter {
        providers: vec![first.clone(), second.clone()],
    }))
    .with_spending_budget_ledger(ledger);

    usecase
        .execute(budgeted_request())
        .await
        .expect_err("the upstream failure is reported to the caller");

    let recorded = events.lock().unwrap().clone();
    assert_eq!(
        recorded
            .iter()
            .filter(|event| **event == BudgetEvent::UpstreamCall)
            .count(),
        1,
        "a budgeted request must not retry on another provider: {recorded:?}"
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
        .execute_combo(&combo, budgeted_request(), ApiFormat::OpenAI)
        .await
        .unwrap_err();
    assert!(error.is_cost_unavailable());
    let mut req = budgeted_request();
    req.model = ModelId::new("rook/budget");
    let error = usecase
        .execute_virtual_combo(req, ApiFormat::OpenAI)
        .await
        .unwrap_err();
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
