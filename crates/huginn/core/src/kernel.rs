//! Minimal transactional service composition and lifecycle ownership.
use crate::{AgentError, CancellationToken, Result};
use async_trait::async_trait;
use serde::Serialize;
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ServiceId(pub String);
impl ServiceId {
    pub fn new(name: &str, major: u32) -> Self {
        Self(format!("{name}@{major}"))
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Manifest {
    pub id: String,
    pub provides: Vec<ServiceId>,
    pub requires: Vec<ServiceId>,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum State {
    Discovered,
    Blocked,
    Active,
    Failed,
    Unloaded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstitutionPolicy {
    ReportUndrained,
    RequireFullDrain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRecord {
    pub plugin_id: String,
    pub generation: u64,
}

#[derive(Debug, Clone)]
pub struct SubstitutionDiagnostic {
    pub service: ServiceId,
    pub previous_owner: String,
    pub alternative_owner: String,
    pub outcome: std::result::Result<(), String>,
    pub drained: Vec<String>,
    pub undrained: Vec<String>,
}

struct SubstitutionAbort<'a> {
    service: &'a ServiceId,
    previous_owner: &'a str,
    alternative_id: &'a str,
    gates: &'a [Arc<LeaseGate>],
    resume_ids: &'a [String],
    drained: Vec<String>,
    undrained: Vec<String>,
    error: AgentError,
}
#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub manifest: Manifest,
    pub state: State,
    pub generation: u64,
    pub last_error: Option<String>,
}
type Services = BTreeMap<ServiceId, Arc<dyn Any + Send + Sync>>;

#[derive(Default)]
struct ServiceSlots {
    entries: BTreeMap<ServiceId, ServiceSlot>,
}

struct ServiceSlot {
    value: Arc<dyn Any + Send + Sync>,
    gate: Arc<LeaseGate>,
}

/// Controls admission and tracks active operations for one provider generation.
pub struct LeaseGate {
    generation: u64,
    state: Mutex<LeaseState>,
    idle: tokio::sync::Notify,
}

struct LeaseState {
    open: bool,
    active: usize,
    revoked: bool,
}

impl LeaseGate {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            state: Mutex::new(LeaseState {
                open: true,
                active: 0,
                revoked: false,
            }),
            idle: tokio::sync::Notify::new(),
        }
    }

    fn acquire(self: &Arc<Self>) -> Result<LeaseGuard> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AgentError::Composition("lease gate poisoned".into()))?;
        if !state.open || state.revoked {
            return Err(AgentError::Composition(format!(
                "service generation {} is not accepting leases",
                self.generation
            )));
        }
        state.active = state
            .active
            .checked_add(1)
            .ok_or_else(|| AgentError::Composition("service lease counter overflow".into()))?;
        drop(state);
        Ok(LeaseGuard(self.clone()))
    }

    fn close(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AgentError::Composition("lease gate poisoned".into()))?;
        state.open = false;
        drop(state);
        Ok(())
    }

    fn reopen(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AgentError::Composition("lease gate poisoned".into()))?;
        if !state.revoked {
            state.open = true;
        }
        drop(state);
        Ok(())
    }

    fn revoke(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AgentError::Composition("lease gate poisoned".into()))?;
        state.open = false;
        state.revoked = true;
        drop(state);
        Ok(())
    }

    async fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.idle.notified();
            if self
                .state
                .lock()
                .map(|state| state.active == 0)
                .unwrap_or(false)
            {
                return true;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return false;
            }
        }
    }
}

fn close_gates(service_slots: &Mutex<ServiceSlots>, gates: &[Arc<LeaseGate>]) -> Result<()> {
    let _slots = service_slots
        .lock()
        .map_err(|_| AgentError::Composition("service slot table poisoned".into()))?;
    let mut closed = Vec::new();
    for gate in gates {
        if let Err(error) = gate.close() {
            let reopen_errors: Vec<_> = closed
                .iter()
                .filter_map(|gate: &&Arc<LeaseGate>| gate.reopen().err())
                .map(|error| error.to_string())
                .collect();
            if reopen_errors.is_empty() {
                return Err(error);
            }
            return Err(AgentError::Composition(format!(
                "{error}; failed to restore closed lease gates: {}",
                reopen_errors.join(", ")
            )));
        }
        closed.push(gate);
    }
    Ok(())
}

fn reopen_gates(service_slots: &Mutex<ServiceSlots>, gates: &[Arc<LeaseGate>]) -> Result<()> {
    let _slots = service_slots
        .lock()
        .map_err(|_| AgentError::Composition("service slot table poisoned".into()))?;
    let errors: Vec<_> = gates
        .iter()
        .filter_map(|gate| gate.reopen().err())
        .map(|error| error.to_string())
        .collect();
    if !errors.is_empty() {
        return Err(AgentError::Composition(format!(
            "failed to reopen lease gates: {}",
            errors.join(", ")
        )));
    }
    Ok(())
}

async fn wait_gates_idle(gates: &[Arc<LeaseGate>], timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    for gate in gates {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() || !gate.wait_idle(remaining).await {
            return false;
        }
    }
    true
}

struct LeaseGuard(Arc<LeaseGate>);
impl Drop for LeaseGuard {
    fn drop(&mut self) {
        let became_idle = self.0.state.lock().is_ok_and(|mut state| {
            state.active = state.active.saturating_sub(1);
            state.active == 0
        });
        if became_idle {
            self.0.idle.notify_one();
        }
    }
}

