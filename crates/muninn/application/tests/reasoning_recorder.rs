use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use muninn_application::{
    HookContext, ReasoningMemory, RecorderOptions, RecorderStep, StartTraceOptions, StepInput,
    StreamingTraceRecorder, ToolCallObserver, ToolCallOptions,
};
use muninn_core::model::*;
use muninn_core::ports::*;
use muninn_core::{MemoryError, MemoryResult};
use serde_json::json;
use uuid::Uuid;

#[derive(Default)]
struct Store {
    trace: Mutex<Option<ReasoningTrace>>,
    events: Mutex<Vec<String>>,
    fail: Option<&'static str>,
    embeddings: Mutex<Vec<String>>,
}
impl Store {
    fn event(&self, event: &str) -> MemoryResult<()> {
        self.events.lock().unwrap().push(event.into());
        if self.fail == Some(event) {
            Err(MemoryError::storage("synthetic write failure"))
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl Embedder for Store {
    fn dimensions(&self) -> usize {
        1
    }
    async fn embed(&self, text: &str) -> MemoryResult<Vec<f64>> {
        self.embeddings.lock().unwrap().push(text.into());
        Ok(vec![1.0])
    }
}
#[async_trait]
impl ReasoningStore for Store {
    async fn create_trace(
        &self,
        r: &TraceRecord,
        _: &OwnerScope,
        _: Option<Uuid>,
    ) -> MemoryResult<ReasoningTrace> {
        self.event("start")?;
        let t = ReasoningTrace {
            id: r.id,
            session_id: r.session_id.clone(),
            task: r.task.clone(),
            task_embedding: r.task_embedding.clone(),
            steps: vec![],
            outcome: None,
            success: None,
            started_at: Utc::now(),
            completed_at: None,
            metadata: r.metadata.clone(),
        };
        *self.trace.lock().unwrap() = Some(t.clone());
        Ok(t)
    }
    async fn validate_message_scope(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<()> {
        self.event("validate")
    }
    async fn add_step(&self, trace_id: Uuid, r: &StepRecord) -> MemoryResult<ReasoningStep> {
        self.event("step")?;
        let mut guard = self.trace.lock().unwrap();
        let t = guard.as_mut().unwrap();
        assert_eq!(trace_id, t.id);
        let s = ReasoningStep {
            id: r.id,
            trace_id,
            step_number: i64::try_from(t.steps.len()).unwrap() + 1,
            thought: r.thought.clone(),
            action: r.action.clone(),
            observation: r.observation.clone(),
            tool_calls: vec![],
            embedding: r.embedding.clone(),
            created_at: Utc::now(),
            metadata: r.metadata.clone(),
        };
        t.steps.push(s.clone());
        drop(guard);
        Ok(s)
    }
    async fn set_step_observation(&self, id: Uuid, text: &str) -> MemoryResult<()> {
        self.event("observation")?;
        let mut g = self.trace.lock().unwrap();
        let s = g
            .as_mut()
            .unwrap()
            .steps
            .iter_mut()
            .find(|s| s.id == id)
            .unwrap();
        s.observation = Some(text.into());
        drop(g);
        Ok(())
    }
    async fn steps_without_embedding(&self, _: Uuid) -> MemoryResult<Vec<ReasoningStep>> {
        self.event("pending")?;
        Ok(self
            .trace
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .steps
            .iter()
            .filter(|s| s.embedding.is_none())
            .cloned()
            .collect())
    }
    async fn set_step_embedding(&self, id: Uuid, embedding: &[f64]) -> MemoryResult<()> {
        self.event("embedding")?;
        let mut g = self.trace.lock().unwrap();
        g.as_mut()
            .unwrap()
            .steps
            .iter_mut()
            .find(|s| s.id == id)
            .unwrap()
            .embedding = Some(embedding.to_vec());
        drop(g);
        Ok(())
    }
    async fn record_tool_call(
        &self,
        c: &ToolCall,
        _: &OwnerScope,
        _: Option<Uuid>,
    ) -> MemoryResult<()> {
        self.event("call")?;
        let mut g = self.trace.lock().unwrap();
        g.as_mut()
            .unwrap()
            .steps
            .iter_mut()
            .find(|s| Some(s.id) == c.step_id)
            .unwrap()
            .tool_calls
            .push(c.clone());
        drop(g);
        Ok(())
    }
    async fn complete_trace(
        &self,
        _: Uuid,
        outcome: Option<&str>,
        success: Option<bool>,
    ) -> MemoryResult<Option<ReasoningTrace>> {
        self.event("complete")?;
        let mut g = self.trace.lock().unwrap();
        let t = g.as_mut().unwrap();
        t.outcome = outcome.map(str::to_owned);
        t.success = success;
        t.completed_at = Some(Utc::now());
        let completed = t.clone();
        drop(g);
        Ok(Some(completed))
    }
    async fn set_trace_outcome_fields(&self, _: Uuid, _: &TraceOutcome) -> MemoryResult<()> {
        self.event("outcome_fields")
    }
    async fn get_trace_with_steps(&self, _: Uuid) -> MemoryResult<Option<ReasoningTrace>> {
        Ok(self.trace.lock().unwrap().clone())
    }
    async fn list_traces(&self, _: &TraceQuery) -> MemoryResult<Vec<ReasoningTrace>> {
        Ok(vec![])
    }
    async fn tool_stats(&self, _: Option<&str>) -> MemoryResult<Vec<ToolStats>> {
        Ok(vec![])
    }
    async fn search_traces(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: bool,
    ) -> MemoryResult<Vec<ScoredTrace>> {
        Ok(vec![])
    }
    async fn search_steps(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: bool,
    ) -> MemoryResult<Vec<ReasoningStepWithContext>> {
        Ok(vec![])
    }
    async fn record_touched(&self, _: Uuid, e: &EntityRef) -> MemoryResult<bool> {
        self.event(&format!("touched:{e:?}"))?;
        Ok(true)
    }
    async fn link_user_to_trace(&self, _: &str, _: Uuid) -> MemoryResult<()> {
        Ok(())
    }
    async fn add_conversation_step(
        &self,
        _: Uuid,
        t: Uuid,
        _: &OwnerScope,
        s: &StepRecord,
    ) -> MemoryResult<ReasoningStep> {
        self.add_step(t, s).await
    }
    async fn link_trace_to_message(&self, _: Uuid, _: Uuid, _: &OwnerScope) -> MemoryResult<bool> {
        Ok(true)
    }
    async fn link_conversation_to_trace(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<bool> {
        Ok(true)
    }
    async fn link_tool_call_to_message(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<bool> {
        Ok(true)
    }
    async fn open_conversation_trace(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<Option<Uuid>> {
        Ok(None)
    }
    async fn ensure_conversation_trace(
        &self,
        _: Uuid,
        _: &TraceRecord,
        _: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        Ok(Uuid::nil())
    }
    async fn conversation_steps(
        &self,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<Option<Vec<ReasoningStep>>> {
        Ok(None)
    }
    async fn step_context(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<Option<StepContext>> {
        Ok(None)
    }
    async fn entity_provenance(
        &self,
        _: &str,
        _: &OwnerScope,
    ) -> MemoryResult<Vec<ConversationStep>> {
        Ok(vec![])
    }
}
fn memory(s: &Arc<Store>) -> ReasoningMemory {
    ReasoningMemory::new(s.clone(), Some(s.clone()), false)
}
async fn read(m: &ReasoningMemory, id: Uuid) -> ReasoningTrace {
    m.get_trace_with_steps(id).await.unwrap().unwrap()
}

#[tokio::test]
async fn explicit_safe_content_order_observation_and_embedding_switches() {
    for enabled in [false, true] {
        let s = Arc::new(Store::default());
        let m = memory(&s);
        let mut r = StreamingTraceRecorder::start(
            &m,
            "session",
            "Approved task",
            RecorderOptions {
                generate_step_embeddings: enabled,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let id = r.trace_id();
        assert_eq!(r.step_id(), None);
        let before = s.events.lock().unwrap().clone();
        assert_eq!(
            r.add_observation("no step").await.unwrap_err().to_string(),
            "invalid input: No active step. Call start_step() first."
        );
        assert_eq!(*s.events.lock().unwrap(), before);
        let first = r
            .start_step(RecorderStep {
                summary: Some("Approved synthetic summary".into()),
                action: Some("Inspect fixture".into()),
                observation: Some("Initial observation".into()),
            })
            .await
            .unwrap();
        r.add_observation("Updated safe observation").await.unwrap();
        let second = r.start_step(RecorderStep::default()).await.unwrap();
        assert_eq!(r.step_id(), Some(second.id));
        let completed = r.finish().await.unwrap();
        assert_eq!(completed.success, Some(true));
        assert_eq!(completed.outcome, None);
        assert!(completed.completed_at.is_some());
        let t = read(&m, id).await;
        assert_eq!(t.session_id, "session");
        assert_eq!(
            t.steps.iter().map(|s| s.step_number).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(t.steps[0].id, first.id);
        assert_eq!(
            t.steps[0].thought.as_deref(),
            Some("Approved synthetic summary")
        );
        assert_eq!(
            t.steps[0].observation.as_deref(),
            Some("Updated safe observation")
        );
        assert!(t.steps[1].thought.is_none());
        assert!(t.steps.iter().all(|s| s.tool_calls.is_empty()));
        let texts = s.embeddings.lock().unwrap();
        assert_eq!(texts[0], "Approved task");
        assert_eq!(texts.len(), if enabled { 2 } else { 1 });
        if enabled {
            assert_eq!(texts[1], "Thought: Approved synthetic summary Action: Inspect fixture Observation: Initial observation");
        }
    }
}

#[tokio::test]
async fn implicit_step_timing_explicit_duration_status_and_tool_payloads() {
    let s = Arc::new(Store::default());
    let m = memory(&s);
    let mut r = StreamingTraceRecorder::start(&m, "s", "task", RecorderOptions::default())
        .await
        .unwrap();
    let args: Metadata = std::iter::once(("safe".into(), json!([1, 2]))).collect();
    let c = r
        .record_tool_call(
            "lookup",
            args.clone(),
            ToolCallOptions {
                result: Some(json!("safe result")),
                auto_observation: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(c.duration_ms.is_some_and(|d| d >= 0));
    assert_eq!(c.status, ToolCallStatus::Success);
    for status in [
        ToolCallStatus::Pending,
        ToolCallStatus::Success,
        ToolCallStatus::Failure,
        ToolCallStatus::Error,
        ToolCallStatus::Timeout,
        ToolCallStatus::Cancelled,
    ] {
        let call = r
            .record_tool_call(
                "lookup",
                Metadata::new(),
                ToolCallOptions {
                    duration_ms: Some(17),
                    status,
                    error: Some("synthetic error".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(call.duration_ms, Some(17));
        assert_eq!(call.status, status);
        assert_eq!(call.error.as_deref(), Some("synthetic error"));
        assert_eq!(call.step_id, c.step_id);
    }
    let t = read(&m, r.trace_id()).await;
    assert_eq!(t.steps.len(), 1);
    assert_eq!(t.steps[0].action.as_deref(), Some("call:lookup"));
    assert_eq!(t.steps[0].thought, None);
    assert_eq!(t.steps[0].observation.as_deref(), Some("safe result"));
    assert_eq!(t.steps[0].tool_calls.len(), 7);
    assert_eq!(t.steps[0].tool_calls[0].arguments, args);
    assert_eq!(t.steps[0].tool_calls[0].result, Some(json!("safe result")));
    r.finish().await.unwrap();
}

#[tokio::test]
async fn explicit_outcomes_failure_override_and_awaited_completion_errors() {
    for mode in ["legacy", "structured", "failure", "close-error", "drop"] {
        let s = Arc::new(Store {
            fail: (mode == "close-error").then_some("complete"),
            ..Default::default()
        });
        let m = memory(&s);
        let mut r = StreamingTraceRecorder::start(&m, "s", "task", RecorderOptions::default())
            .await
            .unwrap();
        let id = r.trace_id();
        r.set_outcome("Approved final summary", false);
        if matches!(mode, "structured" | "failure") {
            r.set_structured_outcome(TraceOutcome {
                success: true,
                summary: "Structured safe summary".into(),
                error_kind: Some("synthetic".into()),
                related_entities: vec![EntityRef::Name("final".into())],
                metrics: Default::default(),
            })
            .unwrap();
            r.start_step(RecorderStep::default()).await.unwrap();
        }
        if mode == "drop" {
            drop(r);
            assert!(read(&m, id).await.completed_at.is_none());
            assert_eq!(*s.events.lock().unwrap(), ["start"]);
            continue;
        }
        let result = if mode == "failure" {
            r.finish_failed("Approved failure summary").await
        } else {
            r.finish().await
        };
        if mode == "close-error" {
            assert_eq!(
                result.unwrap_err().to_string(),
                "storage error: synthetic write failure"
            );
            assert!(read(&m, id).await.completed_at.is_none());
        } else {
            let t = result.unwrap();
            assert_eq!(t.success, Some(mode == "structured"));
            assert_eq!(
                t.outcome.as_deref(),
                Some(match mode {
                    "structured" => "Structured safe summary",
                    "failure" => "Approved failure summary",
                    _ => "Approved final summary",
                })
            );
            assert!(t.completed_at.is_some());
            let events = s.events.lock().unwrap();
            if mode == "structured" {
                assert_eq!(events[2], "complete");
                assert_eq!(events[3], "outcome_fields");
                assert!(events[4].contains("final"));
            }
            if mode == "failure" {
                assert_eq!(events.last().unwrap(), "complete");
            }
        }
    }
}

struct Observer {
    store: Arc<Store>,
    number: u8,
    fail: bool,
}
#[async_trait]
impl ToolCallObserver for Observer {
    async fn on_tool_call_recorded(&self, ctx: &HookContext<'_>) -> MemoryResult<()> {
        let t = self.store.get_trace_with_steps(Uuid::nil()).await?.unwrap();
        assert_eq!(t.steps[0].tool_calls[0].id, ctx.tool_call().id);
        assert_eq!(ctx.step_id(), t.steps[0].id);
        assert_eq!(ctx.tool_call().arguments["approved"], json!(true));
        assert_eq!(t.steps[0].observation.as_deref(), Some("safe result"));
        self.store.event(&format!("hook:{}", self.number))?;
        if self.fail {
            return Err(MemoryError::provider("MUST NOT BE LOGGED"));
        }
        ctx.add_touched_edge(&EntityRef::Name(format!("inferred:{}", self.number)))
            .await?;
        Ok(())
    }
}
#[tokio::test]
async fn observers_are_ordered_after_writes_and_only_hook_failures_are_suppressed() {
    for fail in [
        None,
        Some("call"),
        Some("observation"),
        Some("touched:Name(\"explicit\")"),
        Some("touched:Name(\"inferred:2\")"),
    ] {
        let s = Arc::new(Store {
            fail,
            ..Default::default()
        });
        let mut m = memory(&s);
        for (number, fail) in [(1, true), (2, false), (3, false)] {
            m = m.on_tool_call_recorded(Arc::new(Observer {
                store: s.clone(),
                number,
                fail,
            }));
        }
        let t = m
            .start_trace("s", "task", StartTraceOptions::default())
            .await
            .unwrap();
        let step = m.add_step(t.id, StepInput::default()).await.unwrap();
        let result = m
            .record_tool_call(
                step.id,
                "tool",
                std::iter::once(("approved".into(), json!(true))).collect(),
                ToolCallOptions {
                    result: Some(json!("safe result")),
                    auto_observation: true,
                    touched_entities: vec![EntityRef::Name("explicit".into())],
                    ..Default::default()
                },
            )
            .await;
        let events = s.events.lock().unwrap();
        if matches!(
            fail,
            Some("call" | "observation" | "touched:Name(\"explicit\")")
        ) {
            assert_eq!(
                result.unwrap_err().to_string(),
                "storage error: synthetic write failure"
            );
            assert!(!events.iter().any(|e| e.starts_with("hook:")));
            assert_eq!(events.last().map(String::as_str), fail);
        } else {
            assert!(result.is_ok());
            assert_eq!(
                &events[..6],
                [
                    "start",
                    "step",
                    "call",
                    "observation",
                    "touched:Name(\"explicit\")",
                    "hook:1"
                ]
            );
            assert_eq!(
                &events[6..],
                [
                    "hook:2",
                    "touched:Name(\"inferred:2\")",
                    "hook:3",
                    "touched:Name(\"inferred:3\")"
                ]
            );
        }
    }
}

#[tokio::test]
async fn recorder_reuses_deferred_embeddings_and_propagates_partial_write_errors() {
    for fail in [
        None,
        Some("start"),
        Some("step"),
        Some("observation"),
        Some("embedding"),
        Some("outcome_fields"),
    ] {
        let s = Arc::new(Store {
            fail,
            ..Default::default()
        });
        let m = memory(&s);
        let result = StreamingTraceRecorder::start(
            &m,
            "s",
            "task",
            RecorderOptions {
                trace: StartTraceOptions {
                    generate_embedding: false,
                    ..Default::default()
                },
                generate_step_embeddings: true,
            },
        )
        .await;
        if fail == Some("start") {
            assert!(result.is_err());
            assert!(m.get_trace_with_steps(Uuid::nil()).await.unwrap().is_none());
            assert!(s.embeddings.lock().unwrap().is_empty());
            continue;
        }
        let mut r = result.unwrap();
        let id = r.trace_id();
        let step = r.start_step(RecorderStep::default()).await;
        if fail == Some("step") {
            assert!(step.is_err());
            assert_eq!(r.step_id(), None);
            assert!(read(&m, id).await.steps.is_empty());
            r.finish_failed("Step failed").await.unwrap();
            continue;
        }
        let observation = r.add_observation("Explicit delayed observation").await;
        if fail == Some("observation") {
            assert!(observation.is_err());
            assert_eq!(read(&m, id).await.steps[0].observation, None);
            r.finish_failed("Observation failed").await.unwrap();
            continue;
        }
        observation.unwrap();
        r.set_structured_outcome(TraceOutcome {
            success: true,
            summary: "safe".into(),
            error_kind: Some("synthetic".into()),
            related_entities: vec![EntityRef::Name("last".into())],
            metrics: Default::default(),
        })
        .unwrap();
        // Invalid replacement cannot erase the previous valid outcome or write anything.
        let events = s.events.lock().unwrap().clone();
        let invalid = TraceOutcome {
            success: false,
            summary: "invalid".into(),
            error_kind: None,
            related_entities: vec![],
            metrics: std::iter::once(("n".into(), f64::NAN)).collect(),
        };
        assert!(r.set_structured_outcome(invalid).is_err());
        assert_eq!(*s.events.lock().unwrap(), events);
        let result = r.finish().await;
        assert_eq!(result.is_err(), fail.is_some());
        let t = read(&m, id).await;
        assert_eq!(t.completed_at.is_some(), fail != Some("embedding"));
        assert_eq!(t.steps[0].embedding.is_some(), fail != Some("embedding"));
        assert_eq!(
            *s.embeddings.lock().unwrap(),
            ["Observation: Explicit delayed observation"]
        );
        let events = s.events.lock().unwrap();
        if let Some(fail) = fail {
            assert_eq!(events.last().unwrap(), fail);
        }
        if fail.is_none() {
            assert_eq!(t.outcome.as_deref(), Some("safe"));
            assert!(events.last().unwrap().contains("last"));
        }
        drop(events);
    }
}

struct LogCapture(Mutex<Vec<String>>, std::thread::ThreadId);
impl tracing::Subscriber for LogCapture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if std::thread::current().id() != self.1 {
            return;
        }
        struct Visitor(String);
        impl tracing::field::Visit for Visitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write;
                write!(&mut self.0, "{}={value:?}", field.name()).unwrap();
            }
        }
        let mut visitor = Visitor(String::new());
        event.record(&mut visitor);
        self.0.lock().unwrap().push(visitor.0);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}
#[tokio::test]
async fn observer_failure_log_contains_only_a_static_message() {
    let logs = Arc::new(LogCapture(
        Mutex::new(Vec::new()),
        std::thread::current().id(),
    ));
    // One global test subscriber prevents parallel no-subscriber callsite cache races.
    // Capture only this current-thread test's events, not sibling test payloads.
    tracing::subscriber::set_global_default(logs.clone()).unwrap();
    let s = Arc::new(Store::default());
    let m = memory(&s).on_tool_call_recorded(Arc::new(Observer {
        store: s.clone(),
        number: 1,
        fail: true,
    }));
    let t = m
        .start_trace("s", "task", StartTraceOptions::default())
        .await
        .unwrap();
    let step = m.add_step(t.id, StepInput::default()).await.unwrap();
    m.record_tool_call(
        step.id,
        "synthetic payload",
        std::iter::once(("approved".into(), json!(true))).collect(),
        ToolCallOptions {
            result: Some(json!("safe result")),
            error: Some("not a log field".into()),
            auto_observation: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        *logs.0.lock().unwrap(),
        ["message=tool-call observer failed; continuing"]
    );
}

#[tokio::test]
async fn legacy_constructor_tool_defaults_and_lifecycle_are_preserved() {
    let s = Arc::new(Store::default());
    let m = memory(&s);
    let t = m
        .start_trace("s", "task", StartTraceOptions::default())
        .await
        .unwrap();
    let step = m
        .add_step(
            t.id,
            StepInput {
                thought: Some("Explicit legacy summary".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let c = m
        .record_tool_call(step.id, "tool", Metadata::new(), ToolCallOptions::default())
        .await
        .unwrap();
    assert_eq!(c.status, ToolCallStatus::Success);
    assert_eq!((c.duration_ms, c.result, c.error), (None, None, None));
    let completed = m
        .complete_trace(t.id, Some("done"), Some(false), false)
        .await
        .unwrap();
    assert_eq!(completed.outcome.as_deref(), Some("done"));
    assert_eq!(completed.success, Some(false));
    assert_eq!(
        *s.events.lock().unwrap(),
        ["start", "step", "call", "complete"]
    );
    assert_eq!(
        *s.embeddings.lock().unwrap(),
        ["task", "Thought: Explicit legacy summary"]
    );
}
