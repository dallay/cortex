// csrf_guard — double-submit cookie CSRF protection for MANAGEMENT routes
//
// Applies to state-changing methods (POST, PUT, DELETE) on MANAGEMENT routes.
// Uses double-submit cookie pattern: cookie value vs X-CSRF-Token header value.
// GET requests to MANAGEMENT routes do not need CSRF (they receive the token via cookie).

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::rand::{SecureRandom, SystemRandom};

/// CSRF validation result
#[derive(Debug, Clone, PartialEq)]
pub enum CsrfValidation {
    Valid,
    MissingCookie,
    MissingHeader,
    Mismatch,
}

/// CSRF guard for state-changing requests
///
/// Applied to POST, PUT, DELETE on MANAGEMENT routes.
/// Extracts csrf_token cookie and X-CSRF-Token header, compares them.
#[derive(Clone)]
pub struct CsrfGuard {
    rng: SystemRandom,
}

impl CsrfGuard {
    /// Create a new CSRF guard
    pub fn new() -> Self {
        Self {
            rng: SystemRandom::new(),
        }
    }

    /// Generate a new CSRF token (32 bytes, base64url encoded)
    pub fn generate_token(&self) -> Result<String, CsrfError> {
        let mut bytes = [0u8; 32];
        self.rng
            .fill(&mut bytes)
            .map_err(|_| CsrfError::GenerationFailed)?;
        Ok(URL_SAFE_NO_PAD.encode(bytes))
    }

    /// Validate cookie and header values using constant-time comparison
    pub fn validate(
        &self,
        cookie_value: Option<&str>,
        header_value: Option<&str>,
    ) -> CsrfValidation {
        let cookie = match cookie_value {
            Some(v) => v,
            None => return CsrfValidation::MissingCookie,
        };
        let header = match header_value {
            Some(v) => v,
            None => return CsrfValidation::MissingHeader,
        };

        // Decode both values and compare with constant-time comparison
        let Ok(cookie_decoded) = URL_SAFE_NO_PAD.decode(cookie) else {
            return CsrfValidation::Mismatch;
        };
        let Ok(header_decoded) = URL_SAFE_NO_PAD.decode(header) else {
            return CsrfValidation::Mismatch;
        };

        if cookie_decoded.len() != header_decoded.len() {
            return CsrfValidation::Mismatch;
        }

        // Constant-time comparison
        let mut diff = 0u8;
        for (c, h) in cookie_decoded.iter().zip(header_decoded.iter()) {
            diff |= c ^ h;
        }

        if diff == 0 {
            CsrfValidation::Valid
        } else {
            CsrfValidation::Mismatch
        }
    }

    /// Build the CSRF cookie header value
    pub fn build_cookie_header(&self, token: &str, secure: bool) -> HeaderValue {
        let mut cookie = format!(
            "csrf_token={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=86400",
            token
        );
        if secure {
            cookie.push_str("; Secure");
        }
        HeaderValue::from_str(&cookie).unwrap_or_else(|_| {
            HeaderValue::from_str("csrf_token=invalid; HttpOnly; Path=/").unwrap()
        })
    }
}

impl Default for CsrfGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// CSRF guard middleware function
pub async fn csrf_guard_middleware(
    State(guard): State<Arc<CsrfGuard>>,
    request: Request,
    next: Next,
) -> axum::response::Response {
    let method = request.method().clone();

    // Only apply to state-changing methods
    if !is_state_changing_method(&method) {
        return next.run(request).await;
    }

    // Skip CSRF for Client API routes (/v1/*) — these are machine-to-machine
    // APIs using API key authentication, not browser cookie sessions.
    let path = request.uri().path();
    if path.starts_with("/v1/") || path.starts_with("/v1") {
        return next.run(request).await;
    }

    // Extract cookie and header
    let cookie_value = extract_cookie(request.headers(), "csrf_token");
    let header_value = extract_header(&request, "x-csrf-token");

    // Validate
    let validation = guard.validate(cookie_value.as_deref(), header_value.as_deref());

    match validation {
        CsrfValidation::Valid => next.run(request).await,
        CsrfValidation::MissingCookie | CsrfValidation::MissingHeader => {
            csrf_error_response("csrf_missing", "CSRF token required")
        }
        CsrfValidation::Mismatch => csrf_error_response("csrf_mismatch", "CSRF token mismatch"),
    }
}

/// Check if the HTTP method is state-changing
fn is_state_changing_method(method: &Method) -> bool {
    matches!(
        method,
        &Method::POST | &Method::PUT | &Method::DELETE | &Method::PATCH
    )
}

