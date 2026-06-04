//! Unit tests for usage handler query string mapping.
//!
//! Tests that query string parameters correctly deserialize into
//! `UsageFilters` and `Pagination`, with proper error handling for
//! invalid values and limit clamping.

use rook_core::{Pagination, RequestStatus};
use transport_axum::handlers::usage::{query_to_filters_and_pagination, UsageQuery};

// Helper to build UsageQuery from 7 filter fields (offset/limit passed separately)
fn make_query(
    provider: Option<String>,
    model: Option<String>,
    api_key_id: Option<String>,
    connection_id: Option<String>,
    start: Option<String>,
    end: Option<String>,
    status: Option<String>,
) -> UsageQuery {
    UsageQuery {
        provider,
        model,
        api_key_id,
        connection_id,
        start,
        end,
        status,
        offset: None,
        limit: None,
    }
}

// =============================================================================
// Query string → UsageFilters + Pagination mapping
// =============================================================================

#[test]
fn default_query_returns_empty_filters_and_default_pagination() {
    let query = make_query(None, None, None, None, None, None, None);
    let Ok((filters, pagination)) = query_to_filters_and_pagination(&query, None, None) else {
        panic!("expected Ok");
    };

    assert!(filters.provider.is_none());
    assert!(filters.model.is_none());
    assert!(filters.api_key_id.is_none());
    assert!(filters.connection_id.is_none());
    assert!(filters.start.is_none());
    assert!(filters.end.is_none());
    assert!(filters.status.is_none());

    assert_eq!(pagination.offset, 0);
    assert_eq!(pagination.limit, Pagination::DEFAULT_LIMIT);
}

#[test]
fn all_filter_params_set_correctly() {
    let query = make_query(
        Some("openai".to_string()),
        Some("gpt-4o".to_string()),
        Some("key_abc123".to_string()),
        Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
        Some("2026-06-01T00:00:00Z".to_string()),
        Some("2026-06-30T23:59:59Z".to_string()),
        Some("success".to_string()),
    );
    let Ok((filters, _)) =
        query_to_filters_and_pagination(&query, Some("50".to_string()), Some("200".to_string()))
    else {
        panic!("expected Ok");
    };

    assert_eq!(
        filters.provider.as_ref().map(|p| p.as_str()),
        Some("openai")
    );
    assert_eq!(filters.model.as_ref().map(|m| m.as_str()), Some("gpt-4o"));
    assert_eq!(
        filters.api_key_id.as_ref().map(|id| id.as_str()),
        Some("key_abc123")
    );
    assert_eq!(
        filters.connection_id.as_ref().map(|id| id.to_string()),
        Some("550e8400-e29b-41d4-a716-446655440000".to_string())
    );
    assert!(filters.start.is_some());
    assert!(filters.end.is_some());
    assert_eq!(filters.status, Some(RequestStatus::Success));
}

#[test]
fn limit_clamps_to_maximum() {
    let query = make_query(None, None, None, None, None, None, None);
    let Ok((_, pagination)) =
        query_to_filters_and_pagination(&query, None, Some("5000".to_string()))
    else {
        panic!("expected Ok");
    };

    assert_eq!(
        pagination.limit,
        Pagination::MAX_LIMIT,
        "limit should clamp to 1000, got {}",
        pagination.limit
    );
}

#[test]
fn explicit_limit_below_max_preserved() {
    let query = make_query(None, None, None, None, None, None, None);
    let Ok((_, pagination)) =
        query_to_filters_and_pagination(&query, None, Some("250".to_string()))
    else {
        panic!("expected Ok");
    };

    assert_eq!(pagination.limit, 250);
}

#[test]
fn offset_is_preserved() {
    let query = make_query(None, None, None, None, None, None, None);
    let Ok((_, pagination)) =
        query_to_filters_and_pagination(&query, Some("500".to_string()), Some("100".to_string()))
    else {
        panic!("expected Ok");
    };

    assert_eq!(pagination.offset, 500);
}

// =============================================================================
// Invalid value handling
// =============================================================================

