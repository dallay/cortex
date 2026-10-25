//! Contract doubles only; no remote provider or OS credential-store validation.
use async_trait::async_trait;
use huginn_core::{
    connections::*,
    kernel::{Manifest, Plugin, PluginContext, ServiceId, Supervisor},
    providers::*,
    CancellationToken, Result, Session,
};
use huginn_plugin_api::v1::*;
use huginn_presentation::{providers::ProvidersUiPlugin, state::Ui};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

const SENTINEL: &str = "p03-ui-secret-sentinel-never-public";
#[derive(Default)]
struct Manager {
    records: Mutex<BTreeMap<uuid::Uuid, ConnectionRecord>>,
    selected: Mutex<Option<(uuid::Uuid, String)>>,
    fail: AtomicBool,
    uncertain: AtomicBool,
    recovery_fail: AtomicBool,
}
#[async_trait]
impl ConnectionManagement for Manager {
    fn subscribe(
        &self,
    ) -> tokio::sync::watch::Receiver<huginn_core::providers::ConnectionSnapshot> {
        let (_, receiver) =
            tokio::sync::watch::channel(huginn_core::providers::ConnectionSnapshot {
                source: uuid::Uuid::new_v4(),
                revision: 0,
                phase: huginn_core::providers::ConnectionPhase::Unconfigured,
                selection: None,
            });
        receiver
    }
    async fn recover(&self) -> ConnectionResult<()> {
        if self.recovery_fail.load(Ordering::SeqCst) {
            return Err(ConnectionError::Metadata);
        }
        self.uncertain.store(false, Ordering::SeqCst);
        Ok(())
    }
    async fn list(&self) -> ConnectionResult<Vec<ConnectionRecord>> {
        if self.uncertain.load(Ordering::SeqCst) {
            return Err(ConnectionError::Metadata);
        }
        Ok(self.records.lock().unwrap().values().cloned().collect())
    }
    async fn status(&self, _: uuid::Uuid) -> ConnectionResult<()> {
        if self.fail.load(Ordering::SeqCst) {
            Err(ConnectionError::Locked)
        } else {
            Ok(())
        }
    }
    async fn save(
        &self,
        previous: Option<uuid::Uuid>,
        settings: ConnectionSettings,
        secret: Option<Secret>,
        mode: SecretMode,
    ) -> ConnectionResult<ConnectionRecord> {
        settings.validate()?;
        if settings.requires_secret {
            assert_eq!(secret.as_ref().unwrap().expose(), SENTINEL);
        } else {
            assert!(secret.is_none());
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(ConnectionError::Locked);
        }
        let id = previous.unwrap_or_else(uuid::Uuid::new_v4);
        let record = ConnectionRecord {
            id,
            revision: uuid::Uuid::new_v4(),
            settings,
            secret: secret.map(|_| SecretReference::new()),
            mode,
        };
        self.records.lock().unwrap().insert(id, record.clone());
        *self.selected.lock().unwrap() = None;
        Ok(record)
    }
    async fn select_model(
        &self,
        id: uuid::Uuid,
        model: String,
    ) -> ConnectionResult<ModelSelection> {
        self.status(id).await?;
        self.records
            .lock()
            .unwrap()
            .get_mut(&id)
            .ok_or(ConnectionError::InvalidSelection)?
            .settings
            .model
            .clone_from(&model);
        *self.selected.lock().unwrap() = Some((id, model.clone()));
        Ok(ModelSelection {
            connection: ConnectionIdentity {
                id: id.to_string(),
                adapter: ProviderRegistration::new("double".into(), 1),
                revision: 1,
            },
            model,
        })
    }
    async fn revoke(&self, id: uuid::Uuid) -> ConnectionResult<()> {
        self.records
            .lock()
            .unwrap()
            .remove(&id)
            .ok_or(ConnectionError::InvalidSelection)?;
        *self.selected.lock().unwrap() = None;
        Ok(())
    }
}
struct Catalog(
    Vec<AuthenticationMethod>,
    ProviderRegistration,
    Vec<(ProviderRegistration, ProviderDescriptor)>,
);
impl ProviderCatalog for Catalog {
    fn adapters(&self) -> Vec<(ProviderRegistration, ProviderDescriptor)> {
        let mut adapters = vec![(
            self.1.clone(),
            ProviderDescriptor {
                id: "double".into(),
                display_name: "Test provider".into(),
                models: vec![
                    ModelDescriptor {
                        id: "alpha".into(),
                        supports_tools: true,
                    },
                    ModelDescriptor {
                        id: "beta".into(),
                        supports_tools: true,
                    },
                ],
                authentication: self.0.clone(),
            },
        )];
        adapters.extend(self.2.iter().cloned());
        adapters
    }
    fn register(
        &self,
        _: &mut PluginContext,
        _: Arc<dyn ProviderAdapter>,
    ) -> Result<ProviderRegistration> {
        unreachable!()
    }
}
struct Host {
    contributions: Arc<Contributions>,
    manager: Arc<Manager>,
    authentication: Vec<AuthenticationMethod>,
    extra_adapters: Vec<(ProviderRegistration, ProviderDescriptor)>,
}
#[async_trait]
impl Plugin for Host {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "host".into(),
            provides: vec![
                contributions_id(),
                connections_id(),
                ServiceId::new("agent:provider-catalog", 1),
            ],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(contributions_id(), UiService(self.contributions.clone()))?;
        ctx.provide(connections_id(), ConnectionsService(self.manager.clone()))?;
        ctx.provide(
            ServiceId::new("agent:provider-catalog", 1),
            ProviderCatalogService(Arc::new(Catalog(
                self.authentication.clone(),
                ProviderRegistration::new("double".into(), 1),
                self.extra_adapters.clone(),
            ))),
        )
    }
}
async fn setup() -> (Supervisor, Arc<Ui>, Arc<Manager>) {
    setup_with_authentication(vec![AuthenticationMethod::ApiKey]).await
}
async fn setup_with_authentication(
    authentication: Vec<AuthenticationMethod>,
) -> (Supervisor, Arc<Ui>, Arc<Manager>) {
    setup_with_catalog(authentication, Vec::new()).await
}
async fn setup_with_catalog(
    authentication: Vec<AuthenticationMethod>,
    extra_adapters: Vec<(ProviderRegistration, ProviderDescriptor)>,
) -> (Supervisor, Arc<Ui>, Arc<Manager>) {
    setup_with_mode(authentication, extra_adapters, false).await
}
async fn setup_standalone() -> (Supervisor, Arc<Ui>, Arc<Manager>) {
    setup_with_mode(vec![AuthenticationMethod::ApiKey], Vec::new(), true).await
}
async fn setup_with_mode(
    authentication: Vec<AuthenticationMethod>,
    extra_adapters: Vec<(ProviderRegistration, ProviderDescriptor)>,
    standalone: bool,
) -> (Supervisor, Arc<Ui>, Arc<Manager>) {
    let contributions = Arc::new(Contributions::default());
    let manager = Arc::new(Manager::default());
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(Host {
            contributions: contributions.clone(),
            manager: manager.clone(),
            authentication,
            extra_adapters,
        }))
        .unwrap();
    if standalone {
        supervisor
            .register(Box::new(
                huginn_presentation::providers::StandaloneProvidersUiPlugin(Arc::new(
                    AtomicBool::new(false),
                )),
            ))
            .unwrap();
    } else {
        supervisor.register(Box::new(ProvidersUiPlugin)).unwrap();
    }
    supervisor.start().await.unwrap();
    let ui = Ui::new(
        &Session::new(".".into()),
        contributions,
        CancellationToken::new(),
        Arc::new(huginn_presentation::settings::UiConfig::defaults()),
        Arc::new(|_: &huginn_presentation::settings::UiConfig| Ok(())),
    )
    .0;
    (supervisor, ui, manager)
}

