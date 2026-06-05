// cache-memory — in-memory implementation of CachePort

use async_trait::async_trait;
use dashmap::DashMap;
use rook_core::{CacheKey, CachePort, CacheStats, CompletionResponse, CortexResult};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Thread-safe in-memory cache with TTL support, LRU eviction, and stats tracking.
pub struct InMemoryCache {
    store: DashMap<CacheKey, CompletionResponse>,
    expiry: DashMap<CacheKey, Instant>,
    last_accessed: DashMap<CacheKey, Instant>,
    max_entries: Option<usize>,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl InMemoryCache {
    pub fn new(_ttl: Duration, max_entries: Option<usize>) -> Self {
        Self {
            store: DashMap::new(),
            expiry: DashMap::new(),
            last_accessed: DashMap::new(),
            max_entries,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    fn is_expired(&self, key: &CacheKey) -> bool {
        if let Some(expiry) = self.expiry.get(key) {
            Instant::now() > *expiry
        } else {
            true
        }
    }

    /// Returns current cache statistics
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            entries: self.store.len() as u64,
            max_entries: self.max_entries.unwrap_or(0) as u64,
        }
    }

    /// Delete all entries matching the given signature.
    /// Returns the number of entries deleted.
    pub fn delete_by_signature(&self, signature: &str) -> usize {
        let mut deleted = 0;
        // Collect keys matching the signature
        let keys_to_delete: Vec<CacheKey> = self
            .store
            .iter()
            .filter(|entry| entry.key().signature == signature)
            .map(|entry| entry.key().clone())
            .collect();

        // Delete all matching keys
        for key in keys_to_delete {
            // Only increment deleted if an entry was actually removed
            if self.store.remove(&key).is_some() {
                deleted += 1;
            }
            // Clean up associated metadata regardless
            self.expiry.remove(&key);
            self.last_accessed.remove(&key);
        }

        deleted
    }

    /// Evict the least recently used entry if cache is at capacity.
    ///
    /// **Note on concurrency**: LRU eviction is approximate under concurrent access.
    /// This method iterates `last_accessed` to find the oldest entry, but another
    /// thread may access or modify an entry between selection and removal. Therefore,
    /// strict LRU ordering is not guaranteed without heavier locking. This trade-off
    /// is acceptable for performance; the eviction counter (`evictions`) tracks
    /// actual evictions regardless of ordering precision.
    fn evict_if_needed(&self) {
        if let Some(max) = self.max_entries {
            if self.store.len() >= max {
                // Find the oldest entry by last_accessed timestamp
                if let Some(oldest) = self
                    .last_accessed
                    .iter()
                    .min_by_key(|entry| *entry.value())
                    .map(|entry| entry.key().clone())
                {
                    // Only evict if the entry still exists (check store.remove result)
                    if self.store.remove(&oldest).is_some() {
                        self.expiry.remove(&oldest);
                        self.last_accessed.remove(&oldest);
                        self.evictions.fetch_add(1, Ordering::Relaxed);
                        // Emit Prometheus metric
                        metrics::counter!("rook_cache_evictions").increment(1);
                    }
                }
            }
        }
    }
}

#[async_trait]
impl CachePort for InMemoryCache {
    async fn get(&self, key: &CacheKey) -> CortexResult<Option<CompletionResponse>> {
        if self.is_expired(key) {
            self.store.remove(key);
            self.expiry.remove(key);
            self.last_accessed.remove(key);
            self.misses.fetch_add(1, Ordering::Relaxed);
            return Ok(None);
        }

        if let Some(response) = self.store.get(key).map(|r| r.clone()) {
            // Update last_accessed on hit
            self.last_accessed.insert(key.clone(), Instant::now());
            self.hits.fetch_add(1, Ordering::Relaxed);
            Ok(Some(response))
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            Ok(None)
        }
    }

    async fn set(
        &self,
        key: &CacheKey,
        value: &CompletionResponse,
        ttl: Duration,
    ) -> CortexResult<()> {
        // Only evict if inserting a new key (not when overwriting)
        if !self.store.contains_key(key) {
            self.evict_if_needed();
        }

        self.store.insert(key.clone(), value.clone());
        self.expiry.insert(
            key.clone(),
            Instant::now().checked_add(ttl).unwrap_or(Instant::now()),
        );
        self.last_accessed.insert(key.clone(), Instant::now());
        Ok(())
    }

