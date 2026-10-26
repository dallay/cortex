use super::*;
use crate::catalogued_provider::CataloguedProvider;
use rook_core::{ApiKeyRestrictions, CapabilityState, ModelCapabilities};

type CapabilitySetup = (
    ComboHarness,
    Combo,
    ProviderId,
    ProviderId,
    Arc<Mutex<Vec<String>>>,
    Arc<Mutex<Vec<String>>>,
);

fn setup(first_state: CapabilityState) -> CapabilitySetup {
    let first = ConnectionId::new();
    let second = ConnectionId::new();
    let runtime = ProviderId::new("runtime");
    let first_id = ProviderId::new(first.to_string());
    let second_id = ProviderId::new(second.to_string());
    let first_calls = Arc::new(Mutex::new(Vec::new()));
    let second_calls = Arc::new(Mutex::new(Vec::new()));
    let model = ModelId::new("gpt-test");
    let combo = Combo::new(
        "capabilities".into(),
        ComboStrategy::Priority,
        vec![
            ComboStep {
                provider_id: runtime.clone(),
                model: model.clone(),
                connection_id: Some(first),
                priority: 1,
            },
            ComboStep {
                provider_id: runtime.clone(),
                model: model.clone(),
                connection_id: Some(second),
                priority: 2,
            },
        ],
    );
    let mut capabilities = fixture_capabilities();
    capabilities.tools.state = first_state;
    capabilities.streaming.state = CapabilityState::Unknown;
    let wrap = |id: ProviderId, calls, capabilities| -> Arc<dyn ProviderPort> {
        Arc::new(CataloguedProvider::new(
            Arc::new(SpyProvider {
                id,
                model: model.clone(),
                seen_models: calls,
            }),
            HashMap::from([(model.clone(), capabilities)]),
            fixture_capabilities(),
        ))
    };
    let router = Arc::new(OrderedRouter {
        providers: vec![],
        connection_providers: HashMap::from([
            (
                first,
                wrap(first_id.clone(), first_calls.clone(), capabilities),
            ),
            (
                second,
                wrap(
                    second_id.clone(),
                    second_calls.clone(),
                    fixture_capabilities(),
                ),
            ),
        ]),
    });
    let repository = Arc::new(MapProviderRepository {
        connections: HashMap::from([
            (first, test_connection(first, runtime.clone(), true)),
            (second, test_connection(second, runtime, true)),
        ]),
    });
    let h = combo_harness_with_repositories(
        combo.clone(),
        vec![("rook/capabilities", combo.id)],
        router,
        first_calls.clone(),
        Some(repository),
    );
    (h, combo, first_id, second_id, first_calls, second_calls)
}

