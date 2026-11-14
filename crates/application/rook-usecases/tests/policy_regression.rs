//! Regression tests for the six-policy routing engine (DALLAY-774 Slice 4).
//!
//! These tests prove that:
//! - Priority returns the same winner, registry order, and failover sequence as pre-change
//! - Round-robin preserves counter/rotation semantics
//! - Completion and streaming invoke the same policy evaluator
//! - SelectionReason carries requested and effective policy

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use futures::{
    stream::{self, BoxStream},
    StreamExt,
};
use rook_core::{
    ApiFormat, ApiKeyRestrictions, CapabilityEvidence, CapabilityState, CompletionRequest,
    CompletionResponse, CortexError, CortexResult, FinishReason, HealthStatus, Message, ModelId,
    PolicyKind, ProviderId, ProviderPort, RequestMetadata, Role, RouterPort, StreamChunk,
    TokenUsage,
};
use rook_usecases::policy::{select_policy, PolicySelectionContext};
use rook_usecases::scoring::{
    build_selection_reason_from_policy, CandidateScoreComponents, CandidateSignals, RawEvidence,
    SignalsSnapshot, UnknownReason,
};
use rook_usecases::{
    router_impl::CircuitClock, FallbackRouter, PricingConfig, RouteRequest, TargetRef,
};
use shared_kernel::RequestId;
use std::sync::Mutex;
use std::time::Instant;

mod common;
use common::{NoOpAliasRepository, NoOpAudit, NoOpCache};

// ============================================================================
// Test fixtures
// ============================================================================

#[derive(Clone, Copy, Debug)]
enum Completion {
    Success,
    Fail,
}

struct FakeProvider {
    id: ProviderId,
    models: Vec<ModelId>,
    completion: Completion,
    calls: AtomicUsize,
}

impl FakeProvider {
    fn new(id: &str, completion: Completion) -> Arc<Self> {
        Arc::new(Self {
            id: ProviderId::new(id),
            models: vec![ModelId::new("model")],
            completion,
            calls: AtomicUsize::new(0),
        })
    }
}

fn usage() -> TokenUsage {
    TokenUsage {
        prompt_tokens: 2,
        completion_tokens: 3,
        total_tokens: 5,
        cache_read_tokens: None,
        cache_creation_tokens: None,
        reasoning_tokens: None,
        estimated_cost_usd: None,
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
    fn model_capabilities(&self, _model: &ModelId) -> rook_core::ModelCapabilities {
        rook_core::ModelCapabilities {
            streaming: CapabilityEvidence {
                state: CapabilityState::Supported,
                source: "test-fixture".into(),
            },
            ..Default::default()
        }
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

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.completion {
            Completion::Success => Ok(CompletionResponse {
                id: req.id.clone(),
                provider: self.id.clone(),
                model: req.model.clone(),
                content: "answer".into(),
                content_blocks: vec!["answer".into()],
                thinking: None,
                tool_calls: vec![],
                finish_reason: Some(FinishReason::Stop),
                usage: usage(),
                latency_ms: 0,
                cache_hit: None,
            }),
            Completion::Fail => Err(CortexError::provider("upstream error")),
        }
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let chunk = StreamChunk {
            id: req.id.clone(),
            model: req.model.clone(),
            delta: "chunk".into(),
            thinking: None,
            tool_calls: vec![],
            finish_reason: Some(FinishReason::Stop),
            usage: Some(usage()),
        };
        match self.completion {
            Completion::Success => Ok(stream::iter([Ok(chunk)]).boxed()),
            Completion::Fail => Err(CortexError::provider("upstream error")),
        }
    }
}

struct ManualClock(Mutex<Instant>);

impl CircuitClock for ManualClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

fn router_with_policy(
    providers: Vec<Arc<dyn ProviderPort>>,
    policy: PolicyKind,
) -> (Arc<FallbackRouter>, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock(Mutex::new(Instant::now())));
    (
        Arc::new(FallbackRouter::with_clock(providers, policy, clock.clone())),
        clock,
    )
}

fn request(stream: bool) -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("model"),
        messages: vec![Message {
            role: Role::User,
            content: "hello".into(),
            tool_calls: vec![],
        }],
        temperature: None,
        max_tokens: None,
        stream,
        tools: None,
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

fn route(router: Arc<FallbackRouter>) -> RouteRequest {
    RouteRequest::new(
        router,
        Arc::new(NoOpCache),
        Arc::new(NoOpAudit),
        None,
        None,
        None,
        Arc::new(PricingConfig::default()),
        Arc::new(NoOpAliasRepository),
        rook_usecases::route_request::ModelAliasesConfig {
            enabled: false,
            auto_seed: false,
        },
        None,
    )
}