/// A handle identifies a provider generation but does not itself grant access.
/// Use [`ServiceHandle::acquire`] for each operation that must participate in
/// substitution quiescence.
pub struct ServiceHandle<T: Any + Send + Sync> {
    value: Arc<T>,
    gate: Arc<LeaseGate>,
}
impl<T: Any + Send + Sync> Clone for ServiceHandle<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            gate: self.gate.clone(),
        }
    }
}
impl<T: Any + Send + Sync> ServiceHandle<T> {
    pub fn acquire(&self) -> Result<ServiceLease<T>> {
        Ok(ServiceLease {
            value: self.value.clone(),
            _guard: self.gate.acquire()?,
        })
    }
    pub fn generation(&self) -> u64 {
        self.gate.generation
    }
}

/// A lease keeps one service operation within the provider generation's lifetime.
///
/// This contract is cooperative: do not clone or retain provider references obtained
/// through the service after dropping the lease.
pub struct ServiceLease<T: Any + Send + Sync> {
    value: Arc<T>,
    _guard: LeaseGuard,
}
impl<T: Any + Send + Sync> Deref for ServiceLease<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

/// A dynamic handle resolves the current provider for one service ID on every acquisition.
///
/// Unlike [`ServiceHandle`], this handle does not retain a provider generation. A lease
/// acquired before a substitution keeps its provider until the lease is dropped; a later
/// acquisition observes the committed provider.
pub struct ServiceSlotHandle<T: Any + Send + Sync> {
    service_slots: Arc<Mutex<ServiceSlots>>,
    service: ServiceId,
    marker: std::marker::PhantomData<fn() -> T>,
}

impl<T: Any + Send + Sync> Clone for ServiceSlotHandle<T> {
    fn clone(&self) -> Self {
        Self {
            service_slots: self.service_slots.clone(),
            service: self.service.clone(),
            marker: std::marker::PhantomData,
        }
    }
}

impl<T: Any + Send + Sync> ServiceSlotHandle<T> {
    pub fn acquire(&self) -> Result<ServiceLease<T>> {
        let slots = self
            .service_slots
            .lock()
            .map_err(|_| AgentError::Composition("service slot table poisoned".into()))?;
        let slot = slots.entries.get(&self.service).ok_or_else(|| {
            AgentError::Composition(format!("unavailable service slot {}", self.service.0))
        })?;
        let value = slot.value.clone().downcast::<T>().map_err(|_| {
            AgentError::Composition(format!("incompatible service slot {}", self.service.0))
        })?;
        let guard = slot.gate.acquire()?;
        drop(slots);
        Ok(ServiceLease {
            value,
            _guard: guard,
        })
    }
}

/// Revocable generation identity. Effects are invisible until activation commits.
#[derive(Clone)]
pub struct Activation {
    generation: u64,
    published: Arc<AtomicBool>,
    cancellation: CancellationToken,
}
impl Activation {
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    pub fn is_active(&self) -> bool {
        self.published.load(Ordering::Acquire) && !self.cancellation.is_cancelled()
    }
}

pub struct PluginContext {
    generation: u64,
    services: Services,
    required_services: BTreeSet<ServiceId>,
    service_slots: Arc<Mutex<ServiceSlots>>,
    staged: Services,
    pub cancellation: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
    published: Arc<AtomicBool>,
    resources: Vec<Box<dyn Any + Send + Sync>>,
    commits: Vec<Box<dyn FnOnce() + Send + Sync>>,
}
impl PluginContext {
    fn new(
        generation: u64,
        available_services: &Services,
        manifest: &Manifest,
        service_slots: Arc<Mutex<ServiceSlots>>,
    ) -> Self {
        let required_services: BTreeSet<_> = manifest.requires.iter().cloned().collect();
        let services = available_services
            .iter()
            .filter(|(service, _)| required_services.contains(*service))
            .map(|(service, value)| (service.clone(), value.clone()))
            .collect();

        Self {
            generation,
            services,
            required_services,
            service_slots,
            staged: BTreeMap::new(),
            cancellation: CancellationToken::new(),
            tasks: vec![],
            published: Arc::new(AtomicBool::new(false)),
            resources: vec![],
            commits: vec![],
        }
    }

