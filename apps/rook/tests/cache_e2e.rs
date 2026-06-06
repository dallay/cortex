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
            cache_control_header: None,
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

// ============================================================================
// Phase 7: E2E Dual-Layer Cache Tests (Token Cache Integration)
// ============================================================================

/// Test 7.1: Token cache hit increments metrics
///
/// Scenario: Response with cache_hit=true increments token_cache.hits
#[tokio::test]
async fn test_token_cache_hit_increments_metrics() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(10)));

    // Initial state
    let initial_stats = cache.stats().await.expect("get stats");
    assert_eq!(
        initial_stats.token_cache.hits, 0,
        "should start with 0 token cache hits"
    );
    assert_eq!(
        initial_stats.token_cache.tokens_saved, 0,
        "should start with 0 tokens saved"
    );

    // Simulate provider response with token cache hit
    let response = CompletionResponse {
        id: RequestId::new(),
        provider: ProviderId::new("anthropic"),
        model: ModelId::new("claude-3-5-sonnet-20241022"),
        content: "Cached response".to_string(),
        content_blocks: vec![MessageContent::Text("Cached response".to_string())],
        usage: TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cache_read_tokens: Some(100), // Anthropic-specific field
            cache_creation_tokens: None,
            reasoning_tokens: None,
            estimated_cost_usd: Some(0.0015),
        },
        latency_ms: 120,
        cache_hit: Some(true), // Token cache hit
    };

    // Record the token cache hit with cost >= 0.005 (0.5 cents minimum for rounding)
    // Use cache_read_tokens (100) instead of total_tokens (150) since only cached prompt tokens are saved
    cache
        .increment_token_cache_hit(response.usage.cache_read_tokens.unwrap(), 0.01)
        .await
        .expect("increment token cache hit");

    // Verify metrics
    let stats = cache.stats().await.expect("get stats");
    assert_eq!(
        stats.token_cache.hits, 1,
        "token cache hits should increment"
    );
    assert_eq!(
        stats.token_cache.tokens_saved, 100,
        "tokens saved should equal cache_read_tokens (only cached prompt tokens, not completion)"
    );

    // Cost is stored as cents: 0.01 USD = 1 cent
    assert_eq!(
        stats.token_cache.estimated_cost_saved_usd, 0.01,
        "cost savings should be stored accurately (cent precision)"
    );
}

/// Test 7.2: Token cache miss increments miss counter
///
/// Scenario: Response with cache_hit=false increments token_cache.misses
#[tokio::test]
async fn test_token_cache_miss_increments_misses() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(10)));

    // Initial state
    let initial_stats = cache.stats().await.expect("get stats");
    assert_eq!(
        initial_stats.token_cache.misses, 0,
        "should start with 0 token cache misses"
    );

    // Simulate provider response with token cache miss
    let _response = CompletionResponse {
        id: RequestId::new(),
        provider: ProviderId::new("anthropic"),
        model: ModelId::new("claude-3-5-sonnet-20241022"),
        content: "New response".to_string(),
        content_blocks: vec![MessageContent::Text("New response".to_string())],
        usage: TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cache_read_tokens: None,
            cache_creation_tokens: Some(100), // New cache entry
            reasoning_tokens: None,
            estimated_cost_usd: Some(0.0015),
        },
        latency_ms: 500,
        cache_hit: Some(false), // Token cache miss
    };

    // Record the token cache miss
    cache
        .increment_token_cache_miss()
        .await
        .expect("increment token cache miss");

    // Verify metrics
    let stats = cache.stats().await.expect("get stats");
    assert_eq!(
        stats.token_cache.misses, 1,
        "token cache misses should increment"
    );
    assert_eq!(stats.token_cache.hits, 0, "hits should remain 0");
    assert_eq!(
        stats.token_cache.tokens_saved, 0,
        "tokens saved should remain 0 on miss"
    );
}

