//! Lifecycle-owned provider catalog and turn-scoped routing, without terminal I/O.
use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_core::{
    kernel::{Activation, Manifest, Plugin, PluginContext, ServiceId},
    providers::*,
    AgentError, CancellationToken, ModelDelta, ModelProvider, ModelRequest, ModelService, Result,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard, Weak},
};

pub fn providers_id() -> ServiceId {
    ServiceId::new("agent:providers", 1)
}
pub fn catalog_id() -> ServiceId {
    ServiceId::new("agent:provider-catalog", 1)
}

struct AdapterEntry {
    registration: ProviderRegistration,
    descriptor: ProviderDescriptor,
    implementation: Arc<dyn ProviderAdapter>,
    activation: Activation,
    revoked: CancellationToken,
}
impl AdapterEntry {
    fn active(&self) -> bool {
        self.activation.is_active() && !self.revoked.is_cancelled()
    }
}
struct Connection {
    identity: ConnectionIdentity,
    adapter: Arc<AdapterEntry>,
    revoked: CancellationToken,
}
impl Connection {
    fn active(&self) -> bool {
        self.adapter.active() && !self.revoked.is_cancelled()
    }
}
#[derive(Default)]
struct State {
    adapters: BTreeMap<String, Arc<AdapterEntry>>,
    connections: BTreeMap<String, Arc<Connection>>,
    selected: Option<(Arc<Connection>, String)>,
    revision: u64,
    request_id: u64,
    requests: BTreeMap<u64, (Arc<Connection>, CancellationToken)>,
}
impl State {
    fn revoke_connection(&self, connection: &Arc<Connection>) {
        connection.revoked.cancel();
        for (entry, cancel) in self.requests.values() {
            if Arc::ptr_eq(entry, connection) {
                cancel.cancel();
            }
        }
    }
    fn revoke_adapter(&self, adapter: &Arc<AdapterEntry>) {
        adapter.revoked.cancel();
        for (entry, cancel) in self.requests.values() {
            if Arc::ptr_eq(&entry.adapter, adapter) {
                cancel.cancel();
            }
        }
    }
}
struct CoreLease(Arc<Mutex<State>>);
impl Drop for CoreLease {
    fn drop(&mut self) {
        let state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (_, cancel) in state.requests.values() {
            cancel.cancel();
        }
    }
}

