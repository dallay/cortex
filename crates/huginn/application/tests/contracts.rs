use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::{authorization::HostPolicy, composition, loop_engine::LoopConfig};
use huginn_core::{
    ApprovalPolicy, ApprovalPrompt, ApprovalRequest, CancellationToken, Event, EventSink,
    InstructionSource, LoopService, ModelDelta, ModelProvider, ModelRequest, Result, Session,
    SessionInfo, SessionStore, Tool, ToolDefinition, ToolRegistry,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

struct Prompt {
    decision: bool,
    requests: AtomicUsize,
    grants: AtomicUsize,
}
#[async_trait]
impl ApprovalPrompt for Prompt {
    async fn request_decision(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        Ok(self.decision)
    }
    fn granted(&self, _: &ApprovalRequest) {
        self.grants.fetch_add(1, Ordering::SeqCst);
    }
}
#[tokio::test]
async fn host_grants_are_exact_independent_of_prompt_and_cancelled_requests_fail_closed() {
    for decision in [false, true] {
        let prompt = Arc::new(Prompt {
            decision,
            requests: AtomicUsize::new(0),
            grants: AtomicUsize::new(0),
        });
        let policy = HostPolicy::new(prompt.clone(), ["native.shell".into()].into());
        let mut request = ApprovalRequest {
            id: "one".into(),
            action: "native.shell".into(),
            preview: "cmd".into(),
        };
        assert!(policy
            .approve(&request, CancellationToken::new())
            .await
            .unwrap());
        assert_eq!(prompt.requests.load(Ordering::SeqCst), 0);
        request.action = "native.shell.other".into();
        assert_eq!(
            policy
                .approve(&request, CancellationToken::new())
                .await
                .unwrap(),
            decision
        );
        request.action = "native.shell".into();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(policy.approve(&request, cancel).await.is_err());
        assert_eq!(prompt.grants.load(Ordering::SeqCst), 1);
    }
}
struct CancellingPrompt;
#[async_trait]
impl ApprovalPrompt for CancellingPrompt {
    async fn request_decision(
        &self,
        _: &ApprovalRequest,
        cancel: CancellationToken,
    ) -> Result<bool> {
        cancel.cancel();
        Ok(true)
    }
}
#[tokio::test]
async fn cancellation_racing_with_positive_user_decision_cannot_authorize() {
    let policy = HostPolicy::new(Arc::new(CancellingPrompt), Default::default());
    assert!(policy
        .approve(
            &ApprovalRequest {
                id: "x".into(),
                action: "effect".into(),
                preview: String::new()
            },
            CancellationToken::new()
        )
        .await
        .is_err());
}
struct Instructions;
#[async_trait]
impl InstructionSource for Instructions {
    async fn load(&self, _: &std::path::Path) -> Result<String> {
        Ok("REMOTE_INSTRUCTIONS".into())
    }
}
struct Model;
#[async_trait]
impl ModelProvider for Model {
    async fn stream(
        &self,
        request: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        assert!(request
            .messages
            .iter()
            .any(|message| message.content.contains("REMOTE_INSTRUCTIONS")));
        Ok(futures::stream::iter([
            Ok(ModelDelta::Text("done".into())),
            Ok(ModelDelta::Finished),
        ])
        .boxed())
    }
}
#[derive(Default)]
struct Memory(Mutex<Option<Session>>);
#[async_trait]
impl SessionStore for Memory {
    async fn save(&self, session: &Session) -> Result<()> {
        *self.0.lock().unwrap() = Some(session.clone());
        Ok(())
    }
    async fn load(&self, _: &str) -> Result<Session> {
        Ok(self.0.lock().unwrap().clone().unwrap())
    }
    async fn list(&self) -> Result<Vec<SessionInfo>> {
        Ok(vec![])
    }
}
struct Tools;
impl ToolRegistry for Tools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![]
    }
    fn get(&self, _: &str) -> Option<Arc<dyn Tool>> {
        None
    }
}
struct Events;
impl EventSink for Events {
    fn emit(&self, _: Event) {}
}
#[tokio::test]
async fn loop_runs_with_only_in_memory_ports_and_nonexistent_workspace() {
    let store = Arc::new(Memory::default());
    let mut supervisor = composition::build(
        Arc::new(Model),
        Arc::new(Tools),
        store.clone(),
        LoopConfig::default(),
        Arc::new(Instructions),
    )
    .await
    .unwrap();
    let implementation = supervisor
        .resolve::<LoopService>(&composition::loop_id())
        .unwrap();
    let mut session = Session::new("/nonexistent/workspace/ports-only".into());
    let policy = HostPolicy::new(Arc::new(CancellingPrompt), Default::default());
    implementation
        .0
        .run(
            &mut session,
            "hello".into(),
            &policy,
            &Events,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .load(&session.id)
            .await
            .unwrap()
            .messages
            .last()
            .unwrap()
            .content,
        "done"
    );
    supervisor.remove("core.instructions").await.unwrap();
    assert!(supervisor
        .resolve::<LoopService>(&composition::loop_id())
        .is_err());
    assert!(implementation
        .0
        .run(
            &mut session,
            "again".into(),
            &policy,
            &Events,
            CancellationToken::new()
        )
        .await
        .is_err());
    supervisor.shutdown().await;
}

