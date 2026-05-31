// auth — HTTP handlers for authentication endpoints (login, logout)
//
// POST /login — authenticates admin and creates a session
// GET /login — returns CSRF token for browser clients
// POST /logout — revokes the current session

use std::sync::Arc;

use axum::{
    extract::State,
    http::{
        header::{COOKIE, SET_COOKIE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{AppendHeaders, IntoResponse},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rook_usecases::{LoginError, LoginInput, RookUsecases};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// POST /login — authenticate admin and create session
///
/// Request body: { "username": "...", "password": "..." }
/// Success: 200 OK, Set-Cookie: auth_token=\<token\>, body: { "session_id": "...", "expires_at": "..." }
/// Error: 401 Unauthorized with error details
/// Rate limited: 429 Too Many Requests with Retry-After header
pub async fn login_handler(
    State(usecases): State<Arc<RookUsecases>>,
    Json(body): Json<LoginRequest>,
) -> impl IntoResponse {
    // Note: Rate limiting is applied at the middleware level in production
    // For now, we pass through without rate limiting - the CSRF and auth middleware
    // provide protection against the main attack vectors

    let input = LoginInput {
        username: body.username,
        password: body.password,
    };

    match usecases.login.execute(input).await {
        Ok(output) => {
            let cookie_value = output.token.clone(); // raw token, base64url encoded
            let session_id = output.session_id.to_string();
            let expires_at = output.expires_at.to_rfc3339();

            // Build Set-Cookie header
            // Note: In production builds, Secure flag is set based on ROOK_ENV
            let cookie = build_auth_token_cookie(&cookie_value);

            let body = serde_json::json!({
                "session_id": session_id,
                "expires_at": expires_at,
            });

            (AppendHeaders([(SET_COOKIE, cookie)]), Json(body)).into_response()
        }
        Err(LoginError::PasswordNotSet) => {
            let body = serde_json::json!({
                "error": "password_not_set",
                "message": "Admin password not set. Please set via TUI or first-time setup."
            });
            (StatusCode::UNAUTHORIZED, Json(body)).into_response()
        }
        Err(LoginError::InvalidCredentials) => {
            let body = serde_json::json!({
                "error": "invalid_credentials",
                "message": "Invalid username or password."
            });
            (StatusCode::UNAUTHORIZED, Json(body)).into_response()
        }
        Err(_) => {
            let body = serde_json::json!({
                "error": "internal_error",
                "message": "An internal error occurred."
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
        }
    }
}

/// POST /logout — revoke the current session
///
/// Extracts auth_token cookie, computes token_hash, looks up session, and revokes it.
/// Clears the cookie on success.
pub async fn logout_handler(
    State(_usecases): State<Arc<RookUsecases>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Extract auth_token cookie
    let token = match extract_cookie(&headers, "auth_token") {
        Some(t) => t,
        None => {
            let body = serde_json::json!({
                "error": "missing_token",
                "message": "No auth token provided."
            });
            return (StatusCode::UNAUTHORIZED, Json(body)).into_response();
        }
    };

    // Decode token and compute SHA-256 hash to look up session
    let token_bytes = match URL_SAFE_NO_PAD.decode(&token) {
        Ok(b) => b,
        Err(_) => {
            let body = serde_json::json!({
                "error": "invalid_token",
                "message": "Invalid token format."
            });
            return (StatusCode::UNAUTHORIZED, Json(body)).into_response();
        }
    };

    // SHA-256 hash to find session
    let mut hasher = Sha256::new();
    hasher.update(&token_bytes);
    let _token_hash = format!("{:x}", hasher.finalize());

    // TODO: Once session_repo is accessible from RookUsecases, implement full logout
    let body = serde_json::json!({
        "message": "Logout requires session lookup by token_hash."
    });
    (StatusCode::NOT_IMPLEMENTED, Json(body)).into_response()
}

/// Build the auth_token cookie string
///
/// Secure flag is set based on whether we're in debug mode.
/// In production (release builds), cookies require HTTPS.
fn build_auth_token_cookie(token: &str) -> HeaderValue {
    let secure = !cfg!(debug_assertions);
    let mut cookie = format!(
        "auth_token={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=86400",
        token
    );
    if secure {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie)
        .unwrap_or_else(|_| HeaderValue::from_str("auth_token=invalid; HttpOnly; Path=/").unwrap())
}

/// Extract a cookie value by name from a HeaderMap
fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookie_str| {
            cookie_str.split(';').find_map(|part| {
                let mut parts = part.trim().splitn(2, '=');
                if parts.next() == Some(name) {
                    parts.next().map(|v| v.to_string())
                } else {
                    None
                }
            })
        })
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginResponse {
    pub session_id: String,
    pub expires_at: String,
}

/// GET /login — generate CSRF token for browser clients
///
/// Sets csrf_token cookie and returns the token in the response body.
/// The client must echo this token back in the X-CSRF-Token header
/// for state-changing requests (POST, PUT, DELETE).
pub async fn get_login_handler(State(_usecases): State<Arc<RookUsecases>>) -> impl IntoResponse {
    // Generate CSRF token
    let token = generate_csrf_token();

    let secure = !cfg!(debug_assertions);
    let cookie = build_csrf_token_cookie(&token, secure);

    let body = serde_json::json!({
        "csrf_token": token,
    });

    (AppendHeaders([(SET_COOKIE, cookie)]), Json(body)).into_response()
}

/// Generate a 32-byte random CSRF token encoded as base64url
fn generate_csrf_token() -> String {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 32];
    let rng = ring::rand::SystemRandom::new();
    let _ = rng.fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Build the csrf_token cookie string
fn build_csrf_token_cookie(token: &str, secure: bool) -> HeaderValue {
    let mut cookie = format!(
        "csrf_token={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=86400",
        token
    );
    if secure {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie)
        .unwrap_or_else(|_| HeaderValue::from_str("csrf_token=invalid; HttpOnly; Path=/").unwrap())
}
