//! Product-owned contracts for Huginn, the Cortex coding agent. No terminal or Rook dependencies.
pub mod connections;
pub mod kernel;
pub mod providers;
pub use kernel::{ServiceHandle, ServiceLease, ServiceSlotHandle};

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
    /// The inference transport failed before a complete answer.
    ///
    /// Repeating the same request is safe because inference has no host effect. Effects
    /// are never retried: `Tool` and `Cancelled` end the turn instead.
    #[error("transient inference failure: {0}")]
    Inference(String),
    #[error("tool: {0}")]
    Tool(String),
    #[error("session: {0}")]
    Session(String),
    #[error("composition: {0}")]
    Composition(String),
    #[error(transparent)]
    Provider(#[from] providers::ProviderError),
    #[error(transparent)]
    Connection(#[from] connections::ConnectionError),
    #[error("operation cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, AgentError>;

impl AgentError {
    /// Whether repeating the *same request* is safe.
    ///
    /// Only inference qualifies, and only when it produced no host effect. This is the
    /// single place the loop asks that question, so a new variant cannot be retried by
    /// accident.
    pub const fn is_retryable_inference(&self) -> bool {
        matches!(self, Self::Inference(_))
    }
}

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
    /// Static adapters are configured by default; sentinels and routers report readiness.
    fn is_configured(&self) -> bool {
        true
    }
    /// Fix the inference identity for one entire turn, including compaction.
    /// Static providers keep their existing implementation. Routers return a
    /// revocable snapshot; callers must reuse it rather than pin each request.
    async fn pin_turn(&self, _cancel: CancellationToken) -> Result<Option<Arc<dyn ModelProvider>>> {
        Ok(None)
    }

    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>>;
}

/// Identity of one `AgentLoop::run` invocation within a session.
///
/// Zero is reserved for events persisted before this identity existed: it means
/// "unknown", never "the first turn". Allocation starts at one and is persisted
/// with the session so a resumed session never reuses a recorded identity.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TurnId(pub u64);

impl TurnId {
    /// Reserved for events persisted before turn identity existed.
    pub const UNKNOWN: Self = Self(0);
}

/// Identity of one model call inside a turn. Zero means unknown.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct StepId(pub u64);

impl StepId {
    /// Reserved for events persisted before step identity existed.
    pub const UNKNOWN: Self = Self(0);
}

/// Non-cryptographic provenance digest (FNV-1a, 64 bit, hex).
///
/// Used to state *which* context a turn was pinned to, so a later audit can tell two
/// turns apart. It is not a security boundary and cannot authenticate content.
pub fn fingerprint(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Steering messages are bounded and never grants: the host decides what to inject.
pub const MAX_STEERING_MESSAGES: usize = 8;
pub const MAX_STEERING_BYTES: usize = 4096;

/// Mailbox the host writes into while a turn is running.
///
/// The loop drains it at step boundaries, so an injected message reaches the model at a
/// point where no tool is mid-flight and no result is being assembled.
#[derive(Default)]
pub struct SteeringQueue {
    pending: std::sync::Mutex<std::collections::VecDeque<String>>,
}

impl SteeringQueue {
    /// Refuses rather than dropping or replacing: losing a steering message silently
    /// would let the model act on a partial instruction.
    pub fn push(&self, message: &str) -> Result<()> {
        let message = message.trim();
        if message.is_empty() {
            return Err(AgentError::Configuration(
                "a steering message cannot be empty".into(),
            ));
        }
        if message.len() > MAX_STEERING_BYTES {
            return Err(AgentError::Configuration(format!(
                "a steering message cannot exceed {MAX_STEERING_BYTES} bytes"
            )));
        }
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.len() >= MAX_STEERING_MESSAGES {
            return Err(AgentError::Configuration(format!(
                "at most {MAX_STEERING_MESSAGES} steering messages can wait"
            )));
        }
        pending.push_back(message.to_string());
        drop(pending);
        Ok(())
    }

    pub fn take(&self) -> Vec<String> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    ConnectionChanged {
        snapshot: providers::ConnectionSnapshot,
    },
    TurnStarted {
        session_id: String,
        /// Missing on records written before turn identity existed.
        #[serde(default)]
        turn: TurnId,
    },
    Text {
        /// Missing on records written before step identity existed.
        #[serde(default)]
        step: StepId,
        text: String,
    },
    ToolStarted {
        #[serde(default)]
        step: StepId,
        call: ToolCall,
    },
    ApprovalRequested {
        #[serde(default)]
        step: StepId,
        request: ApprovalRequest,
    },
    ApprovalResolved {
        #[serde(default)]
        step: StepId,
        id: String,
        approved: bool,
    },
    ToolFinished {
        #[serde(default)]
        step: StepId,
        id: String,
        output: String,
        is_error: bool,
    },
    Usage {
        #[serde(default)]
        step: StepId,
        input_tokens: u64,
        output_tokens: u64,
    },
    Compacted {
        through: usize,
    },
    /// A retryable inference failure was repeated; no effect had been taken.
    ModelRetried {
        turn: TurnId,
        step: StepId,
        attempt: u32,
    },
    /// The turn pinned its system context; later steps reuse exactly this.
    ContextPinned {
        turn: TurnId,
        bytes: usize,
        digest: String,
    },
    /// The host injected messages into a running turn; they are history now.
    Steered {
        turn: TurnId,
        step: StepId,
        count: usize,
    },
    /// A turn-level hook refused the model call. No request was made.
    InferenceBlocked {
        turn: TurnId,
        step: StepId,
        hook: String,
        reason: String,
    },
    TurnFinished {
        #[serde(default)]
        turn: TurnId,
    },
    Error {
        message: String,
    },
}
pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);

    fn emit_for(&self, _operation: OperationId, event: Event) {
        self.emit(event);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct OperationId(pub u64);

pub struct ScopedEventSink<'a> {
    pub sink: &'a dyn EventSink,
    pub operation: OperationId,
}

impl EventSink for ScopedEventSink<'_> {
    fn emit(&self, event: Event) {
        self.sink.emit_for(self.operation, event);
    }

    fn emit_for(&self, _operation: OperationId, event: Event) {
        self.sink.emit_for(self.operation, event);
    }
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
    /// Host-selected exact identities, valid only for the current turn.
    pub selected_skills: Vec<String>,
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
    /// Versioned catalog snapshot metadata, never skill bodies or permission grants.
    #[serde(default)]
    pub skill_catalog_fingerprint: Option<String>,
    pub id: String,
    pub workspace: PathBuf,
    pub messages: Vec<Message>,
    pub events: Vec<Event>,
    pub summary: Option<String>,
    pub summary_through: usize,
    /// A persisted unfinished turn is recovered, never automatically replayed.
    pub interrupted: bool,
    /// Monotonic turn allocator. Zero on sessions written before turn identity.
    #[serde(default)]
    pub turn_seq: u64,
}
impl Session {
    pub fn new(workspace: PathBuf) -> Self {
        Self {
            skill_catalog_fingerprint: None,
            id: uuid::Uuid::new_v4().to_string(),
            workspace,
            messages: vec![],
            events: vec![],
            summary: None,
            summary_through: 0,
            interrupted: false,
            turn_seq: 0,
        }
    }