    pub fn activation(&self) -> Activation {
        Activation {
            generation: self.generation,
            published: self.published.clone(),
            cancellation: self.cancellation.clone(),
        }
    }
    /// Supervisor-assigned identity for generation-owned registrations.
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    pub fn resolve<T: Any + Send + Sync>(&self, id: &ServiceId) -> Result<Arc<T>> {
        self.services
            .get(id)
            .cloned()
            .and_then(|v| v.downcast().ok())
            .ok_or_else(|| {
                AgentError::Composition(format!("missing or incompatible service {}", id.0))
            })
    }
    /// Create a dynamic handle for a service declared in this plugin's manifest.
    /// The handle resolves the current provider each time it acquires a lease.
    pub fn service_slot_handle<T: Any + Send + Sync>(
        &self,
        id: &ServiceId,
    ) -> Result<ServiceSlotHandle<T>> {
        if !self.required_services.contains(id) {
            return Err(AgentError::Composition(format!(
                "plugin requested undeclared service slot {}",
                id.0
            )));
        }

        let slots = self
            .service_slots
            .lock()
            .map_err(|_| AgentError::Composition("service slot table poisoned".into()))?;
        let slot = slots
            .entries
            .get(id)
            .ok_or_else(|| AgentError::Composition(format!("unavailable service slot {}", id.0)))?;
        if !slot.value.is::<T>() {
            return Err(AgentError::Composition(format!(
                "incompatible service slot {}",
                id.0
            )));
        }
        drop(slots);

        Ok(ServiceSlotHandle {
            service_slots: self.service_slots.clone(),
            service: id.clone(),
            marker: std::marker::PhantomData,
        })
    }
    pub fn provide<T: Any + Send + Sync>(&mut self, id: ServiceId, value: T) -> Result<()> {
        if self.staged.contains_key(&id) {
            return Err(AgentError::Composition(format!(
                "duplicate registration {}",
                id.0
            )));
        }
        self.staged.insert(id, Arc::new(value));
        Ok(())
    }
    pub fn own_task(&mut self, task: JoinHandle<()>) {
        self.tasks.push(task);
    }
    /// Dispose an owned registration even when activation fails before deactivate.
    pub fn own_resource<T: Any + Send + Sync>(&mut self, resource: T) {
        self.resources.push(Box::new(resource));
    }
    /// Publish adapter registrations only after activation and manifest validation succeed.
    pub fn on_commit(&mut self, publish: impl FnOnce() + Send + Sync + 'static) {
        self.commits.push(Box::new(publish));
    }
    async fn cleanup(&mut self) {
        self.cancellation.cancel();
        self.commits.clear();
        self.resources.clear();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        for mut task in self.tasks.drain(..) {
            if tokio::time::timeout_at(deadline, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
        self.staged.clear();
        self.services.clear();
    }
}
impl Drop for PluginContext {
    fn drop(&mut self) {
        self.cancellation.cancel();
        for task in &self.tasks {
            task.abort();
        }
    }
}
#[async_trait]
pub trait Plugin: Send + Sync {
    fn manifest(&self) -> Manifest;
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()>;
    async fn deactivate(&mut self) -> Result<()> {
        Ok(())
    }
    /// Returns whether this plugin implements work-drain hooks.
    fn supports_drain(&self) -> bool {
        false
    }
    /// Stop admitting plugin work that is not already covered by service leases.
    /// The supervisor applies the substitution deadline to this hook.
    async fn drain(&mut self) -> Result<()> {
        Ok(())
    }
    /// Resume plugin work after an aborted or committed substitution.
    async fn resume(&mut self) -> Result<()> {
        Ok(())
    }
}
struct Entry {
    plugin: Box<dyn Plugin>,
    diagnostic: Diagnostic,
    context: Option<PluginContext>,
}
#[derive(Default)]
pub struct Supervisor {
    plugins: BTreeMap<String, Entry>,
    services: Services,
    service_slots: Arc<Mutex<ServiceSlots>>,
    lease_gates: BTreeMap<ServiceId, Arc<LeaseGate>>,
    providers: BTreeMap<ServiceId, ProviderRecord>,
    dependents: BTreeMap<ServiceId, BTreeSet<String>>,
    substitutions: Vec<SubstitutionDiagnostic>,
    order: Vec<String>,
    generation: u64,
}
impl Supervisor {
    pub fn register(&mut self, plugin: Box<dyn Plugin>) -> Result<()> {
        let manifest = plugin.manifest();
        if manifest.id.is_empty() || self.plugins.contains_key(&manifest.id) {
            return Err(AgentError::Composition(
                "empty or duplicate plugin id".into(),
            ));
        }
        let diagnostic = Diagnostic {
            manifest: manifest.clone(),
            state: State::Discovered,
            generation: 0,
            last_error: None,
        };
        self.plugins.insert(
            manifest.id,
            Entry {
                plugin,
                diagnostic,
                context: None,
            },
        );
        Ok(())
    }
    pub fn diagnostics(&self) -> Vec<&Diagnostic> {
        self.plugins.values().map(|e| &e.diagnostic).collect()
    }
    pub fn resolve<T: Any + Send + Sync>(&self, id: &ServiceId) -> Result<Arc<T>> {
        self.services
            .get(id)
            .cloned()
            .and_then(|v| v.downcast().ok())
            .ok_or_else(|| AgentError::Composition(format!("unavailable service {}", id.0)))
    }
    pub fn provider_of(&self, service: &ServiceId) -> Option<&ProviderRecord> {
        self.providers.get(service)
    }
    pub fn dependents_of(&self, service: &ServiceId) -> Vec<&str> {
        self.dependents
            .get(service)
            .map(|ids| ids.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }
    pub fn substitution_diagnostics(&self) -> &[SubstitutionDiagnostic] {
        &self.substitutions
    }
    pub fn service_handle<T: Any + Send + Sync>(&self, id: &ServiceId) -> Result<ServiceHandle<T>> {
        let value = self.resolve::<T>(id)?;
        let gate = self.lease_gates.get(id).cloned().ok_or_else(|| {
            AgentError::Composition(format!("missing lease gate for service {}", id.0))
        })?;
        Ok(ServiceHandle { value, gate })
    }
    pub fn service_slot_handle<T: Any + Send + Sync>(
        &self,
        id: &ServiceId,
    ) -> Result<ServiceSlotHandle<T>> {
        let slots = self
            .service_slots
            .lock()
            .map_err(|_| AgentError::Composition("service slot table poisoned".into()))?;
        let slot = slots
            .entries
            .get(id)
            .ok_or_else(|| AgentError::Composition(format!("unavailable service slot {}", id.0)))?;
        if !slot.value.is::<T>() {
            return Err(AgentError::Composition(format!(
                "incompatible service slot {}",
                id.0
            )));
        }
        drop(slots);
        Ok(ServiceSlotHandle {
            service_slots: self.service_slots.clone(),
            service: id.clone(),
            marker: std::marker::PhantomData,
        })
    }
    pub fn resolve_lease<T: Any + Send + Sync>(&self, id: &ServiceId) -> Result<ServiceLease<T>> {
        self.service_handle::<T>(id)?.acquire()
    }
    pub async fn substitute(
        &mut self,
        service: &ServiceId,
        alternative: Box<dyn Plugin>,
        policy: SubstitutionPolicy,
        timeout: Duration,
    ) -> Result<()> {
        let previous_owner = self
            .plugins
            .iter()
            .find(|(_, entry)| {
                entry.diagnostic.state == State::Active
                    && entry.diagnostic.manifest.provides.contains(service)
            })
            .map(|(id, _)| id.clone())
            .ok_or_else(|| {
                AgentError::Composition(format!("no active provider for {}", service.0))
            })?;
        let manifest = alternative.manifest();
        if manifest.id.is_empty()
            || !manifest.provides.contains(service)
            || self.plugins.contains_key(&manifest.id)
        {
            return Err(AgentError::Composition(
                "invalid or duplicate alternative plugin".into(),
            ));
        }
        let previous_provides: BTreeSet<ServiceId> = self
            .plugins
            .get(&previous_owner)
            .expect("previous owner exists")
            .diagnostic
            .manifest
            .provides
            .iter()
            .cloned()
            .collect();
        let alternative_provides: BTreeSet<ServiceId> = manifest.provides.iter().cloned().collect();
        if alternative_provides.len() != manifest.provides.len() {
            return Err(AgentError::Composition(
                "alternative declares duplicate services".into(),
            ));
        }
        let missing_services: Vec<_> = previous_provides
            .difference(&alternative_provides)
            .map(|id| id.0.clone())
            .collect();
        if !missing_services.is_empty() {
            return Err(AgentError::Composition(format!(
                "alternative must preserve every service from the current provider; missing: {}",
                missing_services.join(", ")
            )));
        }
        let conflicts: Vec<_> = alternative_provides
            .difference(&previous_provides)
            .filter(|id| {
                self.plugins.iter().any(|(plugin_id, entry)| {
                    plugin_id != &previous_owner && entry.diagnostic.manifest.provides.contains(id)
                })
            })
            .map(|id| id.0.clone())
            .collect();
        if !conflicts.is_empty() {
            return Err(AgentError::Composition(format!(
                "alternative services are already declared by another plugin: {}",
                conflicts.join(", ")
            )));
        }
        let dependents: Vec<_> = previous_provides
            .iter()
            .flat_map(|id| self.dependents.get(id).into_iter().flatten().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|id| {
                self.plugins
                    .get(id)
                    .is_some_and(|entry| entry.diagnostic.state == State::Active)
            })
            .collect();
        let undrained = self.undrained_dependents(&dependents);
        if policy == SubstitutionPolicy::RequireFullDrain && !undrained.is_empty() {
            self.substitutions.push(SubstitutionDiagnostic {
                service: service.clone(),
                previous_owner: previous_owner.clone(),
                alternative_owner: manifest.id.clone(),
                outcome: Err(format!(
                    "dependents do not support drain: {}",
                    undrained.join(", ")
                )),
                drained: Vec::new(),
                undrained: undrained.clone(),
            });
            return Err(AgentError::Composition(format!(
                "dependents do not support drain: {}",
                undrained.join(", ")
            )));
        }
        let alternative_id = manifest.id.clone();
        self.register(alternative)?;
        if let Err(error) = self.stage_plugin(&alternative_id).await {
            return Err(self
                .abort_substitution(SubstitutionAbort {
                    service,
                    previous_owner: &previous_owner,
                    alternative_id: &alternative_id,
                    gates: &[],
                    resume_ids: &[],
                    drained: Vec::new(),
                    undrained,
                    error,
                })
                .await);
        }
        let mut old_gates: Vec<Arc<LeaseGate>> = Vec::new();
        let mut missing_gates = Vec::new();
        for id in &previous_provides {
            if let Some(gate) = self.lease_gates.get(id) {
                if !old_gates.iter().any(|existing| Arc::ptr_eq(existing, gate)) {
                    old_gates.push(gate.clone());
                }
            } else {
                missing_gates.push(id.0.clone());
            }
        }
        if !missing_gates.is_empty() {
            let error = AgentError::Composition(format!(
                "missing lease gate(s) for {}: {}",
                service.0,
                missing_gates.join(", ")
            ));
            return Err(self
                .abort_substitution(SubstitutionAbort {
                    service,
                    previous_owner: &previous_owner,
                    alternative_id: &alternative_id,
                    gates: &old_gates,
                    resume_ids: &[],
                    drained: Vec::new(),
                    undrained,
                    error,
                })
                .await);
        }
        let deadline = tokio::time::Instant::now() + timeout;
        if let Err(error) = close_gates(&self.service_slots, &old_gates) {
            return Err(self
                .abort_substitution(SubstitutionAbort {
                    service,
                    previous_owner: &previous_owner,
                    alternative_id: &alternative_id,
                    gates: &old_gates,
                    resume_ids: &[],
                    drained: Vec::new(),
                    undrained,
                    error,
                })
                .await);
        }
        let mut drained = Vec::new();
        let mut attempted = Vec::new();
        for id in &dependents {
            let entry = self.plugins.get_mut(id).expect("dependent exists");
            if entry.plugin.supports_drain() {
                attempted.push(id.clone());
                let result = tokio::time::timeout_at(deadline, entry.plugin.drain()).await;
                let error = match result {
                    Ok(Ok(())) => {
                        drained.push(id.clone());
                        continue;
                    }
                    Ok(Err(error)) => error,
                    Err(_) => AgentError::Composition(format!("timed out draining dependent {id}")),
                };
                return Err(self
                    .abort_substitution(SubstitutionAbort {
                        service,
                        previous_owner: &previous_owner,
                        alternative_id: &alternative_id,
                        gates: &old_gates,
                        resume_ids: &attempted,
                        drained,
                        undrained,
                        error,
                    })
                    .await);
            }
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() || !wait_gates_idle(&old_gates, remaining).await {
            return Err(self
                .abort_substitution(SubstitutionAbort {
                    service,
                    previous_owner: &previous_owner,
                    alternative_id: &alternative_id,
                    gates: &old_gates,
                    resume_ids: &attempted,
                    drained,
                    undrained,
                    error: AgentError::Composition(format!(
                        "timed out waiting for leases on {}",
                        service.0
                    )),
                })
                .await);
        }

        if let Err(error) = self.publish_staged_plugin(&alternative_id, &old_gates) {
            return Err(self
                .abort_substitution(SubstitutionAbort {
                    service,
                    previous_owner: &previous_owner,
                    alternative_id: &alternative_id,
                    gates: &old_gates,
                    resume_ids: &attempted,
                    drained,
                    undrained,
                    error,
                })
                .await);
        }

        if let Some(mut old) = self.plugins.remove(&previous_owner) {
            self.order.retain(|id| id != &previous_owner);
            for owners in self.dependents.values_mut() {
                owners.remove(&previous_owner);
            }
            self.dependents.retain(|_, owners| !owners.is_empty());
            if let Some(mut ctx) = old.context.take() {
                ctx.cleanup().await;
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), old.plugin.deactivate()).await;
        }
        let resume_errors = self.resume_dependents(&drained).await;
        let outcome = if resume_errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "substitution committed but dependents failed to resume: {}",
                resume_errors.join(", ")
            ))
        };
        self.substitutions.push(SubstitutionDiagnostic {
            service: service.clone(),
            previous_owner,
            alternative_owner: alternative_id,
            outcome: outcome.clone(),
            drained,
            undrained,
        });
        outcome.map_err(AgentError::Composition)
    }

