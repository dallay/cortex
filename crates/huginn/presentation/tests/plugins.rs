use async_trait::async_trait;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, ServiceId, State, Supervisor},
    AgentLoop, ApprovalPolicy, CancellationToken, EventSink, LoopService, Result, Session,
};
use huginn_plugin_api::v1::{ToolResult, UiService};
use huginn_presentation::{
    contributions::ConversationPlugin, contributions_id, presentation_id, PresentationService,
    RatatuiPlugin,
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
    supervisor.register(Box::new(ConversationPlugin)).unwrap();
    supervisor.start().await.unwrap();
    let registry = supervisor
        .resolve::<UiService>(&contributions_id())
        .unwrap()
        .0
        .clone();
    assert!(registry.command("/help").is_some());
    let renderer = registry.renderer("native.read").unwrap();
    let input = ToolResult {
        tool: "native.read".into(),
        output: "output".into(),
        is_error: false,
    };
    assert_eq!(renderer.render(&input).unwrap().title, "Tool output");
    supervisor
        .remove("presentation.conversation")
        .await
        .unwrap();
    assert!(registry.command("/help").is_none());
    assert!(renderer.render(&input).is_err());
    assert!(registry.is_empty());
    supervisor.register(Box::new(ConversationPlugin)).unwrap();
    supervisor.start().await.unwrap();
    assert!(registry.command("/help").is_some());
    let presentation = supervisor
        .resolve::<PresentationService>(&presentation_id())
        .unwrap();
    supervisor.remove("presentation.ratatui").await.unwrap();
    assert!(registry.is_empty());
    assert!(presentation
        .0
        .open(&Session::new(".".into()))
        .await
        .is_err());
    supervisor.shutdown().await;
}
