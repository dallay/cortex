use shared_kernel::{CortexError, ProviderId};

#[test]
fn local_backpressure_returns_a_short_retry_hint() {
    let error = CortexError::local_backpressure();

    assert!(error.is_rate_limited());
    assert_eq!(error.retry_after_secs(), Some(1));
}

#[test]
fn upstream_retry_hint_is_preserved() {
    let error = CortexError::rate_limited(ProviderId::new("provider"), 37);

    assert_eq!(error.retry_after_secs(), Some(37));
}
