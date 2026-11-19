// Stateless signed double-submit CSRF protection for browser management routes.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};
use sha2::{Digest, Sha256};

const KEY_DERIVATION_LABEL: &[u8] = b"cortex/rook/csrf-signing/v1";
const TOKEN_DOMAIN: &[u8] = b"cortex/rook/csrf-token/v1";
const TOKEN_VERSION: u8 = 1;
const MAX_AGE_SECS: u64 = 86_400;
/// Tolerance for clock divergence between replicas that share the signing secret.
const CLOCK_SKEW_SECS: u64 = 60;
const BINDING_LEN: usize = 32;
const NONCE_LEN: usize = 16;
const PAYLOAD_LEN: usize = 1 + 1 + 8 + 8 + BINDING_LEN + NONCE_LEN;
const SIGNATURE_LEN: usize = 32;
const TOKEN_LEN: usize = PAYLOAD_LEN + SIGNATURE_LEN;

/// CSRF validation result.
#[derive(Debug, Clone, PartialEq)]
pub enum CsrfValidation {
    Valid,
    MissingCookie,
    MissingHeader,
    Mismatch,
}

/// Stateless CSRF signer and cookie helper.
#[derive(Clone)]
pub struct CsrfGuard {
    signing_key: [u8; SIGNATURE_LEN],
    rng: SystemRandom,
    #[cfg(test)]
    fail_generation: bool,
}

impl CsrfGuard {
    /// Create an ephemeral guard. Use `from_persistent_secret` in application composition.
    pub fn new() -> Self {
        let mut key = [0u8; SIGNATURE_LEN];
        SystemRandom::new()
            .fill(&mut key)
            .expect("system RNG unavailable");
        Self::with_signing_key(key)
    }

    /// Derive a domain-separated signing key from the persistent Rook secret.
    pub fn from_persistent_secret(secret: &[u8]) -> Self {
        Self::with_signing_key(derive_csrf_signing_key(secret))
    }

    /// Construct a guard with an already-derived 32-byte signing key.
    pub fn with_signing_key(signing_key: [u8; SIGNATURE_LEN]) -> Self {
        Self {
            signing_key,
            rng: SystemRandom::new(),
            #[cfg(test)]
            fail_generation: false,
        }
    }

    /// Generate an anonymous token for callers without a session cookie.
    pub fn generate_token(&self) -> Result<String, CsrfError> {
        self.generate_token_for(None)
    }

    /// Generate a token bound to the current auth cookie, or anonymously if absent.
    pub fn generate_token_for(&self, auth_token_cookie: Option<&str>) -> Result<String, CsrfError> {
        #[cfg(test)]
        if self.fail_generation {
            return Err(CsrfError::GenerationFailed);
        }

        let now = unix_time().ok_or(CsrfError::GenerationFailed)?;
        let expires = now
            .checked_add(MAX_AGE_SECS)
            .ok_or(CsrfError::GenerationFailed)?;
        self.issue_at(auth_token_cookie, now, expires)
    }

    fn issue_at(
        &self,
        auth_token_cookie: Option<&str>,
        issued_at: u64,
        expires_at: u64,
    ) -> Result<String, CsrfError> {
        let mut payload = [0u8; PAYLOAD_LEN];
        payload[0] = TOKEN_VERSION;
        payload[1] = u8::from(auth_token_cookie.is_some());
        payload[2..10].copy_from_slice(&issued_at.to_be_bytes());
        payload[10..18].copy_from_slice(&expires_at.to_be_bytes());
        if let Some(cookie) = auth_token_cookie {
            payload[18..50].copy_from_slice(&session_binding(cookie));
        }
        self.rng
            .fill(&mut payload[50..])
            .map_err(|_| CsrfError::GenerationFailed)?;

        let signature = self.sign(&payload);
        let mut token = Vec::with_capacity(TOKEN_LEN);
        token.extend_from_slice(&payload);
        token.extend_from_slice(signature.as_ref());
        Ok(URL_SAFE_NO_PAD.encode(token))
    }

    /// Validate double-submit equality and an anonymous token.
    pub fn validate(&self, cookie: Option<&str>, header: Option<&str>) -> CsrfValidation {
        self.validate_for(cookie, header, None)
    }

