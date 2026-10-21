use huginn_core::{
    ApprovalPrompt, ApprovalRequest, CancellationToken, Event, EventSink, Message, Role,
    ScopedEventSink, Session,
};
use huginn_presentation::{
    contributions::Contributions,
    state::{Editor, Ui, UiState},
    Interaction,
};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Default)]
struct FakeClock(AtomicU64);
impl FakeClock {
    fn advance_ms(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}
impl huginn_presentation::event::Clock for FakeClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

fn ui() -> Arc<Ui> {
    ui_with_clock(Arc::new(FakeClock::default()))
}
fn ui_with_clock(clock: Arc<FakeClock>) -> Arc<Ui> {
    Ui::new_with_clock(
        &Session::new(".".into()),
        Arc::new(Contributions::default()),
        CancellationToken::new(),
        Arc::new(huginn_presentation::settings::UiConfig::defaults()),
        Arc::new(|_: &huginn_presentation::settings::UiConfig| Ok(())),
        clock,
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
        while ui.lock().modal().is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[test]
fn oversized_ui_diagnostic_is_bounded_without_truncating_conversation() {
    let mut state = UiState::default();
    let original = format!("diagnostic {}", "界".repeat(8192));
    state.project_ui_event(
        huginn_presentation::event::EventKind::Diagnostic,
        original.clone(),
    );
    let record = state.notifications.iter().last().unwrap();
    assert!(record.summary.len() <= 4096);
    assert!(record.summary.ends_with(" [truncated]"));
    let feedback = state.status.feedback.as_ref().unwrap();
    assert!(feedback.len() <= 4096);
    assert!(feedback.ends_with(" [truncated]"));
    assert_eq!(feedback, &record.summary);

    state.conversation(original.clone());
    assert_eq!(state.conversation.last().unwrap(), &original);
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
fn explicit_ui_categories_do_not_depend_on_message_text() {
    let concrete = ui();
    let baseline = concrete.lock().notifications.len();
    let interaction: Arc<dyn Interaction> = concrete.clone();
    interaction.conversation("identical".into());
    interaction.overlay("identical".into());
    interaction.notification("identical".into());
    interaction.status("identical".into());
    interaction.diagnostic("identical".into());

    let state = concrete.lock();
    assert_eq!(state.conversation, ["identical"]);
    let events = state
        .notifications
        .iter()
        .skip(baseline)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 4);
    assert_eq!(
        events[0].kind,
        huginn_presentation::event::EventKind::Overlay
    );
    assert_eq!(
        events[1].kind,
        huginn_presentation::event::EventKind::Notification
    );
    assert_eq!(
        events[2].kind,
        huginn_presentation::event::EventKind::Status
    );
    assert_eq!(
        events[3].kind,
        huginn_presentation::event::EventKind::Diagnostic
    );
    assert!(state.status.diagnostic);
    drop(state);
}

#[test]
fn session_reprojection_keeps_all_messages_beyond_ui_buffer_capacity() {
    let mut session = Session::new("large-session".into());
    for index in 0..300 {
        session
            .messages
            .push(Message::text(Role::User, format!("message-{index}")));
    }
    let mut state = UiState::default();
    state.synchronize(&session);
    assert_eq!(state.conversation.len(), 300);
    assert_eq!(state.message_count, 300);
    assert_eq!(state.conversation.first().unwrap(), "User: message-0");
    assert_eq!(state.conversation.last().unwrap(), "User: message-299");
    assert!(state.notifications.len() <= huginn_presentation::event::UI_EVENT_CAPACITY);
}

#[test]
fn engine_and_host_fallback_errors_share_one_operation_diagnostic() {
    let ui = ui_with_clock(Arc::new(FakeClock::default()));
    let interaction: Arc<dyn Interaction> = ui.clone();
    let operation = interaction.begin(CancellationToken::new());
    let scoped = ScopedEventSink {
        sink: ui.as_ref(),
        operation,
    };

    // The loop records its error before returning; the host then emits the
    // returned error as a fallback through the same scoped sink.
    scoped.emit(Event::Error {
        message: "provider unavailable".into(),
    });
    scoped.emit_for(
        huginn_core::OperationId(operation.0.wrapping_add(100)),
        Event::Error {
            message: "provider unavailable".into(),
        },
    );

    assert_eq!(
        ui.lock()
            .notifications
            .iter()
            .filter(|event| {
                event.kind == huginn_presentation::event::EventKind::Diagnostic
                    && event.summary == "provider unavailable"
            })
            .count(),
        1
    );
    interaction.finish_operation(operation);
}

#[test]
fn explicit_event_operation_scope_survives_interleaved_producers() {
    let clock = Arc::new(FakeClock::default());
    let ui = ui_with_clock(clock);
    let interaction: Arc<dyn Interaction> = ui.clone();
    let operation = interaction.begin(CancellationToken::new());
    let operation_b = interaction.begin(CancellationToken::new());
    let scoped_a = ScopedEventSink {
        sink: ui.as_ref(),
        operation,
    };
    let scoped_b = ScopedEventSink {
        sink: ui.as_ref(),
        operation: operation_b,
    };
    scoped_a.emit(Event::Error {
        message: "engine failure".into(),
    });
    scoped_b.emit(Event::Error {
        message: "engine failure".into(),
    });
    scoped_a.emit(Event::Error {
        message: "engine failure".into(),
    });
    assert_eq!(
        ui.lock()
            .notifications
            .iter()
            .filter(
                |event| event.kind == huginn_presentation::event::EventKind::Diagnostic
                    && event.summary == "engine failure"
            )
            .count(),
        2
    );
    interaction.finish_operation(operation);
    let before_stale_emit = ui.lock().notifications.len();
    ui.emit_for(
        operation,
        Event::Error {
            message: "late producer failure".into(),
        },
    );
    assert_eq!(ui.lock().notifications.len(), before_stale_emit);
    let operation_c = interaction.begin(CancellationToken::new());
    ui.emit_for(
        operation_c,
        Event::Error {
            message: "engine failure".into(),
        },
    );
    assert_eq!(
        ui.lock()
            .notifications
            .iter()
            .filter(
                |event| event.kind == huginn_presentation::event::EventKind::Diagnostic
                    && event.summary == "engine failure"
            )
            .count(),
        3
    );
}

#[test]
fn error_event_producer_uses_injected_clock_and_deduplicates_by_operation() {
    use huginn_presentation::event::EventKind;

    let clock = Arc::new(FakeClock::default());
    let ui = ui_with_clock(clock.clone());
    let interaction: Arc<dyn Interaction> = ui.clone();
    let operation = interaction.begin(CancellationToken::new());
    let emit_error = || {
        ui.emit(Event::Error {
            message: "provider failed".into(),
        });
    };

    emit_error();
    clock.advance_ms(1_999);
    emit_error();
    let matching = || {
        ui.lock()
            .notifications
            .iter()
            .filter(|event| {
                event.kind == EventKind::Diagnostic && event.summary == "provider failed"
            })
            .count()
    };
    assert_eq!(matching(), 1);
    clock.advance_ms(1);
    emit_error();
    assert_eq!(matching(), 2);
    interaction.finish_operation(operation);
}

#[test]
fn oversized_errors_remain_visible_without_dedup_caching_and_have_bounded_ui_summaries() {
    use huginn_presentation::event::{EventKind, ERROR_DEDUP_MAX_MESSAGE_BYTES};

    let clock = Arc::new(FakeClock::default());
    let ui = ui_with_clock(clock);
    let interaction: Arc<dyn Interaction> = ui.clone();
    let operation = interaction.begin(CancellationToken::new());
    let full_message = "s".repeat(ERROR_DEDUP_MAX_MESSAGE_BYTES + 37);
    for _ in 0..2 {
        ui.emit(Event::Error {
            message: full_message.clone(),
        });
    }
    let diagnostics: Vec<_> = ui
        .lock()
        .notifications
        .iter()
        .filter(|event| event.kind == EventKind::Diagnostic)
        .cloned()
        .collect();
    assert_eq!(diagnostics.len(), 2);
    // Oversized errors bypass dedup caching, but their UI projection is bounded.
    assert!(diagnostics.iter().all(|event| {
        event.summary.len() <= 4096
            && event.summary.ends_with(" [truncated]")
            && full_message.starts_with(event.summary.trim_end_matches(" [truncated]"))
    }));
    interaction.finish_operation(operation);
}

#[test]
fn session_and_workspace_context_are_not_conversation() {
    let session = Session::new("session-id".into());
    let (ui, _) = Ui::new(
        &session,
        Arc::new(Contributions::default()),
        CancellationToken::new(),
        Arc::new(huginn_presentation::settings::UiConfig::defaults()),
        Arc::new(|_: &huginn_presentation::settings::UiConfig| Ok(())),
    );
    let state = ui.lock();
    assert!(state.conversation.is_empty());
    assert!(state
        .notifications
        .iter()
        .any(|event| event.summary.starts_with("Session: ")));
    assert!(state
        .notifications
        .iter()
        .any(|event| event.summary.starts_with("Workspace: ")));
    drop(state);
}

#[test]
fn notification_does_not_enter_conversation_history() {
    let ui = ui();
    let before = ui.lock().conversation.clone();
    ui.notification("notice not history".into());
    let state = ui.lock();
    assert_eq!(state.conversation, before);
    assert_eq!(state.notifications.len(), 3);
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
    assert_eq!(state.conversation.len(), 1);
    assert_eq!(state.tail, "```rust\npartial");
    session
        .messages
        .push(Message::text(Role::Assistant, "```rust\ncomplete\n```"));
    state.synchronize(&session);
    state.synchronize(&session);
    assert_eq!(state.conversation.len(), 2);
    assert!(state.tail.is_empty());
    assert!(state.conversation[1].contains("complete"));
    session.interrupted = true;
    state.synchronize(&session);
    assert!(state.status.notice.contains("nothing replayed"));
    assert!(state.status.diagnostic);
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
        tokio::spawn(async move {
            ui.request_decision(&payload, CancellationToken::new())
                .await
        })
    };
    pending(&ui).await;
    {
        let mut state = ui.lock();
        let modal = state.modal_mut().unwrap();
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
    assert!(ui.lock().modal().is_none());
}
#[tokio::test]
async fn approval_cancel_is_fail_closed_and_next_request_uses_new_code() {
    let ui = ui();
    let cancel = CancellationToken::new();
    let task = {
        let ui = ui.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { ui.request_decision(&request(), cancel).await })
    };
    pending(&ui).await;
    let old = ui.lock().modal().unwrap().code.clone();
    cancel.cancel();
    assert!(task.await.unwrap().is_err());
    assert!(ui.lock().modal().is_none());
    let task = {
        let ui = ui.clone();
        tokio::spawn(async move {
            ui.request_decision(&request(), CancellationToken::new())
                .await
        })
    };
    pending(&ui).await;
    assert_ne!(ui.lock().modal().unwrap().code, old);
    ui.resolve_modal(false);
    assert!(!task.await.unwrap().unwrap());
    ui.shutdown();
    assert!(ui
        .request_decision(&request(), CancellationToken::new())
        .await
        .is_err());
    assert!(ui.prompt().await.is_none());
}

#[tokio::test]
async fn dropping_approval_waiter_retires_authority_and_sender() {
    let ui = ui();
    let task = {
        let ui = ui.clone();
        tokio::spawn(async move {
            ui.request_decision(&request(), CancellationToken::new())
                .await
        })
    };
    pending(&ui).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        ui.lock().overlays.snapshot().is_none(),
        "aborted waiter must close Approval"
    );
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
async fn approval_requests_serialize_and_stale_lease_cannot_resolve_successor() {
    use crossterm::event::{Event as Input, KeyCode, KeyEvent, KeyModifiers};
    let ui = ui();
    let payload = request();
    let first = ui.request_decision(&payload, CancellationToken::new());
    tokio::pin!(first);
    assert!(futures::poll!(&mut first).is_pending());
    let old = ui.lock().overlays.snapshot().unwrap();
    let old_ticket = ui.lock().overlays.ticket();
    let second_request = ApprovalRequest {
        id: "second".into(),
        ..request()
    };
    let second = ui.request_decision(&second_request, CancellationToken::new());
    tokio::pin!(second);
    assert!(futures::poll!(&mut second).is_pending());
    assert_eq!(ui.lock().overlays.snapshot(), Some(old));
    overlay_key(&ui, KeyCode::Esc);
    assert!(!first.await.unwrap());
    assert!(futures::poll!(&mut second).is_pending());
    let next = ui.lock().overlays.snapshot().unwrap();
    assert_ne!(next.id, old.id);
    assert_eq!(ui.lock().modal().unwrap().request.id, "second");
    assert!(!ui.lock().modal().unwrap().armed);
    ui.overlay_input(
        Input::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        old_ticket,
        old.focus,
    );
    assert_eq!(ui.lock().overlays.snapshot(), Some(next));
    assert!(futures::poll!(&mut second).is_pending());
    let ticket = ui.lock().overlays.ticket();
    ui.overlay_input(
        Input::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        ticket,
        next.focus,
    );
    assert!(!second.await.unwrap());
    assert!(ui.lock().overlays.snapshot().is_none());
    assert!(ui.lock().editor.text.is_empty());
}

#[tokio::test]
async fn approval_cancellation_wins_ready_answer_and_shutdown_wakes_serial_waiters() {
    let ui = ui();
    let cancel = CancellationToken::new();
    let payload = request();
    let decision = ui.request_decision(&payload, cancel.clone());
    tokio::pin!(decision);
    assert!(futures::poll!(&mut decision).is_pending());
    {
        let mut state = ui.lock();
        let modal = state.modal_mut().unwrap();
        modal.armed = true;
        modal.typed = modal.code.clone();
        drop(state);
    }
    ui.resolve_modal(true); // answer and cancellation both ready before next poll
    cancel.cancel();
    assert!(matches!(
        decision.await,
        Err(huginn_core::AgentError::Cancelled)
    ));

    let first = ui.request_decision(&payload, CancellationToken::new());
    let queued = ui.request_decision(&payload, CancellationToken::new());
    tokio::pin!(first, queued);
    assert!(futures::poll!(&mut first).is_pending());
    assert!(futures::poll!(&mut queued).is_pending());
    ui.shutdown();
    assert!(first.await.is_err());
    assert!(queued.await.is_err());
    assert!(ui.lock().overlays.snapshot().is_none());
}

#[tokio::test]
async fn approval_sender_loss_is_denial_and_late_input_has_no_authority() {
    let ui = ui();
    let payload = request();
    let decision = ui.request_decision(&payload, CancellationToken::new());
    tokio::pin!(decision);
    assert!(futures::poll!(&mut decision).is_pending());
    let (ticket, focus) = {
        let state = ui.lock();
        (state.overlays.ticket(), state.overlays.focus())
    };
    ui.close_overlay(ticket, focus); // drops the unique sender, without sending
    assert!(!decision.await.unwrap());
    ui.overlay_input(
        crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        )),
        ticket,
        focus,
    );
    assert!(ui.lock().overlays.snapshot().is_none());
    assert!(ui.lock().editor.text.is_empty());
}

