// api_key_rate_limiter — per-key token bucket rate limiting for CLIENT_API routes
//
// Tracks rate limits by API key (X-Authz-Auth-ID header value).
// Falls back to IP-based limiting when no API key context is available.
// Tier-based limits: Free (100 cap, 10/s refill), Pro (1000 cap, 100/s refill),
// Enterprise (10000 cap, 1000/s refill).

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use rook_core::ApiKeyTier;
use tokio::sync::Mutex;

/// Rate limit exceeded error with retry-after information
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitExceeded {
    pub retry_after_secs: u64,
    pub limit: u64,
    pub remaining: u64,
    pub reset_unix: u64,
}

/// API key rate limiter result
#[derive(Debug, Clone)]
pub struct RateLimitSnapshot {
    pub limit: u64,
    pub remaining: u64,
    pub reset_unix: u64,
    pub retry_after_secs: u64,
}

/// Token bucket for a single key (API key ID or IP fallback)
#[derive(Debug, Clone)]
struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(capacity: u64) -> Self {
        Self {
            tokens: capacity as f64,
            last_refill: Instant::now(),
        }
    }

    /// Refill tokens based on elapsed time
    fn refill(&mut self, capacity: u64, refill_per_second: f64) {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        let refill_amount = elapsed * refill_per_second;
        self.tokens = (self.tokens + refill_amount).min(capacity as f64);
        self.last_refill = Instant::now();
    }

    /// Try to consume one token
    fn try_consume(&mut self, capacity: u64, refill_per_second: f64) -> bool {
        self.refill(capacity, refill_per_second);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Current token count
    fn tokens(&self) -> f64 {
        self.tokens
    }

    /// Time until next token is available
    fn retry_after(&self, _capacity: u64, refill_per_second: f64) -> Duration {
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            let tokens_needed = 1.0 - self.tokens;
            let secs = (tokens_needed / refill_per_second).ceil() as u64;
            Duration::from_secs(secs.max(1))
        }
    }
}

/// Per-key token bucket rate limiter for CLIENT_API routes
///
/// Uses the `X-Authz-Auth-ID` header (set by authz middleware after API key validation)
/// as the rate limit key. Falls back to IP-based limiting if no API key context.
#[derive(Debug, Clone)]
pub struct ApiKeyRateLimiter {
    buckets: Arc<Mutex<HashMap<String, TokenBucket>>>,
}

impl ApiKeyRateLimiter {
    /// Create a new API key rate limiter with empty bucket map
    pub fn new() -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check if a request from the given key/tier is allowed
    ///
    /// Returns `Ok(RateLimitSnapshot)` if allowed.
    /// Returns `Err(RateLimitExceeded)` if rate limited.
    pub async fn check(
        &self,
        key_id: &str,
        tier: ApiKeyTier,
        client_ip: Option<IpAddr>,
    ) -> Result<RateLimitSnapshot, RateLimitExceeded> {
        let (capacity, refill_per_second) = tier_params(tier);
        let use_key = key_id != "_anonymous";

        let lookup_key = if use_key {
            key_id.to_string()
        } else {
            // Fall back to IP-based limiting
            client_ip
                .map(|ip| format!("ip:{}", ip))
                .unwrap_or_else(|| "ip:unknown".to_string())
        };

        let mut buckets = self.buckets.lock().await;
        let bucket = buckets
            .entry(lookup_key.clone())
            .or_insert_with(|| TokenBucket::new(capacity));

        if bucket.try_consume(capacity, refill_per_second) {
            let remaining = bucket.tokens().floor() as u64;
            let reset_unix = unix_now() + refill_per_second.ceil() as u64;
            Ok(RateLimitSnapshot {
                limit: capacity,
                remaining,
                reset_unix,
                retry_after_secs: 0,
            })
        } else {
            let retry_after = bucket.retry_after(capacity, refill_per_second).as_secs();
            let reset_unix = unix_now() + retry_after;
            Err(RateLimitExceeded {
                retry_after_secs: retry_after,
                limit: capacity,
                remaining: 0,
                reset_unix,
            })
        }
    }

    /// Get current token count for a key (for testing/debugging)
    #[allow(dead_code)]
    pub async fn tokens_for(&self, key_id: &str) -> f64 {
        let buckets = self.buckets.lock().await;
        buckets
            .get(key_id)
            .map(|b| b.tokens())
            .unwrap_or(100.0) // Default capacity
    }