    /// Validate token signature, lifetime, and binding against the current auth cookie.
    pub fn validate_for(
        &self,
        cookie_value: Option<&str>,
        header_value: Option<&str>,
        auth_token_cookie: Option<&str>,
    ) -> CsrfValidation {
        let Some(now) = unix_time() else {
            return CsrfValidation::Mismatch;
        };
        self.validate_for_at(cookie_value, header_value, auth_token_cookie, now)
    }

    fn validate_for_at(
        &self,
        cookie_value: Option<&str>,
        header_value: Option<&str>,
        auth_token_cookie: Option<&str>,
        now: u64,
    ) -> CsrfValidation {
        let cookie = match cookie_value {
            Some(value) => value,
            None => return CsrfValidation::MissingCookie,
        };
        let header = match header_value {
            Some(value) => value,
            None => return CsrfValidation::MissingHeader,
        };
        if !constant_time_equal(cookie.as_bytes(), header.as_bytes()) {
            return CsrfValidation::Mismatch;
        }

        self.validate_token_at(cookie, auth_token_cookie, now)
    }

    /// Return the incoming token when it fully validates for the current context.
    ///
    /// Reusing the token keeps the double-submit pair byte-identical, so a
    /// browser that has not applied the latest `Set-Cookie` yet still sends a
    /// matching header and cookie. A token that fails validation is never
    /// reused; a fresh token bound to the current auth cookie is issued
    /// instead.
    pub fn token_for_session(
        &self,
        incoming_cookie: Option<&str>,
        auth_token_cookie: Option<&str>,
    ) -> Result<String, CsrfError> {
        if let Some(incoming) = incoming_cookie {
            if let Some(now) = unix_time() {
                if self.validate_token_at(incoming, auth_token_cookie, now) == CsrfValidation::Valid
                {
                    return Ok(incoming.to_string());
                }
            }
        }
        self.generate_token_for(auth_token_cookie)
    }

    /// Validate the signature, lifetime, and session binding of a single token.
    fn validate_token_at(
        &self,
        token_value: &str,
        auth_token_cookie: Option<&str>,
        now: u64,
    ) -> CsrfValidation {
        let Ok(token) = URL_SAFE_NO_PAD.decode(token_value) else {
            return CsrfValidation::Mismatch;
        };
        if token.len() != TOKEN_LEN {
            return CsrfValidation::Mismatch;
        }
        let (payload, signature) = token.split_at(PAYLOAD_LEN);
        if payload[0] != TOKEN_VERSION || self.verify(payload, signature).is_err() {
            return CsrfValidation::Mismatch;
        }

        let has_session = payload[1] == 1;
        if payload[1] > 1 || has_session != auth_token_cookie.is_some() {
            return CsrfValidation::Mismatch;
        }
        let issued_at = u64::from_be_bytes(payload[2..10].try_into().expect("fixed payload"));
        let expires_at = u64::from_be_bytes(payload[10..18].try_into().expect("fixed payload"));
        // A small clock divergence between replicas must not reject a token
        // issued moments earlier by a peer that shares the signing secret.
        if issued_at > now.saturating_add(CLOCK_SKEW_SECS)
            || expires_at <= now
            || expires_at < issued_at
            || expires_at - issued_at > MAX_AGE_SECS
        {
            return CsrfValidation::Mismatch;
        }

        let expected_binding = auth_token_cookie
            .map(session_binding)
            .unwrap_or([0u8; BINDING_LEN]);
        if !constant_time_equal(&payload[18..50], &expected_binding) {
            return CsrfValidation::Mismatch;
        }
        CsrfValidation::Valid
    }

    fn sign(&self, payload: &[u8]) -> hmac::Tag {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.signing_key);
        let mut message = Vec::with_capacity(TOKEN_DOMAIN.len() + payload.len());
        message.extend_from_slice(TOKEN_DOMAIN);
        message.extend_from_slice(payload);
        hmac::sign(&key, &message)
    }

    fn verify(&self, payload: &[u8], signature: &[u8]) -> Result<(), ring::error::Unspecified> {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.signing_key);
        let mut message = Vec::with_capacity(TOKEN_DOMAIN.len() + payload.len());
        message.extend_from_slice(TOKEN_DOMAIN);
        message.extend_from_slice(payload);
        hmac::verify(&key, &message, signature)
    }

    /// Build the CSRF cookie header value. HttpOnly is omitted for double-submit clients.
    pub fn build_cookie_header(&self, token: &str, secure: bool) -> HeaderValue {
        let mut cookie = format!("csrf_token={token}; SameSite=Strict; Path=/; Max-Age=86400");
        if secure {
            cookie.push_str("; Secure");
        }
        HeaderValue::from_str(&cookie)
            .unwrap_or_else(|_| HeaderValue::from_static("csrf_token=invalid; Path=/"))
    }
}

