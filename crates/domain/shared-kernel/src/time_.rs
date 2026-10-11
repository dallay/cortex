// Time abstractions — kept minimal, platform-independent

/// Instantaneous time source — wraps std::time::Instant for testability
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instant(std::time::Instant);

impl Instant {
    pub fn now() -> Self {
        Self(std::time::Instant::now())
    }

    pub fn elapsed(&self) -> std::time::Duration {
        self.0.elapsed()
    }

    pub fn checked_add(&self, duration: std::time::Duration) -> Option<Self> {
        self.0.checked_add(duration).map(Self)
    }

    pub fn saturating_duration_since(&self, earlier: Self) -> std::time::Duration {
        self.0.saturating_duration_since(earlier.0)
    }
}

impl Default for Instant {
    fn default() -> Self {
        Self::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_instant_methods() {
        let now = Instant::now();
        let default_now = Instant::default();
        let _elapsed = now.elapsed();

        let added = now.checked_add(Duration::from_secs(10));
        assert!(added.is_some());

        let dur = added.unwrap().saturating_duration_since(now);
        assert!(dur >= Duration::from_secs(9));

        let _saturating_earlier = now.saturating_duration_since(added.unwrap());
        assert_eq!(_saturating_earlier, Duration::from_secs(0));
        assert_ne!(now, added.unwrap());
        let _ = default_now;
    }
}