#[test]
fn empty_connection_id_string_is_rejected() {
    let query = make_query(None, None, None, Some("".to_string()), None, None, None);
    let result = query_to_filters_and_pagination(&query, None, None);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, "INVALID_CONNECTION_ID");
}

#[test]
fn invalid_connection_id_format_returns_error() {
    let query = make_query(
        None,
        None,
        None,
        Some("not-a-valid-uuid".to_string()),
        None,
        None,
        None,
    );
    let result = query_to_filters_and_pagination(&query, None, None);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, "INVALID_CONNECTION_ID");
}

#[test]
fn invalid_status_returns_error() {
    let query = make_query(
        None,
        None,
        None,
        None,
        None,
        None,
        Some("invalid_status".to_string()),
    );
    let result = query_to_filters_and_pagination(&query, None, None);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, "INVALID_STATUS");
}

#[test]
fn invalid_date_format_returns_error() {
    let query = make_query(
        None,
        None,
        None,
        None,
        None,
        Some("not-a-date".to_string()),
        None,
    );
    let result = query_to_filters_and_pagination(&query, None, None);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, "INVALID_DATE");
}

#[test]
fn malformed_start_date_returns_invalid_date_error() {
    let query = make_query(
        None,
        None,
        None,
        None,
        Some("2026-13-45T99:99:99Z".to_string()),
        None,
        None,
    );
    let result = query_to_filters_and_pagination(&query, None, None);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, "INVALID_DATE");
}

// =============================================================================
// Pagination constants
// =============================================================================

#[test]
fn default_limit_is_100() {
    let query = make_query(None, None, None, None, None, None, None);
    let Ok((_, pagination)) = query_to_filters_and_pagination(&query, None, None) else {
        panic!("expected Ok");
    };

    assert_eq!(pagination.limit, 100);
    assert_eq!(pagination.limit, Pagination::DEFAULT_LIMIT);
}

#[test]
fn max_limit_is_1000() {
    let query = make_query(None, None, None, None, None, None, None);
    let Ok((_, pagination)) =
        query_to_filters_and_pagination(&query, None, Some("1000".to_string()))
    else {
        panic!("expected Ok");
    };

    assert_eq!(pagination.limit, 1000);
    assert_eq!(pagination.limit, Pagination::MAX_LIMIT);
}

#[test]
fn zero_offset_becomes_zero() {
    let query = make_query(None, None, None, None, None, None, None);
    let Ok((_, pagination)) =
        query_to_filters_and_pagination(&query, Some("0".to_string()), Some("0".to_string()))
    else {
        panic!("expected Ok");
    };

    assert_eq!(pagination.offset, 0);
}

// =============================================================================
// Partial query params
// =============================================================================

#[test]
fn only_provider_filter_set() {
    let query = make_query(
        Some("openai".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
    );
    let Ok((filters, pagination)) = query_to_filters_and_pagination(&query, None, None) else {
        panic!("expected Ok");
    };

    assert_eq!(
        filters.provider.as_ref().map(|p| p.as_str()),
        Some("openai")
    );
    assert!(filters.model.is_none());
    assert!(filters.api_key_id.is_none());
    assert!(filters.connection_id.is_none());
    assert!(filters.start.is_none());
    assert!(filters.end.is_none());
    assert!(filters.status.is_none());

    // pagination should still have defaults
    assert_eq!(pagination.offset, 0);
    assert_eq!(pagination.limit, 100);
}

#[test]
fn only_status_filter_set() {
    let query = make_query(
        None,
        None,
        None,
        None,
        None,
        None,
        Some("failure".to_string()),
    );
    let Ok((filters, pagination)) = query_to_filters_and_pagination(&query, None, None) else {
        panic!("expected Ok");
    };

    assert_eq!(filters.status, Some(RequestStatus::Failure));
    assert_eq!(pagination.limit, 100); // default
}

#[test]
fn only_offset_set() {
    let query = make_query(None, None, None, None, None, None, None);
    let Ok((_, pagination)) = query_to_filters_and_pagination(&query, Some("50".to_string()), None)
    else {
        panic!("expected Ok");
    };

    assert_eq!(pagination.offset, 50);
    assert_eq!(pagination.limit, 100); // default
}
