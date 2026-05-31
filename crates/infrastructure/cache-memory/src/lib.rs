// cache-memory — in-memory implementation of CachePort

use async_trait::async_trait;
use dashmap::DashMap;
use rook_core::{CacheKey, CachePort, CompletionResponse, CortexResult};
use std::time::{Duration, Instant};

/// Thread-safe in-memory cache with TTL support.
pub struct InMemoryCache {
    store: DashMap<CacheKey, CompletionResponse>,
    expiry: DashMap<CacheKey, Instant>,
}

impl InMemoryCache {
    pub fn new(_ttl: Duration) -> Self {
        Self {
            store: DashMap::new(),
            expiry: DashMap::new(),
        }
    }

    fn is_expired(&self, key: &CacheKey) -> bool {
        if let Some(expiry) = self.expiry.get(key) {
            Instant::now() > *expiry
        } else {
            true
        }
    }
}

#[async_trait]
impl CachePort for InMemoryCache {
    async fn get(&self, key: &CacheKey) -> CortexResult<Option<CompletionResponse>> {
        if self.is_expired(key) {
            self.store.remove(key);
            return Ok(None);
        }
        Ok(self.store.get(key).map(|r| r.clone()))
    }

    async fn set(
        &self,
        key: &CacheKey,
        value: &CompletionResponse,
        ttl: Duration,
    ) -> CortexResult<()> {
        self.store.insert(key.clone(), value.clone());
        self.expiry.insert(
            key.clone(),
            Instant::now().checked_add(ttl).unwrap_or(Instant::now()),
        );
        Ok(())
    }

    async fn delete(&self, key: &CacheKey) -> CortexResult<()> {
        self.store.remove(key);
        self.expiry.remove(key);
        Ok(())
    }

    async fn clear(&self) -> CortexResult<()> {
        self.store.clear();
        self.expiry.clear();
        Ok(())
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
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
                estimated_cost_usd: Some(0.001),
            },
            latency_ms: 50,
        }
    }

    /// Helper: build a CacheKey for testing
    fn make_key() -> CacheKey {
        CacheKey::from(&RequestId::new())
    }

    #[tokio::test]
    async fn cache_set_and_get() {
        let cache = InMemoryCache::new(Duration::from_secs(60));
        let key = make_key();
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
        let cache = InMemoryCache::new(Duration::from_secs(60));
        let key = make_key();

        let result = cache.get(&key).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn cache_delete() {
        let cache = InMemoryCache::new(Duration::from_secs(60));
        let key = make_key();
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
        let cache = InMemoryCache::new(Duration::from_secs(60));
        let key1 = make_key();
        let key2 = make_key();

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
        let cache = InMemoryCache::new(Duration::from_secs(0)); // TTL = 0 = already expired
        let key = make_key();
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
        let cache = InMemoryCache::new(Duration::from_secs(60));
        let key = make_key();

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
        let cache = InMemoryCache::new(Duration::from_secs(60));
        let key_a = CacheKey::from(&RequestId::new());
        let key_b = CacheKey::from(&RequestId::new());

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
}
