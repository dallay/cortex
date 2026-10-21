//! Narrow probes for operation allocation and callback lock ownership.
use super::{Ui, UiState};
use crate::overlay::{Origin, TransitionTicket};
use huginn_plugin_api::v1::CommandResult;

pub const fn operation_sequence(state: &UiState) -> u64 {
    state.next_operation_id
}

pub fn host_mutexes_available(ui: &Ui) -> bool {
    ui.state.try_lock().is_ok()
        && ui.active_operations.try_lock().is_ok()
        && ui.error_deduplicator.try_lock().is_ok()
}

pub fn apply_result(
    ui: &Ui,
    result: CommandResult,
    origin: Origin,
    ticket: TransitionTicket,
    operation: Option<crate::event::OperationId>,
) {
    ui.apply_result(result, origin, ticket, operation);
}
