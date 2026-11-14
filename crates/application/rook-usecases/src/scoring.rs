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
use rook_core::{CircuitStateSnapshot, PolicySelectionDecision, QuotaObservation, TargetRef};
use serde::{Deserialize, Serialize};
use shared_kernel::ProviderId;

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

/// Method used to select the winner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMethod {
    Priority,
    RoundRobin,
    WeightedRandom,
    /// Weighted random over eligible candidates by stable identity.
    Weighted,
    /// Minimum raw p95 latency among comparable candidates.
    LeastLatency,
    /// Minimum projected request-cost upper bound in USD.
    LeastCost,
    /// Resolve deterministically to a concrete policy per request.
    Auto,
}

/// Fallback reason if the selection was not a normal choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionFallback {
    /// Only one candidate remained after filtering.
    SingleCandidate,
    /// Weighted selection fell back due to configuration error.
    WeightedConfigError,
    /// All latency values were unknown; fell back to priority.
    MetricUnavailable,
    /// No comparable signals available for auto resolution.
    NoComparableSignals,
    /// Request has spending budget and comparable USD cost exists.
    BudgetAware,
    /// Streaming request with comparable p95 latency available.
    StreamingLatencyProxy,
    /// Comparable p95 latency evidence available.
    LatencyEvidenceAvailable,
    /// Comparable projected cost evidence available.
    CostEvidenceAvailable,
    /// Values exist but cannot be compared (different currencies, different bases).
    IncompatibleBasis,
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
    /// Auto policy resolution explanation; this is not a degradation.
    pub resolution: Option<SelectionFallback>,
    /// The policy requested by the operator (for auto or fallback).
    pub requested_policy: Option<rook_core::PolicyKind>,
    /// The effective concrete policy that selected the target.
    pub effective_policy: Option<rook_core::PolicyKind>,
}

impl SelectionReason {
    /// Stable bounded label for metric dimensions. This is not derived from `Debug`.
    pub fn policy_metric_labels(&self) -> (&'static str, &'static str) {
        (
            policy_metric_label(self.requested_policy),
            policy_metric_label(self.effective_policy),
        )
    }

    /// Stable bounded label for degradation metrics. Resolution explanations
    /// are deliberately excluded from this label.
    pub fn fallback_metric_label(&self) -> &'static str {
        self.fallback
            .as_ref()
            .map(SelectionFallback::metric_label)
            .unwrap_or("none")
    }
}

fn policy_metric_label(policy: Option<rook_core::PolicyKind>) -> &'static str {
    match policy {
        Some(rook_core::PolicyKind::Priority) => "priority",
        Some(rook_core::PolicyKind::Weighted) => "weighted",
        Some(rook_core::PolicyKind::RoundRobin) => "round-robin",
        Some(rook_core::PolicyKind::LeastLatency) => "least-latency",
        Some(rook_core::PolicyKind::LeastCost) => "least-cost",
        Some(rook_core::PolicyKind::Auto) => "auto",
        None => "unknown",
    }
}

impl SelectionFallback {
    fn metric_label(&self) -> &'static str {
        match self {
            Self::SingleCandidate => "single-candidate",
            Self::WeightedConfigError => "weighted-config-error",
            Self::MetricUnavailable => "metric-unavailable",
            Self::NoComparableSignals => "no-comparable-signals",
            Self::BudgetAware => "budget-aware",
            Self::StreamingLatencyProxy => "streaming-latency-proxy",
            Self::LatencyEvidenceAvailable => "latency-evidence-available",
            Self::CostEvidenceAvailable => "cost-evidence-available",
            Self::IncompatibleBasis => "incompatible-basis",
        }
    }
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

/// Pre-computed normalization bounds for a single metric.
struct NormBounds {
    min: f64,
    max: f64,
    /// `true` when all known values are equal (min == max), so every candidate
    /// gets a neutral 0.5 instead of a division-by-zero result.
    all_equal: bool,
}

impl NormBounds {
    fn from_known_values(values: &[f64]) -> Option<Self> {
        // Filter to only valid (non-negative, finite) values.
        let valid: Vec<f64> = values
            .iter()
            .copied()
            .filter(|&v| v.is_finite() && v >= 0.0)
            .collect();
        if valid.is_empty() {
            return None;
        }
        let min = valid.iter().copied().fold(f64::INFINITY, f64::min);
        let max = valid.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let all_equal = min == max;
        Some(NormBounds {
            min,
            max,
            all_equal,
        })
    }

    fn normalize(&self, value: f64) -> Option<f64> {
        if self.all_equal {
            Some(0.5)
        } else {
            Some((self.max - value) / (self.max - self.min))
        }
    }
}

/// Normalize a single component using inverse min-max normalization.
///
/// For comparable known values, `(max - value) / (max - min)`.
/// If all known values are equal (including a single known value), returns 0.5.
/// If no known values exist, returns None.
/// Unknowns never enter min/max bounds.
/// Invalid (negative, NaN, Inf) values are treated as Unknown.
fn normalize_component(value: RawEvidence<f64>, bounds: &NormBounds) -> Option<f64> {
    // Validate the value itself: must be known, non-negative, and finite.
    let value = match value {
        RawEvidence::Known(v) => v,
        RawEvidence::Unknown(_) => return None,
    };
    if value.is_nan() || value.is_infinite() || value < 0.0 {
        return None;
    }
    bounds.normalize(value)
}

