//! Explicit UI event categories. UI-only events are never conversation entries.
use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use rand::RngExt;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const UI_EVENT_CAPACITY: usize = 256;
pub(crate) const UI_SUMMARY_MAX_BYTES: usize = 4096;
const UI_SUMMARY_TRUNCATED: &str = " [truncated]";

/// Sanitize a UI-only summary and bound its UTF-8 bytes, including the marker.
/// Conversation content and error de-duplication must keep their original text.
pub fn bounded_ui_summary(text: &str) -> String {
    let mut summary = String::with_capacity(text.len().min(UI_SUMMARY_MAX_BYTES));
    for c in text.chars() {
        let c = match c {
            '\n' | '\t' => ' ',
            c if c.is_control() => continue,
            c => c,
        };
        if summary.len() + c.len_utf8() > UI_SUMMARY_MAX_BYTES {
            let mut end = UI_SUMMARY_MAX_BYTES - UI_SUMMARY_TRUNCATED.len();
            while !summary.is_char_boundary(end) {
                end -= 1;
            }
            summary.truncate(end);
            summary.push_str(UI_SUMMARY_TRUNCATED);
            return summary;
        }
        summary.push(c);
    }
    summary
}
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
        let key_bytes: [u8; 32] = rand::rng().random();
        Self {
            clock,
            hash_message,
            cipher: Aes256Gcm::new_from_slice(&key_bytes).expect("32-byte key"),
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
            let nonce = Nonce::try_from(&entry.fingerprint[..12]).ok()?;
            let plaintext = self.cipher.decrypt(&nonce, &entry.fingerprint[12..]).ok()?;
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
            .encrypt(&Nonce::try_from(nonce.as_slice()).expect("12-byte nonce"), message.as_bytes())
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
    pub fn push(&mut self, mut event: UiEventRecord) {
        event.summary = bounded_ui_summary(&event.summary);
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

#[cfg(feature = "test-driver")]
#[doc(hidden)]
pub mod test_support;
