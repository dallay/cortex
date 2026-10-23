// Quota eligibility — connection-scoped evaluation of upstream quota evidence.
//
// Provider-reported quota observations stay separate from locally measured
// usage (`UsageEntry`). Nothing in this module infers capacity: a connection is
// skipped only when a fresh, applicable observation reports exhaustion, and
// every other outcome keeps the connection eligible.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared_kernel::{ConnectionId, ProviderId};

use crate::model::QuotaObservation;
use crate::ports::QuotaObservationQuery;

/// Metrics that describe request capacity for a completion call.
pub const DEFAULT_APPLICABLE_METRICS: &[&str] =
    &["requests", "tokens", "input_tokens", "output_tokens"];

/// Default freshness window for an observation to remain trustworthy.
pub const DEFAULT_FRESHNESS: Duration = Duration::from_secs(300);

/// Default remaining value that counts as exhaustion.
pub const DEFAULT_EXHAUSTED_REMAINING: f64 = 0.0;

/// Identity of one quota series.
///
/// Two observations of the same series describe the same capacity counter, so
/// only the newest one carries meaning.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QuotaObservationKey {
    pub connection_id: Option<ConnectionId>,
    pub metric: String,
    pub scope: Option<String>,
    pub window: Option<String>,
    pub source: String,
}

impl From<&QuotaObservation> for QuotaObservationKey {
    fn from(observation: &QuotaObservation) -> Self {
        Self {
            connection_id: observation.connection_id,
            metric: observation.metric.clone(),
            scope: observation.scope.clone(),
            window: observation.window.clone(),
            source: observation.source.clone(),
        }
    }
}

/// Provenance of a quota decision.
///
/// The payload carries only provider-reported counters and timing. It never
/// carries credentials, headers, or account identifiers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaEvidence {
    pub provider: ProviderId,
    pub connection_id: Option<ConnectionId>,
    pub metric: String,
    pub source: String,
    pub scope: Option<String>,
    pub window: Option<String>,
    pub limit: Option<f64>,
    pub remaining: Option<f64>,
    pub observed_at: DateTime<Utc>,
    /// Effective reset instant, derived from `reset_at` or the relative
    /// `reset_after_milliseconds` anchored to `observed_at`.
    pub reset_at: Option<DateTime<Utc>>,
}

impl QuotaEvidence {
    /// Human-readable provenance without credentials.
    pub fn describe(&self) -> String {
        let mut text = format!(
            "provider={} metric={} source={}",
            self.provider.as_str(),
            self.metric,
            self.source
        );
        if let Some(scope) = &self.scope {
            text.push_str(&format!(" scope={scope}"));
        }
        if let Some(window) = &self.window {
            text.push_str(&format!(" window={window}"));
        }
        if let Some(remaining) = self.remaining {
            text.push_str(&format!(" remaining={remaining}"));
        }
        if let Some(limit) = self.limit {
            text.push_str(&format!(" limit={limit}"));
        }
        if let Some(reset_at) = self.reset_at {
            text.push_str(&format!(" reset_at={}", reset_at.to_rfc3339()));
        }
        text.push_str(&format!(" observed_at={}", self.observed_at.to_rfc3339()));
        text
    }
}

/// Quota state of a single connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QuotaState {
    /// Fresh, applicable evidence reports no remaining capacity.
    Exhausted(QuotaEvidence),
    /// Fresh, applicable evidence reports remaining capacity.
    Available(QuotaEvidence),
    /// Evidence exists but is older than the freshness window, or its reset
    /// instant already passed. Capacity is unknown, never exhausted.
    Stale(QuotaEvidence),
    /// Evidence exists but describes a metric that does not apply to requests.
    NotApplicable(QuotaEvidence),
    /// No evidence at all.
    Unknown,
}

impl QuotaState {
    /// Whether the state proves exhaustion. Only this state may skip a
    /// connection.
    pub fn is_exhausted(&self) -> bool {
        matches!(self, Self::Exhausted(_))
    }

    /// Whether capacity is known to be available.
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }

    /// Whether the state leaves capacity unknown.
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown | Self::Stale(_))
    }

    /// Evidence behind the state, when the state carries any.
    pub fn evidence(&self) -> Option<&QuotaEvidence> {
        match self {
            Self::Exhausted(evidence)
            | Self::Available(evidence)
            | Self::Stale(evidence)
            | Self::NotApplicable(evidence) => Some(evidence),
            Self::Unknown => None,
        }
    }
}

/// Policy that decides when quota evidence is trustworthy and what counts as
/// exhaustion.
///
/// The defaults are a conservative starting point, not a universal provider
/// contract. Callers may narrow the applicable metrics or the freshness
/// window per deployment.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaEligibilityPolicy {
    /// Maximum age of an observation that still carries meaning.
    pub freshness: Duration,
    /// Remaining value at or below which the connection counts as exhausted.
    pub exhausted_remaining: f64,
    /// Metrics that describe capacity for a completion request.
    pub applicable_metrics: Vec<String>,
}

impl Default for QuotaEligibilityPolicy {
    fn default() -> Self {
        Self {
            freshness: DEFAULT_FRESHNESS,
            exhausted_remaining: DEFAULT_EXHAUSTED_REMAINING,
            applicable_metrics: DEFAULT_APPLICABLE_METRICS
                .iter()
                .map(|metric| metric.to_string())
                .collect(),
        }
    }
}

