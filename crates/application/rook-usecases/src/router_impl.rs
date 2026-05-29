// FallbackRouter — implements RouterPort with configurable routing strategy
//
// Routing strategies:
//   - Priority: use highest-priority available provider
//   - RoundRobin: rotate through available providers
//   - WeightedRandom: probabilistic selection by weight
//   - ModelBased: selects by model ID prefix/category
//
// Circuit breaker: providers that fail are temporarily removed from the pool.
// Recovery is automatic after a cool-down period.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use rook_core::{
    CompletionRequest, ModelId, ProviderId, ProviderPort, ProviderRegistryPort, RouterPort,
};
use shared_kernel::{NuxaError, NuxaResult, Utc};
use tokio::sync::RwLock;

/// Number of failures before opening the circuit
const FAILURE_THRESHOLD: u32 = 3;
/// Duration to keep circuit open before attempting recovery
const CIRCUIT_COOLDOWN: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub enum RoutingStrategy {
    Priority,
    RoundRobin,
    WeightedRandom(Vec<f32>),
    ModelBased,
}

/// Circuit breaker state per provider
#[derive(Debug, Clone, Default)]
struct CircuitState {
    failures: u32,
    is_open: bool,
    last_failure: Option<chrono::DateTime<Utc>>,
    cooldown_until: Option<Instant>,
}

impl CircuitState {
    fn record_failure(&mut self) {
        self.failures += 1;
        self.last_failure = Some(Utc::now());
        if self.failures >= FAILURE_THRESHOLD {
            self.is_open = true;
            self.cooldown_until = Some(Instant::now() + CIRCUIT_COOLDOWN);
        }
    }

    #[allow(dead_code)]
    fn record_success(&mut self) {
        self.failures = 0;
        self.is_open = false;
        self.cooldown_until = None;
    }

    fn is_open(&self) -> bool {
        if !self.is_open {
            return false;
        }
        // Check if cool-down has elapsed
        if let Some(until) = self.cooldown_until {
            if Instant::now() >= until {
                return false;
            }
        }
        true
    }
}

/// FallbackRouter — routes requests to providers with fallback support
pub struct FallbackRouter {
    providers: Vec<Arc<dyn ProviderPort>>,
    strategy: RoutingStrategy,
    circuits: DashMap<ProviderId, CircuitState>,
    round_robin_index: RwLock<usize>,
}

impl FallbackRouter {
    pub fn new(providers: Vec<Arc<dyn ProviderPort>>, strategy: RoutingStrategy) -> Self {
        Self {
            providers,
            strategy,
            circuits: DashMap::new(),
            round_robin_index: RwLock::new(0),
        }
    }

    /// Returns providers that are available (circuit closed) and support the model
    fn available_providers<'a>(&'a self, model: &ModelId) -> Vec<&'a Arc<dyn ProviderPort>> {
        self.providers
            .iter()
            .filter(|p| {
                let id = p.id();
                let circuit = self.circuits.get(id).map(|s| s.clone()).unwrap_or_default();
                !circuit.is_open() && p.supports_model(model)
            })
            .collect()
    }
}

impl ProviderRegistryPort for FallbackRouter {
    fn providers(&self) -> Vec<ProviderId> {
        self.providers.iter().map(|p| p.id().clone()).collect()
    }

    fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
        self.providers
            .iter()
            .find(|provider| provider.id() == id)
            .cloned()
    }
}

