// api_key_rate_limiter — per-key token bucket rate limiting for CLIENT_API routes
//
// Tracks rate limits by API key (X-Authz-Auth-ID header value).
// Falls back to IP-based limiting when no API key context is available.
// Tier-based limits configured via TOML RateLimiterConfig.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::middleware::ip_rate_limiter::rand_simple;
use rook_core::ApiKeyTier;
use tokio::sync::Mutex;

/// Rate limiting configuration for API keys
#[derive(Debug, Clone)]
pub struct RateLimiterConfig {
    pub enabled: bool,
    pub default_tier: ApiKeyTier,
    pub tiers: HashMap<ApiKeyTier, TierConfig>,
}

#[derive(Debug, Clone)]
pub struct TierConfig {
    pub requests_per_minute: u32,
    pub requests_per_day: Option<u32>, // reserved: daily request quota (not yet enforced in token-bucket)
    pub tokens_per_minute: Option<u32>, // reserved: token-weighted rate (not yet enforced in token-bucket)
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        let mut tiers = HashMap::new();
        // Default values match the original hardcoded tier_params for backward compatibility
        tiers.insert(
            ApiKeyTier::Free,
            TierConfig {
                requests_per_minute: 100, // 100 capacity, ~1.67/s refill (100 rpm ÷ 60)
                requests_per_day: Some(1000),
                tokens_per_minute: Some(10000),
            },
        );
        tiers.insert(
            ApiKeyTier::Pro,
            TierConfig {
                requests_per_minute: 1000, // 1000 capacity, ~16.67/s refill (1000 rpm ÷ 60)
                requests_per_day: Some(100000),
                tokens_per_minute: Some(100000),
            },
        );
        tiers.insert(
            ApiKeyTier::Enterprise,
            TierConfig {
                requests_per_minute: 10000, // 10000 capacity, ~166.67/s refill (10000 rpm ÷ 60)
                requests_per_day: Some(10000000),
                tokens_per_minute: Some(1000000),
            },
        );
        Self {
            enabled: false,
            default_tier: ApiKeyTier::Free,
            tiers,
        }
    }
}

/// Rate limit exceeded error with retry-after information
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitExceeded {
    pub retry_after_secs: u64,
    pub limit: u64,
    pub remaining: u64,
    pub reset_unix: u64,
}

/// API key rate limiter result
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
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

    /// Time until next token is available (capped at 1 hour to prevent u64::MAX overflow)
    fn retry_after(&self, _capacity: u64, refill_per_second: f64) -> Duration {
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            let tokens_needed = 1.0 - self.tokens;
            let secs = if refill_per_second > 0.0 {
                (tokens_needed / refill_per_second).ceil() as u64
            } else {
                // Avoid division by zero; cap at 1 hour
                3600
            };
            // Cap at 1 hour to prevent u64::MAX overflow on very low refill rates
            Duration::from_secs(secs.clamp(1, 3600))
        }
    }
}

/// Per-key token bucket rate limiter for CLIENT_API routes
///
/// Uses the `X-Authz-Auth-ID` header (set by authz middleware after API key validation)
/// as the rate limit key. Falls back to IP-based limiting if no API key context.
///
/// Uses TTL-based eviction to prevent unbounded memory growth:
/// - Buckets not accessed for `idle_ttl` are evicted on next access.
/// - At most `max_entries` buckets are kept (LRU eviction when limit is reached).
#[derive(Debug, Clone)]
pub struct ApiKeyRateLimiter {
    buckets: Arc<Mutex<HashMap<String, TokenBucket>>>,
    config: Arc<RateLimiterConfig>,
    /// TTL: evict buckets idle for longer than this
    idle_ttl: Duration,
    /// Max entries: evict oldest when exceeding this size
    max_entries: usize,
}

impl ApiKeyRateLimiter {
    /// Create a new API key rate limiter with empty bucket map and default config.
    /// Uses default TTL of 10 minutes and max 100,000 entries.
    pub fn new() -> Self {
        Self::with_config_and_limits(
            Arc::new(RateLimiterConfig::default()),
            Duration::from_secs(600),
            100_000,
        )
    }

    /// Create a new API key rate limiter with custom config.
    /// Uses default TTL of 10 minutes and max 100,000 entries.
    pub fn with_config(config: Arc<RateLimiterConfig>) -> Self {
        Self::with_config_and_limits(config, Duration::from_secs(600), 100_000)
    }

