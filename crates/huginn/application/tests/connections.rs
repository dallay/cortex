mod support {
    pub mod providers;
}
use async_trait::async_trait;
use huginn_application::{
    connections::*,
    providers::{catalog_id, providers_id, ProvidersPlugin},
};
use huginn_core::{
    connections::*, kernel::Supervisor, providers::*, CancellationToken, ModelProvider,
    ModelService,
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use support::providers::*;

const SENTINEL: &str = "p02-sentinel-secret-never-present";
#[derive(Default)]
struct Metadata {
    value: Mutex<ConnectionMetadata>,
    fail: AtomicUsize,
    after: AtomicUsize,
    suspend: AtomicUsize,
    suspend_at: AtomicUsize,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
// Atomically fail exactly the Nth configured operation (for deterministic store failpoints).
fn hit(counter: &AtomicUsize) -> bool {
    let mut old = counter.load(Ordering::SeqCst);
    while old != 0 {
        match counter.compare_exchange(old, old - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return old == 1,
            Err(value) => old = value,
        }
    }
    false
}
#[async_trait]
impl ConnectionStore for Metadata {
    async fn load(&self) -> ConnectionResult<ConnectionMetadata> {
        Ok(self.value.lock().unwrap().clone())
    }
    async fn save(&self, previous: u64, next: &ConnectionMetadata) -> ConnectionResult<()> {
        let suspend = if self.suspend_at.load(Ordering::SeqCst) == 0 || hit(&self.suspend_at) {
            self.suspend.swap(0, Ordering::SeqCst)
        } else {
            0
        };
        if suspend == 1 {
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        if suspend == 3 {
            self.entered.notify_one();
            self.release.notified().await;
        }
        if hit(&self.fail) {
            return Err(ConnectionError::Metadata);
        }
        {
            let mut value = self.value.lock().unwrap();
            if value.version != previous {
                return Err(ConnectionError::Conflict);
            }
            *value = next.clone();
        }
        if suspend == 2 {
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        if hit(&self.after) {
            return Err(ConnectionError::Metadata);
        }
        Ok(())
    }
}
struct Secrets {
    value: Mutex<BTreeMap<SecretReference, String>>,
    write_fail: AtomicUsize,
    delete_fail: AtomicUsize,
    delete_error: Mutex<Option<ConnectionError>>,
    read_error: Mutex<Option<ConnectionError>>,
    read_fail: AtomicUsize,
    read_suspend: AtomicUsize,
    read_entered: tokio::sync::Notify,
    read_release: tokio::sync::Notify,
    mode: SecretMode,
}
impl Secrets {
    fn new(mode: SecretMode) -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(BTreeMap::new()),
            write_fail: AtomicUsize::new(0),
            delete_fail: AtomicUsize::new(0),
            delete_error: Mutex::new(None),
            read_error: Mutex::new(None),
            read_fail: AtomicUsize::new(0),
            read_suspend: AtomicUsize::new(0),
            read_entered: tokio::sync::Notify::new(),
            read_release: tokio::sync::Notify::new(),
            mode,
        })
    }
}
#[async_trait]
impl SecretStore for Secrets {
    fn mode(&self) -> SecretMode {
        self.mode
    }
    async fn read(&self, reference: &SecretReference) -> ConnectionResult<Secret> {
        if hit(&self.read_suspend) {
            self.read_entered.notify_one();
            self.read_release.notified().await;
        }
        if hit(&self.read_fail) {
            return Err(ConnectionError::SecretStore);
        }
        let read_error = *self.read_error.lock().unwrap();
        if let Some(error) = read_error {
            return Err(error);
        }
        Secret::new(
            self.value
                .lock()
                .unwrap()
                .get(reference)
                .cloned()
                .ok_or(ConnectionError::MissingSecret)?,
        )
    }
    async fn write(&self, reference: &SecretReference, secret: &Secret) -> ConnectionResult<()> {
        // Fail after writing too: a failure does not prove the store did nothing.
        self.value
            .lock()
            .unwrap()
            .insert(reference.clone(), secret.expose().into());
        if hit(&self.write_fail) {
            return Err(ConnectionError::SecretStore);
        }
        Ok(())
    }
    async fn delete(&self, reference: &SecretReference) -> ConnectionResult<()> {
        let delete_error = *self.delete_error.lock().unwrap();
        if let Some(error) = delete_error {
            return Err(error);
        }
        if hit(&self.delete_fail) {
            return Err(ConnectionError::SecretStore);
        }
        self.value.lock().unwrap().remove(reference);
        Ok(())
    }
}
struct CredentialAdapter {
    seen: Arc<Mutex<Vec<String>>>,
}
#[async_trait]
impl ProviderAdapter for CredentialAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "secure".into(),
            display_name: "Secure double".into(),
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
            authentication: vec![AuthenticationMethod::ApiKey, AuthenticationMethod::None],
        }
    }
    async fn create_model(
        &self,
        _: &ModelSelection,
        _: CancellationToken,
    ) -> huginn_core::Result<Arc<dyn ModelProvider>> {
        Err(ProviderError::Unavailable.into())
    }
    async fn create_connection_model(
        &self,
        selection: &ModelSelection,
        material: ConnectionMaterial,
        _: CancellationToken,
    ) -> huginn_core::Result<Arc<dyn ModelProvider>> {
        assert_eq!(selection.model, material.settings.model);
        self.seen.lock().unwrap().push(material.secret.map_or_else(
            || material.settings.endpoint.unwrap(),
            |s| s.expose().into(),
        ));
        Ok(Arc::new(TextModel("response")))
    }
}
struct Harness {
    supervisor: Supervisor,
    manager: Arc<ConnectionManager>,
    access: Arc<ConnectionAccess>,
    model: Arc<ModelService>,
    providers: Arc<ProvidersService>,
    seen: Arc<Mutex<Vec<String>>>,
}
struct PublicationFault {
    inner: Arc<dyn ProviderManagement>,
    mode: AtomicUsize,
}
#[async_trait]
impl ProviderManagement for PublicationFault {
    fn diagnostic(&self) -> huginn_core::Result<ProvidersDiagnostic> {
        self.inner.diagnostic()
    }
    fn clear_selection(&self) -> huginn_core::Result<()> {
        self.inner.clear_selection()
    }
    fn invalidate(&self, identity: &ConnectionIdentity) -> huginn_core::Result<()> {
        self.inner.invalidate(identity)
    }
    async fn execute(
        &self,
        action: ProviderAction,
        cancel: CancellationToken,
    ) -> huginn_core::Result<ProviderActionOutput> {
        let selected_identity = match &action {
            ProviderAction::Select(selection) => Some(selection.connection.clone()),
            _ => None,
        };
        let selecting = selected_identity.is_some();
        let output = self.inner.execute(action, cancel).await?;
        if selecting {
            match self.mode.swap(0, Ordering::SeqCst) {
                1 => return Err(ProviderError::Unavailable.into()),
                2 => std::future::pending::<()>().await,
                _ => {}
            }
        }
        Ok(output)
    }
}
async fn open(store: Arc<Metadata>, secrets: Arc<Secrets>) -> Harness {
    let access = ConnectionAccess::new(secrets);
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(ProvidersPlugin::with_credentials(access.clone())))
        .unwrap();
    let seen = Arc::new(Mutex::new(vec![]));
    supervisor
        .register(Box::new(AdapterPlugin::new(
            "test.secure",
            Arc::new(CredentialAdapter { seen: seen.clone() }),
        )))
        .unwrap();
    supervisor.start().await.unwrap();
    let catalog = supervisor
        .resolve::<ProviderCatalogService>(&catalog_id())
        .unwrap();
    let providers = supervisor
        .resolve::<ProvidersService>(&providers_id())
        .unwrap();
    let model = supervisor
        .resolve::<ModelService>(&huginn_application::composition::model_id())
        .unwrap();
    let manager = ConnectionManager::open(
        store,
        access.clone(),
        providers.0.clone(),
        catalog.0.clone(),
    )
    .await
    .unwrap();
    Harness {
        supervisor,
        manager: Arc::new(manager),
        access,
        model,
        providers,
        seen,
    }
}
fn settings() -> ConnectionSettings {
    ConnectionSettings {
        provider: "secure".into(),
        endpoint: Some("https://example.invalid/v1".into()),
        model: "alpha".into(),
        requires_secret: true,
        allow_loopback_http: false,
    }
}
fn secret(value: &str) -> Option<Secret> {
    Some(Secret::new(value.into()).unwrap())
}
const fn empty() -> ConnectionInputs {
    ConnectionInputs {
        cli: None,
        toml: None,
        environment: None,
    }
}

