// Integration tests for unified cache metrics endpoint (Phase 6)

use axum::{
    body::Body,
    extract::Extension,
    http::{Request, StatusCode},
};
use cache_memory::InMemoryCache;
use rook_core::{CachePort, CompletionResponse, MessageContent, ModelId, ProviderId, TokenUsage};
use shared_kernel::{CacheKey, RequestId};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use transport_axum::handlers::cache::get_cache_stats;

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
async fn get_cache_stats_returns_unified_metrics_with_all_sections() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    // Populate signature cache with entries
    let sig1 = "a".repeat(64);
    let key1 = CacheKey {
        request_id: RequestId::new(),
        signature: sig1.clone(),
    };

    cache
        .set(&key1, &make_response("test1"), Duration::from_secs(300))
        .await
        .unwrap();

    // Simulate signature cache hit
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

    // Parse as JSON to check structure
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Verify unified structure has all three sections
    assert!(
        json.get("signature_cache").is_some(),
        "Should have signature_cache section"
    );
    assert!(
        json.get("token_cache").is_some(),
        "Should have token_cache section"
    );
    assert!(
        json.get("combined").is_some(),
        "Should have combined section"
    );

    // Verify signature_cache fields
    let sig_cache = json.get("signature_cache").unwrap();
    assert!(sig_cache.get("hits").is_some());
    assert!(sig_cache.get("misses").is_some());
    assert!(sig_cache.get("hit_rate").is_some());
    assert!(sig_cache.get("entries").is_some());
    assert!(sig_cache.get("evictions").is_some());

    // Verify token_cache fields
    let token_cache = json.get("token_cache").unwrap();
    assert!(token_cache.get("hits").is_some());
    assert!(token_cache.get("misses").is_some());
    assert!(token_cache.get("tokens_saved").is_some());
    assert!(token_cache.get("estimated_cost_saved_usd").is_some());

    // Verify combined fields
    let combined = json.get("combined").unwrap();
    assert!(combined.get("total_requests").is_some());
    assert!(combined.get("cached_requests").is_some());
    assert!(combined.get("cache_rate").is_some());
}

#[tokio::test]
async fn unified_stats_calculates_combined_metrics_correctly() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    // Create signature cache activity: 2 hits, 1 miss
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
        .set(&key1, &make_response("test1"), Duration::from_secs(300))
        .await
        .unwrap();
    cache
        .set(&key2, &make_response("test2"), Duration::from_secs(300))
        .await
        .unwrap();

    let _ = cache.get(&key1).await; // hit
    let _ = cache.get(&key2).await; // hit
    let _ = cache
        .get(&CacheKey {
            request_id: RequestId::new(),
            signature: "c".repeat(64),
        })
        .await; // miss

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

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let combined = json.get("combined").unwrap();

    // total_requests = signature_hits + signature_misses + token_hits + token_misses
    // In this case: 2 + 1 + 0 + 0 = 3
    let total_requests = combined.get("total_requests").unwrap().as_u64().unwrap();
    assert_eq!(total_requests, 3);

    // cached_requests = signature_hits + token_hits
    // In this case: 2 + 0 = 2
    let cached_requests = combined.get("cached_requests").unwrap().as_u64().unwrap();
    assert_eq!(cached_requests, 2);

    // cache_rate = cached_requests / total_requests = 2 / 3 ≈ 0.6667
    let cache_rate = combined.get("cache_rate").unwrap().as_f64().unwrap();
    assert!((cache_rate - 0.6667).abs() < 0.001);
}

#[tokio::test]
async fn unified_stats_with_signature_only() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

    // Only signature cache activity, no token cache
    let sig1 = "a".repeat(64);
    let key1 = CacheKey {
        request_id: RequestId::new(),
        signature: sig1.clone(),
    };

    cache
        .set(&key1, &make_response("test1"), Duration::from_secs(300))
        .await
        .unwrap();
    let _ = cache.get(&key1).await; // hit

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

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Signature cache should have activity
    let sig_cache = json.get("signature_cache").unwrap();
    assert_eq!(sig_cache.get("hits").unwrap().as_u64().unwrap(), 1);

    // Token cache should be empty/zero
    let token_cache = json.get("token_cache").unwrap();
    assert_eq!(token_cache.get("hits").unwrap().as_u64().unwrap(), 0);
    assert_eq!(token_cache.get("misses").unwrap().as_u64().unwrap(), 0);

    // Combined should reflect signature-only activity
    let combined = json.get("combined").unwrap();
    assert_eq!(combined.get("total_requests").unwrap().as_u64().unwrap(), 1);
    assert_eq!(
        combined.get("cached_requests").unwrap().as_u64().unwrap(),
        1
    );
    assert_eq!(combined.get("cache_rate").unwrap().as_f64().unwrap(), 1.0);
}

#[tokio::test]
async fn unified_stats_with_zero_requests_returns_zero_cache_rate() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(300), None));

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

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let combined = json.get("combined").unwrap();
    assert_eq!(combined.get("total_requests").unwrap().as_u64().unwrap(), 0);
    assert_eq!(
        combined.get("cached_requests").unwrap().as_u64().unwrap(),
        0
    );
    assert_eq!(combined.get("cache_rate").unwrap().as_f64().unwrap(), 0.0);
}
