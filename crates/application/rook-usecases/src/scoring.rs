// Scoring stage — immutable signal snapshots and pure normalization
//
// This module captures one logical signal snapshot per selection invocation,
// computes per-candidate score components with raw evidence and optional
// normalized values, and produces a SelectionReason explaining the decision.
//
// Design principles:
// - Side-effect-free normalization logic
// - Known/Unknown distinction with typed reasons
// - Inverse min-max normalization (higher is better)
// - Independent normalization per metric (p50, p95, cost dimensions)
// - No fabricated scores or global rankings

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use rook_core::{CircuitStateSnapshot, QuotaObservation};
use serde::{Deserialize, Serialize};
use shared_kernel::ProviderId;

use crate::eligibility::TargetRef;

// ============================================================================
// Unknown reasons
// ============================================================================

/// Why a scoring signal is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    /// No data source for this signal (missing pricing entry, no telemetry).
    Missing,
    /// Data source returned invalid values (negative latency, negative price).
    Invalid,
    /// Data source returned non-finite values (NaN, Infinity).
    NonFinite,
    /// Values exist but cannot be compared (different currencies, different bases).
    IncompatibleBasis,
    /// Data source is unavailable (telemetry disabled, transient read failure).
    SourceUnavailable,
}

// ============================================================================
// Raw evidence
// ============================================================================

/// A raw evidence value that is either known or unknown with a reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RawEvidence<T> {
    Known(T),
    Unknown(UnknownReason),
}

impl<T> RawEvidence<T> {
    pub fn is_known(&self) -> bool {
        matches!(self, RawEvidence::Known(_))
    }

    pub fn as_known(&self) -> Option<&T> {
        match self {
            RawEvidence::Known(v) => Some(v),
            RawEvidence::Unknown(_) => None,
        }
    }
}

// ============================================================================
// Score components
// ============================================================================

/// A single scoring dimension with raw evidence and optional normalized value.
///
/// The normalized value is in [0.0, 1.0] where higher is better.
/// It may be None if the raw value is unknown or non-comparable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreComponent {
    /// Raw evidence from the source (known value or unknown reason).
    pub raw: RawEvidence<f64>,
    /// Normalized value in [0.0, 1.0], higher is better.
    /// None if unknown or non-comparable.
    pub normalized: Option<f64>,
    /// Unit or basis for this component (e.g. "ms", "usd_per_million_input").
    pub unit: String,
}

/// All score components for one candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateScoreComponents {
    /// Latency p50 in milliseconds.
    pub latency_p50: ScoreComponent,
    /// Latency p95 in milliseconds.
    pub latency_p95: ScoreComponent,
    /// Input cost per million tokens in USD.
    pub cost_input: ScoreComponent,
    /// Output cost per million tokens in USD.
    pub cost_output: ScoreComponent,
    /// Provider priority (raw u8 value, lower number = higher priority).
    pub priority: RawEvidence<u8>,
    /// Quota state observation. None when no quota data is available.
    pub quota: Option<QuotaObservation>,
    /// Circuit breaker state. None when no circuit data is available.
    pub circuit: Option<CircuitStateSnapshot>,
}

// ============================================================================
// Candidate signals
// ============================================================================

/// Signals captured for one candidate at one point in time.
#[derive(Debug, Clone)]
pub struct CandidateSignals {
    /// Provider-scoped latency p50 in milliseconds.
    pub latency_p50_ms: RawEvidence<f64>,
    /// Provider-scoped latency p95 in milliseconds.
    pub latency_p95_ms: RawEvidence<f64>,
    /// Pricing dimensions (per-million token prices in USD).
    pub pricing: Option<CandidatePricing>,
    /// Provider priority from the registry. Unknown when no connection is resolved.
    pub priority: RawEvidence<u8>,
    /// Quota observation for this candidate. None when unavailable.
    pub quota: Option<QuotaObservation>,
    /// Circuit breaker state for this candidate. None when unavailable.
    pub circuit: Option<CircuitStateSnapshot>,
}