    /// Allocates the next turn identity and persists the counter with the session.
    ///
    /// The counter, not the event log, is the allocator: a session whose events were
    /// truncated still cannot reuse an identity it already handed out.
    /// Fails instead of saturating: reusing one identity for every later turn would
    /// make two turns indistinguishable in an audit.
    pub fn allocate_turn(&mut self) -> Result<TurnId> {
        let next = self
            .turn_seq
            .checked_add(1)
            .ok_or_else(|| AgentError::Session("turn counter exhausted in this session".into()))?;
        self.turn_seq = next;
        Ok(TurnId(next))
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
    /// Host-owned discovery only. Entries are not plugin command contributions.
    async fn skill_commands(
        &self,
        _workspace: &std::path::Path,
        _cancellation: CancellationToken,
    ) -> Result<Vec<(String, String)>> {
        Ok(Vec::new())
    }

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

/// Stable service id for the replaceable agent loop.
///
/// It lives in the contract crate so a plugin outside the host application can name
/// the service it provides (ADR-0017).
pub fn agent_loop_id() -> kernel::ServiceId {
    kernel::ServiceId::new("agent:loop", 1)
}

/// Workspace context supplied by a replaceable adapter.
#[async_trait]
pub trait InstructionSource: Send + Sync {
    async fn load(&self, workspace: &std::path::Path) -> Result<String>;
}
pub struct InstructionsService(pub Arc<dyn InstructionSource>);

/// Presentation collects a fresh user decision; it receives no permission grants.
#[async_trait]
pub trait ApprovalPrompt: Send + Sync {
    fn is_available(&self) -> bool {
        true
    }
    async fn request_decision(
        &self,
        request: &ApprovalRequest,
        cancel: CancellationToken,
    ) -> Result<bool>;
    fn granted(&self, _request: &ApprovalRequest) {}
}
pub mod hooks;
pub mod tools;
