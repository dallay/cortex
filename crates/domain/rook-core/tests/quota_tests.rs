//! Integration coverage for the public upstream quota evaluation API.

use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use rook_core::{
    effective_reset_at, evaluate_connection_quota, reduce_latest_observations, ProviderId,
    QuotaEligibilityPolicy, QuotaEvidence, QuotaObservation, QuotaObservationKey,
    QuotaObservationQuery, QuotaState, DEFAULT_APPLICABLE_METRICS, DEFAULT_EXHAUSTED_REMAINING,
    DEFAULT_FRESHNESS,
};
use shared_kernel::ConnectionId;

fn connection(id: &str) -> ConnectionId {
    ConnectionId::parse_str(id).expect("connection id")
}

fn observation(
    connection_id: &str,
    metric: &str,
    remaining: Option<f64>,
    observed_at: DateTime<Utc>,
) -> QuotaObservation {
    QuotaObservation {
        provider_id: "openai".to_string(),
        provider: ProviderId::new("openai-runtime"),
        connection_id: Some(connection(connection_id)),
        metric: metric.to_string(),
        source: "response_headers".to_string(),
        scope: None,
        window: None,
        limit: Some(100.0),
        remaining,
        reset_at: None,
        reset_after_milliseconds: None,
        observed_at,
    }
}

#[test]
fn exhausted_connection_is_reported_with_provenance() {
    let now = Utc::now();
    let connection_id = connection("11111111-1111-4111-8111-111111111111");
    let observations = vec![observation(
        "11111111-1111-4111-8111-111111111111",
        "requests",
        Some(0.0),
        now,
    )];

    let state = evaluate_connection_quota(
        &observations,
        &connection_id,
        Some(&ProviderId::new("openai-runtime")),
        now,
        &QuotaEligibilityPolicy::default(),
    );

    assert!(state.is_exhausted());
    let evidence = state.evidence().expect("evidence");
    assert_eq!(evidence.metric, "requests");
    assert!(evidence.describe().contains("remaining=0"));
}

#[test]
fn observations_of_other_connections_do_not_leak() {
    let now = Utc::now();
    let connection_id = connection("22222222-2222-4222-8222-222222222222");
    let observations = vec![observation(
        "11111111-1111-4111-8111-111111111111",
        "requests",
        Some(0.0),
        now,
    )];

    let state = evaluate_connection_quota(
        &observations,
        &connection_id,
        None,
        now,
        &QuotaEligibilityPolicy::default(),
    );

    assert_eq!(state, QuotaState::Unknown);
}

#[test]
fn provider_scoped_observation_does_not_prove_connection_exhaustion() {
    let now = Utc::now();
    let connection_id = connection("11111111-1111-4111-8111-111111111111");
    let mut scoped = observation(
        "11111111-1111-4111-8111-111111111111",
        "requests",
        Some(0.0),
        now,
    );
    scoped.connection_id = None;

    let state = evaluate_connection_quota(
        &[scoped],
        &connection_id,
        None,
        now,
        &QuotaEligibilityPolicy::default(),
    );

    assert_eq!(state, QuotaState::Unknown);
}

#[test]
fn stale_evidence_is_unknown_not_exhausted() {
    let now = Utc::now();
    let connection_id = connection("11111111-1111-4111-8111-111111111111");
    let observations = vec![observation(
        "11111111-1111-4111-8111-111111111111",
        "requests",
        Some(0.0),
        now - chrono::Duration::minutes(30),
    )];

    let state = evaluate_connection_quota(
        &observations,
        &connection_id,
        None,
        now,
        &QuotaEligibilityPolicy::default(),
    );

    assert!(matches!(state, QuotaState::Stale(_)));
    assert!(!state.is_exhausted());
    assert!(state.is_unknown());
}

#[test]
fn metric_outside_the_policy_is_not_applicable() {
    let now = Utc::now();
    let connection_id = connection("11111111-1111-4111-8111-111111111111");
    let observations = vec![observation(
        "11111111-1111-4111-8111-111111111111",
        "priority_tokens",
        Some(0.0),
        now,
    )];

    let state = evaluate_connection_quota(
        &observations,
        &connection_id,
        None,
        now,
        &QuotaEligibilityPolicy::default(),
    );

    assert!(matches!(state, QuotaState::NotApplicable(_)));
    assert!(!state.is_exhausted());
}

