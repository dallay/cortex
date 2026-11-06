// token_bucket — single token-bucket implementation shared by every rate
// limiter (API key, IP and login). Refill, consume and retry-after semantics
// live here exactly once so a fix applies everywhere at once.

use std::time::{Duration, Instant};

/// Token bucket parameterized by capacity and refill rate.
///
/// `refill_per_second` is expressed in tokens per second. The login limiter
/// historically refills one token every 12 seconds, which is the same
/// parameterization with `refill_per_second = 1.0 / 12.0`.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

/// Retry-after values are capped at one hour to prevent `u64::MAX` overflow
/// on very low refill rates.
const MAX_RETRY_AFTER: u64 = 3600;

impl TokenBucket {
    pub fn new(capacity: u64) -> Self {
        Self {
            tokens: capacity as f64,
            last_refill: Instant::now(),
        }
    }

    /// Refill tokens based on elapsed time.
    fn refill(&mut self, capacity: u64, refill_per_second: f64) {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        let refill_amount = elapsed * refill_per_second;
        self.tokens = (self.tokens + refill_amount).min(capacity as f64);
        self.last_refill = Instant::now();
    }

    /// Try to consume one token.
    pub fn try_consume(&mut self, capacity: u64, refill_per_second: f64) -> bool {
        self.refill(capacity, refill_per_second);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Time until the next token is available.
    ///
    /// Zero when a token is already available. Otherwise the wait is rounded
    /// up to whole seconds, with a one-second floor and the one-hour cap.
    pub fn retry_after(&self, refill_per_second: f64) -> Duration {
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            let tokens_needed = 1.0 - self.tokens;
            let secs = if refill_per_second > 0.0 {
                (tokens_needed / refill_per_second).ceil() as u64
            } else {
                // Avoid division by zero; report the cap
                MAX_RETRY_AFTER
            };
            Duration::from_secs(secs.clamp(1, MAX_RETRY_AFTER))
        }
    }

    /// Current token count (for testing and snapshots).
    pub fn tokens(&self) -> f64 {
        self.tokens
    }

    /// Instant of the last refill (used for idle eviction and LRU ordering).
    pub fn last_refill(&self) -> Instant {
        self.last_refill
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_full_at_capacity() {
        let bucket = TokenBucket::new(5);
        assert_eq!(bucket.tokens(), 5.0);
    }

    #[test]
    fn consume_draws_down_one_token_at_a_time() {
        let mut bucket = TokenBucket::new(3);
        assert!(bucket.try_consume(3, 1.0));
        assert!(bucket.try_consume(3, 1.0));
        assert!(bucket.try_consume(3, 1.0));
        // A tiny refill may have accumulated while consuming, so the bucket is
        // only guaranteed to be below one token, not exactly zero.
        assert!(bucket.tokens() < 1.0);
        assert!(!bucket.try_consume(3, 1.0), "empty bucket must reject");
    }

    #[test]
    fn refill_never_exceeds_capacity() {
        let mut bucket = TokenBucket::new(2);
        // Drain, then let more than the capacity's worth of time pass.
        assert!(bucket.try_consume(2, 1.0));
        assert!(bucket.try_consume(2, 1.0));
        assert!(!bucket.try_consume(2, 1.0));
        std::thread::sleep(Duration::from_millis(20));
        // Two full refills' worth of elapsed time still yields at most capacity.
        assert!(bucket.try_consume(2, 100.0));
        assert!(bucket.tokens() <= 2.0);
    }

    #[test]
    fn retry_after_is_zero_when_a_token_is_available() {
        let bucket = TokenBucket::new(5);
        assert_eq!(bucket.retry_after(1.0), Duration::ZERO);
    }

    #[test]
    fn retry_after_rounds_up_to_whole_seconds_with_a_floor_of_one() {
        let mut bucket = TokenBucket::new(1);
        assert!(bucket.try_consume(1, 1.0));
        // 0.5 tokens missing at 1 token/sec -> 0.5s -> rounds up to 1s floor.
        bucket.tokens = 0.5;
        let wait = bucket.retry_after(1.0);
        assert_eq!(wait, Duration::from_secs(1));
    }

    #[test]
    fn retry_after_is_capped_at_one_hour() {
        let mut bucket = TokenBucket::new(1);
        assert!(bucket.try_consume(1, 1.0));
        // A nearly empty bucket at an extremely low refill rate would overflow
        // u64 seconds without the cap.
        let wait = bucket.retry_after(1.0 / (3600.0 * 1_000_000.0));
        assert_eq!(wait, Duration::from_secs(3600));
    }

    #[test]
    fn retry_after_guards_against_zero_refill_rate() {
        let mut bucket = TokenBucket::new(1);
        assert!(bucket.try_consume(1, 1.0));
        let wait = bucket.retry_after(0.0);
        assert_eq!(wait, Duration::from_secs(3600));
    }

    #[test]
    fn login_parameterization_matches_one_token_every_twelve_seconds() {
        let rate = 1.0 / 12.0;
        let mut bucket = TokenBucket::new(5);
        for _ in 0..5 {
            assert!(bucket.try_consume(5, rate));
        }
        assert!(!bucket.try_consume(5, rate));
        // A drained bucket needs one token: between 0 and 12 seconds of wait
        // (up to one token may have refilled while draining), rounded up to
        // at least one second.
        let wait = bucket.retry_after(rate).as_secs();
        assert!((1..=12).contains(&wait), "expected 1..=12s, got {wait}");
    }
}
