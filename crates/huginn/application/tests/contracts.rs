use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::{authorization::HostPolicy, composition, loop_engine::LoopConfig};
use huginn_core::{
    ApprovalDecision, ApprovalPolicy, ApprovalPrompt, ApprovalRequest, CancellationToken, Event,
    EventSink, InstructionSource, LoopService, ModelDelta, ModelProvider, ModelRequest, Result,
    Session, SessionInfo, SessionStore, Tool, ToolDefinition, ToolRegistry,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

struct Prompt {
    decision: ApprovalDecision,
    requests: AtomicUsize,
    grants: AtomicUsize,
}
#[async_trait]
impl ApprovalPrompt for Prompt {
    async fn request_decision(
        &self,
        _: &ApprovalRequest,
        allowed_decisions: &[ApprovalDecision],
        _: CancellationToken,
    ) -> Result<ApprovalDecision> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            allowed_decisions,
            &[ApprovalDecision::Reject, ApprovalDecision::ApproveOnce]
        );
        assert!(allowed_decisions.contains(&self.decision));
        Ok(self.decision)
    }
    fn granted(&self, _: &ApprovalRequest) {
        self.grants.fetch_add(1, Ordering::SeqCst);
    }
}
#[tokio::test]
async fn legacy_action_grants_cannot_authorize_shell_or_mcp_invocations() {
    let prompt = Arc::new(Prompt {
        decision: ApprovalDecision::ApproveOnce,
        requests: AtomicUsize::new(0),
        grants: AtomicUsize::new(0),
    });
    let allowed = [
        "native.shell".to_owned(),
        "native.shell.exec".to_owned(),
        "mcp.github.search".to_owned(),
        "mcp.github.start".to_owned(),
    ]
    .into();
    let policy = HostPolicy::new(prompt.clone(), allowed);

    for (index, action) in [
        "native.shell",
        "native.shell.exec",
        "mcp.github.search",
        "mcp.github.start",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            policy
                .approve(
                    &ApprovalRequest {
                        id: index.to_string(),
                        action: action.into(),
                        preview: "effect".into(),
                        resource: None,
                        provenance: None,
                    },
                    CancellationToken::new(),
                )
                .await
                .unwrap(),
            ApprovalDecision::ApproveOnce
        );
    }
    assert_eq!(prompt.requests.load(Ordering::SeqCst), 4);
    assert_eq!(prompt.grants.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn legacy_action_grants_cannot_authorize_file_mutations() {
    let prompt = Arc::new(Prompt {
        decision: ApprovalDecision::Reject,
        requests: AtomicUsize::new(0),
        grants: AtomicUsize::new(0),
    });
    let allowed = [
        "native.write_file".to_owned(),
        "native.edit_file".to_owned(),
    ]
    .into();
    let policy = HostPolicy::new(prompt.clone(), allowed);

    for (index, action) in ["native.write_file", "native.edit_file"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            policy
                .approve(
                    &ApprovalRequest {
                        id: index.to_string(),
                        action: action.into(),
                        preview: "file mutation".into(),
                        resource: None,
                        provenance: None,
                    },
                    CancellationToken::new(),
                )
                .await
                .unwrap(),
            ApprovalDecision::Reject
        );
    }
    assert_eq!(prompt.requests.load(Ordering::SeqCst), 2);
    assert_eq!(prompt.grants.load(Ordering::SeqCst), 0);
}

struct SessionGrantPrompt;
#[async_trait]
impl ApprovalPrompt for SessionGrantPrompt {
    async fn request_decision(
        &self,
        _: &ApprovalRequest,
        allowed_decisions: &[ApprovalDecision],
        _: CancellationToken,
    ) -> Result<ApprovalDecision> {
        assert!(!allowed_decisions.contains(&ApprovalDecision::ApproveForSession));
        Ok(ApprovalDecision::ApproveForSession)
    }
}
#[tokio::test]
async fn host_policy_rejects_session_grants_until_resource_provenance_is_verified() {
    let policy =
        HostPolicy::new(Arc::new(SessionGrantPrompt), Default::default()).for_session("session");
    assert_eq!(
        policy
            .approve(
                &ApprovalRequest {
                    id: "session-grant".into(),
                    action: "native.write_file".into(),
                    preview: "write file".into(),
                    resource: Some(huginn_core::ApprovalResource {
                        canonical_path: std::env::temp_dir().join("huginn-contract.txt"),
                        sensitive: false,
                    }),
                    provenance: None,
                },
                CancellationToken::new(),
            )
            .await
            .unwrap(),
        ApprovalDecision::Reject
    );
}

