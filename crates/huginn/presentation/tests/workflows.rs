use async_trait::async_trait;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    CancellationToken, Result, Session,
};
use huginn_plugin_api::v1::*;
use huginn_presentation::state::Ui;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

const SENTINEL: &str = "p03-secret-ñ-never-in-public-output";
struct Host(Arc<Contributions>);
#[async_trait]
impl Plugin for Host {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "host".into(),
            provides: vec![contributions_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(contributions_id(), UiService(self.0.clone()))
    }
}
struct Author {
    calls: Arc<AtomicUsize>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    error: bool,
}
#[async_trait]
impl Plugin for Author {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "forms".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let calls = self.calls.clone();
        let entered = self.entered.clone();
        let release = self.release.clone();
        let error = self.error;
        ctx.resolve::<UiService>(&contributions_id())?
            .0
            .clone()
            .register(
                ctx,
                "forms.command".into(),
                Contribution::Command {
                    name: "/form".into(),
                    handler: Arc::new(move |_| {
                        let calls = calls.clone();
                        let entered = entered.clone();
                        let release = release.clone();
                        Ok(CommandResult::Workflow(Workflow::new(
                            "Secret form",
                            "Separate public fields",
                            vec![
                                Field {
                                    id: "endpoint".into(),
                                    label: "Endpoint".into(),
                                    kind: FieldKind::Text,
                                    initial: "https://example.invalid".into(),
                                },
                                Field {
                                    id: "key".into(),
                                    label: "API key".into(),
                                    kind: FieldKind::Secret,
                                    initial: String::new(),
                                },
                            ],
                            Arc::new(move |input, _| {
                                let calls = calls.clone();
                                let entered = entered.clone();
                                let release = release.clone();
                                Box::pin(async move {
                                    assert_eq!(input.secrets["key"].expose(), SENTINEL);
                                    assert!(!format!("{:?}", input).contains(SENTINEL));
                                    assert!(!input.public.contains_key("key"));
                                    calls.fetch_add(1, Ordering::SeqCst);
                                    entered.notify_one();
                                    release.notified().await;
                                    if error {
                                        return Err(huginn_core::AgentError::Composition(
                                            SENTINEL.into(),
                                        ));
                                    }
                                    Ok(CommandResult::Notice(View::text("Finished")))
                                })
                            }),
                        )?))
                    }),
                },
            )?;
        Ok(())
    }
}
struct Harness {
    supervisor: Supervisor,
    ui: Arc<Ui>,
    calls: Arc<AtomicUsize>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
async fn setup(error: bool) -> Harness {
    let registry = Arc::new(Contributions::default());
    let mut supervisor = Supervisor::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    supervisor
        .register(Box::new(Host(registry.clone())))
        .unwrap();
    supervisor
        .register(Box::new(Author {
            calls: calls.clone(),
            entered: entered.clone(),
            release: release.clone(),
            error,
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    let ui = Ui::new(
        &Session::new(".".into()),
        registry,
        CancellationToken::new(),
        Arc::new(huginn_presentation::settings::UiConfig::defaults()),
        Arc::new(|_: &huginn_presentation::settings::UiConfig| Ok(())),
    )
    .0;
    Harness {
        supervisor,
        ui,
        calls,
        entered,
        release,
    }
}
fn fill(ui: &Ui) {
    assert!(ui.dispatch_command("/form"));
    let mut state = ui.lock();
    let form = state.workflow_mut().unwrap();
    form.navigate(true);
    form.insert(SENTINEL);
    assert!(!format!("{:?}", form.snapshot()).contains(SENTINEL));
    assert!(form.snapshot().2[1].contains("********"));
    assert!(state.editor.text.is_empty());
    assert!(state.tail.is_empty());
    assert!(state.events.is_empty());
    assert!(!state.conversation.join("\n").contains(SENTINEL));
    drop(state);
}
#[tokio::test]
async fn cancelling_unsubmitted_form_discards_secrets_without_executing_or_recovering_draft() {
    let h = setup(false).await;
    fill(&h.ui);
    h.ui.cancel_workflow();
    let state = h.ui.lock();
    assert!(state.workflow().is_none());
    assert!(state
        .notifications
        .iter()
        .any(|event| event.summary.contains("discarded")));
    let notice_count = state.notifications.len();
    drop(state);
    h.ui.cancel_workflow();
    assert_eq!(h.ui.lock().notifications.len(), notice_count);
    assert_eq!(h.calls.load(Ordering::SeqCst), 0);
    h.ui.dispatch_command("/form");
    let state = h.ui.lock();
    let form = state.workflow().unwrap();
    assert!(!form.snapshot().2[1].contains('*'));
    assert!(!state.conversation.join("\n").contains(SENTINEL));
    drop(state);
}
#[tokio::test]
async fn late_result_after_cancel_does_not_replace_a_new_form() {
    let h = setup(false).await;
    fill(&h.ui);
    assert!(h.ui.submit_workflow());
    h.entered.notified().await;
    assert!(h.ui.lock().workflow().unwrap().is_pending());
    h.ui.cancel_workflow();
    assert!(h.ui.lock().notifications.iter().any(|event| {
        event.summary.contains("completion may be unknown")
            && event.summary.contains("reopen the command")
    }));
    h.ui.dispatch_command("/form");
    h.release.notify_one();
    tokio::task::yield_now().await;
    h.ui.refresh_contributions();
    assert!(!h.ui.lock().workflow().unwrap().is_pending());
    assert!(!h.ui.lock().conversation.iter().any(|s| s == "Finished"));
}
#[tokio::test]
async fn action_error_is_generic_and_does_not_render_secret_source_errors() {
    let h = setup(true).await;
    fill(&h.ui);
    assert!(h.ui.submit_workflow());
    h.entered.notified().await;
    h.release.notify_one();
    for _ in 0..20 {
        tokio::task::yield_now().await;
        h.ui.refresh_contributions();
        if h.ui
            .lock()
            .workflow()
            .is_some_and(|form| !form.is_pending())
        {
            break;
        }
    }
    let state = h.ui.lock();
    let form = state.workflow().expect("a failed form stays editable");
    let (_, _, _, _, _) = form.snapshot();
    let error = form.error_message().expect("the failure stays visible");
    assert!(error.contains("Action failed"));
    assert!(!error.contains(SENTINEL));
    assert!(!state.conversation.join("\n").contains(SENTINEL));
    assert!(state.events.is_empty());
    drop(state);
}
#[tokio::test]
async fn unloading_or_closing_cleans_pending_tasks_and_forms() {
    let mut h = setup(false).await;
    fill(&h.ui);
    assert!(h.ui.submit_workflow());
    h.entered.notified().await;
    h.supervisor.remove("forms").await.unwrap();
    h.ui.refresh_contributions();
    assert!(h.ui.lock().workflow().is_none());
    h.release.notify_one();
    tokio::task::yield_now().await;
    h.ui.refresh_contributions();
    assert!(!h.ui.lock().conversation.iter().any(|s| s == "Finished"));
    let h = setup(false).await;
    fill(&h.ui);
    assert!(h.ui.submit_workflow());
    h.entered.notified().await;
    h.ui.shutdown();
    assert!(h.ui.lock().workflow().is_none());
    assert!(h.ui.stop.is_cancelled());
}

struct ResultCommands;
#[tokio::test]
async fn authorized_settings_replaces_form_through_shared_authority() {
    let h = setup(false).await;
    fill(&h.ui);
    let before = h.ui.lock().conversation.clone();
    h.ui.dispatch_command("/settings");
    let state = h.ui.lock();
    assert!(
        state.workflow().is_none(),
        "Settings must retire the old form"
    );
    assert_eq!(
        state.overlays.snapshot().unwrap().kind,
        huginn_presentation::overlay::OverlayKind::Settings
    );
    assert_eq!(state.conversation, before);
}

#[tokio::test]
async fn declarative_help_opens_real_host_overlay_without_notification() {
    let mut h = setup(false).await;
    h.supervisor.register(Box::new(ResultCommands)).unwrap();
    h.supervisor.start().await.unwrap();
    let before = h.ui.lock().notifications.len();
    h.ui.dispatch_command("/result help");
    let state = h.ui.lock();
    assert!(
        state.overlays.snapshot().is_some(),
        "Help must actually open"
    );
    assert_eq!(
        state.overlays.snapshot().unwrap().kind,
        huginn_presentation::overlay::OverlayKind::Help
    );
    assert_eq!(state.notifications.len(), before);
}

#[async_trait]
impl Plugin for ResultCommands {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "results".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let registry = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        registry.register(
            ctx,
            "results.command".into(),
            Contribution::Command {
                name: "/result".into(),
                handler: Arc::new(|argument| {
                    if argument == "boundary" {
                        return Ok(CommandResult::Workflow(Workflow::new(
                            "Boundary",
                            "",
                            vec![
                                Field {
                                    id: "public".into(),
                                    label: "Public".into(),
                                    kind: FieldKind::Text,
                                    initial: "captured".into(),
                                },
                                Field {
                                    id: "secret".into(),
                                    label: "Secret".into(),
                                    kind: FieldKind::Secret,
                                    initial: String::new(),
                                },
                                Field {
                                    id: "choice".into(),
                                    label: "Choice".into(),
                                    kind: FieldKind::Choice(vec!["a".into(), "b".into()]),
                                    initial: "1".into(),
                                },
                            ],
                            Arc::new(|input, _| {
                                Box::pin(async move {
                                    assert_eq!(input.public.len(), 2);
                                    assert_eq!(input.secrets.len(), 1);
                                    assert_eq!(input.public["public"], "captured");
                                    assert_eq!(input.public["choice"], "1");
                                    assert_eq!(input.secrets["secret"].expose(), "é".repeat(2048));
                                    Ok(CommandResult::Notice(View::text("boundary receipt")))
                                })
                            }),
                        )?));
                    }
                    if let Some(surface) = argument.strip_prefix("flow-") {
                        let kind = match surface {
                            "provider" => OverlayKind::Provider,
                            "login" => OverlayKind::Login,
                            "model" => OverlayKind::ModelPicker,
                            "confirm" => OverlayKind::Confirm,
                            _ => unreachable!(),
                        };
                        return Ok(CommandResult::Workflow(
                            Workflow::new(
                                "Exact payload",
                                "original body",
                                vec![
                                    Field {
                                        id: "endpoint".into(),
                                        label: "Endpoint".into(),
                                        kind: FieldKind::Text,
                                        initial: "original".into(),
                                    },
                                    Field {
                                        id: "secret".into(),
                                        label: "Secret".into(),
                                        kind: FieldKind::Secret,
                                        initial: String::new(),
                                    },
                                ],
                                Arc::new(|input, _| {
                                    Box::pin(async move {
                                        assert_eq!(input.public["endpoint"], "original");
                                        assert_eq!(input.secrets["secret"].expose(), SENTINEL);
                                        Ok(CommandResult::Workflow(Workflow::new(
                                            "Continuation",
                                            "retained context",
                                            vec![Field {
                                                id: "context".into(),
                                                label: "Context".into(),
                                                kind: FieldKind::Text,
                                                initial: "exact".into(),
                                            }],
                                            Arc::new(|input, _| {
                                                Box::pin(async move {
                                                    assert_eq!(input.public["context"], "exact");
                                                    Ok(CommandResult::Notice(View::text(
                                                        "exact receipt",
                                                    )))
                                                })
                                            }),
                                        )?))
                                    })
                                }),
                            )?
                            .with_overlay_kind(kind)?,
                        ));
                    }
                    let result = match argument {
                        "help" | "async-help" => CommandResult::Overlay(OverlayKind::Help),
                        "settings" => CommandResult::Overlay(OverlayKind::Settings),
                        "provider" => CommandResult::Overlay(OverlayKind::Provider),
                        "login" => CommandResult::Overlay(OverlayKind::Login),
                        "model" => CommandResult::Overlay(OverlayKind::ModelPicker),
                        "confirm" => CommandResult::Overlay(OverlayKind::Confirm),
                        _ => CommandResult::Notice(View::text("safe\u{202e}\x1b")),
                    };
                    if argument.starts_with("async-") {
                        Ok(CommandResult::Workflow(Workflow::new(
                            "Action",
                            "",
                            vec![],
                            Arc::new(move |_, _| {
                                let result = result.clone();
                                Box::pin(async move { Ok(result) })
                            }),
                        )?))
                    } else {
                        Ok(result)
                    }
                }),
            },
        )?;
        Ok(())
    }
}

#[tokio::test]
async fn submission_moves_bounded_utf8_secrets_and_freezes_captured_input_once() {
    let mut h = setup(false).await;
    h.supervisor.register(Box::new(ResultCommands)).unwrap();
    h.supervisor.start().await.unwrap();
    h.ui.dispatch_command("/result boundary");
    assert!(!h.ui.submit_workflow(), "empty secret cannot execute");
    {
        let mut state = h.ui.lock();
        let form = state.workflow_mut().unwrap();
        form.navigate(true);
        form.insert("é");
        let mask = form.snapshot().2[1].clone();
        form.insert(&"é".repeat(2048)); // last character exceeds 4096 bytes
        assert_eq!(form.snapshot().2[1], mask, "mask is length independent");
        form.backspace();
        form.insert("é\n\u{202e}"); // UTF-8 deletion and sanitization
        drop(state);
    }
    assert!(h.ui.submit_workflow());
    assert!(!h.ui.submit_workflow(), "pending submission executes once");
    {
        let mut state = h.ui.lock();
        let form = state.workflow_mut().unwrap();
        form.insert("not captured");
        form.backspace();
        form.navigate(true);
        form.choice(false);
        drop(state);
    }
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while h.ui.lock().workflow().is_some() {
            tokio::task::yield_now().await;
            h.ui.refresh_contributions();
        }
    })
    .await
    .unwrap();
    assert_eq!(
        h.ui.lock().notifications.iter().last().unwrap().summary,
        "boundary receipt"
    );
    assert!(h.ui.lock().editor.text.is_empty());
    assert!(h.ui.lock().conversation.is_empty());
}

#[tokio::test]
async fn result_rejection_preserves_current_payload_focus_and_conversation() {
    let mut h = setup(false).await;
    h.supervisor.register(Box::new(ResultCommands)).unwrap();
    h.supervisor.start().await.unwrap();
    fill(&h.ui);
    let (before, focus, conversation) = {
        let state = h.ui.lock();
        (
            state.workflow().unwrap().snapshot(),
            state.focus(),
            state.conversation.clone(),
        )
    };
    for argument in ["provider", "login", "model", "confirm"] {
        assert!(h.ui.dispatch_command(&format!("/result {argument}")));
        let state = h.ui.lock();
        assert_eq!(state.workflow().unwrap().snapshot(), before);
        assert_eq!(state.focus(), focus);
        assert_eq!(state.conversation, conversation);
        assert_eq!(
            state.notifications.iter().last().unwrap().kind,
            huginn_presentation::event::EventKind::Notification
        );
        assert!(state
            .notifications
            .iter()
            .last()
            .unwrap()
            .summary
            .contains("host-provided payload"));
        drop(state);
    }
    h.ui.dispatch_command("/result notice");
    {
        let state = h.ui.lock();
        assert_eq!(state.notifications.iter().last().unwrap().summary, "safe");
        assert_eq!(
            state.notifications.iter().last().unwrap().kind,
            huginn_presentation::event::EventKind::Notification
        );
        assert_eq!(state.workflow().unwrap().snapshot(), before);
        assert_eq!(state.focus(), focus);
        assert_eq!(state.conversation, conversation);
        drop(state);
    }
    // The original callback still receives the exact secret, not merely a mask.
    assert!(h.ui.submit_workflow());
    h.entered.notified().await;
    assert_eq!(h.calls.load(Ordering::SeqCst), 1);
    h.release.notify_one();
}

#[tokio::test]
async fn async_results_report_notice_or_open_real_help() {
    let mut h = setup(false).await;
    h.supervisor.register(Box::new(ResultCommands)).unwrap();
    h.supervisor.start().await.unwrap();
    for (argument, expected) in [("async-notice", "safe"), ("async-help", "safe")] {
        let before = h.ui.lock().conversation.clone();
        h.ui.dispatch_command(&format!("/result {argument}"));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                h.ui.refresh_contributions();
                if h.ui.lock().workflow().is_none() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let state = h.ui.lock();
        if argument == "async-help" {
            assert_eq!(
                state.overlays.snapshot().unwrap().kind,
                huginn_presentation::overlay::OverlayKind::Help
            );
        }
        assert_eq!(state.notifications.iter().last().unwrap().summary, expected);
        assert_eq!(
            state.notifications.iter().last().unwrap().kind,
            huginn_presentation::event::EventKind::Notification
        );
        assert_eq!(state.conversation, before);
        assert!(state.plugin_dialog().is_none());
        assert!(state.settings().is_none());
        assert!(state.modal().is_none());
        drop(state);
    }
}

#[tokio::test]
async fn every_typed_surface_keeps_payload_secret_callback_and_continuation_operation() {
    use huginn_presentation::overlay::OverlayKind as HostKind;
    let mut h = setup(false).await;
    h.supervisor.register(Box::new(ResultCommands)).unwrap();
    h.supervisor.start().await.unwrap();
    for (argument, kind) in [
        ("provider", HostKind::Provider),
        ("login", HostKind::Login),
        ("model", HostKind::ModelPicker),
        ("confirm", HostKind::Confirm),
    ] {
        h.ui.dispatch_command(&format!("/result flow-{argument}"));
        let (before, ticket) = {
            let mut state = h.ui.lock();
            assert_eq!(state.overlays.snapshot().unwrap().kind, kind);
            let form = state.workflow_mut().unwrap();
            assert_eq!(form.snapshot().1, "original body");
            form.navigate(true);
            form.insert(SENTINEL);
            assert!(!format!("{:?}", form.snapshot()).contains(SENTINEL));
            (state.overlays.snapshot().unwrap(), state.overlays.ticket())
        };
        // Unknown/non-text events still belong to the overlay, never completion
        // or composer. Release events must not submit the captured form.
        for event in [
            crossterm::event::Event::Resize(20, 8),
            crossterm::event::Event::FocusLost,
            crossterm::event::Event::FocusGained,
            crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::F(12),
                crossterm::event::KeyModifiers::NONE,
            )),
            crossterm::event::Event::Key(crossterm::event::KeyEvent::new_with_kind(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
                crossterm::event::KeyEventKind::Release,
            )),
        ] {
            h.ui.overlay_input(event, ticket, before.focus);
            assert_eq!(h.ui.lock().overlays.snapshot(), Some(before));
            assert!(h.ui.lock().editor.text.is_empty());
            assert!(!h.ui.lock().workflow().unwrap().is_pending());
        }
        assert!(h.ui.submit_workflow());
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                h.ui.refresh_contributions();
                if h.ui
                    .lock()
                    .workflow()
                    .is_some_and(|form| form.snapshot().0 == "Continuation")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let next = h.ui.lock().overlays.snapshot().unwrap();
        assert_eq!(next.operation, before.operation);
        assert_eq!(
            next.kind,
            HostKind::Workflow,
            "continuation owns its metadata, including None"
        );
        assert_ne!(next.id, before.id);
        h.ui.overlay_input(
            crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            )),
            ticket,
            before.focus,
        );
        assert_eq!(h.ui.lock().overlays.snapshot(), Some(next));
        h.ui.submit_workflow();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while h.ui.lock().workflow().is_some() {
                tokio::task::yield_now().await;
                h.ui.refresh_contributions();
            }
        })
        .await
        .unwrap();
        assert_eq!(
            h.ui.lock().notifications.iter().last().unwrap().summary,
            "exact receipt"
        );
        assert!(h.ui.lock().conversation.is_empty());
    }
}

