use std::collections::HashMap;

use chrono::Utc;
use rook_core::{PolicyFallbackReason, PolicyKind, TargetRef};
use rook_usecases::policy::{select_policy, PolicySelectionContext};
use rook_usecases::scoring::{
    CandidatePricing, CandidateScoreComponents, CandidateSignals, RawEvidence, ScoreComponent,
    SignalsSnapshot, UnknownReason,
};
use shared_kernel::ProviderId;

fn make_target(id: &str) -> TargetRef {
    TargetRef(ProviderId::new(id))
}

fn make_signals(
    p95: Option<f64>,
    input_price: Option<f64>,
    output_price: Option<f64>,
) -> CandidateSignals {
    CandidateSignals {
        latency_p50_ms: RawEvidence::Unknown(UnknownReason::Missing),
        latency_p95_ms: p95
            .map(RawEvidence::Known)
            .unwrap_or_else(|| RawEvidence::Unknown(UnknownReason::Missing)),
        pricing: if input_price.is_some() && output_price.is_some() {
            Some(CandidatePricing {
                input_per_million: input_price
                    .map(RawEvidence::Known)
                    .unwrap_or_else(|| RawEvidence::Unknown(UnknownReason::Missing)),
                output_per_million: output_price
                    .map(RawEvidence::Known)
                    .unwrap_or_else(|| RawEvidence::Unknown(UnknownReason::Missing)),
            })
        } else {
            None
        },
        priority: RawEvidence::Known(1),
        quota: None,
        circuit: None,
    }
}

fn make_components(
    p95: Option<f64>,
    input_price: Option<f64>,
    output_price: Option<f64>,
) -> CandidateScoreComponents {
    CandidateScoreComponents {
        latency_p50: ScoreComponent {
            raw: RawEvidence::Unknown(UnknownReason::Missing),
            normalized: None,
            unit: "ms".to_string(),
        },
        latency_p95: ScoreComponent {
            raw: p95
                .map(RawEvidence::Known)
                .unwrap_or_else(|| RawEvidence::Unknown(UnknownReason::Missing)),
            normalized: p95.map(|_| 0.5),
            unit: "ms".to_string(),
        },
        cost_input: ScoreComponent {
            raw: input_price
                .map(RawEvidence::Known)
                .unwrap_or_else(|| RawEvidence::Unknown(UnknownReason::Missing)),
            normalized: input_price.map(|_| 0.5),
            unit: "usd_per_million_input".to_string(),
        },
        cost_output: ScoreComponent {
            raw: output_price
                .map(RawEvidence::Known)
                .unwrap_or_else(|| RawEvidence::Unknown(UnknownReason::Missing)),
            normalized: output_price.map(|_| 0.5),
            unit: "usd_per_million_output".to_string(),
        },
        priority: RawEvidence::Known(1),
        quota: None,
        circuit: None,
    }
}

fn make_snapshot(signals: Vec<(&str, CandidateSignals)>) -> SignalsSnapshot {
    SignalsSnapshot {
        captured_at: Utc::now(),
        candidates: signals
            .into_iter()
            .map(|(id, s)| (make_target(id), s))
            .collect(),
    }
}

#[test]
fn priority_selects_first_target() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
    ]);
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Priority,
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
    };
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.selected_target, t1);
    assert_eq!(decision.effective_policy, PolicyKind::Priority);
}

#[test]
fn least_latency_selects_lowest_p95() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(Some(100.0), None, None)),
        ("b", make_signals(Some(50.0), None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(Some(100.0), None, None)),
        (t2.clone(), make_components(Some(50.0), None, None)),
    ]);
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::LeastLatency,
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
    };
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.selected_target, t2);
    assert_eq!(decision.effective_policy, PolicyKind::LeastLatency);
}

#[test]
fn least_latency_falls_back_to_priority_when_all_unknown() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
    ]);
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::LeastLatency,
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
    };
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.selected_target, t1);
    assert_eq!(decision.effective_policy, PolicyKind::Priority);
    assert_eq!(
        decision.fallback_reason,
        Some(PolicyFallbackReason::MetricUnavailable)
    );
}

#[test]
fn least_cost_empty_eligible_targets_returns_all_providers_exhausted() {
    let targets: Vec<TargetRef> = Vec::new();
    let snapshot = make_snapshot(Vec::new());
    let components = HashMap::new();
    let ctx = PolicySelectionContext {
        policy: PolicyKind::LeastCost,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &HashMap::new(),
        rr_index: 0,
        input_token_bound: Some(10_000),
        output_token_bound: 100,
        rng: None,
    };

    let result = select_policy(&ctx);
    assert!(result.is_err());
    assert!(result.unwrap_err().is_all_providers_exhausted());
}

