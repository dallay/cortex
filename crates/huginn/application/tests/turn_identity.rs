//! Turn and step identity: every lifecycle event must be attributable to the turn and
//! model call that produced it, including after a resumed session.

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::{composition, loop_engine::LoopConfig};
use huginn_core::{
    ApprovalPolicy, ApprovalRequest, CancellationToken, Event, EventSink, InstructionSource,
    LoopService, ModelDelta, ModelProvider, ModelRequest, PreparedAction, Result, Session,
    SessionInfo, SessionStore, StepId, Tool, ToolCall, ToolContext, ToolDefinition, ToolRegistry,
    TurnId,
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

/// First model call of each turn returns a tool call; the second returns text.
struct Scripted(AtomicUsize);
#[async_trait]
impl ModelProvider for Scripted {
    async fn stream(
        &self,
        _: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        let call = self.0.fetch_add(1, Ordering::SeqCst);
        let deltas = if call.is_multiple_of(2) {
            vec![
                ModelDelta::Usage {
                    input_tokens: 7,
                    output_tokens: 3,
                },
                ModelDelta::ToolCall(ToolCall {
                    id: format!("call-{call}"),
                    name: "shell".into(),
                    arguments: serde_json::json!({}),
                }),
                ModelDelta::Finished,
            ]
        } else {
            vec![ModelDelta::Text("done".into()), ModelDelta::Finished]
        };
        Ok(futures::stream::iter(deltas.into_iter().map(Ok)).boxed())
    }
}

#[derive(Clone, Default)]
struct Memory(Arc<Mutex<Option<Session>>>);
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
impl Memory {
    fn snapshot(&self) -> Session {
        self.0.lock().unwrap().clone().unwrap()
    }
}

struct Shell;
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
        Ok(PreparedAction {
            approval: Some(ApprovalRequest {
                id: call.id.clone(),
                action: "native.shell".into(),
                preview: "echo".into(),
            }),
            payload: serde_json::json!({}),
        })
    }
    async fn execute(&self, _: PreparedAction, _: &ToolContext) -> Result<String> {
        Ok("ok".into())
    }
}
struct Tools;
impl ToolRegistry for Tools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![Shell.definition()]
    }
    fn get(&self, _: &str) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Shell))
    }
}

struct Approve;
#[async_trait]
impl ApprovalPolicy for Approve {
    async fn approve(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
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
impl Events {
    fn snapshot(&self) -> Vec<Event> {
        self.0.lock().unwrap().clone()
    }
}

async fn start(model: Arc<dyn ModelProvider>) -> (huginn_core::kernel::Supervisor, Memory) {
    let store = Memory::default();
    let supervisor = composition::build(
        model,
        Arc::new(Tools),
        Arc::new(store.clone()),
        LoopConfig::default(),
        Arc::new(Instructions),
    )
    .await
    .unwrap();
    (supervisor, store)
}

fn started_turns(events: &[Event]) -> Vec<TurnId> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::TurnStarted { turn, .. } => Some(*turn),
            _ => None,
        })
        .collect()
}

const fn step_of(event: &Event) -> Option<StepId> {
    match event {
        Event::Text { step, .. }
        | Event::ToolStarted { step, .. }
        | Event::ToolFinished { step, .. }
        | Event::ApprovalRequested { step, .. }
        | Event::ApprovalResolved { step, .. }
        | Event::Usage { step, .. } => Some(*step),
        _ => None,
    }
}

#[tokio::test]
async fn consecutive_turns_are_disjoint_and_steps_are_ordered() {
    let (supervisor, store) = start(Arc::new(Scripted(AtomicUsize::new(0)))).await;
    let slot = supervisor
        .resolve::<LoopService>(&composition::loop_id())
        .unwrap();
    let events = Events::default();
    let mut session = Session::new("/nonexistent/turn-identity".into());

    for prompt in ["first", "second"] {
        slot.0
            .run(
                &mut session,
                prompt.into(),
                &Approve,
                &events,
                CancellationToken::new(),
            )
            .await
            .unwrap();
    }

    let turns = started_turns(&store.snapshot().events);
    assert_eq!(turns.len(), 2, "each run must start exactly one turn");
    assert_ne!(turns[0], turns[1], "consecutive turns must be distinct");
    assert_eq!(turns, vec![TurnId(1), TurnId(2)]);

    let tool_steps: Vec<StepId> = store
        .snapshot()
        .events
        .iter()
        .filter(|event| matches!(event, Event::ToolStarted { .. }))
        .filter_map(step_of)
        .collect();
    assert_eq!(tool_steps, vec![StepId(1), StepId(1)]);
}