impl QuotaEligibilityPolicy {
    /// Whether the metric describes capacity for a completion request.
    pub fn applies_to_metric(&self, metric: &str) -> bool {
        self.applicable_metrics
            .iter()
            .any(|applicable| applicable.eq_ignore_ascii_case(metric))
    }
}

/// Effective reset instant of an observation.
///
/// A relative reset is anchored to the observation instant, so it stays
/// meaningful even when the provider omits an absolute timestamp.
pub fn effective_reset_at(observation: &QuotaObservation) -> Option<DateTime<Utc>> {
    observation.reset_at.or_else(|| {
        observation.reset_after_milliseconds.map(|milliseconds| {
            observation.observed_at + chrono::Duration::milliseconds(milliseconds as i64)
        })
    })
}

/// Evaluate the quota state of one connection.
///
/// Only observations captured for that connection take part: a provider-scoped
/// observation cannot prove that one specific connection is exhausted.
/// Evidence from another connection never leaks into the result.
pub fn evaluate_connection_quota(
    observations: &[QuotaObservation],
    connection_id: &ConnectionId,
    provider: Option<&ProviderId>,
    now: DateTime<Utc>,
    policy: &QuotaEligibilityPolicy,
) -> QuotaState {
    let scoped: Vec<&QuotaObservation> = observations
        .iter()
        .filter(|observation| observation.connection_id.as_ref() == Some(connection_id))
        .filter(|observation| match provider {
            Some(provider) => &observation.provider == provider,
            None => true,
        })
        .collect();

    if scoped.is_empty() {
        return QuotaState::Unknown;
    }

    // Only the newest observation of each series carries meaning.
    let mut latest: Vec<&QuotaObservation> = Vec::new();
    for observation in &scoped {
        let key = QuotaObservationKey::from(*observation);
        let superseded = latest.iter().any(|existing| {
            QuotaObservationKey::from(*existing) == key
                && existing.observed_at >= observation.observed_at
        });
        if !superseded {
            latest.retain(|existing| QuotaObservationKey::from(*existing) != key);
            latest.push(observation);
        }
    }

    let mut exhausted: Option<QuotaEvidence> = None;
    let mut available: Option<QuotaEvidence> = None;
    let mut stale: Option<QuotaEvidence> = None;
    let mut not_applicable: Option<QuotaEvidence> = None;

    for observation in latest {
        let evidence = QuotaEvidence {
            provider: observation.provider.clone(),
            connection_id: observation.connection_id,
            metric: observation.metric.clone(),
            source: observation.source.clone(),
            scope: observation.scope.clone(),
            window: observation.window.clone(),
            limit: observation.limit,
            remaining: observation.remaining,
            observed_at: observation.observed_at,
            reset_at: effective_reset_at(observation),
        };

        if !policy.applies_to_metric(&observation.metric) {
            not_applicable.get_or_insert(evidence);
            continue;
        }

        let age = now.signed_duration_since(observation.observed_at);
        if age
            > chrono::Duration::from_std(policy.freshness)
                .unwrap_or_else(|_| chrono::Duration::zero())
        {
            stale.get_or_insert(evidence);
            continue;
        }

        let Some(remaining) = observation.remaining else {
            // A metric without a remaining counter proves nothing.
            continue;
        };

        if remaining <= policy.exhausted_remaining {
            let expired = evidence
                .reset_at
                .map(|reset_at| reset_at <= now)
                .unwrap_or(false);
            if expired {
                stale.get_or_insert(evidence);
            } else {
                exhausted.get_or_insert(evidence);
            }
        } else {
            available.get_or_insert(evidence);
        }
    }

    if let Some(evidence) = exhausted {
        return QuotaState::Exhausted(evidence);
    }
    if let Some(evidence) = available {
        return QuotaState::Available(evidence);
    }
    if let Some(evidence) = stale {
        return QuotaState::Stale(evidence);
    }
    if let Some(evidence) = not_applicable {
        return QuotaState::NotApplicable(evidence);
    }
    QuotaState::Unknown
}

/// Keep only the newest observation of each series and apply the query filters.
///
/// The result is ordered from newest to oldest observation.
pub fn reduce_latest_observations(
    observations: Vec<QuotaObservation>,
    query: &QuotaObservationQuery,
) -> Vec<QuotaObservation> {
    let mut filtered: Vec<QuotaObservation> = observations
        .into_iter()
        .filter(|observation| match &query.connection_ids {
            ids if ids.is_empty() => true,
            ids => observation
                .connection_id
                .as_ref()
                .map(|connection_id| ids.contains(connection_id))
                .unwrap_or(false),
        })
        .filter(|observation| match &query.provider {
            Some(provider) => &observation.provider == provider,
            None => true,
        })
        .filter(|observation| match &query.metric {
            Some(metric) => &observation.metric == metric,
            None => true,
        })
        .filter(|observation| match &query.source {
            Some(source) => &observation.source == source,
            None => true,
        })
        .collect();

    filtered.sort_by(|left, right| {
        right
            .observed_at
            .cmp(&left.observed_at)
            .then_with(|| right.source.cmp(&left.source))
    });

    let mut seen: Vec<QuotaObservationKey> = Vec::new();
    let mut latest: Vec<QuotaObservation> = Vec::new();
    for observation in filtered {
        let key = QuotaObservationKey::from(&observation);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        latest.push(observation);
        if latest.len() >= query.clamped_limit() {
            break;
        }
    }
    latest
}
