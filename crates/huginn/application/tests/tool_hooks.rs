use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::{authorization::HostPolicy, composition, loop_engine::LoopConfig};
use huginn_core::{
    hooks::{tool_hooks_id, ToolHook, ToolHooksService, ToolOutcome, ToolVerdict},
    kernel::{Manifest, Plugin, PluginContext},
    ApprovalPrompt, ApprovalRequest, CancellationToken, Event, EventSink, InstructionSource,
    LoopService, ModelDelta, ModelProvider, ModelRequest, PreparedAction, Result, Role, Session,
    SessionInfo, SessionStore, Tool, ToolCall, ToolContext, ToolDefinition, ToolRegistry,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

struct Instructions;
#[async_trait]
impl InstructionSource for Instructions {
    async fn load(&self, _: &std::path::Path) -> Result<String> {
        Ok(String::new())
    }
}
/// Calls `shell` once, then finishes with text.
struct ShellModel;
#[async_trait]
impl ModelProvider for ShellModel {
    async fn stream(
        &self,
        request: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        let delta = if request
            .messages
            .last()
            .is_some_and(|m| m.role == Role::User)
        {
            ModelDelta::ToolCall(ToolCall {
                id: "attempt".into(),
                name: "shell".into(),
                arguments: serde_json::json!({"command": "rm -rf ."}),
            })
        } else {
            ModelDelta::Text("done".into())
        };
        Ok(futures::stream::iter([Ok(delta), Ok(ModelDelta::Finished)]).boxed())
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
#[derive(Default)]
struct Counts {
    prepared: AtomicUsize,
    executed: AtomicUsize,
    prompts: AtomicUsize,
}
struct Shell(Arc<Counts>, String);
#[async_trait]
impl Tool for Shell {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "shell".into(),
            description: "effect".into(),
            input_schema: serde_json::json!({}),
        }
    }
    async fn prepare(&self, call: &ToolCall, _: &ToolContext) -> Result<PreparedAction> {
        self.0.prepared.fetch_add(1, Ordering::SeqCst);
        Ok(PreparedAction {
            approval: Some(ApprovalRequest {
                id: call.id.clone(),
                action: "native.shell".into(),
                preview: "rm -rf .".into(),
            }),
            payload: serde_json::json!({}),
        })
    }
    async fn execute(&self, _: PreparedAction, _: &ToolContext) -> Result<String> {
        self.0.executed.fetch_add(1, Ordering::SeqCst);
        Ok(self.1.clone())
    }
}
struct Tools(Arc<Shell>);
impl ToolRegistry for Tools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![self.0.definition()]
    }
    fn get(&self, _: &str) -> Option<Arc<dyn Tool>> {
        Some(self.0.clone())
    }
}
struct Approve(Arc<Counts>);
#[async_trait]
impl ApprovalPrompt for Approve {
    async fn request_decision(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
        self.0.prompts.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}
#[derive(Default)]
struct Events(Mutex<Vec<Event>>);
impl EventSink for Events {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

struct Guard;
#[async_trait]
impl ToolHook for Guard {
    async fn before(&self, call: &ToolCall, _: &ToolContext) -> Result<ToolVerdict> {
        Ok(if call.arguments["command"].as_str() == Some("rm -rf .") {
            ToolVerdict::Block("destructive command".into())
        } else {
            ToolVerdict::Continue
        })
    }
}
struct Annotate;
#[async_trait]
impl ToolHook for Annotate {
    async fn after(
        &self,
        _: &ToolCall,
        outcome: ToolOutcome<'_>,
        _: &ToolContext,
    ) -> Result<Option<String>> {
        Ok(Some(format!("saw {} bytes", outcome.output.len())))
    }
}
struct HookPlugin(&'static str, Arc<dyn ToolHook>);
#[async_trait]
impl Plugin for HookPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.0.into(),
            provides: vec![],
            requires: vec![tool_hooks_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let hooks = ctx.resolve::<ToolHooksService>(&tool_hooks_id())?.0.clone();
        hooks.register(ctx, self.0.into(), self.1.clone())
    }
}

async fn run(hook: HookPlugin) -> (Arc<Counts>, Session, Vec<Event>) {
    run_with_output(vec![hook], "executed".into()).await
}

async fn run_with_output(
    hooks: Vec<HookPlugin>,
    output: String,
) -> (Arc<Counts>, Session, Vec<Event>) {
    let counts = Arc::new(Counts::default());
    let memory = Arc::new(Memory::default());
    let mut supervisor = composition::build(
        Arc::new(ShellModel),
        Arc::new(Tools(Arc::new(Shell(counts.clone(), output)))),
        memory.clone(),
        LoopConfig {
            // Exercise output limits without the independent context-window limit.
            context_tokens: 1_000_000,
            ..Default::default()
        },
        Arc::new(Instructions),
    )
    .await
    .unwrap();
    // Extensions start after the loop is active, as in the interactive host.
    for hook in hooks {
        supervisor.register(Box::new(hook)).unwrap();
    }
    supervisor.start().await.unwrap();
    let implementation = supervisor
        .resolve::<LoopService>(&composition::loop_id())
        .unwrap();
    let policy = HostPolicy::new(Arc::new(Approve(counts.clone())), Default::default());
    let events = Events::default();
    let mut session = Session::new(".".into());
    implementation
        .0
        .run(
            &mut session,
            "clean up".into(),
            &policy,
            &events,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    supervisor.shutdown().await;
    let persisted = memory.load(&session.id).await.unwrap();
    assert_eq!(tool_result(&persisted), tool_result(&session));
    let events = events.0.into_inner().unwrap();
    (counts, persisted, events)
}
fn tool_result(session: &Session) -> &str {
    &session
        .messages
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("attempt"))
        .expect("tool call stays paired with a result")
        .content
}

#[tokio::test]
async fn blocking_hook_prevents_preparation_approval_and_effect() {
    let (counts, session, events) = run(HookPlugin("guard.shell", Arc::new(Guard))).await;
    assert_eq!(counts.prepared.load(Ordering::SeqCst), 0);
    assert_eq!(counts.prompts.load(Ordering::SeqCst), 0);
    assert_eq!(counts.executed.load(Ordering::SeqCst), 0);
    assert_eq!(
        tool_result(&session),
        "tool: blocked by hook guard.shell: destructive command"
    );
    assert!(!events
        .iter()
        .any(|e| matches!(e, Event::ApprovalRequested { .. })));
    assert!(events.iter().any(|e| matches!(
        e,
        Event::ToolFinished { is_error: true, output, .. } if output.contains("blocked by hook guard.shell")
    )));
}

#[tokio::test]
async fn after_hook_annotates_the_result_without_changing_authorization() {
    let (counts, session, _) = run(HookPlugin("note.size", Arc::new(Annotate))).await;
    assert_eq!(counts.prompts.load(Ordering::SeqCst), 1);
    assert_eq!(counts.executed.load(Ordering::SeqCst), 1);
    assert_eq!(
        tool_result(&session),
        "executed\n\n[hook note.size] saw 8 bytes"
    );
}

struct BudgetNote(Arc<Mutex<Vec<(String, bool)>>>);
#[async_trait]
impl ToolHook for BudgetNote {
    async fn after(
        &self,
        _: &ToolCall,
        outcome: ToolOutcome<'_>,
        _: &ToolContext,
    ) -> Result<Option<String>> {
        self.0
            .lock()
            .unwrap()
            .push((outcome.output.to_string(), outcome.is_error));
        Ok(Some("é".repeat(huginn_core::hooks::MAX_NOTE_BYTES / 2)))
    }
}

#[tokio::test]
async fn annotated_results_reserve_visible_notes_within_the_output_budget() {
    use huginn_application::output::{bounded_text, MAX_OUTPUT_BYTES};
    let marker = "\n[output truncated]";
    for bytes in [MAX_OUTPUT_BYTES, MAX_OUTPUT_BYTES + 8] {
        for count in [1, 2, 8] {
            let observed = Arc::new(Mutex::new(vec![]));
            let ids = [
                "note.0", "note.1", "note.2", "note.3", "note.4", "note.5", "note.6", "note.7",
            ];
            let hooks = ids[..count]
                .iter()
                .map(|id| HookPlugin(id, Arc::new(BudgetNote(observed.clone()))))
                .collect();
            let raw = "é".repeat(bytes / 2);
            let expected_input = bounded_text(raw.clone());
            let (counts, session, events) = run_with_output(hooks, raw).await;
            let output = tool_result(&session);
            assert!(
                output.len() <= MAX_OUTPUT_BYTES + marker.len(),
                "{} bytes",
                output.len()
            );
            assert!(output.contains("[hook note.0] é"));
            if count == 2 {
                assert!(output.contains("[hook note.1] é"));
            }
            assert_eq!(
                output.matches(marker).count(),
                if count == 8 { 2 } else { 1 }
            );
            if count == 8 {
                assert!(output.ends_with(marker));
            }
            assert_eq!(counts.prepared.load(Ordering::SeqCst), 1);
            assert_eq!(counts.prompts.load(Ordering::SeqCst), 1);
            assert_eq!(counts.executed.load(Ordering::SeqCst), 1);
            assert_eq!(
                session
                    .messages
                    .iter()
                    .filter(|m| m.tool_call_id.as_deref() == Some("attempt"))
                    .count(),
                1
            );
            let finished: Vec<_> = events
                .iter()
                .filter_map(|e| match e {
                    Event::ToolFinished {
                        output, is_error, ..
                    } => Some((output.as_str(), *is_error)),
                    _ => None,
                })
                .collect();
            assert_eq!(finished, vec![(output, false)]);
            let inputs = observed.lock().unwrap();
            assert_eq!(inputs.len(), count);
            assert!(inputs
                .iter()
                .all(|input| input == &(expected_input.clone(), false)));
            drop(inputs);
        }
    }
}