#[tokio::test]
async fn oauth_only_adapter_is_discoverable_but_never_starts_authentication() {
    let (_supervisor, ui, manager) =
        setup_with_authentication(vec![AuthenticationMethod::OAuth]).await;
    ui.dispatch_command("/login");
    settle(&ui).await;
    let catalog = ui.lock().workflow().unwrap().snapshot();
    assert!(catalog.1.contains("OAuth: unsupported"));
    assert!(catalog.2.iter().any(|row| row.contains("Test provider")));
    submit(&ui).await;
    let snapshot = ui.lock().workflow().unwrap().snapshot();
    assert!(snapshot.0.contains("Unsupported"));
    assert!(snapshot.1.contains("OAuth: unsupported"));
    assert!(snapshot.1.contains("No connection was created"));
    assert_eq!(snapshot.2, ["Action: Back to provider catalog"]);
    assert!(manager.records.lock().unwrap().is_empty());
    assert!(manager.selected.lock().unwrap().is_none());
}

#[tokio::test]
async fn duplicate_provider_display_names_include_the_provider_id() {
    let extra = ["provider-a", "provider-b"]
        .into_iter()
        .map(|id| {
            (
                ProviderRegistration::new(id.into(), 1),
                ProviderDescriptor {
                    id: id.into(),
                    display_name: "Same provider".into(),
                    models: vec![],
                    authentication: vec![],
                },
            )
        })
        .collect::<Vec<_>>();
    let (_supervisor, ui, _manager) =
        setup_with_catalog(vec![AuthenticationMethod::ApiKey], extra).await;
    ui.dispatch_command("/login");
    settle(&ui).await;
    let catalog = ui.lock();
    let options = catalog
        .workflow()
        .unwrap()
        .workflow
        .fields()
        .iter()
        .find_map(|field| match &field.kind {
            FieldKind::Choice(options) => Some(options.clone()),
            _ => None,
        })
        .expect("the catalog offers a provider choice");
    drop(catalog);
    // A duplicated display name carries the provider id.
    assert!(
        options.contains(&"Same provider (provider-a)".to_string()),
        "{options:?}"
    );
    assert!(
        options.contains(&"Same provider (provider-b)".to_string()),
        "{options:?}"
    );
    // A unique display name keeps its plain label.
    assert!(
        options.contains(&"Test provider".to_string()),
        "{options:?}"
    );
}

#[tokio::test]
async fn unsupported_oauth_stage_returns_to_the_root_catalog_without_history_growth() {
    let (_supervisor, ui, manager) =
        setup_with_authentication(vec![AuthenticationMethod::OAuth]).await;
    ui.dispatch_command("/login");
    settle(&ui).await;
    submit(&ui).await;
    assert!(ui
        .lock()
        .workflow()
        .unwrap()
        .snapshot()
        .0
        .contains("Unsupported"));
    // The explicit action returns to the catalog as the first step.
    submit(&ui).await;
    let catalog = ui.lock();
    let form = catalog.workflow().unwrap();
    assert!(form.snapshot().0.contains("Step 1 of 4"));
    drop(catalog);
    // Esc at the root catalog closes the flow. A duplicated history would
    // reopen the unsupported stage instead.
    onboarding_key(
        &ui,
        crossterm::event::KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    );
    assert!(ui.lock().workflow().is_none());
    assert!(manager.records.lock().unwrap().is_empty());
    assert!(manager.selected.lock().unwrap().is_none());
}