#[test]
fn least_cost_mixed_pricing_falls_back_to_priority() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, Some(1.0), Some(1.0))),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, Some(1.0), Some(1.0))),
        (t2.clone(), make_components(None, None, None)),
    ]);
    let ctx = PolicySelectionContext {
        policy: PolicyKind::LeastCost,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &HashMap::new(),
        rr_index: 0,
        input_token_bound: Some(10_000),
        output_token_bound: 100,
        rng: None,
    };

    let decision = select_policy(&ctx).unwrap();

    assert_eq!(decision.selected_target, t1);
    assert_eq!(decision.effective_policy, PolicyKind::Priority);
    assert_eq!(
        decision.fallback_reason,
        Some(PolicyFallbackReason::MetricUnavailable)
    );
}

#[test]
fn least_cost_selects_using_request_weighted_cost() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, Some(1.0), Some(100.0))),
        ("b", make_signals(None, Some(10.0), Some(1.0))),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, Some(1.0), Some(100.0))),
        (t2.clone(), make_components(None, Some(10.0), Some(1.0))),
    ]);
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::LeastCost,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &HashMap::new(),
        rr_index,
        input_token_bound: Some(10_000),
        output_token_bound: 1,
        rng: None,
    };
    let decision = select_policy(&ctx).unwrap();
    // For a large prompt and a one-token completion, provider a is cheaper;
    // summing per-token rates incorrectly selects provider b.
    assert_eq!(decision.selected_target, t1);
    assert_eq!(decision.effective_policy, PolicyKind::LeastCost);
}

#[test]
fn auto_resolves_to_least_cost_when_budget_and_cost_known() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, Some(10.0), Some(20.0))),
        ("b", make_signals(None, Some(5.0), Some(5.0))),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, Some(10.0), Some(20.0))),
        (t2.clone(), make_components(None, Some(5.0), Some(5.0))),
    ]);
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Auto,
        is_streaming: false,
        has_spending_budget: true,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &HashMap::new(),
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 10,
        rng: None,
    };
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.requested_policy, PolicyKind::Auto);
    assert_eq!(decision.effective_policy, PolicyKind::LeastCost);
    assert_eq!(
        decision.fallback_reason,
        Some(PolicyFallbackReason::BudgetAware)
    );
}

#[test]
fn auto_does_not_select_least_cost_without_output_bound() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, Some(1.0), Some(100.0))),
        ("b", make_signals(None, Some(100.0), Some(1.0))),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, Some(1.0), Some(100.0))),
        (t2, make_components(None, Some(100.0), Some(1.0))),
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
        input_token_bound: Some(100),
        output_token_bound: 0,
        rng: None,
    };

    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.selected_target, t1);
    assert_eq!(decision.effective_policy, PolicyKind::Priority);
    assert_eq!(
        decision.fallback_reason,
        Some(PolicyFallbackReason::NoComparableSignals)
    );
}

#[test]
fn auto_resolves_to_priority_when_no_signals() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
    ]);
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Auto,
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
    };
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.requested_policy, PolicyKind::Auto);
    assert_eq!(decision.effective_policy, PolicyKind::Priority);
    assert_eq!(
        decision.fallback_reason,
        Some(PolicyFallbackReason::NoComparableSignals)
    );
}

#[test]
fn round_robin_rotates_through_targets() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
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
    let decision1 = select_at(0);
    let decision2 = select_at(1);
    assert_eq!(decision1.selected_target, t1);
    assert_eq!(decision2.selected_target, t2);
}

#[test]
fn weighted_selects_by_weight() {
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
    ]);

    let mut weights = HashMap::new();
    weights.insert(t1.0.clone(), 0.9);
    weights.insert(t2.0.clone(), 0.1);

    // Use seeded RNG for deterministic test
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: Some(0.5),
    };
    let decision = select_policy(&ctx).unwrap();
    // rng = 0.5 over total weight 1.0 selects the 0.9-weight target.
    assert_eq!(decision.selected_target, t1);
}

// ============================================================================
// Slice 2: Weighted registry hardening tests
// ============================================================================

#[test]
fn weighted_errors_when_no_positive_weight() {
    let t1 = make_target("a");
    let targets = vec![t1.clone()];
    let snapshot = make_snapshot(vec![("a", make_signals(None, None, None))]);
    let components = HashMap::from([(t1.clone(), make_components(None, None, None))]);

    let mut weights = HashMap::new();
    weights.insert(t1.0.clone(), 0.0); // zero weight

    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: None,
    };
    let result = select_policy(&ctx);
    assert!(result.is_err());
}

