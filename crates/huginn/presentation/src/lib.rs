//! Replaceable presentation capability. Terminal APIs stay inside this crate.
pub mod approval;
pub mod completion;
pub mod contributions;
pub mod files;
#[cfg(unix)]
mod input;
#[cfg(all(unix, feature = "test-driver"))]
#[doc(hidden)]
pub use input::test_support as input_test_support;
pub mod providers;
pub mod settings;
pub mod state;
mod terminal;
mod workflow;

use async_trait::async_trait;
use contributions::Contributions;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, ServiceId},
    ApprovalPrompt, CancellationToken, EventSink, Result, Session,
};
pub use huginn_plugin_api::v1::contributions_id;
use huginn_plugin_api::v1::UiService;
use std::sync::Arc;

pub fn presentation_id() -> ServiceId {
    ServiceId::new("agent:presentation", 1)
}

/// Host-only interaction seam for user input; permission policy stays with the host.
#[async_trait]
pub trait Interaction: ApprovalPrompt + EventSink {
    async fn prompt(&self) -> Option<String>;
    fn begin(&self, cancel: CancellationToken);
    fn synchronize(&self, session: &Session);
    fn notice(&self, text: String);
}

#[async_trait]
pub trait Presentation: Send + Sync {
    async fn open(&self, session: &Session) -> Result<Connection>;
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

pub struct RatatuiPlugin {
    lifecycle: Option<CancellationToken>,
    model_configured: bool,
    readiness: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ui_config: Option<Arc<crate::settings::UiConfig>>,
    save_ui_config:
        Option<Arc<dyn Fn(&crate::settings::UiConfig) -> anyhow::Result<()> + Send + Sync>>,
}
impl Default for RatatuiPlugin {
    fn default() -> Self {
        Self::new(true)
    }
}
impl RatatuiPlugin {
    pub const fn new(model_configured: bool) -> Self {
        Self {
            lifecycle: None,
            model_configured,
            readiness: None,
            ui_config: None,
            save_ui_config: None,
        }
    }
    pub fn with_readiness(readiness: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self {
            lifecycle: None,
            model_configured: false,
            readiness: Some(readiness),
            ui_config: None,
            save_ui_config: None,
        }
    }
    /// Inject the host-owned UI config and a persistence closure. The
    /// plugin pipes both into the `Ui` via `Ratatui`.
    pub fn with_settings(
        mut self,
        ui_config: Arc<crate::settings::UiConfig>,
        save_ui_config: Arc<dyn Fn(&crate::settings::UiConfig) -> anyhow::Result<()> + Send + Sync>,
    ) -> Self {
        self.ui_config = Some(ui_config);
        self.save_ui_config = Some(save_ui_config);
        self
    }
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
        ctx.provide(contributions_id(), UiService(contributions.clone()))?;
        ctx.provide(
            presentation_id(),
            PresentationService(Arc::new(terminal::Ratatui {
                contributions,
                lifecycle: ctx.cancellation.clone(),
                drivers,
                model_configured: self.model_configured,
                readiness: self.readiness.clone(),
                ui_config: self.ui_config.clone(),
                save_ui_config: self.save_ui_config.clone(),
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
