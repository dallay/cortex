//! Management-only Codex accounts. These records are not routing connections.
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccount {
    pub id: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexLoginStatus {
    Pending,
    Linked,
    Cancelled,
    Expired,
    Failed,
}

#[derive(Clone, Serialize)]
pub struct CodexAttempt {
    pub id: String,
    pub status: CodexLoginStatus,
}

#[derive(Serialize)]
pub struct CodexAccounts {
    pub enabled: bool,
    pub accounts: Vec<CodexAccount>,
    pub attempt: Option<CodexAttempt>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLogin {
    pub id: String,
    pub auth_url: String,
}

#[derive(Debug)]
pub enum CodexAccountError {
    Busy,
    Capacity,
    NotFound,
    Unavailable,
    /// The loopback callback port is already in use (for example, by `codex login`).
    CallbackBusy,
}

/// Loopback path pinned by the public Codex OAuth client.
pub const CODEX_CALLBACK_PATH: &str = "/auth/callback";

/// Loopback port pinned by the public Codex OAuth client.
pub const CODEX_CALLBACK_PORT: u16 = 1455;

/// The exact redirect URI the public Codex OAuth client accepts.
pub fn codex_redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}{CODEX_CALLBACK_PATH}")
}

pub enum CodexCallback {
    Code(String),
    Denied,
    Failed,
}

#[async_trait]
pub trait CodexAccountsPort: Send + Sync {
    async fn list(&self) -> Result<CodexAccounts, CodexAccountError>;
    async fn start(&self) -> Result<CodexLogin, CodexAccountError>;
    async fn callback(&self, state: &str, response: CodexCallback)
        -> Result<(), CodexAccountError>;
    async fn cancel(&self, id: &str) -> Result<(), CodexAccountError>;
    async fn disconnect(&self, id: &str) -> Result<(), CodexAccountError>;
    async fn shutdown(&self);
}