impl Default for CsrfGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Derive a 32-byte CSRF signing key with HMAC domain separation.
pub fn derive_csrf_signing_key(secret: &[u8]) -> [u8; SIGNATURE_LEN] {
    let derivation_key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let derived = hmac::sign(&derivation_key, KEY_DERIVATION_LABEL);
    let mut key = [0u8; SIGNATURE_LEN];
    key.copy_from_slice(derived.as_ref());
    key
}

fn unix_time() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut difference = 0u8;
    for (left_byte, right_byte) in left.iter().zip(right) {
        difference |= left_byte ^ right_byte;
    }
    difference == 0
}

fn session_binding(auth_token_cookie: &str) -> [u8; BINDING_LEN] {
    let digest = Sha256::digest(auth_token_cookie.as_bytes());
    let mut binding = [0u8; BINDING_LEN];
    binding.copy_from_slice(&digest);
    binding
}

/// CSRF guard middleware function.
pub async fn csrf_guard_middleware(
    State(guard): State<Arc<CsrfGuard>>,
    mut request: Request,
    next: Next,
) -> axum::response::Response {
    // Handlers such as GET /login need this same signer even though GET does not validate CSRF.
    request.extensions_mut().insert(guard.clone());
    let method = request.method().clone();
    if !is_state_changing_method(&method) {
        return next.run(request).await;
    }

    let path = request.uri().path();
    if path.starts_with("/v1/") || path.starts_with("/v1") {
        return next.run(request).await;
    }

    let headers = request.headers();
    let cookie_value = extract_cookie(headers, "csrf_token");
    let auth_token_cookie = extract_cookie(headers, "auth_token");
    let header_value = extract_header(&request, "x-csrf-token");
    let validation = guard.validate_for(
        cookie_value.as_deref(),
        header_value.as_deref(),
        auth_token_cookie.as_deref(),
    );

    match validation {
        CsrfValidation::Valid => {
            let path = request.uri().path();
            let handler_issues_token =
                request.method() == Method::POST && (path == "/login" || path == "/logout");
            let new_token = if handler_issues_token {
                None
            } else {
                match guard.generate_token_for(auth_token_cookie.as_deref()) {
                    Ok(token) => Some(token),
                    Err(error) => return error.into_response(),
                }
            };
            let response = next.run(request).await;
            if let Some(token) = new_token {
                rotate_csrf_token_in_response(response, &guard, &token)
            } else {
                response
            }
        }
        CsrfValidation::MissingCookie | CsrfValidation::MissingHeader => {
            csrf_error_response("csrf_missing", "CSRF token required")
        }
        CsrfValidation::Mismatch => csrf_error_response("csrf_mismatch", "CSRF token mismatch"),
    }
}

fn rotate_csrf_token_in_response(
    mut response: axum::response::Response,
    guard: &CsrfGuard,
    new_token: &str,
) -> axum::response::Response {
    let secure = crate::handlers::auth::is_secure(
        std::env::var("ROOK_ENV").ok().as_deref(),
        cfg!(not(debug_assertions)),
    );
    response.headers_mut().append(
        axum::http::header::SET_COOKIE,
        guard.build_cookie_header(new_token, secure),
    );
    if let Ok(value) = HeaderValue::from_str(new_token) {
        response.headers_mut().insert("X-CSRF-Token", value);
    }
    response
}

fn is_state_changing_method(method: &Method) -> bool {
    matches!(
        method,
        &Method::POST | &Method::PUT | &Method::DELETE | &Method::PATCH
    )
}

fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(cookie_name, value)| (cookie_name == name).then(|| value.to_owned()))
}

fn extract_header(request: &Request, name: &str) -> Option<String> {
    request.headers().iter().find_map(|(key, value)| {
        key.as_str()
            .eq_ignore_ascii_case(name)
            .then(|| value.to_str().ok().map(str::to_owned))
            .flatten()
    })
}