    async fn delete(&self, key: &CacheKey) -> CortexResult<()> {
        self.store.remove(key);
        self.expiry.remove(key);
        self.last_accessed.remove(key);
        Ok(())
    }

    async fn clear(&self) -> CortexResult<()> {
        self.store.clear();
        self.expiry.clear();
        self.last_accessed.clear();
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
        Ok(())
    }

    async fn stats(&self) -> CortexResult<CacheStats> {
        Ok(self.stats())
    }

    async fn delete_by_signature(&self, signature: &str) -> CortexResult<usize> {
        Ok(self.delete_by_signature(signature))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rook_core::{CompletionResponse, ModelId, ProviderId, RequestId, TokenUsage};
    use std::time::Duration;

    /// Helper: build a CompletionResponse for testing
    fn make_response(content: &str) -> CompletionResponse {
        CompletionResponse {
            id: RequestId::new(),
            provider: ProviderId::new("test-provider"),
            model: ModelId::new("test-model"),
            content: content.into(),
            content_blocks: vec![rook_core::MessageContent::Text(content.to_string())],
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
        }
    }

    /// Helper: build a CacheKey for testing
    fn make_key(sig: &str) -> CacheKey {
        CacheKey::test_key(RequestId::new(), sig.to_string())
    }

    #[tokio::test]
    async fn cache_set_and_get() {
        let cache = InMemoryCache::new(Duration::from_secs(60), None);
        let key = make_key("test_sig_1");
        let resp = make_response("hello world");

        cache
            .set(&key, &resp, Duration::from_secs(60))
            .await
            .unwrap();
        let result = cache.get(&key).await.unwrap();

        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.content, "hello world");
        assert_eq!(r.provider.as_str(), "test-provider");
    }

