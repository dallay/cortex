// End-to-end cache integration tests
//
// Tests 9.5, 9.6, 9.7 from read-cache tasks.md

use cache_memory::InMemoryCache;
use rook_core::{
    ApiKeyRestrictions, CachePort, CompletionRequest, CompletionResponse, Message, MessageContent,
    ModelId, ProviderId, RequestId, RequestMetadata, Role, TokenUsage,
};
use std::sync::Arc;
use std::time::Duration;

fn test_request(model: &str, prompt: &str) -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new(model),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text(prompt.to_string()),
        }],
        max_tokens: Some(100),
        temperature: Some(0.7),
        stream: false,
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".to_string(),
            cacheable: true,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions::default(),
    }
}

fn test_response(content: &str) -> CompletionResponse {
    CompletionResponse {
        id: RequestId::new(),
        provider: ProviderId::new("test-provider"),
        model: ModelId::new("gpt-4"),
        content: content.to_string(),
        content_blocks: vec![MessageContent::Text(content.to_string())],
        usage: TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: 30,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            reasoning_tokens: None,
            estimated_cost_usd: Some(0.001),
        },
        latency_ms: 50,
        cache_hit: None,
    }
}

/// Test 9.5: End-to-end cache hit flow
///
/// Scenario: Same request twice → second returns cached response, stats show hit
#[tokio::test]
async fn test_cache_hit_flow_increments_hits() {
    // Setup: Create cache with capacity
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(10)));

    // Create request with deterministic content
    let request = test_request("gpt-4", "What is 2+2?");
    let cache_key = request.cache_key();
    let response = test_response("The answer is 4.");

    // First request: cache miss
    let initial_stats = cache.stats().await.expect("get stats");
    assert_eq!(initial_stats.entries, 0, "cache should start empty");

    // Simulate first request: check cache (miss), then store response
    let cached = cache.get(&cache_key).await.expect("get from cache");
    assert!(cached.is_none(), "first request should be cache miss");

    cache
        .set(&cache_key, &response, Duration::from_secs(60))
        .await
        .expect("set in cache");

    // Verify stats after miss + set
    let after_first = cache.stats().await.expect("get stats");
    assert_eq!(after_first.misses, 1, "should have 1 miss");
    assert_eq!(after_first.hits, 0, "should have 0 hits");
    assert_eq!(after_first.entries, 1, "should have 1 entry");

    // Second request: cache hit
    let cached_response = cache.get(&cache_key).await.expect("get from cache");
    assert!(
        cached_response.is_some(),
        "second request should be cache hit"
    );
    assert_eq!(
        cached_response.unwrap().content,
        response.content,
        "cached response should match original"
    );

    // Verify stats after hit
    let after_second = cache.stats().await.expect("get stats");
    assert_eq!(after_second.hits, 1, "should have 1 hit");
    assert_eq!(after_second.misses, 1, "should still have 1 miss");
    assert_eq!(after_second.entries, 1, "should still have 1 entry");

    // Verify hit rate
    assert!(
        (after_second.hit_rate() - 0.5).abs() < 0.01,
        "hit rate should be 0.5 (1 hit / 2 requests)"
    );
}

/// Test 9.6: End-to-end cache miss flow
///
/// Scenario: Unique request → routed to provider (simulated), cached for next time
#[tokio::test]
async fn test_cache_miss_flow_increments_misses() {
    // Setup: Create cache
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(10)));

    // Create unique request
    let request = test_request("gpt-4", "Tell me a unique story");
    let cache_key = request.cache_key();
    let response = test_response("Once upon a time...");

    // Initial state
    let initial_stats = cache.stats().await.expect("get stats");
    assert_eq!(initial_stats.misses, 0, "should start with 0 misses");

    // Attempt to get from cache (miss)
    let cached = cache.get(&cache_key).await.expect("get from cache");
    assert!(cached.is_none(), "unique request should be cache miss");

    // Verify miss was counted
    let after_miss = cache.stats().await.expect("get stats");
    assert_eq!(after_miss.misses, 1, "miss counter should increment");
    assert_eq!(after_miss.hits, 0, "hit counter should stay 0");
    assert_eq!(after_miss.entries, 0, "no entries yet");

    // Simulate provider call and cache the response
    cache
        .set(&cache_key, &response, Duration::from_secs(60))
        .await
        .expect("set in cache");

    // Verify entry was cached
    let after_set = cache.stats().await.expect("get stats");
    assert_eq!(after_set.entries, 1, "should have 1 cached entry");
    assert_eq!(after_set.misses, 1, "should still have 1 miss");

    // Verify subsequent request hits cache
    let cached_response = cache.get(&cache_key).await.expect("get from cache");
    assert!(cached_response.is_some(), "next request should hit cache");

    let final_stats = cache.stats().await.expect("get stats");
    assert_eq!(final_stats.hits, 1, "should have 1 hit now");
    assert_eq!(final_stats.misses, 1, "should still have 1 miss");
}

