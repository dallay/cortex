#[cfg(feature = "test-driver")]
use huginn_presentation::event::test_support;
use huginn_presentation::event::*;
#[cfg(feature = "test-driver")]
use rand::RngExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{sync::Arc, time::Duration};

#[test]
fn bounded_summary_preserves_short_and_exact_limit_text() {
    assert_eq!(
        bounded_ui_summary("hello\n界\t\x1b\rworld"),
        "hello 界 world"
    );
    assert_eq!(bounded_ui_summary(&"a".repeat(4096)), "a".repeat(4096));
    assert_eq!(bounded_ui_summary(&"🙂".repeat(1024)), "🙂".repeat(1024));
    assert_eq!(bounded_ui_summary(&"\x1b".repeat(8192)), "");
    let truncated = bounded_ui_summary(&"🙂".repeat(1025));
    assert!(truncated.len() <= 4096);
    assert!(truncated.ends_with(" [truncated]"));
}

#[test]
fn ui_event_buffer_bounds_and_sanitizes_retained_summaries() {
    for text in [
        "a".repeat(8192),
        "界🙂".repeat(2048),
        "\x1b\n\t".repeat(4096),
    ] {
        let mut buffer = UiEventBuffer::default();
        buffer.push(UiEventRecord {
            kind: EventKind::Diagnostic,
            summary: text,
        });
        let retained = &buffer.iter().next().unwrap().summary;
        assert!(retained.len() <= 4096, "retained {} bytes", retained.len());
        assert!(retained.ends_with(" [truncated]"));
        assert!(!retained.chars().any(char::is_control));
    }
}

struct FakeClock(AtomicU64);
impl FakeClock {
    const fn at(seconds: u64) -> Self {
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

#[cfg(feature = "test-driver")]
#[test]
fn dedup_nonces_do_not_repeat_after_eviction_expiry_or_operation_finish() {
    let clock = Arc::new(FakeClock::at(0));
    let mut dedup = ErrorDeduplicator::new(clock.clone());
    let op = OperationId(1);
    let mut nonces = std::collections::HashSet::new();

    for _ in 0..=ERROR_DEDUP_CAPACITY {
        dedup.finish_operation(op);
        assert!(dedup.should_emit(op, "same plaintext"));
        let fingerprint = test_support::last_fingerprint(&dedup);
        assert!(nonces.insert(fingerprint[..12].to_vec()));
        assert!(test_support::decrypts_to(
            &dedup,
            fingerprint,
            b"same plaintext"
        ));
    }
    for index in 0..=ERROR_DEDUP_CAPACITY {
        assert!(dedup.should_emit(op, &format!("error {index}")));
        assert!(nonces.insert(test_support::last_fingerprint(&dedup)[..12].to_vec()));
    }
    dedup.finish_operation(op);
    assert!(dedup.should_emit(op, "after finish"));
    assert!(nonces.insert(test_support::last_fingerprint(&dedup)[..12].to_vec()));
    clock.advance_ms(2_000);
    assert!(dedup.should_emit(op, "after expiry"));
    assert!(nonces.insert(test_support::last_fingerprint(&dedup)[..12].to_vec()));
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

#[cfg(feature = "test-driver")]
#[test]
fn dedup_decryption_requires_the_matching_key_even_with_the_same_nonce() {
    let clock = Arc::new(FakeClock::at(0));
    let mut first_key = [0u8; 32];
    first_key = rand::rng().random();
    let mut second_key = [0u8; 32];
    second_key = rand::rng().random();
    if second_key == first_key {
        second_key[0] ^= 1;
    }
    assert_ne!(first_key, second_key);

    let mut first = test_support::with_key(clock.clone(), first_key);
    let mut second = test_support::with_key(clock, second_key);
    assert!(first.should_emit(OperationId(1), "same"));
    assert!(second.should_emit(OperationId(1), "same"));
    let first_fingerprint = test_support::last_fingerprint(&first);
    let second_fingerprint = test_support::last_fingerprint(&second);
    assert_eq!(first_fingerprint[..12], second_fingerprint[..12]);
    for (own, other, fingerprint) in [
        (&first, &second, &first_fingerprint),
        (&second, &first, &second_fingerprint),
    ] {
        assert!(test_support::decrypts_to(own, fingerprint, b"same"));
        assert!(!test_support::decrypts_to(other, fingerprint, b"same"));
    }
}

#[cfg(feature = "test-driver")]
#[test]
fn dedup_nonce_exhaustion_emits_without_panicking_or_caching() {
    let clock = Arc::new(FakeClock::at(0));
    let mut dedup = ErrorDeduplicator::new(clock);
    test_support::exhaust_nonce_after_next(&mut dedup);
    assert!(dedup.should_emit(OperationId(9), "last cached"));
    assert_eq!(dedup.len(), 1);
    let fingerprint = test_support::last_fingerprint(&dedup);
    assert_eq!(&fingerprint[4..12], &(u64::MAX - 1).to_be_bytes());
    assert!(!dedup.should_emit(OperationId(9), "last cached"));

    assert!(dedup.should_emit(OperationId(9), "nonce boundary"));
    assert!(dedup.should_emit(OperationId(9), "nonce boundary"));
    assert_eq!(dedup.len(), 0);
    assert_eq!(dedup.retained_bytes(), 0);
}

#[cfg(feature = "test-driver")]
#[test]
fn dedup_does_not_retain_sanitized_secret_in_plaintext() {
    let clock = Arc::new(FakeClock::at(0));
    let mut dedup = ErrorDeduplicator::new(clock);
    let secret = "token=super-secret-value";

    assert!(dedup.should_emit(OperationId(7), secret));

    let retained = test_support::retained_debug(&dedup);
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