#[tokio::test]
async fn session_grants_match_exact_action_and_canonical_resource_only() {
    let prompt = Arc::new(SessionGrantRecorder::default());
    let policy = HostPolicy::new(prompt.clone(), Default::default()).for_session("session-one");
    let path_a = std::env::temp_dir().join("huginn-contract").join("a.txt");
    let path_b = std::env::temp_dir().join("huginn-contract").join("b.txt");
    let request = |id: &str, action: &str, path: &std::path::Path| ApprovalRequest {
        id: id.into(),
        action: action.into(),
        preview: format!("preview for {}", path.display()),
        resource: Some(huginn_core::ApprovalResource {
            canonical_path: path.to_path_buf(),
            sensitive: false,
        }),
        provenance: Some(huginn_core::ApprovalProvenance::NativeFileBuiltin),
    };
    assert_eq!(
        policy
            .approve(
                &request("a", "native.write_file", &path_a),
                CancellationToken::new()
            )
            .await
            .unwrap(),
        ApprovalDecision::ApproveForSession
    );
    assert_eq!(
        policy
            .approve(
                &request("repeat", "native.write_file", &path_a),
                CancellationToken::new()
            )
            .await
            .unwrap(),
        ApprovalDecision::ApproveForSession
    );
    assert_eq!(
        prompt.requests.load(Ordering::SeqCst),
        1,
        "the exact grant should be reused"
    );
    for (action, path) in [
        ("native.write_file", path_b.as_path()),
        ("native.edit_file", path_a.as_path()),
    ] {
        assert_eq!(
            policy
                .approve(&request("other", action, path), CancellationToken::new())
                .await
                .unwrap(),
            ApprovalDecision::ApproveForSession
        );
    }
    assert_eq!(
        prompt.requests.load(Ordering::SeqCst),
        3,
        "grants must match exact action and canonical path"
    );
}

