// Integration tests for cache management HTTP endpoints

use axum::{
    body::Body,
    extract::Extension,
    http::{Request, StatusCode},
};
use cache_memory::InMemoryCache;
use rook_core::{
    CachePort, CacheStats, CompletionResponse, MessageContent, ModelId, ProviderId, SignatureEntry,
    TokenUsage,
};
use shared_kernel::{CacheKey, RequestId};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use transport_axum::authz::{classify_route, AuthTier};
use transport_axum::handlers::cache::{
    clear_cache, delete_cache_entry, get_cache_stats, get_signature, list_signatures,
};

/// Helper: build a test CompletionResponse
fn make_response(content: &str) -> CompletionResponse {
    CompletionResponse {
        id: RequestId::new(),
        model: ModelId::new("gpt-4o"),
        provider: ProviderId::new("openai"),
        content: content.to_string(),
        content_blocks: vec![MessageContent::Text(content.to_string())],
        usage: TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: 30,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            reasoning_tokens: None,
            estimated_cost_usd: None,
        },
        latency_ms: 100,
        cache_hit: None,
    }
}

#[tokio::test]
async fn get_cache_stats_returns_200_with_json() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    // Populate cache with some entries
    let key1 = CacheKey {
        request_id: RequestId::new(),
        signature: "a".repeat(64),
    };
    let key2 = CacheKey {
        request_id: RequestId::new(),
        signature: "b".repeat(64),
    };
    cache
        .set(&key1, &make_response("test1"), Duration::from_secs(300))
        .await
        .unwrap();
    cache
        .set(&key2, &make_response("test2"), Duration::from_secs(300))
        .await
        .unwrap();

    // Simulate a cache hit
    let _ = cache.get(&key1).await;

    let app = axum::Router::new()
        .route("/api/cache/stats", axum::routing::get(get_cache_stats))
        .layer(Extension(cache));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/cache/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let stats: CacheStats = serde_json::from_slice(&body).unwrap();

    assert_eq!(stats.entries, 2);
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 0);
}

#[tokio::test]
async fn clear_cache_returns_204_and_clears_all_entries() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    // Populate cache
    let key = CacheKey {
        request_id: RequestId::new(),
        signature: "c".repeat(64),
    };
    cache
        .set(&key, &make_response("test"), Duration::from_secs(300))
        .await
        .unwrap();

    let app = axum::Router::new()
        .route("/api/cache", axum::routing::delete(clear_cache))
        .layer(Extension(cache.clone()));

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/cache")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify cache is empty
    let stats = cache.stats().await.unwrap();
    assert_eq!(stats.entries, 0);
}

#[tokio::test]
async fn delete_cache_entry_returns_204_for_valid_signature() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    let signature = "d".repeat(64);
    let key = CacheKey {
        request_id: RequestId::new(),
        signature: signature.clone(),
    };
    cache
        .set(&key, &make_response("test"), Duration::from_secs(300))
        .await
        .unwrap();

    let app = axum::Router::new()
        .route(
            "/api/cache/{signature}",
            axum::routing::delete(delete_cache_entry),
        )
        .layer(Extension(cache.clone()));

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/cache/{}", signature))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify entry was deleted
    let deleted_count = cache.delete_by_signature(&signature).await.unwrap();
    assert_eq!(deleted_count, 0); // Already deleted
}

#[tokio::test]
async fn delete_cache_entry_returns_204_for_missing_signature() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    let signature = "e".repeat(64);

    let app = axum::Router::new()
        .route(
            "/api/cache/{signature}",
            axum::routing::delete(delete_cache_entry),
        )
        .layer(Extension(cache));

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/cache/{}", signature))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Idempotent delete: 204 even if not found
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn delete_cache_entry_returns_400_for_malformed_signature() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    let app = axum::Router::new()
        .route(
            "/api/cache/{signature}",
            axum::routing::delete(delete_cache_entry),
        )
        .layer(Extension(cache));

    // Too short
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/cache/short")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Non-hex characters
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/cache/{}", "z".repeat(64)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cache_stats_reflect_hits_and_misses() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    let key = CacheKey {
        request_id: RequestId::new(),
        signature: "f".repeat(64),
    };
    cache
        .set(&key, &make_response("test"), Duration::from_secs(300))
        .await
        .unwrap();

    // Hit
    let _ = cache.get(&key).await;

    // Miss
    let missing_key = CacheKey {
        request_id: RequestId::new(),
        signature: "g".repeat(64),
    };
    let _ = cache.get(&missing_key).await;

    let stats = cache.stats().await.unwrap();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hit_rate(), 0.5);
}

