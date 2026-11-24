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
}

pub const CODEX_CALLBACK_PATH: &str = "/api/codex/callback";

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
    async fn shutdown(&self);
}