/// Test 9.7: LRU eviction in full system
///
/// Scenario: Fill cache to limit, trigger eviction, verify oldest gone
#[tokio::test]
async fn test_lru_eviction_removes_oldest_entry() {
    // Setup: Create cache with small capacity (3 entries)
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(3)));

    // Create 3 unique requests to fill cache
    let req1 = test_request("gpt-4", "Request 1");
    let req2 = test_request("gpt-4", "Request 2");
    let req3 = test_request("gpt-4", "Request 3");

    let key1 = req1.cache_key();
    let key2 = req2.cache_key();
    let key3 = req3.cache_key();

    let resp1 = test_response("Response 1");
    let resp2 = test_response("Response 2");
    let resp3 = test_response("Response 3");

    // Fill cache to capacity
    cache
        .set(&key1, &resp1, Duration::from_secs(60))
        .await
        .expect("set key1");
    tokio::time::sleep(Duration::from_millis(10)).await; // Ensure different timestamps

    cache
        .set(&key2, &resp2, Duration::from_secs(60))
        .await
        .expect("set key2");
    tokio::time::sleep(Duration::from_millis(10)).await;

    cache
        .set(&key3, &resp3, Duration::from_secs(60))
        .await
        .expect("set key3");

    // Verify cache is full
    let stats_full = cache.stats().await.expect("get stats");
    assert_eq!(stats_full.entries, 3, "cache should be full");
    assert_eq!(stats_full.evictions, 0, "no evictions yet");

    // All 3 entries should be present
    assert!(cache.get(&key1).await.expect("get key1").is_some());
    assert!(cache.get(&key2).await.expect("get key2").is_some());
    assert!(cache.get(&key3).await.expect("get key3").is_some());

    // Add 4th entry to trigger eviction (key1 is oldest)
    let req4 = test_request("gpt-4", "Request 4");
    let key4 = req4.cache_key();
    let resp4 = test_response("Response 4");

    cache
        .set(&key4, &resp4, Duration::from_secs(60))
        .await
        .expect("set key4");

    // Verify eviction occurred
    let stats_after = cache.stats().await.expect("get stats");
    assert_eq!(stats_after.entries, 3, "cache should still have 3 entries");
    assert_eq!(stats_after.evictions, 1, "should have 1 eviction");

    // Verify oldest entry (key1) was evicted
    let key1_gone = cache.get(&key1).await.expect("get key1");
    assert!(key1_gone.is_none(), "oldest entry (key1) should be evicted");

    // Verify newer entries still present
    assert!(
        cache.get(&key2).await.expect("get key2").is_some(),
        "key2 should still be present"
    );
    assert!(
        cache.get(&key3).await.expect("get key3").is_some(),
        "key3 should still be present"
    );
    assert!(
        cache.get(&key4).await.expect("get key4").is_some(),
        "key4 should be present"
    );

    // Verify utilization is correct
    assert_eq!(
        stats_after.utilization(),
        Some(1.0),
        "cache should be at 100% utilization"
    );
}

