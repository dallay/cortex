use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_core::SessionInfo;
use huginn_plugin_api::v1::agent::{
    AgentLoop, ApprovalDecision, ApprovalPolicy, ApprovalRequest, CancellationToken, Event,
    EventSink, InstructionSource, Message, ModelDelta, ModelProvider, ModelRequest, PreparedAction,
    Result, Session, SessionStore, Tool, ToolCall, ToolContext, ToolDefinition, ToolRegistry,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

struct ApprovalTool(Arc<AtomicUsize>);

#[async_trait]
impl Tool for ApprovalTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "protected".into(),
            description: "protected test effect".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    async fn prepare(&self, call: &ToolCall, _: &ToolContext) -> Result<PreparedAction> {
        Ok(PreparedAction {
            approval: Some(ApprovalRequest {
                id: call.id.clone(),
                action: "test.protected".into(),
                preview: "protected effect".into(),
                resource: None,
                provenance: None,
            }),
            payload: serde_json::json!({}),
        })
    }

    async fn execute(&self, _: PreparedAction, _: &ToolContext) -> Result<String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok("executed".into())
    }
}

struct SingleTool(Arc<ApprovalTool>);

impl ToolRegistry for SingleTool {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![self.0.definition()]
    }

    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        (name == "protected").then(|| self.0.clone() as Arc<dyn Tool>)
    }
}

struct ScriptedModel(Mutex<Vec<Vec<ModelDelta>>>);

#[async_trait]
impl ModelProvider for ScriptedModel {
    async fn stream(
        &self,
        _: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        let deltas = self.0.lock().unwrap().remove(0);
        Ok(futures::stream::iter(deltas.into_iter().map(Ok)).boxed())
    }
}

struct MemorySessions;

#[async_trait]
impl SessionStore for MemorySessions {
    async fn save(&self, _: &Session) -> Result<()> {
        Ok(())
    }

    async fn load(&self, id: &str) -> Result<Session> {
        Err(huginn_plugin_api::AgentError::Session(format!(
            "no session {id}"
        )))
    }

    async fn list(&self) -> Result<Vec<SessionInfo>> {
        Ok(vec![])
    }
}

struct NoInstructions;

#[async_trait]
impl InstructionSource for NoInstructions {
    async fn load(&self, _: &Path) -> Result<String> {
        Ok(String::new())
    }
}

struct Decision(ApprovalDecision);

#[async_trait]
impl ApprovalPolicy for Decision {
    async fn approve(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<ApprovalDecision> {
        Ok(self.0)
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<Event>>);

impl EventSink for Events {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

fn model_for_approval_call() -> ScriptedModel {
    ScriptedModel(Mutex::new(vec![
        vec![
            ModelDelta::ToolCall(ToolCall {
                id: "approval-call".into(),
                name: "protected".into(),
                arguments: serde_json::json!({}),
            }),
            ModelDelta::Finished,
        ],
        vec![ModelDelta::Text("done".into()), ModelDelta::Finished],
    ]))
}

#[tokio::test]
async fn approval_decisions_are_observed_through_the_public_agent_loop() {
    for (decision, expected_executions, expected_approval, expected_error) in [
        (ApprovalDecision::ApproveForSession, 1, true, false),
        (ApprovalDecision::Reject, 0, false, true),
    ] {
        let executions = Arc::new(AtomicUsize::new(0));
        let agent_loop = huginn_reference_extension::step_loop::StepLoop::new(
            Arc::new(model_for_approval_call()),
            Arc::new(SingleTool(Arc::new(ApprovalTool(executions.clone())))),
            Arc::new(MemorySessions),
            Arc::new(NoInstructions),
            Default::default(),
        );
        let mut session = Session::new("/workspace".into());
        let events = Events::default();

        agent_loop
            .run(
                &mut session,
                "run protected action".into(),
                &Decision(decision),
                &events,
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(
            executions.load(Ordering::SeqCst),
            expected_executions,
            "{decision:?}"
        );
        let (has_resolution, has_tool_outcome, has_tool_result) =
            {
                let events = events.0.lock().unwrap();
                (
                    events.iter().any(|event| matches!(
                    event,
                    Event::ApprovalResolved { approved, .. } if *approved == expected_approval
                )),
                    events.iter().any(|event| matches!(
                    event,
                    Event::ToolFinished { is_error, .. } if *is_error == expected_error
                )),
                    session.messages.iter().any(|message| matches!(
                    message,
                    Message { tool_call_id: Some(id), .. } if id == "approval-call"
                )),
                )
            };
        assert!(has_resolution, "missing resolution for {decision:?}");
        assert!(has_tool_outcome, "unexpected tool outcome for {decision:?}");
        assert!(has_tool_result, "tool result missing for {decision:?}");
    }
}
