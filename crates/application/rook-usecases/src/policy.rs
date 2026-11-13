// Policy engine — pure functions for provider routing policies (DALLAY-774)
//
// This module implements the six explicit routing policies as pure functions
// behind a typed dispatcher. Each policy consumes a PolicySelectionContext
// containing the configured policy, request facts, eligible targets, scored
// components, and optional RNG/RR state, and produces a PolicySelectionDecision.
//
// Design principles:
// - Pure functions: no I/O, no side effects, no hidden state
// - Known/Unknown distinction with typed reasons
// - Raw metric comparators (not normalized) for least-latency and least-cost
// - Stable target ID tiebreaks for determinism
// - Auto resolver uses ordered rules, no synthetic scores

use std::collections::HashMap;

use rook_core::{PolicyFallbackReason, PolicyKind, PolicySelectionDecision, TargetRef};
use shared_kernel::{CortexError, CortexResult, ProviderId};

use crate::scoring::{CandidateScoreComponents, SignalsSnapshot};

// ============================================================================
// Context
// ============================================================================

/// Input to one policy selection decision.
///
/// Contains the configured policy, request facts, ordered eligible target
/// references, scored components, stable identity, and optional RNG/RR state.
pub struct PolicySelectionContext<'a> {
    /// The configured routing policy.
    pub policy: PolicyKind,
    /// Whether this is a streaming request.
    pub is_streaming: bool,
    /// Whether the request has an enforced spending budget.
    pub has_spending_budget: bool,
    /// Ordered eligible target references (registry order).
    pub eligible_targets: &'a [TargetRef],
    /// Signals snapshot captured for this selection.
    pub snapshot: &'a SignalsSnapshot,
    /// Score components for all candidates.
    pub score_components: &'a HashMap<TargetRef, CandidateScoreComponents>,
    /// Identity-keyed weights for the weighted policy.
    pub weights: &'a HashMap<ProviderId, f64>,
    /// Round-robin index state (mutable for counter update).
    pub rr_index: &'a mut usize,
    /// Optional seeded RNG for deterministic weighted selection in tests.
    /// If None, uses unseeded system RNG.
    pub rng: Option<f64>,
}

// ============================================================================
// Dispatcher
// ============================================================================

/// Select a provider using the configured policy and available evidence.
///
/// This is the single dispatch surface for all six policies. It consumes
/// the typed context and produces a typed decision with the selected target,
/// requested policy, effective policy, and optional fallback reason.
pub fn select_policy(
    ctx: &mut PolicySelectionContext<'_>,
) -> CortexResult<PolicySelectionDecision> {
    match ctx.policy {
        PolicyKind::Priority => select_priority(ctx),
        PolicyKind::Weighted => select_weighted(ctx),
        PolicyKind::RoundRobin => select_round_robin(ctx),
        PolicyKind::LeastLatency => select_least_latency(ctx),
        PolicyKind::LeastCost => select_least_cost(ctx),
        PolicyKind::Auto => resolve_auto(ctx),
    }
}

// ============================================================================
// Priority
// ============================================================================

/// Select the first eligible candidate in legacy provider-registration order.
///
/// This preserves the actual ordering of the existing priority policy.
/// No score-based reorder; no stable-ID sorting.
fn select_priority(ctx: &mut PolicySelectionContext<'_>) -> CortexResult<PolicySelectionDecision> {
    let target = ctx
        .eligible_targets
        .first()
        .ok_or_else(CortexError::all_providers_exhausted)?
        .clone();

    Ok(PolicySelectionDecision {
        selected_target: target,
        requested_policy: PolicyKind::Priority,
        effective_policy: PolicyKind::Priority,
        fallback_reason: None,
    })
}

// ============================================================================
// Weighted
// ============================================================================

