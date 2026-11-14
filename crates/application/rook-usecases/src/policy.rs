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
    /// Round-robin index atomically reserved by the router for this selection.
    pub rr_index: usize,
    /// Upper bound for prompt tokens, when request content is measurable.
    /// Structured tool content is unknown rather than represented by a sentinel value.
    pub input_token_bound: Option<u64>,
    /// Declared completion-token bound (`max_tokens`). Zero means unknown.
    pub output_token_bound: u64,
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
pub fn select_policy(ctx: &PolicySelectionContext<'_>) -> CortexResult<PolicySelectionDecision> {
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
fn select_priority(ctx: &PolicySelectionContext<'_>) -> CortexResult<PolicySelectionDecision> {
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
fn select_weighted(ctx: &PolicySelectionContext<'_>) -> CortexResult<PolicySelectionDecision> {
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
fn select_round_robin(ctx: &PolicySelectionContext<'_>) -> CortexResult<PolicySelectionDecision> {
    let targets = ctx.eligible_targets;
    if targets.is_empty() {
        return Err(CortexError::all_providers_exhausted());
    }

    let idx = ctx.rr_index % targets.len();

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
fn select_least_latency(ctx: &PolicySelectionContext<'_>) -> CortexResult<PolicySelectionDecision> {
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

/// Select the provider with the minimum projected request cost in USD.
///
/// Computes input and output cost using their respective request token bounds.
/// Both pricing rates and a declared completion bound must be known. Exact ties
/// use stable target ID. If no comparable projection exists, falls back to priority.
fn select_least_cost(ctx: &PolicySelectionContext<'_>) -> CortexResult<PolicySelectionDecision> {
    if ctx.eligible_targets.is_empty() {
        return Err(CortexError::all_providers_exhausted());
    }

    // Cost comparisons are valid only when every eligible candidate has a
    // comparable projection. Comparing a priced subset could prefer a costly
    // option over an unpriced candidate with a lower true cost.
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

            // Missing token bounds are not comparable: projected cost cannot
            // be calculated without both request bounds.
            let input_token_bound = ctx.input_token_bound?;
            if ctx.output_token_bound == 0 {
                return None;
            }
            let projected_cost = (input_rate * input_token_bound as f64
                + output_rate * ctx.output_token_bound as f64)
                / 1_000_000.0;
            (projected_cost.is_finite()).then_some((target, projected_cost))
        })
        .collect();

    if known_candidates.len() != ctx.eligible_targets.len() {
        // Any unknown candidate makes the set incomparable — fall back to priority
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
    let mut best = known_candidates
        .first()
        .ok_or_else(CortexError::all_providers_exhausted)?;
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
/// 1. If this request has an enforced spending_budget and comparable projected
///    USD cost exists for at least one eligible candidate, choose least-cost
///    (BudgetAware).
/// 2. Otherwise, if the request is streaming and at least one comparable p95
///    exists, choose least-latency (StreamingLatencyProxy).
/// 3. Otherwise, if at least one comparable p95 exists, choose least-latency
///    (LatencyEvidenceAvailable).
/// 4. Otherwise, if at least one comparable projected request cost exists,
///    choose least-cost (CostEvidenceAvailable).
/// 5. Otherwise choose legacy priority (NoComparableSignals).
fn resolve_auto(ctx: &PolicySelectionContext<'_>) -> CortexResult<PolicySelectionDecision> {
    // Check if any candidate has known p95
    let has_known_p95 = ctx.eligible_targets.iter().any(|target| {
        ctx.snapshot
            .candidates
            .get(target)
            .and_then(|s| s.latency_p95_ms.as_known().copied())
            .map(|v| v.is_finite() && v >= 0.0)
            .unwrap_or(false)
    });

    // LeastCost compares every eligible candidate and falls back to Priority
    // when any candidate lacks a comparable projection. Auto must use that same
    // all-candidates comparability test before reporting a cost policy.
    let has_comparable_cost = has_all_comparable_projected_cost(
        ctx.eligible_targets,
        ctx.score_components,
        ctx.input_token_bound,
        ctx.output_token_bound,
    );

    // Rule 1: BudgetAware
    if ctx.has_spending_budget && has_comparable_cost {
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
    if has_comparable_cost {
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
fn has_all_comparable_projected_cost(
    targets: &[TargetRef],
    components: &HashMap<TargetRef, CandidateScoreComponents>,
    input_token_bound: Option<u64>,
    output_token_bound: u64,
) -> bool {
    input_token_bound.is_some()
        && output_token_bound > 0
        && !targets.is_empty()
        && targets.iter().all(|target| {
            has_comparable_projected_cost(
                std::slice::from_ref(target),
                components,
                input_token_bound.expect("checked input bound"),
                output_token_bound,
            )
        })
}

fn has_comparable_projected_cost(
    targets: &[TargetRef],
    components: &HashMap<TargetRef, CandidateScoreComponents>,
    input_token_bound: u64,
    output_token_bound: u64,
) -> bool {
    if output_token_bound == 0 {
        return false;
    }

    targets.iter().any(|target| {
        let Some(components) = components.get(target) else {
            return false;
        };
        let (Some(input), Some(output)) = (
            components.cost_input.raw.as_known().copied(),
            components.cost_output.raw.as_known().copied(),
        ) else {
            return false;
        };
        if !input.is_finite() || input < 0.0 || !output.is_finite() || output < 0.0 {
            return false;
        }
        ((input * input_token_bound as f64 + output * output_token_bound as f64) / 1_000_000.0)
            .is_finite()
    })
}

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

/// Check whether at least one candidate has known rates and a finite projected request cost.
pub fn has_comparable_cost(
    targets: &[TargetRef],
    components: &HashMap<TargetRef, CandidateScoreComponents>,
    input_token_bound: Option<u64>,
    output_token_bound: u64,
) -> bool {
    let Some(input_token_bound) = input_token_bound else {
        return false;
    };
    has_comparable_projected_cost(targets, components, input_token_bound, output_token_bound)
}
