// token_bucket — shared TokenBucket implementation for rate limiting
//
// Both IpRateLimiter and ApiKeyRateLimiter use identical TokenBucket logic.
// Centralized here to eliminate duplication.

use std::time::{Duration, Instant};

/// Rate limit exceeded error with retry-after information
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitExceeded {
    pub retry_after_secs: u64,
    pub limit: u64,
    pub remaining: u64,
    pub reset_unix: u64,
}

/// Token bucket for rate limiting.
///
/// Refills tokens based on elapsed wall-clock time at a configurable
/// `refill_per_second` rate up to `capacity`.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    /// Create a new bucket with `capacity` tokens, fully refilled.
    #[inline]
    pub fn new(capacity: u64) -> Self {
        Self {
            tokens: capacity as f64,
            last_refill: Instant::now(),
        }
    }

    /// Refill tokens based on elapsed time since last refill.
    #[inline]
    pub fn refill(&mut self, capacity: u64, refill_per_second: f64) {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        let refill_amount = elapsed * refill_per_second;
        self.tokens = (self.tokens + refill_amount).min(capacity as f64);
        self.last_refill = Instant::now();
    }

    /// Try to consume one token. Refills before consuming.
    ///
    /// Returns `true` if a token was consumed, `false` if the bucket was empty.
    #[inline]
    pub fn try_consume(&mut self, capacity: u64, refill_per_second: f64) -> bool {
        self.refill(capacity, refill_per_second);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Current token count (after refill).
    #[inline]
    pub fn tokens(&self) -> f64 {
        self.tokens
    }

    /// Estimated time until at least one token is available.
    ///
    /// Caps at 1 hour to prevent u64::MAX overflow on very low refill rates.
    #[inline]
    pub fn retry_after(&self, _capacity: u64, refill_per_second: f64) -> Duration {
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