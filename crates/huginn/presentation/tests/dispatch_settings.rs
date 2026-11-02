//! Tests for `/settings` host short-circuit dispatch and completion catalog.
//!
//! Exercises `Ui::dispatch_command` and `Ui::refresh_completion` for the
//! `/settings` command without any plugin registration. The host short-circuit
//! lives entirely inside presentation (REQ-01, REQ-16).

use huginn_core::{CancellationToken, Session};
use huginn_presentation::{contributions::Contributions, settings::UiConfig, state::Ui};
use std::sync::Arc;

fn ui() -> Arc<Ui> {
    Ui::new(
        &Session::new(".".into()),
        Arc::new(Contributions::default()),
        CancellationToken::new(),
        Arc::new(UiConfig::defaults()),
        Arc::new(|_: &UiConfig| Ok(())),
    )
    .0
}

#[test]
fn dispatch_opens_settings_dialog() {
    let ui = ui();

    // Dispatch should return true (consumed)
    assert!(ui.dispatch_command("/settings"));

    // Settings dialog should be open and the editor must not keep the
    // command text, because the command never reaches the model.
    let state = ui.lock();
    assert!(state.settings().is_some(), "Settings dialog should be open");
    assert!(state.editor.text.is_empty(), "Editor should be cleared");
    drop(state);
}

#[test]
fn dispatch_opens_settings_with_trailing_whitespace() {
    let ui = ui();

    // Trailing whitespace must not break the host short-circuit.
    assert!(ui.dispatch_command("/settings   "));

    let state = ui.lock();
    assert!(state.settings().is_some(), "Settings dialog should be open");
    drop(state);
}

#[test]
fn dispatch_settings_clears_editor() {
    let ui = ui();

    // Dispatch /settings command (without pre-filling editor)
    assert!(ui.dispatch_command("/settings"));

    let state = ui.lock();
    assert!(
        state.editor.text.is_empty(),
        "Editor should be empty after dispatch"
    );
    assert!(state.settings().is_some(), "Settings should be open");
    drop(state);
}

#[test]
fn dispatch_settings_is_not_sent_as_a_prompt() {
    let ui = ui();

    // A contributed command consumes the input and returns true. The host
    // short-circuit matches before any plugin lookup, so a plugin can never
    // shadow `/settings` (REQ-01, REQ-15).
    assert!(ui.dispatch_command("/settings"));

    let state = ui.lock();
    assert!(state.settings().is_some());
    assert!(
        state.conversation.is_empty(),
        "The command must not be recorded as a conversation turn"
    );
    drop(state);
}

#[test]
fn refresh_completion_lists_settings() {
    let ui = ui();

    // Set up the editor with a trigger.
    ui.lock().editor.insert("/");

    // Refresh completions without a configured model.
    ui.refresh_completion(false);

    let state = ui.lock();
    let settings_cmd = state
        .completion
        .items
        .iter()
        .find(|item| item.label == "/settings");
    assert!(settings_cmd.is_some(), "/settings must be listed");
    assert_eq!(settings_cmd.unwrap().description, "TUI settings");
    drop(state);
}

#[test]
fn refresh_completion_lists_settings_even_when_model_configured() {
    let ui = ui();

    // Set up the editor with a trigger.
    ui.lock().editor.insert("/");

    // Refresh completions with a configured model.
    ui.refresh_completion(true);

    let state = ui.lock();
    let settings_cmd = state
        .completion
        .items
        .iter()
        .find(|item| item.label == "/settings");
    assert!(
        settings_cmd.is_some(),
        "/settings must stay listed when the model is configured"
    );
    assert_eq!(settings_cmd.unwrap().description, "TUI settings");
    drop(state);
}