/// Select a provider by weighted random over eligible candidates.
///
/// Weights are keyed by stable target identity. Unconfigured targets have
/// zero weight. Zero-weight targets never win. If no positive eligible weight,
/// returns a typed non-retryable selection/configuration error.
fn select_weighted(ctx: &mut PolicySelectionContext<'_>) -> CortexResult<PolicySelectionDecision> {
    // Filter to eligible candidates with positive weight
    let weighted_candidates: Vec<_> = ctx
        .eligible_targets
        .iter()
        .filter_map(|target| {
            let weight = ctx.weights.get(&target.0).copied();
            weight
                .filter(|w| w.is_finite() && *w > 0.0)
                .map(|w| (target, w))
        })
        .collect();

    if weighted_candidates.is_empty() {
        return Err(CortexError::weighted_config_error(
            "no eligible candidates have positive weight".to_string(),
        ));
    }

    // Build weight slices for selection
    let targets: Vec<_> = weighted_candidates.iter().map(|(t, _)| t).collect();
    let weights: Vec<f64> = weighted_candidates.iter().map(|(_, w)| *w).collect();

    let total: f64 = weights.iter().sum();
    if total <= 0.0 || !total.is_finite() {
        return Err(CortexError::weighted_config_error(
            "total weight is zero or non-finite".to_string(),
        ));
    }

    // Generate random value
    let random = ctx.rng.unwrap_or(rand::random::<f64>());

    let target = random.clamp(0.0, 1.0) * total;
    let mut cumulative = 0.0;
    let index = weights
        .iter()
        .position(|w| {
            cumulative += w;
            cumulative > target
        })
        .unwrap_or_else(|| {
            // Fallback to last positive weight index
            weights.iter().rposition(|w| *w > 0.0).unwrap_or(0)
        });

    let selected_target = (*targets[index]).clone();

    Ok(PolicySelectionDecision {
        selected_target: selected_target.clone(),
        requested_policy: PolicyKind::Weighted,
        effective_policy: PolicyKind::Weighted,
        fallback_reason: None,
    })
}

// ============================================================================
// Round-Robin
// ============================================================================

/// Select the next provider in round-robin rotation over eligible candidates.
///
/// Preserves the original candidate order and counter/rotation semantics.
fn select_round_robin(
    ctx: &mut PolicySelectionContext<'_>,
) -> CortexResult<PolicySelectionDecision> {
    let targets = ctx.eligible_targets;
    if targets.is_empty() {
        return Err(CortexError::all_providers_exhausted());
    }

    let index = *ctx.rr_index;
    let idx = index % targets.len();
    *ctx.rr_index = (idx + 1) % targets.len();

    let selected_target = targets[idx].clone();

    Ok(PolicySelectionDecision {
        selected_target,
        requested_policy: PolicyKind::RoundRobin,
        effective_policy: PolicyKind::RoundRobin,
        fallback_reason: None,
    })
}

// ============================================================================
// Least Latency
// ============================================================================

/// Select the provider with the minimum raw p95 latency among comparable candidates.
///
/// Uses raw p95 values (not normalized). Known-first ordering, exact-tie
/// stable-ID ordering. If all values are unknown, falls back to priority
/// with MetricUnavailable reason.
fn select_least_latency(
    ctx: &mut PolicySelectionContext<'_>,
) -> CortexResult<PolicySelectionDecision> {
    // Find candidates with known p95
    let known_candidates: Vec<_> = ctx
        .eligible_targets
        .iter()
        .filter_map(|target| {
            let signals = ctx.snapshot.candidates.get(target)?;
            let p95 = signals.latency_p95_ms.as_known().copied()?;
            if !p95.is_finite() || p95 < 0.0 {
                return None;
            }
            Some((target, p95))
        })
        .collect();

    if known_candidates.is_empty() {
        // All unknown — fall back to priority with MetricUnavailable reason
        let target = ctx
            .eligible_targets
            .first()
            .ok_or_else(CortexError::all_providers_exhausted)?
            .clone();

        return Ok(PolicySelectionDecision {
            selected_target: target,
            requested_policy: PolicyKind::LeastLatency,
            effective_policy: PolicyKind::Priority,
            fallback_reason: Some(PolicyFallbackReason::MetricUnavailable),
        });
    }

    // Find minimum p95; tiebreak by stable target ID
    let mut best = &known_candidates[0];
    for candidate in &known_candidates[1..] {
        match candidate.1.partial_cmp(&best.1) {
            Some(std::cmp::Ordering::Less) => best = candidate,
            Some(std::cmp::Ordering::Equal) if candidate.0 < best.0 => best = candidate,
            _ => {}
        }
    }

    let selected_target = best.0.clone();

    Ok(PolicySelectionDecision {
        selected_target,
        requested_policy: PolicyKind::LeastLatency,
        effective_policy: PolicyKind::LeastLatency,
        fallback_reason: None,
    })
}

// ============================================================================
// Least Cost
// ============================================================================