#[tokio::test]
async fn unload_cancels_admitted_save_before_publication() {
    let store = Arc::new(Metadata::default());
    let mut h = open(store.clone(), Secrets::new(SecretMode::Secure)).await;
    h.supervisor
        .register(Box::new(ConnectionsPlugin(h.manager.clone())))
        .unwrap();
    h.supervisor.start().await.unwrap();
    let service = h
        .supervisor
        .resolve::<ConnectionsService>(&connections_id())
        .unwrap();
    store.suspend.store(3, Ordering::SeqCst);
    let mut saving = Box::pin(
        service
            .0
            .save(None, settings(), secret("save"), SecretMode::Secure),
    );
    assert!(futures::poll!(saving.as_mut()).is_pending());
    store.entered.notified().await;
    h.supervisor
        .remove("builtin.providers.connections")
        .await
        .unwrap();
    store.release.notify_one();
    assert_eq!(saving.await, Err(ConnectionError::Revoked));
    let diagnostic = h.providers.0.diagnostic().unwrap();
    assert!(diagnostic.connections.is_empty());
    assert!(diagnostic.selection.is_none());
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn unload_cancels_admitted_select_before_publication() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store, secrets.clone()).await;
    let record = h
        .manager
        .create(settings(), secret("select"))
        .await
        .unwrap();
    h.supervisor
        .register(Box::new(ConnectionsPlugin(h.manager.clone())))
        .unwrap();
    h.supervisor.start().await.unwrap();
    secrets.read_suspend.store(1, Ordering::SeqCst);
    let mut selecting = Box::pin(h.manager.select(record.id));
    assert!(futures::poll!(selecting.as_mut()).is_pending());
    secrets.read_entered.notified().await;
    h.supervisor
        .remove("builtin.providers.connections")
        .await
        .unwrap();
    secrets.read_release.notify_one();
    assert_eq!(selecting.await, Err(ConnectionError::Revoked));
    let diagnostic = h.providers.0.diagnostic().unwrap();
    assert!(diagnostic.connections.is_empty());
    assert!(diagnostic.selection.is_none());
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn unload_cancels_admitted_recovery_before_publication() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets.clone()).await;
    let record = h
        .manager
        .create(settings(), secret("recover"))
        .await
        .unwrap();
    store.after.store(1, Ordering::SeqCst);
    assert_eq!(
        h.manager.select(record.id).await,
        Err(ConnectionError::Metadata)
    );
    h.supervisor
        .register(Box::new(ConnectionsPlugin(h.manager.clone())))
        .unwrap();
    h.supervisor.start().await.unwrap();
    secrets.read_suspend.store(1, Ordering::SeqCst);
    let mut recovering = Box::pin(h.manager.recover());
    assert!(futures::poll!(recovering.as_mut()).is_pending());
    secrets.read_entered.notified().await;
    h.supervisor
        .remove("builtin.providers.connections")
        .await
        .unwrap();
    secrets.read_release.notify_one();
    assert_eq!(recovering.await, Err(ConnectionError::Revoked));
    let diagnostic = h.providers.0.diagnostic().unwrap();
    assert!(diagnostic.connections.is_empty());
    assert!(diagnostic.selection.is_none());
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn unload_wakes_suspended_operation_without_waiting_for_store() {
    let store = Arc::new(Metadata::default());
    let mut h = open(store.clone(), Secrets::new(SecretMode::Secure)).await;
    h.supervisor
        .register(Box::new(ConnectionsPlugin(h.manager.clone())))
        .unwrap();
    h.supervisor.start().await.unwrap();
    store.suspend.store(3, Ordering::SeqCst);
    let saving = h.manager.create(settings(), secret("save"));
    let unloading = async {
        store.entered.notified().await;
        h.supervisor
            .remove("builtin.providers.connections")
            .await
            .unwrap();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(saving, unloading)
    })
    .await
    .expect("unload must wake cancellation without a store response");
    assert_eq!(result, Err(ConnectionError::Revoked));
    assert!(h.providers.0.diagnostic().unwrap().connections.is_empty());
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn unloading_old_lease_preserves_replacement_manager_selection() {
    let store = Arc::new(Metadata::default());
    let mut h = open(store.clone(), Secrets::new(SecretMode::Secure)).await;
    let record = h
        .manager
        .create(settings(), secret("replacement"))
        .await
        .unwrap();
    h.supervisor
        .register(Box::new(ConnectionsPlugin(h.manager.clone())))
        .unwrap();
    h.supervisor.start().await.unwrap();
    let replacement = reopen(&h, store).await.unwrap();
    let selected = replacement.select(record.id).await.unwrap();
    h.supervisor
        .remove("builtin.providers.connections")
        .await
        .unwrap();
    assert_eq!(
        h.providers.0.diagnostic().unwrap().selection,
        Some(selected.clone())
    );
    h.access
        .resolve(&selected.connection, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(replacement.select(record.id).await.unwrap(), selected);
    assert_eq!(text(h.model.0.as_ref()).await.unwrap(), "response");
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn durable_model_choice_preserves_pinned_authorization_and_secret_revision() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets.clone()).await;
    let record = h
        .manager
        .create(settings(), secret(SENTINEL))
        .await
        .unwrap();
    let initial = h.manager.select(record.id).await.unwrap();
    let pinned = h
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    let selected = h
        .manager
        .select_model(record.id, "beta".into())
        .await
        .unwrap();
    assert_eq!(
        selected.connection, initial.connection,
        "model preference must not rotate authorization"
    );
    let records = h.manager.list().await.unwrap();
    assert_eq!(records[0].revision, record.revision);
    assert_eq!(records[0].secret, record.secret);
    assert_eq!(records[0].settings.model, "beta");
    assert!(h
        .manager
        .select_model(record.id, "unknown".into())
        .await
        .is_err());
    assert_eq!(
        h.providers.0.diagnostic().unwrap().selection.unwrap().model,
        "beta"
    );
    let request = huginn_core::ModelRequest {
        messages: vec![],
        tools: vec![],
        max_tokens: 32,
    };
    assert!(pinned
        .stream(request.clone(), CancellationToken::new())
        .await
        .is_ok());
    // The adapter checks that factory material and the snapshot model agree.
    assert!(h.model.0.pin_turn(CancellationToken::new()).await.is_ok());
    h.manager.revoke(record.id).await.unwrap();
    assert!(pinned
        .stream(request, CancellationToken::new())
        .await
        .is_err());
    h.supervisor.shutdown().await;
    let h = open(store, secrets).await;
    assert!(
        !h.model.0.is_configured(),
        "revocation must survive restart"
    );
}

