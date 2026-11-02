//! Feature-gated probes for composer state and callback lock ownership.
use super::{Ui, UiState};
use crate::overlay::{Origin, TransitionTicket};
use huginn_plugin_api::v1::CommandResult;

pub fn paste_text(state: &mut UiState, text: &str) {
    state.insert_pasted_text(text);
}

pub fn insert_text(state: &mut UiState, text: &str) {
    state.insert_editor_text(text);
}

pub fn visible_text(state: &UiState) -> String {
    state.editor.text.clone()
}

pub fn submitted_text(state: &UiState) -> String {
    state.submitted_editor_text()
}

pub const fn pending_paste_count(state: &UiState) -> usize {
    state.pending_pastes.len()
}

pub const fn cursor(state: &UiState) -> usize {
    state.editor.cursor
}

pub fn set_cursor(state: &mut UiState, cursor: usize) {
    assert!(state.editor.text.is_char_boundary(cursor));
    state.editor.cursor = cursor;
}

pub fn move_left(state: &mut UiState) {
    state.move_editor_left();
}

pub fn move_right(state: &mut UiState) {
    state.move_editor_right();
}

pub fn backspace(state: &mut UiState) {
    state.backspace_editor();
}

pub fn delete(state: &mut UiState) {
    state.delete_editor();
}

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
    ui.apply_result(result, origin, ticket, operation, Vec::new());
}