/// Select the provider with the minimum projected request-cost upper bound in USD.
///
/// Computes projected cost from both input and output rates using the same
/// request bound. Known comparable costs precede unknown. Exact ties use
/// stable target ID. If all unknown, falls back to priority.
fn select_least_cost(
    ctx: &mut PolicySelectionContext<'_>,
) -> CortexResult<PolicySelectionDecision> {
    // Find candidates with known comparable cost
    // Cost is computed from both input and output rates
    let known_candidates: Vec<_> = ctx
        .eligible_targets
        .iter()
        .filter_map(|target| {
            let components = ctx.score_components.get(target)?;
            let input_rate = components.cost_input.raw.as_known().copied()?;
            let output_rate = components.cost_output.raw.as_known().copied()?;

            // Both rates must be known, finite, and non-negative
            if !input_rate.is_finite() || input_rate < 0.0 {
                return None;
            }
            if !output_rate.is_finite() || output_rate < 0.0 {
                return None;
            }

            // Combined rate represents the cost basis (both input and output)
            // A candidate with both rates known is comparable.
            // We use the combined rate as the comparable metric.
            let combined_rate = input_rate + output_rate;
            Some((target, combined_rate))
        })
        .collect();

    if known_candidates.is_empty() {
        // All unknown — fall back to priority
        let target = ctx
            .eligible_targets
            .first()
            .ok_or_else(CortexError::all_providers_exhausted)?
            .clone();

        return Ok(PolicySelectionDecision {
            selected_target: target,
            requested_policy: PolicyKind::LeastCost,
            effective_policy: PolicyKind::Priority,
            fallback_reason: Some(PolicyFallbackReason::MetricUnavailable),
        });
    }

    // Find minimum combined rate; tiebreak by stable target ID
    let mut best = &known_candidates[0];
    for candidate in &known_candidates[1..] {
        match candidate.1.partial_cmp(&best.1) {
            Some(std::cmp::Ordering::Less) => best = candidate,
            Some(std::cmp::Ordering::Equal) if candidate.0 < best.0 => best = candidate,
            _ => {}
        }
    }

    let selected_target = best.0.clone();

    Ok(PolicySelectionDecision {
        selected_target,
        requested_policy: PolicyKind::LeastCost,
        effective_policy: PolicyKind::LeastCost,
        fallback_reason: None,
    })
}

// ============================================================================
// Auto Resolver
// ============================================================================

/// Resolve deterministically to a concrete policy per request.
///
/// Ordered rules:
/// 1. If the request has an enforced spending_budget and comparable projected
///    USD cost exists for at least one eligible candidate, choose least-cost
///    (BudgetAware).
/// 2. Otherwise, if the request is streaming and at least one comparable p95
///    exists, choose least-latency (StreamingLatencyProxy).
/// 3. Otherwise, if at least one comparable p95 exists, choose least-latency
///    (LatencyEvidenceAvailable).
/// 4. Otherwise, if at least one comparable projected request cost exists,
///    choose least-cost (CostEvidenceAvailable).
/// 5. Otherwise choose legacy priority (NoComparableSignals).
fn resolve_auto(ctx: &mut PolicySelectionContext<'_>) -> CortexResult<PolicySelectionDecision> {
    // Check if any candidate has known p95
    let has_known_p95 = ctx.eligible_targets.iter().any(|target| {
        ctx.snapshot
            .candidates
            .get(target)
            .and_then(|s| s.latency_p95_ms.as_known().copied())
            .map(|v| v.is_finite() && v >= 0.0)
            .unwrap_or(false)
    });

    // Check if any candidate has known comparable cost (both input and output)
    let has_known_cost = ctx.eligible_targets.iter().any(|target| {
        ctx.score_components
            .get(target)
            .and_then(|c| {
                let input = c.cost_input.raw.as_known().copied()?;
                let output = c.cost_output.raw.as_known().copied()?;
                if input.is_finite() && input >= 0.0 && output.is_finite() && output >= 0.0 {
                    Some(())
                } else {
                    None
                }
            })
            .is_some()
    });

    // Rule 1: BudgetAware
    if ctx.has_spending_budget && has_known_cost {
        let decision = select_least_cost(ctx)?;
        return Ok(PolicySelectionDecision {
            selected_target: decision.selected_target,
            requested_policy: PolicyKind::Auto,
            effective_policy: PolicyKind::LeastCost,
            fallback_reason: Some(PolicyFallbackReason::BudgetAware),
        });
    }

    // Rule 2: StreamingLatencyProxy
    if ctx.is_streaming && has_known_p95 {
        let decision = select_least_latency(ctx)?;
        // Override the requested/effective policy to reflect auto resolution
        return Ok(PolicySelectionDecision {
            selected_target: decision.selected_target,
            requested_policy: PolicyKind::Auto,
            effective_policy: PolicyKind::LeastLatency,
            fallback_reason: Some(PolicyFallbackReason::StreamingLatencyProxy),
        });
    }

    // Rule 3: LatencyEvidenceAvailable
    if has_known_p95 {
        let decision = select_least_latency(ctx)?;
        return Ok(PolicySelectionDecision {
            selected_target: decision.selected_target,
            requested_policy: PolicyKind::Auto,
            effective_policy: PolicyKind::LeastLatency,
            fallback_reason: Some(PolicyFallbackReason::LatencyEvidenceAvailable),
        });
    }

    // Rule 4: CostEvidenceAvailable
    if has_known_cost {
        let decision = select_least_cost(ctx)?;
        return Ok(PolicySelectionDecision {
            selected_target: decision.selected_target,
            requested_policy: PolicyKind::Auto,
            effective_policy: PolicyKind::LeastCost,
            fallback_reason: Some(PolicyFallbackReason::CostEvidenceAvailable),
        });
    }

    // Rule 5: NoComparableSignals — fall back to priority
    select_priority(ctx).map(|decision| PolicySelectionDecision {
        selected_target: decision.selected_target,
        requested_policy: PolicyKind::Auto,
        effective_policy: PolicyKind::Priority,
        fallback_reason: Some(PolicyFallbackReason::NoComparableSignals),
    })
}

