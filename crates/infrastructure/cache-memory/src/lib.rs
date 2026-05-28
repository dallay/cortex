// cache-memory — in-memory implementation of CachePort

use async_trait::async_trait;
use dashmap::DashMap;
use rook_core::{CacheKey, CachePort, CompletionResponse, NuxaResult};
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
    async fn get(&self, key: &CacheKey) -> NuxaResult<Option<CompletionResponse>> {
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
    ) -> NuxaResult<()> {
        self.store.insert(key.clone(), value.clone());
        self.expiry.insert(
            key.clone(),
            Instant::now().checked_add(ttl).unwrap_or(Instant::now()),
        );
        Ok(())
    }

    async fn delete(&self, key: &CacheKey) -> NuxaResult<()> {
        self.store.remove(key);
        self.expiry.remove(key);
        Ok(())
    }

    async fn clear(&self) -> NuxaResult<()> {
        self.store.clear();
        self.expiry.clear();
        Ok(())
    }
}
