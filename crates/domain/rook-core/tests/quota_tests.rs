//! Integration coverage for the public upstream quota evaluation API.

use chrono::{DateTime, Utc};
use rook_core::{
    evaluate_connection_quota, reduce_latest_observations, ProviderId, ProviderKind,
    QuotaEligibilityPolicy, QuotaObservation, QuotaObservationQuery, QuotaState,
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
        provider_kind: ProviderKind::OpenAI,
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
