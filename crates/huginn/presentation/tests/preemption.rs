//! Tests for settings modal preemption behavior.
//!
//! Exercises the precedence chain via `Ui` to verify that an approval modal
//! preempts the settings dialog (REQ-12) and that Esc inside the settings
//! dialog closes it without cancelling a running turn (REQ-13).

use huginn_core::{ApprovalPrompt, ApprovalRequest, CancellationToken, Session};
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

fn request() -> ApprovalRequest {
    ApprovalRequest {
        id: "one".into(),
        action: "native.edit".into(),
        preview: "Test approval".into(),
        resource: None,
        provenance: None,
    }
}

fn overlay_key(ui: &Ui, code: crossterm::event::KeyCode) {
    let (ticket, focus) = {
        let state = ui.lock();
        (state.overlays.ticket(), state.overlays.focus())
    };
    ui.overlay_input(
        crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        )),
        ticket,
        focus,
    );
}

#[tokio::test]
async fn settings_modal_preempted_by_approval() {
    let ui = ui();

    // Open settings first.
    ui.dispatch_command("/settings");
    {
        let state = ui.lock();
        assert!(state.settings().is_some(), "Settings should be open");
        drop(state);
    }

    // Request approval. The approval modal has to preempt the settings
    // dialog, so the settings working copy is dropped without saving.
    let cancel = CancellationToken::new();
    let task = {
        let ui = ui.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            ui.request_decision(
                &request(),
                &[
                    huginn_core::ApprovalDecision::Reject,
                    huginn_core::ApprovalDecision::ApproveOnce,
                ],
                cancel,
            )
            .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while ui.lock().modal().is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("approval modal should appear");

    {
        let state = ui.lock();
        assert!(
            state.settings().is_none(),
            "Settings should be cleared by the approval modal"
        );
        assert!(state.modal().is_some(), "Approval modal should be present");
        drop(state);
    }

    // Resolve the approval so the spawned task completes.
    ui.resolve_modal(false);
    let _ = task.await;

    // Settings must stay closed after the approval resolves.
    let state = ui.lock();
    assert!(
        state.settings().is_none(),
        "Settings should stay closed after the approval resolves"
    );
    drop(state);
}

#[test]
fn settings_esc_does_not_cancel_turn() {
    let ui = ui();

    // Install a running turn with a cancel token.
    let cancel = CancellationToken::new();
    ui.lock().turn_cancel = Some(cancel.clone());

    // Open settings.
    ui.dispatch_command("/settings");
    {
        let state = ui.lock();
        assert!(state.settings().is_some(), "Settings should be open");
        assert!(state.turn_cancel.is_some(), "Turn should be running");
        drop(state);
    }

    // Esc closes the dialog.
    overlay_key(&ui, crossterm::event::KeyCode::Esc);

    {
        let state = ui.lock();
        assert!(
            state.settings().is_none(),
            "Settings should be closed by Esc"
        );
        drop(state);
    }

    // The running turn must not be cancelled by Esc inside the dialog.
    assert!(
        !cancel.is_cancelled(),
        "Esc inside the settings dialog must not cancel the running turn"
    );
}

#[test]
fn settings_survives_tab_navigation() {
    let ui = ui();

    ui.dispatch_command("/settings");
    {
        let state = ui.lock();
        assert!(state.settings().is_some(), "Settings should be open");
        drop(state);
    }

    // Tab cycles to the next tab without closing the dialog.
    overlay_key(&ui, crossterm::event::KeyCode::Tab);
    {
        let state = ui.lock();
        assert!(
            state.settings().is_some(),
            "Settings should stay open while cycling tabs"
        );
        drop(state);
    }

    // Down moves the selection without closing the dialog.
    overlay_key(&ui, crossterm::event::KeyCode::Down);
    {
        let state = ui.lock();
        assert!(
            state.settings().is_some(),
            "Settings should stay open while moving the selection"
        );
        drop(state);
    }
}

#[test]
fn settings_can_be_reopened_after_esc() {
    let ui = ui();

    ui.dispatch_command("/settings");
    overlay_key(&ui, crossterm::event::KeyCode::Esc);
    {
        let state = ui.lock();
        assert!(state.settings().is_none(), "Esc should close the dialog");
        drop(state);
    }

    ui.dispatch_command("/settings");
    let state = ui.lock();
    assert!(
        state.settings().is_some(),
        "Settings should reopen after Esc"
    );
    drop(state);
}
