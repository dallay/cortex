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

/// Literal loopback addresses only: no DNS name can change a local exception
/// into a request to a remote host. Use 127.0.0.1 or [::1], not a LAN address.
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
