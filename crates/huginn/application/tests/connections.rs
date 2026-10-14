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
    entered: tokio::sync::Notify,
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
        let suspend = self.suspend.swap(0, Ordering::SeqCst);
        if suspend == 1 {
            self.entered.notify_one();
            std::future::pending::<()>().await;
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
            models: vec![ModelDescriptor {
                id: "alpha".into(),
                supports_tools: true,
            }],
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
