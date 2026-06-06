// Unit tests for FallbackRouter::circuit_states() method.
// Verifies that circuit breaker state snapshots are correctly exposed.

use std::sync::Arc;

use async_trait::async_trait;
use rook_core::{
    ApiFormat, ApiKeyRestrictions, CompletionRequest, CompletionResponse, CortexError,
    CortexResult, HealthStatus, MessageContent, ModelId, ProviderId, ProviderPort, RequestMetadata,
    Role, RouterPort, StreamChunk,
};
use rook_usecases::{FallbackRouter, RoutingStrategy};
use shared_kernel::RequestId;

// --- Fake Provider for Testing ---

#[derive(Clone)]
struct FakeProvider {
    id: ProviderId,
    models: Vec<ModelId>,
}

impl FakeProvider {
    fn new(id: &str, models: Vec<&str>) -> Self {
        Self {
            id: ProviderId::new(id),
            models: models.into_iter().map(ModelId::new).collect(),
        }
    }
}

#[async_trait]
impl ProviderPort for FakeProvider {
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

    async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        Err(CortexError::provider("fake provider error"))
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        Err(CortexError::provider("fake provider error"))
    }

    async fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy {
            provider: self.id.clone(),
            latency_ms: 50,
        }
    }
}

#[tokio::test]
async fn test_circuit_states_returns_snapshot_for_all_providers() {
    // Arrange: Create router with 3 providers
    let provider1 = Arc::new(FakeProvider::new("provider-1", vec!["gpt-4"]));
    let provider2 = Arc::new(FakeProvider::new("provider-2", vec!["gpt-3.5"]));
    let provider3 = Arc::new(FakeProvider::new("provider-3", vec!["claude-3"]));

    let providers: Vec<Arc<dyn ProviderPort>> = vec![provider1, provider2, provider3];
    let router = FallbackRouter::new(providers, RoutingStrategy::Priority);

    // Trigger failures on provider-2 to open the circuit
    let request = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("gpt-3.5"),
        messages: vec![rook_core::Message {
            role: Role::User,
            content: MessageContent::Text("test".to_string()),
        }],
        temperature: None,
        max_tokens: None,
        stream: false,
        tool_choice: None,
        tools: None,
        metadata: RequestMetadata {
            origin: "test".to_string(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions::default(),
    };

    // Trigger 3 failures to open circuit for provider-2
    for _ in 0..3 {
        let provider = router.select(&request).await.ok();
        if let Some(p) = provider {
            let _ = p.complete(&request).await;
            router
                .on_failure(p.id(), &CortexError::provider("test"))
                .await;
        }
    }

    // Act: Get circuit states
    let states = router.circuit_states();

    // Assert: Should return entries for ALL 3 providers (not just failed ones)
    assert_eq!(states.len(), 3, "Should have entries for all 3 providers");

    // Build a map for easy lookup
    let states_map: std::collections::HashMap<&str, &rook_core::CircuitStateSnapshot> =
        states.iter().map(|(id, s)| (id.as_str(), s)).collect();

    // Assert provider-1: default state (never failed)
    let p1 = states_map
        .get("provider-1")
        .expect("provider-1 should be present");
    assert_eq!(p1.failures, 0, "provider-1 should have 0 failures");
    assert!(!p1.is_open, "provider-1 should not be open");
    assert!(
        p1.last_failure.is_none(),
        "provider-1 should have no last_failure"
    );
    assert!(
        p1.cooldown_until.is_none(),
        "provider-1 should have no cooldown_until"
    );

    // Assert provider-2: failed 3 times, circuit is open
    let p2 = states_map
        .get("provider-2")
        .expect("provider-2 should be present");
    assert_eq!(p2.failures, 3, "provider-2 should have 3 failures");
    assert!(p2.is_open, "provider-2 should be open");
    assert!(
        p2.last_failure.is_some(),
        "provider-2 should have last_failure"
    );
    assert!(
        p2.cooldown_until.is_some(),
        "provider-2 should have cooldown_until"
    );

    // Assert provider-3: default state (never failed)
    let p3 = states_map
        .get("provider-3")
        .expect("provider-3 should be present");
    assert_eq!(p3.failures, 0, "provider-3 should have 0 failures");
    assert!(!p3.is_open, "provider-3 should not be open");
    assert!(
        p3.last_failure.is_none(),
        "provider-3 should have no last_failure"
    );
    assert!(
        p3.cooldown_until.is_none(),
        "provider-3 should have no cooldown_until"
    );
}

#[test]
fn test_circuit_states_empty_router() {
    // Arrange: Create empty router
    let router = FallbackRouter::new_empty(RoutingStrategy::Priority);

    // Act: Get circuit states
    let states = router.circuit_states();

    // Assert: Should be empty
    assert_eq!(states.len(), 0);
}
