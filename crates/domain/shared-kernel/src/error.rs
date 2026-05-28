// Error types for the shared kernel.
// Downstream crates extend these with their own variants via newtypes
// or by wrapping in their own error enum.

use std::fmt;

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct NuxaError {
    #[from]
    source: Box<dyn std::error::Error + Send + Sync + 'static>,
}

impl NuxaError {
    pub fn provider(msg: impl Into<String>) -> Self {
        Self {
            source: Box::new(ProviderError(msg.into())),
        }
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            source: Box::new(NotFoundError(msg.into())),
        }
    }

    pub fn rate_limited(provider: super::ProviderId, retry_after_secs: u64) -> Self {
        Self {
            source: Box::new(RateLimitedError {
                provider: provider.0.to_string(),
                retry_after_secs,
            }),
        }
    }

    pub fn all_providers_exhausted() -> Self {
        Self {
            source: Box::new(AllProvidersExhaustedError),
        }
    }

    pub fn is_all_providers_exhausted(&self) -> bool {
        self.source.is::<AllProvidersExhaustedError>()
    }

    pub fn is_rate_limited(&self) -> bool {
        self.source.is::<RateLimitedError>()
    }

    pub fn retry_after_secs(&self) -> Option<u64> {
        self.source
            .downcast_ref::<RateLimitedError>()
            .map(|e| e.retry_after_secs)
    }
}

#[derive(Debug)]
pub struct ProviderError(pub String);

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "provider error: {}", self.0)
    }
}

impl std::error::Error for ProviderError {}

#[derive(Debug)]
pub struct NotFoundError(pub String);

impl fmt::Display for NotFoundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not found: {}", self.0)
    }
}

impl std::error::Error for NotFoundError {}

#[derive(Debug)]
pub struct RateLimitedError {
    pub provider: String,
    pub retry_after_secs: u64,
}

impl fmt::Display for RateLimitedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rate limited by {}, retry after {}s",
            self.provider, self.retry_after_secs
        )
    }
}

impl std::error::Error for RateLimitedError {}

#[derive(Debug)]
pub struct AllProvidersExhaustedError;

impl fmt::Display for AllProvidersExhaustedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "all providers exhausted")
    }
}

impl std::error::Error for AllProvidersExhaustedError {}

// ---------------------------------------------------------------------------
// Result type alias
// ---------------------------------------------------------------------------

pub type NuxaResult<T> = Result<T, NuxaError>;
