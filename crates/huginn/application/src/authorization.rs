//! Trusted host policy. Presentation plugins can collect decisions but never see grants.
use crate::permission_policy::{AuthorizationOutcome, Capability, EffectivePolicy, PolicySettings};
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
    effective_policy: Arc<EffectivePolicy>,
    session_grants: Arc<Mutex<HashSet<(String, String, PathBuf)>>>,
}
impl<P: ApprovalPrompt + ?Sized> Clone for HostPolicy<P> {
    fn clone(&self) -> Self {
        Self {
            prompt: self.prompt.clone(),
            allowed: self.allowed.clone(),
            effective_policy: self.effective_policy.clone(),
            session_grants: self.session_grants.clone(),
        }
    }
}
impl<P: ApprovalPrompt + ?Sized> HostPolicy<P> {
    pub fn new(prompt: Arc<P>, allowed: BTreeSet<String>) -> Self {
        Self {
            prompt,
            allowed,
            effective_policy: Arc::new(EffectivePolicy::resolve(
                PolicySettings::default(),
                None,
                Default::default(),
            )),
            session_grants: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn with_effective_policy(mut self, policy: Arc<EffectivePolicy>) -> Self {
        self.effective_policy = policy;
        self
    }

    pub fn effective_policy(&self) -> &EffectivePolicy {
        &self.effective_policy
    }

    pub fn for_session(&self, session_id: impl Into<String>) -> SessionPolicy<P> {
        SessionPolicy {
            host: self.clone(),
            session_id: session_id.into(),
        }
    }

    async fn approve_for(
        &self,
        session_id: Option<&str>,
        request: &ApprovalRequest,
        cancel: CancellationToken,
    ) -> Result<ApprovalDecision> {
        if cancel.is_cancelled() || !self.prompt.is_available() {
            return Err(AgentError::Cancelled);
        }
        let capability = match request.action.as_str() {
            "native.write_file" | "native.edit_file" => Some(Capability::FileWrite),
            action if action == "native.shell" || action.starts_with("native.shell.") => {
                Some(Capability::Shell)
            }
            _ => None,
        };
        // Restrictions are checked before any volatile grant lookup. Neither a
        // session approval nor a matching workspace grant can weaken them.
        if capability.is_some_and(|capability| {
            self.effective_policy.decide(capability, true) == AuthorizationOutcome::Denied
        }) {
            return Ok(ApprovalDecision::Reject);
        }
        let eligible = matches!(
            request.action.as_str(),
            "native.write_file" | "native.edit_file"
        ) && request.provenance == Some(ApprovalProvenance::NativeFileBuiltin)
            && request.resource.as_ref().is_some_and(|resource| {
                !resource.sensitive && resource.canonical_path.is_absolute()
            });
        let session_eligible = eligible && session_id.is_some();
        if session_eligible {
            let resource = request
                .resource
                .as_ref()
                .expect("eligibility checks resource");
            let key = (
                session_id
                    .expect("session eligibility checks identity")
                    .to_owned(),
                request.action.clone(),
                resource.canonical_path.clone(),
            );
            if self
                .session_grants
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(&key)
            {
                return Ok(ApprovalDecision::ApproveForSession);
            }
        }
        let allowed_decisions = if session_eligible {
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
                        if !allowed_decisions.contains(&decision) || (decision == ApprovalDecision::ApproveForSession && !session_eligible) {
                            ApprovalDecision::Reject
                        } else {
                            if decision == ApprovalDecision::ApproveForSession {
                                let resource = request.resource.as_ref().expect("eligible request has a resource");
                                self.session_grants.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .insert((session_id.expect("session grants require identity").to_owned(), request.action.clone(), resource.canonical_path.clone()));
                            }
                            decision
                        }
                    })
                }
            }
        }
    }
}

pub struct SessionPolicy<P: ApprovalPrompt + ?Sized> {
    host: HostPolicy<P>,
    session_id: String,
}

#[async_trait]
impl<P: ApprovalPrompt + ?Sized> ApprovalPolicy for SessionPolicy<P> {
    async fn approve(
        &self,
        request: &ApprovalRequest,
        cancel: CancellationToken,
    ) -> Result<ApprovalDecision> {
        self.host
            .approve_for(Some(&self.session_id), request, cancel)
            .await
    }
}

#[async_trait]
impl<P: ApprovalPrompt + ?Sized> ApprovalPolicy for HostPolicy<P> {
    async fn approve(
        &self,
        request: &ApprovalRequest,
        cancel: CancellationToken,
    ) -> Result<ApprovalDecision> {
        self.approve_for(None, request, cancel).await
    }
}