async fn receipt(ui: &Ui) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !ui.refresh_contributions() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn settings_apply_reenters_ui_once_and_late_receipt_cannot_alter_approval() {
    for succeeds in [true, false] {
        let slot = Arc::new(std::sync::Mutex::new(std::sync::Weak::<Ui>::new()));
        let (entered, mut entering) = tokio::sync::mpsc::unbounded_channel();
        let (release, released) = std::sync::mpsc::channel();
        let released = std::sync::Mutex::new(released);
        let calls = Arc::new(AtomicU64::new(0));
        let ui = Ui::new(
            &Session::new(".".into()),
            Arc::new(Contributions::default()),
            CancellationToken::new(),
            Arc::new(huginn_presentation::settings::UiConfig::defaults()),
            Arc::new({
                let slot = slot.clone();
                let calls = calls.clone();
                move |_| {
                    let ui = slot.lock().unwrap().upgrade().unwrap();
                    assert!(
                        ui.state.try_lock().is_ok(),
                        "save callback must run outside UiState"
                    );
                    calls.fetch_add(1, Ordering::SeqCst);
                    entered.send(()).unwrap();
                    released.lock().unwrap().recv().unwrap();
                    if succeeds {
                        Ok(())
                    } else {
                        anyhow::bail!("disk full")
                    }
                }
            }),
        )
        .0;
        *slot.lock().unwrap() = Arc::downgrade(&ui);
        ui.dispatch_command("/settings");
        {
            let mut state = ui.lock();
            state.settings_mut().unwrap().working.sound.bell = true;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(ui.apply_settings());
        entering.recv().await.unwrap();
        assert!(!ui.apply_settings());
        let approval = {
            let ui = ui.clone();
            tokio::spawn(async move {
                ui.request_decision(&request(), CancellationToken::new())
                    .await
            })
        };
        pending(&ui).await;
        let before = ui.lock().overlays.snapshot();
        ui.dispatch_command("/settings");
        assert_eq!(
            ui.lock().overlays.snapshot(),
            before,
            "ordinary open cannot preempt Approval"
        );
        release.send(()).unwrap();
        receipt(&ui).await;
        assert_eq!(ui.lock().overlays.snapshot(), before);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(ui.lock().settings().is_none());
        overlay_key(&ui, crossterm::event::KeyCode::Esc);
        assert!(!approval.await.unwrap().unwrap());
        ui.dispatch_command("/settings");
        assert_eq!(
            ui.lock().settings().unwrap().working.sound.bell,
            if succeeds {
                true
            } else {
                huginn_presentation::settings::UiConfig::defaults()
                    .sound
                    .bell
            }
        );
    }
}

#[tokio::test]
async fn settings_failed_apply_preserves_copy_and_error_and_can_retry() {
    let calls = Arc::new(AtomicU64::new(0));
    let ui = Ui::new(
        &Session::new(".".into()),
        Arc::new(Contributions::default()),
        CancellationToken::new(),
        Arc::new(huginn_presentation::settings::UiConfig::defaults()),
        Arc::new({
            let calls = calls.clone();
            move |_| {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    anyhow::bail!("disk full");
                }
                Ok(())
            }
        }),
    )
    .0;
    ui.dispatch_command("/settings");
    ui.lock().settings_mut().unwrap().working.theme.name_index = 1;
    let before = ui.lock().settings().unwrap().projected_ui_config();
    let notifications = ui.lock().notifications.len();
    assert!(ui.apply_settings());
    receipt(&ui).await;
    assert_eq!(ui.lock().settings().unwrap().projected_ui_config(), before);
    assert_eq!(
        ui.lock().settings().unwrap().last_error.as_deref(),
        Some("disk full")
    );
    assert_eq!(ui.lock().notifications.len(), notifications);
    assert!(ui.apply_settings());
    receipt(&ui).await;
    assert!(ui.lock().overlays.snapshot().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(ui.lock().conversation.is_empty());
}

#[tokio::test]
async fn stale_settings_input_cannot_mutate_replacement_or_cancel_turn() {
    let ui = ui();
    let turn = CancellationToken::new();
    ui.begin(turn.clone());
    ui.dispatch_command("/settings");
    let (ticket, focus) = {
        let state = ui.lock();
        (state.overlays.ticket(), state.overlays.focus())
    };
    ui.lock().settings_mut().unwrap().working.theme.name_index = 1;
    ui.dispatch_command("/settings");
    let before = ui.lock().overlays.snapshot();
    ui.overlay_input(
        crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        )),
        ticket,
        focus,
    );
    assert_eq!(ui.lock().overlays.snapshot(), before);
    assert_eq!(ui.lock().settings().unwrap().working.theme.name_index, 0);
    overlay_key(&ui, crossterm::event::KeyCode::Esc);
    assert!(!turn.is_cancelled());
    assert!(ui.lock().overlays.snapshot().is_none());
    assert!(ui.lock().editor.text.is_empty());
}