#[test]
fn expired_reset_restores_eligibility() {
    let now = Utc::now();
    let connection_id = connection("11111111-1111-4111-8111-111111111111");
    let mut exhausted = observation(
        "11111111-1111-4111-8111-111111111111",
        "requests",
        Some(0.0),
        now - chrono::Duration::seconds(30),
    );
    exhausted.reset_at = Some(now - chrono::Duration::seconds(10));

    let state = evaluate_connection_quota(
        &[exhausted],
        &connection_id,
        None,
        now,
        &QuotaEligibilityPolicy::default(),
    );

    assert!(matches!(state, QuotaState::Stale(_)));
    assert!(!state.is_exhausted());
}

#[test]
fn future_reset_keeps_exhaustion() {
    let now = Utc::now();
    let connection_id = connection("11111111-1111-4111-8111-111111111111");
    let mut exhausted = observation(
        "11111111-1111-4111-8111-111111111111",
        "requests",
        Some(0.0),
        now,
    );
    exhausted.reset_at = Some(now + chrono::Duration::seconds(45));

    let state = evaluate_connection_quota(
        &[exhausted],
        &connection_id,
        None,
        now,
        &QuotaEligibilityPolicy::default(),
    );

    assert!(state.is_exhausted());
}

#[test]
fn relative_reset_is_anchored_to_observation() {
    let now = Utc::now();
    let connection_id = connection("11111111-1111-4111-8111-111111111111");
    let mut exhausted = observation(
        "11111111-1111-4111-8111-111111111111",
        "requests",
        Some(0.0),
        now - chrono::Duration::seconds(10),
    );
    exhausted.reset_after_milliseconds = Some(60_000);

    let state = evaluate_connection_quota(
        &[exhausted],
        &connection_id,
        None,
        now,
        &QuotaEligibilityPolicy::default(),
    );

    assert!(state.is_exhausted());
    let evidence = state.evidence().expect("evidence");
    assert!(evidence.reset_at.is_some());
}

#[test]
fn newer_available_observation_wins_over_older_exhaustion() {
    let now = Utc::now();
    let connection_id = connection("11111111-1111-4111-8111-111111111111");
    let older = observation(
        "11111111-1111-4111-8111-111111111111",
        "requests",
        Some(0.0),
        now - chrono::Duration::seconds(120),
    );
    let newer = observation(
        "11111111-1111-4111-8111-111111111111",
        "requests",
        Some(25.0),
        now,
    );

    let state = evaluate_connection_quota(
        &[older, newer],
        &connection_id,
        None,
        now,
        &QuotaEligibilityPolicy::default(),
    );

    assert!(state.is_available());
    assert!(!state.is_exhausted());
}

#[test]
fn reduce_keeps_newest_observation_per_series() {
    let now = Utc::now();
    let older = observation(
        "11111111-1111-4111-8111-111111111111",
        "requests",
        Some(0.0),
        now - chrono::Duration::seconds(60),
    );
    let newer = observation(
        "11111111-1111-4111-8111-111111111111",
        "requests",
        Some(10.0),
        now,
    );
    let other = observation(
        "22222222-2222-4222-8222-222222222222",
        "requests",
        Some(5.0),
        now,
    );

    let query = QuotaObservationQuery {
        connection_ids: vec![connection("11111111-1111-4111-8111-111111111111")],
        ..Default::default()
    };
    let reduced = reduce_latest_observations(vec![older, newer, other], &query);

    assert_eq!(reduced.len(), 1);
    assert_eq!(reduced[0].remaining, Some(10.0));
}

// ---------------------------------------------------------------------------
// Contract coverage that used to live in an inline `#[cfg(test)]` module.
// Every case below drives the public quota functions with a fixed clock.
// ---------------------------------------------------------------------------

const CONNECTION_A: &str = "11111111-1111-4111-8111-111111111111";
const CONNECTION_B: &str = "22222222-2222-4222-8222-222222222222";

/// A fixed instant plus `second` seconds. No wall clock is read.
fn fixed_timestamp(second: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0)
        .single()
        .expect("fixed timestamp")
        + chrono::Duration::seconds(second)
}

