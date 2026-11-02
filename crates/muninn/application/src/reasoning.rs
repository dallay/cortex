//! Reasoning memory use cases (`memory/reasoning.py`).
//!
//! # Privacy policy (CORTEX)
//!
//! Muninn must not be used to capture a provider's private chain of thought.
//! The `thought` field (and the hosted `reasoning` field mapped onto it) is
//! kept only for API and graph-schema compatibility with upstream: callers
//! store explicit, approved reasoning summaries there, never raw hidden
//! reasoning tokens emitted by a model.

use std::sync::Arc;

use muninn_core::model::{
    AgentStep, ConversationTrace, EntityProvenance, EntityRef, Metadata, ReasoningStep,
    ReasoningStepWithContext, ReasoningTrace, StepExplanation, ToolCall, ToolCallStatus, ToolStats,
};
use muninn_core::ports::{
    Embedder, OwnerScope, ReasoningStore, StepRecord, TraceQuery, TraceRecord,
};
use muninn_core::{MemoryError, MemoryResult};
use serde_json::Value;
use uuid::Uuid;

/// Upstream `_format_observation` cap.
const MAX_OBSERVATION_CHARS: usize = 1000;

/// Options for [`ReasoningMemory::start_trace`].
#[derive(Debug, Clone)]
pub struct StartTraceOptions {
    pub generate_embedding: bool,
    pub metadata: Metadata,
    /// Message that initiated the trace (`INITIATED_BY`).
    pub triggered_by_message_id: Option<Uuid>,
    /// Scope the trace to a user (`(:User)-[:HAS_TRACE]->`).
    pub user_identifier: Option<String>,
}

impl Default for StartTraceOptions {
    fn default() -> Self {
        Self {
            generate_embedding: true,
            metadata: Metadata::new(),
            triggered_by_message_id: None,
            user_identifier: None,
        }
    }
}

/// Content of one step for [`ReasoningMemory::add_step`].
///
/// `thought` must hold an explicit, approved reasoning summary, never a
/// provider's private chain of thought (see the module privacy policy).
#[derive(Debug, Clone)]
pub struct StepInput {
    pub thought: Option<String>,
    pub action: Option<String>,
    pub observation: Option<String>,
    pub generate_embedding: bool,
    pub metadata: Metadata,
}

impl Default for StepInput {
    fn default() -> Self {
        Self {
            thought: None,
            action: None,
            observation: None,
            generate_embedding: true,
            metadata: Metadata::new(),
        }
    }
}

/// Options for [`ReasoningMemory::record_tool_call`].
#[derive(Debug, Clone)]
pub struct ToolCallOptions {
    pub user_identifier: Option<String>,
    pub result: Option<Value>,
    pub status: ToolCallStatus,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
    /// Set the step's observation from the result (ReAct style).
    pub auto_observation: bool,
    /// Message that triggered the call (`TRIGGERED_BY`).
    pub message_id: Option<Uuid>,
    /// Entities the call touched, written as `TOUCHED` edges from the step.
    pub touched_entities: Vec<EntityRef>,
}

impl Default for ToolCallOptions {
    fn default() -> Self {
        Self {
            user_identifier: None,
            result: None,
            status: ToolCallStatus::Success,
            duration_ms: None,
            error: None,
            auto_observation: false,
            message_id: None,
            touched_entities: Vec::new(),
        }
    }
}

/// Options for [`ReasoningMemory::get_similar_traces`] and
/// [`ReasoningMemory::search_steps`].
#[derive(Debug, Clone)]
pub struct SimilarityOptions {
    pub limit: i64,
    pub success_only: bool,
    pub threshold: f64,
}

impl SimilarityOptions {
    /// Upstream `get_similar_traces` defaults.
    pub const TRACES: Self = Self {
        limit: 5,
        success_only: true,
        threshold: 0.7,
    };
    /// Upstream `search_steps` defaults.
    pub const STEPS: Self = Self {
        limit: 10,
        success_only: true,
        threshold: 0.7,
    };
}

pub struct ReasoningMemory {
    store: Arc<dyn ReasoningStore>,
    embedder: Option<Arc<dyn Embedder>>,
    multi_tenant: bool,
}

impl ReasoningMemory {
    pub fn new(
        store: Arc<dyn ReasoningStore>,
        embedder: Option<Arc<dyn Embedder>>,
        multi_tenant: bool,
    ) -> Self {
        Self {
            store,
            embedder,
            multi_tenant,
        }
    }

    fn owner_scope(&self, owner: Option<&str>) -> MemoryResult<OwnerScope> {
        if self.multi_tenant && owner.is_none() {
            return Err(MemoryError::invalid("user_identifier required"));
        }
        OwnerScope::from_option(owner)
    }
    async fn embed(&self, text: &str, enabled: bool) -> MemoryResult<Option<Vec<f64>>> {
        match (&self.embedder, enabled) {
            (Some(embedder), true) => Ok(Some(embedder.embed(text).await?)),
            _ => Ok(None),
        }
    }

