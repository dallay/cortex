use super::{
    evaluate_candidates, AuthState, CandidateFacts, Rejection, RejectionReason, TargetRef,
};
use shared_kernel::ProviderId;

fn eligible_candidate(target: &str) -> CandidateFacts {
    CandidateFacts {
        target: TargetRef(ProviderId::new(target)),
        enabled: true,
        auth: AuthState::Valid,
        model_supported: true,
        capabilities_supported: true,
        circuit_open: false,
        quota_exhausted: false,
        key_model_allowed: true,
        key_provider_allowed: true,
    }
}

#[test]
fn rejection_reason_serializes_to_stable_snake_case() {
    let cases = [
        (RejectionReason::AuthMissing, "auth_missing"),
        (RejectionReason::AuthExpired, "auth_expired"),
        (RejectionReason::ModelUnsupported, "model_unsupported"),
        (RejectionReason::CapabilityMissing, "capability_missing"),
        (RejectionReason::CircuitOpen, "circuit_open"),
        (RejectionReason::QuotaExhausted, "quota_exhausted"),
        (RejectionReason::KeyModelNotAllowed, "key_model_not_allowed"),
        (
            RejectionReason::KeyProviderNotAllowed,
            "key_provider_not_allowed",
        ),
        (RejectionReason::ConnectionDisabled, "connection_disabled"),
    ];

    for (reason, expected) in cases {
        assert_eq!(
            serde_json::to_string(&reason).expect("serialize rejection reason"),
            format!("\"{expected}\""),
        );
    }
}

#[test]
fn eligibility_rejects_each_ineligible_candidate_with_one_reason() {
    let mut missing_auth = eligible_candidate("missing-auth");
    missing_auth.auth = AuthState::Missing;
    let mut expired_auth = eligible_candidate("expired-auth");
    expired_auth.auth = AuthState::Expired;
    let mut unsupported_model = eligible_candidate("unsupported-model");
    unsupported_model.model_supported = false;
    let mut missing_capability = eligible_candidate("missing-capability");
    missing_capability.capabilities_supported = false;
    let mut open_circuit = eligible_candidate("open-circuit");
    open_circuit.circuit_open = true;
    let mut exhausted_quota = eligible_candidate("exhausted-quota");
    exhausted_quota.quota_exhausted = true;
    let mut disallowed_model = eligible_candidate("disallowed-model");
    disallowed_model.key_model_allowed = false;
    let mut disallowed_provider = eligible_candidate("disallowed-provider");
    disallowed_provider.key_provider_allowed = false;
    let mut disabled_connection = eligible_candidate("disabled-connection");
    disabled_connection.enabled = false;

    let cases = [
        (missing_auth, RejectionReason::AuthMissing),
        (expired_auth, RejectionReason::AuthExpired),
        (unsupported_model, RejectionReason::ModelUnsupported),
        (missing_capability, RejectionReason::CapabilityMissing),
        (open_circuit, RejectionReason::CircuitOpen),
        (exhausted_quota, RejectionReason::QuotaExhausted),
        (disallowed_model, RejectionReason::KeyModelNotAllowed),
        (disallowed_provider, RejectionReason::KeyProviderNotAllowed),
        (disabled_connection, RejectionReason::ConnectionDisabled),
    ];

    let result = evaluate_candidates(cases.iter().map(|(candidate, _)| candidate.clone()));

    assert!(result.eligible.is_empty());
    assert_eq!(result.rejected.len(), cases.len());
    for ((candidate, expected_reason), rejection) in cases.iter().zip(&result.rejected) {
        assert_eq!(
            rejection,
            &Rejection {
                target: candidate.target.clone(),
                reason: *expected_reason,
            }
        );
    }
}

#[test]
fn eligibility_keeps_fully_eligible_candidates() {
    let candidate = eligible_candidate("eligible");

    let result = evaluate_candidates([candidate.clone()]);

    assert_eq!(result.eligible, vec![candidate.target]);
    assert!(result.rejected.is_empty());
}

#[test]
fn eligibility_uses_key_policy_precedence_for_multiple_failures() {
    let mut candidate = eligible_candidate("multiple-failures");
    candidate.key_provider_allowed = false;
    candidate.enabled = false;
    candidate.model_supported = false;

    let result = evaluate_candidates([candidate.clone()]);

    assert_eq!(
        result.rejected,
        vec![Rejection {
            target: candidate.target,
            reason: RejectionReason::KeyProviderNotAllowed,
        }]
    );
    assert!(result.eligible.is_empty());
}