/// Build score components for all candidates in the snapshot.
///
/// This is a pure function: it reads the snapshot and produces components
/// without side effects. Normalization is independent per metric.
/// Bounds are computed once per metric (O(N) total), not once per candidate.
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

    // Pre-compute normalization bounds once per metric (O(N) total, not O(N²)).
    let p50_bounds = NormBounds::from_known_values(&latency_p50_known);
    let p95_bounds = NormBounds::from_known_values(&latency_p95_known);
    let cost_input_bounds = NormBounds::from_known_values(&cost_input_known);
    let cost_output_bounds = NormBounds::from_known_values(&cost_output_known);

    // Build components for each candidate.
    let mut components = HashMap::new();
    for (target, signals) in &snapshot.candidates {
        let latency_p50 = ScoreComponent {
            raw: signals.latency_p50_ms.clone(),
            normalized: match &p50_bounds {
                Some(b) => normalize_component(signals.latency_p50_ms.clone(), b),
                None => None,
            },
            unit: "ms".to_string(),
        };
        let latency_p95 = ScoreComponent {
            raw: signals.latency_p95_ms.clone(),
            normalized: match &p95_bounds {
                Some(b) => normalize_component(signals.latency_p95_ms.clone(), b),
                None => None,
            },
            unit: "ms".to_string(),
        };

        // Cost components: only normalize if pricing exists.
        let (cost_input, cost_output) = if let Some(ref pricing) = signals.pricing {
            let input = ScoreComponent {
                raw: pricing.input_per_million.clone(),
                normalized: match &cost_input_bounds {
                    Some(b) => normalize_component(pricing.input_per_million.clone(), b),
                    None => None,
                },
                unit: "usd_per_million_input".to_string(),
            };
            let output = ScoreComponent {
                raw: pricing.output_per_million.clone(),
                normalized: match &cost_output_bounds {
                    Some(b) => normalize_component(pricing.output_per_million.clone(), b),
                    None => None,
                },
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
        fallback: (snapshot.candidates.len() == 1).then_some(SelectionFallback::SingleCandidate),
        resolution: None,
        requested_policy: None,
        effective_policy: None,
    })
}

/// Build an extended selection reason from a policy engine decision.
///
/// Includes requested policy, effective policy, and typed fallback reason.
pub fn build_selection_reason_from_policy(
    _snapshot: &SignalsSnapshot,
    selected_target: &TargetRef,
    decision: &PolicySelectionDecision,
    score_components: &HashMap<TargetRef, CandidateScoreComponents>,
) -> Option<SelectionReason> {
    let components = score_components.get(selected_target)?.clone();

    // Convert policy kind to selection method
    let method = match decision.effective_policy {
        rook_core::PolicyKind::Priority => SelectionMethod::Priority,
        rook_core::PolicyKind::Weighted => SelectionMethod::Weighted,
        rook_core::PolicyKind::RoundRobin => SelectionMethod::RoundRobin,
        rook_core::PolicyKind::LeastLatency => SelectionMethod::LeastLatency,
        rook_core::PolicyKind::LeastCost => SelectionMethod::LeastCost,
        rook_core::PolicyKind::Auto => SelectionMethod::Auto,
    };

    // Convert fallback reason
    let fallback = decision.fallback_reason.as_ref().and_then(|reason| {
        match reason {
            rook_core::PolicyFallbackReason::MetricUnavailable => {
                Some(SelectionFallback::MetricUnavailable)
            }
            rook_core::PolicyFallbackReason::NoComparableSignals => {
                Some(SelectionFallback::NoComparableSignals)
            }
            rook_core::PolicyFallbackReason::BudgetAware
            | rook_core::PolicyFallbackReason::StreamingLatencyProxy
            | rook_core::PolicyFallbackReason::LatencyEvidenceAvailable
            | rook_core::PolicyFallbackReason::CostEvidenceAvailable => {
                // Auto resolver rules — not fallbacks, just explanations
                None
            }
            rook_core::PolicyFallbackReason::WeightedConfigError => {
                Some(SelectionFallback::WeightedConfigError)
            }
            rook_core::PolicyFallbackReason::IncompatibleBasis => {
                Some(SelectionFallback::IncompatibleBasis)
            }
        }
    });

    let resolution = decision
        .fallback_reason
        .as_ref()
        .and_then(|reason| match reason {
            rook_core::PolicyFallbackReason::BudgetAware => Some(SelectionFallback::BudgetAware),
            rook_core::PolicyFallbackReason::StreamingLatencyProxy => {
                Some(SelectionFallback::StreamingLatencyProxy)
            }
            rook_core::PolicyFallbackReason::LatencyEvidenceAvailable => {
                Some(SelectionFallback::LatencyEvidenceAvailable)
            }
            rook_core::PolicyFallbackReason::CostEvidenceAvailable => {
                Some(SelectionFallback::CostEvidenceAvailable)
            }
            _ => None,
        });

    Some(SelectionReason {
        selected_target: selected_target.clone(),
        method,
        components,
        fallback,
        resolution,
        requested_policy: Some(decision.requested_policy),
        effective_policy: Some(decision.effective_policy),
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
                            RawEvidence::Unknown(if !p50.is_finite() {
                                UnknownReason::NonFinite
                            } else {
                                UnknownReason::Invalid
                            })
                        },
                        if p95_valid {
                            RawEvidence::Known(p95)
                        } else {
                            RawEvidence::Unknown(if !p95.is_finite() {
                                UnknownReason::NonFinite
                            } else {
                                UnknownReason::Invalid
                            })
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
                RawEvidence::Unknown(if !entry.prompt_per_million.is_finite() {
                    UnknownReason::NonFinite
                } else {
                    UnknownReason::Invalid
                })
            } else {
                RawEvidence::Known(entry.prompt_per_million)
            },
            output_per_million: if entry.completion_per_million.is_sign_negative()
                || entry.completion_per_million.is_nan()
                || entry.completion_per_million.is_infinite()
            {
                RawEvidence::Unknown(if !entry.completion_per_million.is_finite() {
                    UnknownReason::NonFinite
                } else {
                    UnknownReason::Invalid
                })
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
