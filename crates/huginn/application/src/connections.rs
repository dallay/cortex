//! Serialized headless management with durable write intents and revision-bound access.
use async_trait::async_trait;
use huginn_core::{connections::*, providers::*, CancellationToken};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

struct Binding {
    identity: ConnectionIdentity,
    record: ConnectionRecord,
}
/// Shared by the manager and the only model router. An identity cannot adopt a new secret.
pub struct ConnectionAccess {
    secrets: Arc<dyn SecretStore>,
    bindings: Mutex<BTreeMap<String, Binding>>,
    runtime_secrets: Mutex<BTreeMap<String, Secret>>,
    serial: AsyncMutex<()>,
    epoch: AtomicU64,
}
impl ConnectionAccess {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Arc<Self> {
        Arc::new(Self {
            secrets,
            bindings: Mutex::new(BTreeMap::new()),
            runtime_secrets: Mutex::new(BTreeMap::new()),
            serial: AsyncMutex::new(()),
            epoch: AtomicU64::new(0),
        })
    }
    fn bind(&self, identity: ConnectionIdentity, record: ConnectionRecord) -> ConnectionResult<()> {
        self.bindings
            .lock()
            .map_err(|_| ConnectionError::Revoked)?
            .insert(identity.id.clone(), Binding { identity, record });
        Ok(())
    }
    fn bind_runtime(
        &self,
        identity: ConnectionIdentity,
        settings: ConnectionSettings,
        secret: Option<Secret>,
    ) -> ConnectionResult<()> {
        let record = ConnectionRecord {
            id: Uuid::parse_str(&identity.id).map_err(|_| ConnectionError::InvalidSelection)?,
            revision: Uuid::new_v4(),
            settings,
            secret: None,
            mode: self.secrets.mode(),
        };
        if let Some(secret) = secret {
            self.runtime_secrets
                .lock()
                .map_err(|_| ConnectionError::SecretStore)?
                .insert(identity.id.clone(), secret);
        }
        self.bindings
            .lock()
            .map_err(|_| ConnectionError::Revoked)?
            .insert(identity.id.clone(), Binding { identity, record });
        Ok(())
    }
    fn unbind(&self, id: Uuid) {
        self.runtime_secrets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id.to_string());
        self.bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id.to_string());
    }
    fn identity(&self, id: Uuid) -> ConnectionResult<ConnectionIdentity> {
        self.bindings
            .lock()
            .map_err(|_| ConnectionError::Revoked)?
            .get(&id.to_string())
            .map(|b| b.identity.clone())
            .ok_or(ConnectionError::Revoked)
    }
    // Keep binding retirement and runtime invalidation serialized with rebinding.
    #[allow(clippy::significant_drop_tightening)]
    fn invalidate_all(&self, providers: &dyn ProviderManagement, owner: u64) {
        if self.epoch.load(Ordering::SeqCst) != owner {
            return;
        }
        let mut bindings = self
            .bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.runtime_secrets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        for binding in std::mem::take(&mut *bindings).into_values() {
            // Exact identities cannot revoke a replacement generation on recovery.
            let _ = providers.invalidate(&binding.identity);
        }
        let _ = providers.clear_selection();
    }
}
#[async_trait]
impl ConnectionCredentials for ConnectionAccess {
    async fn resolve(
        &self,
        identity: &ConnectionIdentity,
        cancel: CancellationToken,
    ) -> ConnectionResult<ConnectionMaterial> {
        if cancel.is_cancelled() {
            return Err(ConnectionError::Cancelled);
        }
        let record = self
            .bindings
            .lock()
            .map_err(|_| ConnectionError::Revoked)?
            .get(&identity.id)
            .filter(|b| b.identity == *identity)
            .map(|b| b.record.clone())
            .ok_or(ConnectionError::Revoked)?;
        if record.secret.is_some() && record.mode != self.secrets.mode() {
            return Err(ConnectionError::MissingSecret);
        }
        let secret = match &record.secret {
            Some(reference) => Some(tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(ConnectionError::Cancelled),
                result = self.secrets.read(reference) => result?,
            }),
            None if record.settings.requires_secret => Some(Secret::new(
                self.runtime_secrets
                    .lock()
                    .map_err(|_| ConnectionError::SecretStore)?
                    .get(&identity.id)
                    .ok_or(ConnectionError::MissingSecret)?
                    .expose()
                    .to_owned(),
            )?),
            None => None,
        };
        // Secret I/O may race rotation: recheck the exact runtime identity afterwards.
        let bound = self.bindings.lock().map_err(|_| ConnectionError::Revoked)?;
        if cancel.is_cancelled()
            || !bound
                .get(&identity.id)
                .is_some_and(|b| b.identity == *identity)
        {
            return Err(ConnectionError::Revoked);
        }
        drop(bound);
        Ok(ConnectionMaterial {
            settings: record.settings,
            secret,
        })
    }
}