/// Pricing dimensions for one candidate.
#[derive(Debug, Clone)]
pub struct CandidatePricing {
    /// Input cost per million tokens in USD.
    pub input_per_million: RawEvidence<f64>,
    /// Output cost per million tokens in USD.
    pub output_per_million: RawEvidence<f64>,
}

// ============================================================================
// Signals snapshot
// ============================================================================

/// One immutable signal snapshot for one selection invocation.
///
/// Contains signals for all eligible candidates, captured at a single point
/// in time. The snapshot is keyed by stable target identity.
#[derive(Debug, Clone)]
pub struct SignalsSnapshot {
    /// When this snapshot was captured.
    pub captured_at: DateTime<Utc>,
    /// Signals for each eligible candidate.
    pub candidates: HashMap<TargetRef, CandidateSignals>,
}

// ============================================================================
// Selection reason
// ============================================================================

/// Method used to select the winner (existing strategies only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMethod {
    Priority,
    RoundRobin,
    WeightedRandom,
}

/// Fallback reason if the selection was not a normal choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionFallback {
    /// Only one candidate remained after filtering.
    SingleCandidate,
    /// Weighted selection fell back due to configuration error.
    WeightedConfigError,
}

/// Explanation of one selection decision.
///
/// Describes the actual selected target, the method used, and the evidence
/// considered. Does not contain a fabricated total score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionReason {
    /// The selected target.
    pub selected_target: TargetRef,
    /// The selection method used.
    pub method: SelectionMethod,
    /// Score components for the selected candidate.
    pub components: CandidateScoreComponents,
    /// Fallback reason if applicable.
    pub fallback: Option<SelectionFallback>,
}

/// Result of one selection decision, combining the chosen provider with its explanation.
///
/// This type makes SelectionReason a first-class output of the selection pipeline,
/// not an afterthought. It is used internally in the failover loop and can be
/// accessed by the HTTP layer for future telemetry (DALLAY-779).
#[derive(Clone)]
pub struct SelectionResult {
    /// The selected provider.
    pub provider: Arc<dyn rook_core::ProviderPort>,
    /// Explanation of why this provider was selected.
    pub reason: SelectionReason,
}

impl std::fmt::Debug for SelectionResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectionResult")
            .field("provider_id", &self.provider.id())
            .field("reason", &self.reason)
            .finish()
    }
}

impl SelectionResult {
    pub fn new(provider: Arc<dyn rook_core::ProviderPort>, reason: SelectionReason) -> Self {
        Self { provider, reason }
    }
}

// ============================================================================
// Pure normalization
// ============================================================================

/// Normalize a single component using inverse min-max normalization.
///
/// For comparable known values, `(max - value) / (max - min)`.
/// If all known values are equal (including a single known value), returns 0.5.
/// If no known values exist, returns None.
/// Unknowns never enter min/max bounds.
/// Invalid (negative, NaN, Inf) values are treated as Unknown.
fn normalize_component(value: RawEvidence<f64>, known_values: &[f64]) -> Option<f64> {
    // Validate the value itself: must be known, non-negative, and finite.
    let value = match value {
        RawEvidence::Known(v) => v,
        RawEvidence::Unknown(_) => return None,
    };
    if value.is_nan() || value.is_infinite() || value < 0.0 {
        return None;
    }

    // Filter known_values to only valid (non-negative, finite) values.
    let valid_known: Vec<f64> = known_values
        .iter()
        .copied()
        .filter(|&v| v.is_finite() && v >= 0.0)
        .collect();

    // No valid known values means no bounds.
    if valid_known.is_empty() {
        return None;
    }

    // Single known value or all equal values → neutral 0.5.
    let min = valid_known.iter().copied().fold(f64::INFINITY, f64::min);
    let max = valid_known
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    if (max - min).abs() < f64::EPSILON {
        return Some(0.5);
    }

    // Inverse min-max: (max - value) / (max - min)
    // Higher value → lower normalized score (for latency/cost, lower is better)
    Some((max - value) / (max - min))
}

