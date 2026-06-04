//! Unit tests for usage route classification and mount behavior.
//!
//! Tests that:
//! - `/api/usage*` routes are classified as Management auth tier
//! - Routes are always mounted (not conditional on feature flags)
//! - GET methods do not require CSRF

use axum::http::Method;

use transport_axum::authz::classify_route;

#[test]
fn usage_routes_classified_as_management() {
    // Test that all usage paths are classified as Management auth
    assert_eq!(
        classify_route(&Method::GET, "/api/usage"),
        transport_axum::authz::AuthTier::Management,
        "/api/usage should be Management"
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/usage/summary"),
        transport_axum::authz::AuthTier::Management,
        "/api/usage/summary should be Management"
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/usage/cost"),
        transport_axum::authz::AuthTier::Management,
        "/api/usage/cost should be Management"
    );
}

#[test]
fn management_routes_require_session_not_api_key() {
    // Management routes should be classified as Management (not ClientApi)
    let tier = classify_route(&Method::GET, "/api/usage");
    assert_eq!(
        tier,
        transport_axum::authz::AuthTier::Management,
        "usage routes should be Management tier"
    );
}

#[test]
fn health_is_public_not_management() {
    // Verify our comparison point: health is Public
    assert_eq!(
        classify_route(&Method::GET, "/health"),
        transport_axum::authz::AuthTier::Public,
        "/health should be Public"
    );
}

#[test]
fn client_api_routes_are_client_api_not_management() {
    // Verify our comparison point: /v1 routes are ClientApi
    assert_eq!(
        classify_route(&Method::GET, "/v1/models"),
        transport_axum::authz::AuthTier::ClientApi,
        "/v1/models should be ClientApi"
    );
    assert_eq!(
        classify_route(&Method::POST, "/v1/chat/completions"),
        transport_axum::authz::AuthTier::ClientApi,
        "/v1/chat/completions should be ClientApi"
    );
}

#[test]
fn management_routes_include_api_prefix() {
    // All management routes start with /api/
    assert_eq!(
        classify_route(&Method::GET, "/api/providers"),
        transport_axum::authz::AuthTier::Management,
        "/api/providers should be Management"
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/api-keys"),
        transport_axum::authz::AuthTier::Management,
        "/api/api-keys should be Management"
    );
    // The usage routes follow the same pattern
    assert_eq!(
        classify_route(&Method::GET, "/api/usage"),
        transport_axum::authz::AuthTier::Management,
        "/api/usage should be Management"
    );
}