fn series_observation(
    connection_id: Option<&str>,
    metric: &str,
    source: &str,
    observed_at: DateTime<Utc>,
    remaining: Option<f64>,
) -> QuotaObservation {
    QuotaObservation {
        provider_id: "openai".to_string(),
        provider: ProviderId::new("openai-primary"),
        connection_id: connection_id.map(connection),
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

fn series_evidence(
    remaining: Option<f64>,
    limit: Option<f64>,
    reset_at: Option<DateTime<Utc>>,
) -> QuotaEvidence {
    QuotaEvidence {
        provider: ProviderId::new("openai-primary"),
        connection_id: Some(connection(CONNECTION_A)),
        metric: "requests".to_string(),
        source: "headers".to_string(),
        scope: Some("account".to_string()),
        window: Some("daily".to_string()),
        limit,
        remaining,
        observed_at: fixed_timestamp(0),
        reset_at,
    }
}

#[test]
fn quota_observation_key_copies_the_series_identity() {
    let observation = series_observation(
        Some(CONNECTION_A),
        "requests",
        "headers",
        fixed_timestamp(0),
        Some(10.0),
    );
    let key = QuotaObservationKey::from(&observation);
    assert_eq!(key.connection_id, Some(connection(CONNECTION_A)));
    assert_eq!(key.metric, "requests");
    assert_eq!(key.source, "headers");
    assert_eq!(key.scope.as_deref(), Some("account"));
    assert_eq!(key.window.as_deref(), Some("daily"));
}

#[test]
fn quota_evidence_describe_includes_optional_fields_in_order() {
    let reset_at = fixed_timestamp(30);
    let rendered = series_evidence(Some(12.5), Some(100.0), Some(reset_at)).describe();
    assert_eq!(
        rendered,
        format!(
            "provider=openai-primary metric=requests source=headers scope=account window=daily remaining=12.5 limit=100 reset_at={} observed_at={}",
            reset_at.to_rfc3339(),
            fixed_timestamp(0).to_rfc3339()
        )
    );
}

#[test]
fn quota_evidence_describe_omits_absent_optional_fields() {
    let mut minimal = series_evidence(None, None, None);
    minimal.scope = None;
    minimal.window = None;
    let rendered = minimal.describe();
    assert_eq!(
        rendered,
        format!(
            "provider=openai-primary metric=requests source=headers observed_at={}",
            fixed_timestamp(0).to_rfc3339()
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
    let evidence = series_evidence(Some(0.0), Some(100.0), None);
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
    let mut observation =
        series_observation(None, "requests", "headers", fixed_timestamp(0), Some(1.0));
    observation.reset_at = Some(fixed_timestamp(60));
    observation.reset_after_milliseconds = Some(600_000);
    assert_eq!(effective_reset_at(&observation), Some(fixed_timestamp(60)));
}

#[test]
fn effective_reset_at_anchors_the_relative_reset_to_the_observation() {
    let mut observation =
        series_observation(None, "requests", "headers", fixed_timestamp(0), Some(1.0));
    observation.reset_after_milliseconds = Some(60_000);
    assert_eq!(effective_reset_at(&observation), Some(fixed_timestamp(60)));
}

#[test]
fn effective_reset_at_is_none_without_reset_information() {
    let observation =
        series_observation(None, "requests", "headers", fixed_timestamp(0), Some(1.0));
    assert_eq!(effective_reset_at(&observation), None);
}

#[test]
fn evaluate_connection_quota_without_observations_is_unknown() {
    let state = evaluate_connection_quota(
        &[],
        &connection(CONNECTION_A),
        None,
        fixed_timestamp(0),
        &QuotaEligibilityPolicy::default(),
    );
    assert_eq!(state, QuotaState::Unknown);
}

#[test]
fn evaluate_connection_quota_ignores_observations_of_other_providers() {
    let mut observation = series_observation(
        Some(CONNECTION_A),
        "requests",
        "headers",
        fixed_timestamp(0),
        Some(0.0),
    );
    observation.provider = ProviderId::new("anthropic-primary");
    let state = evaluate_connection_quota(
        &[observation],
        &connection(CONNECTION_A),
        Some(&ProviderId::new("openai-primary")),
        fixed_timestamp(0),
        &QuotaEligibilityPolicy::default(),
    );
    assert_eq!(state, QuotaState::Unknown);
}

#[test]
fn evaluate_connection_quota_reports_available_when_capacity_remains() {
    let observations = vec![series_observation(
        Some(CONNECTION_A),
        "requests",
        "headers",
        fixed_timestamp(0),
        Some(42.0),
    )];
    let state = evaluate_connection_quota(
        &observations,
        &connection(CONNECTION_A),
        None,
        fixed_timestamp(0),
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
fn evaluate_connection_quota_reports_stale_for_observations_beyond_the_freshness_window() {
    let observations = vec![series_observation(
        Some(CONNECTION_A),
        "requests",
        "headers",
        fixed_timestamp(0),
        Some(0.0),
    )];
    // The observation is 301 seconds old, one second past the default window.
    let state = evaluate_connection_quota(
        &observations,
        &connection(CONNECTION_A),
        None,
        fixed_timestamp(301),
        &QuotaEligibilityPolicy::default(),
    );
    match state {
        QuotaState::Stale(evidence) => assert_eq!(evidence.remaining, Some(0.0)),
        other => panic!("unexpected state: {other:?}"),
    }
}

#[test]
fn evaluate_connection_quota_keeps_observations_inside_the_freshness_window() {
    let observations = vec![series_observation(
        Some(CONNECTION_A),
        "requests",
        "headers",
        fixed_timestamp(0),
        Some(0.0),
    )];
    // Exactly at the window length the observation is still fresh.
    let state = evaluate_connection_quota(
        &observations,
        &connection(CONNECTION_A),
        None,
        fixed_timestamp(300),
        &QuotaEligibilityPolicy::default(),
    );
    assert!(state.is_exhausted());
}

#[test]
fn evaluate_connection_quota_ignores_observations_without_a_remaining_counter() {
    let observations = vec![series_observation(
        Some(CONNECTION_A),
        "requests",
        "headers",
        fixed_timestamp(0),
        None,
    )];
    let state = evaluate_connection_quota(
        &observations,
        &connection(CONNECTION_A),
        None,
        fixed_timestamp(0),
        &QuotaEligibilityPolicy::default(),
    );
    assert_eq!(state, QuotaState::Unknown);
}

#[test]
fn evaluate_connection_quota_prefers_exhausted_over_available() {
    let observations = vec![
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "headers",
            fixed_timestamp(0),
            Some(10.0),
        ),
        series_observation(
            Some(CONNECTION_A),
            "tokens",
            "headers",
            fixed_timestamp(1),
            Some(0.0),
        ),
    ];
    let state = evaluate_connection_quota(
        &observations,
        &connection(CONNECTION_A),
        None,
        fixed_timestamp(1),
        &QuotaEligibilityPolicy::default(),
    );
    match state {
        QuotaState::Exhausted(evidence) => assert_eq!(evidence.metric, "tokens"),
        other => panic!("unexpected state: {other:?}"),
    }
}

#[test]
fn evaluate_connection_quota_prefers_available_over_stale() {
    let observations = vec![
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "headers",
            fixed_timestamp(0),
            Some(10.0),
        ),
        series_observation(
            Some(CONNECTION_A),
            "tokens",
            "headers",
            fixed_timestamp(-600),
            Some(0.0),
        ),
    ];
    let state = evaluate_connection_quota(
        &observations,
        &connection(CONNECTION_A),
        None,
        fixed_timestamp(0),
        &QuotaEligibilityPolicy::default(),
    );
    assert!(state.is_available());
}

#[test]
fn evaluate_connection_quota_keeps_the_first_observation_when_timestamps_tie() {
    let observations = vec![
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "headers",
            fixed_timestamp(0),
            Some(11.0),
        ),
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "headers",
            fixed_timestamp(0),
            Some(22.0),
        ),
    ];
    let state = evaluate_connection_quota(
        &observations,
        &connection(CONNECTION_A),
        None,
        fixed_timestamp(0),
        &QuotaEligibilityPolicy::default(),
    );
    match state {
        QuotaState::Available(evidence) => assert_eq!(evidence.remaining, Some(11.0)),
        other => panic!("unexpected state: {other:?}"),
    }
}

#[test]
fn evaluate_connection_quota_honours_a_custom_exhaustion_threshold() {
    let observations = vec![series_observation(
        Some(CONNECTION_A),
        "requests",
        "headers",
        fixed_timestamp(0),
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
    let state = evaluate_connection_quota(
        &observations,
        &connection(CONNECTION_A),
        None,
        fixed_timestamp(0),
        &policy,
    );
    assert!(state.is_exhausted());
}

#[test]
fn reduce_latest_observations_orders_newest_first() {
    let observations = vec![
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "headers",
            fixed_timestamp(0),
            Some(1.0),
        ),
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "headers",
            fixed_timestamp(10),
            Some(2.0),
        ),
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "headers",
            fixed_timestamp(5),
            Some(3.0),
        ),
    ];
    let reduced = reduce_latest_observations(observations, &QuotaObservationQuery::default());
    assert_eq!(reduced.len(), 1);
    assert_eq!(reduced[0].observed_at, fixed_timestamp(10));
    assert_eq!(reduced[0].remaining, Some(2.0));
}

#[test]
fn reduce_latest_observations_filters_by_connection_provider_metric_and_source() {
    let mut foreign_provider = series_observation(
        Some(CONNECTION_A),
        "requests",
        "headers",
        fixed_timestamp(0),
        Some(1.0),
    );
    foreign_provider.provider = ProviderId::new("anthropic-primary");

    let observations = vec![
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "headers",
            fixed_timestamp(0),
            Some(1.0),
        ),
        series_observation(
            Some(CONNECTION_B),
            "requests",
            "headers",
            fixed_timestamp(1),
            Some(2.0),
        ),
        series_observation(
            Some(CONNECTION_A),
            "tokens",
            "headers",
            fixed_timestamp(2),
            Some(3.0),
        ),
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "body",
            fixed_timestamp(3),
            Some(4.0),
        ),
        foreign_provider,
    ];

    let query = QuotaObservationQuery {
        connection_ids: vec![connection(CONNECTION_A)],
        provider: Some(ProviderId::new("openai-primary")),
        metric: Some("requests".to_string()),
        source: Some("headers".to_string()),
        limit: 0,
    };
    let reduced = reduce_latest_observations(observations, &query);
    assert_eq!(reduced.len(), 1);
    assert_eq!(reduced[0].observed_at, fixed_timestamp(0));
    assert_eq!(reduced[0].remaining, Some(1.0));
}