#[tokio::test]
async fn unsupported_oauth_stage_escape_returns_to_the_catalog_once() {
    let (_supervisor, ui, manager) =
        setup_with_authentication(vec![AuthenticationMethod::OAuth]).await;
    ui.dispatch_command("/login");
    settle(&ui).await;
    submit(&ui).await;
    assert!(ui
        .lock()
        .workflow()
        .unwrap()
        .snapshot()
        .0
        .contains("Unsupported"));
    // Esc uses the host-owned back transition and restores the catalog.
    onboarding_key(
        &ui,
        crossterm::event::KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    );
    let catalog = ui.lock();
    let form = catalog.workflow().unwrap();
    assert!(form.snapshot().0.contains("Step 1 of 4"));
    drop(catalog);
    onboarding_key(
        &ui,
        crossterm::event::KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    );
    assert!(ui.lock().workflow().is_none());
    assert!(manager.records.lock().unwrap().is_empty());
}

#[tokio::test]
async fn mixed_authentication_keeps_key_and_local_forms_without_oauth_choices() {
    let (_supervisor, ui, manager) = setup_with_authentication(vec![
        AuthenticationMethod::ApiKey,
        AuthenticationMethod::OAuth,
        AuthenticationMethod::None,
    ])
    .await;
    login(&ui).await; // API-key behavior and secret masking remain unchanged.
    assert!(
        manager
            .records
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .settings
            .requires_secret
    );
    ui.dispatch_command("/providers");
    settle(&ui).await;
    assert!(ui
        .lock()
        .workflow()
        .unwrap()
        .snapshot()
        .1
        .contains("OAuth: unsupported"));
    ui.cancel_workflow();
    ui.dispatch_command("/login");
    settle(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        assert!(form.snapshot().1.contains("OAuth: unsupported"));
        assert!(form.snapshot().2[0].contains("Test provider"));
        let initial = form.snapshot().2;
        form.choice(true);
        assert_eq!(form.snapshot().2, initial);
        drop(state);
    }
    submit(&ui).await;
    assert_eq!(
        ui.lock().workflow().unwrap().snapshot().0,
        "Provider already configured"
    );
    ui.lock().workflow_mut().unwrap().choice(true);
    submit(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        assert!(form.snapshot().2[0].contains("API key"));
        form.choice(true);
        assert!(form.snapshot().2[0].contains("No authentication"));
        drop(state);
    }
    submit(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        assert_eq!(form.snapshot().2.len(), 3, "no secret or storage field");
        form.insert("http://127.0.0.1:11434");
        form.navigate(true);
        form.navigate(true);
        form.choice(true);
        drop(state);
    }
    submit(&ui).await;
    let records = manager.records.lock().unwrap();
    assert_eq!(records.len(), 2);
    assert!(records
        .values()
        .any(|record| !record.settings.requires_secret && record.secret.is_none()));
    drop(records);
}
async fn settle(ui: &Ui) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            ui.refresh_contributions();
            if !ui.lock().workflow().is_some_and(|w| w.is_pending()) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn submit(ui: &Ui) {
    assert!(ui.submit_workflow());
    settle(ui).await;
}
fn assert_surface(ui: &Ui, kind: huginn_presentation::overlay::OverlayKind) {
    assert_eq!(ui.lock().overlays.snapshot().unwrap().kind, kind);
}
async fn login(ui: &Ui) {
    use huginn_presentation::overlay::OverlayKind as Surface;
    assert!(ui.dispatch_command("/login"));
    assert_surface(ui, Surface::Login); // loading is a real autoexecuted workflow
    settle(ui).await;
    assert_surface(ui, Surface::Login);
    submit(ui).await; // choose provider
    if ui.lock().workflow().unwrap().snapshot().0 == "Provider already configured" {
        ui.lock().workflow_mut().unwrap().choice(true);
        submit(ui).await;
    }
    if ui.lock().workflow().unwrap().snapshot().2[0].contains("Authentication") {
        submit(ui).await; // first supported method in this test catalog is API key
    }
    assert_surface(ui, Surface::Login);
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        form.insert("https://example.invalid/v1");
        form.navigate(true); // alpha
        form.navigate(true); // local HTTP stays disabled
        form.navigate(true);
        form.choice(true); // explicit ephemeral
        form.navigate(true);
        form.insert(SENTINEL);
        assert!(!format!("{:?}", form.snapshot()).contains(SENTINEL));
        drop(state);
    }
    submit(ui).await;
}

