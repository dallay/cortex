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
struct Catalog(Vec<AuthenticationMethod>);
impl ProviderCatalog for Catalog {
    fn adapters(&self) -> Vec<(ProviderRegistration, ProviderDescriptor)> {
        vec![(
            ProviderRegistration::new("double".into(), 1),
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
        )]
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
            ProviderCatalogService(Arc::new(Catalog(self.authentication.clone()))),
        )
    }
}
async fn setup() -> (Supervisor, Arc<Ui>, Arc<Manager>) {
    setup_with_authentication(vec![AuthenticationMethod::ApiKey]).await
}
async fn setup_with_authentication(
    authentication: Vec<AuthenticationMethod>,
) -> (Supervisor, Arc<Ui>, Arc<Manager>) {
    let contributions = Arc::new(Contributions::default());
    let manager = Arc::new(Manager::default());
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(Host {
            contributions: contributions.clone(),
            manager: manager.clone(),
            authentication,
        }))
        .unwrap();
    supervisor.register(Box::new(ProvidersUiPlugin)).unwrap();
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
async fn oauth_only_adapter_is_reported_as_unsupported_not_absent() {
    for command in ["/login", "/providers"] {
        let (_supervisor, ui, manager) =
            setup_with_authentication(vec![AuthenticationMethod::OAuth]).await;
        assert!(ui.dispatch_command(command));
        settle(&ui).await;
        let state = ui.lock();
        assert!(
            state.workflow.is_none(),
            "OAuth must not expose an actionable form"
        );
        let output = state.committed.join("\n");
        assert!(output.contains("Test provider"));
        assert!(output.contains("OAuth: unsupported"));
        assert!(!output.contains("No provider adapters available"));
        drop(state);
        assert!(manager.records.lock().unwrap().is_empty());
        assert!(manager.selected.lock().unwrap().is_none());
    }
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
        .workflow
        .as_ref()
        .unwrap()
        .snapshot()
        .1
        .contains("OAuth: unsupported"));
    ui.cancel_workflow();
    ui.dispatch_command("/login");
    settle(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow.as_mut().unwrap();
        assert!(form.snapshot().1.contains("OAuth: unsupported"));
        assert!(form.snapshot().2[0].contains("API key"));
        form.choice(true);
        assert!(form.snapshot().2[0].contains("no authentication"));
        form.choice(true);
        assert!(
            form.snapshot().2[0].contains("API key"),
            "OAuth must not be a choice"
        );
        form.choice(true);
        drop(state);
    }
    submit(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow.as_mut().unwrap();
        assert_eq!(
            form.snapshot().2.len(),
            3,
            "local form has no secret or storage field"
        );
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
            if !ui.lock().workflow.as_ref().is_some_and(|w| w.is_pending()) {
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
async fn login(ui: &Ui) {
    assert!(ui.dispatch_command("/login"));
    settle(ui).await;
    submit(ui).await; // choose provider/authentication
    {
        let mut state = ui.lock();
        let form = state.workflow.as_mut().unwrap();
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
async fn commands_create_select_update_revoke_without_restart_and_never_publish_secret() {
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
    settle(&ui).await;
    submit(&ui).await;
    ui.lock().workflow.as_mut().unwrap().choice(true);
    submit(&ui).await;
    assert_eq!(*manager.selected.lock().unwrap(), Some((id, "beta".into())));
    assert!(ui.lock().committed.join("\n").contains("NEXT turn"));
    let previous_revision = manager.records.lock().unwrap()[&id].revision;
    ui.dispatch_command("/providers");
    settle(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow.as_mut().unwrap();
        form.navigate(true);
        form.choice(true);
        drop(state);
    }
    submit(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow.as_mut().unwrap();
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
    settle(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow.as_mut().unwrap();
        form.navigate(true);
        form.choice(true);
        drop(state);
    }
    submit(&ui).await;
    assert!(manager.records.lock().unwrap().is_empty());
    let state = ui.lock();
    assert!(state.workflow.is_none());
    assert!(state.editor.text.is_empty());
    assert!(state.events.is_empty());
    assert!(state.tail.is_empty());
    assert!(!state.committed.join("\n").contains(SENTINEL));
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
        let form = state.workflow.as_mut().unwrap();
        form.navigate(true);
        form.choice(true);
        drop(state);
    }
    submit(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow.as_mut().unwrap();
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
async fn providers_recovers_before_listing_and_reports_failed_recovery() {
    let (_supervisor, ui, manager) = setup().await;
    login(&ui).await;
    manager.uncertain.store(true, Ordering::SeqCst);
    manager.recovery_fail.store(true, Ordering::SeqCst);
    ui.dispatch_command("/providers");
    settle(&ui).await;
    assert!(ui.lock().workflow.is_none());
    assert!(manager.uncertain.load(Ordering::SeqCst));
    assert!(ui
        .lock()
        .committed
        .join("\n")
        .contains("Connection action failed"));
    manager.recovery_fail.store(false, Ordering::SeqCst);
    ui.dispatch_command("/providers");
    settle(&ui).await;
    assert!(ui.lock().workflow.is_some());
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
    assert!(format!("{:?}", ui.lock().workflow.as_ref().unwrap().snapshot()).contains("locked"));
    ui.cancel_workflow();
    ui.dispatch_command("/login");
    settle(&ui).await;
    submit(&ui).await;
    {
        let mut state = ui.lock();
        let form = state.workflow.as_mut().unwrap();
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
