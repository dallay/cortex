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
