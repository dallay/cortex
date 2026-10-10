//! Fault/stream fixture compiled only with the explicit test-driver feature.
use async_trait::async_trait;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, ServiceId, Supervisor},
    AgentLoop, ApprovalPolicy, CancellationToken, Event, EventSink, LoopService, Message, Result,
    Role, Session,
};
use huginn_presentation::{
    contributions::ConversationPlugin, presentation_id, PresentationService, RatatuiPlugin,
};
use std::{sync::Arc, time::Duration};

struct Engine;
#[async_trait]
impl AgentLoop for Engine {
    async fn run(
        &self,
        session: &mut Session,
        prompt: String,
        policy: &dyn ApprovalPolicy,
        events: &dyn EventSink,
        cancel: CancellationToken,
    ) -> Result<()> {
        session
            .messages
            .push(Message::text(Role::User, prompt.clone()));
        if prompt == "approve" {
            let approved = policy
                .approve(
                    &huginn_core::ApprovalRequest {
                        id: "test".into(),
                        action: "native.edit".into(),
                        preview: "--- file\n+++ file\n-old\n+new\nEND_OF_DIFF".into(),
                    },
                    cancel.clone(),
                )
                .await?;
            session.messages.push(Message::text(
                Role::Assistant,
                if approved { "APPROVED" } else { "DENIED" },
            ));
            return Ok(());
        }
        for index in 0..40 {
            tokio::select! {
                _ = cancel.cancelled() => { session.interrupted = true; return Err(huginn_core::AgentError::Cancelled); },
                _ = tokio::time::sleep(Duration::from_millis(20)) => events.emit(Event::Text { text: format!("chunk{index} ") }),
            }
        }
        session
            .messages
            .push(Message::text(Role::Assistant, "STREAM_COMPLETE"));
        events.emit(Event::TurnFinished);
        Ok(())
    }
}
struct EnginePlugin;
#[async_trait]
impl Plugin for EnginePlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "fixture.loop".into(),
            provides: vec![ServiceId::new("agent:loop", 1)],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(
            ServiceId::new("agent:loop", 1),
            LoopService(Arc::new(Engine)),
        )
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(EnginePlugin))?;
    supervisor.register(Box::new(RatatuiPlugin::default()))?;
    supervisor.register(Box::new(ConversationPlugin::default()))?;
    supervisor.start().await?;
    let mut session = Session::new(".".into());
    let presentation = supervisor.resolve::<PresentationService>(&presentation_id())?;
    let connection = presentation.0.open(&session, Default::default()).await?;
    if std::env::args().nth(1).as_deref() == Some("drop-reopen") {
        let ui = connection.ui.clone();
        assert_eq!(ui.prompt().await.as_deref(), Some("reopen"));
        drop(connection);
        assert!(!crossterm::terminal::is_raw_mode_enabled()?);
        let next = presentation.0.open(&session, Default::default()).await?;
        next.ui.notice("NEW_GENERATION_READY".into());
        assert_eq!(next.ui.prompt().await.as_deref(), Some("replacement input"));
        next.finish().await?;
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("drop") {
        drop(connection);
        assert!(
            !crossterm::terminal::is_raw_mode_enabled()?,
            "connection Drop must synchronously restore raw mode"
        );
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("unload") {
        supervisor.remove("presentation.ratatui").await?;
        assert!(
            !crossterm::terminal::is_raw_mode_enabled()?,
            "unload must complete terminal cleanup"
        );
        supervisor.register(Box::new(RatatuiPlugin::default()))?;
        supervisor.start().await?;
        let next = supervisor
            .resolve::<PresentationService>(&presentation_id())?
            .0
            .open(&session, Default::default())
            .await?;
        next.finish().await?;
        drop(connection);
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("panic") {
        panic!("intentional PTY restoration fixture");
    }
    let ui = connection.ui.clone();
    while let Some(prompt) = ui.prompt().await {
        let cancel = CancellationToken::new();
        ui.begin(cancel.clone());
        let result = Engine
            .run(&mut session, prompt, ui.as_ref(), ui.as_ref(), cancel)
            .await;
        ui.synchronize(&session);
        if let Err(error) = result {
            ui.notice(error.to_string());
        }
    }
    connection.finish().await?;
    supervisor.shutdown().await;
    Ok(())
}