/// Build score components for all candidates in the snapshot.
///
/// This is a pure function: it reads the snapshot and produces components
/// without side effects. Normalization is independent per metric.
pub fn build_score_components(
    snapshot: &SignalsSnapshot,
) -> HashMap<TargetRef, CandidateScoreComponents> {
    // Collect all known values for each normalization group.
    let latency_p50_known: Vec<f64> = snapshot
        .candidates
        .values()
        .filter_map(|s| s.latency_p50_ms.as_known().copied())
        .collect();
    let latency_p95_known: Vec<f64> = snapshot
        .candidates
        .values()
        .filter_map(|s| s.latency_p95_ms.as_known().copied())
        .collect();

    // For cost, we normalize input and output independently.
    // Only candidates with compatible pricing (same currency/basis) are pooled.
    // Since all pricing is in USD per million tokens, they are compatible.
    let cost_input_known: Vec<f64> = snapshot
        .candidates
        .values()
        .filter_map(|s| {
            s.pricing
                .as_ref()
                .and_then(|p| p.input_per_million.as_known().copied())
        })
        .collect();
    let cost_output_known: Vec<f64> = snapshot
        .candidates
        .values()
        .filter_map(|s| {
            s.pricing
                .as_ref()
                .and_then(|p| p.output_per_million.as_known().copied())
        })
        .collect();

    // Build components for each candidate.
    let mut components = HashMap::new();
    for (target, signals) in &snapshot.candidates {
        let latency_p50 = ScoreComponent {
            raw: signals.latency_p50_ms.clone(),
            normalized: normalize_component(signals.latency_p50_ms.clone(), &latency_p50_known),
            unit: "ms".to_string(),
        };
        let latency_p95 = ScoreComponent {
            raw: signals.latency_p95_ms.clone(),
            normalized: normalize_component(signals.latency_p95_ms.clone(), &latency_p95_known),
            unit: "ms".to_string(),
        };

        // Cost components: only normalize if pricing exists.
        let (cost_input, cost_output) = if let Some(ref pricing) = signals.pricing {
            let input = ScoreComponent {
                raw: pricing.input_per_million.clone(),
                normalized: normalize_component(
                    pricing.input_per_million.clone(),
                    &cost_input_known,
                ),
                unit: "usd_per_million_input".to_string(),
            };
            let output = ScoreComponent {
                raw: pricing.output_per_million.clone(),
                normalized: normalize_component(
                    pricing.output_per_million.clone(),
                    &cost_output_known,
                ),
                unit: "usd_per_million_output".to_string(),
            };
            (input, output)
        } else {
            // No pricing mapping → unknown.
            let unknown_input = ScoreComponent {
                raw: RawEvidence::Unknown(UnknownReason::Missing),
                normalized: None,
                unit: "usd_per_million_input".to_string(),
            };
            let unknown_output = ScoreComponent {
                raw: RawEvidence::Unknown(UnknownReason::Missing),
                normalized: None,
                unit: "usd_per_million_output".to_string(),
            };
            (unknown_input, unknown_output)
        };

        components.insert(
            target.clone(),
            CandidateScoreComponents {
                latency_p50,
                latency_p95,
                cost_input,
                cost_output,
                priority: signals.priority.clone(),
                quota: signals.quota.clone(),
                circuit: signals.circuit.clone(),
            },
        );
    }

    components
}

/// Build a selection reason from the snapshot, selected target, and method.
pub fn build_selection_reason(
    snapshot: &SignalsSnapshot,
    selected_target: &TargetRef,
    method: SelectionMethod,
) -> Option<SelectionReason> {
    let components = build_score_components(snapshot);
    let components = components.get(selected_target)?.clone();

    Some(SelectionReason {
        selected_target: selected_target.clone(),
        method,
        components,
        fallback: None,
    })
}

