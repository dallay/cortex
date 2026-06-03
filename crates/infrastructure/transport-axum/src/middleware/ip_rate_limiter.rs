// ip_rate_limiter — per-IP token bucket rate limiting for unauthenticated CLIENT_API routes
//
// Applies to unauthenticated requests only. Authenticated requests bypass this limiter.
// Uses in-memory storage that is lost on restart — acceptable for MVP.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::Mutex;

/// Rate limit exceeded error with retry-after information
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitExceeded {
    pub retry_after_secs: u64,
    pub limit: u64,
    pub remaining: u64,
    pub reset_unix: u64,
}

/// Token bucket for a single IP address
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

/// IP rate limiter using per-IP token bucket algorithm
///
/// Configurable capacity and refill rate via IpRateLimitConfig.
/// Default: 30 requests per minute per IP.
#[derive(Debug, Clone)]
pub struct IpRateLimiter {
    buckets: Arc<Mutex<HashMap<IpAddr, TokenBucket>>>,
    capacity: u64,
    refill_per_second: f64,
}

impl IpRateLimiter {
    /// Create a new IP rate limiter with default capacity (30 rpm)
    pub fn new() -> Self {
        Self::with_capacity(30)
    }

    /// Create a new IP rate limiter with custom capacity (requests per minute)
    pub fn with_capacity(requests_per_minute: u32) -> Self {
        let capacity = requests_per_minute as u64;
        let refill_per_second = requests_per_minute as f64 / 60.0;
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            capacity,
            refill_per_second,
        }
    }

    /// Check if a request from the given IP is allowed
    ///
    /// Returns `Ok(())` if the request is allowed.
    /// Returns `Err(RateLimitExceeded)` if rate limited.
    pub async fn check(&self, ip: IpAddr) -> Result<(), RateLimitExceeded> {
        let mut buckets = self.buckets.lock().await;
        let bucket = buckets
            .entry(ip)
            .or_insert_with(|| TokenBucket::new(self.capacity));

        if bucket.try_consume(self.capacity, self.refill_per_second) {
            Ok(())
        } else {
            let retry_after_secs = bucket
                .retry_after(self.capacity, self.refill_per_second)
                .as_secs()
                .max(1);
            let reset_unix = unix_now() + retry_after_secs;
            Err(RateLimitExceeded {
                retry_after_secs,
                limit: self.capacity,
                remaining: 0,
                reset_unix,
            })
        }
    }

    /// Get current token count for an IP (for testing/debugging)
    #[allow(dead_code)]
    pub async fn tokens_for(&self, ip: IpAddr) -> f64 {
        let buckets = self.buckets.lock().await;
        buckets
            .get(&ip)
            .map(|b| b.tokens())
            .unwrap_or(self.capacity as f64)
    }

    /// Reset rate limit for an IP (for testing)
    #[allow(dead_code)]
    pub async fn reset(&self, ip: IpAddr) {
        let mut buckets = self.buckets.lock().await;
        buckets.remove(&ip);
    }
}

impl Default for IpRateLimiter {
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
        let limiter = IpRateLimiter::new();
        let result = limiter.check(localhost()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn allows_up_to_capacity_requests() {
        let limiter = IpRateLimiter::with_capacity(30);
        for _ in 0..30 {
            let result = limiter.check(localhost()).await;
            assert!(result.is_ok(), "Should allow request up to capacity");
        }
    }

    #[tokio::test]
    async fn blocks_request_exceeding_capacity() {
        let limiter = IpRateLimiter::with_capacity(30);
        // Exhaust the bucket
        for _ in 0..30 {
            let _ = limiter.check(localhost()).await;
        }
        // Next request should be rate limited
        let result = limiter.check(localhost()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rate_limit_error_contains_retry_after() {
        let limiter = IpRateLimiter::with_capacity(30);
        // Exhaust the bucket
        for _ in 0..30 {
            let _ = limiter.check(localhost()).await;
        }
        let result = limiter.check(localhost()).await;
        let err = result.expect_err("Should be rate limited");
        assert!(err.retry_after_secs >= 1);
        assert_eq!(err.limit, 30);
        assert_eq!(err.remaining, 0);
    }

    #[tokio::test]
    async fn different_ips_have_independent_buckets() {
        let limiter = IpRateLimiter::with_capacity(30);
        let ip1 = IpAddr::from([127, 0, 0, 1]);
        let ip2 = IpAddr::from([127, 0, 0, 2]);

        // Exhaust ip1's bucket
        for _ in 0..30 {
            let _ = limiter.check(ip1).await;
        }

        // ip2 should still be allowed
        let result = limiter.check(ip2).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn reset_clears_bucket() {
        let limiter = IpRateLimiter::with_capacity(30);
        let ip = localhost();

        // Exhaust the bucket
        for _ in 0..30 {
            let _ = limiter.check(ip).await;
        }

        // Reset and verify
        limiter.reset(ip).await;
        let result = limiter.check(ip).await;
        assert!(result.is_ok());
    }
}
