//! Product-owned contracts for Huginn, the Cortex coding agent. No terminal or Rook dependencies.
pub mod approval;
pub mod kernel;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc};
pub use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("configuration: {0}")]
    Configuration(String),
    #[error("model: {0}")]
    Model(String),
    #[error("tool: {0}")]
    Tool(String),
    #[error("session: {0}")]
    Session(String),
    #[error("composition: {0}")]
    Composition(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, AgentError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}
impl Message {
    pub fn text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }
    pub const fn tool(id: String, content: String) -> Self {
        Self {
            role: Role::Tool,
            content,
            tool_calls: vec![],
            tool_call_id: Some(id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
}
#[derive(Debug, Clone)]
pub enum ModelDelta {
    Text(String),
    ToolCall(ToolCall),
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    Finished,
}
#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    TurnStarted {
        session_id: String,
    },
    Text {
        text: String,
    },
    ToolStarted {
        call: ToolCall,
    },
    ApprovalRequested {
        request: ApprovalRequest,
    },
    ApprovalResolved {
        id: String,
        approved: bool,
    },
    ToolFinished {
        id: String,
        output: String,
        is_error: bool,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    Compacted {
        through: usize,
    },
    TurnFinished,
    Error {
        message: String,
    },
}
pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    /// Specific action identity, e.g. native.shell or mcp.github.search.
    pub action: String,
    pub preview: String,
}
#[async_trait]
pub trait ApprovalPolicy: Send + Sync {
    async fn approve(&self, request: &ApprovalRequest, cancel: CancellationToken) -> Result<bool>;
}

#[derive(Clone)]
pub struct ToolContext {
    pub workspace: PathBuf,
    pub cancellation: CancellationToken,
}
pub struct PreparedAction {
    pub approval: Option<ApprovalRequest>,
    pub payload: Value,
}
#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn prepare(&self, call: &ToolCall, ctx: &ToolContext) -> Result<PreparedAction>;
    async fn execute(&self, action: PreparedAction, ctx: &ToolContext) -> Result<String>;
}
pub trait ToolRegistry: Send + Sync {
    fn definitions(&self) -> Vec<ToolDefinition>;
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub workspace: PathBuf,
    pub messages: Vec<Message>,
    pub events: Vec<Event>,
    pub summary: Option<String>,
    pub summary_through: usize,
    /// A persisted unfinished turn is recovered, never automatically replayed.
    pub interrupted: bool,
}
impl Session {
    pub fn new(workspace: PathBuf) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            workspace,
            messages: vec![],
            events: vec![],
            summary: None,
            summary_through: 0,
            interrupted: false,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub workspace: PathBuf,
    pub interrupted: bool,
}
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn save(&self, session: &Session) -> Result<()>;
    async fn load(&self, id: &str) -> Result<Session>;
    async fn list(&self) -> Result<Vec<SessionInfo>>;
}
#[async_trait]
pub trait AgentLoop: Send + Sync {
    async fn run(
        &self,
        session: &mut Session,
        prompt: String,
        approvals: &dyn ApprovalPolicy,
        events: &dyn EventSink,
        cancellation: CancellationToken,
    ) -> Result<()>;

    /// Compact completed conversation history when supported by this loop.
    async fn compact(
        &self,
        _session: &mut Session,
        _events: &dyn EventSink,
        _cancellation: CancellationToken,
    ) -> Result<()> {
        Err(AgentError::Model(
            "manual compaction is not supported by this agent loop".into(),
        ))
    }
}

// Wrappers let the composition kernel register trait objects through typed services.
pub struct ModelService(pub Arc<dyn ModelProvider>);
pub struct ToolsService(pub Arc<dyn ToolRegistry>);
pub struct SessionsService(pub Arc<dyn SessionStore>);
pub struct LoopService(pub Arc<dyn AgentLoop>);
