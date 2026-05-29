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

#[async_trait]
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