#[test]
fn settings_horizontal_arrows_cycle_toasts_without_leaving_the_row() {
    use crossterm::event::KeyCode;
    let ui = ui();
    assert!(ui.dispatch_command("/settings"));
    for _ in 0..3 {
        overlay_key(&ui, KeyCode::Tab);
    }
    // REQ-05 permits Left to advance too. Both arrows edit only the
    // selected cycle row; Tab/BackTab remain section navigation.
    for arrow in [KeyCode::Right, KeyCode::Left] {
        for (expected, label) in [(1, "level: warn"), (2, "level: error"), (0, "level: info")] {
            overlay_key(&ui, arrow);
            let state = ui.lock();
            let dialog = state.settings().unwrap();
            assert_eq!(dialog.working.toasts.level, expected, "SET-05: {arrow:?}");
            assert_eq!(dialog.active_tab, 3);
            let snapshot = dialog.snapshot();
            assert_eq!(snapshot.selected, 0);
            assert_eq!(snapshot.items[0].label, label);
            assert_eq!(dialog.working.toasts.duration, 0);
            drop(state);
        }
    }
    overlay_key(&ui, KeyCode::Down);
    for arrow in [KeyCode::Right, KeyCode::Left] {
        for (expected, label) in [
            (1, "duration: 5s"),
            (2, "duration: 10s"),
            (3, "duration: off"),
            (0, "duration: 3s"),
        ] {
            overlay_key(&ui, arrow);
            let state = ui.lock();
            let dialog = state.settings().unwrap();
            assert_eq!(dialog.working.toasts.duration, expected);
            assert_eq!(dialog.active_tab, 3);
            let snapshot = dialog.snapshot();
            assert_eq!(snapshot.selected, 1);
            assert_eq!(snapshot.items[1].label, label);
            assert_eq!(dialog.working.toasts.level, 0);
            drop(state);
        }
    }
}