struct State {
    metadata: ConnectionMetadata,
    failed: bool,
    cleanup_status: BTreeMap<SecretReference, ConnectionError>,
}
struct MetadataMutation<'a> {
    access: &'a ConnectionAccess,
    providers: &'a dyn ProviderManagement,
    armed: bool,
    owner: u64,
}
impl Drop for MetadataMutation<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.access.invalidate_all(self.providers, self.owner);
        }
    }
}
pub struct ConnectionManager {
    store: Arc<dyn ConnectionStore>,
    access: Arc<ConnectionAccess>,
    providers: Arc<dyn ProviderManagement>,
    catalog: Arc<dyn ProviderCatalog>,
    state: AsyncMutex<State>,
    owner: u64,
}
pub struct ResolvedConnection {
    pub selection: Option<ModelSelection>,
    pub provenance: ConnectionProvenance,
}
impl ConnectionManager {
    /// Open only after the adapter catalog has activated. Missing credentials remain
    /// visible in metadata but never become router-ready.
    pub async fn open(
        store: Arc<dyn ConnectionStore>,
        access: Arc<ConnectionAccess>,
        providers: Arc<dyn ProviderManagement>,
        catalog: Arc<dyn ProviderCatalog>,
    ) -> ConnectionResult<Self> {
        let serial = access.serial.lock().await;
        let owner = access.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        access.invalidate_all(providers.as_ref(), owner);
        let mut opening = MetadataMutation {
            access: &access,
            providers: providers.as_ref(),
            armed: true,
            owner,
        };
        let metadata = store.load().await?;
        let mut references = std::collections::BTreeSet::new();
        for (id, record) in &metadata.connections {
            if *id != record.id || record.settings.requires_secret != record.secret.is_some() {
                return Err(ConnectionError::Metadata);
            }
            record.settings.validate()?;
            if let Some(reference) = &record.secret {
                if !references.insert(reference.clone()) || metadata.cleanup.contains_key(reference)
                {
                    return Err(ConnectionError::Metadata);
                }
            }
        }
        let manager = Self {
            store,
            access: access.clone(),
            providers: providers.clone(),
            catalog,
            state: AsyncMutex::new(State {
                metadata,
                failed: false,
                cleanup_status: BTreeMap::new(),
            }),
            owner,
        };
        let mut state = manager.state.lock().await;
        // Reopening on the same activation must retire stale bindings before installing
        // the authoritative records, including records removed by an uncertain write.
        if let Err(error) = manager.reconcile_locked(&mut state).await {
            if !state.cleanup_status.values().any(|e| *e == error) || state.failed {
                return Err(error);
            }
        }
        for record in state.metadata.connections.values() {
            record.settings.validate()?;
            // A locked/missing backend is not silently downgraded. Diagnosis can still list records.
            if manager.check_ready(record).await.is_ok() {
                // Adapter availability is per record, not a fatal metadata error.
                match manager.install(record).await {
                    Ok(_) | Err(ConnectionError::AdapterUnavailable) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        if let Some(id) = state.metadata.selected {
            if !state.metadata.connections.contains_key(&id) {
                return Err(ConnectionError::InvalidSelection);
            }
            if manager.access.identity(id).is_ok() {
                manager
                    .select_runtime(&state.metadata.connections[&id])
                    .await?;
            }
        }
        drop(state);
        opening.armed = false;
        drop(opening);
        drop(serial);
        Ok(manager)
    }
    async fn operation(&self) -> ConnectionResult<tokio::sync::MutexGuard<'_, ()>> {
        let guard = self.access.serial.lock().await;
        if self.access.epoch.load(Ordering::SeqCst) != self.owner {
            return Err(ConnectionError::Revoked);
        }
        Ok(guard)
    }
    const fn check_state(state: &State) -> ConnectionResult<()> {
        if state.failed {
            Err(ConnectionError::Metadata)
        } else {
            Ok(())
        }
    }
    async fn persist(
        &self,
        state: &mut State,
        mut next: ConnectionMetadata,
    ) -> ConnectionResult<()> {
        next.version = state
            .metadata
            .version
            .checked_add(1)
            .ok_or(ConnectionError::Metadata)?;
        // If this future is dropped during I/O, stale in-memory metadata must not
        // perform another mutation. Reopening reads the authoritative document.
        state.failed = true;
        let mut mutation = MetadataMutation {
            access: &self.access,
            providers: self.providers.as_ref(),
            armed: true,
            owner: self.owner,
        };
        // The state lock stays held through the I/O and guard drop: mutation and
        // authoritative recovery must remain serialized, even on cancellation.
        self.store.save(state.metadata.version, &next).await?;
        state.metadata = next;
        state.failed = false;
        mutation.armed = false;
        Ok(())
    }
    async fn check_secret(&self, record: &ConnectionRecord) -> ConnectionResult<()> {
        if record.secret.is_some() && record.mode != self.access.secrets.mode() {
            return Err(ConnectionError::MissingSecret);
        }
        match &record.secret {
            Some(reference) => {
                self.access.secrets.read(reference).await?;
                Ok(())
            }
            None if record.settings.requires_secret => Err(ConnectionError::MissingSecret),
            None => Ok(()),
        }
    }
    async fn check_ready(&self, record: &ConnectionRecord) -> ConnectionResult<()> {
        if !self
            .catalog
            .adapters()
            .iter()
            .any(|(r, _)| r.provider == record.settings.provider)
        {
            return Err(ConnectionError::AdapterUnavailable);
        }
        self.check_secret(record).await
    }
    async fn install(&self, record: &ConnectionRecord) -> ConnectionResult<ConnectionIdentity> {
        let adapter = self
            .catalog
            .adapters()
            .into_iter()
            .find(|(r, _)| r.provider == record.settings.provider)
            .ok_or(ConnectionError::AdapterUnavailable)?
            .0;
        let output = self
            .providers
            .execute(
                ProviderAction::Connect {
                    id: record.id.to_string(),
                    adapter,
                },
                CancellationToken::new(),
            )
            .await
            .map_err(|_| ConnectionError::AdapterUnavailable)?;
        let ProviderActionOutput::Connected(identity) = output else {
            return Err(ConnectionError::AdapterUnavailable);
        };
        self.access.bind(identity.clone(), record.clone())?;
        Ok(identity)
    }
    async fn invalidate(&self, id: Uuid) {
        if let Ok(identity) = self.access.identity(id) {
            self.access.unbind(id);
            // Already-revoked runtime handles are harmless; this never revokes another ID.
            let _ = self
                .providers
                .execute(ProviderAction::Revoke(identity), CancellationToken::new())
                .await;
        }
    }
    async fn select_runtime(&self, record: &ConnectionRecord) -> ConnectionResult<ModelSelection> {
        self.check_secret(record).await?;
        let identity = self.access.identity(record.id)?;
        let selection = ModelSelection {
            connection: identity,
            model: record.settings.model.clone(),
        };
        self.providers
            .execute(
                ProviderAction::Select(selection.clone()),
                CancellationToken::new(),
            )
            .await
            .map_err(|_| ConnectionError::InvalidSelection)?;
        Ok(selection)
    }
    pub async fn list(&self) -> ConnectionResult<Vec<ConnectionRecord>> {
        let _operation = self.operation().await?;
        let state = self.state.lock().await;
        Self::check_state(&state)?;
        Ok(state.metadata.connections.values().cloned().collect())
    }
    pub async fn status(&self, id: Uuid) -> ConnectionResult<()> {
        let _operation = self.operation().await?;
        let state = self.state.lock().await;
        Self::check_state(&state)?;
        self.check_ready(
            state
                .metadata
                .connections
                .get(&id)
                .ok_or(ConnectionError::InvalidSelection)?,
        )
        .await
    }
    pub async fn create(
        &self,
        settings: ConnectionSettings,
        secret: Option<Secret>,
    ) -> ConnectionResult<ConnectionRecord> {
        let _operation = self.operation().await?;
        self.write_connection(None, settings, secret).await
    }
    pub async fn update(
        &self,
        id: Uuid,
        settings: ConnectionSettings,
        secret: Option<Secret>,
    ) -> ConnectionResult<ConnectionRecord> {
        let _operation = self.operation().await?;
        self.write_connection(Some(id), settings, secret).await
    }
    // The lock serializes the entire mutation, including installation and cleanup.
    #[allow(clippy::significant_drop_tightening)]
    async fn write_connection(
        &self,
        previous: Option<Uuid>,
        settings: ConnectionSettings,
        secret: Option<Secret>,
    ) -> ConnectionResult<ConnectionRecord> {
        settings.validate()?;
        if settings.requires_secret != secret.is_some() {
            return Err(ConnectionError::MissingSecret);
        }
        let descriptor = self
            .catalog
            .adapters()
            .into_iter()
            .find(|(r, _)| r.provider == settings.provider)
            .ok_or(ConnectionError::AdapterUnavailable)?
            .1;
        if !descriptor.models.iter().any(|m| m.id == settings.model) {
            return Err(ConnectionError::InvalidConfiguration);
        }
        let authentication = if settings.requires_secret {
            AuthenticationMethod::ApiKey
        } else {
            AuthenticationMethod::None
        };
        if !descriptor.authentication.contains(&authentication) {
            return Err(ConnectionError::InvalidConfiguration);
        }
        let mut state = self.state.lock().await;
        Self::check_state(&state)?;
        let id = previous.unwrap_or_else(Uuid::new_v4);
        let old = match previous {
            Some(id) => Some(
                state
                    .metadata
                    .connections
                    .get(&id)
                    .cloned()
                    .ok_or(ConnectionError::InvalidSelection)?,
            ),
            None => None,
        };
        let record = ConnectionRecord {
            id,
            revision: Uuid::new_v4(),
            settings,
            secret: secret.as_ref().map(|_| SecretReference::new()),
            mode: self.access.secrets.mode(),
        };
        let mut intent = state.metadata.clone();
        if let Some(old) = &old {
            intent.connections.remove(&old.id);
            if intent.selected == Some(old.id) {
                intent.selected = None;
            }
            if let Some(reference) = &old.secret {
                intent.cleanup.insert(reference.clone(), old.mode);
            }
        }
        if let Some(reference) = &record.secret {
            intent.cleanup.insert(reference.clone(), record.mode);
        }
        // Revocation precedes awaited durable intent: even a failed rotation cannot retain old turns.
        if old.is_some() {
            self.invalidate(id).await;
        }
        self.persist(&mut state, intent).await?;
        if let (Some(reference), Some(secret)) = (&record.secret, &secret) {
            // Cancellation/drop leaves a durable orphan intent for the next open/reconcile.
            self.access.secrets.write(reference, secret).await?;
        }
        let mut commit = state.metadata.clone();
        if let Some(reference) = &record.secret {
            commit.cleanup.remove(reference);
        }
        commit.connections.insert(id, record.clone());
        self.persist(&mut state, commit).await?;
        self.install(&record).await?;
        // Selection must be explicit after rotation; old turns never adopt replacements.
        if let Err(error) = self.reconcile_locked(&mut state).await {
            if state.failed || !state.cleanup_status.values().any(|e| *e == error) {
                return Err(error);
            }
        }
        Ok(record)
    }
    // Keep persistence and runtime selection in the same serialized transition.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn select(&self, id: Uuid) -> ConnectionResult<ModelSelection> {
        let _operation = self.operation().await?;
        self.select_locked(id).await
    }
    #[allow(clippy::significant_drop_tightening)]
    async fn select_locked(&self, id: Uuid) -> ConnectionResult<ModelSelection> {
        let mut state = self.state.lock().await;
        Self::check_state(&state)?;
        self.providers
            .clear_selection()
            .map_err(|_| ConnectionError::InvalidSelection)?;
        let record = state
            .metadata
            .connections
            .get(&id)
            .cloned()
            .ok_or(ConnectionError::InvalidSelection)?;
        self.check_secret(&record).await?;
        if self.access.identity(id).is_err() {
            self.install(&record).await?;
        }
        let mut next = state.metadata.clone();
        next.selected = Some(id);
        self.persist(&mut state, next).await?;
        self.select_runtime(&record).await
    }
    // Durable revocation and cleanup are one serialized metadata mutation.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn revoke(&self, id: Uuid) -> ConnectionResult<()> {
        let _operation = self.operation().await?;
        let mut state = self.state.lock().await;
        Self::check_state(&state)?;
        let record = state
            .metadata
            .connections
            .get(&id)
            .cloned()
            .ok_or(ConnectionError::InvalidSelection)?;
        self.invalidate(id).await;
        let mut next = state.metadata.clone();
        next.connections.remove(&id);
        if next.selected == Some(id) {
            next.selected = None;
        }
        if let Some(reference) = record.secret {
            next.cleanup.insert(reference, record.mode);
        }
        self.persist(&mut state, next).await?;
        self.reconcile_locked(&mut state).await
    }
    pub async fn reconcile(&self) -> ConnectionResult<()> {
        let _operation = self.operation().await?;
        let mut state = self.state.lock().await;
        Self::check_state(&state)?;
        self.reconcile_locked(&mut state).await
    }
    async fn reconcile_locked(&self, state: &mut State) -> ConnectionResult<()> {
        let mut next = state.metadata.clone();
        state.cleanup_status.clear();
        for (reference, mode) in &state.metadata.cleanup {
            // Never garbage-collect a live reference, including corrupted metadata.
            if state
                .metadata
                .connections
                .values()
                .any(|r| r.secret.as_ref() == Some(reference))
            {
                return Err(ConnectionError::Metadata);
            }
            if *mode == self.access.secrets.mode() {
                if let Err(error) = self.access.secrets.delete(reference).await {
                    state.cleanup_status.insert(reference.clone(), error);
                    continue;
                }
            } else if *mode == SecretMode::Secure {
                state
                    .cleanup_status
                    .insert(reference.clone(), ConnectionError::BackendUnavailable);
                continue;
            }
            next.cleanup.remove(reference);
        }
        if next != state.metadata {
            self.persist(state, next).await?;
        }
        state
            .cleanup_status
            .values()
            .next()
            .copied()
            .map_or(Ok(()), Err)
    }
    /// Pending cleanup failures, without exposing credential bytes. Reconcile retries them.
    pub async fn cleanup_status(
        &self,
    ) -> ConnectionResult<BTreeMap<SecretReference, ConnectionError>> {
        let _operation = self.operation().await?;
        let state = self.state.lock().await;
        Self::check_state(&state)?;
        Ok(state.cleanup_status.clone())
    }
    pub async fn resolve(&self, inputs: ConnectionInputs) -> ConnectionResult<ResolvedConnection> {
        let _operation = self.operation().await?;
        if let Some((choice, provenance)) = inputs.highest() {
            let id = match choice {
                ConnectionChoice::Stored(id) => id,
                ConnectionChoice::Explicit { settings, secret } => {
                    // Clear before validation/I/O too: an invalid winner must never
                    // leave a lower-precedence connection usable for new turns.
                    let state = self.state.lock().await;
                    Self::check_state(&state)?;
                    self.providers
                        .clear_selection()
                        .map_err(|_| ConnectionError::InvalidSelection)?;
                    drop(state);
                    let id = Uuid::new_v4();
                    let record = ConnectionRecord {
                        id,
                        revision: Uuid::new_v4(),
                        mode: self.access.secrets.mode(),
                        secret: None,
                        settings: settings.clone(),
                    };
                    settings.validate()?;
                    if settings.requires_secret != secret.is_some() {
                        return Err(ConnectionError::MissingSecret);
                    }
                    let identity = self.install(&record).await?;
                    self.access
                        .bind_runtime(identity.clone(), settings.clone(), secret)?;
                    let selection = ModelSelection {
                        connection: identity,
                        model: settings.model,
                    };
                    self.providers
                        .execute(
                            ProviderAction::Select(selection.clone()),
                            CancellationToken::new(),
                        )
                        .await
                        .map_err(|_| ConnectionError::InvalidSelection)?;
                    return Ok(ResolvedConnection {
                        selection: Some(selection),
                        provenance,
                    });
                }
            };
            return Ok(ResolvedConnection {
                selection: Some(self.select_locked(id).await?),
                provenance,
            });
        }
        let selected = {
            let state = self.state.lock().await;
            Self::check_state(&state)?;
            state.metadata.selected
        };
        match selected {
            Some(id) => Ok(ResolvedConnection {
                selection: Some(self.select_locked(id).await?),
                provenance: ConnectionProvenance::Persisted,
            }),
            None => Ok(ResolvedConnection {
                selection: None,
                provenance: ConnectionProvenance::Unconfigured,
            }),
        }
    }
}
