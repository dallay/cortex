//! Preflight guards that must hold before any upstream I/O.
//!
//! These tests cover the two routing surfaces Rook supports: the eligibility
//! stage (routers that publish a candidate snapshot) and the legacy path
//! (routers that do not). Both must skip or reject a candidate before the
//! provider is called.

use super::*;
use crate::catalogued_provider::CataloguedProvider;
use rook_core::QuotaObservationQuery;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct ConcurrentQuotaStore {
    active_reads: AtomicUsize,
    max_active_reads: AtomicUsize,
    query_count: AtomicUsize,
}

impl ConcurrentQuotaStore {
    fn observe_max(&self, active: usize) {
        self.max_active_reads.fetch_max(active, Ordering::SeqCst);
    }
}

#[async_trait]
impl QuotaObservationRecorderPort for ConcurrentQuotaStore {
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
        query: &QuotaObservationQuery,
    ) -> CortexResult<Vec<QuotaObservation>> {
        assert_eq!(query.connection_ids.len(), 1);
        self.query_count.fetch_add(1, Ordering::SeqCst);
        let active = self.active_reads.fetch_add(1, Ordering::SeqCst) + 1;
        self.observe_max(active);
        tokio::time::sleep(Duration::from_millis(40)).await;
        self.active_reads.fetch_sub(1, Ordering::SeqCst);
        Ok(Vec::new())
    }
}

/// Quota store whose reads always fail.
struct FailingQuotaStore;

#[async_trait]
impl QuotaObservationRecorderPort for FailingQuotaStore {
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
        Err(CortexError::provider("quota store unavailable"))
    }
}

#[tokio::test(start_paused = true)]
async fn quota_reads_are_concurrent_bounded_and_issued_once_per_connection() {
    let connection_count = super::QUOTA_READ_CONCURRENCY + 2;
    let mut providers = Vec::with_capacity(connection_count);
    let mut connections = HashMap::new();
    for index in 0..connection_count {
        let connection_id = ConnectionId::new();
        let runtime_id = ProviderId::new(format!("quota-concurrency-runtime-{index}"));
        providers.push(connection_bound_provider(
            Arc::new(SpyProvider {
                id: ProviderId::new(connection_id.to_string()),
                model: TEST_MODEL.clone(),
                seen_models: Arc::new(Mutex::new(Vec::new())),
            }),
            connection_id,
        ));
        connections.insert(
            connection_id,
            test_connection(connection_id, runtime_id, true),
        );
    }
    let router: Arc<dyn RouterPort> = Arc::new(crate::FallbackRouter::new(
        providers,
        rook_core::PolicyKind::Priority,
    ));
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(QuotaAwareProviderRepository {
        connections,
        runtime_connections: HashMap::new(),
    });
    let quota_store = Arc::new(ConcurrentQuotaStore {
        active_reads: AtomicUsize::new(0),
        max_active_reads: AtomicUsize::new(0),
        query_count: AtomicUsize::new(0),
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository)
        .with_quota_observation_reader(quota_store.clone());
    let mut req = request();
    req.metadata.cacheable = false;

    usecase
        .execute(req)
        .await
        .expect("quota reads should complete within the shared deadline");

    assert_eq!(
        quota_store.query_count.load(Ordering::SeqCst),
        connection_count
    );
    let max_active = quota_store.max_active_reads.load(Ordering::SeqCst);
    assert!(
        max_active > 1,
        "reads should overlap; observed {max_active}"
    );
    assert!(
        max_active <= super::QUOTA_READ_CONCURRENCY,
        "read concurrency exceeded its configured limit: {max_active}"
    );
}

#[tokio::test]
async fn quota_read_failure_keeps_connection_eligible() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("openai-quota-read-failure-runtime");
    let adapter_id = ProviderId::new(connection_id.to_string());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let router: Arc<dyn RouterPort> = Arc::new(crate::FallbackRouter::new(
        vec![connection_bound_provider(
            Arc::new(SpyProvider {
                id: adapter_id.clone(),
                model: TEST_MODEL.clone(),
                seen_models: calls.clone(),
            }),
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
        .with_quota_observation_reader(Arc::new(FailingQuotaStore));
    let mut req = request();
    req.metadata.cacheable = false;

    let response = usecase
        .execute(req)
        .await
        .expect("a failed quota read must keep the connection eligible");

    assert_eq!(response.provider, adapter_id);
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [TEST_MODEL.to_string()],
        "the connection must still receive the request"
    );
}

#[tokio::test]
async fn legacy_router_fails_closed_when_connection_state_is_unavailable() {
    let connection_id = ConnectionId::new();
    let adapter_id = ProviderId::new(connection_id.to_string());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![connection_bound_provider(
            Arc::new(SpyProvider {
                id: adapter_id,
                model: TEST_MODEL.clone(),
                seen_models: calls.clone(),
            }),
            connection_id,
        )],
        connection_providers: HashMap::new(),
    });
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(FailingProviderRepository {
        fail_routing_state: true,
        pending_routing_state: false,
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository);

    let error = usecase.execute(request()).await.unwrap_err();

    assert!(
        error.to_string().contains("all providers exhausted"),
        "unreadable connection state must fail closed, got: {error}"
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "no upstream call is allowed when the connection state is unreadable"
    );
}

#[tokio::test]
async fn legacy_router_skips_provider_without_required_capabilities() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn ProviderPort> = Arc::new(CataloguedProvider::new(
        Arc::new(SpyProvider {
            id: ProviderId::new("legacy-unknown-capability"),
            model: TEST_MODEL.clone(),
            seen_models: calls.clone(),
        }),
        HashMap::new(),
        fixture_capabilities(),
    ));
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![provider],
        connection_providers: HashMap::new(),
    });
    let usecase = usecase_with_router(router);
    let mut req = request();
    req.tools = Some(serde_json::json!([
        {"type": "function", "function": {"name": "lookup"}}
    ]));

    let error = usecase.execute(req).await.unwrap_err();

    assert!(
        error.to_string().contains("all providers exhausted"),
        "a candidate without the required capabilities must be skipped, got: {error}"
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "a candidate without the required capabilities must not reach the upstream"
    );
}

