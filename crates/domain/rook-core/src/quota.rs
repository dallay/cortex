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
    pub const fn is_exhausted(&self) -> bool {
        matches!(self, Self::Exhausted(_))
    }

    /// Whether capacity is known to be available.
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }

    /// Whether the state leaves capacity unknown.
    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown | Self::Stale(_))
    }

    /// Evidence behind the state, when the state carries any.
    pub const fn evidence(&self) -> Option<&QuotaEvidence> {
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
                .map(ToString::to_string)
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
        .filter(|observation| provider.is_none() || &observation.provider == provider.unwrap())
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
        .filter(|observation| {
            query.provider.as_ref().is_none()
                || &observation.provider == query.provider.as_ref().unwrap()
        })
        .filter(|observation| {
            query.metric.as_ref().is_none() || &observation.metric == query.metric.as_ref().unwrap()
        })
        .filter(|observation| {
            query.source.as_ref().is_none() || &observation.source == query.source.as_ref().unwrap()
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn timestamp(second: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0)
            .single()
            .expect("fixed timestamp")
            + chrono::Duration::seconds(second)
    }

    fn observation(
        connection_id: Option<ConnectionId>,
        metric: &str,
        source: &str,
        observed_at: DateTime<Utc>,
        remaining: Option<f64>,
    ) -> QuotaObservation {
        QuotaObservation {
            provider_id: "openai".to_string(),
            provider: ProviderId::new("openai-primary"),
            connection_id,
            metric: metric.to_string(),
            source: source.to_string(),
            scope: Some("account".to_string()),
            window: Some("daily".to_string()),
            limit: Some(100.0),
            remaining,
            reset_at: None,
            reset_after_milliseconds: None,
            observed_at,
        }
    }

    fn evidence(
        remaining: Option<f64>,
        limit: Option<f64>,
        reset_at: Option<DateTime<Utc>>,
    ) -> QuotaEvidence {
        QuotaEvidence {
            provider: ProviderId::new("openai-primary"),
            connection_id: Some(ConnectionId::new()),
            metric: "requests".to_string(),
            source: "headers".to_string(),
            scope: Some("account".to_string()),
            window: Some("daily".to_string()),
            limit,
            remaining,
            observed_at: timestamp(0),
            reset_at,
        }
    }

    #[test]
    fn quota_observation_key_copies_the_series_identity() {
        let connection_id = ConnectionId::new();
        let observation = observation(
            Some(connection_id),
            "requests",
            "headers",
            timestamp(0),
            Some(10.0),
        );
        let key = QuotaObservationKey::from(&observation);
        assert_eq!(key.connection_id, Some(connection_id));
        assert_eq!(key.metric, "requests");
        assert_eq!(key.source, "headers");
        assert_eq!(key.scope.as_deref(), Some("account"));
        assert_eq!(key.window.as_deref(), Some("daily"));
    }

    #[test]
    fn quota_evidence_describe_includes_optional_fields_in_order() {
        let reset_at = timestamp(30);
        let rendered = evidence(Some(12.5), Some(100.0), Some(reset_at)).describe();
        assert_eq!(
            rendered,
            format!(
                "provider=openai-primary metric=requests source=headers scope=account window=daily remaining=12.5 limit=100 reset_at={} observed_at={}",
                reset_at.to_rfc3339(),
                timestamp(0).to_rfc3339()
            )
        );
    }

    #[test]
    fn quota_evidence_describe_omits_absent_optional_fields() {
        let mut minimal = evidence(None, None, None);
        minimal.scope = None;
        minimal.window = None;
        let rendered = minimal.describe();
        assert_eq!(
            rendered,
            format!(
                "provider=openai-primary metric=requests source=headers observed_at={}",
                timestamp(0).to_rfc3339()
            )
        );
        assert!(!rendered.contains("remaining="));
        assert!(!rendered.contains("limit="));
        assert!(!rendered.contains("scope="));
        assert!(!rendered.contains("window="));
        assert!(!rendered.contains("reset_at="));
    }

    #[test]
    fn quota_state_predicates_classify_each_variant() {
        let evidence = evidence(Some(0.0), Some(100.0), None);
        let exhausted = QuotaState::Exhausted(evidence.clone());
        assert!(exhausted.is_exhausted());
        assert!(!exhausted.is_available());
        assert!(!exhausted.is_unknown());
        assert_eq!(exhausted.evidence(), Some(&evidence));

        let available = QuotaState::Available(evidence.clone());
        assert!(!available.is_exhausted());
        assert!(available.is_available());
        assert!(!available.is_unknown());
        assert_eq!(available.evidence(), Some(&evidence));

        let stale = QuotaState::Stale(evidence.clone());
        assert!(!stale.is_exhausted());
        assert!(!stale.is_available());
        assert!(stale.is_unknown());
        assert_eq!(stale.evidence(), Some(&evidence));

        let not_applicable = QuotaState::NotApplicable(evidence.clone());
        assert!(!not_applicable.is_exhausted());
        assert!(!not_applicable.is_available());
        // A metric that does not apply leaves capacity unknown.
        assert!(!not_applicable.is_unknown());
        assert_eq!(not_applicable.evidence(), Some(&evidence));

        let unknown = QuotaState::Unknown;
        assert!(!unknown.is_exhausted());
        assert!(!unknown.is_available());
        assert!(unknown.is_unknown());
        assert_eq!(unknown.evidence(), None);
    }

    #[test]
    fn default_policy_uses_the_documented_defaults() {
        let policy = QuotaEligibilityPolicy::default();
        assert_eq!(policy.freshness, DEFAULT_FRESHNESS);
        assert_eq!(policy.freshness, Duration::from_secs(300));
        assert_eq!(policy.exhausted_remaining, DEFAULT_EXHAUSTED_REMAINING);
        assert_eq!(policy.exhausted_remaining, 0.0);
        assert_eq!(policy.applicable_metrics, DEFAULT_APPLICABLE_METRICS);
        assert_eq!(policy.applicable_metrics.len(), 4);
    }

    #[test]
    fn policy_applies_to_metric_is_case_insensitive() {
        let policy = QuotaEligibilityPolicy::default();
        assert!(policy.applies_to_metric("requests"));
        assert!(policy.applies_to_metric("REQUESTS"));
        assert!(policy.applies_to_metric("tokens"));
        assert!(policy.applies_to_metric("input_tokens"));
        assert!(policy.applies_to_metric("output_tokens"));
        assert!(!policy.applies_to_metric("cost_usd"));
        assert!(!policy.applies_to_metric(""));
    }

    #[test]
    fn effective_reset_at_prefers_the_absolute_reset_instant() {
        let mut observation = observation(None, "requests", "headers", timestamp(0), Some(1.0));
        observation.reset_at = Some(timestamp(60));
        observation.reset_after_milliseconds = Some(600_000);
        assert_eq!(effective_reset_at(&observation), Some(timestamp(60)));
    }

    #[test]
    fn effective_reset_at_anchors_the_relative_reset_to_the_observation() {
        let mut observation = observation(None, "requests", "headers", timestamp(0), Some(1.0));
        observation.reset_after_milliseconds = Some(60_000);
        assert_eq!(effective_reset_at(&observation), Some(timestamp(60)));
    }

    #[test]
    fn effective_reset_at_is_none_without_reset_information() {
        let observation = observation(None, "requests", "headers", timestamp(0), Some(1.0));
        assert_eq!(effective_reset_at(&observation), None);
    }

    #[test]
    fn evaluate_connection_quota_without_observations_is_unknown() {
        let connection_id = ConnectionId::new();
        let state = evaluate_connection_quota(
            &[],
            &connection_id,
            None,
            timestamp(0),
            &QuotaEligibilityPolicy::default(),
        );
        assert_eq!(state, QuotaState::Unknown);
    }

    #[test]
    fn evaluate_connection_quota_ignores_observations_of_other_connections() {
        let connection_id = ConnectionId::new();
        let other = ConnectionId::new();
        let observations = vec![observation(
            Some(other),
            "requests",
            "headers",
            timestamp(0),
            Some(0.0),
        )];
        let state = evaluate_connection_quota(
            &observations,
            &connection_id,
            None,
            timestamp(0),
            &QuotaEligibilityPolicy::default(),
        );
        assert_eq!(state, QuotaState::Unknown);
    }

    #[test]
    fn evaluate_connection_quota_ignores_observations_of_other_providers() {
        let connection_id = ConnectionId::new();
        let mut observation = observation(
            Some(connection_id),
            "requests",
            "headers",
            timestamp(0),
            Some(0.0),
        );
        observation.provider = ProviderId::new("anthropic-primary");
        let state = evaluate_connection_quota(
            &[observation],
            &connection_id,
            Some(&ProviderId::new("openai-primary")),
            timestamp(0),
            &QuotaEligibilityPolicy::default(),
        );
        assert_eq!(state, QuotaState::Unknown);
    }

    #[test]
    fn evaluate_connection_quota_reports_available_when_capacity_remains() {
        let connection_id = ConnectionId::new();
        let observations = vec![observation(
            Some(connection_id),
            "requests",
            "headers",
            timestamp(0),
            Some(42.0),
        )];
        let state = evaluate_connection_quota(
            &observations,
            &connection_id,
            None,
            timestamp(0),
            &QuotaEligibilityPolicy::default(),
        );
        match state {
            QuotaState::Available(evidence) => {
                assert_eq!(evidence.remaining, Some(42.0));
                assert_eq!(evidence.limit, Some(100.0));
                assert_eq!(evidence.provider, ProviderId::new("openai-primary"));
            }
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn evaluate_connection_quota_reports_exhausted_at_or_below_the_threshold() {
        let connection_id = ConnectionId::new();
        let observations = vec![observation(
            Some(connection_id),
            "requests",
            "headers",
            timestamp(0),
            Some(0.0),
        )];
        let state = evaluate_connection_quota(
            &observations,
            &connection_id,
            None,
            timestamp(0),
            &QuotaEligibilityPolicy::default(),
        );
        assert!(state.is_exhausted());
    }

    #[test]
    fn evaluate_connection_quota_reports_stale_for_observations_beyond_the_freshness_window() {
        let connection_id = ConnectionId::new();
        let observations = vec![observation(
            Some(connection_id),
            "requests",
            "headers",
            timestamp(0),
            Some(0.0),
        )];
        // The observation is 301 seconds old, one second past the default window.
        let now = timestamp(0) + chrono::Duration::seconds(301);
        let state = evaluate_connection_quota(
            &observations,
            &connection_id,
            None,
            now,
            &QuotaEligibilityPolicy::default(),
        );
        match state {
            QuotaState::Stale(evidence) => assert_eq!(evidence.remaining, Some(0.0)),
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn evaluate_connection_quota_keeps_observations_inside_the_freshness_window() {
        let connection_id = ConnectionId::new();
        let observations = vec![observation(
            Some(connection_id),
            "requests",
            "headers",
            timestamp(0),
            Some(0.0),
        )];
        let now = timestamp(0) + chrono::Duration::seconds(300);
        let state = evaluate_connection_quota(
            &observations,
            &connection_id,
            None,
            now,
            &QuotaEligibilityPolicy::default(),
        );
        assert!(state.is_exhausted());
    }

    #[test]
    fn evaluate_connection_quota_reports_not_applicable_for_unknown_metrics() {
        let connection_id = ConnectionId::new();
        let observations = vec![observation(
            Some(connection_id),
            "cost_usd",
            "headers",
            timestamp(0),
            Some(0.0),
        )];
        let state = evaluate_connection_quota(
            &observations,
            &connection_id,
            None,
            timestamp(0),
            &QuotaEligibilityPolicy::default(),
        );
        match state {
            QuotaState::NotApplicable(evidence) => assert_eq!(evidence.metric, "cost_usd"),
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn evaluate_connection_quota_ignores_observations_without_a_remaining_counter() {
        let connection_id = ConnectionId::new();
        let observations = vec![observation(
            Some(connection_id),
            "requests",
            "headers",
            timestamp(0),
            None,
        )];
        let state = evaluate_connection_quota(
            &observations,
            &connection_id,
            None,
            timestamp(0),
            &QuotaEligibilityPolicy::default(),
        );
        assert_eq!(state, QuotaState::Unknown);
    }

    #[test]
    fn evaluate_connection_quota_reports_stale_when_the_reset_instant_already_passed() {
        let connection_id = ConnectionId::new();
        let mut observation = observation(
            Some(connection_id),
            "requests",
            "headers",
            timestamp(0),
            Some(0.0),
        );
        observation.reset_at = Some(timestamp(10));
        let now = timestamp(0) + chrono::Duration::seconds(20);
        let state = evaluate_connection_quota(
            &[observation],
            &connection_id,
            None,
            now,
            &QuotaEligibilityPolicy::default(),
        );
        match state {
            QuotaState::Stale(evidence) => assert_eq!(evidence.reset_at, Some(timestamp(10))),
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn evaluate_connection_quota_keeps_exhausted_evidence_when_the_reset_is_still_ahead() {
        let connection_id = ConnectionId::new();
        let mut observation = observation(
            Some(connection_id),
            "requests",
            "headers",
            timestamp(0),
            Some(0.0),
        );
        observation.reset_at = Some(timestamp(60));
        let now = timestamp(0) + chrono::Duration::seconds(20);
        let state = evaluate_connection_quota(
            &[observation],
            &connection_id,
            None,
            now,
            &QuotaEligibilityPolicy::default(),
        );
        assert!(state.is_exhausted());
    }

    #[test]
    fn evaluate_connection_quota_prefers_exhausted_over_available() {
        let connection_id = ConnectionId::new();
        let observations = vec![
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(0),
                Some(10.0),
            ),
            observation(
                Some(connection_id),
                "tokens",
                "headers",
                timestamp(1),
                Some(0.0),
            ),
        ];
        let state = evaluate_connection_quota(
            &observations,
            &connection_id,
            None,
            timestamp(1),
            &QuotaEligibilityPolicy::default(),
        );
        match state {
            QuotaState::Exhausted(evidence) => assert_eq!(evidence.metric, "tokens"),
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn evaluate_connection_quota_prefers_available_over_stale() {
        let connection_id = ConnectionId::new();
        let observations = vec![
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(0),
                Some(10.0),
            ),
            observation(
                Some(connection_id),
                "tokens",
                "headers",
                timestamp(0) - chrono::Duration::seconds(600),
                Some(0.0),
            ),
        ];
        let state = evaluate_connection_quota(
            &observations,
            &connection_id,
            None,
            timestamp(0),
            &QuotaEligibilityPolicy::default(),
        );
        assert!(state.is_available());
    }

    #[test]
    fn evaluate_connection_quota_keeps_the_newest_observation_of_a_series() {
        let connection_id = ConnectionId::new();
        let observations = vec![
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(0),
                Some(0.0),
            ),
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(10),
                Some(50.0),
            ),
        ];
        let state = evaluate_connection_quota(
            &observations,
            &connection_id,
            None,
            timestamp(10),
            &QuotaEligibilityPolicy::default(),
        );
        match state {
            QuotaState::Available(evidence) => assert_eq!(evidence.remaining, Some(50.0)),
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn evaluate_connection_quota_keeps_the_first_observation_when_timestamps_tie() {
        let connection_id = ConnectionId::new();
        let observations = vec![
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(0),
                Some(11.0),
            ),
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(0),
                Some(22.0),
            ),
        ];
        let state = evaluate_connection_quota(
            &observations,
            &connection_id,
            None,
            timestamp(0),
            &QuotaEligibilityPolicy::default(),
        );
        match state {
            QuotaState::Available(evidence) => assert_eq!(evidence.remaining, Some(11.0)),
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn evaluate_connection_quota_honours_a_custom_exhaustion_threshold() {
        let connection_id = ConnectionId::new();
        let observations = vec![observation(
            Some(connection_id),
            "requests",
            "headers",
            timestamp(0),
            Some(5.0),
        )];
        let policy = QuotaEligibilityPolicy {
            freshness: DEFAULT_FRESHNESS,
            exhausted_remaining: 10.0,
            applicable_metrics: DEFAULT_APPLICABLE_METRICS
                .iter()
                .map(ToString::to_string)
                .collect(),
        };
        let state =
            evaluate_connection_quota(&observations, &connection_id, None, timestamp(0), &policy);
        assert!(state.is_exhausted());
    }

    #[test]
    fn reduce_latest_observations_orders_newest_first() {
        let connection_id = ConnectionId::new();
        let observations = vec![
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(0),
                Some(1.0),
            ),
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(10),
                Some(2.0),
            ),
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(5),
                Some(3.0),
            ),
        ];
        let reduced = reduce_latest_observations(observations, &QuotaObservationQuery::default());
        assert_eq!(reduced.len(), 1);
        assert_eq!(reduced[0].observed_at, timestamp(10));
        assert_eq!(reduced[0].remaining, Some(2.0));
    }

    #[test]
    fn reduce_latest_observations_filters_by_connection_provider_metric_and_source() {
        let connection_id = ConnectionId::new();
        let other = ConnectionId::new();
        let mut foreign_provider = observation(
            Some(connection_id),
            "requests",
            "headers",
            timestamp(0),
            Some(1.0),
        );
        foreign_provider.provider = ProviderId::new("anthropic-primary");

        let observations = vec![
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(0),
                Some(1.0),
            ),
            observation(Some(other), "requests", "headers", timestamp(1), Some(2.0)),
            observation(
                Some(connection_id),
                "tokens",
                "headers",
                timestamp(2),
                Some(3.0),
            ),
            observation(
                Some(connection_id),
                "requests",
                "body",
                timestamp(3),
                Some(4.0),
            ),
            foreign_provider,
        ];

        let query = QuotaObservationQuery {
            connection_ids: vec![connection_id],
            provider: Some(ProviderId::new("openai-primary")),
            metric: Some("requests".to_string()),
            source: Some("headers".to_string()),
            limit: 0,
        };
        let reduced = reduce_latest_observations(observations, &query);
        assert_eq!(reduced.len(), 1);
        assert_eq!(reduced[0].observed_at, timestamp(0));
        assert_eq!(reduced[0].remaining, Some(1.0));
    }

    #[test]
    fn reduce_latest_observations_drops_observations_without_a_connection_when_filtered() {
        let connection_id = ConnectionId::new();
        let observations = vec![observation(
            None,
            "requests",
            "headers",
            timestamp(0),
            Some(1.0),
        )];
        let query = QuotaObservationQuery {
            connection_ids: vec![connection_id],
            ..QuotaObservationQuery::default()
        };
        let reduced = reduce_latest_observations(observations, &query);
        assert!(reduced.is_empty());
    }

    #[test]
    fn reduce_latest_observations_keeps_one_entry_per_series() {
        let connection_id = ConnectionId::new();
        let observations = vec![
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(0),
                Some(1.0),
            ),
            observation(
                Some(connection_id),
                "tokens",
                "headers",
                timestamp(1),
                Some(2.0),
            ),
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(10),
                Some(3.0),
            ),
        ];
        let reduced = reduce_latest_observations(observations, &QuotaObservationQuery::default());
        assert_eq!(reduced.len(), 2);
        assert_eq!(reduced[0].metric, "requests");
        assert_eq!(reduced[0].observed_at, timestamp(10));
        assert_eq!(reduced[1].metric, "tokens");
        assert_eq!(reduced[1].observed_at, timestamp(1));
    }

    #[test]
    fn reduce_latest_observations_applies_the_clamped_limit() {
        let connection_id = ConnectionId::new();
        let observations = vec![
            observation(
                Some(connection_id),
                "requests",
                "headers",
                timestamp(0),
                Some(1.0),
            ),
            observation(
                Some(connection_id),
                "tokens",
                "headers",
                timestamp(1),
                Some(2.0),
            ),
            observation(
                Some(connection_id),
                "input_tokens",
                "headers",
                timestamp(2),
                Some(3.0),
            ),
        ];
        let query = QuotaObservationQuery {
            limit: 2,
            ..QuotaObservationQuery::default()
        };
        assert_eq!(query.clamped_limit(), 2);
        let reduced = reduce_latest_observations(observations, &query);
        assert_eq!(reduced.len(), 2);
        assert_eq!(reduced[0].metric, "input_tokens");
        assert_eq!(reduced[1].metric, "tokens");
    }

    #[test]
    fn reduce_latest_observations_without_input_returns_nothing() {
        let reduced = reduce_latest_observations(Vec::new(), &QuotaObservationQuery::default());
        assert!(reduced.is_empty());
    }

    #[test]
    fn evaluate_connection_quota_treats_an_unrepresentable_freshness_as_zero() {
        // `Duration::MAX` cannot be represented as a `chrono::Duration`, so the
        // policy falls back to a zero-length window and every observation is stale.
        let connection_id = ConnectionId::new();
        let observations = vec![observation(
            Some(connection_id),
            "requests",
            "headers",
            timestamp(0),
            Some(42.0),
        )];
        let policy = QuotaEligibilityPolicy {
            freshness: Duration::MAX,
            exhausted_remaining: DEFAULT_EXHAUSTED_REMAINING,
            applicable_metrics: DEFAULT_APPLICABLE_METRICS
                .iter()
                .map(ToString::to_string)
                .collect(),
        };
        let now = timestamp(0) + chrono::Duration::seconds(1);
        let state = evaluate_connection_quota(&observations, &connection_id, None, now, &policy);
        match state {
            QuotaState::Stale(evidence) => assert_eq!(evidence.remaining, Some(42.0)),
            other => panic!("unexpected state: {other:?}"),
        }
    }

    #[test]
    fn reduce_latest_observations_breaks_timestamp_ties_by_source() {
        let connection_id = ConnectionId::new();
        let observations = vec![
            observation(
                Some(connection_id),
                "requests",
                "body",
                timestamp(0),
                Some(1.0),
            ),
            observation(
                Some(connection_id),
                "tokens",
                "headers",
                timestamp(0),
                Some(2.0),
            ),
        ];
        let reduced = reduce_latest_observations(observations, &QuotaObservationQuery::default());
        assert_eq!(reduced.len(), 2);
        // Equal timestamps are ordered by descending source name.
        assert_eq!(reduced[0].source, "headers");
        assert_eq!(reduced[1].source, "body");
    }
}
