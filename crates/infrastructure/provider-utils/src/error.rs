// error — shared HTTP error handling helpers for provider adapters

use reqwest::Response;
use shared_kernel::{CortexError, ProviderId};

/// Truncate and clean an error response body to prevent sensitive data leakage.
///
/// - Truncates to `max_len` characters
/// - Attempts to redact JSON object keys whose lowercase name contains
///   any `SENSITIVE_KEYS` entry (api_key, authorization, token, etc.)
///
/// Used by: providers-openai (with JSON redaction), providers-anthropic (simple truncate)
pub fn sanitize_error_body(body: &str, max_len: usize) -> String {
    const SENSITIVE_KEYS: &[&str] = &[
        "api_key",
        "authorization",
        "token",
        "access_token",
        "secret",
        "headers",
    ];

    // Try to parse as JSON and redact sensitive fields
    if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(obj) = json.as_object_mut() {
            let keys_to_redact: Vec<String> = obj
                .keys()
                .filter(|k| {
                    let lower = k.to_lowercase();
                    SENSITIVE_KEYS.iter().any(|s| lower.contains(s))
                })
                .cloned()
                .collect();
            for key in keys_to_redact {
                obj.insert(key, serde_json::Value::String("(redacted)".to_string()));
            }
        }
        let sanitized = serde_json::to_string(&json).unwrap_or_else(|_| body.to_string());
        truncate(&sanitized, max_len)
    } else {
        // Fall back to plain text truncation
        truncate(body, max_len)
    }
}

/// Truncate a string to at most `max` chars, safe across UTF-8 multi-byte boundaries.
/// Appends "… (truncated)" when actual truncation occurred.
fn truncate(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{truncated}… (truncated)")
    } else {
        truncated.to_string()
    }
}

/// Common HTTP error context extracted from a failed provider response.
#[derive(Debug)]
pub struct HttpErrorContext {
    pub status: reqwest::StatusCode,
    pub retry_after_secs: Option<u64>,
    pub reset_unix: Option<u64>,
    pub sanitized_body: String,
}

/// Extract common HTTP error context from a failed response.
///
/// Reads `Retry-After` and `x-ratelimit-reset` headers for 429 responses,
/// then sanitizes the body for use in error messages.
pub async fn extract_http_error_context(
    provider_id: &ProviderId,
    resp: Response,
    max_body_len: usize,
) -> HttpErrorContext {
    let status = resp.status();
    let retry_after_secs = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    let reset_unix = resp
        .headers()
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    let raw_body = resp.text().await.unwrap_or_default();
    let sanitized_body = sanitize_error_body(&raw_body, max_body_len);

    HttpErrorContext {
        status,
        retry_after_secs,
        reset_unix,
        sanitized_body,
    }
}

/// Map a provider HTTP error response to a typed `CortexError`.
///
/// `on_other` is called for non-special status codes to produce the error message.
pub async fn map_http_error<F>(provider_id: ProviderId, resp: Response, on_other: F) -> CortexError
where
    F: FnOnce(reqwest::StatusCode, String) -> CortexError,
{
    let ctx = extract_http_error_context(&provider_id, resp, 200).await;

    match ctx.status.as_u16() {
        401 => CortexError::auth_failed("authentication failed"),
        429 => {
            let retry_secs = ctx.retry_after_secs.unwrap_or(60);
            if let Some(reset) = ctx.reset_unix {
                CortexError::rate_limited_with_reset(provider_id, retry_secs, reset)
            } else {
                CortexError::rate_limited(provider_id, retry_secs)
            }
        }
        400 => CortexError::invalid_request(ctx.sanitized_body),
        _ => on_other(ctx.status, ctx.sanitized_body),
    }
}