// ============================================================================
// Priority golden fixtures
// ============================================================================

/// Priority selects the first provider in registry order, regardless of circuit state
/// (circuit filtering happens at a different stage).
#[tokio::test]
async fn priority_golden_selects_first_in_registry_order() {
    let p1 = FakeProvider::new("p1", Completion::Success);
    let p2 = FakeProvider::new("p2", Completion::Success);
    let p3 = FakeProvider::new("p3", Completion::Success);
    let (router, _clock) = router_with_policy(
        vec![p1.clone(), p2.clone(), p3.clone()],
        PolicyKind::Priority,
    );

    let response = route(router.clone()).execute(request(false)).await.unwrap();
    assert_eq!(response.provider, *p1.id());
    assert_eq!(p1.calls.load(Ordering::SeqCst), 1);
    assert_eq!(p2.calls.load(Ordering::SeqCst), 0);
    assert_eq!(p3.calls.load(Ordering::SeqCst), 0);
}

/// Priority failover: when first provider fails, try next in registry order.
#[tokio::test]
async fn priority_golden_failover_follows_registry_order() {
    let p1 = FakeProvider::new("p1", Completion::Fail);
    let p2 = FakeProvider::new("p2", Completion::Success);
    let p3 = FakeProvider::new("p3", Completion::Success);
    let (router, _clock) = router_with_policy(
        vec![p1.clone(), p2.clone(), p3.clone()],
        PolicyKind::Priority,
    );

    let response = route(router.clone()).execute(request(false)).await.unwrap();
    // p1 fails, so p2 should be selected
    assert_eq!(response.provider, *p2.id());
    // p1 was called and failed
    assert_eq!(p1.calls.load(Ordering::SeqCst), 1);
    // p2 was called and succeeded
    assert!(p2.calls.load(Ordering::SeqCst) >= 1);
    // p3 may or may not have been called depending on failover behavior
}

/// Priority with circuit-open first provider skips to next eligible.
#[tokio::test]
async fn priority_golden_skips_open_circuit() {
    let p1 = FakeProvider::new("p1", Completion::Success);
    let p2 = FakeProvider::new("p2", Completion::Success);
    let (router, _clock) = router_with_policy(vec![p1.clone(), p2.clone()], PolicyKind::Priority);

    // Open circuit on p1
    for _ in 0..3 {
        router
            .on_failure(p1.id(), &CortexError::provider("failure"))
            .await;
    }
    assert!(router
        .circuit_states()
        .iter()
        .any(|(id, s)| id == p1.id() && s.is_open));

    let response = route(router.clone()).execute(request(false)).await.unwrap();
    // p1 circuit is open, so p2 should be selected
    assert_eq!(response.provider, *p2.id());
}

// ============================================================================
// Round-robin regression tests
// ============================================================================

/// Round-robin rotates through providers in order.
/// Note: the RR index is updated by the policy engine per request.
/// The exact rotation order depends on how the RR index persists across requests.
#[tokio::test]
async fn round_robin_rotates_through_providers() {
    let p1 = FakeProvider::new("p1", Completion::Success);
    let p2 = FakeProvider::new("p2", Completion::Success);
    let p3 = FakeProvider::new("p3", Completion::Success);
    let (router, _clock) = router_with_policy(
        vec![p1.clone(), p2.clone(), p3.clone()],
        PolicyKind::RoundRobin,
    );

    let r1 = route(router.clone()).execute(request(false)).await.unwrap();
    let r2 = route(router.clone()).execute(request(false)).await.unwrap();
    let r3 = route(router.clone()).execute(request(false)).await.unwrap();

    // All three providers should be called at least once
    assert!(p1.calls.load(Ordering::SeqCst) >= 1);
    assert!(p2.calls.load(Ordering::SeqCst) >= 1);
    assert!(p3.calls.load(Ordering::SeqCst) >= 1);

    // Responses should come from different providers (rotation)
    let providers: Vec<_> = vec![&r1.provider, &r2.provider, &r3.provider];
    // At least two different providers should be selected
    let unique = providers
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert!(
        unique >= 2,
        "round-robin should select at least 2 different providers"
    );
}