#[test]
fn weighted_filters_nan_weight() {
    // NaN weights must be filtered out, not cause panic
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
    ]);

    let mut weights = HashMap::new();
    weights.insert(t1.0.clone(), f64::NAN);
    weights.insert(t2.0.clone(), 1.0);

    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: Some(0.5),
    };
    // Should select t2 (only positive finite weight)
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.selected_target, t2);
}

#[test]
fn weighted_filters_infinity_weight() {
    // Infinity weights must be filtered out
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
    ]);

    let mut weights = HashMap::new();
    weights.insert(t1.0.clone(), f64::INFINITY);
    weights.insert(t2.0.clone(), 1.0);

    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: Some(0.5),
    };
    // Should select t2 (only positive finite weight)
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.selected_target, t2);
}

#[test]
fn weighted_filters_negative_weight() {
    // Negative weights must be filtered out
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
    ]);

    let mut weights = HashMap::new();
    weights.insert(t1.0.clone(), -5.0);
    weights.insert(t2.0.clone(), 1.0);

    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: Some(0.5),
    };
    // Should select t2 (only positive finite weight)
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.selected_target, t2);
}

#[test]
fn weighted_errors_when_all_weights_invalid() {
    // All NaN/Inf/negative → error
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
    ]);

    let mut weights = HashMap::new();
    weights.insert(t1.0.clone(), f64::NAN);
    weights.insert(t2.0.clone(), f64::INFINITY);

    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: None,
    };
    let result = select_policy(&ctx);
    assert!(result.is_err());
}

#[test]
fn weighted_unconfigured_target_has_zero_weight() {
    // Targets not in the weight map have zero weight and never win
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
    ]);

    // Only t1 has a weight; t2 is unconfigured
    let mut weights = HashMap::new();
    weights.insert(t1.0.clone(), 1.0);

    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: Some(0.5),
    };
    // Should always select t1 because t2 has zero weight
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.selected_target, t1);
}

#[test]
fn weighted_stable_across_registry_reorder() {
    // Reordering the registry must not shift one target's weight onto another.
    // We verify this by checking that each target's weight is looked up by identity,
    // not by position. With a single target having all the weight, it must always win
    // regardless of its position in the registry.
    let t1 = make_target("a");
    let t2 = make_target("b");
    let t3 = make_target("c");
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
        ("c", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
        (t3.clone(), make_components(None, None, None)),
    ]);

    // Only t2 has weight — it should always win regardless of position
    let mut weights = HashMap::new();
    weights.insert(t2.0.clone(), 1.0);

    // Order 1: [a, b, c] — t2 is in the middle
    let targets_1 = vec![t1.clone(), t2.clone(), t3.clone()];
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets_1,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rng: Some(0.5),
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
    };
    let decision_1 = select_policy(&ctx).unwrap();

    // Order 2: [c, a, b] — t2 is last
    let targets_2 = vec![t3.clone(), t1.clone(), t2.clone()];
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets_2,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rng: Some(0.5),
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
    };
    let decision_2 = select_policy(&ctx).unwrap();

    // Order 3: [b, a, c] — t2 is first
    let targets_3 = vec![t2.clone(), t1.clone(), t3.clone()];
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets_3,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rng: Some(0.5),
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
    };
    let decision_3 = select_policy(&ctx).unwrap();

    // t2 is the only positive-weight target — it must win in every order
    assert_eq!(
        decision_1.selected_target, t2,
        "t2 must win when it's the only weighted target (order 1)"
    );
    assert_eq!(
        decision_2.selected_target, t2,
        "t2 must win when it's the only weighted target (order 2)"
    );
    assert_eq!(
        decision_3.selected_target, t2,
        "t2 must win when it's the only weighted target (order 3)"
    );
}

#[test]
fn weighted_stable_after_adding_new_target() {
    // Adding a new target must not shift existing weights
    let t1 = make_target("a");
    let t2 = make_target("b");
    let t3 = make_target("c");
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
        ("c", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
        (t3.clone(), make_components(None, None, None)),
    ]);

    // Weights for t1 and t2 only
    let mut weights = HashMap::new();
    weights.insert(t1.0.clone(), 1.0);
    weights.insert(t2.0.clone(), 1.0);

    // Original: [a, b]
    let targets_original = vec![t1.clone(), t2.clone()];
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets_original,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rng: Some(0.25),
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
    };
    let decision_original = select_policy(&ctx).unwrap();

    // After adding t3 (unconfigured, zero weight): [a, b, c]
    let targets_with_new = vec![t1.clone(), t2.clone(), t3.clone()];
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets_with_new,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rng: Some(0.25),
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
    };
    let decision_with_new = select_policy(&ctx).unwrap();

    assert_eq!(
        decision_original.selected_target, decision_with_new.selected_target,
        "adding an unweighted target must not shift existing weights"
    );
}