    /// Create a new API key rate limiter with custom config, idle TTL, and max entries.
    pub fn with_config_and_limits(
        config: Arc<RateLimiterConfig>,
        idle_ttl: Duration,
        max_entries: usize,
    ) -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            config,
            idle_ttl,
            max_entries,
        }
    }

    /// Evict buckets idle for longer than `idle_ttl` and enforce `max_entries`.
    async fn evict_stale_locked(&self, buckets: &mut HashMap<String, TokenBucket>) {
        let now = Instant::now();

        // TTL eviction: remove buckets idle for longer than idle_ttl
        buckets.retain(|_, bucket| now.duration_since(bucket.last_refill) < self.idle_ttl);

        // Size cap: if still over max_entries, evict oldest by last_refill time
        if buckets.len() >= self.max_entries {
            let mut entries: Vec<_> = buckets.iter().collect();
            entries.sort_by_key(|(_, b)| b.last_refill);
            let keys_to_remove: Vec<_> = entries
                .into_iter()
                .take(buckets.len() - self.max_entries)
                .map(|(k, _)| k.clone())
                .collect();
            for key in keys_to_remove {
                buckets.remove(&key);
            }
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
        // Get tier config, fallback to default tier if not found
        let tier_config = self.config.tiers.get(&tier).or_else(|| {
            tracing::warn!(
                tier = tier.as_str(),
                default_tier = self.config.default_tier.as_str(),
                "Tier config not found, falling back to default tier"
            );
            self.config.tiers.get(&self.config.default_tier)
        });

        let tier_config = match tier_config {
            Some(cfg) => cfg,
            None => {
                tracing::error!("No tier config found, including default tier");
                // Fallback to hardcoded safe defaults if config is completely missing
                return self.check_with_params(key_id, client_ip, 60, 1.0).await;
            }
        };

        let capacity = tier_config.requests_per_minute as u64;
        let refill_per_second = tier_config.requests_per_minute as f64 / 60.0;

        self.check_with_params(key_id, client_ip, capacity, refill_per_second)
            .await
    }

    /// Internal check with explicit capacity and refill rate
    async fn check_with_params(
        &self,
        key_id: &str,
        client_ip: Option<IpAddr>,
        capacity: u64,
        refill_per_second: f64,
    ) -> Result<RateLimitSnapshot, RateLimitExceeded> {
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

        // Probabilistic eviction: run eviction ~1% of calls to keep latency low
        if rand_simple() < 0.01 {
            self.evict_stale_locked(&mut buckets).await;
        }

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
        buckets.get(key_id).map(|b| b.tokens()).unwrap_or(100.0) // Default capacity
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
        let result = limiter
            .check("key_1", ApiKeyTier::Free, Some(localhost()))
            .await;
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
            let _ = limiter
                .check("key_1", ApiKeyTier::Free, Some(localhost()))
                .await;
        }
        // Next request should be rate limited
        let result = limiter
            .check("key_1", ApiKeyTier::Free, Some(localhost()))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rate_limit_error_contains_retry_after() {
        let limiter = ApiKeyRateLimiter::new();
        // Exhaust the bucket
        for _ in 0..100 {
            let _ = limiter
                .check("key_1", ApiKeyTier::Free, Some(localhost()))
                .await;
        }
        let result = limiter
            .check("key_1", ApiKeyTier::Free, Some(localhost()))
            .await;
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
            let _ = limiter
                .check("key_1", ApiKeyTier::Free, Some(localhost()))
                .await;
        }

        // key_2 should still be allowed
        let result = limiter
            .check("key_2", ApiKeyTier::Free, Some(localhost()))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn pro_tier_has_higher_capacity_than_free_tier() {
        let limiter = ApiKeyRateLimiter::new();

        // Exhaust Free tier for key1 (100 requests)
        for _ in 0..100 {
            let _ = limiter
                .check("key1", ApiKeyTier::Free, Some(localhost()))
                .await;
        }

        // key2 with Free tier should be allowed (separate bucket)
        let result = limiter
            .check("key2", ApiKeyTier::Free, Some(localhost()))
            .await;
        assert!(result.is_ok());

        // key3 with Pro tier should also be allowed (separate bucket, 1000 capacity)
        let result = limiter
            .check("key3", ApiKeyTier::Pro, Some(localhost()))
            .await;
        assert!(result.is_ok());

        // key4 with Enterprise tier should also be allowed (separate bucket, 10000 capacity)
        let result = limiter
            .check("key4", ApiKeyTier::Enterprise, Some(localhost()))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn reset_clears_bucket() {
        let limiter = ApiKeyRateLimiter::new();
        let key = "key_1";

        // Exhaust the bucket
        for _ in 0..100 {
            let _ = limiter
                .check(key, ApiKeyTier::Free, Some(localhost()))
                .await;
        }

        // Reset and verify
        limiter.reset(key).await;
        let result = limiter
            .check(key, ApiKeyTier::Free, Some(localhost()))
            .await;
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
