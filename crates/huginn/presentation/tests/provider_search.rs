//! Host form behavior without a terminal, network, or credential store.
use async_trait::async_trait;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    CancellationToken, Result, Session,
};
use huginn_plugin_api::v1::*;
use huginn_presentation::state::Ui;
use std::sync::{Arc, Mutex};

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

struct Search(Workflow);
#[async_trait]
impl Plugin for Search {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "search".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let workflow = self.0.clone();
        ctx.resolve::<UiService>(&contributions_id())?.0.register(
            ctx,
            "search.command".into(),
            Contribution::Command {
                name: "/search".into(),
                handler: Arc::new(move |_| Ok(CommandResult::Workflow(workflow.clone()))),
            },
        )?;
        Ok(())
    }
}

fn field(id: &str, kind: FieldKind) -> Field {
    Field {
        id: id.into(),
        label: id.into(),
        kind,
        initial: String::new(),
    }
}
fn providers() -> Field {
    field(
        "provider",
        FieldKind::Choice(vec!["Alpha".into(), "Beta".into(), "Alpine".into()]),
    )
}
async fn setup(fields: Vec<Field>) -> (Supervisor, Arc<Ui>, Arc<Mutex<Option<FormInput>>>) {
    let captured = Arc::new(Mutex::new(None));
    let receipt = captured.clone();
    let workflow = Workflow::new(
        "Search",
        "",
        fields,
        Arc::new(move |input, _| {
            *receipt.lock().unwrap() = Some(input);
            Box::pin(async { Ok(CommandResult::Dismiss) })
        }),
    )
    .unwrap()
    .searchable();
    let contributions = Arc::new(Contributions::default());
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(Host(contributions.clone())))
        .unwrap();
    supervisor.register(Box::new(Search(workflow))).unwrap();
    supervisor.start().await.unwrap();
    let ui = Ui::new(
        &Session::new(".".into()),
        contributions,
        CancellationToken::new(),
        Arc::new(huginn_presentation::settings::UiConfig::defaults()),
        Arc::new(|_| Ok(())),
    )
    .0;
    assert!(ui.dispatch_command("/search"));
    (supervisor, ui, captured)
}
fn key(ui: &Ui, code: KeyCode) {
    let (ticket, focus) = {
        let state = ui.lock();
        (state.overlays.ticket(), state.overlays.focus())
    };
    ui.overlay_input(
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        ticket,
        focus,
    );
}
async fn submit(ui: &Ui) {
    assert!(ui.submit_workflow());
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while ui.lock().workflow().is_some() {
            tokio::task::yield_now().await;
            ui.refresh_contributions();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn filtered_arrows_wrap_and_submit_the_original_index() {
    let (_supervisor, ui, captured) = setup(vec![providers()]).await;
    let notification_count = ui.lock().notifications.len();
    ui.lock().workflow_mut().unwrap().insert("aL");
    key(&ui, KeyCode::Up); // wraps from Alpha to Alpine, skipping Beta
    assert_eq!(
        ui.lock().workflow().unwrap().search_rows(),
        vec![("Alpha".into(), false), ("Alpine".into(), true)]
    );
    key(&ui, KeyCode::Down);
    assert_eq!(
        ui.lock().workflow().unwrap().search_rows()[0],
        ("Alpha".into(), true)
    );
    key(&ui, KeyCode::Up);
    submit(&ui).await;
    assert_eq!(
        captured.lock().unwrap().as_ref().unwrap().public["provider"],
        "2"
    );
    assert!(ui.lock().conversation.is_empty());
    assert_eq!(ui.lock().notifications.len(), notification_count);
}

#[tokio::test]
async fn search_query_budget_counts_utf8_bytes_and_backspace_removes_a_character() {
    let (_supervisor, ui, _) = setup(vec![providers()]).await;
    let mut state = ui.lock();
    let form = state.workflow_mut().unwrap();
    form.insert(&"é".repeat(2048));
    form.insert("éx\n\t\x1b");
    assert_eq!(form.search_query(), "é".repeat(2048));
    assert_eq!(form.search_query().len(), 4096);
    form.backspace();
    form.insert("界"); // three bytes cannot fit in the two freed bytes
    assert_eq!(form.search_query(), "é".repeat(2047));
    form.insert("ab");
    assert_eq!(form.search_query(), format!("{}ab", "é".repeat(2047)));
    drop(state);
    assert!(!ui.submit_workflow());
}

#[tokio::test]
async fn only_the_first_choice_filters_and_other_fields_keep_their_input_semantics() {
    let (_supervisor, ui, captured) = setup(vec![
        field("name", FieldKind::Text),
        providers(),
        field(
            "storage",
            FieldKind::Choice(vec!["Secure".into(), "Ephemeral".into()]),
        ),
        field("key", FieldKind::Secret),
    ])
    .await;
    ui.lock().workflow_mut().unwrap().insert("draft");
    key(&ui, KeyCode::Tab);
    ui.lock().workflow_mut().unwrap().insert("bEtA\n\t");
    key(&ui, KeyCode::Tab);
    key(&ui, KeyCode::Right);
    key(&ui, KeyCode::Tab);
    ui.lock()
        .workflow_mut()
        .unwrap()
        .insert("search-secret-sentinel");
    {
        let state = ui.lock();
        let form = state.workflow().unwrap();
        assert_eq!(form.search_query(), "bEtA");
        assert!(!form.searching());
        assert!(!format!("{:?}", form.snapshot()).contains("search-secret-sentinel"));
    }
    submit(&ui).await;
    let receipt = captured.lock().unwrap();
    let input = receipt.as_ref().unwrap();
    assert_eq!(input.public["name"], "draft");
    assert_eq!(input.public["provider"], "1");
    assert_eq!(input.public["storage"], "1");
    assert!(!input.public.contains_key("key"));
    assert_eq!(input.secrets["key"].expose(), "search-secret-sentinel");
}

#[tokio::test]
async fn empty_search_cannot_submit_from_another_field_and_can_recover_by_deleting_query() {
    let (_supervisor, ui, captured) =
        setup(vec![providers(), field("name", FieldKind::Text)]).await;
    ui.lock().workflow_mut().unwrap().insert("z");
    for code in [KeyCode::Up, KeyCode::Down, KeyCode::Tab] {
        key(&ui, code);
    }
    assert!(!ui.submit_workflow());
    assert!(!ui.lock().workflow().unwrap().is_pending());
    assert!(captured.lock().unwrap().is_none());
    key(&ui, KeyCode::BackTab);
    key(&ui, KeyCode::Backspace);
    assert_eq!(ui.lock().workflow().unwrap().search_rows().len(), 3);
    submit(&ui).await;
    assert_eq!(
        captured.lock().unwrap().as_ref().unwrap().public["provider"],
        "0"
    );
}

#[tokio::test]
async fn searchable_metadata_without_a_choice_preserves_text_entry() {
    let (_supervisor, ui, captured) = setup(vec![field("name", FieldKind::Text)]).await;
    ui.lock().workflow_mut().unwrap().insert("plain text");
    key(&ui, KeyCode::Backspace);
    assert!(!ui.lock().workflow().unwrap().searching());
    assert!(ui.lock().workflow().unwrap().search_rows().is_empty());
    submit(&ui).await;
    assert_eq!(
        captured.lock().unwrap().as_ref().unwrap().public["name"],
        "plain tex"
    );
}