struct Reentrant {
    ui: Arc<std::sync::Mutex<std::sync::Weak<Ui>>>,
    entered: Arc<tokio::sync::Notify>,
    dropped: Arc<tokio::sync::Notify>,
}
struct ReenterOnDrop(
    Arc<std::sync::Mutex<std::sync::Weak<Ui>>>,
    Arc<tokio::sync::Notify>,
);
impl Drop for ReenterOnDrop {
    fn drop(&mut self) {
        let ui = self.0.lock().unwrap().upgrade().unwrap();
        assert!(
            ui.state.try_lock().is_ok(),
            "pending future destructor must not hold UiState"
        );
        ui.lock().editor.insert("drop reentered");
        self.1.notify_one();
    }
}
#[async_trait]
impl Plugin for Reentrant {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "reentrant".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let registry = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        let slot = self.ui.clone();
        registry.register(
            ctx,
            "reentrant.dialog".into(),
            Contribution::Dialog(Dialog {
                title: "Exact dialog".into(),
                body: "Original payload".into(),
                choices: vec!["original choice".into()],
                on_select: Arc::new(move |index| {
                    assert_eq!(index, 0);
                    let ui = slot.lock().unwrap().upgrade().unwrap();
                    assert!(ui.state.try_lock().is_ok());
                    ui.dispatch_command("/settings");
                    Ok(View::text("stale dialog callback"))
                }),
            }),
        )?;
        registry.register(
            ctx,
            "reentrant.dialog-command".into(),
            Contribution::Command {
                name: "/reentrant-dialog".into(),
                handler: Arc::new(|_| Ok(CommandResult::Dialog("reentrant.dialog".into()))),
            },
        )?;
        let slot = self.ui.clone();
        registry.register(
            ctx,
            "reentrant.command".into(),
            Contribution::Command {
                name: "/reenter".into(),
                handler: Arc::new(move |_| {
                    let ui = slot.lock().unwrap().upgrade().unwrap();
                    ui.dispatch_command("/settings");
                    Ok(CommandResult::Overlay(OverlayKind::Help))
                }),
            },
        )?;
        let slot = self.ui.clone();
        let entered = self.entered.clone();
        let dropped = self.dropped.clone();
        registry.register(
            ctx,
            "reentrant.pending".into(),
            Contribution::Command {
                name: "/pending-reenter".into(),
                handler: Arc::new(move |_| {
                    let slot = slot.clone();
                    let entered = entered.clone();
                    let dropped = dropped.clone();
                    Ok(CommandResult::Workflow(Workflow::new(
                        "Pending",
                        "",
                        vec![],
                        Arc::new(move |_, _| {
                            let slot = slot.clone();
                            let entered = entered.clone();
                            let dropped = dropped.clone();
                            Box::pin(async move {
                                let _guard = ReenterOnDrop(slot, dropped);
                                entered.notify_one();
                                std::future::pending().await
                            })
                        }),
                    )?))
                }),
            },
        )?;
        Ok(())
    }
}