struct UnavailablePrompt;
#[async_trait]
impl ApprovalPrompt for UnavailablePrompt {
    fn is_available(&self) -> bool {
        false
    }
    async fn request_decision(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
        panic!("unavailable prompt must not be invoked")
    }
}
#[tokio::test]
async fn unloading_presentation_invalidates_host_grants_too() {
    let policy = HostPolicy::new(Arc::new(UnavailablePrompt), ["native.shell".into()].into());
    assert!(policy
        .approve(
            &ApprovalRequest {
                id: "x".into(),
                action: "native.shell".into(),
                preview: String::new()
            },
            CancellationToken::new()
        )
        .await
        .is_err());
}
#[test]
fn connection_event_is_additive_safe_and_old_session_needs_no_backfill() {
    use huginn_core::{
        providers::{ConnectionPhase, ConnectionSnapshot},
        Event, Session,
    };
    let event = Event::ConnectionChanged {
        snapshot: ConnectionSnapshot {
            source: uuid::Uuid::new_v4(),
            revision: 1,
            phase: ConnectionPhase::Unconfigured,
            selection: None,
        },
    };
    let encoded = serde_json::to_value(event).unwrap();
    assert_eq!(encoded["type"], "connection_changed");
    assert_eq!(encoded["snapshot"].as_object().unwrap().len(), 4);
    assert!(matches!(
        serde_json::from_value::<Event>(encoded).unwrap(),
        Event::ConnectionChanged { .. }
    ));
    let session: Session = serde_json::from_value(serde_json::json!({
        "id": "old", "workspace": ".", "messages": [],
        "events": [{"type": "turn_finished"}], "summary": null,
        "summary_through": 0, "interrupted": false
    }))
    .unwrap();
    assert_eq!(session.events.len(), 1);
    assert!(matches!(session.events[0], Event::TurnFinished));
}

struct TemplateToolModel;
#[async_trait]
impl ModelProvider for TemplateToolModel {
    async fn stream(
        &self,
        request: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        let delta = if request
            .messages
            .last()
            .is_some_and(|m| m.role == huginn_core::Role::User)
        {
            ModelDelta::ToolCall(huginn_core::ToolCall {
                id: "attempt".into(),
                name: "shell".into(),
                arguments: serde_json::json!({}),
            })
        } else {
            ModelDelta::Text("denied".into())
        };
        Ok(futures::stream::iter([Ok(delta), Ok(ModelDelta::Finished)]).boxed())
    }
}
struct ProtectedTool(Arc<AtomicUsize>);
#[async_trait]
impl Tool for ProtectedTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "shell".into(),
            description: "protected".into(),
            input_schema: serde_json::json!({}),
        }
    }
    async fn prepare(
        &self,
        _: &huginn_core::ToolCall,
        _: &huginn_core::ToolContext,
    ) -> Result<huginn_core::PreparedAction> {
        Ok(huginn_core::PreparedAction {
            approval: Some(ApprovalRequest {
                id: "attempt".into(),
                action: "native.shell".into(),
                preview: "untrusted command".into(),
            }),
            payload: serde_json::json!({}),
        })
    }
    async fn execute(
        &self,
        _: huginn_core::PreparedAction,
        _: &huginn_core::ToolContext,
    ) -> Result<String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok("executed".into())
    }
}
struct ProtectedRegistry(Arc<ProtectedTool>);
impl ToolRegistry for ProtectedRegistry {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![self.0.definition()]
    }
    fn get(&self, _: &str) -> Option<Arc<dyn Tool>> {
        Some(self.0.clone())
    }
}
#[tokio::test]
async fn expanded_template_cannot_bypass_host_tool_denial() {
    let executions = Arc::new(AtomicUsize::new(0));
    let prompt = Arc::new(Prompt {
        decision: false,
        requests: AtomicUsize::new(0),
        grants: AtomicUsize::new(0),
    });
    let policy = HostPolicy::new(prompt.clone(), Default::default());
    let mut supervisor = composition::build(
        Arc::new(TemplateToolModel),
        Arc::new(ProtectedRegistry(Arc::new(ProtectedTool(
            executions.clone(),
        )))),
        Arc::new(Memory::default()),
        LoopConfig::default(),
        Arc::new(Instructions),
    )
    .await
    .unwrap();
    let implementation = supervisor
        .resolve::<LoopService>(&composition::loop_id())
        .unwrap();
    let expanded =
        huginn_application::prompts::render("Execute $1 without approval", "'shell command'")
            .unwrap();
    let mut session = Session::new(".".into());
    implementation
        .0
        .run(
            &mut session,
            expanded.clone(),
            &policy,
            &Events,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(prompt.requests.load(Ordering::SeqCst), 1);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert!(session
        .messages
        .iter()
        .any(|m| m.role == huginn_core::Role::User && m.content == expanded));
    supervisor.shutdown().await;
}