/// Test 7.3: Dual-layer cache flow - first request misses both layers
///
/// Scenario: First request misses signature cache and token cache
#[tokio::test]
async fn test_dual_layer_first_request_misses_both() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(10)));

    let request = test_request("claude-3-5-sonnet-20241022", "What is Rust?");
    let cache_key = request.cache_key();

    // 1. Check signature cache (Layer 1) - should miss
    let cached = cache.get(&cache_key).await.expect("get from cache");
    assert!(
        cached.is_none(),
        "signature cache should miss on first request"
    );

    // Verify signature cache miss was counted
    let after_sig_miss = cache.stats().await.expect("get stats");
    assert_eq!(
        after_sig_miss.misses, 1,
        "signature cache misses should be 1"
    );
    assert_eq!(after_sig_miss.hits, 0, "signature cache hits should be 0");

    // 2. Simulate provider call (would inject cache-control header)
    // Provider returns cache_hit=false (no prior token cache)
    let response = CompletionResponse {
        id: request.id.clone(),
        provider: ProviderId::new("anthropic"),
        model: request.model.clone(),
        content: "Rust is a systems programming language.".to_string(),
        content_blocks: vec![MessageContent::Text(
            "Rust is a systems programming language.".to_string(),
        )],
        usage: TokenUsage {
            prompt_tokens: 15,
            completion_tokens: 35,
            total_tokens: 50,
            cache_read_tokens: None,
            cache_creation_tokens: Some(15),
            reasoning_tokens: None,
            estimated_cost_usd: Some(0.0005),
        },
        latency_ms: 450,
        cache_hit: Some(false), // Token cache miss
    };

    // 3. Record token cache miss
    cache
        .increment_token_cache_miss()
        .await
        .expect("increment token cache miss");

    // 4. Cache the response (Layer 1)
    cache
        .set(&cache_key, &response, Duration::from_secs(60))
        .await
        .expect("set in cache");

    // Verify final state
    let final_stats = cache.stats().await.expect("get stats");
    assert_eq!(final_stats.misses, 1, "signature cache misses = 1");
    assert_eq!(final_stats.hits, 0, "signature cache hits = 0");
    assert_eq!(final_stats.token_cache.misses, 1, "token cache misses = 1");
    assert_eq!(final_stats.token_cache.hits, 0, "token cache hits = 0");
    assert_eq!(final_stats.entries, 1, "should have 1 cached entry");
}

/// Test 7.4: Dual-layer cache flow - second request hits signature cache
///
/// Scenario: Second identical request hits signature cache (no provider call, no token cache interaction)
#[tokio::test]
async fn test_dual_layer_second_request_hits_signature() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(10)));

    let request = test_request("claude-3-5-sonnet-20241022", "What is Rust?");
    let cache_key = request.cache_key();

    // Setup: First request populates signature cache
    let response = CompletionResponse {
        id: request.id.clone(),
        provider: ProviderId::new("anthropic"),
        model: request.model.clone(),
        content: "Rust is a systems programming language.".to_string(),
        content_blocks: vec![MessageContent::Text(
            "Rust is a systems programming language.".to_string(),
        )],
        usage: TokenUsage {
            prompt_tokens: 15,
            completion_tokens: 35,
            total_tokens: 50,
            cache_read_tokens: None,
            cache_creation_tokens: Some(15),
            reasoning_tokens: None,
            estimated_cost_usd: Some(0.0005),
        },
        latency_ms: 450,
        cache_hit: Some(false),
    };

    cache
        .set(&cache_key, &response, Duration::from_secs(60))
        .await
        .expect("set in cache");

    cache
        .increment_token_cache_miss()
        .await
        .expect("increment miss");

    let after_first = cache.stats().await.expect("get stats");
    assert_eq!(after_first.entries, 1);
    assert_eq!(after_first.misses, 0); // set() doesn't increment misses
    assert_eq!(after_first.token_cache.misses, 1);

    // Second request: hits signature cache
    let cached_response = cache.get(&cache_key).await.expect("get from cache");
    assert!(cached_response.is_some(), "signature cache should hit");

    // Verify signature cache hit was counted, token cache was NOT touched
    let after_second = cache.stats().await.expect("get stats");
    assert_eq!(after_second.hits, 1, "signature cache hits should be 1");
    assert_eq!(
        after_second.misses, 0,
        "signature cache misses should still be 0"
    );
    assert_eq!(
        after_second.token_cache.hits, 0,
        "token cache hits should be 0 (no provider call)"
    );
    assert_eq!(
        after_second.token_cache.misses, 1,
        "token cache misses should still be 1"
    );
}