#[test]
fn reduce_latest_observations_drops_observations_without_a_connection_when_filtered() {
    let observations = vec![series_observation(
        None,
        "requests",
        "headers",
        fixed_timestamp(0),
        Some(1.0),
    )];
    let query = QuotaObservationQuery {
        connection_ids: vec![connection(CONNECTION_A)],
        ..QuotaObservationQuery::default()
    };
    let reduced = reduce_latest_observations(observations, &query);
    assert!(reduced.is_empty());
}

#[test]
fn reduce_latest_observations_keeps_one_entry_per_series() {
    let observations = vec![
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "headers",
            fixed_timestamp(0),
            Some(1.0),
        ),
        series_observation(
            Some(CONNECTION_A),
            "tokens",
            "headers",
            fixed_timestamp(1),
            Some(2.0),
        ),
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "headers",
            fixed_timestamp(10),
            Some(3.0),
        ),
    ];
    let reduced = reduce_latest_observations(observations, &QuotaObservationQuery::default());
    assert_eq!(reduced.len(), 2);
    assert_eq!(reduced[0].metric, "requests");
    assert_eq!(reduced[0].observed_at, fixed_timestamp(10));
    assert_eq!(reduced[1].metric, "tokens");
    assert_eq!(reduced[1].observed_at, fixed_timestamp(1));
}

