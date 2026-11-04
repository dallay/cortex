//! Process-local exclusion for operations on a logical session.

use crate::{AgentError, Result};
use std::{collections::BTreeSet, sync::Mutex};

static ACTIVE: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// An owned, non-cloneable permit for one session operation.
///
/// All loop implementations share this registry, keyed by the exact persisted
/// `Session::id`, including independently loaded snapshots and loop instances.
/// Admission has capacity one per ID and no waiting queue. Different IDs do not
/// exclude one another; this is not a process-wide load limit.
///
/// Acquire before any host access or session mutation. Keep the permit through
/// final persistence; dropping it releases admission, including on future drop
/// or unwind. No registry lock is held across host work or await points.
/// This does not merge stale snapshots or replace cross-process leases.
#[derive(Debug)]
#[must_use = "hold the permit until the session operation finishes"]
pub struct SessionOperation {
    id: String,
}

impl SessionOperation {
    pub fn try_enter(session_id: &str) -> Result<Self> {
        let mut active = ACTIVE
            .lock()
            .map_err(|_| AgentError::Session("in-process admission lock poisoned".into()))?;
        if !active.insert(session_id.to_owned()) {
            return Err(AgentError::Session(
                "session already has an active in-process operation (capacity 1; no waiting queue)"
                    .into(),
            ));
        }
        drop(active);
        Ok(Self {
            id: session_id.to_owned(),
        })
    }
}

impl Drop for SessionOperation {
    fn drop(&mut self) {
        // Poisoned state remains closed: never recover admission speculatively.
        if let Ok(mut active) = ACTIVE.lock() {
            active.remove(&self.id);
        }
    }
}