// ============================================================================
// Metric comparators
// ============================================================================

/// Compare two score components for ordering purposes.
///
/// Known values with normalized scores sort before unknown values.
/// Within known values, higher normalized score sorts first.
/// When both are unknown, falls back to stable-ID comparison.
fn compare_component(a: &ScoreComponent, b: &ScoreComponent) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (&a.normalized, &b.normalized) {
        (Some(a_norm), Some(b_norm)) => {
            // Both known: higher normalized score wins (reverse ordering).
            b_norm.partial_cmp(a_norm).unwrap_or(Ordering::Equal)
        }
        (Some(_), None) => Ordering::Less, // a is known, b is unknown → a comes first
        (None, Some(_)) => Ordering::Greater, // b is known, a is unknown → b comes first
        (None, None) => Ordering::Equal,   // Both unknown: fall through to ID tiebreak
    }
}

/// Compare two candidates by their score components, using known-first ordering.
///
/// This comparator guarantees:
/// - Known values sort before unknown values (known-first)
/// - Within known values, higher normalized score sorts first
/// - Stable-ID tiebreaker using TargetRef for determinism
/// - Transitive total order within compatible bases
///
/// Returns `Ordering::Less` if `a` should be selected before `b`.
pub fn compare_candidates(
    a_target: &TargetRef,
    a_components: &CandidateScoreComponents,
    b_target: &TargetRef,
    b_components: &CandidateScoreComponents,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    // Compare by latency p50 (known-first, higher normalized wins).
    let latency = compare_component(&a_components.latency_p50, &b_components.latency_p50);
    if latency != Ordering::Equal {
        return latency;
    }

    // Tiebreak by cost input.
    let cost = compare_component(&a_components.cost_input, &b_components.cost_input);
    if cost != Ordering::Equal {
        return cost;
    }

    // Tiebreak by priority (lower u8 value = higher priority).
    match (&a_components.priority, &b_components.priority) {
        (RawEvidence::Known(a_p), RawEvidence::Known(b_p)) => {
            if *a_p != *b_p {
                return a_p.cmp(b_p);
            }
        }
        (RawEvidence::Known(_), RawEvidence::Unknown(_)) => return Ordering::Less,
        (RawEvidence::Unknown(_), RawEvidence::Known(_)) => return Ordering::Greater,
        (RawEvidence::Unknown(_), RawEvidence::Unknown(_)) => {}
    }

    // Final stable-ID tiebreaker: deterministic, transitive.
    a_target.cmp(b_target)
}

/// Check whether all components for a candidate are unknown.
pub fn all_components_unknown(components: &CandidateScoreComponents) -> bool {
    components.latency_p50.normalized.is_none()
        && components.latency_p95.normalized.is_none()
        && components.cost_input.normalized.is_none()
        && components.cost_output.normalized.is_none()
        && matches!(components.priority, RawEvidence::Unknown(_))
}