#[test]
fn reduce_latest_observations_applies_the_clamped_limit() {
    let observations = vec![
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "headers",
            fixed_timestamp(0),
            Some(1.0),
        ),
        series_observation(
            Some(CONNECTION_A),
            "tokens",
            "headers",
            fixed_timestamp(1),
            Some(2.0),
        ),
        series_observation(
            Some(CONNECTION_A),
            "input_tokens",
            "headers",
            fixed_timestamp(2),
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
fn reduce_latest_observations_breaks_timestamp_ties_by_source() {
    let observations = vec![
        series_observation(
            Some(CONNECTION_A),
            "requests",
            "body",
            fixed_timestamp(0),
            Some(1.0),
        ),
        series_observation(
            Some(CONNECTION_A),
            "tokens",
            "headers",
            fixed_timestamp(0),
            Some(2.0),
        ),
    ];
    let reduced = reduce_latest_observations(observations, &QuotaObservationQuery::default());
    assert_eq!(reduced.len(), 2);
    // Equal timestamps are ordered by descending source name.
    assert_eq!(reduced[0].source, "headers");
    assert_eq!(reduced[1].source, "body");
}

#[test]
fn evaluate_connection_quota_treats_an_unrepresentable_freshness_as_zero() {
    // `Duration::MAX` cannot be represented as a `chrono::Duration`, so the
    // policy falls back to a zero-length window. The staleness comparison is
    // strict (`age > freshness`), so only a *positive* age exceeds that
    // window: an observation observed exactly now still carries meaning.
    let observations = vec![series_observation(
        Some(CONNECTION_A),
        "requests",
        "headers",
        fixed_timestamp(0),
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

    // Zero age: the observation is not stale, so its capacity counts.
    let at_observation = evaluate_connection_quota(
        &observations,
        &connection(CONNECTION_A),
        None,
        fixed_timestamp(0),
        &policy,
    );
    match at_observation {
        QuotaState::Available(evidence) => assert_eq!(evidence.remaining, Some(42.0)),
        other => panic!("unexpected state at zero age: {other:?}"),
    }

    // Positive age: the zero-length window is exceeded, so it is stale.
    let one_second_later = evaluate_connection_quota(
        &observations,
        &connection(CONNECTION_A),
        None,
        fixed_timestamp(1),
        &policy,
    );
    match one_second_later {
        QuotaState::Stale(evidence) => assert_eq!(evidence.remaining, Some(42.0)),
        other => panic!("unexpected state after one second: {other:?}"),
    }
}
