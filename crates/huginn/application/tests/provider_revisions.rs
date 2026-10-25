#![cfg(feature = "test-hooks")]
use huginn_application::providers::test_support::ObservationState;
use huginn_core::providers::*;

#[test]
fn last_observable_revision_is_reserved_for_fail_closed_absence() {
    let mut state = ObservationState::new(u64::MAX - 1);
    let observed = state.subscribe();
    state.observe(
        ConnectionPhase::Selected,
        Some(SelectionSnapshot {
            provider: "p".into(),
            connection: "c".into(),
            model: "m".into(),
            mode: SelectionMode::Unmanaged,
        }),
    );
    assert_eq!(observed.borrow().phase, ConnectionPhase::Unavailable);
    assert_eq!(observed.borrow().revision, u64::MAX);
    assert!(observed.borrow().selection.is_none());
}