#[tokio::test]
async fn reentrant_callbacks_cannot_commit_stale_tickets_and_pending_drop_is_outside_lock() {
    let mut h = setup(false).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(tokio::sync::Notify::new());
    h.supervisor
        .register(Box::new(Reentrant {
            ui: Arc::new(std::sync::Mutex::new(Arc::downgrade(&h.ui))),
            entered: entered.clone(),
            dropped: dropped.clone(),
        }))
        .unwrap();
    h.supervisor.start().await.unwrap();
    h.ui.dispatch_command("/reenter");
    assert!(
        h.ui.lock().settings().is_some(),
        "stale outer Help cannot replace reentrant Settings"
    );
    let (settings, ticket) = {
        let state = h.ui.lock();
        (state.overlays.snapshot().unwrap(), state.overlays.ticket())
    };
    h.ui.dispatch_command("/reentrant-dialog");
    assert!(h.ui.lock().settings().is_none());
    assert_eq!(
        h.ui.lock().plugin_dialog().unwrap().handle.body(),
        "Original payload"
    );
    h.ui.overlay_input(
        crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        )),
        ticket,
        settings.focus,
    );
    assert_eq!(h.ui.lock().plugin_dialog().unwrap().selected, 0);
    let notifications = h.ui.lock().notifications.len();
    h.ui.choose_plugin_dialog();
    assert!(
        h.ui.lock().settings().is_some(),
        "dialog callback reentered the host"
    );
    assert_eq!(
        h.ui.lock().notifications.len(),
        notifications,
        "stale callback result must not mutate replacement"
    );
    h.ui.dispatch_command("/pending-reenter");
    entered.notified().await;
    h.ui.dispatch_command("/settings");
    tokio::time::timeout(std::time::Duration::from_secs(2), dropped.notified())
        .await
        .unwrap();
    assert_eq!(h.ui.lock().editor.text, "drop reentered");
    assert!(h.ui.lock().settings().is_some());
}

