use shared_kernel::Instant;

use std::time::Duration;

#[test]
fn test_instant_methods() {
    let now = Instant::now();
    let default_now = Instant::default();
    let _elapsed = now.elapsed();

    let added = now.checked_add(Duration::from_secs(10));
    assert!(added.is_some());

    let dur = added.unwrap().saturating_duration_since(now);
    assert_eq!(dur, Duration::from_secs(10));

    let _saturating_earlier = now.saturating_duration_since(added.unwrap());
    assert_eq!(_saturating_earlier, Duration::from_secs(0));
    assert_ne!(now, added.unwrap());
    let _ = default_now;
}
