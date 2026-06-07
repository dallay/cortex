//! HTTP status code classification for credential validation probes.
//!
//! The `ProbeClassification` enum and the [`classify_status_code`] helper
//! translate an upstream HTTP status code (and the textual reason for
//! network-level failures) into a domain-level signal that providers and
//! the `from_health` mapper in `rook-usecases` consume.
//!
//! This module lives in `rook-core` (zero deps) and takes a `u16` rather
//! than `reqwest::StatusCode` so the domain crate stays free of network
//! dependencies. Provider crates convert `StatusCode::as_u16()` at the
//! call site.

/// Domain-level classification of an upstream HTTP response from a
/// credential validation probe.
///
/// Providers do not return this directly — they use it to decide which
/// [`crate::HealthStatus`] variant to construct. The classification is
/// stable and serializable so that the same enum can be reused by tests
/// that exercise the `classify_status_code` helper without going through
/// the network stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeClassification {
    /// 2xx — credentials accepted, server is healthy.
    Ok,
    /// 429 — credentials valid but the upstream is rate-limiting the
    /// caller (e.g. weekly quota exhausted). This is a *warning* in
    /// the test-connection UX, not a failure.
    RateLimited,
    /// 401 or 403 — credentials rejected. Hard failure.
    AuthRejected(u16),
    /// 5xx — upstream server error. Hard failure.
    ServerError(u16),
    /// Other 4xx — treated as a generic hard failure (the request
    /// shape is wrong or the resource does not exist).
    ClientError(u16),
    /// Network/transport error: DNS failure, connection refused,
    /// timeout, TLS error, etc. The textual reason is preserved for
    /// operator visibility.
    NetworkError(String),
}

/// Classify an HTTP status code into a [`ProbeClassification`].
///
/// The mapping follows the spec's §10 acceptance criteria for the
/// credential-validation-warning change:
///
/// | Status | Classification     | UI surface |
/// |--------|--------------------|------------|
/// | 2xx    | `Ok`               | green      |
/// | 429    | `RateLimited`      | yellow     |
/// | 401/403| `AuthRejected`     | red        |
/// | 5xx    | `ServerError`      | red        |
/// | other 4xx | `ClientError`   | red        |
///
/// Callers that observe a transport-layer error (no response at all)
/// should construct [`ProbeClassification::NetworkError`] directly
/// rather than calling this helper with `0`.
pub fn classify_status_code(status_code: u16) -> ProbeClassification {
    match status_code {
        200..=299 => ProbeClassification::Ok,
        429 => ProbeClassification::RateLimited,
        401 | 403 => ProbeClassification::AuthRejected(status_code),
        500..=599 => ProbeClassification::ServerError(status_code),
        400..=499 => ProbeClassification::ClientError(status_code),
        // Any other (1xx, 3xx that escaped the redirect-following,
        // or unknown future codes) is conservatively treated as a
        // server error so it lands in the red bucket rather than
        // being silently passed as a warning.
        _ => ProbeClassification::ServerError(status_code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
