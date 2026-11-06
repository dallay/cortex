//! Trusted host policy. Presentation plugins can collect decisions but never see grants.
use async_trait::async_trait;
use huginn_core::{
    AgentError, ApprovalDecision, ApprovalPolicy, ApprovalPrompt, ApprovalProvenance,
    ApprovalRequest, CancellationToken, Result,
};
use std::{
    collections::{BTreeSet, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub struct HostPolicy<P: ApprovalPrompt + ?Sized> {
    prompt: Arc<P>,
    allowed: BTreeSet<String>,
    session_grants: Mutex<HashSet<(String, PathBuf)>>,
}
impl<P: ApprovalPrompt + ?Sized> HostPolicy<P> {
    pub fn new(prompt: Arc<P>, allowed: BTreeSet<String>) -> Self {
        Self {
            prompt,
            allowed,
            session_grants: Mutex::new(HashSet::new()),
        }
    }
}
#[async_trait]
impl<P: ApprovalPrompt + ?Sized> ApprovalPolicy for HostPolicy<P> {
    async fn approve(
        &self,
        request: &ApprovalRequest,
        cancel: CancellationToken,
    ) -> Result<ApprovalDecision> {
        if cancel.is_cancelled() || !self.prompt.is_available() {
            return Err(AgentError::Cancelled);
        }
        let eligible = matches!(
            request.action.as_str(),
            "native.write_file" | "native.edit_file"
        ) && request.provenance == Some(ApprovalProvenance::NativeFileBuiltin)
            && request.resource.as_ref().is_some_and(|resource| {
                !resource.sensitive && resource.canonical_path.is_absolute()
            });
        if eligible {
            let resource = request
                .resource
                .as_ref()
                .expect("eligibility checks resource");
            let key = (request.action.clone(), resource.canonical_path.clone());
            if self
                .session_grants
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(&key)
            {
                return Ok(ApprovalDecision::ApproveForSession);
            }
        }
        let allowed_decisions = if eligible {
            vec![
                ApprovalDecision::Reject,
                ApprovalDecision::ApproveOnce,
                ApprovalDecision::ApproveForSession,
            ]
        } else {
            vec![ApprovalDecision::Reject, ApprovalDecision::ApproveOnce]
        };
        let requires_confirmation = request.action == "native.shell"
            || request.action.starts_with("native.shell.")
            || matches!(
                request.action.as_str(),
                "native.write_file" | "native.edit_file"
            )
            || request.action.starts_with("mcp.");
        if !requires_confirmation && self.allowed.contains(&request.action) {
            self.prompt.granted(request);
            return if cancel.is_cancelled() || !self.prompt.is_available() {
                Err(AgentError::Cancelled)
            } else {
                Ok(ApprovalDecision::ApproveOnce)
            };
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AgentError::Cancelled),
            answer = self.prompt.request_decision(request, &allowed_decisions, cancel.clone()) => {
                if cancel.is_cancelled() || !self.prompt.is_available() {
                    Err(AgentError::Cancelled)
                } else {
                    answer.map(|decision| {
                        if !allowed_decisions.contains(&decision) || (decision == ApprovalDecision::ApproveForSession && !eligible) {
                            ApprovalDecision::Reject
                        } else {
                            if decision == ApprovalDecision::ApproveForSession {
                                let resource = request.resource.as_ref().expect("eligible request has a resource");
                                self.session_grants.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .insert((request.action.clone(), resource.canonical_path.clone()));
                            }
                            decision
                        }
                    })
                }
            }
        }
    }
}
