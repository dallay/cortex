//! Replaceable presentation capability. Terminal APIs stay inside this crate.
pub mod contributions;
pub mod state;
mod terminal;

use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, ServiceId},
    ApprovalPolicy, CancellationToken, EventSink, Result, Session,
};
use async_trait::async_trait;
use contributions::Contributions;
use std::{collections::BTreeSet, sync::Arc};

pub fn presentation_id() -> ServiceId {
    ServiceId::new("agent:presentation", 1)
}
pub fn contributions_id() -> ServiceId {
    ServiceId::new("agent:ui-contributions", 1)
}

/// Host-only interaction seam; contributions never receive this authority.
#[async_trait]
pub trait Interaction: ApprovalPolicy + EventSink {
    async fn prompt(&self) -> Option<String>;
    fn begin(&self, cancel: CancellationToken);
    fn synchronize(&self, session: &Session);
    fn notice(&self, text: String);
}

#[async_trait]
pub trait Presentation: Send + Sync {
    async fn open(&self, session: &Session, allowed: BTreeSet<String>) -> Result<Connection>;
}
pub struct PresentationService(pub Arc<dyn Presentation>);

/// Owns the driver task, ensuring no terminal survives its host connection.
pub struct Connection {
    pub ui: Arc<dyn Interaction>,
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<Result<()>>>,
    cleanup: Arc<dyn Fn() + Send + Sync>,
}
impl Connection {
    pub fn new(
        ui: Arc<dyn Interaction>,
        stop: CancellationToken,
        task: tokio::task::JoinHandle<Result<()>>,
        cleanup: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            ui,
            stop,
            task: Some(task),
            cleanup,
        }
    }
    pub async fn finish(mut self) -> Result<()> {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.await.map_err(|error| {
                huginn_core::AgentError::Composition(format!("presentation task: {error}"))
            })??;
        }
        Ok(())
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
        (self.cleanup)();
    }
}

#[derive(Default)]
pub struct RatatuiPlugin {
    lifecycle: Option<CancellationToken>,
}
#[async_trait]
impl Plugin for RatatuiPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "presentation.ratatui".into(),
            provides: vec![presentation_id(), contributions_id()],
            requires: vec![ServiceId::new("agent:loop", 1)],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        // Resolve the declared contract, not merely its name.
        ctx.resolve::<huginn_core::LoopService>(&ServiceId::new("agent:loop", 1))?;
        let contributions = Arc::new(Contributions::default());
        let drivers = Arc::new(std::sync::Mutex::new(Vec::<(
            CancellationToken,
            CancellationToken,
        )>::new()));
        let owned = drivers.clone();
        let lifecycle = ctx.cancellation.clone();
        ctx.own_task(tokio::spawn(async move {
            lifecycle.cancelled().await;
            let drivers = std::mem::take(
                &mut *owned
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            for (stop, _) in &drivers {
                stop.cancel();
            }
            for (_, done) in drivers {
                done.cancelled().await;
            }
        }));
        self.lifecycle = Some(ctx.cancellation.clone());
        ctx.provide(contributions_id(), contributions.clone())?;
        ctx.provide(
            presentation_id(),
            PresentationService(Arc::new(terminal::Ratatui {
                contributions,
                lifecycle: ctx.cancellation.clone(),
                drivers,
            })),
        )
    }
    async fn deactivate(&mut self) -> Result<()> {
        if let Some(lifecycle) = self.lifecycle.take() {
            lifecycle.cancel();
        }
        Ok(())
    }
}