/// Build a signals snapshot from eligible candidates.
///
/// This function acquires telemetry (provider-scoped p50/p95), pricing (per-dimension),
/// circuit breaker state, and quota observations for each eligible candidate.
/// It is the only place that reads these sources; normalization is pure.
pub fn build_signals_snapshot(
    eligible: &[&TargetRef],
    model: &rook_core::ModelId,
    telemetry: &Option<std::sync::Arc<observability::TelemetryTracker>>,
    pricing: &crate::PricingConfig,
    circuit_states: &HashMap<ProviderId, CircuitStateSnapshot>,
    quota_observations: &HashMap<ProviderId, QuotaObservation>,
) -> SignalsSnapshot {
    let mut candidates = HashMap::new();
    for target in eligible {
        let provider_id = &target.0;

        // Latency: provider-scoped p50/p95 from telemetry
        // Validate at the boundary: reject NaN, Inf, and negative values.
        let (latency_p50_ms, latency_p95_ms) = if let Some(tracker) = telemetry {
            match tracker.compute_latency_percentiles(provider_id) {
                Some(stats) => {
                    let p50 = stats.p50 as f64;
                    let p95 = stats.p95 as f64;
                    let p50_valid = p50.is_finite() && p50 >= 0.0;
                    let p95_valid = p95.is_finite() && p95 >= 0.0;
                    (
                        if p50_valid {
                            RawEvidence::Known(p50)
                        } else {
                            RawEvidence::Unknown(UnknownReason::Invalid)
                        },
                        if p95_valid {
                            RawEvidence::Known(p95)
                        } else {
                            RawEvidence::Unknown(UnknownReason::Invalid)
                        },
                    )
                }
                None => (
                    RawEvidence::Unknown(UnknownReason::SourceUnavailable),
                    RawEvidence::Unknown(UnknownReason::SourceUnavailable),
                ),
            }
        } else {
            (
                RawEvidence::Unknown(UnknownReason::SourceUnavailable),
                RawEvidence::Unknown(UnknownReason::SourceUnavailable),
            )
        };

        // Pricing: per-dimension lookup
        let pricing_entry = pricing.get(provider_id, model);
        let pricing = pricing_entry.map(|entry| CandidatePricing {
            input_per_million: if entry.prompt_per_million.is_sign_negative()
                || entry.prompt_per_million.is_nan()
                || entry.prompt_per_million.is_infinite()
            {
                RawEvidence::Unknown(UnknownReason::Invalid)
            } else {
                RawEvidence::Known(entry.prompt_per_million)
            },
            output_per_million: if entry.completion_per_million.is_sign_negative()
                || entry.completion_per_million.is_nan()
                || entry.completion_per_million.is_infinite()
            {
                RawEvidence::Unknown(UnknownReason::Invalid)
            } else {
                RawEvidence::Known(entry.completion_per_million)
            },
        });

        // Circuit breaker state: only include if actually available.
        // Do NOT fabricate a healthy state when data is missing.
        let circuit = circuit_states.get(provider_id).cloned();

        // Quota observation: only include if actually available.
        // Do NOT fabricate a quota record with Utc::now() when data is missing.
        let quota = quota_observations.get(provider_id).cloned();

        candidates.insert(
            (**target).clone(),
            CandidateSignals {
                latency_p50_ms,
                latency_p95_ms,
                pricing,
                priority: RawEvidence::Unknown(UnknownReason::Missing),
                quota,
                circuit,
            },
        );
    }

    SignalsSnapshot {
        captured_at: chrono::Utc::now(),
        candidates,
    }
}

#[cfg(test)]
mod normalization_tests {
    use super::*;

    fn make_snapshot(
        values: Vec<(TargetRef, RawEvidence<f64>, RawEvidence<f64>)>,
    ) -> SignalsSnapshot {
        let mut candidates = HashMap::new();
        for (target, p50, p95) in values {
            candidates.insert(
                target,
                CandidateSignals {
                    latency_p50_ms: p50,
                    latency_p95_ms: p95,
                    pricing: None,
                    priority: RawEvidence::Unknown(UnknownReason::Missing),
                    quota: None,
                    circuit: None,
                },
            );
        }
        SignalsSnapshot {
            captured_at: Utc::now(),
            candidates,
        }
    }

    fn target(id: &str) -> TargetRef {
        TargetRef(ProviderId::new(id.to_string()))
    }

