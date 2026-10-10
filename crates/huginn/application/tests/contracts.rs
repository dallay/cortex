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