#[tokio::test]
async fn session_grants_do_not_cross_session_identities() {
    let prompt = Arc::new(SessionGrantRecorder::default());
    let host_policy = HostPolicy::new(prompt.clone(), Default::default());
    let policy = host_policy.for_session("session-one");
    let path = std::env::temp_dir()
        .join("huginn-contract")
        .join("shared.txt");
    let request = ApprovalRequest {
        id: "request".into(),
        action: "native.write_file".into(),
        preview: "write file".into(),
        resource: Some(huginn_core::ApprovalResource {
            canonical_path: path,
            sensitive: false,
        }),
        provenance: Some(huginn_core::ApprovalProvenance::NativeFileBuiltin),
    };

    assert_eq!(
        policy
            .approve(&request, CancellationToken::new())
            .await
            .unwrap(),
        ApprovalDecision::ApproveForSession
    );
    assert_eq!(
        host_policy
            .for_session("session-two")
            .approve(&request, CancellationToken::new())
            .await
            .unwrap(),
        ApprovalDecision::ApproveForSession
    );
    assert_eq!(prompt.requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn exact_generic_allowlist_grant_does_not_cover_sibling_action() {
    let prompt = Arc::new(Prompt {
        decision: ApprovalDecision::ApproveOnce,
        requests: AtomicUsize::new(0),
        grants: AtomicUsize::new(0),
    });
    let policy = HostPolicy::new(prompt.clone(), ["native.custom.read".to_owned()].into());
    for action in ["native.custom.read", "native.custom.read_all"] {
        assert_eq!(
            policy
                .approve(
                    &ApprovalRequest {
                        id: action.into(),
                        action: action.into(),
                        preview: "effect".into(),
                        resource: None,
                        provenance: None,
                    },
                    CancellationToken::new(),
                )
                .await
                .unwrap(),
            ApprovalDecision::ApproveOnce
        );
    }
    assert_eq!(prompt.grants.load(Ordering::SeqCst), 1);
    assert_eq!(prompt.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn sensitive_resources_never_offer_or_accept_session_grants() {
    struct SensitivePrompt;
    #[async_trait]
    impl ApprovalPrompt for SensitivePrompt {
        async fn request_decision(
            &self,
            _: &ApprovalRequest,
            allowed: &[ApprovalDecision],
            _: CancellationToken,
        ) -> Result<ApprovalDecision> {
            assert_eq!(
                allowed,
                &[ApprovalDecision::Reject, ApprovalDecision::ApproveOnce]
            );
            Ok(ApprovalDecision::ApproveForSession)
        }
    }
    let policy =
        HostPolicy::new(Arc::new(SensitivePrompt), Default::default()).for_session("session");
    let mut request = ApprovalRequest {
        id: "sensitive".into(),
        action: "native.write_file".into(),
        preview: "redacted".into(),
        resource: Some(huginn_core::ApprovalResource {
            canonical_path: std::env::temp_dir().join(".env"),
            sensitive: true,
        }),
        provenance: Some(huginn_core::ApprovalProvenance::NativeFileBuiltin),
    };
    assert_eq!(
        policy
            .approve(&request, CancellationToken::new())
            .await
            .unwrap(),
        ApprovalDecision::Reject
    );
    request.resource.as_mut().unwrap().sensitive = false;
    request.provenance = None;
    assert_eq!(
        policy
            .approve(&request, CancellationToken::new())
            .await
            .unwrap(),
        ApprovalDecision::Reject
    );
}

#[derive(Default)]
struct SessionGrantRecorder {
    requests: AtomicUsize,
}
#[async_trait]
impl ApprovalPrompt for SessionGrantRecorder {
    async fn request_decision(
        &self,
        _: &ApprovalRequest,
        allowed: &[ApprovalDecision],
        _: CancellationToken,
    ) -> Result<ApprovalDecision> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        assert!(allowed.contains(&ApprovalDecision::ApproveForSession));
        Ok(ApprovalDecision::ApproveForSession)
    }
}

#[tokio::test]
async fn host_grants_are_exact_independent_of_prompt_and_cancelled_requests_fail_closed() {
    for decision in [ApprovalDecision::Reject, ApprovalDecision::ApproveOnce] {
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
            resource: None,
            provenance: None,
        };
        assert_eq!(
            policy
                .approve(&request, CancellationToken::new())
                .await
                .unwrap(),
            decision
        );
        assert_eq!(prompt.requests.load(Ordering::SeqCst), 1);
        assert_eq!(prompt.grants.load(Ordering::SeqCst), 0);
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
        assert_eq!(prompt.grants.load(Ordering::SeqCst), 0);
    }
}
struct CancellingPrompt;
#[async_trait]
impl ApprovalPrompt for CancellingPrompt {
    async fn request_decision(
        &self,
        _: &ApprovalRequest,
        _: &[ApprovalDecision],
        cancel: CancellationToken,
    ) -> Result<ApprovalDecision> {
        cancel.cancel();
        Ok(ApprovalDecision::ApproveOnce)
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
                preview: String::new(),
                resource: None,
                provenance: None
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
    async fn request_decision(
        &self,
        _: &ApprovalRequest,
        _: &[ApprovalDecision],
        _: CancellationToken,
    ) -> Result<ApprovalDecision> {
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
                preview: String::new(),
                resource: None,
                provenance: None
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
    assert!(matches!(session.events[0], Event::TurnFinished { .. }));
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

struct SessionGrantModel(AtomicUsize);
#[async_trait]
impl ModelProvider for SessionGrantModel {
    async fn stream(
        &self,
        _: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        let delta = if self.0.fetch_add(1, Ordering::SeqCst).is_multiple_of(2) {
            ModelDelta::ToolCall(huginn_core::ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                name: "session_write".into(),
                arguments: serde_json::json!({}),
            })
        } else {
            ModelDelta::Text("complete".into())
        };
        Ok(futures::stream::iter([Ok(delta), Ok(ModelDelta::Finished)]).boxed())
    }
}

struct SessionGrantTool(Arc<AtomicUsize>);
#[async_trait]
impl Tool for SessionGrantTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "session_write".into(),
            description: "test write".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }
    }
    async fn prepare(
        &self,
        call: &huginn_core::ToolCall,
        _: &huginn_core::ToolContext,
    ) -> Result<huginn_core::PreparedAction> {
        Ok(huginn_core::PreparedAction {
            approval: Some(ApprovalRequest {
                id: call.id.clone(),
                action: "native.write_file".into(),
                preview: "write exact resource".into(),
                resource: Some(huginn_core::ApprovalResource {
                    canonical_path: "/workspace/allowed.txt".into(),
                    sensitive: false,
                }),
                provenance: Some(huginn_core::ApprovalProvenance::NativeFileBuiltin),
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
        Ok("written".into())
    }
}

struct SessionGrantRegistry(Arc<SessionGrantTool>);
impl ToolRegistry for SessionGrantRegistry {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![self.0.definition()]
    }
    fn get(&self, _: &str) -> Option<Arc<dyn Tool>> {
        Some(self.0.clone())
    }
}

#[tokio::test]
async fn session_grant_executes_first_and_matching_next_action_without_another_prompt() {
    let executions = Arc::new(AtomicUsize::new(0));
    let prompt = Arc::new(SessionGrantRecorder::default());
    let host_policy = HostPolicy::new(prompt.clone(), Default::default());
    let store = Arc::new(Memory::default());
    let mut supervisor = composition::build(
        Arc::new(SessionGrantModel(AtomicUsize::new(0))),
        Arc::new(SessionGrantRegistry(Arc::new(SessionGrantTool(
            executions.clone(),
        )))),
        store,
        LoopConfig::default(),
        Arc::new(Instructions),
    )
    .await
    .unwrap();
    let implementation = supervisor
        .resolve::<LoopService>(&composition::loop_id())
        .unwrap();
    let mut session = Session::new("/workspace".into());
    let policy = host_policy.for_session(session.id.clone());
    let events = Events;

    for prompt_text in ["first", "matching next"] {
        implementation
            .0
            .run(
                &mut session,
                prompt_text.into(),
                &policy,
                &events,
                CancellationToken::new(),
            )
            .await
            .unwrap();
    }

    assert_eq!(executions.load(Ordering::SeqCst), 2);
    assert_eq!(prompt.requests.load(Ordering::SeqCst), 1);
    assert!(session
        .events
        .iter()
        .any(|event| matches!(event, Event::ApprovalResolved { approved: true, .. })));
    supervisor.shutdown().await;
}

#[tokio::test]
async fn rejected_effect_is_recorded_denied_and_not_executed() {
    struct RejectSessionGrant;
    #[async_trait]
    impl ApprovalPrompt for RejectSessionGrant {
        async fn request_decision(
            &self,
            _: &ApprovalRequest,
            allowed: &[ApprovalDecision],
            _: CancellationToken,
        ) -> Result<ApprovalDecision> {
            assert!(allowed.contains(&ApprovalDecision::Reject));
            Ok(ApprovalDecision::Reject)
        }
    }
    let executions = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(Memory::default());
    let mut supervisor = composition::build(
        Arc::new(SessionGrantModel(AtomicUsize::new(0))),
        Arc::new(SessionGrantRegistry(Arc::new(SessionGrantTool(
            executions.clone(),
        )))),
        store,
        LoopConfig::default(),
        Arc::new(Instructions),
    )
    .await
    .unwrap();
    let implementation = supervisor
        .resolve::<LoopService>(&composition::loop_id())
        .unwrap();
    let mut session = Session::new("/workspace".into());
    implementation
        .0
        .run(
            &mut session,
            "reject".into(),
            &HostPolicy::new(Arc::new(RejectSessionGrant), Default::default()),
            &Events,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert!(session.events.iter().any(|event| matches!(
        event,
        Event::ApprovalResolved {
            approved: false,
            ..
        }
    )));
    supervisor.shutdown().await;
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
                resource: None,
                provenance: None,
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
        decision: ApprovalDecision::Reject,
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