    fn undrained_dependents(&self, dependents: &[String]) -> Vec<String> {
        dependents
            .iter()
            .filter(|id| {
                self.plugins
                    .get(id.as_str())
                    .is_some_and(|entry| !entry.plugin.supports_drain())
            })
            .cloned()
            .collect()
    }

    async fn abort_substitution(&mut self, abort: SubstitutionAbort<'_>) -> AgentError {
        let mut outcome = abort.error.to_string();
        if let Err(reopen_error) = reopen_gates(&self.service_slots, abort.gates) {
            outcome.push_str(&format!(
                "; failed to reopen previous leases: {reopen_error}"
            ));
        }
        for cleanup_error in self.discard_staged_plugin(abort.alternative_id).await {
            outcome.push_str(&format!("; staged cleanup failed: {cleanup_error}"));
        }
        let resume_errors = self.resume_dependents(abort.resume_ids).await;
        if !resume_errors.is_empty() {
            outcome.push_str(&format!("; resume failures: {}", resume_errors.join(", ")));
        }
        self.substitutions.push(SubstitutionDiagnostic {
            service: abort.service.clone(),
            previous_owner: abort.previous_owner.into(),
            alternative_owner: abort.alternative_id.into(),
            outcome: Err(outcome.clone()),
            drained: abort.drained,
            undrained: abort.undrained,
        });
        AgentError::Composition(outcome)
    }

