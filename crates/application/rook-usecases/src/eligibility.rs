use serde::{Deserialize, Serialize};

// TargetRef is now defined in rook-core to avoid cyclic dependencies.
pub use rook_core::TargetRef;

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
#[path = "eligibility/tests.rs"]
mod tests;