#[tokio::test]
async fn repeated_ui_commands_after_dedup_expiry_preserve_stream_and_conversation() {
    use huginn_core::{Event, EventSink};
    use huginn_presentation::{event::Clock, Interaction};
    use std::{sync::atomic::AtomicU64, time::Duration};
    #[derive(Default)]
    struct FakeClock(AtomicU64);
    impl Clock for FakeClock {
        fn now(&self) -> Duration {
            Duration::from_millis(self.0.load(Ordering::SeqCst))
        }
    }
    let (mut supervisor, original, manager) = setup().await;
    supervisor
        .register(Box::new(
            huginn_presentation::contributions::ConversationPlugin,
        ))
        .unwrap();
    supervisor.start().await.unwrap();
    let clock = Arc::new(FakeClock::default());
    let ui = Ui::new_with_clock(
        &Session::new(".".into()),
        original.contributions.clone(),
        CancellationToken::new(),
        Arc::new(huginn_presentation::settings::UiConfig::defaults()),
        Arc::new(|_: &huginn_presentation::settings::UiConfig| Ok(())),
        clock.clone(),
    )
    .0;
    login(&ui).await;
    let cancel = CancellationToken::new();
    let operation = ui.begin(cancel.clone());
    ui.conversation("legitimate conversation".into());
    ui.emit_for(
        operation,
        Event::Text {
            text: "stream remains intact".into(),
        },
    );
    let baseline = ui.lock().conversation.clone();
    for round in 0..2 {
        if round == 1 {
            clock.0.fetch_add(2_001, Ordering::SeqCst);
        }
        for command in [
            "/help",
            "/providers",
            "/login",
            "/model",
            "/settings",
            "/logout",
        ] {
            assert!(ui.dispatch_command(command), "{command}");
            settle(&ui).await;
            let (ticket, focus) = {
                let state = ui.lock();
                (state.overlays.ticket(), state.overlays.focus())
            };
            ui.close_overlay(ticket, focus);
            let state = ui.lock();
            assert!(state.overlays.snapshot().is_none(), "{command}");
            assert_eq!(state.conversation, baseline, "{command}, round {round}");
            assert_eq!(state.tail, "stream remains intact");
            assert!(state.editor.text.is_empty());
            assert!(!cancel.is_cancelled());
            drop(state);
        }
    }
    assert_eq!(manager.records.lock().unwrap().len(), 1);
    ui.finish_operation(operation);
}

#[tokio::test]
async fn commands_create_select_update_revoke_without_restart_and_never_publish_secret() {
    use huginn_presentation::overlay::OverlayKind as Surface;
    let (_supervisor, ui, manager) = setup().await;
    for name in ["/login", "/providers", "/model", "/logout"] {
        assert!(ui.contributions.command(name).is_some());
    }
    login(&ui).await;
    let id = *manager.records.lock().unwrap().keys().next().unwrap();
    assert_eq!(
        manager.records.lock().unwrap()[&id].mode,
        SecretMode::Ephemeral
    );
    assert!(manager.selected.lock().unwrap().is_none());
    ui.dispatch_command("/model");
    assert_surface(&ui, Surface::ModelPicker);
    settle(&ui).await;
    assert_surface(&ui, Surface::ModelPicker);
    submit(&ui).await;
    assert_surface(&ui, Surface::ModelPicker);
    ui.lock().workflow_mut().unwrap().choice(true);
    submit(&ui).await;
    assert_eq!(*manager.selected.lock().unwrap(), Some((id, "beta".into())));
    assert!(ui
        .lock()
        .notifications
        .iter()
        .any(|event| event.summary.contains("NEXT turn")));
    let previous_revision = manager.records.lock().unwrap()[&id].revision;
    ui.dispatch_command("/providers");
    assert_surface(&ui, Surface::Provider);
    settle(&ui).await;
    assert_surface(&ui, Surface::Provider);
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        form.navigate(true);
        form.choice(true);
        drop(state);
    }
    submit(&ui).await;
    assert_surface(&ui, Surface::Login); // exact update form, not a replacement lookup
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        for _ in 0..4 {
            form.navigate(true);
        }
        form.insert(SENTINEL);
        drop(state);
    }
    submit(&ui).await;
    assert_ne!(
        manager.records.lock().unwrap()[&id].revision,
        previous_revision
    );
    assert!(manager.selected.lock().unwrap().is_none());
    ui.dispatch_command("/logout");
    assert_surface(&ui, Surface::Confirm);
    settle(&ui).await;
    assert_surface(&ui, Surface::Confirm);
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        form.navigate(true);
        form.choice(true);
        drop(state);
    }
    submit(&ui).await;
    assert!(manager.records.lock().unwrap().is_empty());
    let state = ui.lock();
    assert!(state.workflow().is_none());
    assert!(state.editor.text.is_empty());
    assert!(state.events.is_empty());
    assert!(state.tail.is_empty());
    assert!(!state.conversation.join("\n").contains(SENTINEL));
    drop(state);
    assert!(!format!("{:?}", ui.contributions.widgets()).contains(SENTINEL));
}

#[tokio::test]
async fn update_preserves_public_model_http_and_ephemeral_storage_defaults() {
    let (_supervisor, ui, manager) = setup().await;
    login(&ui).await;
    let id = *manager.records.lock().unwrap().keys().next().unwrap();
    {
        let mut records = manager.records.lock().unwrap();
        let record = records.get_mut(&id).unwrap();
        record.settings.model = "beta".into();
        record.settings.allow_loopback_http = true;
        drop(records);
    }
    ui.dispatch_command("/providers");
    settle(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        form.navigate(true);
        form.choice(true);
        drop(state);
    }
    submit(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        let fields = form.snapshot().2;
        assert_eq!(fields[1], "Model: beta");
        assert!(fields[2].contains("Yes, explicitly"));
        assert!(fields[3].contains("Ephemeral"));
        assert_eq!(fields[4], "API key: ");
        for _ in 0..4 {
            form.navigate(true);
        }
        form.insert(SENTINEL);
        drop(state);
    }
    submit(&ui).await;
    let records = manager.records.lock().unwrap();
    assert_eq!(records[&id].settings.model, "beta");
    assert!(records[&id].settings.allow_loopback_http);
    assert_eq!(records[&id].mode, SecretMode::Ephemeral);
}

