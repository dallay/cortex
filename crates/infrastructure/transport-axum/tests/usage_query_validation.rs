//! Integration tests for usage query parsing and filter mapping.

use axum::http::StatusCode;
use rook_core::{Pagination, RequestStatus};
use transport_axum::handlers::usage::{query_to_filters_and_pagination, UsageQuery};

fn query() -> UsageQuery {
    UsageQuery {
        provider: None,
        model: None,
        api_key_id: None,
        connection_id: None,
        start: None,
        end: None,
        status: None,
        offset: None,
        limit: None,
    }
}

fn with_status(value: &str) -> UsageQuery {
    UsageQuery {
        status: Some(value.to_string()),
        ..query()
    }
}

#[test]
fn empty_query_maps_to_unrestricted_filters_and_default_pagination() {
    let (filters, pagination) =
        query_to_filters_and_pagination(&query(), None, None).expect("an empty query must map");
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
fn every_documented_status_value_is_accepted() {
    for (value, expected) in [
        ("success", RequestStatus::Success),
        ("failure", RequestStatus::Failure),
        ("rate_limited", RequestStatus::RateLimited),
        ("timeout", RequestStatus::Timeout),
    ] {
        let (filters, _) = query_to_filters_and_pagination(&with_status(value), None, None)
            .unwrap_or_else(|error| panic!("{value} must be accepted: {error:?}"));
        assert_eq!(filters.status, Some(expected));
    }
}

#[test]
fn unknown_status_is_rejected_with_the_documented_message() {
    let error = query_to_filters_and_pagination(&with_status("cancelled"), None, None)
        .expect_err("an unknown status must be rejected");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert_eq!(error.code, "INVALID_STATUS");
    assert_eq!(
        error.message,
        "Invalid status: must be one of success, failure, rate_limited, timeout"
    );
}

#[test]
fn invalid_start_date_names_the_start_field() {
    let query = UsageQuery {
        start: Some("not-a-date".to_string()),
        ..query()
    };
    let error = query_to_filters_and_pagination(&query, None, None)
        .expect_err("an invalid start date must be rejected");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert_eq!(error.code, "INVALID_DATE");
    assert_eq!(
        error.message,
        "Invalid start date format (use RFC 3339, e.g. 2026-06-01T00:00:00Z)"
    );
}

#[test]
fn invalid_end_date_names_the_end_field() {
    let query = UsageQuery {
        end: Some("not-a-date".to_string()),
        ..query()
    };
    let error = query_to_filters_and_pagination(&query, None, None)
        .expect_err("an invalid end date must be rejected");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert_eq!(error.code, "INVALID_DATE");
    assert_eq!(
        error.message,
        "Invalid end date format (use RFC 3339, e.g. 2026-03-30T23:59:59Z)"
    );
}

#[test]
fn inverted_date_range_is_rejected() {
    let query = UsageQuery {
        start: Some("2026-06-02T00:00:00Z".to_string()),
        end: Some("2026-06-01T00:00:00Z".to_string()),
        ..query()
    };
    let error = query_to_filters_and_pagination(&query, None, None)
        .expect_err("an inverted date range must be rejected");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert_eq!(error.code, "INVALID_DATE_RANGE");
    assert_eq!(error.message, "start must be before or equal to end");
}

#[test]
fn equal_start_and_end_is_accepted() {
    let query = UsageQuery {
        start: Some("2026-06-01T00:00:00Z".to_string()),
        end: Some("2026-06-01T00:00:00Z".to_string()),
        ..query()
    };
    let (filters, _) = query_to_filters_and_pagination(&query, None, None)
        .expect("an equal date range must be accepted");
    assert!(filters.start.is_some());
    assert!(filters.end.is_some());
}

#[test]
fn invalid_connection_id_is_rejected() {
    let query = UsageQuery {
        connection_id: Some("not-a-uuid".to_string()),
        ..query()
    };
    let error = query_to_filters_and_pagination(&query, None, None)
        .expect_err("an invalid connection id must be rejected");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert_eq!(error.code, "INVALID_CONNECTION_ID");
    assert_eq!(
        error.message,
        "Invalid connection ID format (expected UUID)"
    );
}

#[test]
fn invalid_offset_is_rejected() {
    let error = query_to_filters_and_pagination(&query(), Some("abc".to_string()), None)
        .expect_err("a non-numeric offset must be rejected");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert_eq!(error.code, "INVALID_OFFSET");
    assert_eq!(error.message, "offset must be a non-negative integer");
}

#[test]
fn negative_offset_is_rejected() {
    let error = query_to_filters_and_pagination(&query(), Some("-1".to_string()), None)
        .expect_err("a negative offset must be rejected");
    assert_eq!(error.code, "INVALID_OFFSET");
}

#[test]
fn invalid_limit_is_rejected() {
    let error = query_to_filters_and_pagination(&query(), None, Some("abc".to_string()))
        .expect_err("a non-numeric limit must be rejected");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert_eq!(error.code, "INVALID_LIMIT");
    assert_eq!(error.message, "limit must be a non-negative integer");
}

#[test]
fn limit_above_the_maximum_is_clamped() {
    let (_, pagination) = query_to_filters_and_pagination(&query(), None, Some("5000".to_string()))
        .expect("an oversized limit must be clamped, not rejected");
    assert_eq!(pagination.limit, Pagination::MAX_LIMIT);
}

#[test]
fn explicit_offset_and_limit_are_preserved() {
    let (_, pagination) =
        query_to_filters_and_pagination(&query(), Some("250".to_string()), Some("10".to_string()))
            .expect("valid pagination must be preserved");
    assert_eq!(pagination.offset, 250);
    assert_eq!(pagination.limit, 10);
}

#[test]
fn identity_filters_are_carried_over_verbatim() {
    let query = UsageQuery {
        provider: Some("openai".to_string()),
        model: Some("gpt-4o".to_string()),
        api_key_id: Some("key_abc123".to_string()),
        ..query()
    };
    let (filters, _) =
        query_to_filters_and_pagination(&query, None, None).expect("identity filters must map");
    assert_eq!(
        filters.provider.as_ref().map(|p| p.as_str()),
        Some("openai")
    );
    assert_eq!(filters.model.as_ref().map(|m| m.as_str()), Some("gpt-4o"));
    assert_eq!(
        filters.api_key_id.as_ref().map(|id| id.as_str()),
        Some("key_abc123")
    );
}
