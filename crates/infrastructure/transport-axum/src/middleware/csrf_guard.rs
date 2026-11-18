// csrf_guard — double-submit cookie CSRF protection for MANAGEMENT routes
//
// Applies to state-changing methods (POST, PUT, DELETE) on MANAGEMENT routes.
// Uses double-submit cookie pattern: cookie value vs X-CSRF-Token header value.
// GET requests to MANAGEMENT routes do not need CSRF (they receive the token via cookie).

use parking_lot::Mutex;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

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
    #[cfg(test)]
    fail_generation: bool,
    tokens: Arc<Mutex<HashMap<Vec<u8>, Instant>>>,
}

impl CsrfGuard {
    /// Create a new CSRF guard
    pub fn new() -> Self {
        Self {
            rng: SystemRandom::new(),
            #[cfg(test)]
            fail_generation: false,
            tokens: issued_tokens(),
        }
    }

    /// Generate a new CSRF token (32 bytes, base64url encoded)
    pub fn generate_token(&self) -> Result<String, CsrfError> {
        #[cfg(test)]
        if self.fail_generation {
            return Err(CsrfError::GenerationFailed);
        }
        let mut bytes = [0u8; 32];
        self.rng
            .fill(&mut bytes)
            .map_err(|_| CsrfError::GenerationFailed)?;
        let mut tokens = self.tokens.lock();
        let now = Instant::now();
        // Evict an unused token at capacity; its client can fetch a fresh token.
        // Consumed tokens are removed, so eviction can never permit a replay.
        if tokens.len() >= 100_000 {
            if let Some(key) = tokens.keys().next().cloned() {
                tokens.remove(&key);
            }
        }
        tokens.insert(bytes.to_vec(), now);
        Ok(URL_SAFE_NO_PAD.encode(bytes))
    }

    /// Validate cookie and header values using constant-time comparison, then
    /// atomically consume the issued, unexpired token.
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

        if cookie_decoded.len() != 32 || cookie_decoded.len() != header_decoded.len() {
            return CsrfValidation::Mismatch;
        }

        // Constant-time comparison
        let mut diff = 0u8;
        for (c, h) in cookie_decoded.iter().zip(header_decoded.iter()) {
            diff |= c ^ h;
        }

        if diff == 0 {
            // Atomically remove the issued token so concurrent replays cannot both pass.
            let issued = self.tokens.lock().remove(&cookie_decoded);
            if !issued.is_some_and(|issued| issued.elapsed() < Duration::from_secs(86400)) {
                return CsrfValidation::Mismatch;
            }
            CsrfValidation::Valid
        } else {
            CsrfValidation::Mismatch
        }
    }

    /// Build the CSRF cookie header value.
    ///
    /// Note: HttpOnly is intentionally omitted so that JavaScript can read
    /// the cookie for the double-submit CSRF pattern.
    pub fn build_cookie_header(&self, token: &str, secure: bool) -> HeaderValue {
        let mut cookie = format!(
            "csrf_token={}; SameSite=Strict; Path=/; Max-Age=86400",
            token
        );
        if secure {
            cookie.push_str("; Secure");
        }
        HeaderValue::from_str(&cookie)
            .unwrap_or_else(|_| HeaderValue::from_str("csrf_token=invalid; Path=/").unwrap())
    }
}

// Initial issuance in auth handlers and middleware rotation share the same
// process-local token registry, including across cloned router guards.
fn issued_tokens() -> Arc<Mutex<HashMap<Vec<u8>, Instant>>> {
    static TOKENS: OnceLock<Arc<Mutex<HashMap<Vec<u8>, Instant>>>> = OnceLock::new();
    TOKENS
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

impl Default for CsrfGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// CSRF guard middleware function
///
/// Validates the CSRF token for state-changing requests, then rotates it
/// on success so that each consumed token is single-use.
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
        CsrfValidation::Valid => {
            // Generate a fresh CSRF token to replace the one that was just consumed.
            // This prevents CSRF token reuse attacks.
            let new_token = match guard.generate_token() {
                Ok(token) => token,
                Err(error) => return error.into_response(),
            };
            // Login issues its own token in the body, cookie, and response header.
            let is_login = request.method() == Method::POST && request.uri().path() == "/login";
            let response = next.run(request).await;
            if is_login && response.status().is_success() {
                response
            } else {
                rotate_csrf_token_in_response(response, &guard, &new_token)
            }
        }
        CsrfValidation::MissingCookie | CsrfValidation::MissingHeader => {
            csrf_error_response("csrf_missing", "CSRF token required")
        }
        CsrfValidation::Mismatch => csrf_error_response("csrf_mismatch", "CSRF token mismatch"),
    }
}