#[test]
fn settings_section_keys_preserve_noncycle_values_and_escape_toasts() {
    use crossterm::event::KeyCode;
    let ui = ui();
    ui.dispatch_command("/settings");
    let defaults = ui.lock().settings().unwrap().projected_ui_config();
    for (key, tab) in [
        (KeyCode::Right, 1),
        (KeyCode::Right, 2),
        (KeyCode::Left, 1),
        (KeyCode::Left, 0),
        (KeyCode::Left, 4),
        (KeyCode::Right, 0),
        (KeyCode::BackTab, 4),
        (KeyCode::BackTab, 3),
        (KeyCode::Tab, 4),
        (KeyCode::BackTab, 3),
        (KeyCode::BackTab, 2),
    ] {
        overlay_key(&ui, key);
        let state = ui.lock();
        let dialog = state.settings().unwrap();
        assert_eq!(dialog.active_tab, tab);
        assert_eq!(dialog.projected_ui_config(), defaults);
        assert_eq!(dialog.snapshot().selected, 0);
        drop(state);
    }
}

#[tokio::test]
async fn settings_cycle_arrows_only_persist_displayed_values_on_enter() {
    use crossterm::event::KeyCode;
    use huginn_presentation::settings::UiConfig;
    let saved = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ui = Ui::new(
        &Session::new(".".into()),
        Arc::new(Contributions::default()),
        CancellationToken::new(),
        Arc::new(UiConfig::defaults()),
        Arc::new({
            let saved = saved.clone();
            move |config| {
                saved.lock().unwrap().push(config.clone());
                Ok(())
            }
        }),
    )
    .0;
    let turn = CancellationToken::new();
    ui.begin(turn.clone());
    for apply in [false, true] {
        ui.dispatch_command("/settings");
        for _ in 0..3 {
            overlay_key(&ui, KeyCode::Tab);
        }
        overlay_key(&ui, KeyCode::Right);
        overlay_key(&ui, KeyCode::Down);
        overlay_key(&ui, KeyCode::Left);
        let displayed = ui.lock().settings().unwrap().projected_ui_config();
        assert_eq!(displayed.toasts.level, "warn");
        assert_eq!(displayed.toasts.duration_secs, 5);
        assert!(saved.lock().unwrap().is_empty());
        overlay_key(&ui, if apply { KeyCode::Enter } else { KeyCode::Esc });
        if apply {
            receipt(&ui).await;
            assert_eq!(*saved.lock().unwrap(), vec![displayed]);
        }
        assert!(ui.lock().overlays.snapshot().is_none());
        assert!(!turn.is_cancelled());
        assert!(ui.lock().conversation.is_empty());
        assert!(ui.lock().editor.text.is_empty());
    }
    ui.dispatch_command("/settings");
    assert_eq!(
        ui.lock().settings().unwrap().projected_ui_config(),
        saved.lock().unwrap()[0]
    );
}

