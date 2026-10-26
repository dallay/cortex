use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use rook_core::*;
use rook_usecases::{FallbackRouter, PricingConfig, RouteRequest, RoutingStrategy};

mod common;

struct Provider {
    id: ProviderId,
    models: Vec<ModelId>,
    capabilities: ModelCapabilities,
    calls: AtomicUsize,
}

impl Provider {
    fn new(id: &str, tools: CapabilityState) -> Arc<Self> {
        let capabilities = ModelCapabilities {
            tools: CapabilityEvidence {
                state: tools,
                source: "test_fixture".into(),
            },
            ..Default::default()
        };
        Arc::new(Self {
            id: ProviderId::new(id),
            models: vec![ModelId::new("custom")],
            capabilities,
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl ProviderPort for Provider {
    fn id(&self) -> &ProviderId {
        &self.id
    }
    fn supported_models(&self) -> &[ModelId] {
        &self.models
    }
    fn model_capabilities(&self, _: &ModelId) -> ModelCapabilities {
        self.capabilities.clone()
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
    async fn complete(&self, _: &CompletionRequest) -> CortexResult<CompletionResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(CortexError::provider("test upstream called"))
    }
    async fn stream(
        &self,
        _: &CompletionRequest,
    ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(CortexError::provider("test upstream called"))
    }
}

fn request() -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("custom"),
        messages: vec![],
        stream: false,
        max_tokens: None,
        temperature: None,
        tools: Some(serde_json::json!([{"type":"function","function":{"name":"lookup"}}])),
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".into(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions::default(),
    }
}

fn route(router: Arc<dyn RouterPort>) -> RouteRequest {
    RouteRequest::new(
        router,
        Arc::new(common::NoOpCache),
        Arc::new(common::NoOpAudit),
        None,
        None,
        None,
        Arc::new(PricingConfig::default()),
        Arc::new(common::NoOpTranslator),
        Arc::new(common::NoOpAliasRepository),
        rook_usecases::route_request::ModelAliasesConfig {
            enabled: false,
            auto_seed: false,
        },
        None,
    )
}

#[tokio::test]
async fn tools_unknown_and_unsupported_never_execute_upstream() {
    for state in [CapabilityState::Unknown, CapabilityState::Unsupported] {
        let provider = Provider::new("blocked", state);
        let router = Arc::new(FallbackRouter::new(
            vec![provider.clone()],
            RoutingStrategy::Priority,
        ));
        assert!(route(router).execute(request()).await.is_err());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn selection_skips_incompatible_candidates_and_does_not_open_circuit() {
    let blocked = Provider::new("blocked", CapabilityState::Unsupported);
    let allowed = Provider::new("allowed", CapabilityState::Supported);
    let router = FallbackRouter::new(vec![blocked.clone(), allowed], RoutingStrategy::Priority);
    assert_eq!(
        router.select(&request()).await.unwrap().id().as_str(),
        "allowed"
    );
    assert_eq!(
        router
            .select_excluding(&request(), &[])
            .await
            .unwrap()
            .id()
            .as_str(),
        "allowed"
    );
    assert!(router.circuit_states().iter().all(|(_, c)| c.failures == 0));
    let mut text = request();
    text.tools = None;
    assert_eq!(router.select(&text).await.unwrap().id().as_str(), "blocked");
}

#[tokio::test]
async fn declared_metadata_is_forwarded_and_adapter_limits_cannot_be_overridden() {
    let inner = Provider::new("wrapped", CapabilityState::Supported);
    let declared = ModelCapabilities {
        tools: CapabilityEvidence {
            state: CapabilityState::Supported,
            source: "connection_configuration".into(),
        },
        ..Default::default()
    };
    let ceiling = ModelCapabilities {
        tools: CapabilityEvidence {
            state: CapabilityState::Unsupported,
            source: "adapter".into(),
        },
        ..Default::default()
    };
    let provider = rook_usecases::catalogued_provider::CataloguedProvider::new(
        inner.clone(),
        [(ModelId::new("custom"), declared)].into(),
        ceiling,
    );
    assert_eq!(
        provider
            .model_capabilities(&ModelId::new("custom"))
            .tools
            .state,
        CapabilityState::Unsupported
    );
    assert!(provider.complete(&request()).await.is_err());
    assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unknown_native_streaming_never_executes_upstream() {
    let provider = Provider::new("unknown-stream", CapabilityState::Supported);
    let router = Arc::new(FallbackRouter::new(
        vec![provider.clone()],
        RoutingStrategy::Priority,
    ));
    let mut req = request();
    req.stream = true;
    assert!(route(router).execute_stream(req).await.is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn weighted_selection_preserves_weights_after_capability_filtering_and_exclusion() {
    let blocked = Provider::new("blocked", CapabilityState::Unsupported);
    let first = Provider::new("first", CapabilityState::Supported);
    let excluded = Provider::new("excluded", CapabilityState::Supported);
    let weighted = Provider::new("weighted", CapabilityState::Supported);
    let router = FallbackRouter::new(
        vec![blocked, first, excluded, weighted],
        RoutingStrategy::WeightedRandom(vec![1.0, 0.0, 0.0, 1.0]),
    );
    let excluded = [ProviderId::new("excluded")];
    let mut selected_weighted = false;
    let mut selected_weighted_excluding = false;
    // Multiple draws avoid relying on the random sample being strictly above zero.
    for _ in 0..32 {
        selected_weighted |= router.select(&request()).await.unwrap().id().as_str() == "weighted";
        let selected = router
            .select_excluding(&request(), &excluded)
            .await
            .unwrap();
        assert_ne!(selected.id(), &excluded[0]);
        selected_weighted_excluding |= selected.id().as_str() == "weighted";
    }
    assert!(selected_weighted);
    assert!(selected_weighted_excluding);
    assert!(router.circuit_states().iter().all(|(_, c)| c.failures == 0));
}
