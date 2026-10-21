//! Observation-only access to revision boundaries.
use super::State;
use huginn_core::providers::{ConnectionPhase, ConnectionSnapshot, SelectionSnapshot};

pub struct ObservationState(State);

impl ObservationState {
    pub fn new(revision: u64) -> Self {
        let state = State::new();
        state
            .observed
            .send_modify(|snapshot| snapshot.revision = revision);
        Self(state)
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<ConnectionSnapshot> {
        self.0.observed.subscribe()
    }

    pub fn observe(
        &mut self,
        phase: ConnectionPhase,
        selection: Option<SelectionSnapshot>,
    ) -> bool {
        self.0.observe(phase, selection)
    }
}