#[test]
fn settings_catalog_and_shell_navigation_never_persist_and_add_provider_only_notifies() {
    use crossterm::event::KeyCode;
    use huginn_presentation::settings::{TAB_NAMES, THEME_NAMES};
    let calls = Arc::new(AtomicU64::new(0));
    let ui = Ui::new(
        &Session::new(".".into()),
        Arc::new(Contributions::default()),
        CancellationToken::new(),
        Arc::new(huginn_presentation::settings::UiConfig::defaults()),
        Arc::new({
            let calls = calls.clone();
            move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }),
    )
    .0;
    for configured in [false, true] {
        ui.lock().editor.insert("/");
        ui.refresh_completion(configured);
        assert!(ui
            .lock()
            .completion
            .items
            .iter()
            .any(|item| item.label == "/settings" && item.description == "TUI settings"));
        ui.lock().editor = Editor::default();
    }
    let before = ui.lock().conversation.clone();
    assert!(ui.dispatch_command("/settings   "));
    overlay_key(&ui, KeyCode::Down);
    {
        let state = ui.lock();
        let snapshot = state.settings().unwrap().snapshot();
        assert_eq!(snapshot.tabs, TAB_NAMES);
        assert_eq!(snapshot.items.len(), THEME_NAMES.len());
        assert_eq!(snapshot.selected, 1);
        assert!(snapshot.items[0].checked);
        assert!(!snapshot.items[1].checked);
        drop(state);
    }
    overlay_key(&ui, KeyCode::Char(' '));
    for expected_rows in [3, 1, 2, 1] {
        overlay_key(&ui, KeyCode::Tab);
        assert_eq!(
            ui.lock().settings().unwrap().snapshot().items.len(),
            expected_rows
        );
    }
    overlay_key(&ui, KeyCode::Enter); // (+) Add provider: never starts Providers
    assert!(ui.lock().overlays.snapshot().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        ui.lock().notifications.iter().last().unwrap().summary,
        "Add provider not implemented yet"
    );
    assert_eq!(ui.lock().conversation, before);
    assert!(ui.lock().editor.text.is_empty());
    ui.dispatch_command("/settings");
    assert_eq!(ui.lock().settings().unwrap().working.theme.name_index, 0);
}

