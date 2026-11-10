//! Preflight guards that must hold before any upstream I/O.
//!
//! These tests cover the two routing surfaces Rook supports: the eligibility
//! stage (routers that publish a candidate snapshot) and the legacy path
//! (routers that do not). Both must skip or reject a candidate before the
//! provider is called.

use super::*;
use crate::catalogued_provider::CataloguedProvider;
use rook_core::QuotaObservationQuery;

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
        crate::RoutingStrategy::Priority,
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
