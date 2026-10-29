//! Narrow failure-injection seam for kernel integration tests.
use super::{AgentError, Result, ServiceId, Supervisor};
use std::thread;

pub fn force_lease_counter_overflow(supervisor: &Supervisor, service: &ServiceId) -> Result<()> {
    let slots = supervisor
        .service_slots
        .lock()
        .map_err(|_| AgentError::Composition("service slot table poisoned".into()))?;
    let slot = slots.entries.get(service).ok_or_else(|| {
        AgentError::Composition(format!("missing test service slot {}", service.0))
    })?;
    let mut state = slot
        .gate
        .state
        .lock()
        .map_err(|_| AgentError::Composition("lease gate poisoned".into()))?;
    state.active = usize::MAX;
    drop(state);
    drop(slots);
    Ok(())
}

pub fn remove_lease_gate(supervisor: &mut Supervisor, service: &ServiceId) -> bool {
    supervisor.lease_gates.remove(service).is_some()
}

pub fn poison_lease_gate(supervisor: &Supervisor, service: &ServiceId) -> Result<()> {
    let gate = supervisor
        .lease_gates
        .get(service)
        .cloned()
        .ok_or_else(|| AgentError::Composition(format!("missing test lease gate {}", service.0)))?;
    let _ = thread::spawn(move || {
        let _state = gate.state.lock().unwrap();
        panic!("poison lease gate for test");
    })
    .join();
    Ok(())
}

pub fn poison_service_slots(supervisor: &Supervisor) {
    let service_slots = supervisor.service_slots.clone();
    let _ = thread::spawn(move || {
        let _slots = service_slots.lock().unwrap();
        panic!("poison service slot table for test");
    })
    .join();
}

pub fn lease_locks_are_unpoisoned(supervisor: &Supervisor, service: &ServiceId) -> bool {
    let Ok(slots) = supervisor.service_slots.lock() else {
        return false;
    };
    let Some(slot) = slots.entries.get(service) else {
        return false;
    };
    !slot.gate.state.is_poisoned()
}
