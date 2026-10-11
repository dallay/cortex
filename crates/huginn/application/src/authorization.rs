//! Trusted host policy. Presentation plugins can collect decisions but never see grants.
use async_trait::async_trait;
use huginn_core::{
    AgentError, ApprovalPolicy, ApprovalPrompt, ApprovalRequest, CancellationToken, Result,
};
use std::{collections::BTreeSet, sync::Arc};

pub struct HostPolicy<P: ApprovalPrompt + ?Sized> {
    prompt: Arc<P>,
    allowed: BTreeSet<String>,
}
impl<P: ApprovalPrompt + ?Sized> HostPolicy<P> {
    pub const fn new(prompt: Arc<P>, allowed: BTreeSet<String>) -> Self {
        Self { prompt, allowed }
    }
}
#[async_trait]
impl<P: ApprovalPrompt + ?Sized> ApprovalPolicy for HostPolicy<P> {
    async fn approve(&self, request: &ApprovalRequest, cancel: CancellationToken) -> Result<bool> {
        if cancel.is_cancelled() || !self.prompt.is_available() {
            return Err(AgentError::Cancelled);
        }
        if self.allowed.contains(&request.action) {
            self.prompt.granted(request);
            return if cancel.is_cancelled() || !self.prompt.is_available() {
                Err(AgentError::Cancelled)
            } else {
                Ok(true)
            };
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AgentError::Cancelled),
            answer = self.prompt.request_decision(request, cancel.clone()) => {
                if cancel.is_cancelled() || !self.prompt.is_available() { Err(AgentError::Cancelled) } else { answer }
            }
        }
    }
}