#[tokio::test]
async fn revoked_origin_and_reregistered_name_cannot_mutate_replacement() {
    let mut h = setup(false).await;
    let old_command = h.ui.contributions.command("/form").unwrap();
    fill(&h.ui);
    h.ui.submit_workflow();
    h.entered.notified().await;
    h.supervisor.remove("forms").await.unwrap();
    let replacement_calls = Arc::new(AtomicUsize::new(0));
    h.supervisor
        .register(Box::new(Author {
            calls: replacement_calls.clone(),
            entered: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
            error: false,
        }))
        .unwrap();
    h.supervisor.start().await.unwrap();
    h.ui.refresh_contributions();
    assert!(h.ui.lock().workflow().is_none());
    h.ui.dispatch_command("/form");
    let before = h.ui.lock().overlays.snapshot();
    let notifications = h.ui.lock().notifications.len();
    h.ui.dispatch_handle(&old_command, "");
    h.release.notify_one();
    tokio::task::yield_now().await;
    h.ui.refresh_contributions();
    assert_eq!(h.ui.lock().overlays.snapshot(), before);
    assert_eq!(h.ui.lock().notifications.len(), notifications);
    assert!(!h.ui.lock().workflow().unwrap().is_pending());
    assert_eq!(replacement_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn approval_preempts_secret_workflow_and_rejects_all_ordinary_requests() {
    use huginn_core::ApprovalPrompt;
    let mut h = setup(false).await;
    h.supervisor.register(Box::new(ResultCommands)).unwrap();
    h.supervisor.start().await.unwrap();
    fill(&h.ui);
    let old = h.ui.lock().overlays.snapshot().unwrap();
    let old_ticket = h.ui.lock().overlays.ticket();
    let task = {
        let ui = h.ui.clone();
        tokio::spawn(async move {
            ui.request_decision(
                &huginn_core::ApprovalRequest {
                    id: "one".into(),
                    action: "write".into(),
                    preview: "exact preview".into(),
                },
                CancellationToken::new(),
            )
            .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while h.ui.lock().modal().is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let approval = h.ui.lock().overlays.snapshot();
    for command in [
        "/settings",
        "/result help",
        "/result settings",
        "/result flow-confirm",
        "/form",
    ] {
        h.ui.dispatch_command(command);
        assert_eq!(h.ui.lock().overlays.snapshot(), approval);
    }
    h.ui.overlay_input(
        crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        )),
        old_ticket,
        old.focus,
    );
    assert_eq!(h.ui.lock().overlays.snapshot(), approval);
    assert!(!task.is_finished());
    h.ui.resolve_modal(false);
    assert!(!task.await.unwrap().unwrap());
    assert!(h.ui.lock().overlays.snapshot().is_none());
    assert!(h.ui.lock().conversation.is_empty());
    assert_eq!(h.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn failed_form_keeps_public_values_and_focus_but_clears_submitted_secret() {
    let h = setup(true).await;
    fill(&h.ui);
    {
        let mut state = h.ui.lock();
        let form = state.workflow_mut().unwrap();
        form.navigate(false);
        form.insert("https://example.invalid/v1");
        form.navigate(true);
        drop(state);
    }
    assert!(h.ui.submit_workflow());
    h.entered.notified().await;
    h.release.notify_one();
    for _ in 0..30 {
        tokio::task::yield_now().await;
        h.ui.refresh_contributions();
    }
    let state = h.ui.lock();
    let form = state
        .workflow()
        .expect("a failed form must remain editable");
    assert!(!form.is_pending());
    let (_, _, fields, selected, _) = form.snapshot();
    assert_eq!(selected, 1);
    assert!(fields[0].contains("https://example.invalid/v1"));
    assert!(!fields[1].contains('*'));
    let error = form.error_message().expect("the failure stays visible");
    assert!(!error.contains(SENTINEL));
    drop(state);
}