#[tokio::test]
async fn model_picker_starts_on_the_saved_model_and_submits_it_unchanged() {
    let (_supervisor, ui, manager) = setup().await;
    login(&ui).await;
    let id = *manager.records.lock().unwrap().keys().next().unwrap();
    manager
        .records
        .lock()
        .unwrap()
        .get_mut(&id)
        .unwrap()
        .settings
        .model = "beta".into();
    ui.dispatch_command("/model");
    settle(&ui).await;
    submit(&ui).await; // choose the only connection, which opens the model form
    {
        let state = ui.lock();
        let form = state.workflow().unwrap();
        assert_eq!(
            form.snapshot().2[0],
            "Model: beta",
            "the picker must start on the stored model, not on the first catalog entry"
        );
        drop(state);
    }
    submit(&ui).await;
    assert_eq!(*manager.selected.lock().unwrap(), Some((id, "beta".into())));
}

#[tokio::test]
async fn providers_disconnect_keeps_real_confirm_callback_and_local_cancel_is_recoverable() {
    use huginn_presentation::overlay::OverlayKind as Surface;
    let (_supervisor, ui, manager) = setup().await;
    login(&ui).await;
    for cancel in [true, false] {
        ui.dispatch_command("/providers");
        settle(&ui).await;
        {
            let mut state = ui.lock();
            let form = state.workflow_mut().unwrap();
            form.navigate(true);
            form.choice(true);
            form.choice(true); // Disconnect is a continuation, not an Approval
            drop(state);
        }
        submit(&ui).await;
        assert_surface(&ui, Surface::Confirm);
        assert!(ui.lock().modal().is_none());
        ui.lock().workflow_mut().unwrap().choice(true);
        if cancel {
            ui.cancel_workflow();
            assert!(!ui.submit_workflow());
            assert_eq!(manager.records.lock().unwrap().len(), 1);
        } else {
            submit(&ui).await;
            assert!(manager.records.lock().unwrap().is_empty());
            assert!(ui
                .lock()
                .notifications
                .iter()
                .last()
                .unwrap()
                .summary
                .contains("revoked locally"));
        }
    }
    assert!(ui.lock().editor.text.is_empty());
    assert!(ui.lock().conversation.is_empty());
}

#[tokio::test]
async fn providers_recovers_before_listing_and_reports_failed_recovery() {
    let (_supervisor, ui, manager) = setup().await;
    login(&ui).await;
    manager.uncertain.store(true, Ordering::SeqCst);
    manager.recovery_fail.store(true, Ordering::SeqCst);
    ui.dispatch_command("/providers");
    settle(&ui).await;
    assert!(ui.lock().workflow().is_none());
    assert!(manager.uncertain.load(Ordering::SeqCst));
    assert!(ui
        .lock()
        .notifications
        .iter()
        .any(|event| event.summary.contains("Connection action failed")));
    manager.recovery_fail.store(false, Ordering::SeqCst);
    ui.dispatch_command("/providers");
    settle(&ui).await;
    assert!(ui.lock().workflow().is_some());
    assert!(!manager.uncertain.load(Ordering::SeqCst));
    submit(&ui).await; // choose connection/action
    submit(&ui).await; // select model
    assert!(manager.selected.lock().unwrap().is_some());
}

#[tokio::test]
async fn locked_status_is_actionable_and_cancelled_login_does_not_save() {
    let (mut supervisor, ui, manager) = setup().await;
    login(&ui).await;
    manager.fail.store(true, Ordering::SeqCst);
    ui.dispatch_command("/providers");
    settle(&ui).await;
    assert!(format!("{:?}", ui.lock().workflow().unwrap().snapshot()).contains("locked"));
    ui.cancel_workflow();
    ui.dispatch_command("/login");
    settle(&ui).await;
    submit(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        for _ in 0..4 {
            form.navigate(true);
        }
        form.insert(SENTINEL);
        drop(state);
    }
    ui.cancel_workflow();
    assert_eq!(manager.records.lock().unwrap().len(), 1);
    supervisor.remove("builtin.providers.ui").await.unwrap();
    ui.refresh_contributions();
    assert!(!ui.dispatch_command("/login"));
    assert_eq!(
        manager.list().await.unwrap().len(),
        1,
        "headless service survives optional UI removal"
    );
}

#[tokio::test]
async fn mixed_authentication_has_one_provider_then_authentication_inside_form() {
    let (_supervisor, ui, _) = setup_with_authentication(vec![
        AuthenticationMethod::None,
        AuthenticationMethod::ApiKey,
    ])
    .await;
    ui.dispatch_command("/login");
    settle(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        let initial = form.snapshot().2;
        form.choice(true);
        assert_eq!(
            form.snapshot().2,
            initial,
            "one provider must not expand into authentication entries"
        );
        drop(state);
    }
    submit(&ui).await;
    assert!(ui
        .lock()
        .workflow()
        .unwrap()
        .snapshot()
        .2
        .iter()
        .any(|line| line.contains("Authentication")));
}