#[tokio::test]
async fn restart_recovers_model_preference_without_rotating_credentials() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets.clone()).await;
    let record = h
        .manager
        .create(settings(), secret(SENTINEL))
        .await
        .unwrap();
    h.manager
        .select_model(record.id, "beta".into())
        .await
        .unwrap();
    h.supervisor.shutdown().await;
    let h = open(store, secrets).await;
    assert_eq!(
        h.providers.0.diagnostic().unwrap().selection.unwrap().model,
        "beta"
    );
    assert!(h.model.0.pin_turn(CancellationToken::new()).await.is_ok());
}

#[tokio::test]
async fn legacy_loopback_http_stays_visible_and_revocable_without_becoming_usable() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Ephemeral);
    let mut legacy = settings();
    legacy.endpoint = Some("http://127.0.0.1:11434".into());
    legacy.requires_secret = false;
    legacy.allow_loopback_http = false; // serde(default) representation of old metadata
    let id = uuid::Uuid::new_v4();
    store.value.lock().unwrap().connections.insert(
        id,
        ConnectionRecord {
            id,
            revision: uuid::Uuid::new_v4(),
            settings: legacy,
            secret: None,
            mode: SecretMode::Ephemeral,
        },
    );
    let mut h = open(store.clone(), secrets).await;
    let records = h.manager.list().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, id);
    assert_eq!(
        h.manager.status(id).await,
        Err(ConnectionError::InvalidConfiguration)
    );
    assert_eq!(
        h.manager.select(id).await,
        Err(ConnectionError::InvalidConfiguration)
    );
    assert!(h.model.0.pin_turn(CancellationToken::new()).await.is_err());

    let mut repaired = records[0].settings.clone();
    repaired.allow_loopback_http = true;
    h.manager.update(id, repaired, None).await.unwrap();
    assert_eq!(h.manager.status(id).await, Ok(()));
    h.manager.revoke(id).await.unwrap();
    assert!(h.manager.list().await.unwrap().is_empty());
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn two_connections_restart_rotation_and_revocation_are_isolated() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets.clone()).await;
    let first = h
        .manager
        .create(settings(), secret(SENTINEL))
        .await
        .unwrap();
    let second = h
        .manager
        .create(settings(), secret("second-secret"))
        .await
        .unwrap();
    let initial = h.manager.select(first.id).await.unwrap();
    let pinned = h
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    h.manager.select(second.id).await.unwrap();
    assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
    assert_eq!(h.seen.lock().unwrap().as_slice(), [SENTINEL]);
    let rotated = h
        .manager
        .update(first.id, settings(), secret("rotated-secret"))
        .await
        .unwrap();
    assert_ne!(first.secret, rotated.secret);
    assert!(text(pinned.as_ref()).await.is_err());
    assert!(h
        .access
        .resolve(&initial.connection, CancellationToken::new())
        .await
        .is_err());
    assert!(secrets
        .value
        .lock()
        .unwrap()
        .get(first.secret.as_ref().unwrap())
        .is_none());
    assert_eq!(text(h.model.0.as_ref()).await.unwrap(), "response");
    assert_eq!(h.seen.lock().unwrap().last().unwrap(), "second-secret");
    let encoded = serde_json::to_string(&store.load().await.unwrap()).unwrap();
    assert!(!encoded.contains(SENTINEL));
    assert!(!format!("{:?}", h.manager.list().await.unwrap()).contains(SENTINEL));
    h.supervisor.shutdown().await;
    drop(h);
    let mut restarted = open(store.clone(), secrets.clone()).await;
    let resolved = restarted.manager.resolve(empty()).await.unwrap();
    assert_eq!(resolved.provenance, ConnectionProvenance::Persisted);
    assert_eq!(
        resolved.selection.unwrap().connection.id,
        second.id.to_string()
    );
    text(restarted.model.0.as_ref()).await.unwrap();
    assert_eq!(restarted.seen.lock().unwrap().as_slice(), ["second-secret"]);
    let pinned = restarted
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    restarted.manager.revoke(second.id).await.unwrap();
    assert!(text(pinned.as_ref()).await.is_err());
    assert_eq!(restarted.manager.list().await.unwrap(), vec![rotated]);
    restarted.supervisor.shutdown().await;
}

#[tokio::test]
async fn ephemeral_restart_and_locked_or_missing_credentials_are_not_ready() {
    for mode in [SecretMode::Ephemeral, SecretMode::Secure] {
        for error in [
            ConnectionError::MissingSecret,
            ConnectionError::Locked,
            ConnectionError::BackendUnavailable,
        ] {
            let store = Arc::new(Metadata::default());
            let secrets = Secrets::new(mode);
            let mut h = open(store.clone(), secrets.clone()).await;
            let record = h
                .manager
                .create(settings(), secret(SENTINEL))
                .await
                .unwrap();
            h.manager.select(record.id).await.unwrap();
            h.supervisor.shutdown().await;
            drop(h);
            let secrets = if mode == SecretMode::Ephemeral {
                Secrets::new(mode)
            } else {
                secrets
            };
            *secrets.read_error.lock().unwrap() = Some(error);
            let mut h = open(store, secrets).await;
            assert!(!h.model.0.is_configured());
            assert_eq!(h.manager.status(record.id).await, Err(error));
            assert!(h.manager.resolve(empty()).await.is_err());
            assert!(!format!("{error:?} {error}").contains(SENTINEL));
            h.supervisor.shutdown().await;
        }
    }
}

