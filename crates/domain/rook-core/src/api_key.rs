use std::str::FromStr;

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use shared_kernel::{ModelId, ProviderId};
use smol_str::SmolStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApiKeyId(SmolStr);

impl ApiKeyId {
    pub fn new(value: impl Into<SmolStr>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ApiKeyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Canonical set of valid API key scopes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnownScope {
    ChatRead,
    ChatWrite,
    ProvidersRead,
    ProvidersWrite,
    Admin,
}

impl KnownScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatRead => "chat:read",
            Self::ChatWrite => "chat:write",
            Self::ProvidersRead => "providers:read",
            Self::ProvidersWrite => "providers:write",
            Self::Admin => "admin",
        }
    }
}

impl FromStr for KnownScope {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "chat:read" => Ok(Self::ChatRead),
            "chat:write" => Ok(Self::ChatWrite),
            "providers:read" => Ok(Self::ProvidersRead),
            "providers:write" => Ok(Self::ProvidersWrite),
            "admin" => Ok(Self::Admin),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKeyScope(SmolStr);

impl ApiKeyScope {
    /// Strict parse — only canonical scope values are accepted.
    /// Returns `Err(UnknownScope)` for anything outside the known set.
    pub fn parse(value: &str) -> Result<Self, ApiKeyValidationError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(ApiKeyValidationError::EmptyScope);
        }
        if value.parse::<KnownScope>().is_err() {
            return Err(ApiKeyValidationError::UnknownScope(value.to_string()));
        }
        Ok(Self(value.into()))
    }

    /// Lenient parse for reading from the database.
    /// Accepts any non-empty string and logs a warning for unknown scopes
    /// so existing DB rows are never rejected.
    pub fn parse_lenient(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty() {
            tracing::warn!(scope = "<empty>", "parse_lenient received empty scope");
        } else if value.parse::<KnownScope>().is_err() {
            tracing::warn!(scope = value, "unknown API key scope loaded from database");
        }
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApiKeyTier {
    Free,
    Pro,
    Enterprise,
}

impl ApiKeyTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Pro => "pro",
            Self::Enterprise => "enterprise",
        }
    }
}

impl FromStr for ApiKeyTier {
    type Err = ApiKeyValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "free" => Ok(Self::Free),
            "pro" => Ok(Self::Pro),
            "enterprise" => Ok(Self::Enterprise),
            _ => Err(ApiKeyValidationError::InvalidTier(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeySubject {
    pub id: ApiKeyId,
    pub label: String,
    pub scopes: Vec<ApiKeyScope>,
    pub tier: ApiKeyTier,
    /// Empty vec means unrestricted (all models allowed).
    pub allowed_models: Vec<ModelId>,
    /// Empty vec means unrestricted (all providers allowed).
    pub allowed_providers: Vec<ProviderId>,
    pub spending_budget: Option<SpendingBudget>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ApiKeyValidationError {
    #[error("API key scope must not be empty")]
    EmptyScope,
    #[error("unknown API key scope: {0}")]
    UnknownScope(String),
    #[error("invalid API key tier: {0}")]
    InvalidTier(String),
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ApiKeyRepositoryError {
    #[error("duplicate API key hash")]
    DuplicateHash,
    #[error("API key not found: {0}")]
    NotFound(ApiKeyId),
    #[error("database error: {0}")]
    Database(String),
}

/// Calendar window for a per-key spending budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpendingBudgetWindow {
    Daily,
    Monthly,
}

impl SpendingBudgetWindow {
    /// UTC calendar bounds of the window that contains `at`.
    ///
    /// The window starts at the beginning of the period and ends at the start of
    /// the next period, so consecutive windows never overlap.
    ///
    /// Returns `None` when the calendar cannot represent the window bounds for
    /// `at`. Callers must fail closed instead of assuming a valid window.
    pub fn utc_bounds(&self, at: DateTime<Utc>) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let date = at.date_naive();
        let start_date = match self {
            SpendingBudgetWindow::Daily => date,
            SpendingBudgetWindow::Monthly => date.with_day(1)?,
        };
        let start = start_date.and_hms_opt(0, 0, 0)?.and_utc();
        let end = match self {
            SpendingBudgetWindow::Daily => start.checked_add_signed(chrono::Duration::days(1))?,
            SpendingBudgetWindow::Monthly => {
                let (next_year, next_month) = if date.month() == 12 {
                    (date.year().checked_add(1)?, 1)
                } else {
                    (date.year(), date.month() + 1)
                };
                chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)?
                    .and_hms_opt(0, 0, 0)?
                    .and_utc()
            }
        };
        Some((start, end))
    }
}

/// Requested spending policy for API-key management. Monetary values are integer micro-USD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendingBudgetRequest {
    pub amount_micros_usd: u64,
    pub request_limit_micros_usd: u64,
    pub window: SpendingBudgetWindow,
}

/// Strict per-key spending policy. Monetary values are integer micro-USD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendingBudget {
    pub api_key_id: ApiKeyId,
    pub amount_micros_usd: u64,
    pub request_limit_micros_usd: u64,
    pub window: SpendingBudgetWindow,
}

