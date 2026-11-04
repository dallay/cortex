//! Observed usage in a frame rejected by provider validation.

use crate::{CortexError, TokenUsage};

/// Preserves usage without turning an upstream failure into a valid chunk.
/// Only wraps provider validation errors, which remain retryable.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct StreamValidationError {
    #[source]
    error: CortexError,
    pub usage: TokenUsage,
}

impl StreamValidationError {
    pub fn attach(error: CortexError, usage: Option<TokenUsage>) -> CortexError {
        match usage {
            Some(usage) => CortexError::from(
                Box::new(Self { error, usage }) as Box<dyn std::error::Error + Send + Sync>
            ),
            None => error,
        }
    }

    pub fn usage(error: &CortexError) -> Option<&TokenUsage> {
        error.downcast_ref::<Self>().map(|error| &error.usage)
    }
}