#[tokio::test]
async fn every_step_event_carries_the_step_identity_of_its_model_call() {
    let (supervisor, store) = start(Arc::new(Scripted(AtomicUsize::new(0)))).await;
    let slot = supervisor
        .resolve::<LoopService>(&composition::loop_id())
        .unwrap();
    let events = Events::default();
    let mut session = Session::new("/nonexistent/turn-identity".into());
    slot.0
        .run(
            &mut session,
            "go".into(),
            &Approve,
            &events,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    let recorded = store.snapshot().events;
    // The tool cycle belongs to the first model call; the final text to the second.
    let tool_cycle: Vec<StepId> = recorded
        .iter()
        .filter(|event| {
            matches!(
                event,
                Event::Usage { .. }
                    | Event::ToolStarted { .. }
                    | Event::ApprovalRequested { .. }
                    | Event::ApprovalResolved { .. }
                    | Event::ToolFinished { .. }
            )
        })
        .filter_map(step_of)
        .collect();
    assert_eq!(
        tool_cycle,
        vec![StepId(1), StepId(1), StepId(1), StepId(1), StepId(1)],
        "the usage, tool and approval events of one model call share one step"
    );

    let text_steps: Vec<StepId> = events
        .snapshot()
        .iter()
        .filter(|event| matches!(event, Event::Text { .. }))
        .filter_map(step_of)
        .collect();
    assert_eq!(
        text_steps,
        vec![StepId(2)],
        "the answer of the second model call is step 2 (text deltas are live-only)"
    );
}

#[tokio::test]
async fn resume_after_interruption_keeps_historical_ids_and_allocates_a_fresh_turn() {
    let (supervisor, store) = start(Arc::new(Scripted(AtomicUsize::new(0)))).await;
    let slot = supervisor
        .resolve::<LoopService>(&composition::loop_id())
        .unwrap();
    let events = Events::default();
    let mut session = Session::new("/nonexistent/turn-identity".into());

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    slot.0
        .run(
            &mut session,
            "interrupted".into(),
            &Approve,
            &events,
            cancelled,
        )
        .await
        .unwrap_err();
    let interrupted_snapshot = store.snapshot();
    assert_eq!(started_turns(&interrupted_snapshot.events), vec![TurnId(1)]);
    assert!(
        interrupted_snapshot.interrupted,
        "a failed turn must stay marked as interrupted"
    );
    assert!(
        !interrupted_snapshot
            .events
            .iter()
            .any(|event| matches!(event, Event::TurnFinished { .. })),
        "a failed turn must not report completion"
    );

    // Resume is modelled as a fresh process reading the persisted session.
    let mut resumed = store.load("ignored").await.unwrap();
    slot.0
        .run(
            &mut resumed,
            "continue".into(),
            &Approve,
            &events,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    let started = started_turns(&store.snapshot().events);
    assert_eq!(
        started,
        vec![TurnId(1), TurnId(2)],
        "resume must keep the historical turn id and allocate the next one"
    );
    assert!(
        store
            .snapshot()
            .events
            .iter()
            .any(|event| matches!(event, Event::TurnFinished { .. })),
        "the resumed turn must report completion"
    );
}

#[tokio::test]
async fn legacy_persisted_events_without_identity_still_load() {
    let legacy = serde_json::json!({
        "id": "legacy-session",
        "workspace": "/nonexistent/turn-identity",
        "messages": [],
        "events": [
            {"type": "turn_started", "session_id": "legacy-session"},
            {"type": "text", "text": "old answer"},
            {"type": "turn_finished"}
        ],
        "summary": null,
        "summary_through": 0,
        "interrupted": false
    });
    let session: Session = serde_json::from_value(legacy).expect("legacy session must deserialize");

    assert_eq!(
        started_turns(&session.events),
        vec![TurnId::default()],
        "records written before identity existed must load as unknown, not fail"
    );
    assert!(
        session
            .events
            .iter()
            .filter_map(step_of)
            .all(|step| step == StepId::default()),
        "legacy step-scoped events load as unknown"
    );
}
