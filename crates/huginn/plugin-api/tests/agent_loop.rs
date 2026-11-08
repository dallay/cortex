//! The agent loop is authorable from outside the host application (ADR-0017).

use async_trait::async_trait;
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    v1::agent::{
        agent_loop_id, AgentLoop, ApprovalPolicy, ApprovalRequest, CancellationToken, Event,
        EventSink, LoopService, Message, Result, Role, Session, StepId,
    },
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Events(Mutex<Vec<Event>>);
impl EventSink for Events {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

struct NoApproval;
#[async_trait]
impl ApprovalPolicy for NoApproval {
    async fn approve(
        &self,
        _: &ApprovalRequest,
        _: CancellationToken,
    ) -> Result<huginn_core::ApprovalDecision> {
        unreachable!("the echo loop never requests an effect")
    }
}

/// Authored only against the public SDK surface.
struct EchoLoop;
#[async_trait]
impl AgentLoop for EchoLoop {
    async fn run(
        &self,
        session: &mut Session,
        prompt: String,
        _approvals: &dyn ApprovalPolicy,
        events: &dyn EventSink,
        _cancel: CancellationToken,
    ) -> Result<()> {
        let turn = session.allocate_turn()?;
        session.events.push(Event::TurnStarted {
            session_id: session.id.clone(),
            turn,
        });
        session.messages.push(Message::text(Role::User, prompt));
        events.emit(Event::Text {
            step: StepId(1),
            text: "echo".into(),
        });
        session.events.push(Event::TurnFinished { turn });
        Ok(())
    }
}

struct LoopPlugin;
#[async_trait]
impl Plugin for LoopPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "author.echo-loop".into(),
            provides: vec![agent_loop_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(agent_loop_id(), LoopService(Arc::new(EchoLoop)))
    }
}

#[tokio::test]
async fn plugin_outside_the_host_application_can_provide_the_agent_loop() {
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(LoopPlugin)).unwrap();
    supervisor.start().await.unwrap();

    let loop_service = supervisor
        .resolve::<LoopService>(&agent_loop_id())
        .expect("the loop service must be resolvable by its contract id");

    let events = Events::default();
    let mut session = Session::new("/nonexistent/plugin-loop".into());
    loop_service
        .0
        .run(
            &mut session,
            "hello".into(),
            &NoApproval,
            &events,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(session.messages.last().unwrap().content, "hello");
    assert_eq!(
        events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, Event::Text { text, .. } if text == "echo"))
            .count(),
        1
    );
    assert!(session
        .events
        .iter()
        .any(|event| matches!(event, Event::TurnFinished { .. })));
    supervisor.shutdown().await;
}