    async fn stage_plugin(&mut self, id: &str) -> Result<()> {
        self.generation += 1;
        let service_slots = self.service_slots.clone();
        let entry = self.plugins.get_mut(id).expect("registered plugin");
        entry.diagnostic.generation = self.generation;
        entry.diagnostic.last_error = None;
        let mut ctx = PluginContext::new(
            self.generation,
            &self.services,
            &entry.diagnostic.manifest,
            service_slots,
        );
        let result = entry.plugin.activate(&mut ctx).await.and_then(|()| {
            let declared: BTreeSet<_> =
                entry.diagnostic.manifest.provides.iter().cloned().collect();
            let actual: BTreeSet<_> = ctx.staged.keys().cloned().collect();
            if declared == actual {
                Ok(())
            } else {
                Err(AgentError::Composition(
                    "registrations differ from manifest".into(),
                ))
            }
        });
        match result {
            Ok(()) => {
                entry.context = Some(ctx);
                Ok(())
            }
            Err(error) => {
                ctx.cleanup().await;
                entry.diagnostic.state = State::Failed;
                entry.diagnostic.last_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    fn publish_staged_plugin(&mut self, id: &str, retired_gates: &[Arc<LeaseGate>]) -> Result<()> {
        let service_slots = self.service_slots.clone();
        {
            let _slots = service_slots
                .lock()
                .map_err(|_| AgentError::Composition("service slot table poisoned".into()))?;
        }

        // Normal slot acquisition invokes no user code under this mutex and uses a
        // checked lease counter, so it cannot poison the table before the next lock.
        let entry = self.plugins.get_mut(id).expect("staged plugin");
        let mut ctx = entry.context.take().ok_or_else(|| {
            AgentError::Composition(format!("plugin {id} has no staged activation"))
        })?;
        // Callbacks can re-enter the slot API. Run them after the preflight lock is
        // released and before the publication lock is acquired.
        for publish in ctx.commits.drain(..) {
            publish();
        }
        let mut slots = match service_slots.lock() {
            Ok(slots) => slots,
            Err(_) => {
                self.plugins
                    .get_mut(id)
                    .expect("staged plugin remains registered")
                    .context = Some(ctx);
                return Err(AgentError::Composition(
                    "service slot table poisoned".into(),
                ));
            }
        };
        let gate = Arc::new(LeaseGate::new(ctx.generation));
        self.services.extend(ctx.staged.clone());
        for (service, value) in &ctx.staged {
            slots.entries.insert(
                service.clone(),
                ServiceSlot {
                    value: value.clone(),
                    gate: gate.clone(),
                },
            );
            self.lease_gates.insert(service.clone(), gate.clone());
            self.providers.insert(
                service.clone(),
                ProviderRecord {
                    plugin_id: id.to_string(),
                    generation: ctx.generation,
                },
            );
        }
        for requirement in &entry.diagnostic.manifest.requires {
            self.dependents
                .entry(requirement.clone())
                .or_default()
                .insert(id.to_string());
        }
        for gate in retired_gates {
            let _ = gate.revoke();
        }
        ctx.published.store(true, Ordering::Release);
        entry.context = Some(ctx);
        entry.diagnostic.state = State::Active;
        self.order.push(id.to_string());
        drop(slots);
        Ok(())
    }

    async fn discard_staged_plugin(&mut self, id: &str) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(mut entry) = self.plugins.remove(id) {
            if let Some(mut ctx) = entry.context.take() {
                ctx.cleanup().await;
            }
            match tokio::time::timeout(Duration::from_secs(5), entry.plugin.deactivate()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("{id}: deactivate failed: {error}")),
                Err(_) => errors.push(format!("{id}: deactivate timed out")),
            }
        }
        errors
    }

    async fn resume_dependents(&mut self, ids: &[String]) -> Vec<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut errors = Vec::new();
        for id in ids.iter().rev() {
            if tokio::time::Instant::now() >= deadline {
                errors.push(format!("{id}: resume deadline expired"));
                break;
            }
            let Some(entry) = self.plugins.get_mut(id) else {
                errors.push(format!("{id}: dependent no longer registered"));
                continue;
            };
            match tokio::time::timeout_at(deadline, entry.plugin.resume()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("{id}: {error}")),
                Err(_) => {
                    errors.push(format!("{id}: resume timed out"));
                    break;
                }
            }
        }
        errors
    }
    pub async fn start(&mut self) -> Result<()> {
        let order_start = self.order.len();
        let owners = self.provider_owners()?;
        self.validate_dependency_cycles(&owners)?;

        loop {
            let ready = self.ready_plugins();
            if ready.is_empty() {
                break;
            }
            for id in ready {
                self.activate_plugin(&id, order_start).await?;
            }
        }

        self.mark_blocked_plugins();
        Ok(())
    }

    fn provider_owners(&self) -> Result<BTreeMap<ServiceId, String>> {
        let mut owners = BTreeMap::new();
        for (id, entry) in &self.plugins {
            for service in &entry.diagnostic.manifest.provides {
                if let Some(previous) = owners.insert(service.clone(), id.clone()) {
                    return Err(AgentError::Composition(format!(
                        "ambiguous provider for {}: {} and {}",
                        service.0, previous, id
                    )));
                }
            }
        }
        Ok(owners)
    }

    fn validate_dependency_cycles(&self, owners: &BTreeMap<ServiceId, String>) -> Result<()> {
        fn visit(
            id: &str,
            entries: &BTreeMap<String, Entry>,
            owners: &BTreeMap<ServiceId, String>,
            visiting: &mut BTreeSet<String>,
            visited: &mut BTreeSet<String>,
        ) -> Result<()> {
            if visited.contains(id) {
                return Ok(());
            }
            if !visiting.insert(id.into()) {
                return Err(AgentError::Composition(format!("dependency cycle at {id}")));
            }
            for requirement in &entries[id].diagnostic.manifest.requires {
                if let Some(owner) = owners.get(requirement) {
                    visit(owner, entries, owners, visiting, visited)?;
                }
            }
            visiting.remove(id);
            visited.insert(id.into());
            Ok(())
        }

        let mut visited = BTreeSet::new();
        for id in self.plugins.keys() {
            visit(
                id,
                &self.plugins,
                owners,
                &mut BTreeSet::new(),
                &mut visited,
            )?;
        }
        Ok(())
    }

    fn ready_plugins(&self) -> Vec<String> {
        self.plugins
            .iter()
            .filter(|(_, entry)| {
                !matches!(entry.diagnostic.state, State::Active | State::Failed)
                    && entry
                        .diagnostic
                        .manifest
                        .requires
                        .iter()
                        .all(|requirement| self.services.contains_key(requirement))
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    async fn activate_plugin(&mut self, id: &str, order_start: usize) -> Result<()> {
        self.generation += 1;
        let service_slots = self.service_slots.clone();
        let entry = self.plugins.get_mut(id).expect("registered plugin");
        entry.diagnostic.generation = self.generation;
        entry.diagnostic.last_error = None;
        let mut ctx = PluginContext::new(
            self.generation,
            &self.services,
            &entry.diagnostic.manifest,
            service_slots,
        );
        let result = entry.plugin.activate(&mut ctx).await.and_then(|()| {
            let declared: BTreeSet<_> =
                entry.diagnostic.manifest.provides.iter().cloned().collect();
            let actual: BTreeSet<_> = ctx.staged.keys().cloned().collect();
            if declared == actual {
                Ok(())
            } else {
                Err(AgentError::Composition(
                    "registrations differ from manifest".into(),
                ))
            }
        });
        match result {
            Ok(()) => {
                self.plugins
                    .get_mut(id)
                    .expect("activating plugin remains registered")
                    .context = Some(ctx);
                if let Err(error) = self.publish_staged_plugin(id, &[]) {
                    if let Some(mut ctx) = self
                        .plugins
                        .get_mut(id)
                        .and_then(|entry| entry.context.take())
                    {
                        ctx.cleanup().await;
                    }
                    if let Some(entry) = self.plugins.get_mut(id) {
                        entry.diagnostic.state = State::Failed;
                        entry.diagnostic.last_error = Some(error.to_string());
                    }
                    self.rollback_start(order_start).await;
                    Err(error)
                } else {
                    Ok(())
                }
            }
            Err(err) => {
                ctx.cleanup().await;
                entry.diagnostic.state = State::Failed;
                entry.diagnostic.last_error = Some(err.to_string());
                self.rollback_start(order_start).await;
                Err(err)
            }
        }
    }

    async fn rollback_start(&mut self, order_start: usize) {
        let activated: Vec<_> = self.order[order_start..].iter().rev().cloned().collect();
        for activated in activated {
            self.stop_entry(&activated, State::Blocked).await;
        }
        self.order.truncate(order_start);
        for entry in self.plugins.values_mut() {
            if !matches!(entry.diagnostic.state, State::Active | State::Failed) {
                entry.diagnostic.state = State::Blocked;
            }
        }
    }

    fn mark_blocked_plugins(&mut self) {
        for entry in self.plugins.values_mut() {
            if entry.diagnostic.state != State::Active && entry.diagnostic.state != State::Failed {
                entry.diagnostic.state = State::Blocked;
                let missing: Vec<_> = entry
                    .diagnostic
                    .manifest
                    .requires
                    .iter()
                    .filter(|requirement| !self.services.contains_key(*requirement))
                    .map(|requirement| requirement.0.clone())
                    .collect();
                entry.diagnostic.last_error = Some(if missing.is_empty() {
                    "blocked: dependencies could not be activated".into()
                } else {
                    format!(
                        "blocked: missing required service(s): {}",
                        missing.join(", ")
                    )
                });
            }
        }
    }
    async fn stop_entry(&mut self, id: &str, state: State) {
        if let Some(e) = self.plugins.get_mut(id) {
            for service in &e.diagnostic.manifest.provides {
                if let Some(gate) = self.lease_gates.get(service) {
                    let _ = gate.revoke();
                }
            }
            let service_slots = self.service_slots.clone();
            if let Ok(mut slots) = service_slots.lock() {
                for service in &e.diagnostic.manifest.provides {
                    slots.entries.remove(service);
                }
            }
            if let Some(mut ctx) = e.context.take() {
                ctx.cleanup().await;
                if let Err(err) =
                    tokio::time::timeout(Duration::from_secs(5), e.plugin.deactivate())
                        .await
                        .unwrap_or_else(|_| {
                            Err(AgentError::Composition("deactivation timed out".into()))
                        })
                {
                    e.diagnostic.last_error = Some(err.to_string());
                }
                for service in &e.diagnostic.manifest.provides {
                    self.services.remove(service);
                    self.providers.remove(service);
                    self.dependents.remove(service);
                    if let Some(gate) = self.lease_gates.remove(service) {
                        let _ = gate.revoke();
                    }
                }
            }
            e.diagnostic.state = state;
        }
        for dependents in self.dependents.values_mut() {
            dependents.remove(id);
        }
        self.dependents
            .retain(|_, dependents| !dependents.is_empty());
    }
    pub async fn remove(&mut self, id: &str) -> Result<()> {
        if !self.plugins.contains_key(id) {
            return Err(AgentError::Composition(format!("unknown plugin {id}")));
        }
        let mut affected = BTreeSet::from([id.to_string()]);
        loop {
            let unavailable: BTreeSet<_> = affected
                .iter()
                .flat_map(|p| self.plugins[p].diagnostic.manifest.provides.clone())
                .collect();
            let previous = affected.len();
            for (p, entry) in &self.plugins {
                if entry
                    .diagnostic
                    .manifest
                    .requires
                    .iter()
                    .any(|r| unavailable.contains(r))
                {
                    affected.insert(p.clone());
                }
            }
            if previous == affected.len() {
                break;
            }
        }
        for p in self
            .order
            .clone()
            .into_iter()
            .rev()
            .filter(|p| affected.contains(p))
        {
            self.stop_entry(&p, State::Blocked).await;
        }
        self.order.retain(|p| !affected.contains(p));
        self.plugins.remove(id);
        Ok(())
    }
    pub async fn shutdown(&mut self) {
        for id in std::mem::take(&mut self.order).into_iter().rev() {
            self.stop_entry(&id, State::Unloaded).await;
        }
    }
}

#[cfg(test)]
mod staged_abort_tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DropSignal(Arc<AtomicUsize>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct ResourcePlugin {
        id: &'static str,
        service: ServiceId,
        value: u64,
        dropped: Option<Arc<AtomicUsize>>,
    }

    #[async_trait]
    impl Plugin for ResourcePlugin {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: self.id.into(),
                provides: vec![self.service.clone()],
                requires: vec![],
            }
        }

        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            ctx.provide(self.service.clone(), self.value)?;
            if let Some(dropped) = &self.dropped {
                ctx.own_resource(DropSignal(dropped.clone()));
            }
            Ok(())
        }
    }

    fn provider(service: &ServiceId) -> ResourcePlugin {
        ResourcePlugin {
            id: "original",
            service: service.clone(),
            value: 1,
            dropped: None,
        }
    }

    fn alternative(service: &ServiceId, dropped: Arc<AtomicUsize>) -> ResourcePlugin {
        ResourcePlugin {
            id: "alternative",
            service: service.clone(),
            value: 2,
            dropped: Some(dropped),
        }
    }

    struct CommitCallbackPlugin {
        service: ServiceId,
        published: Arc<AtomicUsize>,
        dropped: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Plugin for CommitCallbackPlugin {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "commit-callback".into(),
                provides: vec![self.service.clone()],
                requires: vec![],
            }
        }

        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            ctx.provide(self.service.clone(), 42_u64)?;
            ctx.own_resource(DropSignal(self.dropped.clone()));
            let published = self.published.clone();
            ctx.on_commit(move || {
                published.fetch_add(1, Ordering::SeqCst);
            });
            Ok(())
        }
    }

    #[test]
    fn lease_counter_overflow_returns_an_error_without_poisoning_the_gate() {
        let service = ServiceId::new("test:overflow", 1);
        let gate = Arc::new(LeaseGate::new(1));
        gate.state.lock().unwrap().active = usize::MAX;
        let service_slots = Arc::new(Mutex::new(ServiceSlots {
            entries: BTreeMap::from([(
                service.clone(),
                ServiceSlot {
                    value: Arc::new(42_u64),
                    gate: gate.clone(),
                },
            )]),
        }));
        let slot = ServiceSlotHandle::<u64> {
            service_slots: service_slots.clone(),
            service,
            marker: std::marker::PhantomData,
        };

        let result = slot.acquire();

        assert!(result.is_err());
        assert!(!gate.state.is_poisoned());
        assert!(!service_slots.is_poisoned());
    }

    #[tokio::test]
    async fn missing_gate_after_staging_discards_alternative_and_records_abort() {
        let service = ServiceId::new("test:value", 1);
        let mut supervisor = Supervisor::default();
        supervisor.register(Box::new(provider(&service))).unwrap();
        supervisor.start().await.unwrap();
        supervisor.lease_gates.remove(&service);
        let dropped = Arc::new(AtomicUsize::new(0));

        let result = supervisor
            .substitute(
                &service,
                Box::new(alternative(&service, dropped.clone())),
                SubstitutionPolicy::ReportUndrained,
                Duration::from_secs(1),
            )
            .await;

        assert!(result.is_err());
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(!supervisor.plugins.contains_key("alternative"));
        assert_eq!(
            supervisor.provider_of(&service).unwrap().plugin_id,
            "original"
        );
        let diagnostic = supervisor.substitution_diagnostics().last().unwrap();
        assert_eq!(diagnostic.alternative_owner, "alternative");
        assert!(diagnostic.outcome.is_err());
    }

    #[tokio::test]
    async fn gate_close_failure_discards_staged_alternative_and_preserves_owner() {
        let service = ServiceId::new("test:value", 1);
        let mut supervisor = Supervisor::default();
        supervisor.register(Box::new(provider(&service))).unwrap();
        supervisor.start().await.unwrap();
        let gate = supervisor.lease_gates.get(&service).unwrap().clone();
        let poison = std::thread::spawn(move || {
            let _state = gate.state.lock().unwrap();
            panic!("poison gate mutex for test");
        });
        assert!(poison.join().is_err());
        let dropped = Arc::new(AtomicUsize::new(0));

        let result = supervisor
            .substitute(
                &service,
                Box::new(alternative(&service, dropped.clone())),
                SubstitutionPolicy::ReportUndrained,
                Duration::from_secs(1),
            )
            .await;

        assert!(result.is_err());
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(!supervisor.plugins.contains_key("alternative"));
        assert_eq!(
            supervisor.provider_of(&service).unwrap().plugin_id,
            "original"
        );
        let diagnostic = supervisor.substitution_diagnostics().last().unwrap();
        assert_eq!(diagnostic.alternative_owner, "alternative");
        assert!(diagnostic.outcome.is_err());
    }

    #[tokio::test]
    async fn failed_slot_publication_does_not_run_commit_callbacks() {
        let service = ServiceId::new("test:commit-callback", 1);
        let mut supervisor = Supervisor::default();
        let published = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicUsize::new(0));
        supervisor
            .register(Box::new(CommitCallbackPlugin {
                service: service.clone(),
                published: published.clone(),
                dropped: dropped.clone(),
            }))
            .unwrap();

        let service_slots = supervisor.service_slots.clone();
        let poison = std::thread::spawn(move || {
            let _slots = service_slots.lock().unwrap();
            panic!("poison service slot table for test");
        });
        assert!(poison.join().is_err());

        let result = supervisor.start().await;

        assert!(result.is_err());
        assert_eq!(published.load(Ordering::SeqCst), 0);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(!supervisor.services.contains_key(&service));
    }
}
