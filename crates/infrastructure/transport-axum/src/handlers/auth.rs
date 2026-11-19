// auth — HTTP handlers for authentication endpoints (login, logout)
//
// POST /login — authenticates admin and creates a session
// GET /login — returns CSRF token for browser clients
// POST /logout — revokes the current session

use std::sync::Arc;

use axum::{
    extract::{Extension, State},
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
    Extension(csrf_guard): Extension<Arc<crate::middleware::csrf_guard::CsrfGuard>>,
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

            // Issue a fresh CSRF token bundled into the login response body.
            // This lets the client seed its CSRF cache before the first
            // state-changing request, avoiding the GET /login round-trip
            // that races with the Set-Cookie landing in WebKit's cookie jar.
            let csrf_token = match csrf_guard.generate_token_for(Some(&cookie_value)) {
                Ok(token) => token,
                Err(_) => {
                    // RNG failure is extremely rare. Return 500 rather than
                    // issuing a predictable token.
                    let body = serde_json::json!({
                        "error": "internal_error",
                        "message": "An internal error occurred."
                    });
                    return (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response();
                }
            };
            let rook_env = rook_environment();
            let csrf_cookie = csrf_guard.build_cookie_header(
                &csrf_token,
                is_secure(rook_env.as_deref(), cfg!(not(debug_assertions))),
            );

            // Build auth_token Set-Cookie header
            let auth_cookie = build_auth_token_cookie(
                &cookie_value,
                rook_env.as_deref(),
                cfg!(not(debug_assertions)),
            );

            let csrf_header = HeaderValue::from_str(&csrf_token).expect("base64url token");
            let body = LoginResponse {
                session_id,
                expires_at,
                csrf_token,
            };

            (
                AppendHeaders([
                    (SET_COOKIE, auth_cookie),
                    (SET_COOKIE, csrf_cookie),
                    (
                        axum::http::HeaderName::from_static("x-csrf-token"),
                        csrf_header,
                    ),
                ]),
                Json(body),
            )
                .into_response()
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
    State(usecases): State<Arc<RookUsecases>>,
    Extension(csrf_guard): Extension<Arc<crate::middleware::csrf_guard::CsrfGuard>>,
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
    let token_hash = hex::encode(hasher.finalize());

    // Prepare the anonymous CSRF token before revoking the session so RNG failure
    // cannot leave the client logged out without a usable CSRF context.
    let csrf_token = match csrf_guard.generate_token() {
        Ok(token) => token,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "internal_error",
                    "message": "An internal error occurred."
                })),
            )
                .into_response();
        }
    };
    let secure = is_secure(rook_environment().as_deref(), cfg!(not(debug_assertions)));
    let csrf_cookie = csrf_guard.build_cookie_header(&csrf_token, secure);
    let csrf_header = HeaderValue::from_str(&csrf_token).expect("base64url token");

    // Revoke the session.
    // "Session not found" and "already revoked" are both fine — the client's intent
    // (logout) is achieved either way. Always clear the cookie and return success.
    match usecases.revoke_session_by_token_hash(&token_hash).await {
        Ok(()) => {
            let body = serde_json::json!({ "message": "Logged out successfully." });
            let clear_cookie = build_clear_auth_token_cookie(
                rook_environment().as_deref(),
                cfg!(not(debug_assertions)),
            );
            (
                AppendHeaders([
                    (SET_COOKIE, clear_cookie),
                    (SET_COOKIE, csrf_cookie.clone()),
                    (
                        axum::http::HeaderName::from_static("x-csrf-token"),
                        csrf_header.clone(),
                    ),
                ]),
                Json(body),
            )
                .into_response()
        }
        Err(_) => {
            // Session already gone — treat as successful logout and clear the cookie.
            let body = serde_json::json!({ "message": "Logged out successfully." });
            let clear_cookie = build_clear_auth_token_cookie(
                rook_environment().as_deref(),
                cfg!(not(debug_assertions)),
            );
            (
                AppendHeaders([
                    (SET_COOKIE, clear_cookie),
                    (SET_COOKIE, csrf_cookie.clone()),
                    (
                        axum::http::HeaderName::from_static("x-csrf-token"),
                        csrf_header.clone(),
                    ),
                ]),
                Json(body),
            )
                .into_response()
        }
    }
}

/// Build the auth_token cookie string
///
/// Secure is required in release builds and can be enabled in debug deployments.
fn build_auth_token_cookie(token: &str, rook_env: Option<&str>, is_release: bool) -> HeaderValue {
    let secure = is_secure(rook_env, is_release);
    let mut cookie = format!(
        "auth_token={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=86400",
        token
    );
    if secure {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie).unwrap_or_else(|_| {
        let fallback = if secure {
            "auth_token=invalid; HttpOnly; Path=/; Secure"
        } else {
            "auth_token=invalid; HttpOnly; Path=/"
        };
        HeaderValue::from_str(fallback).unwrap()
    })
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
    /// Fresh CSRF token — the same value set as the csrf_token cookie.
    /// Clients read this cookie and echo it in the X-CSRF-Token header
    /// for state-changing requests (double-submit cookie pattern).
    pub csrf_token: String,
}

/// GET /api/me — return current authenticated user info
///
/// Requires a valid session cookie (auth_token). The authz middleware validates
/// the session before this handler runs; if the cookie is missing or invalid,
/// the middleware returns 401 before reaching here.
///
/// Response: 200 OK with `{ "username": "admin", "displayName": "Rook Admin" }`
pub async fn get_me_handler(State(_usecases): State<Arc<RookUsecases>>) -> impl IntoResponse {
    let body = serde_json::json!({
        "username": "admin",
        "displayName": "Rook Admin",
    });
    (StatusCode::OK, Json(body)).into_response()
}

/// GET /login — return a fresh CSRF token for browser clients
///
/// Always generates a new CSRF token rather than reusing the existing one,
/// ensuring token rotation even on GET requests. Sets csrf_token cookie and
/// returns the token in the response body. The client must echo this token
/// back in the X-CSRF-Token header for state-changing requests (POST, PUT, DELETE).
pub async fn get_login_handler(
    State(_usecases): State<Arc<RookUsecases>>,
    Extension(csrf_guard): Extension<Arc<crate::middleware::csrf_guard::CsrfGuard>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Bind the token to the current session when a session cookie is present.
    let auth_token = extract_cookie(&headers, "auth_token");
    let token = match csrf_guard.generate_token_for(auth_token.as_deref()) {
        Ok(token) => token,
        Err(_) => {
            // RNG failure — preserve any existing incoming CSRF cookie so the
            // client can continue using it rather than losing CSRF protection.
            let body = serde_json::json!({
                "error": "internal_error",
                "message": "Failed to generate CSRF token",
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response();
        }
    };

    let cookie = csrf_guard.build_cookie_header(
        &token,
        is_secure(rook_environment().as_deref(), cfg!(not(debug_assertions))),
    );

    let body = serde_json::json!({
        "csrf_token": token,
    });

    (AppendHeaders([(SET_COOKIE, cookie)]), Json(body)).into_response()
}

fn build_clear_auth_token_cookie(rook_env: Option<&str>, is_release: bool) -> HeaderValue {
    if is_secure(rook_env, is_release) {
        HeaderValue::from_static(
            "auth_token=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0; Secure",
        )
    } else {
        HeaderValue::from_static("auth_token=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0")
    }
}

fn rook_environment() -> Option<String> {
    std::env::var("ROOK_ENV").ok()
}

pub(crate) fn is_secure(rook_env: Option<&str>, is_release: bool) -> bool {
    is_release || rook_env == Some("production")
}
