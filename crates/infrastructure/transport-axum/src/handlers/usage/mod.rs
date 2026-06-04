// usage — HTTP handlers for GET /api/usage, /api/usage/summary, /api/usage/cost

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use rook_core::{
    ApiKeyId, ConnectionId, CostBreakdown, ModelId, Pagination, ProviderId, RequestStatus,
    UsageEntry, UsageFilters, UsageSummary,
};
use serde::Deserialize;

use crate::HttpError;

type Usecases = Arc<rook_usecases::RookUsecases>;

// -----------------------------------------------------------------------------
// Query string DTOs
// -----------------------------------------------------------------------------

/// Query string parameters for list and summary endpoints.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQuery {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key_id: Option<String>,
    pub connection_id: Option<String>,
    pub start: Option<String>, // RFC 3339 DateTime
    pub end: Option<String>,   // RFC 3339 DateTime
    pub status: Option<String>,
    pub offset: Option<String>, // passed as string in URL, converted to u64
    pub limit: Option<String>,  // passed as string in URL, converted to u64
}

// -----------------------------------------------------------------------------
// Response DTOs
// -----------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageListResponse {
    pub entries: Vec<UsageEntry>,
    pub total: u64,
}

// -----------------------------------------------------------------------------
// Query → Filters + Pagination mapping (pure, testable)
// -----------------------------------------------------------------------------

/// Maps raw query parameters to `UsageFilters` and `Pagination`.
/// Returns `Err(HttpError)` if any value cannot be parsed.
pub fn query_to_filters_and_pagination(
    query: &UsageQuery,
    offset: Option<String>,
    limit: Option<String>,
) -> Result<(UsageFilters, Pagination), HttpError> {
    let filters = UsageFilters {
        // ProviderId::new is infallible (SmolStr under the hood)
        provider: query.provider.clone().map(ProviderId::new),
        // ModelId::new is infallible (SmolStr under the hood)
        model: query.model.clone().map(ModelId::new),
        // ApiKeyId::new is infallible (SmolStr under the hood)
        api_key_id: query.api_key_id.clone().map(ApiKeyId::new),
        // ConnectionId::parse_str can fail on invalid UUID
        connection_id: query
            .connection_id
            .as_ref()
            .map(|s| {
                ConnectionId::parse_str(s).map_err(|_| HttpError {
                    status: axum::http::StatusCode::BAD_REQUEST,
                    code: "INVALID_CONNECTION_ID",
                    message: "Invalid connection ID format (expected UUID)".to_string(),
                })
            })
            .transpose()?,
        // start/end: parse RFC 3339 datetime
        start: query
            .start
            .as_ref()
            .map(|s| DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc)))
            .transpose()
            .map_err(|_| HttpError {
                status: axum::http::StatusCode::BAD_REQUEST,
                code: "INVALID_DATE",
                message: "Invalid start date format (use RFC 3339, e.g. 2026-06-01T00:00:00Z)"
                    .to_string(),
            })?,
        end: query
            .end
            .as_ref()
            .map(|s| DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc)))
            .transpose()
            .map_err(|_| HttpError {
                status: axum::http::StatusCode::BAD_REQUEST,
                code: "INVALID_DATE",
                message: "Invalid end date format (use RFC 3339, e.g. 2026-30T23:59:59Z)"
                    .to_string(),
            })?,
        // status: parse via serde, accepting snake_case
        status: query
            .status
            .as_ref()
            .map(|s| serde_json::from_str::<RequestStatus>(&format!("\"{s}\"")))
            .transpose()
            .map_err(|_| HttpError {
                status: axum::http::StatusCode::BAD_REQUEST,
                code: "INVALID_STATUS",
                message: "Invalid status: must be one of success, failure, rate_limited, timeout"
                    .to_string(),
            })?,
    };

    let pagination = Pagination {
        offset: offset
            .as_ref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0),
        limit: limit
            .as_ref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(Pagination::DEFAULT_LIMIT),
    }
    .clamped();

    Ok((filters, pagination))
}

// -----------------------------------------------------------------------------
// Handlers
// -----------------------------------------------------------------------------

/// GET /api/usage — paginated list of usage entries
pub async fn list_usage(
    State(usecases): State<Usecases>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<UsageListResponse>, HttpError> {
    let usage_recorder = usecases.usage_recorder.as_ref().ok_or_else(|| HttpError {
        status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
        code: "USAGE_RECORDER_UNAVAILABLE",
        message: "Usage recording is not available".to_string(),
    })?;

    let (filters, pagination) =
        query_to_filters_and_pagination(&query, query.offset.clone(), query.limit.clone())?;

    let entries = usage_recorder.list(filters.clone(), pagination).await?;
    let total = usage_recorder.count(filters).await?;

    Ok(Json(UsageListResponse { entries, total }))
}

/// GET /api/usage/summary — aggregated usage statistics
pub async fn usage_summary(
    State(usecases): State<Usecases>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<UsageSummary>, HttpError> {
    let usage_recorder = usecases.usage_recorder.as_ref().ok_or_else(|| HttpError {
        status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
        code: "USAGE_RECORDER_UNAVAILABLE",
        message: "Usage recording is not available".to_string(),
    })?;

    let (filters, _) = query_to_filters_and_pagination(&query, None, None)?;

    let summary = usage_recorder.summary(filters).await?;

    Ok(Json(summary))
}

/// GET /api/usage/cost — cost breakdown by provider, model, and API key
pub async fn usage_cost(
    State(usecases): State<Usecases>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<CostBreakdown>, HttpError> {
    let usage_recorder = usecases.usage_recorder.as_ref().ok_or_else(|| HttpError {
        status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
        code: "USAGE_RECORDER_UNAVAILABLE",
        message: "Usage recording is not available".to_string(),
    })?;

    let (filters, _) = query_to_filters_and_pagination(&query, None, None)?;

    let breakdown = usage_recorder.cost_breakdown(filters).await?;

    Ok(Json(breakdown))
}
