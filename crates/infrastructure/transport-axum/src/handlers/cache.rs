// Cache management HTTP handlers

use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    Json,
};
use rook_core::{CachePort, CacheStats, SignatureEntry};
use std::sync::Arc;

/// GET /api/cache/stats — Return cache statistics (both signature and token cache layers)
pub async fn get_cache_stats(
    Extension(cache): Extension<Arc<dyn CachePort>>,
) -> Result<Json<CacheStats>, StatusCode> {
    cache
        .stats()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// DELETE /api/cache — Clear entire cache
pub async fn clear_cache(
    Extension(cache): Extension<Arc<dyn CachePort>>,
) -> Result<StatusCode, StatusCode> {
    cache
        .clear()
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// DELETE /api/cache/:signature — Delete specific cache entry by signature
///
/// Returns 204 regardless of whether entry existed (idempotent delete).
pub async fn delete_cache_entry(
    Path(signature): Path<String>,
    Extension(cache): Extension<Arc<dyn CachePort>>,
) -> Result<StatusCode, StatusCode> {
    // Validate signature format (64 hex characters for SHA-256)
    if signature.len() != 64 || !signature.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Delete all entries matching this signature
    cache
        .delete_by_signature(&signature)
        .await
        .map(|_| StatusCode::NO_CONTENT) // Idempotent: always 204 regardless of count
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// GET /api/cache/signatures — List all cached signature entries with metadata
pub async fn list_signatures(
    Extension(cache): Extension<Arc<dyn CachePort>>,
) -> Result<Json<Vec<SignatureEntry>>, StatusCode> {
    cache
        .list_signatures()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// GET /api/cache/signature/:sig — Retrieve cached response by signature
///
/// Returns 200 with the cached response if found and not expired.
/// Returns 404 if signature is not found or expired.
/// Returns 400 if signature format is invalid (not 64 hex chars).
pub async fn get_signature(
    Path(signature): Path<String>,
    Extension(cache): Extension<Arc<dyn CachePort>>,
) -> Result<Json<rook_core::CompletionResponse>, StatusCode> {
    // Validate signature format (64 hex characters for SHA-256)
    if signature.len() != 64 || !signature.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(StatusCode::BAD_REQUEST);
    }

    match cache.get_by_signature(&signature).await {
        Ok(Some(response)) => Ok(Json(response)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
