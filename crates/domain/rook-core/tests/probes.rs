//! Contract coverage for the public probe classification helper.
//!
//! The transport-error mapping (`ProbeClassification::NetworkError`) is
//! produced by the provider adapters, not by this crate: `classify_status_code`
//! only ever maps HTTP status codes. The boundary this crate owns is therefore
//! "a status code never yields a network error".

use rook_core::probes::{classify_status_code, ProbeClassification};

#[test]
fn classify_2xx_as_ok() {
    assert_eq!(classify_status_code(200), ProbeClassification::Ok);
    assert_eq!(classify_status_code(201), ProbeClassification::Ok);
    assert_eq!(classify_status_code(204), ProbeClassification::Ok);
    assert_eq!(classify_status_code(299), ProbeClassification::Ok);
}

#[test]
fn classify_3xx_as_server_error_outside_known_buckets() {
    // 3xx are not part of the documented classification buckets
    // (callers should follow redirects before reaching this helper).
    // We conservatively bucket them as server errors so they fall
    // into the red bucket rather than being passed as warnings.
    assert_eq!(
        classify_status_code(301),
        ProbeClassification::ServerError(301)
    );
    assert_eq!(
        classify_status_code(302),
        ProbeClassification::ServerError(302)
    );
    assert_eq!(
        classify_status_code(308),
        ProbeClassification::ServerError(308)
    );
}

#[test]
fn classify_400_as_client_error() {
    assert_eq!(
        classify_status_code(400),
        ProbeClassification::ClientError(400)
    );
    assert_eq!(
        classify_status_code(404),
        ProbeClassification::ClientError(404)
    );
}

#[test]
fn classify_401_and_403_as_auth_rejected() {
    assert_eq!(
        classify_status_code(401),
        ProbeClassification::AuthRejected(401)
    );
    assert_eq!(
        classify_status_code(403),
        ProbeClassification::AuthRejected(403)
    );
}

#[test]
fn classify_429_as_rate_limited() {
    assert_eq!(classify_status_code(429), ProbeClassification::RateLimited);
}

#[test]
fn classify_5xx_as_server_error() {
    assert_eq!(
        classify_status_code(500),
        ProbeClassification::ServerError(500)
    );
    assert_eq!(
        classify_status_code(502),
        ProbeClassification::ServerError(502)
    );
    assert_eq!(
        classify_status_code(503),
        ProbeClassification::ServerError(503)
    );
    assert_eq!(
        classify_status_code(504),
        ProbeClassification::ServerError(504)
    );
}

#[test]
fn classify_999_as_server_error_unknown_code() {
    // Unknown / future status codes are conservatively server errors
    // so they don't silently pass as warnings.
    assert_eq!(
        classify_status_code(999),
        ProbeClassification::ServerError(999)
    );
}

#[test]
fn classify_0_as_server_error() {
    // StatusCode::as_u16() never returns 0, but guard anyway.
    assert_eq!(classify_status_code(0), ProbeClassification::ServerError(0));
}

#[test]
fn classify_1xx_as_server_error() {
    // Informational responses never reach the helper in practice, and they
    // are not part of any documented bucket.
    assert_eq!(
        classify_status_code(100),
        ProbeClassification::ServerError(100)
    );
    assert_eq!(
        classify_status_code(199),
        ProbeClassification::ServerError(199)
    );
}

#[test]
fn classify_bucket_boundaries_are_inclusive() {
    assert_eq!(classify_status_code(200), ProbeClassification::Ok);
    assert_eq!(classify_status_code(299), ProbeClassification::Ok);
    assert_eq!(
        classify_status_code(300),
        ProbeClassification::ServerError(300)
    );
    assert_eq!(
        classify_status_code(399),
        ProbeClassification::ServerError(399)
    );
    assert_eq!(
        classify_status_code(400),
        ProbeClassification::ClientError(400)
    );
    assert_eq!(
        classify_status_code(499),
        ProbeClassification::ClientError(499)
    );
    assert_eq!(
        classify_status_code(500),
        ProbeClassification::ServerError(500)
    );
    assert_eq!(
        classify_status_code(599),
        ProbeClassification::ServerError(599)
    );
    assert_eq!(
        classify_status_code(600),
        ProbeClassification::ServerError(600)
    );
}

#[test]
fn auth_rejection_is_limited_to_401_and_403() {
    assert_eq!(
        classify_status_code(400),
        ProbeClassification::ClientError(400)
    );
    assert_eq!(
        classify_status_code(402),
        ProbeClassification::ClientError(402)
    );
    assert_eq!(
        classify_status_code(404),
        ProbeClassification::ClientError(404)
    );
}

#[test]
fn status_classification_never_reports_a_network_error() {
    // A transport failure has no HTTP status code; it is reported by the
    // provider adapters as `ProbeClassification::NetworkError`. The status
    // mapping must never produce that variant.
    for status in [
        0u16, 100, 199, 200, 299, 300, 399, 400, 401, 402, 403, 404, 429, 499, 500, 599, 600, 999,
    ] {
        assert!(
            !matches!(
                classify_status_code(status),
                ProbeClassification::NetworkError(_)
            ),
            "status {status} must not be classified as a network error"
        );
    }
}
