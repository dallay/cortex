// Integration tests for the logout response contract.
//
// The logout response must clear the `auth_token` cookie even when the
// anonymous CSRF token cannot be generated, so a failed RNG never leaves the
// client authenticated.

use axum::http::{header::SET_COOKIE, StatusCode};
use transport_axum::{handlers::auth::logout_response, CsrfGuard};

#[test]
fn logout_response_clears_auth_cookie_when_anonymous_token_is_unavailable() {
    let response = logout_response(None);

    assert_eq!(response.status(), StatusCode::OK);
    let cookies: Vec<_> = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    assert_eq!(cookies.len(), 1);
    assert!(cookies[0].starts_with("auth_token="));
    assert!(cookies[0].contains("Max-Age=0"));
    assert!(response.headers().get("x-csrf-token").is_none());
}

#[test]
fn logout_response_clears_auth_cookie_and_sets_anonymous_csrf_when_available() {
    let guard = CsrfGuard::new();
    let token = guard.generate_token().expect("anonymous CSRF token");
    let csrf_cookie = guard.build_cookie_header(&token, false);

    let response = logout_response(Some((token.clone(), csrf_cookie)));

    assert_eq!(response.status(), StatusCode::OK);
    let cookies: Vec<_> = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    assert_eq!(cookies.len(), 2);
    assert!(cookies[0].starts_with("auth_token="));
    assert!(cookies[0].contains("Max-Age=0"));
    assert!(cookies[1].starts_with("csrf_token="));
    assert_eq!(
        response
            .headers()
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok()),
        Some(token.as_str())
    );
}
