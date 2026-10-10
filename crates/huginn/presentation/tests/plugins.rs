use async_trait::async_trait;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, ServiceId, State, Supervisor},
    AgentLoop, ApprovalPolicy, CancellationToken, EventSink, LoopService, Result, Session,
};
use huginn_presentation::{
    contributions::{Contributions, ConversationPlugin},
    contributions_id, presentation_id, PresentationService, RatatuiPlugin,
};
use std::sync::Arc;

struct EmptyLoop;
#[async_trait]
impl AgentLoop for EmptyLoop {
    async fn run(
        &self,
        _: &mut Session,
        _: String,
        _: &dyn ApprovalPolicy,
        _: &dyn EventSink,
        _: CancellationToken,
    ) -> Result<()> {
        Ok(())
    }
}
struct LoopPlugin;
#[async_trait]
impl Plugin for LoopPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "test.loop".into(),
            provides: vec![ServiceId::new("agent:loop", 1)],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(
            ServiceId::new("agent:loop", 1),
            LoopService(Arc::new(EmptyLoop)),
        )
    }
}
#[tokio::test]
async fn missing_loop_blocks_terminal_plugin_without_opening_stdin() {
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(RatatuiPlugin::default()))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(supervisor
        .resolve::<PresentationService>(&presentation_id())
        .is_err());
    assert_eq!(supervisor.diagnostics()[0].state, State::Blocked);
}
#[tokio::test]
async fn unload_invalidates_command_renderer_and_old_presentation_generation() {
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(LoopPlugin)).unwrap();
    supervisor
        .register(Box::new(RatatuiPlugin::default()))
        .unwrap();
    supervisor
        .register(Box::new(ConversationPlugin::default()))
        .unwrap();
    supervisor.start().await.unwrap();
    let registry = supervisor
        .resolve::<Arc<Contributions>>(&contributions_id())
        .unwrap();
    assert!(registry.command("/help").is_some());
    assert_eq!(registry.tool_label(), "Tool output");
    supervisor
        .remove("presentation.conversation")
        .await
        .unwrap();
    assert!(registry.command("/help").is_none());
    assert_eq!(registry.tool_label(), "Tool result");
    supervisor
        .register(Box::new(ConversationPlugin::default()))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(registry.command("/help").is_some());
    let presentation = supervisor
        .resolve::<PresentationService>(&presentation_id())
        .unwrap();
    supervisor.remove("presentation.ratatui").await.unwrap();
    assert!(registry.command("/help").is_none());
    assert!(presentation
        .0
        .open(&Session::new(".".into()), Default::default())
        .await
        .is_err());
    supervisor.shutdown().await;
}