#[tokio::test]
async fn legacy_router_skips_connection_with_exhausted_quota() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("legacy-exhausted-runtime");
    let adapter_id = ProviderId::new(connection_id.to_string());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![connection_bound_provider(
            Arc::new(SpyProvider {
                id: adapter_id,
                model: TEST_MODEL.clone(),
                seen_models: calls.clone(),
            }),
            connection_id,
        )],
        connection_providers: HashMap::new(),
    });
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(QuotaAwareProviderRepository {
        connections: HashMap::from([(
            connection_id,
            test_connection(connection_id, runtime_id.clone(), true),
        )]),
        runtime_connections: HashMap::new(),
    });
    let now = chrono::Utc::now();
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

    let error = usecase.execute(request()).await.unwrap_err();

    assert!(
        error.to_string().contains("all providers exhausted"),
        "an exhausted connection must be skipped, got: {error}"
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "an exhausted connection must not reach the upstream"
    );
}

#[tokio::test]
async fn legacy_router_does_not_call_managed_provider_without_a_connection() {
    let connection_id = ConnectionId::new();
    let adapter_id = ProviderId::new(connection_id.to_string());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let router: Arc<dyn RouterPort> = Arc::new(OrderedRouter {
        providers: vec![connection_bound_provider(
            Arc::new(SpyProvider {
                id: adapter_id,
                model: TEST_MODEL.clone(),
                seen_models: calls.clone(),
            }),
            connection_id,
        )],
        connection_providers: HashMap::new(),
    });
    let repository: Arc<dyn ProviderRepositoryPort> = Arc::new(QuotaAwareProviderRepository {
        connections: HashMap::new(),
        runtime_connections: HashMap::new(),
    });
    let usecase = usecase_with_router_and_provider_repository(router, repository);

    let error = usecase.execute(request()).await.unwrap_err();

    assert!(
        error.to_string().contains("all providers exhausted"),
        "a managed provider without a stored connection must not be selected, got: {error}"
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "a managed provider without a stored connection must not reach the upstream"
    );
}

#[tokio::test]
async fn scoring_preserves_preflight_quota_without_another_read() {
    let connection_id = ConnectionId::new();
    let runtime_id = ProviderId::new("quota-evidence-runtime");
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
    let observation = quota_observation(
        connection_id,
        runtime_id,
        10.0,
        now,
        Some(now + chrono::Duration::seconds(60)),
    );
    let store = Arc::new(FakeQuotaStore::new(vec![observation.clone()]));
    let usecase = usecase_with_router_and_provider_repository(router, repository)
        .with_quota_observation_reader(store.clone());
    let req = request();
    let eligibility = usecase.eligibility_result(&req).await.unwrap().unwrap();
    let snapshot = usecase
        .build_scoring_snapshot(
            &req,
            Some(&eligibility.result),
            &[],
            &eligibility.quota_observations,
            Some(&eligibility.connections_by_target),
        )
        .unwrap();
    let reason = crate::scoring::build_selection_reason(
        &snapshot,
        &crate::TargetRef(target),
        crate::scoring::SelectionMethod::Priority,
    )
    .unwrap();
    assert_eq!(reason.components.quota, Some(observation));
    assert_eq!(store.queries.lock().unwrap().len(), 1);
}