    #[tokio::test]
    async fn cache_get_missing_returns_none() {
        let cache = InMemoryCache::new(Duration::from_secs(60), None);
        let key = make_key("missing_sig");

        let result = cache.get(&key).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn cache_delete() {
        let cache = InMemoryCache::new(Duration::from_secs(60), None);
        let key = make_key("delete_sig");
        let resp = make_response("to be deleted");

        cache
            .set(&key, &resp, Duration::from_secs(60))
            .await
            .unwrap();
        cache.delete(&key).await.unwrap();

        let result = cache.get(&key).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn cache_clear() {
        let cache = InMemoryCache::new(Duration::from_secs(60), None);
        let key1 = make_key("clear_sig_1");
        let key2 = make_key("clear_sig_2");

        cache
            .set(&key1, &make_response("value1"), Duration::from_secs(60))
            .await
            .unwrap();
        cache
            .set(&key2, &make_response("value2"), Duration::from_secs(60))
            .await
            .unwrap();

        cache.clear().await.unwrap();

        assert!(cache.get(&key1).await.unwrap().is_none());
        assert!(cache.get(&key2).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cache_expired_key_returns_none() {
        let cache = InMemoryCache::new(Duration::from_secs(0), None); // TTL = 0 = already expired
        let key = make_key("expired_sig");
        let resp = make_response("expired content");

        cache
            .set(&key, &resp, Duration::from_secs(0))
            .await
            .unwrap();
        // Give the expiry check a chance to trigger
        tokio::time::sleep(Duration::from_millis(1)).await;

        let result = cache.get(&key).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn cache_overwrite_same_key() {
        let cache = InMemoryCache::new(Duration::from_secs(60), None);
        let key = make_key("overwrite_sig");

        cache
            .set(&key, &make_response("first"), Duration::from_secs(60))
            .await
            .unwrap();
        cache
            .set(&key, &make_response("second"), Duration::from_secs(60))
            .await
            .unwrap();

        let result = cache.get(&key).await.unwrap().unwrap();
        assert_eq!(result.content, "second");
    }

    #[tokio::test]
    async fn cache_multiple_keys_independent() {
        let cache = InMemoryCache::new(Duration::from_secs(60), None);
        let key_a = make_key("independent_a");
        let key_b = make_key("independent_b");

        cache
            .set(&key_a, &make_response("content A"), Duration::from_secs(60))
            .await
            .unwrap();
        cache
            .set(&key_b, &make_response("content B"), Duration::from_secs(60))
            .await
            .unwrap();

        assert_eq!(
            cache.get(&key_a).await.unwrap().unwrap().content,
            "content A"
        );
        assert_eq!(
            cache.get(&key_b).await.unwrap().unwrap().content,
            "content B"
        );

        cache.delete(&key_a).await.unwrap();
        assert!(cache.get(&key_a).await.unwrap().is_none());
        assert!(cache.get(&key_b).await.unwrap().is_some());
    }

    // -------------------------------------------------------------------------
    // LRU Eviction Tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn lru_eviction_when_at_capacity() {
        let cache = InMemoryCache::new(Duration::from_secs(60), Some(3));

        let key1 = make_key("lru_sig_1");
        let key2 = make_key("lru_sig_2");
        let key3 = make_key("lru_sig_3");
        let key4 = make_key("lru_sig_4");

        // Fill cache to capacity
        cache
            .set(&key1, &make_response("value1"), Duration::from_secs(60))
            .await
            .unwrap();
        cache
            .set(&key2, &make_response("value2"), Duration::from_secs(60))
            .await
            .unwrap();
        cache
            .set(&key3, &make_response("value3"), Duration::from_secs(60))
            .await
            .unwrap();

        // Insert one more - should evict oldest (key1)
        cache
            .set(&key4, &make_response("value4"), Duration::from_secs(60))
            .await
            .unwrap();

        // key1 should be evicted, others should exist
        assert!(cache.get(&key1).await.unwrap().is_none());
        assert!(cache.get(&key2).await.unwrap().is_some());
        assert!(cache.get(&key3).await.unwrap().is_some());
        assert!(cache.get(&key4).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn lru_eviction_respects_access_order() {
        let cache = InMemoryCache::new(Duration::from_secs(60), Some(3));

        let key1 = make_key("access_sig_1");
        let key2 = make_key("access_sig_2");
        let key3 = make_key("access_sig_3");
        let key4 = make_key("access_sig_4");

        // Fill cache
        cache
            .set(&key1, &make_response("value1"), Duration::from_secs(60))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        cache
            .set(&key2, &make_response("value2"), Duration::from_secs(60))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        cache
            .set(&key3, &make_response("value3"), Duration::from_secs(60))
            .await
            .unwrap();

        // Access key1 to make it more recent
        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = cache.get(&key1).await.unwrap();

        // Insert key4 - should evict key2 (oldest accessed, not key1)
        tokio::time::sleep(Duration::from_millis(10)).await;
        cache
            .set(&key4, &make_response("value4"), Duration::from_secs(60))
            .await
            .unwrap();

        assert!(cache.get(&key1).await.unwrap().is_some());
        assert!(cache.get(&key2).await.unwrap().is_none());
        assert!(cache.get(&key3).await.unwrap().is_some());
        assert!(cache.get(&key4).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn lru_no_eviction_below_capacity() {
        let cache = InMemoryCache::new(Duration::from_secs(60), Some(5));

        let key1 = make_key("below_cap_1");
        let key2 = make_key("below_cap_2");
        let key3 = make_key("below_cap_3");

        cache
            .set(&key1, &make_response("value1"), Duration::from_secs(60))
            .await
            .unwrap();
        cache
            .set(&key2, &make_response("value2"), Duration::from_secs(60))
            .await
            .unwrap();
        cache
            .set(&key3, &make_response("value3"), Duration::from_secs(60))
            .await
            .unwrap();

        // All should still exist
        assert!(cache.get(&key1).await.unwrap().is_some());
        assert!(cache.get(&key2).await.unwrap().is_some());
        assert!(cache.get(&key3).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn lru_unlimited_cache_never_evicts() {
        let cache = InMemoryCache::new(Duration::from_secs(60), None);

        // Create keys first to reuse them
        let keys: Vec<CacheKey> = (0..100)
            .map(|i| make_key(&format!("unlimited_{}", i)))
            .collect();

        // Add many entries
        for (i, key) in keys.iter().enumerate() {
            cache
                .set(
                    key,
                    &make_response(&format!("value{}", i)),
                    Duration::from_secs(60),
                )
                .await
                .unwrap();
        }

        // All should still exist
        for key in &keys {
            assert!(cache.get(key).await.unwrap().is_some());
        }
    }

    // -------------------------------------------------------------------------
    // Stats Tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn stats_track_hits_and_misses() {
        let cache = InMemoryCache::new(Duration::from_secs(60), None);
        let key = make_key("stats_sig");

        // Initial stats
        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);

        // Miss
        let _ = cache.get(&key).await.unwrap();
        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 1);

        // Set and hit
        cache
            .set(&key, &make_response("value"), Duration::from_secs(60))
            .await
            .unwrap();
        let _ = cache.get(&key).await.unwrap();
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);

        // Another hit
        let _ = cache.get(&key).await.unwrap();
        let stats = cache.stats();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
    }

    #[tokio::test]
    async fn stats_track_evictions() {
        let cache = InMemoryCache::new(Duration::from_secs(60), Some(2));

        let key1 = make_key("evict_stat_1");
        let key2 = make_key("evict_stat_2");
        let key3 = make_key("evict_stat_3");

        cache
            .set(&key1, &make_response("value1"), Duration::from_secs(60))
            .await
            .unwrap();
        cache
            .set(&key2, &make_response("value2"), Duration::from_secs(60))
            .await
            .unwrap();

        let stats = cache.stats();
        assert_eq!(stats.evictions, 0);

        // Trigger eviction
        cache
            .set(&key3, &make_response("value3"), Duration::from_secs(60))
            .await
            .unwrap();

        let stats = cache.stats();
        assert_eq!(stats.evictions, 1);
    }

    #[tokio::test]
    async fn stats_track_entries() {
        let cache = InMemoryCache::new(Duration::from_secs(60), Some(10));

        let stats = cache.stats();
        assert_eq!(stats.entries, 0);

        let key1 = make_key("entry_1");
        let key2 = make_key("entry_2");

        cache
            .set(&key1, &make_response("value1"), Duration::from_secs(60))
            .await
            .unwrap();
        cache
            .set(&key2, &make_response("value2"), Duration::from_secs(60))
            .await
            .unwrap();

        let stats = cache.stats();
        assert_eq!(stats.entries, 2);

        cache.delete(&key1).await.unwrap();

        let stats = cache.stats();
        assert_eq!(stats.entries, 1);
    }

    #[tokio::test]
    async fn stats_reset_on_clear() {
        let cache = InMemoryCache::new(Duration::from_secs(60), Some(10));

        let key = make_key("clear_stats");
        cache
            .set(&key, &make_response("value"), Duration::from_secs(60))
            .await
            .unwrap();
        let _ = cache.get(&key).await.unwrap();
        let _ = cache.get(&make_key("miss")).await.unwrap();

        let stats = cache.stats();
        assert!(stats.hits > 0);
        assert!(stats.misses > 0);

        cache.clear().await.unwrap();

        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.evictions, 0);
        assert_eq!(stats.entries, 0);
    }

    #[tokio::test]
    async fn lru_no_eviction_on_overwrite() {
        let cache = InMemoryCache::new(Duration::from_secs(60), Some(3));

        // Fill cache to capacity
        let key1 = make_key("key1");
        let key2 = make_key("key2");
        let key3 = make_key("key3");

        cache
            .set(&key1, &make_response("value1"), Duration::from_secs(60))
            .await
            .unwrap();
        cache
            .set(&key2, &make_response("value2"), Duration::from_secs(60))
            .await
            .unwrap();
        cache
            .set(&key3, &make_response("value3"), Duration::from_secs(60))
            .await
            .unwrap();

        assert_eq!(cache.store.len(), 3);
        assert_eq!(cache.stats().evictions, 0);

        // Overwrite key1 — should NOT trigger eviction
        cache
            .set(
                &key1,
                &make_response("value1_updated"),
                Duration::from_secs(60),
            )
            .await
            .unwrap();

        assert_eq!(cache.store.len(), 3);
        assert_eq!(cache.stats().evictions, 0, "Overwriting should not evict");

        // Verify key1 was updated
        let result = cache.get(&key1).await.unwrap().unwrap();
        assert_eq!(result.content, "value1_updated");
    }

    // -------------------------------------------------------------------------
    // Concurrent Access Test
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn concurrent_access_no_panics() {
        use std::sync::Arc;

        let cache = Arc::new(InMemoryCache::new(Duration::from_secs(60), Some(50)));
        let mut handles = vec![];

        for i in 0..10 {
            let cache_clone = Arc::clone(&cache);
            let handle = tokio::spawn(async move {
                for j in 0..20 {
                    let key = make_key(&format!("concurrent_{}_{}", i, j));
                    cache_clone
                        .set(
                            &key,
                            &make_response(&format!("value_{}_{}", i, j)),
                            Duration::from_secs(60),
                        )
                        .await
                        .unwrap();
                    let _ = cache_clone.get(&key).await.unwrap();
                    if j % 5 == 0 {
                        cache_clone.delete(&key).await.unwrap();
                    }
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        // Should not panic and should have consistent state
        let stats = cache.stats();
        assert!(stats.hits > 0);
        assert!(stats.entries <= 50);
    }
}