#[test]
fn weighted_stable_after_removing_target() {
    // Removing a target must not shift its weight onto another
    let t1 = make_target("a");
    let t2 = make_target("b");
    let t3 = make_target("c");
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
        ("c", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
        (t3.clone(), make_components(None, None, None)),
    ]);

    let mut weights = HashMap::new();
    weights.insert(t1.0.clone(), 1.0);
    weights.insert(t2.0.clone(), 1.0);
    weights.insert(t3.0.clone(), 1.0);

    // Original: [a, b, c]
    let targets_original = vec![t1.clone(), t2.clone(), t3.clone()];
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets_original,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rng: Some(0.5),
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
    };
    let decision_original = select_policy(&ctx).unwrap();

    // After removing t3: [a, b]
    let targets_without_c = vec![t1.clone(), t2.clone()];
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets_without_c,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rng: Some(0.5),
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
    };
    let decision_without_c = select_policy(&ctx).unwrap();

    // With rng=0.5 and equal weights, original total=3.0, target=1.5 → picks index 1 (b)
    // After removing c, total=2.0, target=1.0 → picks index 1 (b)
    // Both should pick the same target
    assert_eq!(
        decision_original.selected_target, decision_without_c.selected_target,
        "removing a target must not shift its weight onto another"
    );
}

#[test]
fn weighted_statistical_distribution() {
    // Verify weighted selection distributes proportionally across many runs
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
    ]);

    let mut weights = HashMap::new();
    weights.insert(t1.0.clone(), 0.9);
    weights.insert(t2.0.clone(), 0.1);

    let total_runs = 1000;
    let mut t1_count = 0;
    let mut t2_count = 0;

    for i in 0..total_runs {
        let rng_val = i as f64 / total_runs as f64;
        let rr_index = 0;
        let ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rr_index,
            input_token_bound: Some(0),
            output_token_bound: 0,
            rng: Some(rng_val),
        };
        let decision = select_policy(&ctx).unwrap();
        if decision.selected_target == t1 {
            t1_count += 1;
        } else {
            t2_count += 1;
        }
    }

    // With 90/10 weights, t1 should be selected ~90% of the time
    let t1_ratio = t1_count as f64 / total_runs as f64;
    let t2_ratio = t2_count as f64 / total_runs as f64;
    // Allow 5% tolerance
    assert!(
        (t1_ratio - 0.9).abs() < 0.05,
        "t1 ratio {:.2} should be close to 0.9 (got {} out of {})",
        t1_ratio,
        t1_count,
        total_runs
    );
    assert!(
        (t2_ratio - 0.1).abs() < 0.05,
        "t2 ratio {:.2} should be close to 0.1 (got {} out of {})",
        t2_ratio,
        t2_count,
        total_runs
    );
}

#[test]
fn weighted_empty_to_populated_bootstrap() {
    // Start with empty weights, then add weights — should work after update
    let t1 = make_target("a");
    let targets = vec![t1.clone()];
    let snapshot = make_snapshot(vec![("a", make_signals(None, None, None))]);
    let components = HashMap::from([(t1.clone(), make_components(None, None, None))]);

    // Empty weights → error
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
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
    };
    assert!(select_policy(&ctx).is_err());

    // After adding weight → success
    let mut weights = HashMap::new();
    weights.insert(t1.0.clone(), 1.0);
    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: Some(0.5),
    };
    let decision = select_policy(&ctx).unwrap();
    assert_eq!(decision.selected_target, t1);
}

#[test]
fn weighted_large_weights_no_overflow() {
    // Very large weights should not overflow f64
    let t1 = make_target("a");
    let t2 = make_target("b");
    let targets = vec![t1.clone(), t2.clone()];
    let snapshot = make_snapshot(vec![
        ("a", make_signals(None, None, None)),
        ("b", make_signals(None, None, None)),
    ]);
    let components = HashMap::from([
        (t1.clone(), make_components(None, None, None)),
        (t2.clone(), make_components(None, None, None)),
    ]);

    let mut weights = HashMap::new();
    weights.insert(t1.0.clone(), 1e300);
    weights.insert(t2.0.clone(), 1e300);

    let rr_index = 0;
    let ctx = PolicySelectionContext {
        policy: PolicyKind::Weighted,
        is_streaming: false,
        has_spending_budget: false,
        eligible_targets: &targets,
        snapshot: &snapshot,
        score_components: &components,
        weights: &weights,
        rr_index,
        input_token_bound: Some(0),
        output_token_bound: 0,
        rng: Some(0.5),
    };
    // Should not panic or return infinity error
    let decision = select_policy(&ctx).unwrap();
    // rng = 0.5 over total weight 2e300 lands on the second candidate.
    assert_eq!(decision.selected_target, t2);
}
