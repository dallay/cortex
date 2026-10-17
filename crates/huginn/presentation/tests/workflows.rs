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
    let form = state.workflow.as_mut().unwrap();
    form.navigate(true);
    form.insert(SENTINEL);
    assert!(!format!("{:?}", form.snapshot()).contains(SENTINEL));
    assert!(form.snapshot().2[1].contains("********"));
    assert!(state.editor.text.is_empty());
    assert!(state.tail.is_empty());
    assert!(state.events.is_empty());
    assert!(!state.committed.join("\n").contains(SENTINEL));
    drop(state);
}
#[tokio::test]
async fn cancelling_unsubmitted_form_discards_secrets_without_executing_or_recovering_draft() {
    let h = setup(false).await;
    fill(&h.ui);
    h.ui.cancel_workflow();
    let state = h.ui.lock();
    assert!(state.workflow.is_none());
    assert!(state
        .committed
        .last()
        .is_some_and(|notice| notice.contains("discarded")));
    let notice_count = state.committed.len();
    drop(state);
    h.ui.cancel_workflow();
    assert_eq!(h.ui.lock().committed.len(), notice_count);
    assert_eq!(h.calls.load(Ordering::SeqCst), 0);
    h.ui.dispatch_command("/form");
    let state = h.ui.lock();
    let form = state.workflow.as_ref().unwrap();
    assert!(!form.snapshot().2[1].contains('*'));
    assert!(!state.committed.join("\n").contains(SENTINEL));
    drop(state);
}
#[tokio::test]
async fn late_result_after_cancel_does_not_replace_a_new_form() {
    let h = setup(false).await;
    fill(&h.ui);
    assert!(h.ui.submit_workflow());
    h.entered.notified().await;
    assert!(h.ui.lock().workflow.as_ref().unwrap().is_pending());
    h.ui.cancel_workflow();
    assert!(h.ui.lock().committed.last().is_some_and(|notice| {
        notice.contains("completion may be unknown") && notice.contains("reopen the command")
    }));
    h.ui.dispatch_command("/form");
    h.release.notify_one();
    tokio::task::yield_now().await;
    h.ui.refresh_contributions();
    assert!(!h.ui.lock().workflow.as_ref().unwrap().is_pending());
    assert!(!h.ui.lock().committed.iter().any(|s| s == "Finished"));
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
        if h.ui.lock().workflow.is_none() {
            break;
        }
    }
    let state = h.ui.lock();
    assert!(state.workflow.is_none());
    assert!(state.committed.join("\n").contains("UI action failed"));
    assert!(!state.committed.join("\n").contains(SENTINEL));
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
    assert!(h.ui.lock().workflow.is_none());
    h.release.notify_one();
    tokio::task::yield_now().await;
    h.ui.refresh_contributions();
    assert!(!h.ui.lock().committed.iter().any(|s| s == "Finished"));
    let h = setup(false).await;
    fill(&h.ui);
    assert!(h.ui.submit_workflow());
    h.entered.notified().await;
    h.ui.shutdown();
    assert!(h.ui.lock().workflow.is_none());
    assert!(h.ui.stop.is_cancelled());
}
