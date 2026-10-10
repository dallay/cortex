use huginn_core::{
    ApprovalPolicy, ApprovalRequest, CancellationToken, Event, EventSink, Message, Role, Session,
};
use huginn_presentation::{
    contributions::Contributions,
    state::{Editor, Ui, UiState},
    Interaction,
};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

fn ui() -> Arc<Ui> {
    Ui::new(
        &Session::new(".".into()),
        BTreeSet::new(),
        Arc::new(Contributions::default()),
        CancellationToken::new(),
    )
    .0
}
fn request() -> ApprovalRequest {
    ApprovalRequest {
        id: "one".into(),
        action: "native.edit".into(),
        preview: format!(
            "--- file\n+++ file\n{}\nEND",
            "+addition\n-removal\n".repeat(1000)
        ),
    }
}
async fn pending(ui: &Ui) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while ui.lock().modal.is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[test]
fn editor_multiline_unicode_edits_without_splitting_utf8() {
    let mut editor = Editor::default();
    editor.insert("你好\nnext");
    editor.home();
    editor.vertical(false);
    editor.end();
    editor.backspace();
    assert_eq!(editor.text, "你\nnext");
    editor.right();
    editor.delete();
    editor.insert("a\n");
    assert_eq!(editor.text, "你\na\next");
    assert!(editor.text.is_char_boundary(editor.cursor));
}
#[test]
fn resume_and_synchronize_commit_messages_once_not_mutable_tail() {
    let mut state = UiState::default();
    let mut session = Session::new(".".into());
    session.messages.push(Message::text(Role::User, "hello"));
    state.synchronize(&session);
    state.project(
        Event::Text {
            text: "```rust\npartial".into(),
        },
        "Tool",
        "Tool error",
    );
    assert_eq!(state.committed.len(), 1);
    assert_eq!(state.tail, "```rust\npartial");
    session
        .messages
        .push(Message::text(Role::Assistant, "```rust\ncomplete\n```"));
    state.synchronize(&session);
    state.synchronize(&session);
    assert_eq!(state.committed.len(), 2);
    assert!(state.tail.is_empty());
    assert!(state.committed[1].contains("complete"));
    session.interrupted = true;
    state.synchronize(&session);
    assert!(state.status.contains("nothing replayed"));
}
#[tokio::test]
async fn stalled_renderer_keeps_all_transitions_and_composer_responsive() {
    let ui = ui();
    ui.lock().editor.insert("next\nprompt");
    let start = std::time::Instant::now();
    for _ in 0..10_000 {
        ui.emit(Event::Text { text: "x".into() });
    }
    ui.emit(Event::ApprovalResolved {
        id: "one".into(),
        approved: false,
    });
    ui.emit(Event::TurnFinished);
    // Token deltas fold into `tail` and are not retained; only the two
    // authoritative transitions stay in the event log.
    assert_eq!(ui.lock().events.len(), 2);
    assert_eq!(ui.lock().tail.len(), 10_000);
    assert_eq!(ui.lock().editor.text, "next\nprompt");
    assert!(start.elapsed() < Duration::from_secs(5));
}
#[tokio::test]
async fn approval_retains_entire_diff_and_requires_fresh_armed_code() {
    let ui = ui();
    let payload = request();
    let task = {
        let ui = ui.clone();
        let payload = payload.clone();
        tokio::spawn(async move { ui.approve(&payload, CancellationToken::new()).await })
    };
    pending(&ui).await;
    {
        let mut state = ui.lock();
        let modal = state.modal.as_mut().unwrap();
        modal.last_scroll = 200;
        modal.scroll = usize::MAX;
        modal.scroll_up();
        assert_eq!(
            modal.scroll, 199,
            "End followed by Up must leave the bottom"
        );
        assert_eq!(modal.request.preview, payload.preview);
        modal.typed = modal.code.clone();
        assert!(
            !modal.confirm(),
            "even a matching code before display cannot approve"
        );
        modal.armed = true;
        modal.typed = "y".into();
        assert!(!modal.confirm());
        modal.typed = modal.code.clone();
        assert!(modal.confirm());
        drop(state);
    }
    ui.resolve_modal(true);
    assert!(task.await.unwrap().unwrap());
    assert!(ui.lock().modal.is_none());
}
#[tokio::test]
async fn approval_cancel_is_fail_closed_and_next_request_uses_new_code() {
    let ui = ui();
    let cancel = CancellationToken::new();
    let task = {
        let ui = ui.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { ui.approve(&request(), cancel).await })
    };
    pending(&ui).await;
    let old = ui.lock().modal.as_ref().unwrap().code.clone();
    cancel.cancel();
    assert!(task.await.unwrap().is_err());
    assert!(ui.lock().modal.is_none());
    let task = {
        let ui = ui.clone();
        tokio::spawn(async move { ui.approve(&request(), CancellationToken::new()).await })
    };
    pending(&ui).await;
    assert_ne!(ui.lock().modal.as_ref().unwrap().code, old);
    ui.resolve_modal(false);
    assert!(!task.await.unwrap().unwrap());
    ui.shutdown();
    assert!(ui
        .approve(&request(), CancellationToken::new())
        .await
        .is_err());
    assert!(ui.prompt().await.is_none());
}
#[test]
fn approval_preview_matches_line_adapter_semantics() {
    use huginn_presentation::state::approval_preview;
    let shell = approval_preview("native.shell", 2, "Directory: /tmp/work\nCommand: pwd");
    assert!(shell.starts_with(
        "Approval for native.shell (request #2 this turn):\nAction: native.shell\nEffect: run command in workspace (/tmp/work)\n"
    ));
    assert!(shell.ends_with("Directory: /tmp/work\nCommand: pwd"));
    let edit = approval_preview(
        "native.edit",
        1,
        "--- README.md\n+++ README.md\n@@\n-old\n+new",
    );
    assert!(edit.contains("\nEffect: edit README.md\n"));
    let unknown = approval_preview("mcp.fixture.echo", 1, "just some text");
    assert!(!unknown.contains("Effect:"));
    assert!(unknown.contains("just some text"));
}