#[async_trait]
impl RouterPort for FallbackRouter {
    async fn select(&self, req: &CompletionRequest) -> NuxaResult<Arc<dyn ProviderPort>> {
        let candidates: Vec<_> = self.available_providers(&req.model).into_iter().collect();

        if candidates.is_empty() {
            return Err(NuxaError::all_providers_exhausted());
        }

        let selected = match &self.strategy {
            RoutingStrategy::Priority => {
                // Return first candidate (assumes sorted by priority)
                candidates.first().cloned()
            }
            RoutingStrategy::RoundRobin => {
                let mut index = self.round_robin_index.write().await;
                let idx = *index % candidates.len();
                *index = idx + 1;
                Some(candidates[idx])
            }
            RoutingStrategy::WeightedRandom(weights) => {
                if weights.len() != candidates.len() {
                    // Fall back to first if weights don't match
                    candidates.first().cloned()
                } else {
                    use rand::Rng;
                    let mut rng = rand::thread_rng();
                    let total: f32 = weights.iter().sum();
                    let r = rng.gen::<f32>() * total;
                    let mut sum = 0.0_f32;
                    candidates
                        .iter()
                        .zip(weights.iter())
                        .find(|(_, &w)| {
                            sum += w;
                            sum >= r
                        })
                        .map(|(p, _)| p)
                        .cloned()
                }
            }
            RoutingStrategy::ModelBased => {
                // TODO: implement model-based routing (e.g., "anthropic/" → claude providers)
                candidates.first().cloned()
            }
        };

        selected
            .ok_or_else(NuxaError::all_providers_exhausted)
            .cloned()
    }

    async fn on_failure(&self, provider: &ProviderId, _error: &NuxaError) {
        let mut state = self.circuits.entry(provider.clone()).or_default();
        state.record_failure();
        tracing::warn!(
            provider = %provider,
            failures = state.failures,
            is_open = state.is_open,
            "provider circuit breaker updated"
        );
    }