    #[test]
    fn inverse_min_max_normalization() {
        // [10, 20, 30] → [1.0, 0.5, 0.0]
        let snapshot = make_snapshot(vec![
            (
                target("a"),
                RawEvidence::Known(10.0),
                RawEvidence::Known(10.0),
            ),
            (
                target("b"),
                RawEvidence::Known(20.0),
                RawEvidence::Known(20.0),
            ),
            (
                target("c"),
                RawEvidence::Known(30.0),
                RawEvidence::Known(30.0),
            ),
        ]);
        let components = build_score_components(&snapshot);

        let a = components.get(&target("a")).unwrap();
        let b = components.get(&target("b")).unwrap();
        let c = components.get(&target("c")).unwrap();

        assert!((a.latency_p50.normalized.unwrap() - 1.0).abs() < f64::EPSILON);
        assert!((b.latency_p50.normalized.unwrap() - 0.5).abs() < f64::EPSILON);
        assert!((c.latency_p50.normalized.unwrap() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn equal_values_normalize_to_half() {
        // [20, 20, 20] → [0.5, 0.5, 0.5]
        let snapshot = make_snapshot(vec![
            (
                target("a"),
                RawEvidence::Known(20.0),
                RawEvidence::Known(20.0),
            ),
            (
                target("b"),
                RawEvidence::Known(20.0),
                RawEvidence::Known(20.0),
            ),
            (
                target("c"),
                RawEvidence::Known(20.0),
                RawEvidence::Known(20.0),
            ),
        ]);
        let components = build_score_components(&snapshot);

        for comp in components.values() {
            assert!(
                (comp.latency_p50.normalized.unwrap() - 0.5).abs() < f64::EPSILON,
                "equal values should normalize to 0.5"
            );
        }
    }

    #[test]
    fn single_known_value_normalizes_to_half() {
        let snapshot = make_snapshot(vec![(
            target("a"),
            RawEvidence::Known(15.0),
            RawEvidence::Known(15.0),
        )]);
        let components = build_score_components(&snapshot);

        let a = components.get(&target("a")).unwrap();
        assert!(
            (a.latency_p50.normalized.unwrap() - 0.5).abs() < f64::EPSILON,
            "single known value should normalize to 0.5"
        );
    }

    #[test]
    fn no_known_values_stay_unknown() {
        let snapshot = make_snapshot(vec![
            (
                target("a"),
                RawEvidence::Unknown(UnknownReason::Missing),
                RawEvidence::Unknown(UnknownReason::Missing),
            ),
            (
                target("b"),
                RawEvidence::Unknown(UnknownReason::Missing),
                RawEvidence::Unknown(UnknownReason::Missing),
            ),
        ]);
        let components = build_score_components(&snapshot);

        for comp in components.values() {
            assert!(
                comp.latency_p50.normalized.is_none(),
                "no known values should remain unknown"
            );
        }
    }

    #[test]
    fn mixed_known_unknown_unknowns_dont_affect_bounds() {
        // Known: [10, 30], Unknown should not affect bounds
        let snapshot = make_snapshot(vec![
            (
                target("a"),
                RawEvidence::Known(10.0),
                RawEvidence::Known(10.0),
            ),
            (
                target("b"),
                RawEvidence::Unknown(UnknownReason::Missing),
                RawEvidence::Unknown(UnknownReason::Missing),
            ),
            (
                target("c"),
                RawEvidence::Known(30.0),
                RawEvidence::Known(30.0),
            ),
        ]);
        let components = build_score_components(&snapshot);

        let a = components.get(&target("a")).unwrap();
        let b = components.get(&target("b")).unwrap();
        let c = components.get(&target("c")).unwrap();

        // a=10 should normalize to 1.0 (best), c=30 to 0.0 (worst)
        assert!((a.latency_p50.normalized.unwrap() - 1.0).abs() < f64::EPSILON);
        assert!(b.latency_p50.normalized.is_none(), "unknown stays unknown");
        assert!((c.latency_p50.normalized.unwrap() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn known_zero_vs_unknown() {
        // Known(0) enters bounds, unknown doesn't
        let snapshot = make_snapshot(vec![
            (
                target("a"),
                RawEvidence::Known(0.0),
                RawEvidence::Known(0.0),
            ),
            (
                target("b"),
                RawEvidence::Unknown(UnknownReason::Missing),
                RawEvidence::Unknown(UnknownReason::Missing),
            ),
            (
                target("c"),
                RawEvidence::Known(20.0),
                RawEvidence::Known(20.0),
            ),
        ]);
        let components = build_score_components(&snapshot);

        let a = components.get(&target("a")).unwrap();
        let b = components.get(&target("b")).unwrap();
        let c = components.get(&target("c")).unwrap();

        // a=0 should normalize to 1.0 (best), c=20 to 0.0 (worst)
        assert!((a.latency_p50.normalized.unwrap() - 1.0).abs() < f64::EPSILON);
        assert!(b.latency_p50.normalized.is_none(), "unknown stays unknown");
        assert!((c.latency_p50.normalized.unwrap() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn invalid_negative_values_are_unknown() {
        // Negative latency should be treated as unknown
        let snapshot = make_snapshot(vec![
            (
                target("a"),
                RawEvidence::Known(10.0),
                RawEvidence::Known(10.0),
            ),
            (
                target("b"),
                RawEvidence::Unknown(UnknownReason::Invalid),
                RawEvidence::Unknown(UnknownReason::Invalid),
            ),
        ]);
        let components = build_score_components(&snapshot);

        let a = components.get(&target("a")).unwrap();
        let b = components.get(&target("b")).unwrap();

        // a should normalize to 0.5 (single known value)
        assert!(
            (a.latency_p50.normalized.unwrap() - 0.5).abs() < f64::EPSILON,
            "single known value should normalize to 0.5"
        );
        assert!(b.latency_p50.normalized.is_none(), "invalid stays unknown");
    }

    #[test]
    fn p50_p95_normalized_separately() {
        let snapshot = make_snapshot(vec![
            (
                target("a"),
                RawEvidence::Known(10.0),
                RawEvidence::Known(50.0),
            ),
            (
                target("b"),
                RawEvidence::Known(30.0),
                RawEvidence::Known(90.0),
            ),
        ]);
        let components = build_score_components(&snapshot);

        let a = components.get(&target("a")).unwrap();
        let b = components.get(&target("b")).unwrap();

        // p50: [10, 30] → a=1.0, b=0.0
        assert!((a.latency_p50.normalized.unwrap() - 1.0).abs() < f64::EPSILON);
        assert!((b.latency_p50.normalized.unwrap() - 0.0).abs() < f64::EPSILON);

        // p95: [50, 90] → a=1.0, b=0.0
        assert!((a.latency_p95.normalized.unwrap() - 1.0).abs() < f64::EPSILON);
        assert!((b.latency_p95.normalized.unwrap() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn permuted_order_same_normalization() {
        // Same eligible set, permuted order → identical normalization results
        let snapshot1 = make_snapshot(vec![
            (
                target("a"),
                RawEvidence::Known(10.0),
                RawEvidence::Known(10.0),
            ),
            (
                target("b"),
                RawEvidence::Known(20.0),
                RawEvidence::Known(20.0),
            ),
            (
                target("c"),
                RawEvidence::Known(30.0),
                RawEvidence::Known(30.0),
            ),
        ]);
        let snapshot2 = make_snapshot(vec![
            (
                target("c"),
                RawEvidence::Known(30.0),
                RawEvidence::Known(30.0),
            ),
            (
                target("a"),
                RawEvidence::Known(10.0),
                RawEvidence::Known(10.0),
            ),
            (
                target("b"),
                RawEvidence::Known(20.0),
                RawEvidence::Known(20.0),
            ),
        ]);

        let comp1 = build_score_components(&snapshot1);
        let comp2 = build_score_components(&snapshot2);

        // Same target should have same normalization regardless of order
        let a1 = comp1.get(&target("a")).unwrap();
        let a2 = comp2.get(&target("a")).unwrap();
        assert!(
            (a1.latency_p50.normalized.unwrap() - a2.latency_p50.normalized.unwrap()).abs()
                < f64::EPSILON
        );
    }
}