#[tokio::test]
async fn precedence_never_fills_an_invalid_explicit_connection() {
    let store = Arc::new(Metadata::default());
    let mut h = open(store.clone(), Secrets::new(SecretMode::Secure)).await;
    let a = h
        .manager
        .create(settings(), secret("a-secret"))
        .await
        .unwrap();
    let b = h
        .manager
        .create(settings(), secret("b-secret"))
        .await
        .unwrap();
    assert_eq!(
        h.manager.resolve(empty()).await.unwrap().provenance,
        ConnectionProvenance::Unconfigured
    );
    h.manager.select(a.id).await.unwrap();
    let pinned = h
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    for (cli, toml, environment, expected) in [
        (true, true, true, ConnectionProvenance::Cli),
        (false, true, true, ConnectionProvenance::Toml),
        (false, false, true, ConnectionProvenance::Environment),
        (false, false, false, ConnectionProvenance::Persisted),
    ] {
        let resolved = h
            .manager
            .resolve(ConnectionInputs {
                cli: cli.then_some(ConnectionChoice::Stored(b.id)),
                toml: toml.then_some(ConnectionChoice::Stored(b.id)),
                environment: environment.then_some(ConnectionChoice::Stored(b.id)),
            })
            .await
            .unwrap();
        assert_eq!(resolved.provenance, expected);
        assert_eq!(resolved.selection.unwrap().connection.id, b.id.to_string());
    }
    let before_explicit = h.manager.list().await.unwrap();
    let explicit = h
        .manager
        .resolve(ConnectionInputs {
            cli: Some(ConnectionChoice::Explicit {
                settings: settings(),
                secret: Some(Secret::new(SENTINEL.into()).unwrap()),
            }),
            toml: None,
            environment: None,
        })
        .await
        .unwrap();
    assert_eq!(explicit.provenance, ConnectionProvenance::Cli);
    let selection = explicit.selection.as_ref().unwrap();
    assert!(!h
        .manager
        .list()
        .await
        .unwrap()
        .iter()
        .any(|r| r.id.to_string() == selection.connection.id));
    assert_eq!(h.manager.list().await.unwrap(), before_explicit);
    assert_eq!(
        h.access
            .resolve(&selection.connection, CancellationToken::new())
            .await
            .unwrap()
            .secret
            .unwrap()
            .expose(),
        SENTINEL
    );
    // clear_selection through a failed explicit choice clears only new admission;
    // the turn pinned before clearing retains its original authorization.
    assert!(matches!(
        h.manager
            .resolve(ConnectionInputs {
                cli: Some(ConnectionChoice::Explicit {
                    settings: settings(),
                    secret: None,
                }),
                toml: None,
                environment: None,
            })
            .await,
        Err(ConnectionError::MissingSecret)
    ));
    let diagnostic = h
        .providers
        .0
        .execute(ProviderAction::Diagnose, CancellationToken::new())
        .await
        .unwrap();
    let ProviderActionOutput::Diagnostic(diagnostic) = diagnostic else {
        panic!("expected provider diagnostic")
    };
    assert!(diagnostic.selection.is_none());
    assert!(!diagnostic.ready);
    assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");

    for tier in 0..3 {
        let invalid = ConnectionChoice::Explicit {
            settings: settings(),
            secret: None,
        };
        let inputs = match tier {
            0 => ConnectionInputs {
                cli: Some(invalid),
                toml: Some(ConnectionChoice::Stored(a.id)),
                environment: Some(ConnectionChoice::Stored(b.id)),
            },
            1 => ConnectionInputs {
                cli: None,
                toml: Some(invalid),
                environment: Some(ConnectionChoice::Stored(b.id)),
            },
            _ => ConnectionInputs {
                cli: None,
                toml: None,
                environment: Some(invalid),
            },
        };
        assert!(matches!(
            h.manager.resolve(inputs).await,
            Err(ConnectionError::MissingSecret)
        ));
        assert!(text(h.model.0.as_ref()).await.is_err());
        h.manager.select(b.id).await.unwrap();
    }
    assert!(matches!(
        h.manager
            .resolve(ConnectionInputs {
                cli: Some(ConnectionChoice::Stored(uuid::Uuid::new_v4())),
                toml: None,
                environment: None
            })
            .await,
        Err(ConnectionError::InvalidSelection)
    ));
    assert!(text(h.model.0.as_ref()).await.is_err());
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn model_selection_second_secret_failure_is_recoverable_without_revoking_pins() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets.clone()).await;
    let record = h.manager.create(settings(), secret("a")).await.unwrap();
    let initial = h.manager.select(record.id).await.unwrap();
    let pinned = h
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    secrets.read_fail.store(2, Ordering::SeqCst);
    assert_eq!(
        h.manager.select_model(record.id, "beta".into()).await,
        Err(ConnectionError::SecretStore)
    );
    assert_eq!(
        store.load().await.unwrap().connections[&record.id]
            .settings
            .model,
        "beta"
    );
    assert!(
        text(h.model.0.as_ref()).await.is_err(),
        "stale alpha must not admit new turns"
    );
    assert_eq!(h.manager.list().await, Err(ConnectionError::Metadata));
    assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
    h.manager.recover().await.unwrap();
    let selected = h.providers.0.diagnostic().unwrap().selection.unwrap();
    assert_eq!(selected.model, "beta");
    assert_eq!(selected.connection, initial.connection);
    assert_eq!(h.manager.list().await.unwrap()[0].settings.model, "beta");
    assert_eq!(text(h.model.0.as_ref()).await.unwrap(), "response");
    assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn cancelled_model_selection_during_save_preserves_pins_and_recovers() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets).await;
    let record = h.manager.create(settings(), secret("a")).await.unwrap();
    h.manager.select(record.id).await.unwrap();
    let pinned = h
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();

    store.suspend.store(1, Ordering::SeqCst);
    let manager = h.manager.clone();
    let task = tokio::spawn(async move { manager.select_model(record.id, "beta".into()).await });
    store.entered.notified().await;
    assert!(h.providers.0.diagnostic().unwrap().selection.is_none());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    // Cancellation during durable save must not revoke a turn pinned before
    // the preference change, and recovery must reconcile the authoritative row.
    assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
    h.manager.recover().await.unwrap();
    assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
    assert_eq!(text(h.model.0.as_ref()).await.unwrap(), "response");
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn dropped_model_selection_second_read_preserves_pins_and_recovers() {
    for abort in [false, true] {
        let store = Arc::new(Metadata::default());
        let secrets = Secrets::new(SecretMode::Secure);
        let mut h = open(store.clone(), secrets.clone()).await;
        let record = h.manager.create(settings(), secret("a")).await.unwrap();
        let initial = h.manager.select(record.id).await.unwrap();
        let pinned = h
            .model
            .0
            .pin_turn(CancellationToken::new())
            .await
            .unwrap()
            .unwrap();
        secrets.read_suspend.store(2, Ordering::SeqCst);
        if abort {
            let manager = h.manager.clone();
            let task =
                tokio::spawn(async move { manager.select_model(record.id, "beta".into()).await });
            secrets.read_entered.notified().await;
            assert_eq!(
                store.load().await.unwrap().connections[&record.id]
                    .settings
                    .model,
                "beta"
            );
            assert!(h.providers.0.diagnostic().unwrap().selection.is_none());
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            let mut selecting = Box::pin(h.manager.select_model(record.id, "beta".into()));
            assert!(futures::poll!(selecting.as_mut()).is_pending());
            assert_eq!(
                store.load().await.unwrap().connections[&record.id]
                    .settings
                    .model,
                "beta"
            );
            assert!(h.providers.0.diagnostic().unwrap().selection.is_none());
            drop(selecting);
        }
        assert_eq!(h.manager.list().await, Err(ConnectionError::Metadata));
        assert!(text(h.model.0.as_ref()).await.is_err());
        assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
        // A dropped recovery remains pending and must not revoke the original pin.
        secrets.read_suspend.store(1, Ordering::SeqCst);
        let mut recovery = Box::pin(h.manager.recover());
        assert!(futures::poll!(recovery.as_mut()).is_pending());
        drop(recovery);
        assert_eq!(h.manager.list().await, Err(ConnectionError::Metadata));
        assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
        h.manager.recover().await.unwrap();
        let selected = h.providers.0.diagnostic().unwrap().selection.unwrap();
        assert_eq!(selected.model, "beta");
        assert_eq!(selected.connection, initial.connection);
        assert_eq!(text(h.model.0.as_ref()).await.unwrap(), "response");
        assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
        h.supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn retired_model_selection_cannot_publish_or_recover_after_second_read() {
    for replacement in [false, true] {
        let store = Arc::new(Metadata::default());
        let secrets = Secrets::new(SecretMode::Secure);
        let mut h = open(store.clone(), secrets.clone()).await;
        let record = h.manager.create(settings(), secret("a")).await.unwrap();
        h.manager.select(record.id).await.unwrap();
        h.supervisor
            .register(Box::new(ConnectionsPlugin(h.manager.clone())))
            .unwrap();
        h.supervisor.start().await.unwrap();
        secrets.read_suspend.store(2, Ordering::SeqCst);
        let mut selecting = Box::pin(h.manager.select_model(record.id, "beta".into()));
        assert!(futures::poll!(selecting.as_mut()).is_pending());
        h.supervisor
            .remove("builtin.providers.connections")
            .await
            .unwrap();
        secrets.read_release.notify_one();
        assert_eq!(selecting.await, Err(ConnectionError::Revoked));
        let current = if replacement {
            let manager = reopen(&h, store.clone()).await.unwrap();
            Some(
                manager
                    .select_model(record.id, "alpha".into())
                    .await
                    .unwrap(),
            )
        } else {
            None
        };
        assert_eq!(h.manager.recover().await, Err(ConnectionError::Revoked));
        assert_eq!(h.providers.0.diagnostic().unwrap().selection, current);
        assert_eq!(
            store.load().await.unwrap().connections[&record.id]
                .settings
                .model,
            if replacement { "alpha" } else { "beta" }
        );
        h.supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn failed_or_dropped_model_publication_closes_admission_and_retries() {
    for mode in [1, 2] {
        let store = Arc::new(Metadata::default());
        let mut h = open(store.clone(), Secrets::new(SecretMode::Secure)).await;
        let faults = Arc::new(PublicationFault {
            inner: h.providers.0.clone(),
            mode: AtomicUsize::new(0),
        });
        h.manager = Arc::new(
            ConnectionManager::open(
                store.clone(),
                h.access.clone(),
                faults.clone(),
                h.supervisor
                    .resolve::<ProviderCatalogService>(&catalog_id())
                    .unwrap()
                    .0
                    .clone(),
            )
            .await
            .unwrap(),
        );
        let record = h.manager.create(settings(), secret("a")).await.unwrap();
        let initial = h.manager.select(record.id).await.unwrap();
        let pinned = h
            .model
            .0
            .pin_turn(CancellationToken::new())
            .await
            .unwrap()
            .unwrap();
        faults.mode.store(mode, Ordering::SeqCst);
        let mut selecting = Box::pin(h.manager.select_model(record.id, "beta".into()));
        if mode == 1 {
            assert_eq!(selecting.await, Err(ConnectionError::InvalidSelection));
        } else {
            assert!(futures::poll!(selecting.as_mut()).is_pending());
            drop(selecting);
        }
        assert_eq!(
            store.load().await.unwrap().connections[&record.id]
                .settings
                .model,
            "beta"
        );
        assert!(h.providers.0.diagnostic().unwrap().selection.is_none());
        assert_eq!(h.manager.list().await, Err(ConnectionError::Metadata));
        assert!(text(h.model.0.as_ref()).await.is_err());
        assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
        h.manager.recover().await.unwrap();
        let selected = h.providers.0.diagnostic().unwrap().selection.unwrap();
        assert_eq!(selected.model, "beta");
        assert_eq!(selected.connection, initial.connection);
        assert_eq!(text(h.model.0.as_ref()).await.unwrap(), "response");
        assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
        h.supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn selection_second_secret_read_failure_closes_new_admission_only() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets.clone()).await;
    let a = h.manager.create(settings(), secret("a")).await.unwrap();
    let b = h.manager.create(settings(), secret("b")).await.unwrap();
    h.manager.select(a.id).await.unwrap();
    let pinned = h
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    secrets.read_fail.store(2, Ordering::SeqCst);
    assert!(h.manager.select(b.id).await.is_err());
    assert_eq!(store.load().await.unwrap().selected, Some(b.id));
    assert!(text(h.model.0.as_ref()).await.is_err());
    assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn cancelled_mutation_recovers_authoritative_state_on_same_manager() {
    for suspend in [1, 2] {
        cancelled_mutation_scenarios_for_suspend(suspend).await;
    }
}

async fn cancelled_mutation_scenarios_for_suspend(suspend: usize) {
    for updating in [false, true] {
        for step in 1..=if updating { 2 } else { 1 } {
            for abort in [false, true] {
                cancelled_mutation_scenario(suspend, updating, step, abort).await;
            }
        }
    }
}

async fn cancelled_mutation_scenario(suspend: usize, updating: bool, step: usize, abort: bool) {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets.clone()).await;
    let a = h.manager.create(settings(), secret("a")).await.unwrap();
    let b = h.manager.create(settings(), secret("b")).await.unwrap();
    let initial = h.manager.select(a.id).await.unwrap();
    let pinned = h
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    store.suspend.store(suspend, Ordering::SeqCst);
    store.suspend_at.store(step, Ordering::SeqCst);
    let manager = h.manager.clone();
    let mutation = async move {
        if updating {
            manager
                .update(a.id, settings(), secret("replacement"))
                .await
                .map(|_| ())
        } else {
            manager.select_model(b.id, "beta".into()).await.map(|_| ())
        }
    };
    if abort {
        let task = tokio::spawn(mutation);
        store.entered.notified().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    } else {
        let mut mutation = Box::pin(mutation);
        assert!(futures::poll!(mutation.as_mut()).is_pending());
    }
    assert_eq!(h.manager.list().await, Err(ConnectionError::Metadata));
    if !updating {
        assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
    } else {
        assert!(text(pinned.as_ref()).await.is_err());
    }
    assert!(text(h.model.0.as_ref()).await.is_err());
    let durable = store.load().await.unwrap();
    h.manager.recover().await.unwrap();
    assert_eq!(
        h.manager.list().await.unwrap(),
        durable.connections.values().cloned().collect::<Vec<_>>()
    );
    assert_eq!(store.load().await.unwrap().selected, durable.selected);
    let selected = h.providers.0.diagnostic().unwrap().selection;
    assert_eq!(
        selected.as_ref().map(|s| s.connection.id.clone()),
        durable.selected.map(|id| id.to_string())
    );
    if let Some(selected) = selected {
        if updating {
            assert_ne!(selected.connection, initial.connection);
        }
        text(h.model.0.as_ref()).await.unwrap();
    } else {
        assert!(text(h.model.0.as_ref()).await.is_err());
    }
    if updating {
        assert!(h
            .access
            .resolve(&initial.connection, CancellationToken::new())
            .await
            .is_err());
        assert!(text(pinned.as_ref()).await.is_err());
    } else {
        assert!(h
            .access
            .resolve(&initial.connection, CancellationToken::new())
            .await
            .is_ok());
        assert_eq!(text(pinned.as_ref()).await.unwrap(), "response");
    }
    assert!(store.load().await.unwrap().cleanup.is_empty());
    assert_eq!(
        secrets.value.lock().unwrap().len(),
        durable.connections.len()
    );
    h.manager.select(b.id).await.unwrap();
    let current = h.providers.0.diagnostic().unwrap().selection.unwrap();
    // Healthy recovery is idempotent, including runtime authorization.
    h.manager.recover().await.unwrap();
    assert_eq!(
        h.providers.0.diagnostic().unwrap().selection.unwrap(),
        current
    );
    h.manager.create(settings(), secret("new")).await.unwrap();
    assert_eq!(
        h.manager.list().await.unwrap().len(),
        durable.connections.len() + 1
    );
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn cancelled_recovery_is_serialized_fail_closed_and_retryable() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets.clone()).await;
    let a = h.manager.create(settings(), secret("a")).await.unwrap();
    let b = h.manager.create(settings(), secret("b")).await.unwrap();
    let initial = h.manager.select(a.id).await.unwrap();
    store.suspend.store(2, Ordering::SeqCst);
    let mut update = Box::pin(h.manager.update(a.id, settings(), secret("replacement")));
    assert!(futures::poll!(update.as_mut()).is_pending());
    drop(update);
    // Recovery cleanup has its own uncertain metadata write. A second recovery
    // cannot race it, and dropping the first must not publish a healthy state.
    store.suspend.store(2, Ordering::SeqCst);
    let mut recovery = Box::pin(h.manager.recover());
    assert!(futures::poll!(recovery.as_mut()).is_pending());
    let mut retry = Box::pin(h.manager.recover());
    assert!(futures::poll!(retry.as_mut()).is_pending());
    drop(recovery);
    drop(retry); // Remove the queued lock waiter before checking the failed state.
    assert_eq!(h.manager.list().await, Err(ConnectionError::Metadata));
    assert!(text(h.model.0.as_ref()).await.is_err());
    h.manager.recover().await.unwrap();
    assert_eq!(h.manager.list().await.unwrap(), vec![b.clone()]);
    assert!(h.providers.0.diagnostic().unwrap().selection.is_none());
    assert!(store.load().await.unwrap().cleanup.is_empty());
    assert_eq!(secrets.value.lock().unwrap().len(), 1);
    assert!(h
        .access
        .resolve(&initial.connection, CancellationToken::new())
        .await
        .is_err());
    h.manager.select(b.id).await.unwrap();
    text(h.model.0.as_ref()).await.unwrap();
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn failed_recovery_retires_partial_bindings_until_authoritative_retry() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets.clone()).await;
    let a = h.manager.create(settings(), secret("a")).await.unwrap();
    h.manager.select(a.id).await.unwrap();
    store.suspend.store(1, Ordering::SeqCst);
    let mut selection = Box::pin(h.manager.select(a.id));
    assert!(futures::poll!(selection.as_mut()).is_pending());
    drop(selection);
    store.value.lock().unwrap().selected = Some(uuid::Uuid::new_v4());
    assert_eq!(
        h.manager.recover().await,
        Err(ConnectionError::InvalidSelection)
    );
    assert_eq!(h.manager.list().await, Err(ConnectionError::Metadata));
    assert!(h.providers.0.diagnostic().unwrap().connections.is_empty());
    assert!(text(h.model.0.as_ref()).await.is_err());
    // Missing credentials must not cause recovery to select another connection.
    store.value.lock().unwrap().selected = Some(a.id);
    *secrets.read_error.lock().unwrap() = Some(ConnectionError::Locked);
    h.manager.recover().await.unwrap();
    assert_eq!(h.manager.list().await.unwrap(), vec![a.clone()]);
    assert!(h.providers.0.diagnostic().unwrap().selection.is_none());
    assert!(text(h.model.0.as_ref()).await.is_err());
    assert_eq!(h.manager.status(a.id).await, Err(ConnectionError::Locked));
    *secrets.read_error.lock().unwrap() = None;
    h.manager.select(a.id).await.unwrap();
    text(h.model.0.as_ref()).await.unwrap();
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn dropped_metadata_save_invalidates_and_reopens_on_same_activation() {
    for (suspend, abort) in [(1, false), (2, false), (1, true), (2, true)] {
        let store = Arc::new(Metadata::default());
        let secrets = Secrets::new(SecretMode::Secure);
        let mut h = open(store.clone(), secrets).await;
        let a = h.manager.create(settings(), secret("a")).await.unwrap();
        let b = h.manager.create(settings(), secret("b")).await.unwrap();
        let initial = h.manager.select(a.id).await.unwrap();
        let pinned = h
            .model
            .0
            .pin_turn(CancellationToken::new())
            .await
            .unwrap()
            .unwrap();
        store.suspend.store(suspend, Ordering::SeqCst);
        if abort {
            let manager = h.manager.clone();
            let task = tokio::spawn(async move { manager.select(b.id).await });
            store.entered.notified().await;
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            let mut save = Box::pin(h.manager.select(b.id));
            assert!(futures::poll!(save.as_mut()).is_pending());
        }
        assert!(h.manager.list().await.is_err());
        assert!(text(pinned.as_ref()).await.is_err());
        assert!(text(h.model.0.as_ref()).await.is_err());
        assert!(h
            .access
            .resolve(&initial.connection, CancellationToken::new())
            .await
            .is_err());
        let providers = h
            .supervisor
            .resolve::<ProvidersService>(&providers_id())
            .unwrap();
        let catalog = h
            .supervisor
            .resolve::<ProviderCatalogService>(&catalog_id())
            .unwrap();
        let reopened = ConnectionManager::open(
            store.clone(),
            h.access.clone(),
            providers.0.clone(),
            catalog.0.clone(),
        )
        .await
        .unwrap();
        let selected = reopened.resolve(empty()).await.unwrap().selection.unwrap();
        assert_eq!(
            selected.connection.id,
            if suspend == 1 { a.id } else { b.id }.to_string()
        );
        assert_ne!(selected.connection, initial.connection);
        assert_eq!(text(h.model.0.as_ref()).await.unwrap(), "response");
        assert!(text(pinned.as_ref()).await.is_err());
        h.supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn partial_write_failures_reconcile_without_deleting_other_connections() {
    // Every metadata write: intent, commit, cleanup; both before and after commit.
    for updating in [false, true] {
        for after in [false, true] {
            for step in 1..=if updating { 3 } else { 2 } {
                let store = Arc::new(Metadata::default());
                let secrets = Secrets::new(SecretMode::Secure);
                let mut h = open(store.clone(), secrets.clone()).await;
                let other = h
                    .manager
                    .create(settings(), secret("other-secret"))
                    .await
                    .unwrap();
                let old = h
                    .manager
                    .create(settings(), secret("old-secret"))
                    .await
                    .unwrap();
                h.manager.select(old.id).await.unwrap();
                let pinned = h
                    .model
                    .0
                    .pin_turn(CancellationToken::new())
                    .await
                    .unwrap()
                    .unwrap();
                if after {
                    store.after.store(step, Ordering::SeqCst);
                } else {
                    store.fail.store(step, Ordering::SeqCst);
                }
                let result = if updating {
                    h.manager.update(old.id, settings(), secret(SENTINEL)).await
                } else {
                    h.manager.create(settings(), secret(SENTINEL)).await
                };
                assert!(result.is_err());
                assert!(text(pinned.as_ref()).await.is_err());
                h.supervisor.shutdown().await;
                drop(h);
                let mut h = open(store.clone(), secrets.clone()).await;
                h.manager.select(other.id).await.unwrap();
                text(h.model.0.as_ref()).await.unwrap();
                assert_eq!(h.seen.lock().unwrap().as_slice(), ["other-secret"]);
                assert!(store.load().await.unwrap().cleanup.is_empty());
                assert!(!serde_json::to_string(&store.load().await.unwrap())
                    .unwrap()
                    .contains(SENTINEL));
                h.supervisor.shutdown().await;
            }
        }
    }
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets.clone()).await;
    let other = h
        .manager
        .create(settings(), secret("other-secret"))
        .await
        .unwrap();
    secrets.write_fail.store(1, Ordering::SeqCst);
    assert!(h
        .manager
        .create(settings(), secret(SENTINEL))
        .await
        .is_err());
    h.manager.reconcile().await.unwrap();
    assert_eq!(secrets.value.lock().unwrap().len(), 1);
    h.manager.select(other.id).await.unwrap();
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn failed_deletion_is_tombstoned_and_retried() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets.clone()).await;
    let a = h
        .manager
        .create(settings(), secret(SENTINEL))
        .await
        .unwrap();
    let b = h
        .manager
        .create(settings(), secret("b-secret"))
        .await
        .unwrap();
    h.manager.select(a.id).await.unwrap();
    let pinned = h
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    secrets.delete_fail.store(1, Ordering::SeqCst);
    assert_eq!(
        h.manager.revoke(a.id).await,
        Err(ConnectionError::SecretStore)
    );
    assert!(text(pinned.as_ref()).await.is_err());
    assert!(!store.load().await.unwrap().connections.contains_key(&a.id));
    assert!(store.load().await.unwrap().selected.is_none());
    h.supervisor.shutdown().await;
    drop(h);
    let mut h = open(store, secrets.clone()).await;
    assert_eq!(h.manager.list().await.unwrap(), vec![b]);
    assert_eq!(secrets.value.lock().unwrap().len(), 1);
    h.supervisor.shutdown().await;
}

fn no_auth(endpoint: &str) -> ConnectionSettings {
    ConnectionSettings {
        endpoint: Some(endpoint.into()),
        requires_secret: false,
        allow_loopback_http: true,
        ..settings()
    }
}

async fn reopen(h: &Harness, store: Arc<Metadata>) -> ConnectionResult<ConnectionManager> {
    ConnectionManager::open(
        store,
        h.access.clone(),
        h.supervisor
            .resolve::<ProvidersService>(&providers_id())
            .unwrap()
            .0
            .clone(),
        h.supervisor
            .resolve::<ProviderCatalogService>(&catalog_id())
            .unwrap()
            .0
            .clone(),
    )
    .await
}

#[tokio::test]
async fn mixed_missing_adapter_startup_recovers_usable_records() {
    let store = Arc::new(Metadata::default());
    let mut h = open(store.clone(), Secrets::new(SecretMode::Secure)).await;
    let usable = h
        .manager
        .create(settings(), secret("usable"))
        .await
        .unwrap();
    let missing = h
        .manager
        .create(settings(), secret("missing"))
        .await
        .unwrap();
    store
        .value
        .lock()
        .unwrap()
        .connections
        .get_mut(&missing.id)
        .unwrap()
        .settings
        .provider = "absent".into();
    let recovered = reopen(&h, store)
        .await
        .expect("missing adapter must not abort recovery");
    assert_eq!(recovered.list().await.unwrap().len(), 2);
    assert_eq!(
        recovered.status(missing.id).await,
        Err(ConnectionError::AdapterUnavailable)
    );
    recovered.select(usable.id).await.unwrap();
    assert_eq!(text(h.model.0.as_ref()).await.unwrap(), "response");
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn locked_cleanup_startup_keeps_unrelated_no_auth_usable() {
    let store = Arc::new(Metadata::default());
    let secrets = Secrets::new(SecretMode::Secure);
    let mut h = open(store.clone(), secrets.clone()).await;
    let usable = h
        .manager
        .create(no_auth("https://one.invalid"), None)
        .await
        .unwrap();
    let reference = SecretReference::new();
    store
        .value
        .lock()
        .unwrap()
        .cleanup
        .insert(reference.clone(), SecretMode::Secure);
    *secrets.delete_error.lock().unwrap() = Some(ConnectionError::Locked);
    let recovered = reopen(&h, store.clone())
        .await
        .expect("cleanup must not block diagnosis");
    assert_eq!(recovered.list().await.unwrap(), vec![usable.clone()]);
    recovered.status(usable.id).await.unwrap();
    assert!(store.load().await.unwrap().cleanup.contains_key(&reference));
    assert_eq!(
        recovered.cleanup_status().await.unwrap().get(&reference),
        Some(&ConnectionError::Locked)
    );
    recovered.select(usable.id).await.unwrap();
    text(h.model.0.as_ref()).await.unwrap();
    assert_eq!(recovered.reconcile().await, Err(ConnectionError::Locked));
    *secrets.delete_error.lock().unwrap() = Some(ConnectionError::BackendUnavailable);
    assert_eq!(
        recovered.reconcile().await,
        Err(ConnectionError::BackendUnavailable)
    );
    let another = recovered
        .create(no_auth("https://two.invalid"), None)
        .await
        .unwrap();
    recovered.select(another.id).await.unwrap();
    text(h.model.0.as_ref()).await.unwrap();
    *secrets.delete_error.lock().unwrap() = None;
    recovered.reconcile().await.unwrap();
    assert!(recovered.cleanup_status().await.unwrap().is_empty());
    assert!(store.load().await.unwrap().cleanup.is_empty());
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn no_auth_mode_changes_and_explicit_endpoints_are_honored() {
    for mode in [SecretMode::Secure, SecretMode::Ephemeral] {
        let store = Arc::new(Metadata::default());
        let mut h = open(store.clone(), Secrets::new(mode)).await;
        for endpoint in ["https://one.invalid", "https://two.invalid"] {
            let record = h.manager.create(no_auth(endpoint), None).await.unwrap();
            store
                .value
                .lock()
                .unwrap()
                .connections
                .get_mut(&record.id)
                .unwrap()
                .mode = if mode == SecretMode::Secure {
                SecretMode::Ephemeral
            } else {
                SecretMode::Secure
            };
            let recovered = reopen(&h, store.clone()).await.unwrap();
            recovered.status(record.id).await.unwrap();
            recovered.select(record.id).await.unwrap();
            assert_eq!(text(h.model.0.as_ref()).await.unwrap(), "response");
            assert_eq!(h.seen.lock().unwrap().last().unwrap(), endpoint);
            h.manager = Arc::new(recovered);
        }
        h.supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn legacy_factory_rejects_even_no_auth_connection_material() {
    let adapter = Adapter {
        id: "secure",
        model: Arc::new(TextModel("wrong target")),
    };
    let identity = ConnectionIdentity {
        id: uuid::Uuid::new_v4().to_string(),
        adapter: ProviderRegistration::new("secure".into(), 1),
        revision: 1,
    };
    let selection = ModelSelection {
        connection: identity,
        model: "alpha".into(),
    };
    for endpoint in ["https://one.invalid", "https://two.invalid"] {
        assert!(adapter
            .create_connection_model(
                &selection,
                ConnectionMaterial {
                    settings: no_auth(endpoint),
                    secret: None
                },
                CancellationToken::new()
            )
            .await
            .is_err());
    }
}

#[tokio::test]
async fn healthy_reopen_retires_all_old_manager_operations() {
    let store = Arc::new(Metadata::default());
    let mut h = open(store.clone(), Secrets::new(SecretMode::Secure)).await;
    let a = h.manager.create(settings(), secret("a")).await.unwrap();
    let other = h.manager.create(settings(), secret("other")).await.unwrap();
    let initial = h.manager.select(a.id).await.unwrap();
    let pinned = h
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    let replacement = reopen(&h, store.clone()).await.unwrap();
    replacement
        .update(a.id, settings(), secret("replacement"))
        .await
        .unwrap();
    let selected = replacement.select(a.id).await.unwrap();
    let before = store.load().await.unwrap();
    assert_eq!(h.manager.revoke(a.id).await, Err(ConnectionError::Revoked));
    assert_eq!(
        h.manager.update(a.id, settings(), secret("stale")).await,
        Err(ConnectionError::Revoked)
    );
    assert_eq!(
        h.manager.select(other.id).await,
        Err(ConnectionError::Revoked)
    );
    assert!(matches!(
        h.manager.resolve(empty()).await,
        Err(ConnectionError::Revoked)
    ));
    assert_eq!(h.manager.reconcile().await, Err(ConnectionError::Revoked));
    assert_eq!(
        h.manager.status(other.id).await,
        Err(ConnectionError::Revoked)
    );
    assert_eq!(h.manager.list().await, Err(ConnectionError::Revoked));
    assert_eq!(h.manager.recover().await, Err(ConnectionError::Revoked));
    assert_eq!(store.load().await.unwrap(), before);
    h.access
        .resolve(&selected.connection, CancellationToken::new())
        .await
        .unwrap();
    assert!(h
        .access
        .resolve(&initial.connection, CancellationToken::new())
        .await
        .is_err());
    assert!(text(pinned.as_ref()).await.is_err());
    assert_eq!(text(h.model.0.as_ref()).await.unwrap(), "response");
    replacement.select(other.id).await.unwrap();
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn fatal_open_retires_partial_runtime_and_retry_is_clean() {
    let store = Arc::new(Metadata::default());
    let mut h = open(store.clone(), Secrets::new(SecretMode::Secure)).await;
    let record = h.manager.create(settings(), secret("a")).await.unwrap();
    h.manager.select(record.id).await.unwrap();
    let pinned = h
        .model
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    store.value.lock().unwrap().selected = Some(uuid::Uuid::new_v4());
    assert!(matches!(
        reopen(&h, store.clone()).await,
        Err(ConnectionError::InvalidSelection)
    ));
    let providers = h
        .supervisor
        .resolve::<ProvidersService>(&providers_id())
        .unwrap();
    assert!(providers.0.diagnostic().unwrap().connections.is_empty());
    assert!(text(h.model.0.as_ref()).await.is_err());
    assert!(text(pinned.as_ref()).await.is_err());
    store.value.lock().unwrap().selected = Some(record.id);
    let recovered = reopen(&h, store).await.unwrap();
    recovered.status(record.id).await.unwrap();
    text(h.model.0.as_ref()).await.unwrap();
    assert!(text(pinned.as_ref()).await.is_err());
    h.supervisor.shutdown().await;
}

#[tokio::test]
async fn reopen_waits_for_mutation_and_cancelled_owner_cannot_invalidate_replacement() {
    let store = Arc::new(Metadata::default());
    let mut h = open(store.clone(), Secrets::new(SecretMode::Secure)).await;
    let record = h.manager.create(settings(), secret("a")).await.unwrap();
    store.suspend.store(2, Ordering::SeqCst);
    let mut mutation = Box::pin(h.manager.select(record.id));
    assert!(futures::poll!(mutation.as_mut()).is_pending());
    let mut opening = Box::pin(reopen(&h, store.clone()));
    assert!(futures::poll!(opening.as_mut()).is_pending());
    drop(mutation);
    let recovered = opening.await.unwrap();
    let selected = recovered.resolve(empty()).await.unwrap().selection.unwrap();
    assert_eq!(
        h.manager.revoke(record.id).await,
        Err(ConnectionError::Revoked)
    );
    h.access
        .resolve(&selected.connection, CancellationToken::new())
        .await
        .unwrap();
    text(h.model.0.as_ref()).await.unwrap();
    h.supervisor.shutdown().await;
}
