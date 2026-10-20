//! Explicit UI event categories. UI-only events are never conversation entries.
use aes_gcm::{
    aead::{Aead, OsRng},
    Aes256Gcm, KeyInit, Nonce,
};
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const UI_EVENT_CAPACITY: usize = 256;
pub const ERROR_DEDUP_WINDOW: Duration = Duration::from_secs(2);
pub const ERROR_DEDUP_CAPACITY: usize = 256;
pub const ERROR_DEDUP_BYTES: usize = 256 * 1024;
pub const ERROR_DEDUP_MAX_MESSAGE_BYTES: usize = 4096;

pub type MessageHasher = Arc<dyn Fn(&str) -> u64 + Send + Sync>;

pub use huginn_core::OperationId;

pub trait Clock: Send + Sync {
    fn now(&self) -> Duration;
}

pub struct MonotonicClock {
    origin: Instant,
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for MonotonicClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct DedupKey {
    operation: OperationId,
    hash: u64,
}

#[derive(Clone, Debug)]
struct DedupEntry {
    key: DedupKey,
    fingerprint: Vec<u8>,
    message_len: usize,
    last_visible: Duration,
}

/// Bounded error de-duplication for a single explicit operation scope.
/// Suppressed emissions do not extend the visibility window.
pub struct ErrorDeduplicator {
    clock: Arc<dyn Clock>,
    hash_message: MessageHasher,
    cipher: Aes256Gcm,
    nonce_prefix: [u8; 4],
    nonce_counter: u64,
    entries: VecDeque<DedupEntry>,
    retained_bytes: usize,
}

impl Default for ErrorDeduplicator {
    fn default() -> Self {
        Self::new(Arc::new(MonotonicClock::default()))
    }
}

impl ErrorDeduplicator {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self::with_hasher(clock, Arc::new(stable_hash))
    }

    /// Injectable hash function exists to exercise collision safety.
    pub fn with_hasher(clock: Arc<dyn Clock>, hash_message: MessageHasher) -> Self {
        // Generate a fresh AES-256 key from the OS CSPRNG for this instance.
        // Nonces need uniqueness under this key, not randomness or separation
        // from the nonces used by other instances with independently generated keys.
        let key = Aes256Gcm::generate_key(&mut OsRng);
        Self {
            clock,
            hash_message,
            cipher: Aes256Gcm::new(&key),
            nonce_prefix: [0; 4],
            nonce_counter: 0,
            entries: VecDeque::new(),
            retained_bytes: 0,
        }
    }