    /// Start a trace for `task`, embedding the task into `task_embedding`.
    pub async fn start_trace(
        &self,
        session_id: &str,
        task: &str,
        options: StartTraceOptions,
    ) -> MemoryResult<ReasoningTrace> {
        let scope = self.owner_scope(options.user_identifier.as_deref())?;
        if let Some(id) = options.triggered_by_message_id {
            self.store.validate_message_scope(id, &scope).await?;
        }
        let record = TraceRecord {
            id: Uuid::new_v4(),
            session_id: session_id.to_owned(),
            task: task.to_owned(),
            task_embedding: self.embed(task, options.generate_embedding).await?,
            metadata: options.metadata,
        };
        let trace = self
            .store
            .create_trace(&record, &scope, options.triggered_by_message_id)
            .await?;
        Ok(trace)
    }

    /// Append a step with the next `step_number`.
    pub async fn add_step(&self, trace_id: Uuid, input: StepInput) -> MemoryResult<ReasoningStep> {
        let embedding = match step_embedding_text(
            input.thought.as_deref(),
            input.action.as_deref(),
            input.observation.as_deref(),
        ) {
            Some(text) => self.embed(&text, input.generate_embedding).await?,
            None => None,
        };
        let record = StepRecord {
            id: Uuid::new_v4(),
            thought: input.thought,
            action: input.action,
            observation: input.observation,
            embedding,
            metadata: input.metadata,
        };
        self.store.add_step(trace_id, &record).await
    }

    /// Record a tool call within a step. The returned call carries the
    /// caller's `arguments` and `result` values (upstream returns its
    /// in-memory model, not the JSON round trip).
    pub async fn record_tool_call(
        &self,
        step_id: Uuid,
        tool_name: &str,
        arguments: Metadata,
        options: ToolCallOptions,
    ) -> MemoryResult<ToolCall> {
        let scope = self.owner_scope(options.user_identifier.as_deref())?;
        if let Some(id) = options.message_id {
            self.store.validate_message_scope(id, &scope).await?;
        }
        let call = ToolCall {
            id: Uuid::new_v4(),
            step_id: Some(step_id),
            tool_name: tool_name.to_owned(),
            arguments,
            result: options.result.filter(|value| !value.is_null()),
            status: options.status,
            duration_ms: options.duration_ms,
            error: options.error,
            created_at: chrono::Utc::now(),
        };
        self.store
            .record_tool_call(&call, &scope, options.message_id)
            .await?;
        if options.auto_observation {
            if let Some(result) = &call.result {
                self.store
                    .set_step_observation(step_id, &format_observation(result))
                    .await?;
            }
        }
        for entity in &options.touched_entities {
            self.store.record_touched(step_id, entity).await?;
        }
        Ok(call)
    }

    /// Set the outcome and `completed_at`. Optionally embeds steps recorded
    /// without an embedding first. Step embeddings are separate writes: a failure
    /// can leave some embeddings persisted, but does not close the trace.
    pub async fn complete_trace(
        &self,
        trace_id: Uuid,
        outcome: Option<&str>,
        success: Option<bool>,
        generate_step_embeddings: bool,
    ) -> MemoryResult<ReasoningTrace> {
        if generate_step_embeddings {
            self.embed_pending_steps(trace_id).await?;
        }
        self.store
            .complete_trace(trace_id, outcome, success)
            .await?
            .ok_or_else(|| MemoryError::not_found(format!("trace {trace_id}")))
    }

    /// `_generate_step_embeddings_batch`. Returns the number of steps embedded.
    async fn embed_pending_steps(&self, trace_id: Uuid) -> MemoryResult<usize> {
        let Some(embedder) = &self.embedder else {
            return Ok(0);
        };
        let pending: Vec<(Uuid, String)> = self
            .store
            .steps_without_embedding(trace_id)
            .await?
            .into_iter()
            .filter_map(|step| {
                step_embedding_text(
                    step.thought.as_deref(),
                    step.action.as_deref(),
                    step.observation.as_deref(),
                )
                .map(|text| (step.id, text))
            })
            .collect();
        if pending.is_empty() {
            return Ok(0);
        }
        let texts: Vec<String> = pending.iter().map(|(_, text)| text.clone()).collect();
        let vectors = embedder.embed_batch(&texts).await?;
        for ((step_id, _), vector) in pending.iter().zip(&vectors) {
            self.store.set_step_embedding(*step_id, vector).await?;
        }
        Ok(pending.len())
    }

    pub async fn get_trace_with_steps(
        &self,
        trace_id: Uuid,
    ) -> MemoryResult<Option<ReasoningTrace>> {
        self.store.get_trace_with_steps(trace_id).await
    }

    /// Traces without steps; use [`Self::get_trace_with_steps`] for details.
    pub async fn list_traces(&self, query: &TraceQuery) -> MemoryResult<Vec<ReasoningTrace>> {
        self.store.list_traces(query).await
    }

    pub async fn get_session_traces(
        &self,
        session_id: &str,
        limit: i64,
    ) -> MemoryResult<Vec<ReasoningTrace>> {
        let query = TraceQuery {
            session_id: Some(session_id.to_owned()),
            limit,
            ..TraceQuery::default()
        };
        self.store.list_traces(&query).await
    }

    pub async fn get_tool_stats(&self, tool_name: Option<&str>) -> MemoryResult<Vec<ToolStats>> {
        self.store.tool_stats(tool_name).await
    }

