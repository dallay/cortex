//! Domain contracts and ports for Sign in with ChatGPT (SIWC) OAuth over Responses API.
//!
//! Pure domain types only — no network, browser, or OS secret-store I/O.
use crate::{connections::ConnectionRevision, AgentError, CancellationToken, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const SIWC_PROVIDER_ID: &str = "chatgpt-siwc";
pub const SIWC_API_BASE_URL: &str = "https://api.openai.com/v1";
pub const SIWC_AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
pub const SIWC_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub const SIWC_OIDC_DISCOVERY_URL: &str =
    "https://auth.openai.com/.well-known/openid-configuration";
pub const SIWC_INITIAL_CLIENT_ID: &str = "dynamic_agent_client";

pub const SIWC_REQUIRED_SCOPES: &[&str] = &[
    "openid",
    "offline_access",
    "resource.invoke",
    "chatgpt.tokens.use.direct",
];

pub const SIWC_REQUESTED_SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

/// Persisted inside the host-owned OS secret store as a serialized `Secret`.
/// Never stored in `sessions.db`, logs, errors, or UI state.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiwcCredentials {
    pub host_id: String,
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_unix: u64,
    #[serde(default)]
    pub granted_scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_sub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_email: Option<String>,
}

impl fmt::Debug for SiwcCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SiwcCredentials")
            .field("host_id", &self.host_id)
            .field("client_id", &self.client_id)
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_at_unix", &self.expires_at_unix)
            .field("granted_scopes", &self.granted_scopes)
            .finish()
    }
}

impl SiwcCredentials {
    pub fn validate_grants(&self) -> Result<()> {
        if self.host_id.trim().is_empty()
            || self.client_id.trim().is_empty()
            || self.access_token.trim().is_empty()
            || self.refresh_token.trim().is_empty()
        {
            return Err(AgentError::Configuration(
                "incomplete SIWC OAuth credentials".into(),
            ));
        }
        for required in SIWC_REQUIRED_SCOPES {
            if !self.granted_scopes.iter().any(|scope| scope == required) {
                return Err(AgentError::Configuration(format!(
                    "SIWC authorization missing required scope: {required}"
                )));
            }
        }
        Ok(())
    }

    pub const fn needs_refresh(&self, now_unix: u64) -> bool {
        now_unix.saturating_add(60) >= self.expires_at_unix
    }

    pub fn to_secret_json(&self) -> Result<String> {
        serde_json::to_string(self)
            .map_err(|_| AgentError::Configuration("could not serialize SIWC credentials".into()))
    }

    pub fn from_secret_json(raw: &str) -> Result<Self> {
        let creds: Self = serde_json::from_str(raw)
            .map_err(|_| AgentError::Configuration("stored SIWC credentials are invalid".into()))?;
        creds.validate_grants()?;
        Ok(creds)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SiwcRevocationOutcome {
    RemoteConfirmed,
    LocalOnly { reason: String },
}

#[async_trait]
pub trait SiwcAuthPort: Send + Sync {
    async fn sign_in(&self, cancel: CancellationToken) -> Result<SiwcCredentials>;
    async fn refresh(
        &self,
        credentials: &SiwcCredentials,
        cancel: CancellationToken,
    ) -> Result<SiwcCredentials>;
    async fn revoke_remote(
        &self,
        credentials: &SiwcCredentials,
        cancel: CancellationToken,
    ) -> SiwcRevocationOutcome;
}

/// Host-owned persistence hook for rotated SIWC OAuth credentials, serialized with logout.
#[async_trait]
pub trait SiwcCredentialPersistence: Send + Sync {
    /// Persist rotated credentials before use, only if the originating revision is still live.
    async fn persist_rotated(
        &self,
        origin: ConnectionRevision,
        credentials: &SiwcCredentials,
    ) -> Result<()>;
    /// Read credentials for the originating revision. Absence and store failures are distinct.
    async fn load_latest(&self, origin: ConnectionRevision) -> Result<Option<SiwcCredentials>>;
}