#[tokio::test]
async fn onboarding_has_four_transient_stages_and_explicit_result_actions() {
    let (_supervisor, ui, manager) = setup().await;
    let before = ui.lock().conversation.len();
    ui.dispatch_command("/login");
    settle(&ui).await;
    assert!(ui
        .lock()
        .workflow()
        .unwrap()
        .snapshot()
        .0
        .contains("Step 1 of 4"));
    submit(&ui).await;
    assert!(ui
        .lock()
        .workflow()
        .unwrap()
        .snapshot()
        .0
        .contains("Step 2 of 4"));
    submit(&ui).await;
    assert!(ui
        .lock()
        .workflow()
        .unwrap()
        .snapshot()
        .0
        .contains("Step 3 of 4"));
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        form.insert("https://example.com/v1");
        form.navigate(true);
        form.insert("model");
        form.navigate(true);
        form.navigate(true);
        form.navigate(true);
        form.insert(SENTINEL);
        drop(state);
    }
    submit(&ui).await;
    {
        let state = ui.lock();
        let result = state.workflow().unwrap().snapshot();
        assert_eq!(state.conversation.len(), before);
        drop(state);
        assert!(result.0.contains("Step 4 of 4"));
        assert!(result.1.contains("not been verified"));
        assert!(result.1.contains("active model is unchanged"));
        assert!(!format!("{result:?}").contains(SENTINEL));
    }
    assert_eq!(manager.records.lock().unwrap().len(), 1);
    assert!(manager.selected.lock().unwrap().is_none());
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        form.choice(true);
        form.choice(true);
        drop(state);
    }
    submit(&ui).await;
    assert!(ui.lock().workflow().is_none());
    assert_eq!(ui.lock().conversation.len(), before);
}

#[tokio::test]
async fn existing_provider_requires_explicit_add_and_rows_distinguish_ids() {
    let (_supervisor, ui, manager) = setup().await;
    login(&ui).await;
    let original = manager
        .records
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    ui.dispatch_command("/login");
    settle(&ui).await;
    submit(&ui).await;
    let snapshot = ui.lock().workflow().unwrap().snapshot();
    assert_eq!(snapshot.0, "Provider already configured");
    assert!(snapshot.1.contains(&original.id.to_string()));
    ui.cancel_workflow();
    assert_eq!(manager.records.lock().unwrap().len(), 1);
    login(&ui).await;
    assert_eq!(manager.records.lock().unwrap().len(), 2);
    assert_eq!(
        manager.records.lock().unwrap().get(&original.id),
        Some(&original)
    );
    ui.dispatch_command("/providers");
    settle(&ui).await;
    let mut state = ui.lock();
    let form = state.workflow_mut().unwrap();
    let mut public = format!("{:?}", form.snapshot());
    form.choice(true);
    public.push_str(&format!("{:?}", form.snapshot()));
    drop(state);
    for id in manager.records.lock().unwrap().keys() {
        assert!(public.contains(&id.to_string()));
    }
    assert!(public.contains("remote availability not checked"));
}

