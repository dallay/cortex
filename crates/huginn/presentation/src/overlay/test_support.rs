//! Access to counter exhaustion without exposing mutable overlay internals.
use super::{FocusLease, OverlayMachine};

pub const fn exhaust_counters<T>(machine: &mut OverlayMachine<T>) {
    machine.revision = u64::MAX;
    machine.focus = FocusLease(u64::MAX);
}