fn csrf_error_response(code: &str, message: &str) -> axum::response::Response {
    let body = serde_json::json!({ "error": code, "message": message });
    (StatusCode::FORBIDDEN, Json(body)).into_response()
}

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
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Barrier,
    };

    const KEY: [u8; 32] = [42; 32];

    #[derive(Clone)]
    struct TestGuard {
        guard: CsrfGuard,
        now: Arc<AtomicU64>,
    }
    impl TestGuard {
        fn new(now: u64) -> Self {
            Self {
                guard: CsrfGuard::with_signing_key(KEY),
                now: Arc::new(AtomicU64::new(now)),
            }
        }
        fn issue(&self, binding: Option<&str>) -> String {
            let issued_at = self.now.load(Ordering::SeqCst);
            self.guard
                .issue_at(binding, issued_at, issued_at + MAX_AGE_SECS)
                .unwrap()
        }
        fn validate(&self, token: &str, binding: Option<&str>) -> CsrfValidation {
            self.guard.validate_for_at(
                Some(token),
                Some(token),
                binding,
                self.now.load(Ordering::SeqCst),
            )
        }
    }

    #[test]
    fn signed_anonymous_token_validates_and_is_reusable() {
        let test = TestGuard::new(1_800_000_000);
        let token = test.issue(None);
        for _ in 0..2 {
            assert_eq!(test.validate(&token, None), CsrfValidation::Valid);
        }
    }

    #[test]
    fn modified_token_is_rejected() {
        let test = TestGuard::new(1_800_000_000);
        let token = test.issue(None);
        let mut modified = token.into_bytes();
        let last = modified.len() - 1;
        modified[last] = if modified[last] == b'A' { b'B' } else { b'A' };
        let modified = String::from_utf8(modified).unwrap();
        assert_eq!(test.validate(&modified, None), CsrfValidation::Mismatch);
    }

    #[test]
    fn expiry_and_lifetime_over_24_hours_are_rejected() {
        let test = TestGuard::new(1_800_000_000);
        let expired = test
            .guard
            .issue_at(None, 1_700_000_000, 1_700_086_400)
            .unwrap();
        assert_eq!(test.validate(&expired, None), CsrfValidation::Mismatch);
        let overlong = test
            .guard
            .issue_at(None, 1_800_000_000, 1_800_086_401)
            .unwrap();
        assert_eq!(test.validate(&overlong, None), CsrfValidation::Mismatch);
    }

    #[test]
    fn session_binding_rejects_a_different_session_and_anonymous_context() {
        let test = TestGuard::new(1_800_000_000);
        let token = test.issue(Some("session-a-cookie"));
        assert_eq!(
            test.validate(&token, Some("session-a-cookie")),
            CsrfValidation::Valid
        );
        assert_eq!(
            test.validate(&token, Some("session-b-cookie")),
            CsrfValidation::Mismatch
        );
        assert_eq!(test.validate(&token, None), CsrfValidation::Mismatch);
    }

    #[test]
    fn anonymous_token_is_rejected_when_auth_cookie_is_present() {
        let test = TestGuard::new(1_800_000_000);
        let token = test.issue(None);
        assert_eq!(
            test.validate(&token, Some("session-cookie")),
            CsrfValidation::Mismatch
        );
    }

    #[test]
    fn small_clock_skew_is_tolerated_and_far_future_tokens_are_rejected() {
        let test = TestGuard::new(1_800_000_000);
        let slightly_ahead = test
            .guard
            .issue_at(None, 1_800_000_030, 1_800_086_430)
            .unwrap();
        assert_eq!(test.validate(&slightly_ahead, None), CsrfValidation::Valid);
        let far_ahead = test
            .guard
            .issue_at(None, 1_800_003_600, 1_800_090_000)
            .unwrap();
        assert_eq!(test.validate(&far_ahead, None), CsrfValidation::Mismatch);
    }

    #[test]
    fn two_concurrent_validations_of_the_same_token_succeed() {
        let test = TestGuard::new(1_800_000_000);
        let token = test.issue(None);
        let barrier = Arc::new(Barrier::new(2));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let guard = test.guard.clone();
                let token = token.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    guard.validate_for_at(Some(&token), Some(&token), None, 1_800_000_000)
                })
            })
            .collect();
        assert!(threads
            .into_iter()
            .all(|thread| thread.join().unwrap() == CsrfValidation::Valid));
    }

    #[test]
    fn cookie_and_header_must_match() {
        let test = TestGuard::new(1_800_000_000);
        let token = test.issue(None);
        let other = test.issue(None);
        assert_eq!(
            test.guard
                .validate_for_at(Some(&token), Some(&other), None, 1_800_000_000),
            CsrfValidation::Mismatch
        );
    }
}