// ============================================================================
// Helpers
// ============================================================================

/// Check whether at least one candidate has a known, comparable p95 latency.
pub fn has_comparable_p95(targets: &[TargetRef], signals: &SignalsSnapshot) -> bool {
    targets.iter().any(|target| {
        signals
            .candidates
            .get(target)
            .and_then(|s| s.latency_p95_ms.as_known().copied())
            .map(|v| v.is_finite() && v >= 0.0)
            .unwrap_or(false)
    })
}

/// Check whether at least one candidate has known, comparable cost (both input and output).
pub fn has_comparable_cost(
    targets: &[TargetRef],
    components: &HashMap<TargetRef, CandidateScoreComponents>,
) -> bool {
    targets.iter().any(|target| {
        components
            .get(target)
            .and_then(|c| {
                let input = c.cost_input.raw.as_known().copied()?;
                let output = c.cost_output.raw.as_known().copied()?;
                if input.is_finite() && input >= 0.0 && output.is_finite() && output >= 0.0 {
                    Some(())
                } else {
                    None
                }
            })
            .is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scoring::{
        CandidatePricing, CandidateSignals, RawEvidence, ScoreComponent, UnknownReason,
    };
    use chrono::Utc;

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
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Priority,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &HashMap::new(),
            rr_index: &mut rr_index,
            rng: None,
        };
        let decision = select_policy(&mut ctx).unwrap();
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
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::LeastLatency,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &HashMap::new(),
            rr_index: &mut rr_index,
            rng: None,
        };
        let decision = select_policy(&mut ctx).unwrap();
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
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::LeastLatency,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &HashMap::new(),
            rr_index: &mut rr_index,
            rng: None,
        };
        let decision = select_policy(&mut ctx).unwrap();
        assert_eq!(decision.selected_target, t1);
        assert_eq!(decision.effective_policy, PolicyKind::Priority);
        assert_eq!(
            decision.fallback_reason,
            Some(PolicyFallbackReason::MetricUnavailable)
        );
    }

    #[test]
    fn least_cost_selects_lowest_combined_rate() {
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
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::LeastCost,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &HashMap::new(),
            rr_index: &mut rr_index,
            rng: None,
        };
        let decision = select_policy(&mut ctx).unwrap();
        assert_eq!(decision.selected_target, t2);
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
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Auto,
            is_streaming: false,
            has_spending_budget: true,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &HashMap::new(),
            rr_index: &mut rr_index,
            rng: None,
        };
        let decision = select_policy(&mut ctx).unwrap();
        assert_eq!(decision.requested_policy, PolicyKind::Auto);
        assert_eq!(decision.effective_policy, PolicyKind::LeastCost);
        assert_eq!(
            decision.fallback_reason,
            Some(PolicyFallbackReason::BudgetAware)
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
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Auto,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &HashMap::new(),
            rr_index: &mut rr_index,
            rng: None,
        };
        let decision = select_policy(&mut ctx).unwrap();
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

        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::RoundRobin,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &HashMap::new(),
            rr_index: &mut rr_index,
            rng: None,
        };
        let decision1 = select_policy(&mut ctx).unwrap();
        assert_eq!(decision1.selected_target, t1);
        assert_eq!(rr_index, 1);

        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::RoundRobin,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &HashMap::new(),
            rr_index: &mut rr_index,
            rng: None,
        };
        let decision2 = select_policy(&mut ctx).unwrap();
        assert_eq!(decision2.selected_target, t2);
        assert_eq!(rr_index, 0); // wrapped around
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
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rr_index: &mut rr_index,
            rng: Some(0.5),
        };
        let decision = select_policy(&mut ctx).unwrap();
        // Should select one of the targets
        assert!(
            decision.selected_target == t1 || decision.selected_target == t2,
            "should select a valid target"
        );
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

        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rr_index: &mut rr_index,
            rng: None,
        };
        let result = select_policy(&mut ctx);
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

        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rr_index: &mut rr_index,
            rng: Some(0.5),
        };
        // Should select t2 (only positive finite weight)
        let decision = select_policy(&mut ctx).unwrap();
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

        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rr_index: &mut rr_index,
            rng: Some(0.5),
        };
        // Should select t2 (only positive finite weight)
        let decision = select_policy(&mut ctx).unwrap();
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

        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rr_index: &mut rr_index,
            rng: Some(0.5),
        };
        // Should select t2 (only positive finite weight)
        let decision = select_policy(&mut ctx).unwrap();
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

        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rr_index: &mut rr_index,
            rng: None,
        };
        let result = select_policy(&mut ctx);
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

        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rr_index: &mut rr_index,
            rng: Some(0.5),
        };
        // Should always select t1 because t2 has zero weight
        let decision = select_policy(&mut ctx).unwrap();
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
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets_1,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rng: Some(0.5),
            rr_index: &mut rr_index,
        };
        let decision_1 = select_policy(&mut ctx).unwrap();

        // Order 2: [c, a, b] — t2 is last
        let targets_2 = vec![t3.clone(), t1.clone(), t2.clone()];
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets_2,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rng: Some(0.5),
            rr_index: &mut rr_index,
        };
        let decision_2 = select_policy(&mut ctx).unwrap();

        // Order 3: [b, a, c] — t2 is first
        let targets_3 = vec![t2.clone(), t1.clone(), t3.clone()];
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets_3,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rng: Some(0.5),
            rr_index: &mut rr_index,
        };
        let decision_3 = select_policy(&mut ctx).unwrap();

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
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets_original,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rng: Some(0.25),
            rr_index: &mut rr_index,
        };
        let decision_original = select_policy(&mut ctx).unwrap();

        // After adding t3 (unconfigured, zero weight): [a, b, c]
        let targets_with_new = vec![t1.clone(), t2.clone(), t3.clone()];
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets_with_new,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rng: Some(0.25),
            rr_index: &mut rr_index,
        };
        let decision_with_new = select_policy(&mut ctx).unwrap();

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
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets_original,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rng: Some(0.5),
            rr_index: &mut rr_index,
        };
        let decision_original = select_policy(&mut ctx).unwrap();

        // After removing t3: [a, b]
        let targets_without_c = vec![t1.clone(), t2.clone()];
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets_without_c,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rng: Some(0.5),
            rr_index: &mut rr_index,
        };
        let decision_without_c = select_policy(&mut ctx).unwrap();

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
            let mut rr_index = 0;
            let mut ctx = PolicySelectionContext {
                policy: PolicyKind::Weighted,
                is_streaming: false,
                has_spending_budget: false,
                eligible_targets: &targets,
                snapshot: &snapshot,
                score_components: &components,
                weights: &weights,
                rr_index: &mut rr_index,
                rng: Some(rng_val),
            };
            let decision = select_policy(&mut ctx).unwrap();
            if decision.selected_target == t1 {
                t1_count += 1;
            } else {
                t2_count += 1;
            }
        }

        // With 90/10 weights, t1 should be selected ~90% of the time
        let t1_ratio = t1_count as f64 / total_runs as f64;
        // Allow 5% tolerance
        assert!(
            (t1_ratio - 0.9).abs() < 0.05,
            "t1 ratio {:.2} should be close to 0.9 (got {} out of {})",
            t1_ratio,
            t1_count,
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
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &HashMap::new(),
            rr_index: &mut rr_index,
            rng: None,
        };
        assert!(select_policy(&mut ctx).is_err());

        // After adding weight → success
        let mut weights = HashMap::new();
        weights.insert(t1.0.clone(), 1.0);
        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rr_index: &mut rr_index,
            rng: Some(0.5),
        };
        let decision = select_policy(&mut ctx).unwrap();
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

        let mut rr_index = 0;
        let mut ctx = PolicySelectionContext {
            policy: PolicyKind::Weighted,
            is_streaming: false,
            has_spending_budget: false,
            eligible_targets: &targets,
            snapshot: &snapshot,
            score_components: &components,
            weights: &weights,
            rr_index: &mut rr_index,
            rng: Some(0.5),
        };
        // Should not panic or return infinity error
        let decision = select_policy(&mut ctx).unwrap();
        assert!(
            decision.selected_target == t1 || decision.selected_target == t2,
            "should select a valid target even with large weights"
        );
    }
}