#[tokio::test]
async fn late_settings_receipt_preserves_new_working_copy_and_focus() {
    for succeeds in [true, false] {
        let (entered, mut entering) = tokio::sync::mpsc::unbounded_channel();
        let (release, released) = std::sync::mpsc::channel();
        let released = std::sync::Mutex::new(released);
        let ui = Ui::new(
            &Session::new(".".into()),
            Arc::new(Contributions::default()),
            CancellationToken::new(),
            Arc::new(huginn_presentation::settings::UiConfig::defaults()),
            Arc::new(move |_| {
                entered.send(()).unwrap();
                released.lock().unwrap().recv().unwrap();
                if succeeds {
                    Ok(())
                } else {
                    anyhow::bail!("disk full")
                }
            }),
        )
        .0;
        let turn = CancellationToken::new();
        ui.begin(turn.clone());
        ui.dispatch_command("/settings");
        ui.lock().settings_mut().unwrap().working.theme.name_index = 1;
        assert!(ui.apply_settings());
        entering.recv().await.unwrap();
        overlay_key(&ui, crossterm::event::KeyCode::Esc);
        assert!(!turn.is_cancelled(), "Esc during a started write is local");
        ui.dispatch_command("/settings");
        ui.lock().settings_mut().unwrap().working.theme.name_index = 2;
        let before = ui.lock().settings().unwrap().projected_ui_config();
        let identity = ui.lock().overlays.snapshot();
        release.send(()).unwrap();
        receipt(&ui).await;
        assert_eq!(ui.lock().overlays.snapshot(), identity);
        assert_eq!(ui.lock().settings().unwrap().projected_ui_config(), before);
        assert!(ui.lock().settings().unwrap().last_error.is_none());
        overlay_key(&ui, crossterm::event::KeyCode::Esc);
        ui.dispatch_command("/settings");
        assert_eq!(
            ui.lock().settings().unwrap().working.theme.name_index,
            if succeeds { 1 } else { 0 }
        );
    }
}

#[tokio::test]
async fn settings_started_write_finishes_after_shutdown_without_reopening_or_notifying() {
    for succeeds in [true, false] {
        let (entered, mut entering) = tokio::sync::mpsc::unbounded_channel();
        let (release, released) = std::sync::mpsc::channel();
        let released = std::sync::Mutex::new(released);
        let calls = Arc::new(AtomicU64::new(0));
        let ui = Ui::new(
            &Session::new(".".into()),
            Arc::new(Contributions::default()),
            CancellationToken::new(),
            Arc::new(huginn_presentation::settings::UiConfig::defaults()),
            Arc::new({
                let calls = calls.clone();
                move |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    entered.send(()).unwrap();
                    released.lock().unwrap().recv().unwrap();
                    if succeeds {
                        Ok(())
                    } else {
                        anyhow::bail!("disk full")
                    }
                }
            }),
        )
        .0;
        ui.dispatch_command("/settings");
        ui.lock().settings_mut().unwrap().working.sound.bell = true;
        assert!(ui.apply_settings());
        entering.recv().await.unwrap();
        ui.shutdown();
        let notifications = ui.lock().notifications.len();
        assert!(!ui.apply_settings());
        release.send(()).unwrap();
        receipt(&ui).await;
        assert!(ui.lock().overlays.snapshot().is_none());
        assert_eq!(ui.lock().notifications.len(), notifications);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(ui.lock().conversation.is_empty());
    }
}