#[tokio::test]
async fn standalone_login_does_not_enter_existing_connection_management() {
    let (_supervisor, ui, manager) = setup_standalone().await;
    let id = uuid::Uuid::new_v4();
    manager.records.lock().unwrap().insert(
        id,
        ConnectionRecord {
            id,
            revision: uuid::Uuid::new_v4(),
            settings: ConnectionSettings {
                provider: "double".into(),
                endpoint: Some("https://example.invalid/v1".into()),
                model: "alpha".into(),
                requires_secret: false,
                allow_loopback_http: false,
            },
            secret: None,
            mode: SecretMode::Ephemeral,
        },
    );

    ui.dispatch_command("/login");
    settle(&ui).await;
    submit(&ui).await;

    let workflow = ui.lock().workflow().unwrap().snapshot();
    assert_eq!(workflow.0, "Authentication method — Step 2 of 4");
    assert_eq!(manager.records.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn provider_search_is_case_insensitive_and_empty_results_do_not_submit() {
    let (_supervisor, ui, manager) = setup().await;
    let before = ui.lock().conversation.len();
    ui.dispatch_command("/login");
    settle(&ui).await;
    ui.lock().workflow_mut().unwrap().insert("tEsT");
    submit(&ui).await;
    assert!(ui
        .lock()
        .workflow()
        .unwrap()
        .snapshot()
        .0
        .contains("Step 2 of 4"));
    ui.cancel_workflow();
    ui.dispatch_command("/login");
    settle(&ui).await;
    ui.lock()
        .workflow_mut()
        .unwrap()
        .insert("no-provider-matches-this");
    assert!(!ui.submit_workflow());
    ui.cancel_workflow();
    assert!(ui.lock().workflow().is_none());
    assert_eq!(ui.lock().conversation.len(), before);
    assert!(manager.records.lock().unwrap().is_empty());
}

#[tokio::test]
async fn escape_from_initial_provider_search_closes_instead_of_reopening_loading() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let (_supervisor, ui, manager) = setup().await;
    ui.dispatch_command("/login");
    settle(&ui).await;
    let (ticket, focus) = {
        let state = ui.lock();
        (state.overlays.ticket(), state.overlays.focus())
    };
    ui.overlay_input(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        ticket,
        focus,
    );
    assert!(ui.lock().workflow().is_none());
    assert!(manager.records.lock().unwrap().is_empty());
}

fn onboarding_key(
    ui: &Ui,
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) {
    let (ticket, focus) = {
        let state = ui.lock();
        (state.overlays.ticket(), state.overlays.focus())
    };
    ui.overlay_input(
        crossterm::event::Event::Key(crossterm::event::KeyEvent::new(code, modifiers)),
        ticket,
        focus,
    );
}
async fn open_onboarding_settings(ui: &Ui) {
    assert!(ui.dispatch_command("/login"));
    settle(ui).await;
    submit(ui).await;
    submit(ui).await;
    assert!(ui
        .lock()
        .workflow()
        .unwrap()
        .snapshot()
        .0
        .contains("Step 3 of 4"));
}
fn edit_onboarding_settings(ui: &Ui) {
    let mut state = ui.lock();
    let form = state.workflow_mut().unwrap();
    form.insert("https://draft.example.invalid/v1");
    form.navigate(true);
    form.choice(true); // beta
    form.navigate(true);
    form.choice(true); // explicit loopback HTTP consent
    form.navigate(true);
    form.choice(true); // ephemeral storage
    form.navigate(true);
    form.insert(SENTINEL);
    drop(state);
}

#[tokio::test]
async fn back_and_forward_restore_search_and_public_drafts_but_require_a_fresh_secret() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let (_supervisor, ui, manager) = setup().await;
    ui.dispatch_command("/login");
    settle(&ui).await;
    ui.lock().workflow_mut().unwrap().insert("tEsT");
    submit(&ui).await;
    submit(&ui).await;
    edit_onboarding_settings(&ui);
    let draft = ui.lock().workflow().unwrap().snapshot();
    onboarding_key(&ui, KeyCode::Esc, KeyModifiers::NONE);
    onboarding_key(&ui, KeyCode::Esc, KeyModifiers::NONE);
    {
        let state = ui.lock();
        let form = state.workflow().unwrap();
        assert_eq!(form.search_query(), "tEsT");
        assert_eq!(form.search_rows(), vec![("Test provider".into(), true)]);
        drop(state);
    }
    submit(&ui).await;
    submit(&ui).await;
    {
        let state = ui.lock();
        let form = state.workflow().unwrap();
        let restored = form.snapshot();
        assert_eq!(restored.2[..4], draft.2[..4]);
        assert_eq!(restored.3, 4);
        assert_eq!(restored.2[4], "API key: ");
        drop(state);
    }
    assert!(!ui.submit_workflow(), "Back must not restore an API key");
    assert!(manager.records.lock().unwrap().is_empty());
    ui.lock().workflow_mut().unwrap().insert(SENTINEL);
    submit(&ui).await;
    assert!(!ui.lock().workflow().unwrap().can_go_back());
    onboarding_key(&ui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(
        ui.lock().workflow().is_none(),
        "saved settings cannot be reopened with Esc"
    );
    let records = manager.records.lock().unwrap();
    assert_eq!(records.len(), 1);
    let record = records.values().next().unwrap();
    assert_eq!(
        record.settings.endpoint.as_deref(),
        Some("https://draft.example.invalid/v1")
    );
    assert_eq!(record.settings.model, "beta");
    assert!(record.settings.allow_loopback_http);
    assert_eq!(record.mode, SecretMode::Ephemeral);
    drop(records);
    assert!(ui.lock().conversation.is_empty());
}

#[tokio::test]
async fn changing_authentication_discards_an_incompatible_forward_draft() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let (_supervisor, ui, manager) = setup_with_authentication(vec![
        AuthenticationMethod::ApiKey,
        AuthenticationMethod::None,
    ])
    .await;
    open_onboarding_settings(&ui).await;
    edit_onboarding_settings(&ui);
    onboarding_key(&ui, KeyCode::Esc, KeyModifiers::NONE);
    ui.lock().workflow_mut().unwrap().choice(true);
    submit(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow_mut().unwrap();
        let snapshot = form.snapshot();
        assert_eq!(snapshot.2.len(), 3);
        assert_eq!(snapshot.3, 0);
        assert!(!snapshot.2.join(" ").contains("draft.example.invalid"));
        assert_eq!(snapshot.2[1], "Model: alpha");
        assert!(snapshot.2[2].ends_with("No"));
        form.insert("http://127.0.0.1:11434");
        form.navigate(true);
        form.navigate(true);
        form.choice(true);
        drop(state);
    }
    submit(&ui).await;
    let records = manager.records.lock().unwrap();
    let record = records.values().next().unwrap();
    assert_eq!(records.len(), 1);
    assert!(!record.settings.requires_secret);
    assert!(record.secret.is_none());
}