/// Extract a cookie value from headers
fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(cookie_name, cookie_value)| {
            (cookie_name == name).then(|| cookie_value.to_string())
        })
}

/// Extract a header value (case-insensitive)
fn extract_header(request: &Request, name: &str) -> Option<String> {
    // Case-insensitive header lookup
    for (key, value) in request.headers() {
        if key.as_str().eq_ignore_ascii_case(name) {
            return value.to_str().ok().map(|s| s.to_string());
        }
    }
    None
}

/// Build a CSRF error response
fn csrf_error_response(code: &str, message: &str) -> axum::response::Response {
    let body = serde_json::json!({
        "error": code,
        "message": message,
    });
    (StatusCode::FORBIDDEN, Json(body)).into_response()
}

/// CSRF-related errors
#[derive(Debug, Clone, PartialEq)]
pub enum CsrfError {
    GenerationFailed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csrf_guard() -> CsrfGuard {
        CsrfGuard::new()
    }

    #[test]
    fn generate_token_produces_base64url_string() {
        let guard = csrf_guard();
        let token = guard.generate_token().expect("Should generate token");

        // Should be base64url encoded (32 bytes = ~43 chars base64)
        let decoded = URL_SAFE_NO_PAD
            .decode(&token)
            .expect("Should be valid base64");
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn generate_token_produces_different_tokens() {
        let guard = csrf_guard();
        let token1 = guard.generate_token().expect("Should generate token");
        let token2 = guard.generate_token().expect("Should generate token");

        assert_ne!(token1, token2, "Tokens should be unique");
    }

    #[test]
    fn validate_returns_valid_when_cookie_and_header_match() {
        let guard = csrf_guard();
        let token = guard.generate_token().expect("Should generate token");

        let result = guard.validate(Some(&token), Some(&token));
        assert_eq!(result, CsrfValidation::Valid);
    }

    #[test]
    fn validate_returns_missing_cookie_when_cookie_absent() {
        let guard = csrf_guard();
        let token = guard.generate_token().expect("Should generate token");

        let result = guard.validate(None, Some(&token));
        assert_eq!(result, CsrfValidation::MissingCookie);
    }

    #[test]
    fn validate_returns_missing_header_when_header_absent() {
        let guard = csrf_guard();
        let token = guard.generate_token().expect("Should generate token");

        let result = guard.validate(Some(&token), None);
        assert_eq!(result, CsrfValidation::MissingHeader);
    }

    #[test]
    fn validate_returns_mismatch_when_values_differ() {
        let guard = csrf_guard();
        let token1 = guard.generate_token().expect("Should generate token");
        let token2 = guard.generate_token().expect("Should generate token");

        // Ensure they're different
        assert_ne!(token1, token2);

        let result = guard.validate(Some(&token1), Some(&token2));
        assert_eq!(result, CsrfValidation::Mismatch);
    }

    #[test]
    fn validate_returns_mismatch_for_invalid_base64() {
        let guard = csrf_guard();

        let result = guard.validate(Some("not-valid-base64!!!"), Some("also-not-valid"));
        assert_eq!(result, CsrfValidation::Mismatch);
    }

    #[test]
    fn build_cookie_header_includes_http_only_and_same_site_strict() {
        let guard = csrf_guard();
        let token = guard.generate_token().expect("Should generate token");

        let header = guard.build_cookie_header(&token, false);
        let header_str = header.to_str().expect("Should be valid str");

        assert!(header_str.contains("HttpOnly"));
        assert!(header_str.contains("SameSite=Strict"));
        assert!(header_str.contains(&format!("csrf_token={}", token)));
    }

    #[test]
    fn build_cookie_header_includes_secure_flag_when_requested() {
        let guard = csrf_guard();
        let token = guard.generate_token().expect("Should generate token");

        let header = guard.build_cookie_header(&token, true);
        let header_str = header.to_str().expect("Should be valid str");

        assert!(header_str.contains("Secure"));
    }

    #[test]
    fn is_state_changing_method_detects_correct_methods() {
        assert!(is_state_changing_method(&Method::POST));
        assert!(is_state_changing_method(&Method::PUT));
        assert!(is_state_changing_method(&Method::DELETE));
        assert!(is_state_changing_method(&Method::PATCH));
        assert!(!is_state_changing_method(&Method::GET));
        assert!(!is_state_changing_method(&Method::HEAD));
        assert!(!is_state_changing_method(&Method::OPTIONS));
    }
}