    /// Reset rate limit for a key (for testing)
    #[allow(dead_code)]
    pub async fn reset(&self, key_id: &str) {
        let mut buckets = self.buckets.lock().await;
        buckets.remove(key_id);
    }
}

impl Default for ApiKeyRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Get tier parameters (capacity, refill per second)
fn tier_params(tier: ApiKeyTier) -> (u64, f64) {
    match tier {
        ApiKeyTier::Free => (100, 10.0),
        ApiKeyTier::Pro => (1_000, 100.0),
        ApiKeyTier::Enterprise => (10_000, 1_000.0),
    }
}

/// Get current Unix timestamp
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn localhost() -> IpAddr {
        IpAddr::from([127, 0, 0, 1])
    }

    #[tokio::test]
    async fn first_request_is_allowed() {
        let limiter = ApiKeyRateLimiter::new();
        let result = limiter.check("key_1", ApiKeyTier::Free, Some(localhost())).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn allows_up_to_capacity_requests() {
        let limiter = ApiKeyRateLimiter::new();
        for _ in 0..100 {
            let result = limiter.check("key_1", ApiKeyTier::Free, Some(localhost()));
            assert!(result.await.is_ok(), "Should allow request up to capacity");
        }
    }

    #[tokio::test]
    async fn blocks_request_exceeding_capacity() {
        let limiter = ApiKeyRateLimiter::new();
        // Exhaust the bucket
        for _ in 0..100 {
            let _ = limiter.check("key_1", ApiKeyTier::Free, Some(localhost())).await;
        }
        // Next request should be rate limited
        let result = limiter.check("key_1", ApiKeyTier::Free, Some(localhost())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rate_limit_error_contains_retry_after() {
        let limiter = ApiKeyRateLimiter::new();
        // Exhaust the bucket
        for _ in 0..100 {
            let _ = limiter.check("key_1", ApiKeyTier::Free, Some(localhost())).await;
        }
        let result = limiter.check("key_1", ApiKeyTier::Free, Some(localhost())).await;
        let err = result.expect_err("Should be rate limited");
        assert!(err.retry_after_secs >= 1);
        assert!(err.limit == 100);
        assert!(err.remaining == 0);
    }

    #[tokio::test]
    async fn different_keys_have_independent_buckets() {
        let limiter = ApiKeyRateLimiter::new();

        // Exhaust key_1's bucket
        for _ in 0..100 {
            let _ = limiter.check("key_1", ApiKeyTier::Free, Some(localhost())).await;
        }

        // key_2 should still be allowed
        let result = limiter.check("key_2", ApiKeyTier::Free, Some(localhost())).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn pro_tier_has_higher_capacity_than_free_tier() {
        let limiter = ApiKeyRateLimiter::new();

        // Exhaust Free tier for key1 (100 requests)
        for _ in 0..100 {
            let _ = limiter.check("key1", ApiKeyTier::Free, Some(localhost())).await;
        }

        // key2 with Free tier should be allowed (separate bucket)
        let result = limiter.check("key2", ApiKeyTier::Free, Some(localhost())).await;
        assert!(result.is_ok());

        // key3 with Pro tier should also be allowed (separate bucket, 1000 capacity)
        let result = limiter.check("key3", ApiKeyTier::Pro, Some(localhost())).await;
        assert!(result.is_ok());

        // key4 with Enterprise tier should also be allowed (separate bucket, 10000 capacity)
        let result = limiter.check("key4", ApiKeyTier::Enterprise, Some(localhost())).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn reset_clears_bucket() {
        let limiter = ApiKeyRateLimiter::new();
        let key = "key_1";

        // Exhaust the bucket
        for _ in 0..100 {
            let _ = limiter.check(key, ApiKeyTier::Free, Some(localhost())).await;
        }

        // Reset and verify
        limiter.reset(key).await;
        let result = limiter.check(key, ApiKeyTier::Free, Some(localhost())).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn anonymous_key_falls_back_to_ip() {
        let limiter = ApiKeyRateLimiter::new();
        let ip = localhost();

        // First anonymous request uses IP
        let result = limiter
            .check("_anonymous", ApiKeyTier::Free, Some(ip))
            .await;
        assert!(result.is_ok());

        // Exhaust the IP bucket
        for _ in 0..99 {
            let _ = limiter
                .check("_anonymous", ApiKeyTier::Free, Some(ip))
                .await;
        }

        // Should be rate limited now
        let result = limiter
            .check("_anonymous", ApiKeyTier::Free, Some(ip))
            .await;
        assert!(result.is_err());
    }
}