    /// Returns true when this message should be projected to the UI.
    pub fn should_emit(&mut self, operation: OperationId, message: &str) -> bool {
        let now = self.clock.now();
        self.purge_expired(now);
        let message = safe_message(message);
        if message.len() > ERROR_DEDUP_MAX_MESSAGE_BYTES {
            return true;
        }
        let key = DedupKey {
            operation,
            hash: (self.hash_message)(&message),
        };
        if let Some(index) = self.entries.iter().enumerate().find_map(|(index, entry)| {
            if entry.key != key {
                return None;
            }
            let nonce = Nonce::from_slice(&entry.fingerprint[..12]);
            let plaintext = self.cipher.decrypt(nonce, &entry.fingerprint[12..]).ok()?;
            (plaintext == message.as_bytes()).then_some(index)
        }) {
            if now.saturating_sub(self.entries[index].last_visible) < ERROR_DEDUP_WINDOW {
                return false;
            }
            self.entries[index].last_visible = now;
            return true;
        }
        let retained_size = message.len() + 28;
        while self.entries.len() >= ERROR_DEDUP_CAPACITY
            || self.retained_bytes + retained_size > ERROR_DEDUP_BYTES
        {
            self.evict_oldest();
        }
        let Some(next_nonce_counter) = self.nonce_counter.checked_add(1) else {
            // A saturated nonce space must not turn a UI error into a panic.
            // Keep the event visible, but stop caching until this sink is dropped.
            self.entries.clear();
            self.retained_bytes = 0;
            return true;
        };
        let counter = self.nonce_counter.to_be_bytes();
        self.nonce_counter = next_nonce_counter;
        // 96-bit nonce: fixed prefix + 64-bit counter. The counter never resets
        // while this instance holds its key, including after cache eviction or
        // operation completion. checked_add above prevents wraparound and reuse.
        // Different instances may use the same nonce under their independent keys.
        let nonce = [self.nonce_prefix.as_slice(), counter.as_slice()].concat();
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), message.as_bytes())
            .expect("AES-GCM encryption");
        let mut fingerprint = nonce;
        fingerprint.extend(ciphertext);
        self.retained_bytes += retained_size;
        self.entries.push_back(DedupEntry {
            key,
            fingerprint,
            message_len: retained_size,
            last_visible: now,
        });
        true
    }

    pub fn finish_operation(&mut self, operation: OperationId) {
        self.entries.retain(|entry| {
            let keep = entry.key.operation != operation;
            if !keep {
                self.retained_bytes -= entry.message_len;
            }
            keep
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    fn purge_expired(&mut self, now: Duration) {
        self.entries.retain(|entry| {
            let keep = now.saturating_sub(entry.last_visible) < ERROR_DEDUP_WINDOW;
            if !keep {
                self.retained_bytes -= entry.message_len;
            }
            keep
        });
    }

    fn evict_oldest(&mut self) {
        if let Some(entry) = self.entries.pop_front() {
            self.retained_bytes -= entry.message_len;
        }
    }
}

fn stable_hash(message: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    message.hash(&mut hasher);
    hasher.finish()
}

fn safe_message(message: &str) -> String {
    message
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    Conversation,
    Overlay,
    Notification,
    Status,
    Diagnostic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiEventRecord {
    pub kind: EventKind,
    pub summary: String,
}

/// A bounded, non-authoritative FIFO projection for non-conversation UI events.
/// Overlay state and terminal status remain authoritative elsewhere.
#[derive(Clone, Debug, Default)]
pub struct UiEventBuffer(VecDeque<UiEventRecord>);

impl UiEventBuffer {
    pub fn push(&mut self, event: UiEventRecord) {
        if self.0.len() == UI_EVENT_CAPACITY {
            self.0.pop_front();
        }
        self.0.push_back(event);
    }

    pub fn iter(&self) -> impl Iterator<Item = &UiEventRecord> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct FakeClock(AtomicU64);
    impl FakeClock {
        fn at(seconds: u64) -> Self {
            Self(AtomicU64::new(seconds * 1000))
        }
        fn advance_ms(&self, ms: u64) {
            self.0.fetch_add(ms, Ordering::SeqCst);
        }
    }
    impl Clock for FakeClock {
        fn now(&self) -> Duration {
            Duration::from_millis(self.0.load(Ordering::SeqCst))
        }
    }

    #[test]
    fn ui_event_buffer_keeps_only_the_latest_capacity_in_fifo_order() {
        let mut buffer = UiEventBuffer::default();
        for index in 0..=UI_EVENT_CAPACITY {
            buffer.push(UiEventRecord {
                kind: EventKind::Status,
                summary: index.to_string(),
            });
        }

        assert_eq!(buffer.len(), UI_EVENT_CAPACITY);
        assert_eq!(buffer.iter().next().unwrap().summary, "1");
        assert_eq!(
            buffer.iter().last().unwrap().summary,
            UI_EVENT_CAPACITY.to_string()
        );
    }

    #[test]
    fn dedup_uses_half_open_two_second_window_and_reanchors_visible_emission() {
        let clock = Arc::new(FakeClock::at(0));
        let mut dedup = ErrorDeduplicator::new(clock.clone());
        let op = OperationId(1);
        assert!(dedup.should_emit(op, "boom"));
        clock.advance_ms(1_999);
        assert!(!dedup.should_emit(op, "boom"));
        clock.advance_ms(1);
        assert!(dedup.should_emit(op, "boom"));
        assert!(!dedup.should_emit(op, "boom"));
    }

    #[test]
    fn dedup_scopes_operations_and_compares_full_text_on_hash_collision() {
        let clock = Arc::new(FakeClock::at(0));
        let mut dedup = ErrorDeduplicator::with_hasher(clock, Arc::new(|_| 0));
        assert!(dedup.should_emit(OperationId(1), "same"));
        assert!(!dedup.should_emit(OperationId(1), "same"));
        assert!(dedup.should_emit(OperationId(2), "same"));
        assert!(dedup.should_emit(OperationId(1), "different"));
        dedup.finish_operation(OperationId(1));
        assert_eq!(dedup.len(), 1);
    }

    #[test]
    fn dedup_enforces_entry_byte_and_oversize_bounds_without_truncation() {
        let clock = Arc::new(FakeClock::at(0));
        let mut dedup = ErrorDeduplicator::new(clock);
        let op = OperationId(1);
        let exact = "x".repeat(ERROR_DEDUP_MAX_MESSAGE_BYTES);
        assert!(dedup.should_emit(op, &exact));
        assert!(!dedup.should_emit(op, &exact));
        let oversized = "y".repeat(ERROR_DEDUP_MAX_MESSAGE_BYTES + 1);
        assert!(dedup.should_emit(op, &oversized));
        assert!(dedup.should_emit(op, &oversized));
        assert_eq!(dedup.len(), 1, "oversized messages are never cached");

        let mut bounded = ErrorDeduplicator::new(Arc::new(FakeClock::at(0)));
        for i in 0..ERROR_DEDUP_CAPACITY {
            assert!(bounded.should_emit(op, &format!("{i:04}")));
        }
        assert_eq!(bounded.len(), ERROR_DEDUP_CAPACITY);
        assert!(bounded.should_emit(op, "newest"));
        assert_eq!(bounded.len(), ERROR_DEDUP_CAPACITY);
        assert!(bounded.should_emit(op, "0000"), "oldest entry was evicted");
        assert!(bounded.retained_bytes() <= ERROR_DEDUP_BYTES);

        let mut byte_bounded = ErrorDeduplicator::new(Arc::new(FakeClock::at(0)));
        for index in 0..ERROR_DEDUP_CAPACITY {
            assert!(byte_bounded.should_emit(op, &format!("{index:04}{}", "b".repeat(1020))));
        }
        assert!(byte_bounded.retained_bytes() <= ERROR_DEDUP_BYTES);
        assert!(byte_bounded.should_emit(op, &format!("new!{}", "c".repeat(1020))));
        assert!(byte_bounded.retained_bytes() <= ERROR_DEDUP_BYTES);
        assert!(byte_bounded.len() <= ERROR_DEDUP_CAPACITY);
    }

    #[test]
    fn dedup_purges_expired_entries_and_releases_retained_bytes() {
        let clock = Arc::new(FakeClock::at(0));
        let mut dedup = ErrorDeduplicator::new(clock.clone());
        let op = OperationId(1);
        assert!(dedup.should_emit(op, "expires"));
        assert_eq!(dedup.len(), 1);
        assert!(dedup.retained_bytes() > 0);

        clock.advance_ms(2_000);
        assert!(dedup.should_emit(OperationId(2), "trigger purge"));
        assert_eq!(dedup.len(), 1);
        assert_eq!(dedup.retained_bytes(), "trigger purge".len() + 28);
        assert!(
            dedup.should_emit(op, "expires"),
            "expired text is emitted again"
        );
    }

    #[test]
    fn dedup_nonces_do_not_repeat_after_eviction_expiry_or_operation_finish() {
        let clock = Arc::new(FakeClock::at(0));
        let mut dedup = ErrorDeduplicator::new(clock.clone());
        let op = OperationId(1);
        let mut nonces = std::collections::HashSet::new();

        for _ in 0..=ERROR_DEDUP_CAPACITY {
            dedup.finish_operation(op);
            assert!(dedup.should_emit(op, "same plaintext"));
            let fingerprint = &dedup.entries.back().unwrap().fingerprint;
            assert!(nonces.insert(fingerprint[..12].to_vec()));
            assert_eq!(
                dedup
                    .cipher
                    .decrypt(Nonce::from_slice(&fingerprint[..12]), &fingerprint[12..])
                    .unwrap(),
                b"same plaintext"
            );
        }
        for index in 0..=ERROR_DEDUP_CAPACITY {
            assert!(dedup.should_emit(op, &format!("error {index}")));
            assert!(nonces.insert(dedup.entries.back().unwrap().fingerprint[..12].to_vec()));
        }
        dedup.finish_operation(op);
        assert!(dedup.should_emit(op, "after finish"));
        assert!(nonces.insert(dedup.entries.back().unwrap().fingerprint[..12].to_vec()));
        clock.advance_ms(2_000);
        assert!(dedup.should_emit(op, "after expiry"));
        assert!(nonces.insert(dedup.entries.back().unwrap().fingerprint[..12].to_vec()));
    }

    #[test]
    fn dedup_instances_keep_independent_caches() {
        let clock = Arc::new(FakeClock::at(0));
        let mut first = ErrorDeduplicator::new(clock.clone());
        let mut second = ErrorDeduplicator::new(clock);
        let op = OperationId(1);
        assert!(first.should_emit(op, "same"));
        assert!(second.should_emit(op, "same"));
        assert!(!first.should_emit(op, "same"));
        assert!(!second.should_emit(op, "same"));
        first.finish_operation(op);
        assert!(!second.should_emit(op, "same"));
    }

    #[test]
    fn dedup_decryption_requires_the_matching_key_even_with_the_same_nonce() {
        let clock = Arc::new(FakeClock::at(0));
        let mut first = ErrorDeduplicator::new(clock.clone());
        let mut second = ErrorDeduplicator::new(clock);
        // Fixed distinct test keys avoid probabilistic assertions about key generation.
        first.cipher = Aes256Gcm::new_from_slice(&[1; 32]).unwrap();
        second.cipher = Aes256Gcm::new_from_slice(&[2; 32]).unwrap();
        second.nonce_prefix = first.nonce_prefix;
        assert!(first.should_emit(OperationId(1), "same"));
        assert!(second.should_emit(OperationId(1), "same"));
        let first_fingerprint = &first.entries.back().unwrap().fingerprint;
        let second_fingerprint = &second.entries.back().unwrap().fingerprint;
        assert_eq!(first_fingerprint[..12], second_fingerprint[..12]);
        for (own, other, fingerprint) in [
            (&first, &second, &first_fingerprint),
            (&second, &first, &second_fingerprint),
        ] {
            let nonce = Nonce::from_slice(&fingerprint[..12]);
            assert_eq!(
                own.cipher.decrypt(nonce, &fingerprint[12..]).unwrap(),
                b"same"
            );
            assert!(other.cipher.decrypt(nonce, &fingerprint[12..]).is_err());
        }
    }

    #[test]
    fn dedup_nonce_exhaustion_emits_without_panicking_or_caching() {
        let clock = Arc::new(FakeClock::at(0));
        let mut dedup = ErrorDeduplicator::new(clock);
        dedup.nonce_counter = u64::MAX - 1;
        assert!(dedup.should_emit(OperationId(9), "last cached"));
        assert_eq!(dedup.nonce_counter, u64::MAX);
        assert_eq!(dedup.len(), 1);
        let fingerprint = &dedup.entries.back().unwrap().fingerprint;
        assert_eq!(&fingerprint[4..12], &(u64::MAX - 1).to_be_bytes());
        assert!(!dedup.should_emit(OperationId(9), "last cached"));

        assert!(dedup.should_emit(OperationId(9), "nonce boundary"));
        assert!(dedup.should_emit(OperationId(9), "nonce boundary"));
        assert_eq!(dedup.len(), 0);
        assert_eq!(dedup.retained_bytes(), 0);
    }

    #[test]
    fn dedup_does_not_retain_sanitized_secret_in_plaintext() {
        let clock = Arc::new(FakeClock::at(0));
        let mut dedup = ErrorDeduplicator::new(clock);
        let secret = "token=super-secret-value";

        assert!(dedup.should_emit(OperationId(7), secret));

        let retained = format!("{:?}", dedup.entries);
        assert!(!retained.contains("super-secret-value"));
        assert!(!retained.contains("token="));
        assert!(!dedup.should_emit(OperationId(7), secret));
    }

    #[test]
    fn dedup_interleaved_operations_do_not_suppress_each_other() {
        let clock = Arc::new(FakeClock::at(0));
        let mut dedup = ErrorDeduplicator::new(clock);
        assert!(dedup.should_emit(OperationId(7), "error"));
        assert!(dedup.should_emit(OperationId(8), "error"));
        assert!(!dedup.should_emit(OperationId(7), "error"));
    }
}
