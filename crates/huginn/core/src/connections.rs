//! Headless connection persistence ports. Secrets cannot be serialized or displayed.
use crate::{providers::ConnectionIdentity, CancellationToken};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

pub struct Secret(String);
impl Secret {
    pub fn new(value: String) -> Result<Self, ConnectionError> {
        if value.is_empty() {
            return Err(ConnectionError::MissingSecret);
        }
        Ok(Self(value))
    }
    /// Only trusted credential stores and adapters may consume this value.
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        // Safe zeroing avoids leaving the owned buffer populated after use.
        use zeroize::Zeroize;
        self.0.zeroize();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SecretReference(uuid::Uuid);
impl Default for SecretReference {
    fn default() -> Self {
        Self::new()
    }
}
impl SecretReference {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
    pub fn account(&self) -> String {
        self.0.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecretMode {
    Secure,
    Ephemeral,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionSettings {
    pub provider: String,
    pub endpoint: Option<String>,
    pub model: String,
    pub requires_secret: bool,
    /// Explicit per-connection exception; never permits LAN/private HTTP.
    #[serde(default)]
    pub allow_loopback_http: bool,
}
impl ConnectionSettings {
    /// True when persisted settings violate only the explicit local-HTTP opt-in.
    /// Such records remain diagnosable and revocable, but never usable.
    pub fn is_legacy_loopback_http(&self) -> bool {
        if self.allow_loopback_http {
            return false;
        }
        let Some(endpoint) = self.endpoint.as_deref() else {
            return false;
        };
        let Ok(url) = url::Url::parse(endpoint) else {
            return false;
        };
        if url.scheme() != "http" || !is_loopback_endpoint(&url) {
            return false;
        }
        let mut opted_in = self.clone();
        opted_in.allow_loopback_http = true;
        opted_in.validate().is_ok()
    }

    pub fn validate(&self) -> Result<(), ConnectionError> {
        let id = |s: &str| {
            !s.is_empty()
                && s.len() <= 256
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.:/".contains(&c))
        };
        if !id(&self.provider) || !id(&self.model) {
            return Err(ConnectionError::InvalidConfiguration);
        }
        if let Some(endpoint) = &self.endpoint {
            // Credentials, query parameters and fragments are not metadata.
            let url =
                url::Url::parse(endpoint).map_err(|_| ConnectionError::InvalidConfiguration)?;
            if !matches!(url.scheme(), "https" | "http")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || (url.scheme() == "http"
                    && (!self.allow_loopback_http || !is_loopback_endpoint(&url)))
            {
                return Err(ConnectionError::InvalidConfiguration);
            }
        }
        Ok(())
    }
}

/// Identifiers accepted for provider and model metadata.
pub fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
}

/// Why an endpoint is rejected, so callers can explain the exact fix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointProblem {
    Malformed,
    /// HTTP towards a host that is not a literal loopback address.
    RemoteHttp,
    /// HTTP loopback without the explicit per-connection exception.
    LoopbackHttpDisabled,
}
pub fn endpoint_problem(settings: &ConnectionSettings) -> Option<EndpointProblem> {
    let endpoint = settings.endpoint.as_deref()?;
    let Ok(url) = url::Url::parse(endpoint) else {
        return Some(EndpointProblem::Malformed);
    };
    if url.scheme() != "http" {
        return None;
    }
    if !is_loopback_endpoint(&url) {
        return Some(EndpointProblem::RemoteHttp);
    }
    if !settings.allow_loopback_http {
        return Some(EndpointProblem::LoopbackHttpDisabled);
    }
    None
}

/// Literal loopback addresses only: no DNS name can change a local exception
/// into a request to a remote host. Use `127.0.0.1` or `[::1]`, not a LAN address.
pub fn is_loopback_endpoint(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionRecord {
    pub id: uuid::Uuid,
    pub revision: uuid::Uuid,
    pub settings: ConnectionSettings,
    pub secret: Option<SecretReference>,
    pub mode: SecretMode,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionMetadata {
    pub version: u64,
    pub connections: BTreeMap<uuid::Uuid, ConnectionRecord>,
    pub selected: Option<uuid::Uuid>,
    /// Includes write intents and retired references, persisted before secret I/O.
    pub cleanup: BTreeMap<SecretReference, SecretMode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionError {
    #[error("invalid explicit connection configuration; supply one complete connection")]
    InvalidConfiguration,
    #[error("connection or persisted selection is unavailable; select an existing connection")]
    InvalidSelection,
    #[error("credential is missing; reconnect this connection")]
    MissingSecret,
    #[error("secure credential backend is locked or inaccessible; unlock the OS credential store and retry")]
    Locked,
    #[error("secure credential backend unavailable; enable Keychain/Secret Service or explicitly choose ephemeral mode")]
    BackendUnavailable,
    #[error("credential operation failed; retry reconciliation")]
    SecretStore,
    #[error("connection metadata operation failed; restart and reconcile")]
    Metadata,
    #[error("connection metadata changed concurrently; reopen the manager")]
    Conflict,
    #[error("connection authorization revoked; start a new turn")]
    Revoked,
    #[error("provider registration unavailable; activate its adapter")]
    AdapterUnavailable,
    #[error("operation cancelled")]
    Cancelled,
}
pub type ConnectionResult<T> = std::result::Result<T, ConnectionError>;

#[async_trait]
pub trait SecretStore: Send + Sync {
    fn mode(&self) -> SecretMode;
    async fn read(&self, reference: &SecretReference) -> ConnectionResult<Secret>;
    async fn write(&self, reference: &SecretReference, secret: &Secret) -> ConnectionResult<()>;
    /// Idempotent: missing entries count as successfully deleted.
    async fn delete(&self, reference: &SecretReference) -> ConnectionResult<()>;
    /// Explicit backend routing; never silently downgrade secure credentials.
    async fn read_mode(
        &self,
        mode: SecretMode,
        reference: &SecretReference,
    ) -> ConnectionResult<Secret> {
        if mode != self.mode() {
            return Err(ConnectionError::MissingSecret);
        }
        self.read(reference).await
    }
    async fn write_mode(
        &self,
        mode: SecretMode,
        reference: &SecretReference,
        secret: &Secret,
    ) -> ConnectionResult<()> {
        if mode != self.mode() {
            return Err(ConnectionError::BackendUnavailable);
        }
        self.write(reference, secret).await
    }
    async fn delete_mode(
        &self,
        mode: SecretMode,
        reference: &SecretReference,
    ) -> ConnectionResult<()> {
        if mode != self.mode() {
            return if mode == SecretMode::Ephemeral {
                Ok(())
            } else {
                Err(ConnectionError::BackendUnavailable)
            };
        }
        self.delete(reference).await
    }
}

/// Headless management port, also consumed by optional declarative UI authors.
#[async_trait]
pub trait ConnectionManagement: Send + Sync {
    fn subscribe(&self) -> tokio::sync::watch::Receiver<crate::providers::ConnectionSnapshot>;
    /// Recover uncertain metadata writes from authoritative storage on the same owner.
    /// A healthy manager is unchanged; failed recovery must remain fail-closed.
    async fn recover(&self) -> ConnectionResult<()>;
    async fn list(&self) -> ConnectionResult<Vec<ConnectionRecord>>;
    async fn status(&self, id: uuid::Uuid) -> ConnectionResult<()>;
    async fn save(
        &self,
        id: Option<uuid::Uuid>,
        settings: ConnectionSettings,
        secret: Option<Secret>,
        mode: SecretMode,
    ) -> ConnectionResult<ConnectionRecord>;
    async fn select_model(
        &self,
        id: uuid::Uuid,
        model: String,
    ) -> ConnectionResult<crate::providers::ModelSelection>;
    async fn revoke(&self, id: uuid::Uuid) -> ConnectionResult<()>;
}
pub struct ConnectionsService(pub std::sync::Arc<dyn ConnectionManagement>);
pub fn connections_id() -> crate::kernel::ServiceId {
    crate::kernel::ServiceId::new("agent:connections", 1)
}
#[async_trait]
pub trait ConnectionStore: Send + Sync {
    async fn load(&self) -> ConnectionResult<ConnectionMetadata>;
    /// Atomic compare-and-swap of the complete metadata document.
    async fn save(&self, previous: u64, next: &ConnectionMetadata) -> ConnectionResult<()>;
}

pub struct ConnectionMaterial {
    pub settings: ConnectionSettings,
    pub secret: Option<Secret>,
}
#[async_trait]
pub trait ConnectionCredentials: Send + Sync {
    async fn resolve(
        &self,
        identity: &ConnectionIdentity,
        cancel: CancellationToken,
    ) -> ConnectionResult<ConnectionMaterial>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionProvenance {
    Cli,
    Toml,
    Environment,
    Persisted,
    Unconfigured,
}

/// An explicit choice is atomic, not a collection of fields to fill from another identity.
pub enum ConnectionChoice {
    Stored(uuid::Uuid),
    Explicit {
        settings: ConnectionSettings,
        secret: Option<Secret>,
    },
}
pub struct ConnectionInputs {
    pub cli: Option<ConnectionChoice>,
    pub toml: Option<ConnectionChoice>,
    pub environment: Option<ConnectionChoice>,
}
impl ConnectionInputs {
    pub fn highest(self) -> Option<(ConnectionChoice, ConnectionProvenance)> {
        self.cli
            .map(|v| (v, ConnectionProvenance::Cli))
            .or_else(|| self.toml.map(|v| (v, ConnectionProvenance::Toml)))
            .or_else(|| {
                self.environment
                    .map(|v| (v, ConnectionProvenance::Environment))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(endpoint: Option<&str>, allow_loopback_http: bool) -> ConnectionSettings {
        ConnectionSettings {
            provider: "ollama".into(),
            endpoint: endpoint.map(str::to_owned),
            model: "model".into(),
            requires_secret: false,
            allow_loopback_http,
        }
    }

    #[test]
    fn identifiers_accept_only_bounded_safe_characters() {
        assert!(valid_identifier("llama3.2:8b-instruct"));
        assert!(!valid_identifier(""));
        assert!(!valid_identifier("has space"));
        assert!(!valid_identifier(&"a".repeat(257)));
    }

    #[test]
    fn endpoint_problem_names_the_exact_failure() {
        assert_eq!(
            endpoint_problem(&settings(Some("not a url"), false)),
            Some(EndpointProblem::Malformed)
        );
        assert_eq!(
            endpoint_problem(&settings(Some("http://192.168.1.2:11434"), true)),
            Some(EndpointProblem::RemoteHttp)
        );
        assert_eq!(
            endpoint_problem(&settings(Some("http://127.0.0.1:11434"), false)),
            Some(EndpointProblem::LoopbackHttpDisabled)
        );
        assert_eq!(
            endpoint_problem(&settings(Some("http://127.0.0.1:11434"), true)),
            None
        );
        assert_eq!(
            endpoint_problem(&settings(Some("https://ollama.com"), false)),
            None
        );
        assert_eq!(endpoint_problem(&settings(None, false)), None);
    }
}
