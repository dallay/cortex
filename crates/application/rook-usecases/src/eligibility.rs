use serde::{Deserialize, Serialize};
use shared_kernel::ProviderId;

/// Stable identity of a provider target considered by routing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TargetRef(pub ProviderId);

/// Closed set of reasons that can make a provider target ineligible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    AuthMissing,
    AuthExpired,
    ModelUnsupported,
    CapabilityMissing,
    CircuitOpen,
    QuotaExhausted,
    KeyModelNotAllowed,
    KeyProviderNotAllowed,
    ConnectionDisabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejection {
    pub target: TargetRef,
    pub reason: RejectionReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EligibilityResult {
    pub eligible: Vec<TargetRef>,
    pub rejected: Vec<Rejection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthState {
    Valid,
    Missing,
    Expired,
}

#[derive(Debug, Clone)]
pub(crate) struct CandidateFacts {
    pub target: TargetRef,
    pub enabled: bool,
    pub auth: AuthState,
    pub model_supported: bool,
    pub capabilities_supported: bool,
    pub circuit_open: bool,
    pub quota_exhausted: bool,
    pub key_model_allowed: bool,
    pub key_provider_allowed: bool,
}

impl CandidateFacts {
    /// Whether this candidate passed stable filters that make a quota read unnecessary.
    pub(crate) fn requires_quota_check(&self) -> bool {
        // Circuit state is refreshed after quota I/O. Do not skip the read from an
        // earlier open-circuit snapshot because its cooldown may expire meanwhile.
        self.rejection_reason_before_runtime_state().is_none()
    }

    fn rejection_reason_before_runtime_state(&self) -> Option<RejectionReason> {
        if !self.key_model_allowed {
            Some(RejectionReason::KeyModelNotAllowed)
        } else if !self.key_provider_allowed {
            Some(RejectionReason::KeyProviderNotAllowed)
        } else if !self.enabled {
            Some(RejectionReason::ConnectionDisabled)
        } else if self.auth == AuthState::Missing {
            Some(RejectionReason::AuthMissing)
        } else if self.auth == AuthState::Expired {
            Some(RejectionReason::AuthExpired)
        } else if !self.model_supported {
            Some(RejectionReason::ModelUnsupported)
        } else if !self.capabilities_supported {
            Some(RejectionReason::CapabilityMissing)
        } else {
            None
        }
    }

    fn rejection_reason_before_quota(&self) -> Option<RejectionReason> {
        self.rejection_reason_before_runtime_state()
            .or_else(|| self.circuit_open.then_some(RejectionReason::CircuitOpen))
    }

    fn rejection_reason(&self) -> Option<RejectionReason> {
        // Keep this order stable: one candidate can carry multiple failed facts,
        // but the result must expose exactly one deterministic reason.
        self.rejection_reason_before_quota().or_else(|| {
            self.quota_exhausted
                .then_some(RejectionReason::QuotaExhausted)
        })
    }
}

pub(crate) fn evaluate_candidates(
    candidates: impl IntoIterator<Item = CandidateFacts>,
) -> EligibilityResult {
    let mut result = EligibilityResult {
        eligible: Vec::new(),
        rejected: Vec::new(),
    };

    for candidate in candidates {
        if let Some(reason) = candidate.rejection_reason() {
            result.rejected.push(Rejection {
                target: candidate.target,
                reason,
            });
        } else {
            result.eligible.push(candidate.target);
        }
    }

    result
}

#[cfg(test)]
mod tests {
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
}
