use thiserror::Error;

/// Error categories surfaced by every Muninn operation.
///
/// Categories are stable so transports (TCK bridge, MCP) can map them to
/// wire errors without inspecting messages.
#[derive(Debug, Error)]
pub enum MemoryError {
    /// The caller supplied an invalid argument.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// A referenced node does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The graph database is unreachable or rejected the operation.
    #[error("storage error: {0}")]
    Storage(String),
    /// An embedding or LLM provider failed.
    #[error("provider error: {0}")]
    Provider(String),
    /// The configured embedder does not match the persisted vector indexes.
    #[error("embedding dimension mismatch: {0}")]
    DimensionMismatch(String),
    /// The operation exists upstream but is not available in this backend.
    #[error("not supported: {0}")]
    NotSupported(String),
}

impl MemoryError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::Storage(message.into())
    }

    pub fn provider(message: impl Into<String>) -> Self {
        Self::Provider(message.into())
    }
}

pub type MemoryResult<T> = Result<T, MemoryError>;