    /// Past traces with a similar task; each carries its score in
    /// `metadata.similarity`. Without an embedder the result is empty.
    pub async fn get_similar_traces(
        &self,
        task: &str,
        options: &SimilarityOptions,
    ) -> MemoryResult<Vec<ReasoningTrace>> {
        let Some(embedder) = &self.embedder else {
            return Ok(Vec::new());
        };
        if options.limit <= 0 {
            return Ok(Vec::new());
        }
        let embedding = embedder.embed(task).await?;
        let hits = self
            .store
            .search_traces(
                &embedding,
                options.limit,
                options.threshold,
                options.success_only,
            )
            .await?;
        Ok(hits
            .into_iter()
            .map(|hit| {
                let mut trace = hit.trace;
                trace.metadata = Metadata::new();
                trace
                    .metadata
                    .insert("similarity".to_owned(), Value::from(hit.score));
                trace
            })
            .collect())
    }

    /// Vector search over step thought/action/observation.
    pub async fn search_steps(
        &self,
        query: &str,
        options: &SimilarityOptions,
    ) -> MemoryResult<Vec<ReasoningStepWithContext>> {
        let Some(embedder) = &self.embedder else {
            return Ok(Vec::new());
        };
        if options.limit <= 0 {
            return Ok(Vec::new());
        }
        let embedding = embedder.embed(query).await?;
        self.store
            .search_steps(
                &embedding,
                options.limit,
                options.threshold,
                options.success_only,
            )
            .await
    }

    /// Context block for prompts (`ReasoningMemory.get_context`).
    pub async fn get_context(
        &self,
        query: &str,
        max_traces: i64,
        success_only: bool,
    ) -> MemoryResult<String> {
        let options = SimilarityOptions {
            limit: max_traces,
            success_only,
            ..SimilarityOptions::TRACES
        };
        let traces = self.get_similar_traces(query, &options).await?;
        if traces.is_empty() {
            return Ok(String::new());
        }
        let mut parts = vec!["### Similar Past Tasks".to_owned()];
        for trace in traces {
            let similarity = trace
                .metadata
                .get("similarity")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            parts.push(format!("\n**Task**: {}", trace.task));
            parts.push(format!("- Similarity: {similarity:.2}"));
            if let Some(outcome) = trace.outcome.as_deref().filter(|o| !o.is_empty()) {
                parts.push(format!("- Outcome: {outcome}"));
            }
            if let Some(success) = trace.success {
                parts.push(format!("- Success: {}", if success { "Yes" } else { "No" }));
            }
        }
        Ok(parts.join("\n"))
    }

    // --- Cross-memory links ---

    /// `(:ReasoningTrace)-[:INITIATED_BY]->(:Message)`.
    pub async fn link_trace_to_message(
        &self,
        trace_id: Uuid,
        message_id: Uuid,
        user_identifier: Option<&str>,
    ) -> MemoryResult<bool> {
        let scope = self.owner_scope(user_identifier)?;
        self.store
            .link_trace_to_message(trace_id, message_id, &scope)
            .await
    }

    /// `(:Conversation)-[:HAS_TRACE]->(:ReasoningTrace)`.
    pub async fn link_conversation_to_trace(
        &self,
        conversation_id: Uuid,
        trace_id: Uuid,
        user_identifier: Option<&str>,
    ) -> MemoryResult<bool> {
        let scope = self.owner_scope(user_identifier)?;
        self.store
            .link_conversation_to_trace(conversation_id, trace_id, &scope)
            .await
    }

    /// `(:ToolCall)-[:TRIGGERED_BY]->(:Message)`.
    pub async fn link_tool_call_to_message(
        &self,
        tool_call_id: Uuid,
        message_id: Uuid,
        user_identifier: Option<&str>,
    ) -> MemoryResult<bool> {
        let scope = self.owner_scope(user_identifier)?;
        self.store
            .link_tool_call_to_message(tool_call_id, message_id, &scope)
            .await
    }

    /// `(:ReasoningStep)-[:TOUCHED]->(:Entity)`.
    pub async fn record_touched_entity(
        &self,
        step_id: Uuid,
        entity: &EntityRef,
    ) -> MemoryResult<bool> {
        self.store.record_touched(step_id, entity).await
    }

    // --- Hosted (Platinum) flat reasoning model ---
    //
    // A hosted conversation owns reasoning traces through
    // `(:Conversation)-[:HAS_TRACE]->(:ReasoningTrace)`, the same edge
    // upstream's `LINK_CONVERSATION_TO_TRACE` writes. `record_conversation_step`
    // appends to the conversation's most recent open trace, creating one
    // (session id = the conversation's, task = `"Conversation <id>"`) when
    // none is open, so a hosted step is an ordinary `ReasoningStep` whose
    // `thought`/`action`/`observation` hold `reasoning`/`action_taken`/
    // `result`. Reads aggregate every trace linked to the conversation.

