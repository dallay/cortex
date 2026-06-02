// Error types for the shared kernel.
// Downstream crates extend these with their own variants via newtypes
// or by wrapping in their own error enum.

use std::fmt;

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct CortexError {
    #[from]
    source: Box<dyn std::error::Error + Send + Sync + 'static>,
}

impl CortexError {
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

    pub fn auth_failed(msg: impl Into<String>) -> Self {
        Self {
            source: Box::new(AuthFailedError(msg.into())),
        }
    }

    pub fn invalid_request(msg: impl Into<String>) -> Self {
        Self {
            source: Box::new(InvalidRequestError(msg.into())),
        }
    }

    pub fn is_auth_failed(&self) -> bool {
        self.source.is::<AuthFailedError>()
    }

    pub fn is_invalid_request(&self) -> bool {
        self.source.is::<InvalidRequestError>()
    }

    pub fn is_all_providers_exhausted(&self) -> bool {
        self.source.is::<AllProvidersExhaustedError>()
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self {
            source: Box::new(ForbiddenError(msg.into())),
        }
    }

    pub fn is_forbidden(&self) -> bool {
        self.source.is::<ForbiddenError>() || self.source.is::<RestrictionViolation>()
    }

    /// Error code for `MODEL_NOT_ALLOWED` and `PROVIDER_NOT_ALLOWED` so the
    /// HTTP layer can return a specific `code` to the client.
    pub fn forbidden_code(&self) -> Option<&'static str> {
        // Check for structured RestrictionViolation first
        if let Some(violation) = self.source.downcast_ref::<RestrictionViolation>() {
            return Some(match violation {
                RestrictionViolation::ModelNotAllowed(_) => "model_not_allowed",
                RestrictionViolation::ProviderNotAllowed(_) => "provider_not_allowed",
            });
        }
        // Fallback to legacy ForbiddenError message parsing for backwards compatibility
        if let Some(ForbiddenError(msg)) = self.source.downcast_ref::<ForbiddenError>() {
            if msg.starts_with("model ") {
                Some("model_not_allowed")
            } else if msg.starts_with("provider ") {
                Some("provider_not_allowed")
            } else {
                None
            }
        } else {
            None
        }
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

#[derive(Debug)]
pub struct AuthFailedError(pub String);

impl fmt::Display for AuthFailedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "authentication failed: {}", self.0)
    }
}

impl std::error::Error for AuthFailedError {}

#[derive(Debug)]
pub struct InvalidRequestError(pub String);

impl fmt::Display for InvalidRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid request: {}", self.0)
    }
}

impl std::error::Error for InvalidRequestError {}

#[derive(Debug)]
pub struct ForbiddenError(pub String);

impl fmt::Display for ForbiddenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "forbidden: {}", self.0)
    }
}

impl std::error::Error for ForbiddenError {}

// ---------------------------------------------------------------------------
// Restriction Violation Errors
// ---------------------------------------------------------------------------

/// Structured error for API key restriction violations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RestrictionViolation {
    #[error("model '{0}' is not permitted by this API key")]
    ModelNotAllowed(super::ModelId),
    #[error("provider '{0}' is not permitted by this API key")]
    ProviderNotAllowed(super::ProviderId),
}

impl From<RestrictionViolation> for CortexError {
    fn from(violation: RestrictionViolation) -> Self {
        Self {
            source: Box::new(violation),
        }
    }
}

// ---------------------------------------------------------------------------
// Result type alias
// ---------------------------------------------------------------------------

pub type CortexResult<T> = Result<T, CortexError>;