/// Round-robin with exclusions: excluded providers are skipped.
#[tokio::test]
async fn round_robin_skips_excluded() {
    let p1 = FakeProvider::new("p1", Completion::Success);
    let p2 = FakeProvider::new("p2", Completion::Success);
    let p3 = FakeProvider::new("p3", Completion::Success);
    let (router, _clock) = router_with_policy(
        vec![p1.clone(), p2.clone(), p3.clone()],
        PolicyKind::RoundRobin,
    );

    // Open circuit on p2 to exclude it
    for _ in 0..3 {
        router
            .on_failure(p2.id(), &CortexError::provider("failure"))
            .await;
    }

    let r1 = route(router.clone()).execute(request(false)).await.unwrap();
    let r2 = route(router.clone()).execute(request(false)).await.unwrap();

    // Should rotate between p1 and p3, skipping p2
    assert_ne!(r1.provider, *p2.id());
    assert_ne!(r2.provider, *p2.id());
}

// ============================================================================
// Policy engine pure-function tests
// ============================================================================

fn make_target(id: &str) -> TargetRef {
    TargetRef(ProviderId::new(id))
}

fn make_signals(p95: Option<f64>) -> CandidateSignals {
    CandidateSignals {
        latency_p50_ms: RawEvidence::Unknown(UnknownReason::Missing),
        latency_p95_ms: p95
            .map(RawEvidence::Known)
            .unwrap_or_else(|| RawEvidence::Unknown(UnknownReason::Missing)),
        pricing: None,
        priority: RawEvidence::Known(1),
        quota: None,
        circuit: None,
    }
}

fn make_components(p95: Option<f64>) -> CandidateScoreComponents {
    rook_usecases::scoring::CandidateScoreComponents {
        latency_p50: rook_usecases::scoring::ScoreComponent {
            raw: RawEvidence::Unknown(UnknownReason::Missing),
            normalized: None,
            unit: "ms".to_string(),
        },
        latency_p95: rook_usecases::scoring::ScoreComponent {
            raw: p95
                .map(RawEvidence::Known)
                .unwrap_or_else(|| RawEvidence::Unknown(UnknownReason::Missing)),
            normalized: p95.map(|_| 0.5),
            unit: "ms".to_string(),
        },
        cost_input: rook_usecases::scoring::ScoreComponent {
            raw: RawEvidence::Unknown(UnknownReason::Missing),
            normalized: None,
            unit: "usd_per_million_input".to_string(),
        },
        cost_output: rook_usecases::scoring::ScoreComponent {
            raw: RawEvidence::Unknown(UnknownReason::Missing),
            normalized: None,
            unit: "usd_per_million_output".to_string(),
        },
        priority: RawEvidence::Known(1),
        quota: None,
        circuit: None,
    }
}

fn make_snapshot(signals: Vec<(&str, CandidateSignals)>) -> SignalsSnapshot {
    SignalsSnapshot {
        captured_at: chrono::Utc::now(),
        candidates: signals
            .into_iter()
            .map(|(id, s)| (make_target(id), s))
            .collect(),
    }
}

/// Priority policy via pure function selects first target in registry order.
#[test]
fn policy_priority_selects_first_eligible() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![("a", make_signals(None)), ("b", make_signals(None))]);
    let components = HashMap::from([
        (t1.clone(), make_components(None)),
        (t2.clone(), make_components(None)),
    ]);
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Priority,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &HashMap::new(),
        rr_index: 0,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: None,
    };
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.selected_target, t1);
    assert_eq!(decision.requested_policy, PolicyKind::Priority);
    assert_eq!(decision.effective_policy, PolicyKind::Priority);
    assert!(decision.fallback_reason.is_none());
}

/// Round-robin policy via pure function rotates correctly.
#[test]
fn policy_round_robin_rotates() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let t3 = make_target("c");
    let targets = vec![t1.clone(), t2.clone(), t3.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None)),
        ("b", make_signals(None)),
        ("c", make_signals(None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None)),
        (t2.clone(), make_components(None)),
        (t3.clone(), make_components(None)),
    ]);

    let select_at = |rr_index| {
        select_policy(&PolicySelectionContext {
            policy: PolicyKind::RoundRobin,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &HashMap::new(),
            rr_index,
            input_token_bound: Some(0),
            output_token_bound: 0,
            rng: None,
        })
        .unwrap()
    };

    let d1 = select_at(0);
    let d2 = select_at(1);
    let d3 = select_at(2);
    let d4 = select_at(3);

    assert_eq!(d1.selected_target, t1);
    assert_eq!(d2.selected_target, t2);
    assert_eq!(d3.selected_target, t3);
    assert_eq!(d4.selected_target, t1); // wraps
}

