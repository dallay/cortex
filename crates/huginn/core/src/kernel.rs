//! Minimal transactional service composition and lifecycle ownership.
use crate::{AgentError, CancellationToken, Result};
use async_trait::async_trait;
use serde::Serialize;
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
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
#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub manifest: Manifest,
    pub state: State,
    pub generation: u64,
    pub last_error: Option<String>,
}
type Services = BTreeMap<ServiceId, Arc<dyn Any + Send + Sync>>;

pub struct PluginContext {
    services: Services,
    staged: Services,
    pub cancellation: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
}
impl PluginContext {
    pub fn resolve<T: Any + Send + Sync>(&self, id: &ServiceId) -> Result<Arc<T>> {
        self.services
            .get(id)
            .cloned()
            .and_then(|v| v.downcast().ok())
            .ok_or_else(|| {
                AgentError::Composition(format!("missing or incompatible service {}", id.0))
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
    async fn cleanup(&mut self) {
        self.cancellation.cancel();
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
    pub async fn start(&mut self) -> Result<()> {
        let order_start = self.order.len();
        let mut owners = BTreeMap::new();
        for (id, entry) in &self.plugins {
            for service in &entry.diagnostic.manifest.provides {
                if owners.insert(service.clone(), id.clone()).is_some() {
                    return Err(AgentError::Composition(format!(
                        "ambiguous provider for {}",
                        service.0
                    )));
                }
            }
        }
        // Validate cycles even when another dependency is missing.
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
                &owners,
                &mut BTreeSet::new(),
                &mut visited,
            )?;
        }
        loop {
            let ready: Vec<_> = self
                .plugins
                .iter()
                .filter(|(_, e)| {
                    !matches!(e.diagnostic.state, State::Active | State::Failed)
                        && e.diagnostic
                            .manifest
                            .requires
                            .iter()
                            .all(|r| self.services.contains_key(r))
                })
                .map(|(id, _)| id.clone())
                .collect();
            if ready.is_empty() {
                break;
            }
            for id in ready {
                self.generation += 1;
                let e = self.plugins.get_mut(&id).expect("registered plugin");
                e.diagnostic.generation = self.generation;
                let mut ctx = PluginContext {
                    services: self
                        .services
                        .iter()
                        .filter(|(service, _)| e.diagnostic.manifest.requires.contains(service))
                        .map(|(id, value)| (id.clone(), value.clone()))
                        .collect(),
                    staged: BTreeMap::new(),
                    cancellation: CancellationToken::new(),
                    tasks: vec![],
                };
                let result = e.plugin.activate(&mut ctx).await.and_then(|()| {
                    let declared: BTreeSet<_> =
                        e.diagnostic.manifest.provides.iter().cloned().collect();
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
                        self.services.extend(ctx.staged.clone());
                        e.context = Some(ctx);
                        e.diagnostic.state = State::Active;
                        self.order.push(id);
                    }
                    Err(err) => {
                        ctx.cleanup().await;
                        e.diagnostic.state = State::Failed;
                        e.diagnostic.last_error = Some(err.to_string());
                        let activated: Vec<_> =
                            self.order[order_start..].iter().rev().cloned().collect();
                        for activated in activated {
                            self.stop_entry(&activated, State::Blocked).await;
                        }
                        self.order.truncate(order_start);
                        for entry in self.plugins.values_mut() {
                            if !matches!(entry.diagnostic.state, State::Active | State::Failed) {
                                entry.diagnostic.state = State::Blocked;
                            }
                        }
                        return Err(err);
                    }
                }
            }
        }
        for e in self.plugins.values_mut() {
            if e.diagnostic.state != State::Active && e.diagnostic.state != State::Failed {
                e.diagnostic.state = State::Blocked;
            }
        }
        Ok(())
    }
    async fn stop_entry(&mut self, id: &str, state: State) {
        if let Some(e) = self.plugins.get_mut(id) {
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
                }
            }
            e.diagnostic.state = state;
        }
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
