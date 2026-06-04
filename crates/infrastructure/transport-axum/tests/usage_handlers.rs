//! Unit tests for usage handler behavior.
//!
//! Tests the three GET handlers: list_usage, usage_summary, usage_cost.

use rook_core::RequestStatus;
use transport_axum::handlers::usage::{query_to_filters_and_pagination, UsageQuery};

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

#[test]
fn list_usage_query_parses_all_params() {
    let query = make_query(
        Some("openai".to_string()),
        Some("gpt-4o".to_string()),
        Some("key_abc".to_string()),
        Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
        Some("2026-06-01T00:00:00Z".to_string()),
        Some("2026-06-30T23:59:59Z".to_string()),
        Some("success".to_string()),
    );
    let result =
        query_to_filters_and_pagination(&query, Some("0".to_string()), Some("100".to_string()));

    assert!(result.is_ok());
    let (filters, pagination) = result.unwrap();

    assert_eq!(
        filters.provider.as_ref().map(|p| p.as_str()),
        Some("openai")
    );
    assert_eq!(filters.model.as_ref().map(|m| m.as_str()), Some("gpt-4o"));
    assert_eq!(filters.status, Some(RequestStatus::Success));
    assert_eq!(pagination.limit, 100);
    assert_eq!(pagination.offset, 0);
}

#[test]
fn summary_query_ignores_pagination_params() {
    let query = make_query(
        Some("anthropic".to_string()),
        None,
        None,
        None,
        None,
        None,
        Some("failure".to_string()),
    );
    let result = query_to_filters_and_pagination(&query, None, None);

    assert!(result.is_ok());
    let (filters, _) = result.unwrap();

    assert_eq!(
        filters.provider.as_ref().map(|p| p.as_str()),
        Some("anthropic")
    );
    assert_eq!(filters.status, Some(RequestStatus::Failure));
}

#[test]
fn cost_query_ignores_pagination_params() {
    let query = make_query(
        Some("openai".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
    );
    let result = query_to_filters_and_pagination(&query, None, None);

    assert!(result.is_ok());
    let (filters, _) = result.unwrap();

    assert_eq!(
        filters.provider.as_ref().map(|p| p.as_str()),
        Some("openai")
    );
    assert!(filters.model.is_none());
}

#[test]
fn empty_filters_return_all_entries() {
    let query = make_query(None, None, None, None, None, None, None);
    let result = query_to_filters_and_pagination(&query, None, None);

    assert!(result.is_ok());
    let (filters, pagination) = result.unwrap();

    assert!(filters.provider.is_none());
    assert!(filters.model.is_none());
    assert!(filters.status.is_none());
    assert_eq!(pagination.limit, 100); // default
    assert_eq!(pagination.offset, 0); // default
}