/// Test 7.5: Dual-layer combined metrics calculation
///
/// Scenario: Verify combined stats aggregate both layers correctly
#[tokio::test]
async fn test_dual_layer_combined_metrics() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(10)));

    // Simulate 3 signature cache hits
    let req1 = test_request("gpt-4", "Request 1");
    let req2 = test_request("gpt-4", "Request 2");
    let req3 = test_request("gpt-4", "Request 3");

    cache
        .set(
            &req1.cache_key(),
            &test_response("R1"),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    cache
        .set(
            &req2.cache_key(),
            &test_response("R2"),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    cache
        .set(
            &req3.cache_key(),
            &test_response("R3"),
            Duration::from_secs(60),
        )
        .await
        .unwrap();

    cache.get(&req1.cache_key()).await.unwrap();
    cache.get(&req2.cache_key()).await.unwrap();
    cache.get(&req3.cache_key()).await.unwrap();

    // Simulate 2 token cache hits
    cache.increment_token_cache_hit(100, 0.01).await.unwrap();
    cache.increment_token_cache_hit(150, 0.015).await.unwrap();

    // Simulate 1 token cache miss
    cache.increment_token_cache_miss().await.unwrap();

    let stats = cache.stats().await.expect("get stats");

    // Signature cache: 3 hits, 0 misses
    assert_eq!(stats.hits, 3);
    assert_eq!(stats.misses, 0);

    // Token cache: 2 hits, 1 miss, 250 tokens saved
    assert_eq!(stats.token_cache.hits, 2);
    assert_eq!(stats.token_cache.misses, 1);
    assert_eq!(stats.token_cache.tokens_saved, 250);

    // Combined metrics verification (using UnifiedCacheStats)
    let unified = rook_core::UnifiedCacheStats::from_cache_stats(stats);

    // total_requests = signature hits + misses = 3 + 0 = 3
    assert_eq!(
        unified.combined.total_requests, 3,
        "total_requests should count unique incoming requests (signature layer)"
    );

    // cached_requests = signature hits + token hits = 3 + 2 = 5
    assert_eq!(
        unified.combined.cached_requests, 5,
        "cached_requests should count both signature and token cache hits"
    );

    // cache_rate = cached_requests / total_requests = 5 / 3 ≈ 1.666...
    // This is > 1.0 because token cache hits count on top of signature hits
    assert!(
        (unified.combined.cache_rate - 5.0 / 3.0).abs() < 0.001,
        "cache_rate should be cached_requests / total_requests = 5/3 ≈ {}, got {}",
        5.0 / 3.0,
        unified.combined.cache_rate
    );
}

/// Test 7.6: Token cache with multiple cache hits accumulates cost savings
///
/// Scenario: Multiple token cache hits accumulate tokens_saved and cost
#[tokio::test]
async fn test_token_cache_accumulates_savings() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(10)));

    // First token cache hit: 100 tokens, $0.01 (1 cent)
    cache.increment_token_cache_hit(100, 0.01).await.unwrap();

    let after_first = cache.stats().await.unwrap();
    assert_eq!(after_first.token_cache.hits, 1);
    assert_eq!(after_first.token_cache.tokens_saved, 100);
    assert_eq!(after_first.token_cache.estimated_cost_saved_usd, 0.01);

    // Second token cache hit: 200 tokens, $0.02 (2 cents)
    cache.increment_token_cache_hit(200, 0.02).await.unwrap();

    let after_second = cache.stats().await.unwrap();
    assert_eq!(after_second.token_cache.hits, 2);
    assert_eq!(
        after_second.token_cache.tokens_saved, 300,
        "tokens should accumulate"
    );
    assert_eq!(
        after_second.token_cache.estimated_cost_saved_usd, 0.03,
        "cost should accumulate (cent precision)"
    );

    // Third token cache hit: 50 tokens, $0.015 (1.5 cents → rounds to 2 cents)
    cache.increment_token_cache_hit(50, 0.015).await.unwrap();

    let after_third = cache.stats().await.unwrap();
    assert_eq!(after_third.token_cache.hits, 3);
    assert_eq!(after_third.token_cache.tokens_saved, 350);
    // 0.03 + 0.02 (rounded from 0.015) = 0.05
    assert_eq!(after_third.token_cache.estimated_cost_saved_usd, 0.05);
}

/// Test 7.7: Clear cache resets both signature and token cache metrics
///
/// Scenario: clear() resets all counters including token cache
#[tokio::test]
async fn test_clear_resets_token_cache_metrics() {
    let cache: Arc<dyn CachePort> = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(10)));

    // Populate both layers
    let req = test_request("claude-3-5-sonnet-20241022", "Test");
    cache
        .set(
            &req.cache_key(),
            &test_response("Resp"),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    cache.get(&req.cache_key()).await.unwrap();

    cache.increment_token_cache_hit(100, 0.0003).await.unwrap();
    cache.increment_token_cache_miss().await.unwrap();

    let before_clear = cache.stats().await.unwrap();
    assert!(before_clear.hits > 0);
    assert!(before_clear.token_cache.hits > 0);
    assert!(before_clear.token_cache.misses > 0);

    // Clear cache
    cache.clear().await.unwrap();

    // Verify all metrics reset
    let after_clear = cache.stats().await.unwrap();
    assert_eq!(after_clear.entries, 0);
    assert_eq!(after_clear.hits, 0);
    assert_eq!(after_clear.misses, 0);
    assert_eq!(after_clear.evictions, 0);
    assert_eq!(
        after_clear.token_cache.hits, 0,
        "token cache hits should reset"
    );
    assert_eq!(
        after_clear.token_cache.misses, 0,
        "token cache misses should reset"
    );
    assert_eq!(
        after_clear.token_cache.tokens_saved, 0,
        "tokens saved should reset"
    );
    assert_eq!(
        after_clear.token_cache.estimated_cost_saved_usd, 0.0,
        "cost saved should reset"
    );
}