/// Durable budget reservation for one request and one UTC calendar window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendingBudgetReservation {
    pub api_key_id: ApiKeyId,
    pub request_id: shared_kernel::RequestId,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub reserved_micros_usd: u64,
    pub budget_micros_usd: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendingReservationOutcome {
    /// This call created a new reservation and is the only outcome that grants admission.
    Reserved,
    /// The request ID already exists. A caller must not repeat upstream I/O.
    Existing,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SpendingBudgetReservationError {
    #[error("spending budget exceeded")]
    BudgetExceeded,
    #[error("request ID already belongs to a different reservation")]
    IdempotencyConflict,
    #[error("invalid spending reservation")]
    InvalidReservation,
    #[error("spending reservation was not found")]
    NotFound,
    #[error("settled cost exceeds the reserved maximum")]
    CostExceedsReservation,
    #[error("invalid spending reservation state transition")]
    InvalidTransition,
    #[error("spending reservation database error: {0}")]
    Database(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyRecord {
    pub id: ApiKeyId,
    pub label: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub scopes: Vec<ApiKeyScope>,
    pub tier: ApiKeyTier,
    pub is_active: bool,
    pub revoked_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    /// Empty vec means unrestricted (all models allowed).
    pub allowed_models: Vec<ModelId>,
    /// Empty vec means unrestricted (all providers allowed).
    pub allowed_providers: Vec<ProviderId>,
    pub spending_budget: Option<SpendingBudget>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_five_known_scopes_parse_ok() {
        assert!(ApiKeyScope::parse("chat:read").is_ok());
        assert!(ApiKeyScope::parse("chat:write").is_ok());
        assert!(ApiKeyScope::parse("providers:read").is_ok());
        assert!(ApiKeyScope::parse("providers:write").is_ok());
        assert!(ApiKeyScope::parse("admin").is_ok());
    }

    #[test]
    fn unknown_scope_returns_unknown_scope_error() {
        let err = ApiKeyScope::parse("read").unwrap_err();
        assert_eq!(err, ApiKeyValidationError::UnknownScope("read".to_string()));
    }

    #[test]
    fn empty_scope_returns_empty_scope_error() {
        let err = ApiKeyScope::parse("").unwrap_err();
        assert_eq!(err, ApiKeyValidationError::EmptyScope);

        let err_ws = ApiKeyScope::parse("   ").unwrap_err();
        assert_eq!(err_ws, ApiKeyValidationError::EmptyScope);
    }

    #[test]
    fn uppercase_scope_is_rejected() {
        // Scope matching is case-sensitive; uppercase variants are unknown.
        let err = ApiKeyScope::parse("Chat:Read").unwrap_err();
        assert!(
            matches!(err, ApiKeyValidationError::UnknownScope(_)),
            "expected UnknownScope, got {:?}",
            err
        );

        let err2 = ApiKeyScope::parse("ADMIN").unwrap_err();
        assert!(matches!(err2, ApiKeyValidationError::UnknownScope(_)));
    }

    #[test]
    fn parse_lenient_accepts_unknown_scope_without_error() {
        // Should not panic or return an error; just log a warning.
        let scope = ApiKeyScope::parse_lenient("legacy:custom");
        assert_eq!(scope.as_str(), "legacy:custom");
    }

    #[test]
    fn parse_lenient_accepts_known_scope() {
        let scope = ApiKeyScope::parse_lenient("chat:read");
        assert_eq!(scope.as_str(), "chat:read");
    }

    #[test]
    fn api_key_record_has_allowed_models_and_providers_fields() {
        use shared_kernel::{ModelId, ProviderId};
        let record = ApiKeyRecord {
            id: ApiKeyId::new("test-key"),
            label: "Test".to_string(),
            key_hash: "hash".to_string(),
            key_prefix: "prefix".to_string(),
            scopes: vec![],
            tier: ApiKeyTier::Free,
            is_active: true,
            revoked_at: None,
            expires_at: None,
            created_at: chrono::Utc::now(),
            last_used_at: None,
            allowed_models: vec![ModelId::new("gpt-4"), ModelId::new("claude-3")],
            allowed_providers: vec![ProviderId::new("openai")],
            spending_budget: None,
        };
        assert_eq!(record.allowed_models.len(), 2);
        assert_eq!(record.allowed_providers.len(), 1);
    }

    #[test]
    fn daily_window_covers_the_utc_calendar_day() {
        let at = DateTime::parse_from_rfc3339("2026-10-28T13:45:12Z")
            .unwrap()
            .with_timezone(&Utc);
        let (start, end) = SpendingBudgetWindow::Daily
            .utc_bounds(at)
            .expect("daily bounds");
        assert_eq!(
            start,
            DateTime::parse_from_rfc3339("2026-10-28T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
        assert_eq!(
            end,
            DateTime::parse_from_rfc3339("2026-10-29T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn monthly_window_covers_the_utc_calendar_month() {
        let at = DateTime::parse_from_rfc3339("2026-10-28T13:45:12Z")
            .unwrap()
            .with_timezone(&Utc);
        let (start, end) = SpendingBudgetWindow::Monthly
            .utc_bounds(at)
            .expect("monthly bounds");
        assert_eq!(
            start,
            DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
        assert_eq!(
            end,
            DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn monthly_window_rolls_over_the_year_boundary() {
        let at = DateTime::parse_from_rfc3339("2026-12-31T23:59:59Z")
            .unwrap()
            .with_timezone(&Utc);
        let (start, end) = SpendingBudgetWindow::Monthly
            .utc_bounds(at)
            .expect("monthly bounds");
        assert_eq!(
            start,
            DateTime::parse_from_rfc3339("2026-12-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
        assert_eq!(
            end,
            DateTime::parse_from_rfc3339("2027-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn window_bounds_are_unavailable_instead_of_panicking_at_calendar_limits() {
        let at = chrono::NaiveDate::MAX
            .and_hms_opt(23, 59, 59)
            .expect("the maximum calendar date has a valid time")
            .and_utc();
        assert!(SpendingBudgetWindow::Daily.utc_bounds(at).is_none());
        assert!(SpendingBudgetWindow::Monthly.utc_bounds(at).is_none());
    }

    #[test]
    fn api_key_record_empty_restrictions_means_unrestricted() {
        let record = ApiKeyRecord {
            id: ApiKeyId::new("test-key"),
            label: "Test".to_string(),
            key_hash: "hash".to_string(),
            key_prefix: "prefix".to_string(),
            scopes: vec![],
            tier: ApiKeyTier::Free,
            is_active: true,
            revoked_at: None,
            expires_at: None,
            created_at: chrono::Utc::now(),
            last_used_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
            spending_budget: None,
        };
        // Empty means unrestricted — both fields are present and empty
        assert!(record.allowed_models.is_empty());
        assert!(record.allowed_providers.is_empty());
    }
}