#[tokio::test]
async fn replaced_settings_writes_remain_serialized_in_submission_order() {
    let (entered, mut entering) = tokio::sync::mpsc::unbounded_channel();
    let (release, released) = std::sync::mpsc::channel();
    let released = std::sync::Mutex::new(released);
    let saved = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ui = Ui::new(
        &Session::new(".".into()),
        Arc::new(Contributions::default()),
        CancellationToken::new(),
        Arc::new(huginn_presentation::settings::UiConfig::defaults()),
        Arc::new({
            let saved = saved.clone();
            move |config| {
                entered.send(config.theme.name.clone()).unwrap();
                released.lock().unwrap().recv().unwrap();
                saved.lock().unwrap().push(config.theme.name.clone());
                Ok(())
            }
        }),
    )
    .0;
    ui.dispatch_command("/settings");
    ui.lock().settings_mut().unwrap().working.theme.name_index = 1;
    let first = ui
        .lock()
        .settings()
        .unwrap()
        .projected_ui_config()
        .theme
        .name;
    assert!(ui.apply_settings());
    assert_eq!(entering.recv().await.unwrap(), first);
    ui.dispatch_command("/settings");
    ui.lock().settings_mut().unwrap().working.theme.name_index = 2;
    let second = ui
        .lock()
        .settings()
        .unwrap()
        .projected_ui_config()
        .theme
        .name;
    assert!(ui.apply_settings());
    release.send(()).unwrap();
    assert_eq!(entering.recv().await.unwrap(), second);
    assert_eq!(
        saved.lock().unwrap().as_slice(),
        std::slice::from_ref(&first)
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while ui.lock().settings().is_some() {
            ui.refresh_contributions();
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(*saved.lock().unwrap(), [first, second.clone()]);
    ui.dispatch_command("/settings");
    assert_eq!(
        ui.lock()
            .settings()
            .unwrap()
            .projected_ui_config()
            .theme
            .name,
        second
    );
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
        tokio::spawn(async move {
            ui.request_decision(&request(), CancellationToken::new())
                .await
        })
    };
    pending(&ui).await;
    assert_eq!(ui.lock().modal().unwrap().number, 1);
    {
        let mut state = ui.lock();
        let modal = state.modal_mut().unwrap();
        modal.armed = true;
        modal.typed = modal.code.clone();
        drop(state);
    }
    ui.resolve_modal(true);
    assert!(first.await.unwrap().unwrap());
    let second = {
        let ui = ui.clone();
        tokio::spawn(async move {
            ui.request_decision(&request(), CancellationToken::new())
                .await
        })
    };
    pending(&ui).await;
    assert_eq!(ui.lock().modal().unwrap().number, 2);
    ui.resolve_modal(false);
    assert!(!second.await.unwrap().unwrap());
    // A new turn resets the numbering so its first approval is #1 again.
    ui.begin(CancellationToken::new());
    let third = {
        let ui = ui.clone();
        tokio::spawn(async move {
            ui.request_decision(&request(), CancellationToken::new())
                .await
        })
    };
    pending(&ui).await;
    assert_eq!(ui.lock().modal().unwrap().number, 1);
    ui.resolve_modal(false);
    assert!(!third.await.unwrap().unwrap());
}

#[test]
fn actionable_feedback_and_diagnostic_reset_on_new_operation() {
    let ui = ui();
    ui.emit(Event::ToolFinished {
        id: "error".into(),
        output: "failed".into(),
        is_error: true,
    });
    {
        let state = ui.lock();
        assert_eq!(state.status.feedback.as_deref(), Some("Tool error"));
        assert!(state.status.feedback_diagnostic);
        drop(state);
    }

    ui.begin(CancellationToken::new());
    let state = ui.lock();
    assert!(state.status.feedback.is_none());
    assert!(!state.status.feedback_diagnostic);
    drop(state);
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
    assert!(!ui.lock().status.diagnostic);
    ui.emit(Event::ToolFinished {
        id: "call-2".into(),
        output: "kaput".into(),
        is_error: true,
    });
    assert!(ui.lock().tail.contains("Tool error: kaput"));
    assert!(!ui.lock().tail.contains("failed"));
    assert!(ui.lock().status.diagnostic);
    assert_eq!(ui.lock().status.notice, "Tool error");
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
#[test]
fn connection_attachment_rejects_stale_sources_revisions_and_retired_close() {
    use huginn_core::providers::{
        ConnectionPhase, ConnectionSnapshot, SelectionMode, SelectionSnapshot,
    };
    use huginn_presentation::state::UiState;
    let mut state = UiState::default();
    let initial = ConnectionSnapshot {
        source: uuid::Uuid::new_v4(),
        revision: 1,
        phase: ConnectionPhase::Unconfigured,
        selection: None,
    };
    let old = state.attach_connection(initial.clone());
    let mut a = initial.clone();
    a.revision = 2;
    a.phase = ConnectionPhase::Selected;
    a.selection = Some(SelectionSnapshot {
        provider: "a".into(),
        connection: "a".into(),
        model: "alpha".into(),
        mode: SelectionMode::Unmanaged,
    });
    let mut b = a.clone();
    b.revision = 3;
    b.selection.as_mut().unwrap().model = "beta".into();
    assert!(state.update_connection(old, b.clone()));
    assert!(!state.update_connection(old, a.clone()));
    assert!(!state.update_connection(old, b.clone()));
    state.project(
        huginn_core::Event::ConnectionChanged { snapshot: a },
        "tool",
        "error",
    );
    assert_eq!(state.connection.as_ref(), Some(&b));
    assert!(state.conversation.is_empty() && state.tail.is_empty());
    let mut reconnected = initial;
    reconnected.source = uuid::Uuid::new_v4();
    let current = state.attach_connection(reconnected.clone());
    assert!(!state.update_connection(old, b.clone()));
    assert!(!state.close_connection(old));
    state.project(
        huginn_core::Event::ConnectionChanged { snapshot: b },
        "tool",
        "error",
    );
    assert_eq!(state.connection.as_ref(), Some(&reconnected));
    assert!(state.close_connection(current));
    assert_eq!(
        state.connection.as_ref().unwrap().phase,
        ConnectionPhase::Unavailable
    );
    assert!(state.connection.as_ref().unwrap().selection.is_none());
}

#[tokio::test]
async fn watch_attachment_initial_read_coalesces_concurrent_updates_without_gap() {
    use huginn_core::providers::{ConnectionPhase, ConnectionSnapshot};
    let initial = ConnectionSnapshot {
        source: uuid::Uuid::new_v4(),
        revision: 1,
        phase: ConnectionPhase::Unconfigured,
        selection: None,
    };
    let (sender, mut receiver) = tokio::sync::watch::channel(initial.clone());
    let mut newest = initial;
    newest.revision = 2;
    // A production watch clone may already have an unread update before open.
    sender.send_replace(newest.clone());
    let mut state = huginn_presentation::state::UiState::default();
    let attachment = state.attach_connection(receiver.borrow_and_update().clone());
    assert_eq!(state.connection.as_ref(), Some(&newest));
    newest.revision = 3;
    sender.send_replace(newest.clone());
    receiver.changed().await.unwrap();
    assert!(state.update_connection(attachment, receiver.borrow_and_update().clone()));
    assert_eq!(state.connection.as_ref(), Some(&newest));
    assert!(!receiver.has_changed().unwrap());
    drop(sender);
    assert!(receiver.changed().await.is_err());
    assert!(state.close_connection(attachment));
}

#[cfg(feature = "test-driver")]
mod lock_tests {
    use super::*;
    use async_trait::async_trait;
    use huginn_core::Result;
    use huginn_plugin_api::v1::{CommandResult, View};
    use huginn_presentation::{overlay::Origin, settings::UiConfig, state::test_support};
    use std::sync::Mutex;

    #[test]
    fn typed_status_snapshot_is_single_authority_for_notices_and_diagnostics() {
        let mut state = UiState::default();
        assert!(state.status.notice.is_empty());
        assert!(!state.status.diagnostic);

        state.project_ui_event(
            huginn_presentation::event::EventKind::Notification,
            "connected locally".into(),
        );
        assert_eq!(state.status.notice, "connected locally");
        assert!(!state.status.diagnostic);

        state.project_ui_event(
            huginn_presentation::event::EventKind::Diagnostic,
            "request failed".into(),
        );
        assert_eq!(state.status.notice, "request failed");
        assert!(state.status.diagnostic);

        state.project_ui_event(
            huginn_presentation::event::EventKind::Status,
            "Ready".into(),
        );
        assert_eq!(state.status.notice, "Ready");
        assert!(!state.status.diagnostic);
        assert!(
            state.connection.is_none(),
            "status must not imply connection availability"
        );
    }
    use huginn_core::kernel::{Manifest, Plugin, PluginContext, Supervisor};
    use huginn_plugin_api::v1::{contributions_id, Contribution, UiService};

    #[test]
    fn stale_result_does_not_allocate_an_operation_or_mutate_owner() {
        let ui = Ui::new(
            &Session::new(".".into()),
            Arc::new(Contributions::default()),
            CancellationToken::new(),
            Arc::new(UiConfig::defaults()),
            Arc::new(|_| Ok(())),
        )
        .0;
        let stale = ui.lock().overlays.ticket();
        ui.dispatch_command("/settings");
        let before = ui.lock().overlays.snapshot();
        let sequence = test_support::operation_sequence(&ui.lock());
        test_support::apply_result(
            &ui,
            CommandResult::Overlay(huginn_plugin_api::v1::OverlayKind::Help),
            Origin::Host,
            stale,
            None,
        );
        assert_eq!(
            test_support::operation_sequence(&ui.lock()),
            sequence,
            "stale results must not mutate operation allocation"
        );
        assert_eq!(ui.lock().overlays.snapshot(), before);
    }

    struct RendererPlugin(
        Arc<Contributions>,
        Arc<std::sync::Mutex<std::sync::Weak<Ui>>>,
    );
    struct Host(Arc<Contributions>);
    #[async_trait]
    impl Plugin for Host {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "render-host".into(),
                provides: vec![contributions_id()],
                requires: vec![],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            ctx.provide(contributions_id(), UiService(self.0.clone()))
        }
    }
    #[async_trait]
    impl Plugin for RendererPlugin {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "render-lock-test".into(),
                provides: vec![],
                requires: vec![contributions_id()],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            let slot = self.1.clone();
            self.0.register(
                ctx,
                "renderer".into(),
                Contribution::Renderer {
                    tool: "reentrant".into(),
                    handler: Arc::new(move |_| {
                        let ui = slot.lock().unwrap().upgrade().unwrap();
                        // Return observable content rather than panic: plugin API
                        // intentionally catches callback panics.
                        let outside = test_support::host_mutexes_available(&ui);
                        Ok(View::text(if outside { "unlocked" } else { "locked" }))
                    }),
                },
            )?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn renderer_runs_outside_every_host_mutex() {
        let registry = Arc::new(Contributions::default());
        let slot = Arc::new(Mutex::new(std::sync::Weak::new()));
        let mut supervisor = Supervisor::default();
        supervisor
            .register(Box::new(Host(registry.clone())))
            .unwrap();
        supervisor
            .register(Box::new(RendererPlugin(registry.clone(), slot.clone())))
            .unwrap();
        supervisor.start().await.unwrap();
        let ui = Ui::new(
            &Session::new(".".into()),
            registry,
            CancellationToken::new(),
            Arc::new(UiConfig::defaults()),
            Arc::new(|_| Ok(())),
        )
        .0;
        *slot.lock().unwrap() = Arc::downgrade(&ui);
        ui.emit(Event::ToolStarted {
            call: huginn_core::ToolCall {
                id: "call".into(),
                name: "reentrant".into(),
                arguments: Default::default(),
            },
        });
        ui.emit(Event::ToolFinished {
            id: "call".into(),
            output: "payload".into(),
            is_error: false,
        });
        assert!(
            ui.lock().tail.contains("unlocked"),
            "renderer must run outside every host mutex"
        );
    }
}