/// A service handle is scoped to the core plugin's activation, including actions.
pub struct Providers {
    state: Arc<Mutex<State>>,
    activation: Activation,
    cancellation: CancellationToken,
}
struct RegistrationLease {
    state: Weak<Mutex<State>>,
    entry: Arc<AdapterEntry>,
}
impl Drop for RegistrationLease {
    fn drop(&mut self) {
        self.entry.revoked.cancel();
        if let Some(state) = self.state.upgrade() {
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.revoke_adapter(&self.entry);
            if state
                .adapters
                .get(&self.entry.registration.provider)
                .is_some_and(|entry| Arc::ptr_eq(entry, &self.entry))
            {
                state.adapters.remove(&self.entry.registration.provider);
            }
        }
    }
}
fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
impl Providers {
    /// Linearization point shared with Revoke/Replace and registration cleanup.
    /// A request admitted before revocation is in-flight, even before backend polling.
    fn admit(
        &self,
        connection: Arc<Connection>,
        cancel: &CancellationToken,
    ) -> Result<RequestLifetime> {
        let mut state = self.lock()?;
        self.check()?;
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        if !connection.active() {
            return Err(ProviderError::Revoked.into());
        }
        state.request_id = state
            .request_id
            .checked_add(1)
            .ok_or(ProviderError::Unavailable)?;
        let id = state.request_id;
        let token = cancel.child_token();
        state.requests.insert(id, (connection, token.clone()));
        drop(state);
        Ok(RequestLifetime {
            state: Arc::downgrade(&self.state),
            id,
            token,
        })
    }
    fn lock(&self) -> Result<MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| ProviderError::Unavailable.into())
    }
    fn check(&self) -> Result<()> {
        if self.activation.is_active() {
            Ok(())
        } else {
            Err(ProviderError::Revoked.into())
        }
    }
    fn adapter(state: &State, handle: &ProviderRegistration) -> Result<Arc<AdapterEntry>> {
        state
            .adapters
            .get(&handle.provider)
            .filter(|entry| entry.registration == *handle && entry.active())
            .cloned()
            .ok_or_else(|| ProviderError::Unavailable.into())
    }
    fn connection(state: &State, identity: &ConnectionIdentity) -> Result<Arc<Connection>> {
        state
            .connections
            .get(&identity.id)
            .filter(|entry| entry.identity == *identity)
            .cloned()
            .ok_or_else(|| ProviderError::Revoked.into())
    }
    fn connect(
        state: &mut State,
        id: String,
        adapter: Arc<AdapterEntry>,
    ) -> Result<ConnectionIdentity> {
        if !identifier(&id) {
            return Err(ProviderError::InvalidMetadata.into());
        }
        if state.connections.contains_key(&id) {
            return Err(ProviderError::Duplicate.into());
        }
        state.revision = state
            .revision
            .checked_add(1)
            .ok_or(ProviderError::Unavailable)?;
        let identity = ConnectionIdentity {
            id: id.clone(),
            adapter: adapter.registration.clone(),
            revision: state.revision,
        };
        state.connections.insert(
            id,
            Arc::new(Connection {
                identity: identity.clone(),
                adapter,
                revoked: CancellationToken::new(),
            }),
        );
        Ok(identity)
    }
    fn snapshot(&self) -> Result<(Arc<Connection>, ModelSelection)> {
        self.check()?;
        let state = self.lock()?;
        let (connection, model) = state.selected.clone().ok_or(ProviderError::Unconfigured)?;
        drop(state);
        if !connection.active() {
            return Err(ProviderError::Revoked.into());
        }
        Ok((
            connection.clone(),
            ModelSelection {
                connection: connection.identity.clone(),
                model,
            },
        ))
    }
}
impl ProviderCatalog for Providers {
    fn adapters(&self) -> Vec<(ProviderRegistration, ProviderDescriptor)> {
        if self.check().is_err() {
            return vec![];
        }
        self.lock()
            .map(|state| {
                state
                    .adapters
                    .values()
                    .filter(|entry| entry.active())
                    .map(|entry| (entry.registration.clone(), entry.descriptor.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
    fn register(
        &self,
        ctx: &mut PluginContext,
        adapter: Arc<dyn ProviderAdapter>,
    ) -> Result<ProviderRegistration> {
        self.check()?;
        let declared = ctx.resolve::<ProviderCatalogService>(&catalog_id())?;
        // Compare allocation identity, not trait-object vtables, which may be
        // duplicated across compilation units even for the same concrete type.
        if !std::ptr::addr_eq(declared.0.as_ref(), self as &dyn ProviderCatalog) {
            return Err(ProviderError::Unavailable.into());
        }
        self.stage_registration(ctx, adapter)
    }
}
impl Providers {
    fn stage_registration(
        &self,
        ctx: &mut PluginContext,
        adapter: Arc<dyn ProviderAdapter>,
    ) -> Result<ProviderRegistration> {
        // Core activation may stage its legacy adapter before its own publication.
        if self.cancellation.is_cancelled() {
            return Err(ProviderError::Revoked.into());
        }
        let descriptor = adapter.descriptor();
        if !identifier(&descriptor.id)
            || !identifier(&descriptor.display_name)
            || descriptor.models.is_empty()
            || descriptor.models.len() > 1024
            || descriptor.models.iter().any(|model| !identifier(&model.id))
            || descriptor
                .models
                .iter()
                .map(|model| &model.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != descriptor.models.len()
        {
            return Err(ProviderError::InvalidMetadata.into());
        }
        let mut state = self.lock()?;
        if state.adapters.contains_key(&descriptor.id) {
            return Err(ProviderError::Duplicate.into());
        }
        let registration = ProviderRegistration::new(descriptor.id.clone(), ctx.generation());
        let entry = Arc::new(AdapterEntry {
            registration: registration.clone(),
            descriptor,
            implementation: adapter,
            activation: ctx.activation(),
            revoked: ctx.cancellation.child_token(),
        });
        state
            .adapters
            .insert(registration.provider.clone(), entry.clone());
        drop(state);
        ctx.own_resource(RegistrationLease {
            state: Arc::downgrade(&self.state),
            entry: entry.clone(),
        });
        // Self-cancellation also relays to unpolled producers. The existing kernel
        // owns this generation task and joins/aborts it on every activation exit.
        let state = Arc::downgrade(&self.state);
        ctx.own_task(tokio::spawn(async move {
            entry.revoked.cancelled().await;
            if let Some(state) = state.upgrade() {
                state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .revoke_adapter(&entry);
            }
        }));
        // Activation::is_active is the commit gate; no earlier visibility flag.
        Ok(registration)
    }
}
#[async_trait]
impl ProviderManagement for Providers {
    fn diagnostic(&self) -> Result<ProvidersDiagnostic> {
        self.check()?;
        let state = self.lock()?;
        Ok(ProvidersDiagnostic {
            adapters: state
                .adapters
                .values()
                .filter(|entry| entry.active())
                .map(|entry| (entry.registration.clone(), entry.descriptor.clone()))
                .collect(),
            connections: state
                .connections
                .values()
                .filter(|entry| entry.active())
                .map(|entry| entry.identity.clone())
                .collect(),
            selection: state
                .selected
                .as_ref()
                .map(|(entry, model)| ModelSelection {
                    connection: entry.identity.clone(),
                    model: model.clone(),
                }),
            ready: state
                .selected
                .as_ref()
                .is_some_and(|(entry, _)| entry.active()),
        })
    }
    async fn execute(
        &self,
        action: ProviderAction,
        cancel: CancellationToken,
    ) -> Result<ProviderActionOutput> {
        self.check()?;
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        if action == ProviderAction::Diagnose {
            return self.diagnostic().map(ProviderActionOutput::Diagnostic);
        }
        let mut state = self.lock()?;
        // Recheck after acquiring the serialization point; no I/O or await in the critical section.
        self.check()?;
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let output = match action {
            ProviderAction::Diagnose => unreachable!(),
            ProviderAction::Connect { id, adapter } => {
                let adapter = Self::adapter(&state, &adapter)?;
                Self::connect(&mut state, id, adapter).map(ProviderActionOutput::Connected)
            }
            ProviderAction::Select(selection) => {
                let connection = Self::connection(&state, &selection.connection)?;
                if !connection.active() {
                    return Err(ProviderError::Revoked.into());
                }
                if !connection
                    .adapter
                    .descriptor
                    .models
                    .iter()
                    .any(|model| model.id == selection.model)
                {
                    return Err(ProviderError::InvalidModel.into());
                }
                state.selected = Some((connection, selection.model.clone()));
                Ok(ProviderActionOutput::Selected(selection))
            }
            ProviderAction::Revoke(identity) => {
                let connection = Self::connection(&state, &identity)?;
                state.revoke_connection(&connection);
                state.connections.remove(&identity.id);
                // Retain the selected identity as revoked, never silently substitute it.
                Ok(ProviderActionOutput::Revoked)
            }
            ProviderAction::Replace {
                connection: identity,
                adapter,
            } => {
                let old = Self::connection(&state, &identity)?;
                let adapter = Self::adapter(&state, &adapter)?;
                // Validate all fallible work before invalidating the old authorization.
                state
                    .revision
                    .checked_add(1)
                    .ok_or(ProviderError::Unavailable)?;
                state.revoke_connection(&old);
                state.connections.remove(&identity.id);
                Self::connect(&mut state, identity.id, adapter).map(ProviderActionOutput::Connected)
            }
        };
        drop(state);
        output
    }
}

pub struct ModelRouter(pub Arc<Providers>);
#[derive(Clone)]
struct PinnedModel {
    providers: Arc<Providers>,
    connection: Arc<Connection>,
    inner: Arc<dyn ModelProvider>,
}
struct RequestLifetime {
    state: Weak<Mutex<State>>,
    id: u64,
    token: CancellationToken,
}
impl Drop for RequestLifetime {
    fn drop(&mut self) {
        self.token.cancel();
        if let Some(state) = self.state.upgrade() {
            state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .requests
                .remove(&self.id);
        }
    }
}
impl PinnedModel {
    fn check(&self) -> Result<()> {
        self.providers.check()?;
        if self.connection.active() {
            Ok(())
        } else {
            Err(ProviderError::Revoked.into())
        }
    }
}
#[async_trait]
impl ModelProvider for ModelRouter {
    fn is_configured(&self) -> bool {
        self.0.snapshot().is_ok()
    }
    async fn pin_turn(&self, cancel: CancellationToken) -> Result<Option<Arc<dyn ModelProvider>>> {
        // Snapshot before the first await, not after a factory or instructions load.
        let (connection, selection) = self.0.snapshot()?;
        let lifetime = self.0.admit(connection.clone(), &cancel)?;
        let inner = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            _ = self.0.cancellation.cancelled() => return Err(ProviderError::Revoked.into()),
            _ = connection.revoked.cancelled() => return Err(ProviderError::Revoked.into()),
            _ = connection.adapter.revoked.cancelled() => return Err(ProviderError::Revoked.into()),
            result = connection.adapter.implementation.create_model(&selection, lifetime.token.clone()) => result?,
        };
        let pinned = PinnedModel {
            providers: self.0.clone(),
            connection,
            inner,
        };
        pinned.check()?;
        Ok(Some(Arc::new(pinned)))
    }
    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        self.pin_turn(cancel.clone())
            .await?
            .ok_or(ProviderError::Unconfigured)?
            .stream(request, cancel)
            .await
    }
}
#[async_trait]
impl ModelProvider for PinnedModel {
    async fn pin_turn(&self, _: CancellationToken) -> Result<Option<Arc<dyn ModelProvider>>> {
        self.check()?;
        Ok(Some(Arc::new(self.clone())))
    }
    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        let lifetime = self.providers.admit(self.connection.clone(), &cancel)?;
        let stream = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            _ = self.providers.cancellation.cancelled() => return Err(ProviderError::Revoked.into()),
            _ = self.connection.revoked.cancelled() => return Err(ProviderError::Revoked.into()),
            _ = self.connection.adapter.revoked.cancelled() => return Err(ProviderError::Revoked.into()),
            result = self.inner.stream(request, lifetime.token.clone()) => result?,
        };
        self.check()?;
        let pinned = self.clone();
        Ok(futures::stream::unfold((stream, lifetime, pinned, cancel, false),
            |(mut stream, lifetime, pinned, cancel, ended)| async move {
                if ended { return None; }
                let item = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => Some(Err(AgentError::Cancelled)),
                    _ = pinned.providers.cancellation.cancelled() => Some(Err(ProviderError::Revoked.into())),
                    _ = pinned.connection.revoked.cancelled() => Some(Err(ProviderError::Revoked.into())),
                    _ = pinned.connection.adapter.revoked.cancelled() => Some(Err(ProviderError::Revoked.into())),
                    result = stream.next() => result,
                }?;
                let item = pinned.check().and(item);
                let ended = item.is_err();
                if ended { lifetime.token.cancel(); }
                Some((item, (stream, lifetime, pinned, cancel, ended)))
            }).boxed())
    }
}

/// The only model service publisher. Legacy builders seed one ephemeral connection.
#[derive(Default)]
pub struct ProvidersPlugin {
    legacy: Option<Arc<dyn ModelProvider>>,
}
impl ProvidersPlugin {
    pub fn with_legacy(model: Arc<dyn ModelProvider>) -> Self {
        Self {
            legacy: Some(model),
        }
    }
}
struct LegacyAdapter(Arc<dyn ModelProvider>);
#[async_trait]
impl ProviderAdapter for LegacyAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "legacy".into(),
            display_name: "Configured provider".into(),
            models: vec![ModelDescriptor {
                id: "configured".into(),
                supports_tools: true,
            }],
            authentication: vec![],
        }
    }
    async fn create_model(
        &self,
        _: &ModelSelection,
        cancel: CancellationToken,
    ) -> Result<Arc<dyn ModelProvider>> {
        Ok(self
            .0
            .pin_turn(cancel)
            .await?
            .unwrap_or_else(|| self.0.clone()))
    }
}
#[async_trait]
impl Plugin for ProvidersPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "builtin.providers.core".into(),
            provides: vec![crate::composition::model_id(), providers_id(), catalog_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let providers = Arc::new(Providers {
            state: Arc::new(Mutex::new(State::default())),
            activation: ctx.activation(),
            cancellation: ctx.cancellation.clone(),
        });
        ctx.own_resource(CoreLease(providers.state.clone()));
        if let Some(model) = self.legacy.as_ref().filter(|model| model.is_configured()) {
            let handle =
                providers.stage_registration(ctx, Arc::new(LegacyAdapter(model.clone())))?;
            let mut state = providers.lock()?;
            let adapter = state.adapters[&handle.provider].clone();
            let identity = Providers::connect(&mut state, "legacy".into(), adapter)?;
            state.selected = Some((state.connections[&identity.id].clone(), "configured".into()));
        }
        ctx.provide(catalog_id(), ProviderCatalogService(providers.clone()))?;
        ctx.provide(providers_id(), ProvidersService(providers.clone()))?;
        ctx.provide(
            crate::composition::model_id(),
            ModelService(Arc::new(ModelRouter(providers))),
        )
    }
}