/// SelectionReason from policy decision includes requested and effective policy.
#[test]
fn selection_reason_includes_policy_info() {
    let t1 = make_target("a");
    let snapshot = make_snapshot(vec![("a", make_signals(Some(50.0)))]);
    let components = HashMap::from([(t1.clone(), make_components(Some(50.0)))]);

    let decision = rook_core::PolicySelectionDecision {
        selected_target: t1.clone(),
        requested_policy: PolicyKind::Auto,
        effective_policy: PolicyKind::LeastLatency,
        fallback_reason: Some(rook_core::PolicyFallbackReason::LatencyEvidenceAvailable),
    };

    let reason = build_selection_reason_from_policy(&snapshot, &t1, &decision, &components);
    assert!(reason.is_some());
    let reason = reason.unwrap();
    assert_eq!(reason.requested_policy, Some(PolicyKind::Auto));
    assert_eq!(reason.effective_policy, Some(PolicyKind::LeastLatency));
}

// ============================================================================
// Completion + streaming parity
// ============================================================================

/// Both completion and streaming paths use the same policy evaluator.
/// This test verifies that the same provider is selected for both modes
/// under the same conditions.
#[tokio::test]
async fn completion_and_streaming_use_same_policy() {
    let p1 = FakeProvider::new("p1", Completion::Success);
    let p2 = FakeProvider::new("p2", Completion::Success);
    let (router, _clock) = router_with_policy(vec![p1.clone(), p2.clone()], PolicyKind::Priority);

    // Completion request
    let r1 = route(router.clone()).execute(request(false)).await.unwrap();
    // Streaming request must return a stream selected by the same policy.
    let mut stream_request = request(true);
    stream_request.restrictions.allowed_providers = vec![p1.id().clone()];
    let mut stream_output = route(router.clone())
        .execute_stream(stream_request)
        .await
        .expect("stream selection succeeds");

    let chunk = stream_output
        .next()
        .await
        .expect("stream yields at least one chunk")
        .expect("chunk is Ok");
    assert_eq!(chunk.delta, "chunk");

    // Both paths select p1 under Priority policy. The allowlist makes a
    // discarded stream result observable: selecting any other provider fails.
    assert_eq!(r1.provider, *p1.id());
    assert_eq!(p1.calls.load(Ordering::SeqCst), 2);
    assert_eq!(p2.calls.load(Ordering::SeqCst), 0);
}

/// Least-latency policy selects the provider with lowest p95.
#[test]
fn policy_least_latency_selects_lowest_p95() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(Some(100.0))),
        ("b", make_signals(Some(50.0))),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(Some(100.0))),
        (t2.clone(), make_components(Some(50.0))),
    ]);
    let ctx = PolicySelectionContext {
        policy: PolicyKind::LeastLatency,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &HashMap::new(),
        rr_index: 0,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: None,
    };
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.selected_target, t2);
    assert_eq!(decision.effective_policy, PolicyKind::LeastLatency);
}

/// Least-latency falls back to priority when all p95 values are unknown.
#[test]
fn policy_least_latency_fallback_to_priority() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![("a", make_signals(None)), ("b", make_signals(None))]);
    let components = HashMap::from([
        (t1.clone(), make_components(None)),
        (t2.clone(), make_components(None)),
    ]);
    let ctx = PolicySelectionContext {
        policy: PolicyKind::LeastLatency,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &HashMap::new(),
        rr_index: 0,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: None,
    };
    let decision = select_policy(&ctx).unwrap();
    // Falls back to priority — selects first in registry order
    assert_eq!(decision.selected_target, t1);
    assert_eq!(decision.effective_policy, PolicyKind::Priority);
    assert_eq!(
        decision.fallback_reason,
        Some(rook_core::PolicyFallbackReason::MetricUnavailable)
    );
}

/// Auto resolver with no signals falls back to priority.
#[test]
fn policy_auto_no_signals_falls_back_to_priority() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![("a", make_signals(None)), ("b", make_signals(None))]);
    let components = HashMap::from([
        (t1.clone(), make_components(None)),
        (t2.clone(), make_components(None)),
    ]);
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Auto,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &HashMap::new(),
        rr_index: 0,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: None,
    };
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.requested_policy, PolicyKind::Auto);
    assert_eq!(decision.effective_policy, PolicyKind::Priority);
    assert_eq!(
        decision.fallback_reason,
        Some(rook_core::PolicyFallbackReason::NoComparableSignals)
    );
}