#[tokio::test]
async fn connection_bound_combo_skips_unknown_and_unsupported_tools_before_io() {
    for state in [CapabilityState::Unknown, CapabilityState::Unsupported] {
        let (h, combo, _, _, first, second) = setup(state);
        let mut req = combo_req("gpt-test", ApiKeyRestrictions::default());
        req.tools = Some(serde_json::json!([{"type":"function","function":{"name":"lookup"}}]));
        h.usecase
            .execute_combo(&combo, req, ApiFormat::OpenAI)
            .await
            .unwrap();
        assert!(first.lock().unwrap().is_empty());
        assert_eq!(second.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn unknown_capability_does_not_hide_unauthorized_later_step() {
    let (h, _, first_id, _, first, second) = setup(CapabilityState::Unknown);
    let mut req = combo_req(
        "rook/capabilities",
        ApiKeyRestrictions {
            allowed_models: vec![],
            allowed_providers: vec![first_id],
        },
    );
    req.tools = Some(serde_json::json!([{"type":"function","function":{"name":"lookup"}}]));
    let error = h.usecase.execute(req).await.unwrap_err();
    assert!(error.is_forbidden());
    assert!(first.lock().unwrap().is_empty());
    assert!(second.lock().unwrap().is_empty());
}

#[tokio::test]
async fn catalog_alias_visibility_does_not_depend_on_active_router_registration() {
    let (mut h, _, first, second, _, _) = setup(CapabilityState::Unknown);
    let permitted = ApiKeyRestrictions {
        allowed_models: vec![ModelId::new("rook/capabilities"), ModelId::new("gpt-test")],
        allowed_providers: vec![first.clone(), second],
    };
    h.usecase.router = Arc::new(OrderedRouter {
        providers: vec![],
        connection_providers: HashMap::new(),
    });
    assert_eq!(
        h.usecase.visible_combo_aliases(&permitted).await,
        vec![ModelId::new("rook/capabilities")]
    );
    let denied_provider = ApiKeyRestrictions {
        allowed_providers: vec![first],
        ..permitted.clone()
    };
    assert!(h
        .usecase
        .visible_combo_aliases(&denied_provider)
        .await
        .is_empty());
    let denied_model = ApiKeyRestrictions {
        allowed_models: vec![ModelId::new("rook/capabilities")],
        ..permitted
    };
    assert!(h
        .usecase
        .visible_combo_aliases(&denied_model)
        .await
        .is_empty());
}

#[tokio::test]
async fn synthesized_combo_sse_does_not_require_native_streaming_support() {
    let (h, _, _, _, first, _) = setup(CapabilityState::Supported);
    let mut req = combo_req("rook/capabilities", ApiKeyRestrictions::default());
    req.stream = true;
    let chunks: Vec<_> = h.usecase.execute_stream(req).await.unwrap().collect().await;
    assert!(chunks.iter().all(Result::is_ok));
    assert_eq!(first.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn feature_requests_bypass_old_cache_entries() {
    let (mut h, _, _, _, _, _) = setup(CapabilityState::Unknown);
    let cache = Arc::new(TestCache {
        get_calls: Mutex::new(0),
        set_calls: Mutex::new(0),
    });
    h.usecase.cache = cache.clone();
    let mut req = combo_req("gpt-test", ApiKeyRestrictions::default());
    req.metadata.cacheable = true;
    req.tools = Some(serde_json::json!([{"type":"function","function":{"name":"lookup"}}]));
    assert!(h
        .usecase
        .try_get_cached(&req, &req.cache_key())
        .await
        .unwrap()
        .is_none());
    assert_eq!(*cache.get_calls.lock().unwrap(), 0);
}

#[test]
fn vision_unknown_is_not_a_positive_support_claim() {
    let required = rook_core::RequiredCapabilities {
        vision: true,
        ..Default::default()
    };
    assert_eq!(
        required.check(&ModelCapabilities::default()),
        Err("required_capability_unknown")
    );
}

#[tokio::test]
async fn legacy_combo_preserves_capability_skip_reason_without_upstream_attempts() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(CataloguedProvider::new(
        Arc::new(SpyProvider {
            id: ProviderId::new("legacy"),
            model: ModelId::new("gpt-test"),
            seen_models: calls.clone(),
        }),
        HashMap::new(),
        fixture_capabilities(),
    ));
    let router = Arc::new(crate::FallbackRouter::new(
        vec![provider],
        crate::RoutingStrategy::Priority,
    ));
    let combo = Combo::new(
        "legacy-capability".into(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: ProviderId::new("legacy"),
            model: ModelId::new("gpt-test"),
            connection_id: None,
            priority: 1,
        }],
    );
    let h = combo_harness_with_router(combo.clone(), vec![], router, calls.clone());
    let mut req = combo_req("gpt-test", ApiKeyRestrictions::default());
    req.tools = Some(serde_json::json!([{"type":"function","function":{"name":"lookup"}}]));
    let error = h
        .usecase
        .execute_combo(&combo, req, ApiFormat::OpenAI)
        .await
        .unwrap_err();
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(
        error.combo_skip_reasons().unwrap()[0].1,
        "required_capability_unknown"
    );
    assert!(error.to_string().contains("0 steps"), "{error}");
}