    /// Record one step under a conversation (hosted ReAct model).
    ///
    /// `reasoning` must be an explicit, approved summary, never a provider's
    /// private chain of thought (see the module privacy policy).
    pub async fn record_conversation_step(
        &self,
        conversation_id: Uuid,
        user_identifier: Option<&str>,
        reasoning: &str,
        action_taken: &str,
        result: Option<&str>,
    ) -> MemoryResult<AgentStep> {
        let scope = self.owner_scope(user_identifier)?;
        self.conversation_steps(conversation_id, &scope).await?;
        let trace_id = match self
            .store
            .open_conversation_trace(conversation_id, &scope)
            .await?
        {
            Some(id) => id,
            None => {
                let task = format!("Conversation {conversation_id}");
                let record = TraceRecord {
                    id: Uuid::new_v4(),
                    session_id: String::new(),
                    task_embedding: self.embed(&task, true).await?,
                    task,
                    metadata: Metadata::new(),
                };
                self.store
                    .ensure_conversation_trace(conversation_id, &record, &scope)
                    .await?
            }
        };
        let input = StepInput {
            thought: Some(reasoning.to_owned()),
            action: Some(action_taken.to_owned()),
            observation: result.map(str::to_owned),
            ..StepInput::default()
        };
        let embedding = self
            .embed(
                &step_embedding_text(
                    input.thought.as_deref(),
                    input.action.as_deref(),
                    input.observation.as_deref(),
                )
                .unwrap_or_default(),
                true,
            )
            .await?;
        let record = StepRecord {
            id: Uuid::new_v4(),
            thought: input.thought,
            action: input.action,
            observation: input.observation,
            embedding,
            metadata: input.metadata,
        };
        let step = self
            .store
            .add_conversation_step(conversation_id, trace_id, &scope, &record)
            .await?;
        Ok(AgentStep::from_step(conversation_id, &step))
    }

