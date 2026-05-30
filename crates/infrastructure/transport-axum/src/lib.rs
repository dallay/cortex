// transport-axum — HTTP adapter exposing OpenAI-compatible API
//
// Translates between provider wire formats (OpenAI, Anthropic) and the
// internal domain model. All format-specific logic lives here.

pub mod anthropic_adapter;
pub mod authz;
pub mod handlers;
pub mod middleware;
pub mod openai_adapter;
pub mod provider_dto;
pub mod provider_routes;
pub mod routes;

pub use routes::router;
pub use middleware::{ApiKeyRateLimiter, ApiKeyRateLimitExceeded, CsrfGuard, LoginRateLimiter, RateLimitSnapshot};

use axum::{http::StatusCode, response::IntoResponse, Json};

/// Wrapper for all errors that can be returned as HTTP responses
#[derive(Debug)]
pub struct HttpError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl IntoResponse for HttpError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({
            "error": self.message,
            "code": self.code,
        });
        (self.status, Json(body)).into_response()
    }
}

impl<T: std::fmt::Display> From<T> for HttpError {
    fn from(e: T) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL_ERROR",
            message: e.to_string(),
        }
    }
}