/// Rotate the CSRF token in a successful response.
///
/// Sets a new `csrf_token` cookie and an `X-CSRF-Token` header so the
/// client can update its stored token for the next state-changing request.
fn rotate_csrf_token_in_response(
    mut response: axum::response::Response,
    guard: &CsrfGuard,
    new_token: &str,
) -> axum::response::Response {
    let secure = crate::handlers::auth::is_secure(
        std::env::var("ROOK_ENV").ok().as_deref(),
        cfg!(not(debug_assertions)),
    );
    let cookie_header = guard.build_cookie_header(new_token, secure);

    response
        .headers_mut()
        .append(axum::http::header::SET_COOKIE, cookie_header);
    if let Ok(header_value) = HeaderValue::from_str(new_token) {
        response.headers_mut().insert("X-CSRF-Token", header_value);
    }
    response
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

impl IntoResponse for CsrfError {
    fn into_response(self) -> axum::response::Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "csrf_generation_failed",
                "message": "Unable to generate CSRF token",
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csrf_guard() -> CsrfGuard {
        CsrfGuard::new()
    }

    #[tokio::test]
    async fn rotation_rejects_replay_before_handler_and_accepts_replacement() {
        use axum::{middleware, routing::post, Router};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let guard = Arc::new(CsrfGuard::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let app = Router::new()
            .route(
                "/change",
                post(move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        StatusCode::OK
                    }
                }),
            )
            .layer(middleware::from_fn_with_state(
                guard.clone(),
                csrf_guard_middleware,
            ));
        let server = axum_test::TestServer::new(app);
        let token = guard.generate_token().unwrap();
        let first = server
            .post("/change")
            .add_header("cookie", format!("csrf_token={token}"))
            .add_header("x-csrf-token", &token)
            .await;
        first.assert_status_ok();
        let replacement = first.header("x-csrf-token").to_str().unwrap().to_owned();
        let replay = server
            .post("/change")
            .add_header("cookie", format!("csrf_token={token}"))
            .add_header("x-csrf-token", &token)
            .await;
        replay.assert_status_forbidden();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server
            .post("/change")
            .add_header("cookie", format!("csrf_token={replacement}"))
            .add_header("x-csrf-token", &replacement)
            .await
            .assert_status_ok();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn rng_failure_stops_before_handler_without_setting_cookie() {
        use axum::{middleware, routing::post, Router};
        let guard = Arc::new(CsrfGuard {
            rng: SystemRandom::new(),
            fail_generation: true,
            tokens: issued_tokens(),
        });
        assert_eq!(guard.generate_token(), Err(CsrfError::GenerationFailed));
        let app = Router::new()
            .route(
                "/change",
                post(|| async {
                    panic!("handler must not run");
                }),
            )
            .layer(middleware::from_fn_with_state(guard, csrf_guard_middleware));
        let token = CsrfGuard::new().generate_token().unwrap();
        let response = axum_test::TestServer::new(app)
            .post("/change")
            .add_header("cookie", format!("csrf_token={token}"))
            .add_header("x-csrf-token", &token)
            .await;
        response.assert_status_internal_server_error();
        assert!(response.maybe_header("set-cookie").is_none());
    }

    #[test]
    fn secure_cookie_policy_covers_release_and_production() {
        for (env, release, expected) in [
            (None, true, true),
            (Some("production"), false, true),
            (None, false, false),
        ] {
            let secure = crate::handlers::auth::is_secure(env, release);
            assert_eq!(secure, expected);
            assert_eq!(
                csrf_guard()
                    .build_cookie_header("token", secure)
                    .to_str()
                    .unwrap()
                    .contains("; Secure"),
                expected
            );
        }
    }

    #[test]
    fn validation_rejects_expired_and_unissued_tokens() {
        let guard = CsrfGuard::new();
        let token = guard.generate_token().unwrap();
        let decoded = URL_SAFE_NO_PAD.decode(&token).unwrap();
        guard
            .tokens
            .lock()
            .insert(decoded, Instant::now() - Duration::from_secs(86401));
        assert_eq!(
            guard.validate(Some(&token), Some(&token)),
            CsrfValidation::Mismatch
        );
        let unissued = URL_SAFE_NO_PAD.encode([0; 32]);
        assert_eq!(
            guard.validate(Some(&unissued), Some(&unissued)),
            CsrfValidation::Mismatch
        );
    }

    #[test]
    fn concurrent_validation_consumes_token_atomically() {
        let guard = Arc::new(CsrfGuard::new());
        let token = guard.generate_token().unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let guard = guard.clone();
                let token = token.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    guard.validate(Some(&token), Some(&token))
                })
            })
            .collect();
        let valid = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|result| *result == CsrfValidation::Valid)
            .count();
        assert_eq!(valid, 1);
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
    fn build_cookie_header_includes_same_site_strict_without_http_only() {
        let guard = csrf_guard();
        let token = guard.generate_token().expect("Should generate token");

        let header = guard.build_cookie_header(&token, false);
        let header_str = header.to_str().expect("Should be valid str");

        assert!(
            !header_str.contains("HttpOnly"),
            "HttpOnly should be omitted for double-submit pattern"
        );
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