    async fn conversation_steps(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<Vec<ReasoningStep>> {
        self.store
            .conversation_steps(conversation_id, scope)
            .await?
            .ok_or_else(|| MemoryError::not_found(format!("conversation {conversation_id}")))
    }

    pub async fn list_conversation_steps(
        &self,
        conversation_id: Uuid,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Vec<AgentStep>> {
        let scope = self.owner_scope(user_identifier)?;
        Ok(self
            .conversation_steps(conversation_id, &scope)
            .await?
            .iter()
            .map(|step| AgentStep::from_step(conversation_id, step))
            .collect())
    }

    pub async fn get_conversation_trace(
        &self,
        conversation_id: Uuid,
        user_identifier: Option<&str>,
    ) -> MemoryResult<ConversationTrace> {
        let scope = self.owner_scope(user_identifier)?;
        let steps = self.conversation_steps(conversation_id, &scope).await?;
        Ok(ConversationTrace {
            conversation_id,
            steps: steps
                .iter()
                .map(|step| AgentStep::from_step(conversation_id, step))
                .collect(),
            tool_calls: steps.into_iter().flat_map(|step| step.tool_calls).collect(),
        })
    }

    /// A conversation step with its tool calls and touched entities.
    pub async fn explain_step(
        &self,
        step_id: Uuid,
        user_identifier: Option<&str>,
    ) -> MemoryResult<StepExplanation> {
        let scope = self.owner_scope(user_identifier)?;
        let context = self
            .store
            .step_context(step_id, &scope)
            .await?
            .ok_or_else(|| MemoryError::not_found(format!("step {step_id}")))?;
        let conversation_id = context.conversation_id.ok_or_else(|| {
            MemoryError::not_found(format!(
                "step {step_id} is not recorded under a conversation"
            ))
        })?;
        Ok(StepExplanation {
            step: AgentStep::from_step(conversation_id, &context.step),
            tool_calls: context.step.tool_calls,
            influenced_entities: context.touched_entities,
        })
    }

    /// Conversation steps that touched the entity (empty when none did).
    pub async fn get_entity_provenance(
        &self,
        entity_id: Uuid,
        user_identifier: Option<&str>,
    ) -> MemoryResult<EntityProvenance> {
        let scope = self.owner_scope(user_identifier)?;
        let steps = self
            .store
            .entity_provenance(&entity_id.to_string(), &scope)
            .await?
            .iter()
            .map(|hit| AgentStep::from_step(hit.conversation_id, &hit.step))
            .collect();
        Ok(EntityProvenance { entity_id, steps })
    }
}

/// Text embedded for a step: `"Thought: … Action: … Observation: …"` with
/// empty parts omitted; `None` when every part is empty.
pub fn step_embedding_text(
    thought: Option<&str>,
    action: Option<&str>,
    observation: Option<&str>,
) -> Option<String> {
    let parts: Vec<String> = [
        ("Thought", thought),
        ("Action", action),
        ("Observation", observation),
    ]
    .into_iter()
    .filter_map(|(label, text)| {
        text.filter(|text| !text.is_empty())
            .map(|text| format!("{label}: {text}"))
    })
    .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// `_format_observation`: strings verbatim, objects and arrays as indented
/// JSON, scalars as Python's `str()`; capped at 1000 characters.
pub fn format_observation(result: &Value) -> String {
    let text = match result {
        Value::String(text) => text.clone(),
        Value::Object(_) | Value::Array(_) => {
            serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string())
        }
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Null => "None".to_owned(),
        Value::Number(number) => number.to_string(),
    };
    text.chars().take(MAX_OBSERVATION_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::Utc;
    use muninn_core::ports::{ConversationStep, ScoredTrace, StepContext};
    use serde_json::json;

    use super::*;

    struct RejectEmbedder;
    #[async_trait]
    impl Embedder for RejectEmbedder {
        fn dimensions(&self) -> usize {
            1
        }
        async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
            panic!("embedding must not run before scope validation")
        }
    }

    #[tokio::test]
    async fn foreign_trigger_rejects_before_embedding_and_leaves_no_trace_or_call() {
        let store = Arc::new(FakeStore {
            reject_message: true,
            ..Default::default()
        });
        let memory = ReasoningMemory::new(store.clone(), Some(Arc::new(RejectEmbedder)), true);
        let id = Uuid::new_v4();
        assert!(matches!(
            memory
                .start_trace(
                    "same-session",
                    "task",
                    StartTraceOptions {
                        user_identifier: Some("A".into()),
                        triggered_by_message_id: Some(id),
                        ..Default::default()
                    }
                )
                .await,
            Err(MemoryError::NotFound { .. })
        ));
        assert!(matches!(
            memory
                .record_tool_call(
                    id,
                    "tool",
                    Metadata::new(),
                    ToolCallOptions {
                        user_identifier: Some("A".into()),
                        message_id: Some(id),
                        ..Default::default()
                    }
                )
                .await,
            Err(MemoryError::NotFound { .. })
        ));
        assert!(store.traces.lock().unwrap().is_empty());
        assert!(store.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn hosted_and_link_operations_reject_invalid_scope_before_dependencies() {
        for owner in [None, Some(""), Some("  ")] {
            let store = Arc::new(FakeStore::default());
            let memory = ReasoningMemory::new(store.clone(), Some(Arc::new(RejectEmbedder)), true);
            let id = Uuid::new_v4();
            assert!(matches!(
                memory
                    .record_conversation_step(id, owner, "r", "a", None)
                    .await,
                Err(MemoryError::InvalidInput { .. })
            ));
            assert!(matches!(
                memory.list_conversation_steps(id, owner).await,
                Err(MemoryError::InvalidInput { .. })
            ));
            assert!(matches!(
                memory.get_conversation_trace(id, owner).await,
                Err(MemoryError::InvalidInput { .. })
            ));
            assert!(memory.link_trace_to_message(id, id, owner).await.is_err());
            assert!(memory
                .link_conversation_to_trace(id, id, owner)
                .await
                .is_err());
            assert!(memory
                .link_tool_call_to_message(id, id, owner)
                .await
                .is_err());
            assert!(memory
                .record_tool_call(
                    id,
                    "tool",
                    Metadata::new(),
                    ToolCallOptions {
                        user_identifier: owner.map(str::to_owned),
                        ..Default::default()
                    }
                )
                .await
                .is_err());
            assert!(store.traces.lock().unwrap().is_empty());
            assert!(store.calls.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn blank_owner_rejected_before_trace_creation() {
        let store = Arc::new(FakeStore::default());
        let memory = ReasoningMemory::new(store.clone(), Some(Arc::new(RejectEmbedder)), true);
        let result = memory
            .start_trace(
                "session",
                "task",
                StartTraceOptions {
                    user_identifier: Some("  ".into()),
                    ..Default::default()
                },
            )
            .await;
        assert!(matches!(result, Err(MemoryError::InvalidInput { .. })));
        assert!(store.traces.lock().unwrap().is_empty());
    }

    #[test]
    fn step_embedding_text_skips_empty_parts() {
        assert_eq!(
            step_embedding_text(Some("t"), None, Some("o")).as_deref(),
            Some("Thought: t Observation: o")
        );
        assert_eq!(step_embedding_text(None, Some(""), None), None);
    }

    #[test]
    fn observations_are_capped_and_strings_kept_verbatim() {
        assert_eq!(format_observation(&json!("plain")), "plain");
        assert_eq!(format_observation(&json!(42)), "42");
        assert_eq!(
            format_observation(&json!("x".repeat(1500))).chars().count(),
            1000
        );
        assert!(format_observation(&json!({"a": 1})).contains("\"a\": 1"));
    }

    /// Embeds every text as a one-element vector of its length.
    struct LengthEmbedder;

    #[async_trait]
    impl Embedder for LengthEmbedder {
        fn dimensions(&self) -> usize {
            1
        }

        async fn embed(&self, text: &str) -> MemoryResult<Vec<f64>> {
            #[allow(clippy::cast_precision_loss)]
            Ok(vec![text.len() as f64])
        }
    }

    fn trace_from(record: &TraceRecord) -> ReasoningTrace {
        ReasoningTrace {
            id: record.id,
            session_id: record.session_id.clone(),
            task: record.task.clone(),
            task_embedding: record.task_embedding.clone(),
            steps: Vec::new(),
            outcome: None,
            success: None,
            started_at: Utc::now(),
            completed_at: None,
            metadata: record.metadata.clone(),
        }
    }

    /// In-memory store recording the calls the use cases make.
    #[derive(Default)]
    struct FakeStore {
        closed: Mutex<bool>,
        fail_completion: bool,
        reject_message: bool,
        fail_step_write: bool,
        traces: Mutex<Vec<TraceRecord>>,
        steps: Mutex<Vec<(Uuid, StepRecord)>>,
        calls: Mutex<Vec<ToolCall>>,
        observations: Mutex<Vec<(Uuid, String)>>,
        touched: Mutex<Vec<(Uuid, EntityRef)>>,
        users: Mutex<Vec<(String, Uuid)>>,
        conversation_traces: Mutex<Vec<(Uuid, Uuid)>>,
        conversations: Vec<Uuid>,
    }

    #[async_trait]
    impl ReasoningStore for FakeStore {
        async fn create_trace(
            &self,
            trace: &TraceRecord,
            scope: &OwnerScope,
            message_id: Option<Uuid>,
        ) -> MemoryResult<ReasoningTrace> {
            if let Some(owner) = scope.as_str() {
                self.link_user_to_trace(owner, trace.id).await?;
            }
            if let Some(id) = message_id {
                self.link_trace_to_message(trace.id, id, scope).await?;
            }
            let _ = message_id;
            self.traces.lock().unwrap().push(trace.clone());
            Ok(trace_from(trace))
        }
        async fn validate_message_scope(
            &self,
            _message_id: Uuid,
            _scope: &OwnerScope,
        ) -> MemoryResult<()> {
            if self.reject_message {
                Err(MemoryError::not_found("message"))
            } else {
                Ok(())
            }
        }
        async fn add_conversation_step(
            &self,
            _conversation_id: Uuid,
            trace_id: Uuid,
            _scope: &OwnerScope,
            step: &StepRecord,
        ) -> MemoryResult<ReasoningStep> {
            self.add_step(trace_id, step).await
        }
        async fn link_user_to_trace(&self, user: &str, trace_id: Uuid) -> MemoryResult<()> {
            self.users.lock().unwrap().push((user.to_owned(), trace_id));
            Ok(())
        }
        async fn add_step(&self, trace_id: Uuid, step: &StepRecord) -> MemoryResult<ReasoningStep> {
            let number = {
                let mut steps = self.steps.lock().unwrap();
                steps.push((trace_id, step.clone()));
                steps.iter().filter(|(id, _)| *id == trace_id).count()
            };
            Ok(ReasoningStep {
                id: step.id,
                trace_id,
                step_number: i64::try_from(number).unwrap(),
                thought: step.thought.clone(),
                action: step.action.clone(),
                observation: step.observation.clone(),
                tool_calls: Vec::new(),
                embedding: step.embedding.clone(),
                created_at: Utc::now(),
                metadata: step.metadata.clone(),
            })
        }
        async fn set_step_observation(&self, step_id: Uuid, text: &str) -> MemoryResult<()> {
            self.observations
                .lock()
                .unwrap()
                .push((step_id, text.to_owned()));
            Ok(())
        }
        async fn steps_without_embedding(
            &self,
            trace_id: Uuid,
        ) -> MemoryResult<Vec<ReasoningStep>> {
            Ok(self
                .steps
                .lock()
                .unwrap()
                .iter()
                .filter(|(id, step)| *id == trace_id && step.embedding.is_none())
                .map(|(_, step)| ReasoningStep {
                    id: step.id,
                    trace_id,
                    step_number: 1,
                    thought: step.thought.clone(),
                    action: step.action.clone(),
                    observation: step.observation.clone(),
                    tool_calls: Vec::new(),
                    embedding: None,
                    created_at: Utc::now(),
                    metadata: step.metadata.clone(),
                })
                .collect())
        }
        async fn set_step_embedding(&self, _: Uuid, _: &[f64]) -> MemoryResult<()> {
            if self.fail_step_write {
                return Err(MemoryError::storage("step write failed"));
            }
            Ok(())
        }
        async fn record_tool_call(
            &self,
            call: &ToolCall,
            scope: &OwnerScope,
            message_id: Option<Uuid>,
        ) -> MemoryResult<()> {
            if let Some(id) = message_id {
                self.link_tool_call_to_message(call.id, id, scope).await?;
            }
            let _ = message_id;
            self.calls.lock().unwrap().push(call.clone());
            Ok(())
        }
        async fn complete_trace(
            &self,
            trace_id: Uuid,
            outcome: Option<&str>,
            success: Option<bool>,
        ) -> MemoryResult<Option<ReasoningTrace>> {
            if self.fail_completion {
                return Err(MemoryError::storage("completion failed"));
            }
            let mut trace = trace_from(&self.traces.lock().unwrap()[0]);
            assert_eq!(trace_id, trace.id);
            trace.outcome = outcome.map(str::to_owned);
            trace.success = success;
            trace.completed_at = Some(Utc::now());
            *self.closed.lock().unwrap() = true;
            Ok(Some(trace))
        }
        async fn get_trace_with_steps(&self, _: Uuid) -> MemoryResult<Option<ReasoningTrace>> {
            Ok(None)
        }
        async fn list_traces(&self, _: &TraceQuery) -> MemoryResult<Vec<ReasoningTrace>> {
            Ok(Vec::new())
        }
        async fn tool_stats(&self, _: Option<&str>) -> MemoryResult<Vec<ToolStats>> {
            Ok(Vec::new())
        }
        async fn search_traces(
            &self,
            _: &[f64],
            _: i64,
            _: f64,
            _: bool,
        ) -> MemoryResult<Vec<ScoredTrace>> {
            Ok(self
                .traces
                .lock()
                .unwrap()
                .iter()
                .map(|record| ScoredTrace {
                    trace: trace_from(record),
                    score: 0.9,
                })
                .collect())
        }
        async fn search_steps(
            &self,
            _: &[f64],
            _: i64,
            _: f64,
            _: bool,
        ) -> MemoryResult<Vec<ReasoningStepWithContext>> {
            Ok(Vec::new())
        }
        async fn link_trace_to_message(
            &self,
            _: Uuid,
            _: Uuid,
            scope: &OwnerScope,
        ) -> MemoryResult<bool> {
            let _ = scope;
            Ok(true)
        }
        async fn link_conversation_to_trace(
            &self,
            _: Uuid,
            _: Uuid,
            scope: &OwnerScope,
        ) -> MemoryResult<bool> {
            let _ = scope;
            Ok(true)
        }
        async fn link_tool_call_to_message(
            &self,
            _: Uuid,
            _: Uuid,
            scope: &OwnerScope,
        ) -> MemoryResult<bool> {
            let _ = scope;
            Ok(true)
        }
        async fn record_touched(&self, step_id: Uuid, entity: &EntityRef) -> MemoryResult<bool> {
            self.touched.lock().unwrap().push((step_id, entity.clone()));
            Ok(true)
        }
        async fn open_conversation_trace(
            &self,
            conversation: Uuid,
            scope: &OwnerScope,
        ) -> MemoryResult<Option<Uuid>> {
            let _ = scope;
            Ok(self
                .conversation_traces
                .lock()
                .unwrap()
                .iter()
                .find(|(c, _)| *c == conversation)
                .map(|(_, t)| *t))
        }
        async fn ensure_conversation_trace(
            &self,
            conversation: Uuid,
            trace: &TraceRecord,
            scope: &OwnerScope,
        ) -> MemoryResult<Uuid> {
            let _ = scope;
            if !self.conversations.contains(&conversation) {
                return Err(MemoryError::not_found("conversation"));
            }
            self.traces.lock().unwrap().push(trace.clone());
            self.conversation_traces
                .lock()
                .unwrap()
                .push((conversation, trace.id));
            Ok(trace.id)
        }
        async fn conversation_steps(
            &self,
            conversation: Uuid,
            scope: &OwnerScope,
        ) -> MemoryResult<Option<Vec<ReasoningStep>>> {
            let _ = scope;
            Ok(self.conversations.contains(&conversation).then(Vec::new))
        }
        async fn step_context(
            &self,
            step_id: Uuid,
            _scope: &OwnerScope,
        ) -> MemoryResult<Option<StepContext>> {
            let steps = self.steps.lock().unwrap();
            Ok(steps
                .iter()
                .find(|(_, s)| s.id == step_id)
                .map(|(trace_id, s)| StepContext {
                    conversation_id: None,
                    step: ReasoningStep {
                        id: s.id,
                        trace_id: *trace_id,
                        step_number: 1,
                        thought: s.thought.clone(),
                        action: None,
                        observation: None,
                        tool_calls: Vec::new(),
                        embedding: None,
                        created_at: Utc::now(),
                        metadata: Metadata::new(),
                    },
                    touched_entities: Vec::new(),
                }))
        }
        async fn entity_provenance(
            &self,
            _: &str,
            _: &OwnerScope,
        ) -> MemoryResult<Vec<ConversationStep>> {
            Ok(Vec::new())
        }
    }

    fn memory(store: Arc<FakeStore>, multi_tenant: bool) -> ReasoningMemory {
        ReasoningMemory::new(store, Some(Arc::new(LengthEmbedder)), multi_tenant)
    }

    struct FailingEmbedder;
    #[async_trait]
    impl Embedder for FailingEmbedder {
        fn dimensions(&self) -> usize {
            1
        }
        async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
            Err(MemoryError::provider("embedding failed"))
        }
    }

    #[tokio::test]
    async fn completion_failures_leave_trace_open_and_success_is_coherent() {
        for failure in ["embed", "step", "close", "none"] {
            let store = Arc::new(FakeStore {
                fail_completion: failure == "close",
                fail_step_write: failure == "step",
                ..FakeStore::default()
            });
            let mem = memory(store.clone(), false);
            let trace = mem
                .start_trace("s", "task", StartTraceOptions::default())
                .await
                .unwrap();
            mem.add_step(
                trace.id,
                StepInput {
                    thought: Some("pending".into()),
                    generate_embedding: false,
                    ..StepInput::default()
                },
            )
            .await
            .unwrap();
            let embedder: Arc<dyn Embedder> = if failure == "embed" {
                Arc::new(FailingEmbedder)
            } else {
                Arc::new(LengthEmbedder)
            };
            let completing = ReasoningMemory::new(store.clone(), Some(embedder), false);
            let result = completing
                .complete_trace(trace.id, Some("done"), Some(true), true)
                .await;
            assert_eq!(
                *store.closed.lock().unwrap(),
                failure == "none",
                "{failure}"
            );
            if failure == "none" {
                let completed = result.unwrap();
                assert_eq!(completed.id, trace.id);
                assert_eq!(completed.outcome.as_deref(), Some("done"));
                assert_eq!(completed.success, Some(true));
                assert!(completed.completed_at.is_some());
            } else {
                assert!(result.is_err(), "{failure}");
            }
        }
    }

    #[tokio::test]
    async fn start_trace_embeds_the_task_and_scopes_the_user() {
        let store = Arc::new(FakeStore::default());
        let options = StartTraceOptions {
            user_identifier: Some("alice".into()),
            ..StartTraceOptions::default()
        };
        let trace = memory(store.clone(), false)
            .start_trace("s1", "find bob", options)
            .await
            .unwrap();
        assert_eq!(trace.task_embedding, Some(vec![8.0]));
        assert_eq!(store.users.lock().unwrap()[0], ("alice".into(), trace.id));
    }

    #[tokio::test]
    async fn multi_tenant_traces_require_a_user() {
        let store = Arc::new(FakeStore::default());
        let err = memory(store.clone(), true)
            .start_trace("s1", "task", StartTraceOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
        assert!(store.traces.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn steps_without_content_are_not_embedded() {
        let store = Arc::new(FakeStore::default());
        let step = memory(store, false)
            .add_step(Uuid::new_v4(), StepInput::default())
            .await
            .unwrap();
        assert_eq!(step.embedding, None);
    }

    #[tokio::test]
    async fn tool_calls_write_observation_and_touched_edges() {
        let store = Arc::new(FakeStore::default());
        let step_id = Uuid::new_v4();
        let options = ToolCallOptions {
            result: Some(json!("42 rows")),
            status: ToolCallStatus::Timeout,
            auto_observation: true,
            touched_entities: vec![EntityRef::Name("Alice".into())],
            ..ToolCallOptions::default()
        };
        let call = memory(store.clone(), false)
            .record_tool_call(step_id, "sql", Metadata::new(), options)
            .await
            .unwrap();
        assert_eq!(call.status, ToolCallStatus::Timeout);
        assert_eq!(store.calls.lock().unwrap()[0].step_id, Some(step_id));
        assert_eq!(
            store.observations.lock().unwrap()[0],
            (step_id, "42 rows".into())
        );
        assert_eq!(store.touched.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn similar_traces_carry_their_score() {
        let store = Arc::new(FakeStore::default());
        let memory = memory(store, false);
        memory
            .start_trace("s1", "task", StartTraceOptions::default())
            .await
            .unwrap();
        let hits = memory
            .get_similar_traces("task", &SimilarityOptions::TRACES)
            .await
            .unwrap();
        assert_eq!(hits[0].metadata["similarity"], json!(0.9));
    }

    #[tokio::test]
    async fn conversation_steps_reuse_the_open_trace() {
        let conversation = Uuid::new_v4();
        let store = Arc::new(FakeStore {
            conversations: vec![conversation],
            ..FakeStore::default()
        });
        let memory = memory(store.clone(), false);
        let first = memory
            .record_conversation_step(conversation, None, "hypothesis", "ran test", Some("passed"))
            .await
            .unwrap();
        let second = memory
            .record_conversation_step(conversation, None, "next", "ran more", None)
            .await
            .unwrap();
        assert_eq!(first.conversation_id, conversation);
        assert_eq!(first.reasoning, "hypothesis");
        assert_eq!(first.result.as_deref(), Some("passed"));
        assert_eq!(second.result, None);
        assert_eq!(store.conversation_traces.lock().unwrap().len(), 1);
        let steps = store.steps.lock().unwrap();
        assert_eq!(steps[0].0, steps[1].0);
    }

    #[tokio::test]
    async fn conversation_steps_need_an_existing_conversation() {
        let store = Arc::new(FakeStore::default());
        let err = memory(store, false)
            .record_conversation_step(Uuid::new_v4(), None, "r", "a", None)
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::NotFound(_)));
    }

    #[tokio::test]
    async fn context_reads_require_owner_before_store_access() {
        for multi_tenant in [false, true] {
            let memory = memory(Arc::new(FakeStore::default()), multi_tenant);
            for user in [None, Some(""), Some("   ")] {
                if !multi_tenant && user.is_none() {
                    continue;
                }
                assert!(matches!(
                    memory.explain_step(Uuid::new_v4(), user).await,
                    Err(MemoryError::InvalidInput(_))
                ));
                assert!(matches!(
                    memory.get_entity_provenance(Uuid::new_v4(), user).await,
                    Err(MemoryError::InvalidInput(_))
                ));
            }
        }
    }

    #[tokio::test]
    async fn explaining_a_step_outside_a_conversation_is_not_found() {
        let store = Arc::new(FakeStore::default());
        let memory = memory(store, false);
        let step = memory
            .add_step(
                Uuid::new_v4(),
                StepInput {
                    thought: Some("t".into()),
                    ..StepInput::default()
                },
            )
            .await
            .unwrap();
        let err = memory.explain_step(step.id, None).await.unwrap_err();
        assert!(matches!(err, MemoryError::NotFound(_)));
    }
}