/// Auto resolver with known p95 selects least-latency.
#[test]
fn policy_auto_with_p95_selects_least_latency() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(Some(100.0))),
        ("b", make_signals(Some(50.0))),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(Some(100.0))),
        (t2.clone(), make_components(Some(50.0))),
    ]);
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Auto,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &HashMap::new(),
        rr_index: 0,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: None,
    };
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.requested_policy, PolicyKind::Auto);
    assert_eq!(decision.effective_policy, PolicyKind::LeastLatency);
    assert_eq!(
        decision.fallback_reason,
        Some(rook_core::PolicyFallbackReason::LatencyEvidenceAvailable)
    );
    // Should select the lowest-latency target
    assert_eq!(decision.selected_target, t2);
}

#[test]
fn policy_auto_mixed_priced_and_unpriced_candidates_reports_priority_fallback() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![("a", make_signals(None)), ("b", make_signals(None))]);
    let mut priced = make_components(None);
    priced.cost_input.raw = RawEvidence::Known(5.0);
    priced.cost_output.raw = RawEvidence::Known(15.0);
    let components = HashMap::from([(t1.clone(), priced), (t2.clone(), make_components(None))]);
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Auto,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &HashMap::new(),
        rr_index: 0,
        input_token_bound: Some(0),
        output_token_bound: 100,
        rng: None,
    };

    let decision = select_policy(&ctx).unwrap();

    assert_eq!(decision.selected_target, t1);
    assert_eq!(decision.requested_policy, PolicyKind::Auto);
    assert_eq!(decision.effective_policy, PolicyKind::Priority);
    assert_eq!(
        decision.fallback_reason,
        Some(rook_core::PolicyFallbackReason::NoComparableSignals)
    );
}

/// Auto resolver with budget selects least-cost.
#[test]
fn policy_auto_with_budget_selects_least_cost() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![("a", make_signals(None)), ("b", make_signals(None))]);
    // Components with known cost
    let components_with_cost =
        |input: f64, output: f64| -> HashMap<TargetRef, CandidateScoreComponents> {
            HashMap::from([
                (
                    t1.clone(),
                    CandidateScoreComponents {
                        latency_p50: rook_usecases::scoring::ScoreComponent {
                            raw: RawEvidence::Unknown(UnknownReason::Missing),
                            normalized: None,
                            unit: "ms".to_string(),
                        },
                        latency_p95: rook_usecases::scoring::ScoreComponent {
                            raw: RawEvidence::Unknown(UnknownReason::Missing),
                            normalized: None,
                            unit: "ms".to_string(),
                        },
                        cost_input: rook_usecases::scoring::ScoreComponent {
                            raw: RawEvidence::Known(input),
                            normalized: Some(0.5),
                            unit: "usd_per_million_input".to_string(),
                        },
                        cost_output: rook_usecases::scoring::ScoreComponent {
                            raw: RawEvidence::Known(output),
                            normalized: Some(0.5),
                            unit: "usd_per_million_output".to_string(),
                        },
                        priority: RawEvidence::Known(1),
                        quota: None,
                        circuit: None,
                    },
                ),
                (
                    t2.clone(),
                    CandidateScoreComponents {
                        latency_p50: rook_usecases::scoring::ScoreComponent {
                            raw: RawEvidence::Unknown(UnknownReason::Missing),
                            normalized: None,
                            unit: "ms".to_string(),
                        },
                        latency_p95: rook_usecases::scoring::ScoreComponent {
                            raw: RawEvidence::Unknown(UnknownReason::Missing),
                            normalized: None,
                            unit: "ms".to_string(),
                        },
                        cost_input: rook_usecases::scoring::ScoreComponent {
                            raw: RawEvidence::Known(input * 2.0),
                            normalized: Some(0.3),
                            unit: "usd_per_million_input".to_string(),
                        },
                        cost_output: rook_usecases::scoring::ScoreComponent {
                            raw: RawEvidence::Known(output * 2.0),
                            normalized: Some(0.3),
                            unit: "usd_per_million_output".to_string(),
                        },
                        priority: RawEvidence::Known(1),
                        quota: None,
                        circuit: None,
                    },
                ),
            ])
        };
    let components = components_with_cost(5.0, 15.0);
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Auto,
        is_streaming: false,
        has_spending_budget: true,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &HashMap::new(),
        rr_index: 0,
        input_token_bound: Some(0),
        output_token_bound: 100,
        rng: None,
    };
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.requested_policy, PolicyKind::Auto);
    assert_eq!(decision.effective_policy, PolicyKind::LeastCost);
    assert_eq!(
        decision.fallback_reason,
        Some(rook_core::PolicyFallbackReason::BudgetAware)
    );
    // t1 has lower cost
    assert_eq!(decision.selected_target, t1);
}