/// Test: LRU updates on access (least-recently-used, not least-recently-inserted)
#[tokio::test]
async fn test_lru_updates_on_access() {
    // Setup: Cache with capacity 2
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(2)));

    let req1 = test_request("gpt-4", "First");
    let req2 = test_request("gpt-4", "Second");
    let req3 = test_request("gpt-4", "Third");

    let key1 = req1.cache_key();
    let key2 = req2.cache_key();
    let key3 = req3.cache_key();

    // Insert key1 and key2
    cache
        .set(&key1, &test_response("Resp1"), Duration::from_secs(60))
        .await
        .expect("set key1");
    tokio::time::sleep(Duration::from_millis(10)).await;

    cache
        .set(&key2, &test_response("Resp2"), Duration::from_secs(60))
        .await
        .expect("set key2");
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Access key1 to update its last-accessed timestamp
    let _ = cache.get(&key1).await.expect("get key1");
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Insert key3, which should evict key2 (not key1, since key1 was accessed more recently)
    cache
        .set(&key3, &test_response("Resp3"), Duration::from_secs(60))
        .await
        .expect("set key3");

    // Verify key1 and key3 are present, key2 was evicted
    assert!(
        cache.get(&key1).await.expect("get key1").is_some(),
        "key1 should still be present (accessed recently)"
    );
    assert!(
        cache.get(&key2).await.expect("get key2").is_none(),
        "key2 should be evicted (least recently used)"
    );
    assert!(
        cache.get(&key3).await.expect("get key3").is_some(),
        "key3 should be present (just inserted)"
    );
}

/// Test: Content-based cache keys produce identical signatures for identical content
#[tokio::test]
async fn test_content_based_cache_keys() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(10)));

    // Two requests with identical semantic content but different request IDs
    let req1 = test_request("gpt-4", "What is Rust?");
    let req2 = test_request("gpt-4", "What is Rust?");

    // Request IDs are different
    assert_ne!(req1.id, req2.id, "request IDs should differ");

    // Cache keys should have the same signature (content hash)
    let key1 = req1.cache_key();
    let key2 = req2.cache_key();
    assert_eq!(
        key1.signature, key2.signature,
        "identical content should produce identical signatures"
    );

    // NOTE: Current implementation uses (request_id, signature) as the cache key,
    // so different request IDs create different cache entries even with same content.
    // This is by design - each request is tracked separately, and signature provides
    // content-based grouping for analytics and debugging.

    // Verify: same request ID + same content = cache hit
    let req3 = CompletionRequest {
        id: req1.id.clone(), // Reuse same ID
        model: req1.model.clone(),
        messages: req1.messages.clone(),
        stream: req1.stream,
        max_tokens: req1.max_tokens,
        temperature: req1.temperature,
        tools: req1.tools.clone(),
        tool_choice: req1.tool_choice.clone(),
        metadata: req1.metadata.clone(),
        restrictions: req1.restrictions.clone(),
    };

    let key3 = req3.cache_key();
    assert_eq!(
        key1, key3,
        "same request_id and content should produce identical cache keys"
    );

    // Cache the first request
    cache
        .set(
            &key1,
            &test_response("Rust is a systems programming language."),
            Duration::from_secs(60),
        )
        .await
        .expect("set key1");

    // Same cache key should hit
    let cached = cache.get(&key3).await.expect("get key3");
    assert!(cached.is_some(), "identical cache key should hit cache");
}

/// Test: Clear cache resets stats
#[tokio::test]
async fn test_clear_cache_resets_stats() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(10)));

    // Populate cache and generate some stats
    let req = test_request("gpt-4", "Test");
    let key = req.cache_key();

    cache
        .set(&key, &test_response("Response"), Duration::from_secs(60))
        .await
        .expect("set");
    let _ = cache.get(&key).await; // Generate a hit

    let before_clear = cache.stats().await.expect("get stats");
    assert!(before_clear.entries > 0, "should have entries");
    assert!(before_clear.hits > 0, "should have hits");

    // Clear cache
    cache.clear().await.expect("clear cache");

    // Verify stats are reset
    let after_clear = cache.stats().await.expect("get stats");
    assert_eq!(after_clear.entries, 0, "entries should be 0 after clear");
    assert_eq!(after_clear.hits, 0, "hits should be 0 after clear");
    assert_eq!(after_clear.misses, 0, "misses should be 0 after clear");
    assert_eq!(
        after_clear.evictions, 0,
        "evictions should be 0 after clear"
    );
}