#[tokio::test]
async fn ctrl_c_discards_backward_and_forward_drafts_without_saving() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let (_supervisor, ui, manager) = setup().await;
    open_onboarding_settings(&ui).await;
    edit_onboarding_settings(&ui);
    onboarding_key(&ui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(ui.lock().workflow().unwrap().can_go_back());
    onboarding_key(&ui, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(ui.lock().workflow().is_none());
    open_onboarding_settings(&ui).await;
    let state = ui.lock();
    let snapshot = state.workflow().unwrap().snapshot();
    assert_eq!(snapshot.3, 0);
    assert!(!snapshot.2.join(" ").contains("draft.example.invalid"));
    assert_eq!(snapshot.2[1], "Model: alpha");
    assert!(snapshot.2[3].contains("Secure OS store"));
    assert_eq!(snapshot.2[4], "API key: ");
    assert!(state.conversation.is_empty());
    drop(state);
    assert!(manager.records.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_onboarding_save_keeps_public_settings_and_retries_with_a_fresh_secret() {
    let (_supervisor, ui, manager) = setup().await;
    open_onboarding_settings(&ui).await;
    edit_onboarding_settings(&ui);
    let draft = ui.lock().workflow().unwrap().snapshot();
    manager.fail.store(true, Ordering::SeqCst);
    submit(&ui).await;
    {
        let state = ui.lock();
        let form = state.workflow().unwrap();
        let snapshot = form.snapshot();
        assert_eq!(snapshot.0, draft.0);
        assert_eq!(snapshot.2[..4], draft.2[..4]);
        assert_eq!(snapshot.3, draft.3);
        assert_eq!(snapshot.2[4], "API key: ");
        assert!(form.error_message().unwrap().contains("Could not save"));
        assert!(!format!("{snapshot:?}").contains(SENTINEL));
        drop(state);
    }
    assert!(manager.records.lock().unwrap().is_empty());
    assert!(!ui.submit_workflow());
    manager.fail.store(false, Ordering::SeqCst);
    ui.lock().workflow_mut().unwrap().insert(SENTINEL);
    submit(&ui).await;
    assert!(ui
        .lock()
        .workflow()
        .unwrap()
        .snapshot()
        .0
        .contains("Step 4 of 4"));
    assert_eq!(manager.records.lock().unwrap().len(), 1);
    assert!(manager.selected.lock().unwrap().is_none());
}

#[tokio::test]
async fn provider_selection_recovery_failure_stays_editable_and_can_retry() {
    let (_supervisor, ui, manager) = setup().await;
    manager.uncertain.store(true, Ordering::SeqCst);
    manager.recovery_fail.store(true, Ordering::SeqCst);
    ui.dispatch_command("/login");
    settle(&ui).await;
    submit(&ui).await;
    {
        let state = ui.lock();
        let form = state.workflow().unwrap();
        assert!(form.snapshot().0.contains("Step 1 of 4"));
        assert!(form.error_message().is_some());
        assert!(!form.is_pending());
        drop(state);
    }
    assert!(manager.uncertain.load(Ordering::SeqCst));
    assert!(manager.records.lock().unwrap().is_empty());
    manager.recovery_fail.store(false, Ordering::SeqCst);
    submit(&ui).await;
    assert!(!manager.uncertain.load(Ordering::SeqCst));
    assert!(ui
        .lock()
        .workflow()
        .unwrap()
        .snapshot()
        .0
        .contains("Step 2 of 4"));
}

#[tokio::test]
async fn existing_provider_summary_filters_other_providers_and_defaults_to_management() {
    use huginn_presentation::overlay::OverlayKind as Surface;
    let (_supervisor, ui, manager) = setup().await;
    login(&ui).await;
    let original = manager.list().await.unwrap()[0].clone();
    let mut second = original.clone();
    second.id = uuid::Uuid::from_u128(1);
    second.mode = SecretMode::Secure;
    second.settings.endpoint = Some("https://second.example.invalid/v1".into());
    let mut unrelated = original.clone();
    unrelated.id = uuid::Uuid::from_u128(2);
    unrelated.settings.provider = "another-provider".into();
    {
        let mut records = manager.records.lock().unwrap();
        records.insert(second.id, second.clone());
        records.insert(unrelated.id, unrelated.clone());
    }
    let baseline = manager.list().await.unwrap();
    ui.dispatch_command("/login");
    settle(&ui).await;
    submit(&ui).await;
    let summary = ui.lock().workflow().unwrap().snapshot();
    assert_eq!(summary.0, "Provider already configured");
    for record in [&original, &second] {
        assert!(summary.1.contains(&record.id.to_string()));
        assert!(summary
            .1
            .contains(record.settings.endpoint.as_deref().unwrap()));
        assert!(summary.1.contains(&format!("{:?}", record.mode)));
    }
    assert!(!summary.1.contains(&unrelated.id.to_string()));
    assert!(summary.2[0].contains("Manage existing connections"));
    submit(&ui).await;
    assert_surface(&ui, Surface::Provider);
    assert_eq!(manager.list().await.unwrap(), baseline);
    assert!(manager.selected.lock().unwrap().is_none());
}

#[tokio::test]
async fn configured_result_actions_route_without_implicitly_selecting_a_model() {
    use huginn_presentation::overlay::OverlayKind as Surface;
    for (action, surface, fields) in [(0, Surface::ModelPicker, 1), (1, Surface::Provider, 2)] {
        let (_supervisor, ui, manager) = setup().await;
        login(&ui).await;
        let saved = manager.list().await.unwrap();
        assert!(!ui.lock().workflow().unwrap().can_go_back());
        for _ in 0..action {
            ui.lock().workflow_mut().unwrap().choice(true);
        }
        submit(&ui).await;
        assert_surface(&ui, surface);
        assert_eq!(ui.lock().workflow().unwrap().snapshot().2.len(), fields);
        assert_eq!(manager.list().await.unwrap(), saved);
        assert!(manager.selected.lock().unwrap().is_none());
        assert!(ui.lock().conversation.is_empty());
        if action == 0 {
            submit(&ui).await; // explicitly choose a connection
            assert!(manager.selected.lock().unwrap().is_none());
            ui.lock().workflow_mut().unwrap().choice(true);
            submit(&ui).await; // explicitly choose beta
            assert_eq!(
                *manager.selected.lock().unwrap(),
                Some((saved[0].id, "beta".into()))
            );
        }
    }
}

#[tokio::test]
async fn revoking_provider_ui_removes_history_and_does_not_recover_old_drafts_on_reload() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let (mut supervisor, ui, manager) = setup().await;
    open_onboarding_settings(&ui).await;
    edit_onboarding_settings(&ui);
    onboarding_key(&ui, KeyCode::Esc, KeyModifiers::NONE);
    supervisor.remove("builtin.providers.ui").await.unwrap();
    ui.refresh_contributions();
    assert!(ui.lock().workflow().is_none());
    assert!(!ui.dispatch_command("/login"));
    supervisor.register(Box::new(ProvidersUiPlugin)).unwrap();
    supervisor.start().await.unwrap();
    open_onboarding_settings(&ui).await;
    let snapshot = ui.lock().workflow().unwrap().snapshot();
    assert!(!snapshot.2.join(" ").contains("draft.example.invalid"));
    assert_eq!(snapshot.2[4], "API key: ");
    assert!(manager.records.lock().unwrap().is_empty());
}