#[test]
fn cache_routes_require_management_auth() {
    use axum::http::Method;

    // Verify that cache management routes are classified as Management tier
    // which requires session authentication (not API key or anonymous)
    assert_eq!(
        classify_route(&Method::GET, "/api/cache/stats"),
        AuthTier::Management,
        "/api/cache/stats should require Management auth"
    );
    assert_eq!(
        classify_route(&Method::DELETE, "/api/cache"),
        AuthTier::Management,
        "/api/cache DELETE should require Management auth"
    );
    assert_eq!(
        classify_route(&Method::DELETE, "/api/cache/somesignature"),
        AuthTier::Management,
        "/api/cache/{{signature}} DELETE should require Management auth"
    );
    // Signature inspection endpoints
    assert_eq!(
        classify_route(&Method::GET, "/api/cache/signatures"),
        AuthTier::Management,
        "/api/cache/signatures should require Management auth"
    );
    assert_eq!(
        classify_route(&Method::GET, "/api/cache/signature/somesig"),
        AuthTier::Management,
        "/api/cache/signature/{{sig}} should require Management auth"
    );
}

// ============================================================================
// Signature Inspection Endpoints (Phase 2, Task 2.8)
// ============================================================================

#[tokio::test]
async fn list_signatures_returns_200_with_empty_list_when_cache_empty() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    let app = axum::Router::new()
        .route("/api/cache/signatures", axum::routing::get(list_signatures))
        .layer(Extension(cache));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/cache/signatures")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let entries: Vec<SignatureEntry> = serde_json::from_slice(&body).unwrap();
    assert_eq!(entries.len(), 0);
}

#[tokio::test]
async fn list_signatures_returns_200_with_signature_entries() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    // Populate cache with entries
    let sig1 = "a".repeat(64);
    let sig2 = "b".repeat(64);
    let key1 = CacheKey {
        request_id: RequestId::new(),
        signature: sig1.clone(),
    };
    let key2 = CacheKey {
        request_id: RequestId::new(),
        signature: sig2.clone(),
    };

    cache
        .set(&key1, &make_response("response1"), Duration::from_secs(300))
        .await
        .unwrap();
    cache
        .set(&key2, &make_response("response2"), Duration::from_secs(300))
        .await
        .unwrap();

    let app = axum::Router::new()
        .route("/api/cache/signatures", axum::routing::get(list_signatures))
        .layer(Extension(cache));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/cache/signatures")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let entries: Vec<SignatureEntry> = serde_json::from_slice(&body).unwrap();
    assert_eq!(entries.len(), 2);

    // Verify entries contain correct signatures
    let signatures: Vec<&str> = entries.iter().map(|e| e.signature.as_str()).collect();
    assert!(signatures.contains(&sig1.as_str()));
    assert!(signatures.contains(&sig2.as_str()));
}

#[tokio::test]
async fn get_signature_returns_200_with_cached_response() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    let signature = "c".repeat(64);
    let key = CacheKey {
        request_id: RequestId::new(),
        signature: signature.clone(),
    };
    let expected_response = make_response("cached content");

    cache
        .set(&key, &expected_response, Duration::from_secs(300))
        .await
        .unwrap();

    let app = axum::Router::new()
        .route(
            "/api/cache/signature/{sig}",
            axum::routing::get(get_signature),
        )
        .layer(Extension(cache));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/cache/signature/{}", signature))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let cached_response: CompletionResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(cached_response.content, "cached content");
    assert_eq!(cached_response.model, expected_response.model);
}

#[tokio::test]
async fn get_signature_returns_404_when_signature_not_found() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    let signature = "d".repeat(64);

    let app = axum::Router::new()
        .route(
            "/api/cache/signature/{sig}",
            axum::routing::get(get_signature),
        )
        .layer(Extension(cache));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/cache/signature/{}", signature))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_signature_returns_400_for_invalid_signature_format() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    let app = axum::Router::new()
        .route(
            "/api/cache/signature/{sig}",
            axum::routing::get(get_signature),
        )
        .layer(Extension(cache.clone()));

    // Test 1: Too short
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/cache/signature/short")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Test 2: Non-hex characters (65 chars, contains 'z')
    let invalid_sig = "z".repeat(64);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/cache/signature/{}", invalid_sig))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Test 3: Correct length but contains non-hex chars
    let invalid_sig = format!("{}xyz", "a".repeat(61));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/cache/signature/{}", invalid_sig))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
