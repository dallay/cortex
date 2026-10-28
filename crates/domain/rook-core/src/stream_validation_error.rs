//! Uso observado en un frame rechazado por la validación del proveedor.

use crate::{CortexError, TokenUsage};

/// Conserva el uso sin convertir un fallo upstream en un chunk válido.
/// Solo envuelve errores de validación del proveedor, que siguen siendo reintentables.
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