    fn providers(&self) -> Vec<ProviderId> {
        self.providers.iter().map(|p| p.id().clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::BoxStream;
    use rook_core::{
        CompletionRequest, CompletionResponse, HealthStatus, Message, ModelId, ProviderId,
        ProviderPort, Role, StreamChunk, TokenUsage,
    };

    struct StubProvider {
        id: ProviderId,
        models: Vec<ModelId>,
    }

    impl StubProvider {
        fn new(id: &str, models: &[&str]) -> Arc<Self> {
            Arc::new(Self {
                id: ProviderId::new(id),
                models: models.iter().map(|model| ModelId::new(*model)).collect(),
            })
        }
    }

    #[async_trait]
    impl ProviderPort for StubProvider {
        fn id(&self) -> &ProviderId {
            &self.id
        }

        fn supported_models(&self) -> &[ModelId] {
            &self.models
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

        async fn complete(&self, req: &CompletionRequest) -> NuxaResult<CompletionResponse> {
            Ok(CompletionResponse {
                id: req.id.clone(),
                provider: self.id.clone(),
                model: req.model.clone(),
                content: "ok".to_string(),
                usage: TokenUsage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                    estimated_cost_usd: None,
                },
                latency_ms: 1,
            })
        }

        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> NuxaResult<BoxStream<'_, NuxaResult<StreamChunk>>> {
            Err(NuxaError::provider("streaming not supported"))
        }
    }

    fn request(model: &str) -> CompletionRequest {
        CompletionRequest {
            id: shared_kernel::RequestId::new(),
            model: ModelId::new(model),
            messages: vec![Message {
                role: Role::User,
                content: "hello".to_string(),
            }],
            stream: false,
            max_tokens: None,
            temperature: None,
            metadata: rook_core::RequestMetadata {
                origin: "test".to_string(),
                cacheable: true,
                priority: 1,
            },
        }
    }

    // -------------------------------------------------------------------------
    // New tests below
    // -------------------------------------------------------------------------

    /// Provider that records how many times it was selected.
    struct CountingProvider {
        id: ProviderId,
        models: Vec<ModelId>,
        select_count: std::sync::Arc<std::sync::Mutex<usize>>,
    }

    impl CountingProvider {
        fn new(id: &str, models: &[&str]) -> Arc<Self> {
            Arc::new(Self {
                id: ProviderId::new(id),
                models: models.iter().map(|m| ModelId::new(*m)).collect(),
                select_count: Arc::new(std::sync::Mutex::new(0)),
            })
        }
    }

    struct CountingProviderWrapper {
        inner: Arc<CountingProvider>,
    }

    impl CountingProviderWrapper {
        fn new(id: &str, models: &[&str]) -> Arc<Self> {
            Arc::new(Self {
                inner: CountingProvider::new(id, models),
            })
        }
    }

    #[async_trait]
    impl ProviderPort for CountingProviderWrapper {
        fn id(&self) -> &ProviderId {
            &self.inner.id
        }
        fn supported_models(&self) -> &[ModelId] {
            &self.inner.models
        }
        fn is_available(&self) -> bool {
            true
        }
        async fn health_check(&self) -> HealthStatus {
            HealthStatus::Healthy {
                provider: self.inner.id.clone(),
                latency_ms: 1,
            }
        }
        async fn complete(&self, req: &CompletionRequest) -> NuxaResult<CompletionResponse> {
            *self.inner.select_count.lock().unwrap() += 1;
            Ok(CompletionResponse {
                id: req.id.clone(),
                provider: self.inner.id.clone(),
                model: req.model.clone(),
                content: format!("provider-{}", self.inner.id.as_str()),
                usage: TokenUsage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                    estimated_cost_usd: None,
                },
                latency_ms: 1,
            })
        }
        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> NuxaResult<BoxStream<'_, NuxaResult<StreamChunk>>> {
            Err(NuxaError::provider("not supported"))
        }
    }

    #[test]
    fn fallback_router_new_stores_providers_and_strategy() {
        let p1 = StubProvider::new("a", &["model-a"]);
        let router = FallbackRouter::new(vec![p1.clone()], RoutingStrategy::Priority);
        assert_eq!(
            <FallbackRouter as ProviderRegistryPort>::providers(&router),
            vec![ProviderId::new("a")]
        );
    }

    #[test]
    fn provider_registry_get_returns_provider_by_id() {
        let p1 = StubProvider::new("a", &["model-a"]);
        let p2 = StubProvider::new("b", &["model-b"]);
        let router =
            FallbackRouter::new(vec![p1.clone(), p2.clone()], RoutingStrategy::Priority);

        let found = router.get(&ProviderId::new("a"));
        assert!(found.is_some());
        assert_eq!(found.unwrap().id(), &ProviderId::new("a"));

        let not_found = router.get(&ProviderId::new("nonexistent"));
        assert!(not_found.is_none());
    }

    #[test]
    fn provider_registry_providers_lists_all_registered_ids() {
        let p1 = StubProvider::new("a", &["model-a"]);
        let p2 = StubProvider::new("b", &["model-b"]);
        let router = FallbackRouter::new(vec![p1, p2], RoutingStrategy::Priority);
        let ids = <FallbackRouter as ProviderRegistryPort>::providers(&router);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&ProviderId::new("a")));
        assert!(ids.contains(&ProviderId::new("b")));
    }

    #[test]
    fn select_with_priority_strategy_returns_first_available() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let p1 = StubProvider::new("first", &["model-x"]);
                let p2 = StubProvider::new("second", &["model-x"]);
                let router =
                    FallbackRouter::new(vec![p1.clone(), p2.clone()], RoutingStrategy::Priority);

                let selected = router
                    .select(&request("model-x"))
                    .await
                    .expect("should select");
                assert_eq!(selected.id(), &ProviderId::new("first"));
            });
    }

    #[test]
    fn select_returns_error_when_no_providers_available() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let router = FallbackRouter::new(vec![], RoutingStrategy::Priority);
                let result = router.select(&request("any-model")).await;
                assert!(result.is_err());
                match result {
                    Ok(_) => panic!("expected error"),
                    Err(e) => assert!(e.is_all_providers_exhausted()),
                }
            });
    }

    #[test]
    fn select_returns_error_when_no_provider_supports_model() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let p = StubProvider::new("a", &["only-this-model"]);
                let router = FallbackRouter::new(vec![p], RoutingStrategy::Priority);
                let result = router.select(&request("different-model")).await;
                assert!(result.is_err());
                match result {
                    Ok(_) => panic!("expected error"),
                    Err(e) => assert!(e.is_all_providers_exhausted()),
                }
            });
    }

    #[test]
    fn select_round_robin_rotates_across_providers() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let p1 = CountingProviderWrapper::new("a", &["model-x"]);
                let p2 = CountingProviderWrapper::new("b", &["model-x"]);
                let router = FallbackRouter::new(
                    vec![p1.clone() as Arc<dyn ProviderPort>, p2.clone() as Arc<dyn ProviderPort>],
                    RoutingStrategy::RoundRobin,
                );

                // Round-robin: a, b, a, b
                let ids: Vec<_> = futures::future::join_all([
                    router.select(&request("model-x")),
                    router.select(&request("model-x")),
                    router.select(&request("model-x")),
                    router.select(&request("model-x")),
                ])
                .await
                .into_iter()
                .map(|r| r.expect("select ok").id().clone())
                .collect();

                assert_eq!(ids[0], ProviderId::new("a"));
                assert_eq!(ids[1], ProviderId::new("b"));
                assert_eq!(ids[2], ProviderId::new("a"));
                assert_eq!(ids[3], ProviderId::new("b"));
            });
    }

    #[test]
    fn on_failure_records_failure_count() {
        let p = StubProvider::new("failing", &["model-x"]);
        let router = FallbackRouter::new(vec![p], RoutingStrategy::Priority);

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");

        // Record 2 failures — circuit should NOT open yet
        rt.block_on(async {
            router.on_failure(&ProviderId::new("failing"), &NuxaError::provider("err1")).await;
        });
        rt.block_on(async {
            router.on_failure(&ProviderId::new("failing"), &NuxaError::provider("err2")).await;
        });

        // 3rd failure — circuit opens
        rt.block_on(async {
            router.on_failure(&ProviderId::new("failing"), &NuxaError::provider("err3")).await;
        });

        // Now provider should be unavailable (circuit open)
        let result = rt.block_on(async { router.select(&request("model-x")).await });

        assert!(result.is_err());
        match result {
            Ok(_) => panic!("expected error"),
            Err(e) => assert!(e.is_all_providers_exhausted()),
        }
    }

    #[test]
    fn circuit_breaker_opens_after_threshold_and_blocks_provider() {
        let p = StubProvider::new("recoverable", &["model-x"]);
        let router = FallbackRouter::new(vec![p.clone()], RoutingStrategy::Priority);

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");

        // Open the circuit with 3 failures
        rt.block_on(async {
            for _ in 0..3 {
                let _ = router.on_failure(&ProviderId::new("recoverable"), &NuxaError::provider("boom")).await;
            }
        });

        // Verify circuit is open
        let result = rt.block_on(async { router.select(&request("model-x")).await });

        assert!(result.is_err());
        match result {
            Ok(_) => panic!("expected error"),
            Err(e) => assert!(e.is_all_providers_exhausted()),
        }
    }

    #[test]
    fn model_based_strategy_falls_back_to_first_candidate() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let p1 = StubProvider::new("anthropic", &["claude-3"]);
                let p2 = StubProvider::new("openai", &["gpt-4"]);
                let router = FallbackRouter::new(
                    vec![p1.clone(), p2.clone()],
                    RoutingStrategy::ModelBased,
                );

                // ModelBased currently falls back to first — verify it selects without error
                let selected = router
                    .select(&request("claude-3"))
                    .await
                    .expect("should select");
                assert_eq!(selected.id(), &ProviderId::new("anthropic"));
            });
    }

    #[test]
    fn weighted_random_strategy_falls_back_to_first_when_weights_mismatch() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let p1 = StubProvider::new("a", &["model-x"]);
                let p2 = StubProvider::new("b", &["model-x"]);
                let router = FallbackRouter::new(
                    vec![p1.clone(), p2.clone()],
                    // 3 weights but only 2 providers — should fall back to first
                    RoutingStrategy::WeightedRandom(vec![0.5, 0.3, 0.2]),
                );

                let selected = router
                    .select(&request("model-x"))
                    .await
                    .expect("should select");
                assert_eq!(selected.id(), &ProviderId::new("a"));
            });
    }

    #[test]
    fn routing_strategy_clone_works() {
        // Verify RoutingStrategy derives Clone
        let s1 = RoutingStrategy::Priority;
        let s2 = s1.clone();
        assert!(matches!(s2, RoutingStrategy::Priority));

        let s3 = RoutingStrategy::WeightedRandom(vec![0.5, 0.5]);
        let s4 = s3.clone();
        assert!(matches!(s4, RoutingStrategy::WeightedRandom(_)));
    }
}