#[tokio::test]
async fn approval_modal_numbers_requests_per_turn_and_resets() {
    let ui = ui();
    let first = {
        let ui = ui.clone();
        tokio::spawn(async move { ui.approve(&request(), CancellationToken::new()).await })
    };
    pending(&ui).await;
    assert_eq!(ui.lock().modal.as_ref().unwrap().number, 1);
    ui.resolve_modal(true);
    assert!(first.await.unwrap().unwrap());
    let second = {
        let ui = ui.clone();
        tokio::spawn(async move { ui.approve(&request(), CancellationToken::new()).await })
    };
    pending(&ui).await;
    assert_eq!(ui.lock().modal.as_ref().unwrap().number, 2);
    ui.resolve_modal(false);
    assert!(!second.await.unwrap().unwrap());
    // A new turn resets the numbering so its first approval is #1 again.
    ui.begin(CancellationToken::new());
    let third = {
        let ui = ui.clone();
        tokio::spawn(async move { ui.approve(&request(), CancellationToken::new()).await })
    };
    pending(&ui).await;
    assert_eq!(ui.lock().modal.as_ref().unwrap().number, 1);
    ui.resolve_modal(false);
    assert!(!third.await.unwrap().unwrap());
}

#[test]
fn tool_error_uses_contributed_label_and_marks_status() {
    let ui = ui();
    ui.emit(Event::ToolFinished {
        id: "call-1".into(),
        output: "boom".into(),
        is_error: false,
    });
    assert!(ui.lock().tail.contains("Tool result: boom"));
    assert!(!ui.lock().status_error);
    ui.emit(Event::ToolFinished {
        id: "call-2".into(),
        output: "kaput".into(),
        is_error: true,
    });
    assert!(ui.lock().tail.contains("Tool error: kaput"));
    assert!(!ui.lock().tail.contains("failed"));
    assert!(ui.lock().status_error);
    assert_eq!(ui.lock().status, "Tool error");
}

#[test]
fn projected_text_cannot_emit_terminal_escape_sequences() {
    let ui = ui();
    ui.emit(Event::Text {
        text: "\x1b[2Jhello\x00".into(),
    });
    assert!(!ui.lock().tail.contains('\x1b'));
    assert!(!ui.lock().tail.contains('\x00'));
}

#[test]
fn safe_filters_unicode_bidi_controls_but_keeps_newlines_and_tabs() {
    use huginn_presentation::state::safe;
    let cleaned = safe("\u{200E}action\u{202B} preview\u{2066}detail\u{2069}\n\t");
    assert!(
        !cleaned.contains(['\u{200E}', '\u{202B}', '\u{2066}', '\u{2069}']),
        "bidi controls must be filtered; got: {cleaned:?}"
    );
    assert!(cleaned.contains("action previewdetail"));
    assert!(cleaned.ends_with("\n\t"));
}
